//! Hardware scanner. See `platform::scan` for the contract.
//!
//! Sources: `sysctlbyname` (CPU, memory), CPUID on Intel, one
//! `system_profiler -json` run, and `ioreg -a` for PCI devices (ids, device
//! paths, codecs, MACs, drives, XHCI ports), HID / PS2 input, battery and lid.
//! On a Hackintosh the IDs are whatever the running config injects.
//! ACPI tables are not dumped on macOS: the IORegistry only holds the tables
//! the boot loader already patched, which are not a valid base for new SSDTs.

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use tracing::{debug, warn};

use crate::contracts::{
    AudioDevice, ChassisInfo, CpuInfo, DetectedHardware, FirmwareInfo, GpuInfo, InputDevice,
    MemoryInfo, MotherboardInfo, NetworkDevice, NetworkKind, PciLocation, StorageDevice,
    UsbControllerInfo,
};
use crate::error::AppError;
use crate::platform::common::{
    base_clock_from_brand, capture_output, is_hdmi_codec_vendor, read_cpuid, resolve_hypervisor,
    storage_kind_from_class, usb_controller_kind, CpuidInfo,
};
use crate::tasks::cancellation::CancellationToken;

use super::ioreg::{self, IoPciDevice, OPENCORE_GUID};
use super::profiler::{self, ProfilerReport};
use super::sysctl;

const PROFILER_TIMEOUT: Duration = Duration::from_secs(60);
const IOREG_TIMEOUT: Duration = Duration::from_secs(20);

const SYSTEM_PROFILER: &str = "/usr/sbin/system_profiler";
const IOREG: &str = "/usr/sbin/ioreg";
const NVRAM: &str = "/usr/sbin/nvram";

pub async fn scan(
    acpi_dir: &Path,
    cancel: &CancellationToken,
) -> Result<DetectedHardware, AppError> {
    debug!(acpi_dir = %acpi_dir.display(), "macOS scan started");
    cancel.check()?;
    // The default detail level: "mini" leaves out the MAC addresses.
    let mut profiler_args = vec!["-json"];
    profiler_args.extend_from_slice(profiler::DATA_TYPES);

    let (profiler_out, pci_out, hid_out, ps2_out, battery_out, lid_out, nvram_out) = tokio::join!(
        run(SYSTEM_PROFILER, &profiler_args, PROFILER_TIMEOUT, cancel),
        run(
            IOREG,
            &["-a", "-r", "-c", "IOPCIDevice"],
            IOREG_TIMEOUT,
            cancel
        ),
        run(
            IOREG,
            &["-a", "-r", "-d", "1", "-c", "IOHIDDevice"],
            IOREG_TIMEOUT,
            cancel
        ),
        run(
            IOREG,
            &["-a", "-r", "-c", "ApplePS2Controller"],
            IOREG_TIMEOUT,
            cancel
        ),
        run(
            IOREG,
            &["-a", "-r", "-d", "1", "-c", "AppleSmartBattery"],
            IOREG_TIMEOUT,
            cancel
        ),
        run(
            IOREG,
            &["-a", "-r", "-d", "1", "-k", "AppleClamshellState"],
            IOREG_TIMEOUT,
            cancel
        ),
        run(NVRAM, &["-x", "-p"], IOREG_TIMEOUT, cancel),
    );
    cancel.check()?;

    let mut warnings = Vec::new();
    let mut take = |result: Result<Vec<u8>, AppError>, what: &str| -> Vec<u8> {
        result.unwrap_or_else(|e| {
            warn!(error = %e, "{what} unavailable");
            warnings.push(format!("{what} unavailable: {}", e.message));
            Vec::new()
        })
    };
    let profiler_json = take(profiler_out, "system_profiler");
    let pci_xml = take(pci_out, "PCI registry");
    let hid_xml = take(hid_out, "HID registry");
    let ps2_xml = take(ps2_out, "PS2 registry");
    let battery_xml = take(battery_out, "Battery registry");
    let lid_xml = take(lid_out, "Power management registry");
    let nvram_xml = take(nvram_out, "NVRAM");

    let profiler = if profiler_json.is_empty() {
        ProfilerReport::default()
    } else {
        profiler::parse(&String::from_utf8_lossy(&profiler_json)).unwrap_or_else(|e| {
            warnings.push(e.message);
            ProfilerReport::default()
        })
    };
    let pci = ioreg::parse_pci_tree(&pci_xml).unwrap_or_else(|e| {
        warnings.push(e.message);
        Vec::new()
    });
    let mut input = ioreg::parse_ps2_devices(&ps2_xml).unwrap_or_default();
    input.extend(ioreg::parse_hid_devices(&hid_xml).unwrap_or_else(|e| {
        warnings.push(e.message);
        Vec::new()
    }));

    let host = HostFacts::read();
    let nvram = |name: &str| ioreg::nvram_text(&nvram_xml, &format!("{OPENCORE_GUID}:{name}"));
    let collected = Collected {
        profiler,
        pci,
        input,
        has_battery: ioreg::has_entries(&battery_xml),
        has_lid: ioreg::has_entries(&lid_xml),
        opencore_version: nvram("opencore-version"),
        oem_vendor: nvram("oem-vendor"),
        oem_product: nvram("oem-product"),
        oem_board: nvram("oem-board"),
    };
    let mut hw = assemble(collected, &host);
    hw.warnings.splice(0..0, warnings);
    Ok(hw)
}

async fn run(
    program: &str,
    args: &[&str],
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, AppError> {
    let mut command = Command::new(program);
    command.args(args).env("LC_ALL", "C");
    let output = capture_output(command, timeout, cancel, program).await?;
    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::new(
            "SCAN_PROCESS",
            format!("{program} failed: {}", stderr.trim()),
        ));
    }
    Ok(output.stdout)
}

/// Values read in-process from sysctl / CPUID.
#[derive(Debug, Clone, Default)]
struct HostFacts {
    apple_silicon: bool,
    cpuid: Option<CpuidInfo>,
    brand: Option<String>,
    vendor: Option<String>,
    family: Option<u32>,
    model: Option<u32>,
    stepping: Option<u32>,
    /// Intel-only feature word lists (`machdep.cpu.features` and friends).
    feature_words: Vec<String>,
    cores: Option<u64>,
    threads: Option<u64>,
    packages: Option<u64>,
    frequency_hz: Option<u64>,
    memory_bytes: Option<u64>,
    hw_model: Option<String>,
    in_vm: bool,
}

impl HostFacts {
    fn read() -> Self {
        // Under Rosetta CPUID describes a virtual Intel CPU; treat it as Apple silicon.
        let apple_silicon = sysctl::number("hw.optional.arm64") == Some(1)
            || sysctl::number("sysctl.proc_translated") == Some(1);
        let to_u32 = |v: u64| u32::try_from(v).ok();
        Self {
            apple_silicon,
            cpuid: if apple_silicon { None } else { read_cpuid() },
            brand: sysctl::string("machdep.cpu.brand_string"),
            vendor: sysctl::string("machdep.cpu.vendor"),
            family: sysctl::number("machdep.cpu.family").and_then(to_u32),
            model: sysctl::number("machdep.cpu.model").and_then(to_u32),
            stepping: sysctl::number("machdep.cpu.stepping").and_then(to_u32),
            feature_words: [
                "machdep.cpu.features",
                "machdep.cpu.leaf7_features",
                "machdep.cpu.extfeatures",
            ]
            .iter()
            .filter_map(|name| sysctl::string(name))
            .collect(),
            cores: sysctl::number("hw.physicalcpu")
                .or_else(|| sysctl::number("machdep.cpu.core_count")),
            threads: sysctl::number("hw.logicalcpu")
                .or_else(|| sysctl::number("machdep.cpu.thread_count")),
            packages: sysctl::number("hw.packages"),
            frequency_hz: sysctl::number("hw.cpufrequency"),
            memory_bytes: sysctl::number("hw.memsize"),
            hw_model: sysctl::string("hw.model"),
            in_vm: sysctl::number("kern.hv_vmm_present") == Some(1),
        }
    }
}

/// Everything gathered from child processes.
#[derive(Debug, Clone, Default)]
struct Collected {
    profiler: ProfilerReport,
    pci: Vec<IoPciDevice>,
    input: Vec<InputDevice>,
    has_battery: bool,
    has_lid: bool,
    opencore_version: Option<String>,
    oem_vendor: Option<String>,
    oem_product: Option<String>,
    oem_board: Option<String>,
}

fn assemble(c: Collected, host: &HostFacts) -> DetectedHardware {
    let mut warnings = vec![
        "ACPI tables are not dumped on macOS (the registry only holds tables already patched by the boot loader); \
         scan the target PC from Windows or Linux to generate SSDTs from its own DSDT"
            .to_string(),
    ];
    if host.apple_silicon {
        warnings.push("Apple silicon Mac: the scan describes this Mac only".to_string());
    }
    if let Some(version) = &c.opencore_version {
        warnings.push(format!(
            "Booted through OpenCore ({version}): device ids and SMBIOS reflect the running configuration"
        ));
    }
    let hypervisor = host.in_vm.then(|| {
        resolve_hypervisor(host.cpuid.as_ref(), None, host.hw_model.as_deref())
            .unwrap_or_else(|| "Unknown hypervisor".to_string())
    });
    DetectedHardware {
        host_os: "macos".into(),
        cpu: cpu_info(host, &c.profiler),
        gpus: gpus(&c.pci, &c.profiler),
        audio: audio(&c.pci, &c.profiler),
        network: network(&c.pci, &c.profiler),
        input: c.input.clone(),
        memory: MemoryInfo {
            total_mb: host.memory_bytes.or(c.profiler.memory_bytes).unwrap_or(0) / (1024 * 1024),
        },
        motherboard: motherboard(&c, host),
        storage: storage(&c.pci, &c.profiler),
        usb_controllers: usb_controllers(&c.pci, &c.profiler),
        chassis: ChassisInfo {
            chassis_types: Vec::new(),
            manufacturer: vendor(&c),
            has_battery: c.has_battery,
            has_lid: c.has_lid,
        },
        firmware: firmware(&c, host),
        acpi_tables_dir: None,
        hypervisor,
        warnings,
    }
}

fn cpu_info(host: &HostFacts, profiler: &ProfilerReport) -> CpuInfo {
    let count = |v: Option<u64>| v.and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let mut cpu = CpuInfo {
        cores: count(host.cores),
        threads: count(host.threads),
        packages: count(host.packages).max(1),
        ..Default::default()
    };
    if host.apple_silicon {
        cpu.vendor = "Apple".into();
        cpu.name = profiler
            .chip
            .clone()
            .or_else(|| host.brand.clone())
            .unwrap_or_else(|| "Apple silicon".into());
        return cpu;
    }
    match &host.cpuid {
        Some(id) => {
            cpu.vendor = id.vendor.clone();
            cpu.name = if id.brand.is_empty() {
                host.brand.clone().unwrap_or_default()
            } else {
                id.brand.clone()
            };
            cpu.family = Some(id.family);
            cpu.model = Some(id.model);
            cpu.stepping = Some(id.stepping);
            cpu.features = id.features.clone();
        }
        None => {
            cpu.vendor = host.vendor.clone().unwrap_or_default();
            cpu.name = host
                .brand
                .clone()
                .or_else(|| profiler.chip.clone())
                .unwrap_or_default();
            cpu.family = host.family;
            cpu.model = host.model;
            cpu.stepping = host.stepping;
            let words: Vec<&str> = host.feature_words.iter().map(String::as_str).collect();
            cpu.features = sysctl::features_from_words(&words);
        }
    }
    cpu.base_clock_mhz = host
        .frequency_hz
        .map(|hz| (hz / 1_000_000) as u32)
        .filter(|mhz| *mhz > 0)
        .or_else(|| base_clock_from_brand(&cpu.name));
    cpu
}

fn of_class(pci: &[IoPciDevice], base: u8) -> impl Iterator<Item = &IoPciDevice> {
    pci.iter()
        .filter(move |d| d.class.is_some_and(|c| c.base == base))
}

fn gpus(pci: &[IoPciDevice], profiler: &ProfilerReport) -> Vec<GpuInfo> {
    let from_pci: Vec<GpuInfo> = of_class(pci, 0x03)
        .map(|d| {
            let named = profiler
                .gpus
                .iter()
                .find(|g| g.device_id.as_deref() == Some(d.device_id.as_str()));
            GpuInfo {
                name: named
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| d.name.clone()),
                vendor_id: Some(d.vendor_id.clone()),
                device_id: Some(d.device_id.clone()),
                subsystem_vendor_id: d.subsystem_vendor_id.clone(),
                subsystem_device_id: d.subsystem_device_id.clone(),
                revision: d.revision.clone(),
                vram_mb: named.and_then(|g| g.vram_mb),
                location: d.location.clone(),
            }
        })
        .collect();
    if !from_pci.is_empty() {
        return from_pci;
    }
    profiler
        .gpus
        .iter()
        .map(|g| GpuInfo {
            name: g.name.clone(),
            vendor_id: g.vendor_id.clone(),
            device_id: g.device_id.clone(),
            revision: g.revision.clone(),
            vram_mb: g.vram_mb,
            ..Default::default()
        })
        .collect()
}

fn audio(pci: &[IoPciDevice], profiler: &ProfilerReport) -> Vec<AudioDevice> {
    let mut out = Vec::new();
    for controller in
        of_class(pci, 0x04).filter(|d| d.class.is_some_and(|c| c.sub == 0x03 || c.sub == 0x01))
    {
        let bus = if controller.class.is_some_and(|c| c.sub == 0x03) {
            "hdaudio"
        } else {
            "other"
        };
        let base = AudioDevice {
            name: controller.name.clone(),
            controller_vendor_id: Some(controller.vendor_id.clone()),
            controller_device_id: Some(controller.device_id.clone()),
            location: controller.location.clone(),
            bus: bus.into(),
            ..Default::default()
        };
        if controller.codecs.is_empty() {
            out.push(base);
            continue;
        }
        for codec in &controller.codecs {
            out.push(AudioDevice {
                codec_vendor_id: Some(codec.vendor_id.clone()),
                codec_device_id: Some(codec.device_id.clone()),
                is_hdmi: is_hdmi_codec_vendor(&codec.vendor_id),
                ..base.clone()
            });
        }
    }
    if out.is_empty() {
        out.extend(profiler.builtin_audio.iter().map(|name| AudioDevice {
            name: name.clone(),
            bus: "other".into(),
            ..Default::default()
        }));
    }
    out
}

fn network(pci: &[IoPciDevice], profiler: &ProfilerReport) -> Vec<NetworkDevice> {
    let wifi_mac = profiler
        .network
        .iter()
        .find(|n| n.kind.eq_ignore_ascii_case("AirPort") || n.kind.eq_ignore_ascii_case("Wi-Fi"))
        .and_then(|n| n.mac.clone());
    let mut out: Vec<NetworkDevice> = of_class(pci, 0x02)
        .map(|d| {
            let bluetooth = d.entry_name.to_ascii_lowercase().contains("bluetooth");
            let kind = match d.class.map(|c| c.sub) {
                Some(0x00) => NetworkKind::Ethernet,
                Some(0x80) if bluetooth => NetworkKind::Bluetooth,
                Some(0x80) => NetworkKind::Wifi,
                _ => NetworkKind::Other,
            };
            let mac = d.mac_addresses.first().cloned().or_else(|| {
                (kind == NetworkKind::Wifi)
                    .then(|| wifi_mac.clone())
                    .flatten()
            });
            NetworkDevice {
                name: d.name.clone(),
                kind,
                bus: "pci".into(),
                vendor_id: Some(d.vendor_id.clone()),
                device_id: Some(d.device_id.clone()),
                subsystem_vendor_id: d.subsystem_vendor_id.clone(),
                subsystem_device_id: d.subsystem_device_id.clone(),
                mac_address: mac,
                location: d.location.clone(),
            }
        })
        .collect();
    let has_pci_bluetooth = out.iter().any(|n| n.kind == NetworkKind::Bluetooth);
    for bt in &profiler.bluetooth {
        let transport = bt
            .transport
            .clone()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if has_pci_bluetooth && transport.contains("pci") {
            continue;
        }
        let bus = if transport.contains("usb") {
            "usb"
        } else if transport.contains("pci") {
            "pci"
        } else {
            "other"
        };
        out.push(NetworkDevice {
            name: bt
                .chipset
                .clone()
                .map(|c| format!("Bluetooth {c}"))
                .unwrap_or_else(|| "Bluetooth".into()),
            kind: NetworkKind::Bluetooth,
            bus: bus.into(),
            vendor_id: bt.vendor_id.clone(),
            device_id: bt.product_id.clone(),
            ..Default::default()
        });
    }
    out
}

fn storage(pci: &[IoPciDevice], profiler: &ProfilerReport) -> Vec<StorageDevice> {
    let mut out = Vec::new();
    for controller in of_class(pci, 0x01) {
        let kind = controller
            .class
            .and_then(storage_kind_from_class)
            .unwrap_or("other");
        let base = StorageDevice {
            name: controller.name.clone(),
            kind: kind.into(),
            controller_vendor_id: Some(controller.vendor_id.clone()),
            controller_device_id: Some(controller.device_id.clone()),
            size_bytes: None,
        };
        for drive in &controller.drives {
            out.push(StorageDevice {
                name: drive.name.clone(),
                size_bytes: drive.size_bytes,
                ..base.clone()
            });
        }
        if controller.drives.is_empty() {
            out.push(base);
        }
    }
    if out.iter().all(|d| d.size_bytes.is_none()) {
        out.extend(profiler.drives.iter().map(|d| StorageDevice {
            name: d.name.clone(),
            kind: d.kind.clone(),
            size_bytes: d.size_bytes,
            ..Default::default()
        }));
    }
    out
}

fn usb_controllers(pci: &[IoPciDevice], profiler: &ProfilerReport) -> Vec<UsbControllerInfo> {
    let from_pci: Vec<UsbControllerInfo> = pci
        .iter()
        .filter(|d| d.class.is_some_and(|c| c.is(0x0c, 0x03)))
        .map(|d| {
            let mut ports = d.usb_ports.clone();
            ports.sort_by_key(|p| p.index);
            UsbControllerInfo {
                name: d.name.clone(),
                vendor_id: Some(d.vendor_id.clone()),
                device_id: Some(d.device_id.clone()),
                kind: usb_controller_kind(d.class.and_then(|c| c.prog_if), &d.name).into(),
                location: d.location.clone(),
                ports,
            }
        })
        .collect();
    if !from_pci.is_empty() {
        return from_pci;
    }
    profiler
        .usb_buses
        .iter()
        .map(|bus| {
            let label = format!("{} {}", bus.name, bus.driver.clone().unwrap_or_default());
            UsbControllerInfo {
                name: bus.name.clone(),
                vendor_id: bus.vendor_id.clone(),
                device_id: bus.device_id.clone(),
                kind: usb_controller_kind(None, &label).into(),
                location: PciLocation::default(),
                ports: Vec::new(),
            }
        })
        .collect()
}

/// OEM vendor exposed by OpenCore, "Apple Inc." on a Mac booted natively,
/// unknown on a Hackintosh that hides its SMBIOS strings.
fn vendor(c: &Collected) -> Option<String> {
    c.oem_vendor.clone().or_else(|| {
        c.opencore_version
            .is_none()
            .then(|| "Apple Inc.".to_string())
    })
}

fn motherboard(c: &Collected, host: &HostFacts) -> MotherboardInfo {
    let lpc = c
        .pci
        .iter()
        .find(|d| d.class.is_some_and(|cl| cl.is(0x06, 0x01)));
    MotherboardInfo {
        manufacturer: vendor(c),
        product: c.oem_board.clone(),
        system_manufacturer: vendor(c),
        system_product: c
            .oem_product
            .clone()
            .or_else(|| c.profiler.machine_model.clone())
            .or_else(|| host.hw_model.clone()),
        lpc_vendor_id: lpc.map(|d| d.vendor_id.clone()),
        lpc_device_id: lpc.map(|d| d.device_id.clone()),
        chipset: None,
    }
}

fn firmware(c: &Collected, host: &HostFacts) -> FirmwareInfo {
    if host.apple_silicon {
        return FirmwareInfo {
            uefi: None,
            secure_boot: None,
            bios_vendor: Some("Apple".into()),
            bios_version: c.profiler.boot_rom.clone(),
            bios_date: None,
        };
    }
    let genuine = c.opencore_version.is_none();
    FirmwareInfo {
        uefi: Some(true),
        secure_boot: None,
        bios_vendor: genuine.then(|| "Apple".to_string()),
        bios_version: if genuine {
            c.profiler.boot_rom.clone()
        } else {
            None
        },
        bios_date: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{InputKind, UsbPortInfo};
    use crate::platform::common::PciClass;
    use crate::platform::macos::ioreg::{IoCodec, IoDrive};
    use crate::platform::macos::profiler::{
        ProfilerBluetooth, ProfilerDrive, ProfilerGpu, ProfilerNetwork, ProfilerUsbBus,
    };

    fn pci(vendor: &str, device: &str, class: u32, path: &str) -> IoPciDevice {
        IoPciDevice {
            vendor_id: vendor.into(),
            device_id: device.into(),
            class: Some(PciClass::from_u32(class)),
            name: format!("pci{vendor},{device}"),
            location: PciLocation {
                pci_path: Some(path.into()),
                acpi_path: None,
            },
            ..Default::default()
        }
    }

    fn intel_host() -> HostFacts {
        HostFacts {
            cpuid: Some(CpuidInfo {
                vendor: "GenuineIntel".into(),
                brand: "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz".into(),
                family: 6,
                model: 0x9e,
                stepping: 13,
                features: vec!["sse4_2".into(), "avx2".into()],
                ..Default::default()
            }),
            cores: Some(8),
            threads: Some(8),
            packages: Some(1),
            memory_bytes: Some(32 << 30),
            hw_model: Some("iMac19,1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn assembles_hackintosh_scan() {
        let mut hdef = pci("8086", "a348", 0x040300, "PciRoot(0x0)/Pci(0x1f,0x3)");
        hdef.codecs = vec![IoCodec {
            vendor_id: "10ec".into(),
            device_id: "1220".into(),
            revision: None,
        }];
        let mut gfx = pci(
            "1002",
            "67df",
            0x030000,
            "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)",
        );
        gfx.codecs = vec![];
        let mut hdmi_audio = pci(
            "1002",
            "aaf0",
            0x040300,
            "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x1)",
        );
        hdmi_audio.codecs = vec![IoCodec {
            vendor_id: "1002".into(),
            device_id: "aa01".into(),
            revision: None,
        }];
        let mut lan = pci("8086", "15bc", 0x020000, "PciRoot(0x0)/Pci(0x1f,0x6)");
        lan.mac_addresses = vec!["a4:bb:6d:12:34:56".into()];
        let wifi = pci(
            "14e4",
            "43a0",
            0x028000,
            "PciRoot(0x0)/Pci(0x1c,0x4)/Pci(0x0,0x0)",
        );
        let mut nvme = pci(
            "144d",
            "a808",
            0x010802,
            "PciRoot(0x0)/Pci(0x1d,0x0)/Pci(0x0,0x0)",
        );
        nvme.drives = vec![IoDrive {
            name: "Samsung SSD 970 EVO Plus 1TB".into(),
            size_bytes: Some(1_000_204_886_016),
        }];
        let mut xhci = pci("8086", "a36d", 0x0c0330, "PciRoot(0x0)/Pci(0x14,0x0)");
        xhci.usb_ports = vec![
            UsbPortInfo {
                index: 17,
                speed_class: "usb3".into(),
                ..Default::default()
            },
            UsbPortInfo {
                index: 1,
                speed_class: "usb2".into(),
                ..Default::default()
            },
        ];
        let lpc = pci("8086", "a305", 0x060100, "PciRoot(0x0)/Pci(0x1f,0x0)");

        let collected = Collected {
            profiler: ProfilerReport {
                machine_model: Some("iMac19,1".into()),
                gpus: vec![ProfilerGpu {
                    name: "Radeon RX 580".into(),
                    device_id: Some("67df".into()),
                    vram_mb: Some(8192),
                    ..Default::default()
                }],
                network: vec![ProfilerNetwork {
                    name: "Wi-Fi".into(),
                    kind: "AirPort".into(),
                    interface: Some("en1".into()),
                    mac: Some("00:11:22:33:44:55".into()),
                }],
                bluetooth: vec![ProfilerBluetooth {
                    vendor_id: Some("05ac".into()),
                    product_id: Some("828d".into()),
                    transport: Some("USB".into()),
                    chipset: None,
                }],
                ..Default::default()
            },
            pci: vec![hdef, gfx, hdmi_audio, lan, wifi, nvme, xhci, lpc],
            input: vec![InputDevice {
                name: "ApplePS2Keyboard".into(),
                kind: InputKind::Keyboard,
                bus: "ps2".into(),
                ..Default::default()
            }],
            opencore_version: Some("REL-108-2026-09-27".into()),
            oem_board: Some("PRIME Z390-A".into()),
            oem_vendor: Some("ASUSTeK COMPUTER INC.".into()),
            ..Default::default()
        };
        let hw = assemble(collected, &intel_host());
        assert_eq!(hw.host_os, "macos");
        assert_eq!(hw.cpu.vendor, "GenuineIntel");
        assert_eq!(
            (hw.cpu.family, hw.cpu.model, hw.cpu.stepping),
            (Some(6), Some(0x9e), Some(13))
        );
        assert_eq!(hw.cpu.base_clock_mhz, Some(3600));
        assert_eq!(hw.memory.total_mb, 32 * 1024);
        assert_eq!(hw.gpus.len(), 1);
        assert_eq!(hw.gpus[0].name, "Radeon RX 580");
        assert_eq!(hw.gpus[0].vram_mb, Some(8192));
        assert_eq!(hw.audio.len(), 2);
        assert_eq!(hw.audio[0].codec_device_id.as_deref(), Some("1220"));
        assert!(!hw.audio[0].is_hdmi);
        assert!(hw.audio[1].is_hdmi);
        let kinds: Vec<NetworkKind> = hw.network.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [
                NetworkKind::Ethernet,
                NetworkKind::Wifi,
                NetworkKind::Bluetooth
            ]
        );
        assert_eq!(
            hw.network[1].mac_address.as_deref(),
            Some("00:11:22:33:44:55")
        );
        assert_eq!(hw.network[2].bus, "usb");
        assert_eq!(hw.storage[0].name, "Samsung SSD 970 EVO Plus 1TB");
        assert_eq!(hw.storage[0].kind, "nvme");
        assert_eq!(hw.usb_controllers[0].kind, "xhci");
        assert_eq!(hw.usb_controllers[0].ports[0].index, 1);
        assert_eq!(hw.motherboard.lpc_device_id.as_deref(), Some("a305"));
        assert_eq!(hw.motherboard.product.as_deref(), Some("PRIME Z390-A"));
        assert_eq!(
            hw.motherboard.manufacturer.as_deref(),
            Some("ASUSTeK COMPUTER INC.")
        );
        assert_eq!(hw.firmware.uefi, Some(true));
        assert_eq!(hw.firmware.bios_version, None);
        assert_eq!(hw.hypervisor, None);
        assert!(hw.warnings.iter().any(|w| w.contains("OpenCore")));
    }

    #[test]
    fn assembles_apple_silicon_scan() {
        let host = HostFacts {
            apple_silicon: true,
            brand: Some("Apple M2 Pro".into()),
            cores: Some(12),
            threads: Some(12),
            packages: Some(1),
            memory_bytes: Some(16 << 30),
            hw_model: Some("Mac14,10".into()),
            ..Default::default()
        };
        let mut wlan = pci(
            "14e4",
            "4434",
            0x028000,
            "PciRoot(0x0)/Pci(0x0,0x0)/Pci(0x0,0x0)",
        );
        wlan.entry_name = "wlan".into();
        let mut bt = pci(
            "14e4",
            "5f72",
            0x028000,
            "PciRoot(0x0)/Pci(0x0,0x0)/Pci(0x0,0x1)",
        );
        bt.entry_name = "bluetooth-pcie".into();
        let collected = Collected {
            profiler: ProfilerReport {
                chip: Some("Apple M2 Pro".into()),
                boot_rom: Some("20457.1.29".into()),
                gpus: vec![ProfilerGpu {
                    name: "Apple M2 Pro".into(),
                    ..Default::default()
                }],
                bluetooth: vec![ProfilerBluetooth {
                    vendor_id: Some("004c".into()),
                    transport: Some("PCIe".into()),
                    ..Default::default()
                }],
                drives: vec![ProfilerDrive {
                    name: "APPLE SSD AP0512Z".into(),
                    kind: "nvme".into(),
                    controller: None,
                    size_bytes: Some(500_277_792_768),
                }],
                usb_buses: vec![ProfilerUsbBus {
                    name: "USB 3.1 Bus".into(),
                    driver: Some("AppleT8112USBXHCI".into()),
                    ..Default::default()
                }],
                builtin_audio: vec!["MacBook Pro Speakers".into()],
                ..Default::default()
            },
            pci: vec![wlan, bt],
            has_battery: true,
            has_lid: true,
            ..Default::default()
        };
        let hw = assemble(collected, &host);
        assert_eq!(hw.cpu.vendor, "Apple");
        assert_eq!(hw.cpu.name, "Apple M2 Pro");
        assert!(hw.cpu.features.is_empty());
        assert_eq!(hw.cpu.cores, 12);
        assert_eq!(hw.gpus[0].name, "Apple M2 Pro");
        assert_eq!(hw.audio[0].bus, "other");
        assert_eq!(
            hw.network
                .iter()
                .filter(|n| n.kind == NetworkKind::Bluetooth)
                .count(),
            1
        );
        assert_eq!(hw.storage[0].size_bytes, Some(500_277_792_768));
        assert_eq!(hw.usb_controllers[0].kind, "xhci");
        assert!(hw.chassis.has_battery && hw.chassis.has_lid);
        assert_eq!(hw.firmware.uefi, None);
        assert_eq!(hw.firmware.bios_version.as_deref(), Some("20457.1.29"));
        assert_eq!(hw.motherboard.system_product.as_deref(), Some("Mac14,10"));
        assert_eq!(hw.motherboard.manufacturer.as_deref(), Some("Apple Inc."));
        assert!(hw.acpi_tables_dir.is_none());
    }

    /// Runs the real scanner on this Mac: `cargo test real_scan -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn real_scan_on_this_machine() {
        let dir = std::env::temp_dir().join("oc-scan-acpi");
        let hw = scan(&dir, &CancellationToken::new()).await.unwrap();
        println!("{}", serde_json::to_string_pretty(&hw).unwrap());
        assert_eq!(hw.host_os, "macos");
        assert!(hw.cpu.threads > 0);
        assert!(hw.memory.total_mb > 0);
    }
}
