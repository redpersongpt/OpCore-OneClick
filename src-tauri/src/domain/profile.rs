//! Interpretation layer: raw scanner output → canonical `HardwareProfile`.
//!
//! Scanners only report facts (ids, names, buses); every classification
//! happens here through the knowledge base (`cpu_db`, `gpu_db`, `codec_db`,
//! `device_db`, `chipset_db`) so that a scan, an imported report and a
//! manually edited profile end up with the same interpretation.

use std::path::Path;

use crate::contracts::{
    AudioDevice, ChassisInfo, CpuInfo, DetectedHardware, GpuInfo, InputDevice, InputKind,
    NetworkDevice, NetworkKind, StorageDevice,
};

use super::model::{
    CpuPlatform, CpuVendor, DeviceBus, FormFactor, GpuFamily, GpuVendor, HardwareProfile, InputBus,
    ProfileAudio, ProfileCpu, ProfileGpu, ProfileInput, ProfileNic, ProfileStorage, StorageKind,
    TouchpadVendor, VmKind,
};
use super::{acpi, chipset_db, codec_db, cpu_db, device_db, gpu_db};

/// SMBIOS chassis types (DMTF DSP0134 type 3).
const LAPTOP_CHASSIS: &[u32] = &[8, 9, 10, 11, 14, 30, 31, 32];
const ALL_IN_ONE_CHASSIS: &[u32] = &[13];
const MINI_PC_CHASSIS: &[u32] = &[35, 36];
const DESKTOP_CHASSIS: &[u32] = &[3, 4, 5, 6, 7, 15, 16, 17, 23, 24];

/// Build the canonical profile from a scan. Must never panic on partial data:
/// unknown fields stay Unknown/None and lower `scan_confidence`.
///
/// Rules: CPU via `cpu_db::identify`; GPUs via `gpu_db::identify` (ignore
/// virtual/remote display adapters and BMC chips on bare metal); audio = the
/// analog HDA codec (HDMI/DP codecs skipped, AppleALC-supported ones first);
/// ethernet = every wired NIC; wifi/bluetooth = the primary one (PCIe card
/// and supported chips first, Bluetooth from the same combo card); form
/// factor from chassis types (8,9,10,11,14,30,31,32 → laptop; 13 →
/// all-in-one; 35/36 → mini PC) + battery/lid + mobile CPU; chipset from the
/// LPC id then the board name; RAM in GB; VM from the hypervisor field.
pub fn build_profile(detected: &DetectedHardware) -> HardwareProfile {
    let c = &detected.cpu;
    let identity = cpu_db::identify(&c.name, &c.vendor, c.family, c.model, c.stepping);
    let vm = vm_kind(detected);
    let form_factor = if vm.is_some() {
        FormFactor::Desktop
    } else {
        form_factor(&detected.chassis, identity.is_mobile)
    };
    let gpus = gpus(&detected.gpus, vm.is_some());
    let wifi = primary_wifi(&detected.network);
    let bluetooth = primary_bluetooth(&detected.network, wifi.as_ref());
    let (motherboard_vendor, motherboard_model) = motherboard(detected);
    let acpi_facts = detected
        .acpi_tables_dir
        .as_deref()
        .filter(|dir| !dir.trim().is_empty())
        .and_then(|dir| match acpi::parse_tables(Path::new(dir)) {
            Ok(facts) => Some(facts),
            Err(err) => {
                tracing::warn!(%err, dir, "ACPI tables could not be parsed");
                None
            }
        });

    let mut profile = HardwareProfile {
        cpu: profile_cpu(c, &identity),
        form_factor,
        vm,
        gpus,
        audio: audio(&detected.audio),
        ethernet: ethernet(&detected.network),
        wifi,
        bluetooth,
        input: input(&detected.input, form_factor == FormFactor::Laptop),
        storage: detected.storage.iter().map(storage).collect(),
        motherboard_vendor,
        motherboard_model,
        chipset: None,
        ram_gb: ram_gb(detected.memory.total_mb),
        has_battery: detected.chassis.has_battery,
        firmware_uefi: detected.firmware.uefi,
        acpi: acpi_facts,
        acpi_tables_dir: detected.acpi_tables_dir.clone(),
        source: "scan".into(),
        scan_confidence: 0.0,
    };
    profile.chipset = chipset(detected, form_factor);
    profile.scan_confidence = scan_confidence(detected, &profile);
    profile
}

/// Re-derive classifications after the user edited the profile manually
/// (e.g. re-identify GPU families from ids, fill CPU flags for a newly chosen
/// platform). User-chosen values (platform, form factor, layout id, disabled
/// GPUs) are kept.
pub fn refresh_profile(profile: HardwareProfile) -> HardwareProfile {
    let mut p = profile;
    refresh_cpu(&mut p.cpu);
    for gpu in &mut p.gpus {
        refresh_gpu(gpu);
    }
    if let Some(audio) = p.audio.as_mut() {
        if audio.codec_id.is_none() {
            audio.codec_id = codec_db::find_codec_by_name(&audio.codec_name);
        }
        if audio.codec_name.trim().is_empty() {
            if let Some(id) = audio.codec_id {
                audio.codec_name = codec_db::codec_display_name(id, None);
            }
        }
    }
    for nic in p
        .ethernet
        .iter_mut()
        .chain(p.wifi.as_mut())
        .chain(p.bluetooth.as_mut())
    {
        nic.mac_address = nic.mac_address.as_deref().and_then(normalize_mac);
    }
    if p.input.touchpad_vendor.is_none() {
        p.input.touchpad_vendor = p
            .input
            .touchpad_hid
            .as_deref()
            .and_then(device_db::touchpad_vendor_from_hid);
    }
    let chipset_known = p.chipset.as_deref().is_some_and(|c| !c.trim().is_empty());
    if !chipset_known {
        p.chipset = chipset_db::from_board_name(&p.motherboard_model)
            .filter(|info| p.form_factor != FormFactor::Laptop || info.is_mobile)
            .map(|info| info.name);
    }
    p.scan_confidence = p.scan_confidence.clamp(0.0, 1.0);
    p
}

// ── CPU ─────────────────────────────────────────────────────────────────────

fn profile_cpu(c: &CpuInfo, identity: &cpu_db::CpuIdentity) -> ProfileCpu {
    let name = collapse_whitespace(&c.name);
    let features: Vec<String> = c.features.iter().map(|f| f.to_ascii_lowercase()).collect();
    let has = |flag: &str| features.iter().any(|f| f == flag);
    let known_flags = !features.is_empty();
    // AMD_Vanilla's core-count patch wants physical cores per package.
    let cores = if c.packages > 1 && c.cores >= c.packages {
        c.cores / c.packages
    } else {
        c.cores
    };
    ProfileCpu {
        name: if name.is_empty() {
            identity.codename.clone()
        } else {
            name
        },
        vendor: identity.vendor,
        platform: identity.platform,
        codename: identity.codename.clone(),
        family: c.family,
        model: c.model,
        stepping: c.stepping,
        cores,
        threads: c.threads,
        is_mobile: identity.is_mobile,
        has_avx2: Some(if known_flags {
            has("avx2")
        } else {
            identity.has_avx2
        }),
        has_sse4_2: if known_flags {
            Some(has("sse4_2"))
        } else {
            default_sse4_2(identity.platform)
        },
        is_hybrid: has("hybrid") || identity.is_hybrid,
    }
}

/// SSE4.2 by platform when CPUID flags are unavailable: Penryn and K10 lack it.
fn default_sse4_2(platform: CpuPlatform) -> Option<bool> {
    match platform {
        CpuPlatform::Penryn | CpuPlatform::AmdK10 => Some(false),
        CpuPlatform::Unknown | CpuPlatform::AppleSilicon => None,
        _ => Some(true),
    }
}

fn vendor_string(vendor: CpuVendor) -> &'static str {
    match vendor {
        CpuVendor::Intel => "GenuineIntel",
        CpuVendor::Amd => "AuthenticAMD",
        CpuVendor::Apple => "Apple",
        CpuVendor::Unknown => "",
    }
}

fn is_hybrid_platform(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::AlderLake
            | CpuPlatform::RaptorLake
            | CpuPlatform::ArrowLake
            | CpuPlatform::MeteorLake
            | CpuPlatform::LunarLake
    )
}

fn refresh_cpu(cpu: &mut ProfileCpu) {
    let auto = cpu_db::identify(
        &cpu.name,
        vendor_string(cpu.vendor),
        cpu.family,
        cpu.model,
        cpu.stepping,
    );
    if cpu.platform == CpuPlatform::Unknown && auto.platform != CpuPlatform::Unknown {
        cpu.platform = auto.platform;
        cpu.codename = auto.codename.clone();
        cpu.is_mobile = auto.is_mobile;
        cpu.is_hybrid = auto.is_hybrid;
    }
    let info = cpu_db::platform_info(cpu.platform);
    if cpu.platform != auto.platform {
        // The user picked another platform: values that only mirrored the old
        // automatic classification follow the new platform, values that
        // differ from it (CPUID facts, user edits) stay.
        let brand = cpu_db::identify(&cpu.name, vendor_string(cpu.vendor), None, None, None);
        let fits = brand.platform == cpu.platform;
        if info.vendor != CpuVendor::Unknown {
            cpu.vendor = info.vendor;
        }
        if cpu.codename.trim().is_empty() || cpu.codename == auto.codename {
            cpu.codename = if fits {
                brand.codename.clone()
            } else {
                info.label.to_string()
            };
        }
        if cpu.is_hybrid == auto.is_hybrid {
            cpu.is_hybrid = if fits {
                brand.is_hybrid
            } else {
                is_hybrid_platform(cpu.platform)
            };
        }
        let avx2 = if fits { brand.has_avx2 } else { info.has_avx2 };
        if cpu.has_avx2.is_none_or(|v| v == auto.has_avx2) {
            cpu.has_avx2 = Some(avx2);
        }
        let old_sse = default_sse4_2(auto.platform);
        if cpu.has_sse4_2.is_none() || cpu.has_sse4_2 == old_sse {
            cpu.has_sse4_2 = default_sse4_2(cpu.platform);
        }
    } else {
        if cpu.codename.trim().is_empty() {
            cpu.codename = auto.codename.clone();
        }
        if cpu.vendor == CpuVendor::Unknown {
            cpu.vendor = auto.vendor;
        }
        if cpu.has_avx2.is_none() {
            cpu.has_avx2 = Some(auto.has_avx2);
        }
        if cpu.has_sse4_2.is_none() {
            cpu.has_sse4_2 = default_sse4_2(cpu.platform);
        }
    }
    if cpu.name.trim().is_empty() {
        cpu.name = cpu.codename.clone();
    }
}

// ── Form factor / VM ────────────────────────────────────────────────────────

/// Chassis type first; battery + lid and a mobile CPU break ties. A mobile CPU
/// in a desktop or unknown chassis without battery is a NUC-style mini PC,
/// not a laptop (Dortania maps those to Macmini SMBIOS).
fn form_factor(chassis: &ChassisInfo, mobile_cpu: bool) -> FormFactor {
    let any = |set: &[u32]| chassis.chassis_types.iter().any(|t| set.contains(t));
    let portable_evidence = chassis.has_battery && chassis.has_lid;
    if any(LAPTOP_CHASSIS) {
        FormFactor::Laptop
    } else if any(ALL_IN_ONE_CHASSIS) {
        FormFactor::AllInOne
    } else if any(MINI_PC_CHASSIS) {
        FormFactor::MiniPc
    } else if any(DESKTOP_CHASSIS) {
        // Some laptops report a desktop chassis ("Default string" firmware).
        if portable_evidence || (chassis.has_lid && mobile_cpu) {
            FormFactor::Laptop
        } else if mobile_cpu {
            FormFactor::MiniPc
        } else {
            FormFactor::Desktop
        }
    } else if chassis.has_battery || chassis.has_lid {
        FormFactor::Laptop
    } else if mobile_cpu {
        FormFactor::MiniPc
    } else {
        FormFactor::Desktop
    }
}

fn vm_kind(detected: &DetectedHardware) -> Option<VmKind> {
    if let Some(name) = detected
        .hypervisor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let lower = name.to_ascii_lowercase();
        let kind = if lower.contains("kvm") || lower.contains("qemu") || lower.contains("proxmox") {
            VmKind::Kvm
        } else if lower.contains("vmware") {
            VmKind::Vmware
        } else if lower.contains("hyper-v") || lower.contains("microsoft") {
            VmKind::HyperV
        } else if lower.contains("virtualbox") || lower.contains("vbox") {
            VmKind::VirtualBox
        } else if lower.contains("parallels") {
            VmKind::Parallels
        } else {
            VmKind::Other
        };
        return Some(kind);
    }
    // Emulated display adapters only exist inside a VM.
    detected.gpus.iter().find_map(|g| {
        match g
            .vendor_id
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("15ad") => Some(VmKind::Vmware),
            Some("80ee") => Some(VmKind::VirtualBox),
            Some("1414") => Some(VmKind::HyperV),
            Some("1ab8") => Some(VmKind::Parallels),
            Some("1af4" | "1234" | "1b36") => Some(VmKind::Kvm),
            _ => None,
        }
    })
}

// ── GPUs ────────────────────────────────────────────────────────────────────

/// Server BMC / legacy VGA chips: never a macOS display path.
const BMC_GPU_VENDORS: &[&str] = &["1a03", "102b", "18ca"];

fn gpus(detected: &[GpuInfo], is_vm: bool) -> Vec<ProfileGpu> {
    let mut out: Vec<ProfileGpu> = Vec::new();
    for g in detected {
        let vendor_id = g.vendor_id.as_deref().and_then(hex4);
        let device_id = g.device_id.as_deref().and_then(hex4);
        let name = collapse_whitespace(&g.name);
        if vendor_id.is_none() && device_id.is_none() && name.is_empty() {
            continue;
        }
        let identity = gpu_db::identify_with_revision(
            vendor_id.as_deref(),
            device_id.as_deref(),
            g.revision.as_deref(),
            &name,
        );
        if !is_vm {
            let bmc = vendor_id
                .as_deref()
                .is_some_and(|v| BMC_GPU_VENDORS.contains(&v));
            if identity.vendor == GpuVendor::Virtual || bmc {
                continue;
            }
        }
        let pci_path = g.location.pci_path.clone().filter(|p| !p.is_empty());
        let duplicate = out.iter().any(|o| {
            o.vendor_id == vendor_id
                && o.device_id == device_id
                && o.pci_path == pci_path
                && pci_path.is_some()
        });
        if duplicate {
            continue;
        }
        let display = display_gpu_name(&name, identity.vendor, identity.model_name.as_deref());
        out.push(ProfileGpu {
            name: display,
            vendor: identity.vendor,
            family: identity.family,
            subsystem_id: subsystem(
                g.subsystem_vendor_id.as_deref(),
                g.subsystem_device_id.as_deref(),
            ),
            vendor_id,
            device_id,
            is_igpu: identity.is_igpu,
            pci_path,
            acpi_path: g.location.acpi_path.clone().filter(|p| !p.is_empty()),
            vram_mb: g.vram_mb,
            disabled: false,
        });
    }
    out
}

fn is_generic_gpu_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.is_empty()
        || lower.contains('[')
        || [
            "basic display",
            "basic render",
            "standard vga",
            "video controller",
            "display controller",
            "vga compatible",
            "3d controller",
        ]
        .iter()
        .any(|n| lower.contains(n))
}

fn vendor_prefix(vendor: GpuVendor) -> Option<&'static str> {
    match vendor {
        GpuVendor::Intel => Some("Intel"),
        GpuVendor::Amd => Some("AMD"),
        GpuVendor::Nvidia => Some("NVIDIA"),
        _ => None,
    }
}

/// Driver names (Windows) are kept; generic or pci.ids-style names are
/// replaced by the id table's model name.
fn display_gpu_name(scanned: &str, vendor: GpuVendor, model: Option<&str>) -> String {
    match model {
        Some(model) if is_generic_gpu_name(scanned) => match vendor_prefix(vendor) {
            Some(prefix)
                if !model
                    .to_ascii_lowercase()
                    .starts_with(&prefix.to_ascii_lowercase()) =>
            {
                format!("{prefix} {model}")
            }
            _ => model.to_string(),
        },
        _ if scanned.is_empty() => "Unknown GPU".to_string(),
        _ => scanned.to_string(),
    }
}

fn vendor_of_family(family: GpuFamily) -> GpuVendor {
    use GpuFamily::*;
    match family {
        IntelGma | IntelIronLake | IntelSandyBridge | IntelIvyBridge | IntelHaswell
        | IntelBroadwell | IntelSkylake | IntelKabyLake | IntelCoffeeLake | IntelCometLake
        | IntelIceLake | IntelLowPower | IntelXe | IntelArc => GpuVendor::Intel,
        NvidiaTesla | NvidiaFermi | NvidiaKepler | NvidiaMaxwell | NvidiaPascal | NvidiaModern => {
            GpuVendor::Nvidia
        }
        VirtualDisplay => GpuVendor::Virtual,
        Unknown => GpuVendor::Unknown,
        _ => GpuVendor::Amd,
    }
}

/// Families that only exist as integrated graphics.
fn family_is_igpu(family: GpuFamily) -> bool {
    use GpuFamily::*;
    matches!(
        family,
        IntelGma
            | IntelIronLake
            | IntelSandyBridge
            | IntelIvyBridge
            | IntelHaswell
            | IntelBroadwell
            | IntelSkylake
            | IntelKabyLake
            | IntelCoffeeLake
            | IntelCometLake
            | IntelIceLake
            | IntelLowPower
            | IntelXe
            | AmdApuVega
            | AmdApuRdna
            | AmdApuLegacy
    )
}

/// The ids decide the family when none is set or when the stored family
/// belongs to another vendor (the ids were edited). A family of the same
/// vendor that differs from the id table is the user's correction and is
/// kept: `gpu_db::support` honours a hand-picked family over the table.
fn refresh_gpu(gpu: &mut ProfileGpu) {
    gpu.vendor_id = gpu.vendor_id.as_deref().and_then(hex4);
    gpu.device_id = gpu.device_id.as_deref().and_then(hex4);
    let has_ids = gpu.vendor_id.is_some() && gpu.device_id.is_some();
    let identity = if has_ids || gpu.family == GpuFamily::Unknown {
        Some(gpu_db::identify(
            gpu.vendor_id.as_deref(),
            gpu.device_id.as_deref(),
            &gpu.name,
        ))
    } else {
        None
    };
    let user_family = gpu.family != GpuFamily::Unknown
        && identity
            .as_ref()
            .is_some_and(|id| id.family != gpu.family && id.vendor == vendor_of_family(gpu.family));
    let identity = identity.filter(|_| !user_family);
    match identity {
        Some(id) if id.family != GpuFamily::Unknown => {
            gpu.family = id.family;
            gpu.vendor = id.vendor;
            gpu.is_igpu = id.is_igpu;
            if gpu.name.trim().is_empty() {
                gpu.name = display_gpu_name("", id.vendor, id.model_name.as_deref());
            }
        }
        _ => {
            let vendor = vendor_of_family(gpu.family);
            if vendor != GpuVendor::Unknown {
                gpu.vendor = vendor;
            }
            if family_is_igpu(gpu.family) {
                gpu.is_igpu = true;
            }
        }
    }
    if gpu.name.trim().is_empty() {
        gpu.name = gpu_db::family_label(gpu.family).to_string();
    }
}

// ── Audio ───────────────────────────────────────────────────────────────────

/// The analog codec: HDMI/DP codecs are skipped; AppleALC-supported codecs
/// and plain HD Audio controllers win over DSP-mode (SST/SOF) ones.
fn audio(devices: &[AudioDevice]) -> Option<ProfileAudio> {
    let mut best: Option<((bool, bool), &AudioDevice, u32)> = None;
    for d in devices {
        if d.is_hdmi || !matches!(d.bus.as_str(), "hdaudio" | "sst") {
            continue;
        }
        let Some(id) = d
            .codec_vendor_id
            .as_deref()
            .zip(d.codec_device_id.as_deref())
            .and_then(|(v, dev)| codec_db::parse_codec_id(v, dev))
        else {
            continue;
        };
        if codec_db::is_hdmi_codec_id(id) {
            continue;
        }
        let rank = (codec_db::is_supported(id), d.bus == "hdaudio");
        if best.as_ref().is_none_or(|(r, _, _)| rank > *r) {
            best = Some((rank, d, id));
        }
    }
    let (_, d, id) = best?;
    let subsystem = d
        .codec_subsystem_id
        .as_deref()
        .and_then(codec_db::parse_subsystem);
    Some(ProfileAudio {
        codec_name: codec_db::codec_display_name(id, subsystem),
        codec_id: Some(id),
        controller_vendor_id: d.controller_vendor_id.as_deref().and_then(hex4),
        controller_device_id: d.controller_device_id.as_deref().and_then(hex4),
        controller_pci_path: d.location.pci_path.clone().filter(|p| !p.is_empty()),
        layout_id: None,
    })
}

// ── Network ─────────────────────────────────────────────────────────────────

fn device_bus(bus: &str) -> DeviceBus {
    match bus.trim().to_ascii_lowercase().as_str() {
        "pci" | "pcie" => DeviceBus::Pci,
        "usb" => DeviceBus::Usb,
        "sdio" => DeviceBus::Sdio,
        _ => DeviceBus::Unknown,
    }
}

fn nic(d: &NetworkDevice) -> ProfileNic {
    ProfileNic {
        name: collapse_whitespace(&d.name),
        bus: device_bus(&d.bus),
        vendor_id: d.vendor_id.as_deref().and_then(hex4),
        device_id: d.device_id.as_deref().and_then(hex4),
        subsystem_id: subsystem(
            d.subsystem_vendor_id.as_deref(),
            d.subsystem_device_id.as_deref(),
        ),
        pci_path: d.location.pci_path.clone().filter(|p| !p.is_empty()),
        mac_address: d.mac_address.as_deref().and_then(normalize_mac),
    }
}

/// Every wired NIC, PCI first (USB adapters after).
fn ethernet(devices: &[NetworkDevice]) -> Vec<ProfileNic> {
    let mut nics: Vec<ProfileNic> = devices
        .iter()
        .filter(|d| d.kind == NetworkKind::Ethernet)
        .map(nic)
        .collect();
    nics.sort_by_key(|n| n.bus != DeviceBus::Pci);
    nics
}

/// PCIe cards first, then cards macOS can drive.
fn primary_wifi(devices: &[NetworkDevice]) -> Option<ProfileNic> {
    devices
        .iter()
        .filter(|d| d.kind == NetworkKind::Wifi)
        .map(nic)
        .min_by_key(|n| {
            let unsupported = device_db::wifi_driver(n) == device_db::WifiDriver::Unsupported;
            (n.bus != DeviceBus::Pci, unsupported)
        })
}

/// USB controllers first (the module on the M.2 card), then supported ones,
/// then the one from the Wi-Fi card's vendor.
fn primary_bluetooth(devices: &[NetworkDevice], wifi: Option<&ProfileNic>) -> Option<ProfileNic> {
    let wifi_vendor = wifi.and_then(|w| w.vendor_id.as_deref());
    devices
        .iter()
        .filter(|d| d.kind == NetworkKind::Bluetooth)
        .map(nic)
        .min_by_key(|n| {
            let unsupported =
                device_db::bluetooth_driver(n) == device_db::BluetoothDriver::Unsupported;
            let other_maker = match (wifi_vendor, n.vendor_id.as_deref()) {
                (Some(w), Some(b)) => !same_radio_maker(w, b),
                _ => false,
            };
            (n.bus != DeviceBus::Usb, unsupported, other_maker)
        })
}

/// Wi-Fi PCI vendor and Bluetooth USB vendor of the same combo card
/// (Intel 8086/8087, Broadcom 14e4 with its USB ids and OEM module makers,
/// Realtek 10ec/0bda, MediaTek 14c3/0e8d, Qualcomm Atheros 168c/0cf3).
fn same_radio_maker(wifi_vendor: &str, bt_vendor: &str) -> bool {
    let usb: &[&str] = match wifi_vendor {
        "8086" => &["8087"],
        "14e4" => &[
            "0a5c", "0489", "13d3", "0930", "04ca", "105b", "413c", "0b05",
        ],
        "10ec" => &["0bda"],
        "14c3" => &["0e8d", "13d3", "0489"],
        "168c" | "17cb" => &["0cf3", "04ca", "13d3"],
        _ => &[],
    };
    usb.contains(&bt_vendor) || wifi_vendor == bt_vendor
}

// ── Input ───────────────────────────────────────────────────────────────────

fn input_bus(bus: &str) -> Option<InputBus> {
    match bus.trim().to_ascii_lowercase().as_str() {
        "ps2" => Some(InputBus::Ps2),
        "i2c" => Some(InputBus::I2c),
        "smbus" => Some(InputBus::Smbus),
        "usb" => Some(InputBus::Usb),
        _ => None,
    }
}

fn bus_rank(bus: Option<InputBus>) -> u8 {
    match bus {
        Some(InputBus::I2c) => 0,
        Some(InputBus::Smbus) => 1,
        Some(InputBus::Ps2) => 2,
        Some(InputBus::Usb) => 3,
        _ => 4,
    }
}

/// ACPI ids of I2C HID touchpads that the scanner could not classify (no
/// driver bound): Synaptics, ELAN touchpads (ELAN0xxx/1xxx; 2xxx/9xxx are
/// touchscreens), ALPS, FocalTech, Cypress and Microsoft precision touchpads.
fn is_touchpad_hid(hid: &str) -> bool {
    let h = hid.trim().to_ascii_uppercase();
    h.starts_with("SYN")
        || h.starts_with("ALP")
        || h.starts_with("ETD")
        || h.starts_with("ELAN0")
        || h.starts_with("ELAN1")
        || h.starts_with("CYAP")
        || h.starts_with("FTCS")
        || h.starts_with("FTE")
        || h.starts_with("GXTP")
        || h == "MSFT0001"
}

fn is_touchscreen_hid(hid: &str) -> bool {
    let h = hid.trim().to_ascii_uppercase();
    ["WCOM", "ATML", "ELAN2", "ELAN9", "GDIX", "NTRG"]
        .iter()
        .any(|p| h.starts_with(p))
}

fn touchpad_vendor(vendor: Option<&str>, hid: Option<&str>) -> Option<TouchpadVendor> {
    let from_name = vendor.map(|v| match v.trim().to_ascii_lowercase().as_str() {
        "synaptics" => TouchpadVendor::Synaptics,
        "elan" => TouchpadVendor::Elan,
        "alps" => TouchpadVendor::Alps,
        _ => TouchpadVendor::Other,
    });
    match from_name {
        Some(TouchpadVendor::Other) | None => hid
            .and_then(device_db::touchpad_vendor_from_hid)
            .or(from_name),
        known => known,
    }
}

/// Keyboard bus, touchpad and touchscreen. On laptops a built-in PS/2
/// pointing device the scanner could only call a mouse ("PS/2 Compatible
/// Mouse", OEM PnP ids like DLL/LEN/HPQ) is the touchpad when nothing else is.
fn input(devices: &[InputDevice], laptop: bool) -> ProfileInput {
    let builtin = |d: &&InputDevice| d.bus != "bluetooth";
    let keyboard_bus = devices
        .iter()
        .filter(builtin)
        .filter(|d| d.kind == InputKind::Keyboard)
        .filter_map(|d| input_bus(&d.bus))
        .min_by_key(|b| match b {
            InputBus::Ps2 => 0,
            InputBus::I2c | InputBus::Smbus => 1,
            InputBus::Usb => 2,
            InputBus::Unknown => 3,
        })
        .unwrap_or_default();
    let is_touchpad = |d: &&InputDevice| {
        d.kind == InputKind::Touchpad
            || (d.kind == InputKind::Other
                && d.bus == "i2c"
                && d.hardware_id.as_deref().is_some_and(is_touchpad_hid))
    };
    let touchpad = devices
        .iter()
        .filter(builtin)
        .filter(is_touchpad)
        .min_by_key(|d| bus_rank(input_bus(&d.bus)))
        .or_else(|| {
            devices
                .iter()
                .filter(|d| laptop && d.kind == InputKind::Mouse && d.bus == "ps2")
                .min_by_key(|d| d.hardware_id.is_none())
        });
    let has_touchscreen = devices.iter().filter(builtin).any(|d| {
        d.kind == InputKind::Touchscreen
            || (d.kind == InputKind::Other
                && d.bus == "i2c"
                && d.hardware_id.as_deref().is_some_and(is_touchscreen_hid))
    });
    ProfileInput {
        keyboard_bus,
        touchpad_bus: touchpad.map(|t| input_bus(&t.bus).unwrap_or_default()),
        touchpad_vendor: touchpad
            .and_then(|t| touchpad_vendor(t.vendor.as_deref(), t.hardware_id.as_deref())),
        touchpad_hid: touchpad
            .and_then(|t| t.hardware_id.clone())
            .map(|h| h.trim().to_ascii_uppercase())
            .filter(|h| !h.is_empty()),
        has_touchscreen,
    }
}

// ── Storage / board ─────────────────────────────────────────────────────────

fn storage(d: &StorageDevice) -> ProfileStorage {
    let kind = match d.kind.trim().to_ascii_lowercase().as_str() {
        "nvme" => StorageKind::Nvme,
        "sata" => StorageKind::Sata,
        "raid" => StorageKind::Raid,
        "emmc" => StorageKind::Emmc,
        "usb" => StorageKind::Usb,
        _ => StorageKind::Other,
    };
    ProfileStorage {
        name: collapse_whitespace(&d.name),
        kind,
        vendor_id: d.controller_vendor_id.as_deref().and_then(hex4),
        device_id: d.controller_device_id.as_deref().and_then(hex4),
        size_bytes: d.size_bytes,
    }
}

fn clean(value: Option<&String>) -> Option<String> {
    value
        .map(|v| collapse_whitespace(v))
        .filter(|v| !v.is_empty())
}

/// Baseboard first, system (OEM) strings as fallback.
fn motherboard(detected: &DetectedHardware) -> (String, String) {
    let mb = &detected.motherboard;
    let vendor = clean(mb.manufacturer.as_ref())
        .or_else(|| clean(mb.system_manufacturer.as_ref()))
        .unwrap_or_default();
    let model = clean(mb.product.as_ref())
        .or_else(|| clean(mb.system_product.as_ref()))
        .unwrap_or_default();
    (vendor, model)
}

/// LPC/eSPI id first, then board and system names. A desktop chipset that
/// only comes from a name is not trusted on a laptop (ASUS "X570ZD" is a
/// Ryzen laptop model, not an X570 board).
fn chipset(detected: &DetectedHardware, form_factor: FormFactor) -> Option<String> {
    let mb = &detected.motherboard;
    let lpc = mb.lpc_vendor_id.as_deref().zip(mb.lpc_device_id.as_deref());
    let from_lpc = lpc.and_then(|(v, d)| chipset_db::from_lpc(v, d));
    let board = clean(mb.product.as_ref());
    let system = clean(mb.system_product.as_ref());
    let resolved = chipset_db::resolve(lpc, board.as_deref())
        .or_else(|| chipset_db::resolve(None, system.as_deref()))
        .or_else(|| mb.chipset.as_deref().and_then(chipset_db::from_name));
    let resolved = match resolved {
        Some(info)
            if form_factor == FormFactor::Laptop
                && !info.is_mobile
                && from_lpc.as_ref().is_none_or(|l| l.name != info.name) =>
        {
            from_lpc
        }
        other => other,
    };
    resolved.map(|info| info.name)
}

fn ram_gb(total_mb: u64) -> u32 {
    // Usable memory is a little below the installed amount (firmware and
    // iGPU reservations), so round to the nearest GB.
    u32::try_from((total_mb + 512) / 1024).unwrap_or(u32::MAX)
}

/// Completeness of the facts the planner relies on most.
fn scan_confidence(detected: &DetectedHardware, profile: &HardwareProfile) -> f64 {
    let is_vm = profile.vm.is_some();
    let cpu = match (
        detected.cpu.family.is_some() && detected.cpu.model.is_some(),
        profile.cpu.platform != CpuPlatform::Unknown,
    ) {
        (true, true) => 0.30,
        (false, true) => 0.15,
        _ => 0.0,
    };
    let gpu = if profile.gpus.is_empty() {
        if is_vm {
            0.15
        } else {
            0.0
        }
    } else if profile
        .gpus
        .iter()
        .all(|g| g.vendor_id.is_some() && g.device_id.is_some())
    {
        0.25
    } else {
        0.10
    };
    let chassis_known = detected
        .chassis
        .chassis_types
        .iter()
        .any(|t| !matches!(t, 1 | 2));
    let chassis = if chassis_known || is_vm { 0.15 } else { 0.0 };
    let lpc = if detected.motherboard.lpc_device_id.is_some() || profile.chipset.is_some() || is_vm
    {
        0.15
    } else {
        0.0
    };
    let codec = if profile.audio.is_some() || is_vm {
        0.15
    } else {
        0.0
    };
    let total: f64 = cpu + gpu + chassis + lpc + codec;
    (total * 100.0).round() / 100.0
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn collapse_whitespace(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c != '\0')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase 4-digit hex id ("0x8086", "8086", "10EC") or None.
fn hex4(value: &str) -> Option<String> {
    let v = value.trim();
    let v = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);
    if v.is_empty() || v.len() > 4 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("{:0>4}", v.to_ascii_lowercase()))
}

/// Subsystem as "vvvvdddd" (vendor first).
fn subsystem(vendor: Option<&str>, device: Option<&str>) -> Option<String> {
    Some(format!("{}{}", hex4(vendor?)?, hex4(device?)?))
}

/// "AA:BB:CC:DD:EE:FF"; all-zero, broadcast and malformed addresses are dropped.
fn normalize_mac(value: &str) -> Option<String> {
    let hex: String = value.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let separators_ok = value
        .chars()
        .filter(|c| !c.is_ascii_hexdigit())
        .all(|c| matches!(c, ':' | '-' | '.' | ' '));
    if hex.len() != 12 || !separators_ok {
        return None;
    }
    let upper = hex.to_ascii_uppercase();
    if upper.chars().all(|c| c == '0') || upper.chars().all(|c| c == 'F') {
        return None;
    }
    Some(
        upper
            .as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).into_owned())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Realistic scans shared by the interpretation tests.

    use crate::contracts::*;

    fn loc(pci: &str, acpi: Option<&str>) -> PciLocation {
        PciLocation {
            pci_path: Some(pci.into()),
            acpi_path: acpi.map(Into::into),
        }
    }

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    /// Windows 11 on an ASUS Z390 board: i9-9900K, UHD 630 + RX 580,
    /// ALC1220, I219-V, USB input, NVMe + SATA.
    pub fn windows_z390() -> DetectedHardware {
        DetectedHardware {
            host_os: "windows".into(),
            cpu: CpuInfo {
                name: "Intel(R) Core(TM) i9-9900K CPU @ 3.60GHz".into(),
                vendor: "GenuineIntel".into(),
                family: Some(6),
                model: Some(0x9E),
                stepping: Some(13),
                cores: 8,
                threads: 16,
                packages: 1,
                base_clock_mhz: Some(3600),
                features: ["sse3", "ssse3", "sse4_1", "sse4_2", "avx", "avx2", "vmx"]
                    .iter()
                    .map(|f| f.to_string())
                    .collect(),
            },
            gpus: vec![
                GpuInfo {
                    name: "Intel(R) UHD Graphics 630".into(),
                    vendor_id: s("8086"),
                    device_id: s("3e98"),
                    subsystem_vendor_id: s("1043"),
                    subsystem_device_id: s("8694"),
                    revision: s("02"),
                    vram_mb: None,
                    location: loc("PciRoot(0x0)/Pci(0x2,0x0)", Some("\\_SB.PCI0.GFX0")),
                },
                GpuInfo {
                    name: "Radeon RX 580 Series".into(),
                    vendor_id: s("1002"),
                    device_id: s("67df"),
                    subsystem_vendor_id: s("1da2"),
                    subsystem_device_id: s("e366"),
                    revision: s("e7"),
                    vram_mb: Some(8192),
                    location: loc(
                        "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)",
                        Some("\\_SB.PCI0.PEG0.PEGP"),
                    ),
                },
            ],
            audio: vec![
                AudioDevice {
                    name: "Intel(R) Display Audio".into(),
                    codec_vendor_id: s("8086"),
                    codec_device_id: s("280b"),
                    codec_subsystem_id: s("80860101"),
                    controller_vendor_id: s("8086"),
                    controller_device_id: s("a348"),
                    location: loc("PciRoot(0x0)/Pci(0x1f,0x3)", Some("\\_SB.PCI0.HDAS")),
                    is_hdmi: true,
                    bus: "hdaudio".into(),
                },
                AudioDevice {
                    name: "Realtek High Definition Audio".into(),
                    codec_vendor_id: s("10ec"),
                    codec_device_id: s("1220"),
                    codec_subsystem_id: s("10438724"),
                    controller_vendor_id: s("8086"),
                    controller_device_id: s("a348"),
                    location: loc("PciRoot(0x0)/Pci(0x1f,0x3)", Some("\\_SB.PCI0.HDAS")),
                    is_hdmi: false,
                    bus: "hdaudio".into(),
                },
                AudioDevice {
                    name: "AMD High Definition Audio Device".into(),
                    codec_vendor_id: s("1002"),
                    codec_device_id: s("aa01"),
                    codec_subsystem_id: s("00aa0100"),
                    controller_vendor_id: s("1002"),
                    controller_device_id: s("aaf0"),
                    location: loc("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x1)", None),
                    is_hdmi: true,
                    bus: "hdaudio".into(),
                },
            ],
            network: vec![NetworkDevice {
                name: "Intel(R) Ethernet Connection (7) I219-V".into(),
                kind: NetworkKind::Ethernet,
                bus: "pci".into(),
                vendor_id: s("8086"),
                device_id: s("15bc"),
                subsystem_vendor_id: s("1043"),
                subsystem_device_id: s("8672"),
                mac_address: s("04:92:26:aa:bb:cc"),
                location: loc("PciRoot(0x0)/Pci(0x1f,0x6)", Some("\\_SB.PCI0.GLAN")),
            }],
            input: vec![
                InputDevice {
                    name: "HID Keyboard Device".into(),
                    kind: InputKind::Keyboard,
                    bus: "usb".into(),
                    hardware_id: None,
                    vendor: None,
                },
                InputDevice {
                    name: "HID-compliant mouse".into(),
                    kind: InputKind::Mouse,
                    bus: "usb".into(),
                    hardware_id: None,
                    vendor: None,
                },
            ],
            memory: MemoryInfo { total_mb: 32768 },
            motherboard: MotherboardInfo {
                manufacturer: s("ASUSTeK COMPUTER INC."),
                product: s("ROG STRIX Z390-E GAMING"),
                system_manufacturer: None,
                system_product: None,
                lpc_vendor_id: s("8086"),
                lpc_device_id: s("a305"),
                chipset: None,
            },
            storage: vec![
                StorageDevice {
                    name: "Samsung SSD 970 EVO Plus 1TB".into(),
                    kind: "nvme".into(),
                    controller_vendor_id: s("144d"),
                    controller_device_id: s("a808"),
                    size_bytes: Some(1_000_204_886_016),
                },
                StorageDevice {
                    name: "ST2000DM008-2FR102".into(),
                    kind: "sata".into(),
                    controller_vendor_id: s("8086"),
                    controller_device_id: s("a352"),
                    size_bytes: Some(2_000_398_934_016),
                },
            ],
            usb_controllers: vec![],
            chassis: ChassisInfo {
                chassis_types: vec![3],
                manufacturer: s("Default string"),
                has_battery: false,
                has_lid: false,
            },
            firmware: FirmwareInfo {
                uefi: Some(true),
                secure_boot: Some(true),
                bios_vendor: s("American Megatrends Inc."),
                bios_version: s("1802"),
                bios_date: s("2021-05-11"),
            },
            acpi_tables_dir: None,
            hypervisor: None,
            warnings: vec![],
        }
    }

    /// Ubuntu on a Dell XPS 13 9380: i7-8565U (Whiskey Lake), UHD 620,
    /// ALC3271 (ALC271X), Killer/Intel 9560-class CNVi Wi-Fi + USB BT,
    /// PS/2 keyboard, Synaptics I2C touchpad without a bound driver, PM981.
    pub fn linux_laptop_i2c() -> DetectedHardware {
        DetectedHardware {
            host_os: "linux".into(),
            cpu: CpuInfo {
                name: "Intel(R) Core(TM) i7-8565U CPU @ 1.80GHz".into(),
                vendor: "GenuineIntel".into(),
                family: Some(6),
                model: Some(0x8E),
                stepping: Some(11),
                cores: 4,
                threads: 8,
                packages: 1,
                base_clock_mhz: Some(1800),
                features: ["sse4_1", "sse4_2", "avx", "avx2", "vmx"]
                    .iter()
                    .map(|f| f.to_string())
                    .collect(),
            },
            gpus: vec![GpuInfo {
                name: "WhiskeyLake-U GT2 [UHD Graphics 620]".into(),
                vendor_id: s("8086"),
                device_id: s("3ea0"),
                subsystem_vendor_id: s("1028"),
                subsystem_device_id: s("08e1"),
                revision: s("02"),
                vram_mb: None,
                location: loc("PciRoot(0x0)/Pci(0x2,0x0)", Some("\\_SB.PCI0.GFX0")),
            }],
            audio: vec![
                AudioDevice {
                    name: "Realtek ALC3271".into(),
                    codec_vendor_id: s("10ec"),
                    codec_device_id: s("0299"),
                    codec_subsystem_id: s("102808e1"),
                    controller_vendor_id: s("8086"),
                    controller_device_id: s("9dc8"),
                    location: loc("PciRoot(0x0)/Pci(0x1f,0x3)", None),
                    is_hdmi: false,
                    bus: "hdaudio".into(),
                },
                AudioDevice {
                    name: "Intel Kabylake HDMI".into(),
                    codec_vendor_id: s("8086"),
                    codec_device_id: s("280b"),
                    codec_subsystem_id: s("80860101"),
                    controller_vendor_id: s("8086"),
                    controller_device_id: s("9dc8"),
                    location: loc("PciRoot(0x0)/Pci(0x1f,0x3)", None),
                    is_hdmi: true,
                    bus: "hdaudio".into(),
                },
            ],
            network: vec![
                NetworkDevice {
                    name: "Cannon Point-LP CNVi [Wireless-AC]".into(),
                    kind: NetworkKind::Wifi,
                    bus: "pci".into(),
                    vendor_id: s("8086"),
                    device_id: s("9df0"),
                    subsystem_vendor_id: s("8086"),
                    subsystem_device_id: s("0034"),
                    mac_address: s("a4:c3:f0:11:22:33"),
                    location: loc("PciRoot(0x0)/Pci(0x14,0x3)", None),
                },
                NetworkDevice {
                    name: "Bluetooth 9460/9560".into(),
                    kind: NetworkKind::Bluetooth,
                    bus: "usb".into(),
                    vendor_id: s("8087"),
                    device_id: s("0aaa"),
                    ..Default::default()
                },
            ],
            input: vec![
                InputDevice {
                    name: "AT Translated Set 2 keyboard".into(),
                    kind: InputKind::Keyboard,
                    bus: "ps2".into(),
                    hardware_id: Some("PNP0303".into()),
                    vendor: None,
                },
                InputDevice {
                    name: "PS/2 Generic Mouse".into(),
                    kind: InputKind::Mouse,
                    bus: "ps2".into(),
                    hardware_id: Some("DLL08AF".into()),
                    vendor: None,
                },
                InputDevice {
                    name: "i2c-SYNA3602:00".into(),
                    kind: InputKind::Other,
                    bus: "i2c".into(),
                    hardware_id: Some("SYNA3602".into()),
                    vendor: Some("synaptics".into()),
                },
            ],
            memory: MemoryInfo { total_mb: 15_872 },
            motherboard: MotherboardInfo {
                manufacturer: s("Dell Inc."),
                product: s("0KTW76"),
                system_manufacturer: s("Dell Inc."),
                system_product: s("XPS 13 9380"),
                lpc_vendor_id: s("8086"),
                lpc_device_id: s("9d84"),
                chipset: None,
            },
            storage: vec![StorageDevice {
                name: "PM981 NVMe Samsung 512GB".into(),
                kind: "nvme".into(),
                controller_vendor_id: s("144d"),
                controller_device_id: s("a808"),
                size_bytes: Some(512_110_190_592),
            }],
            usb_controllers: vec![],
            chassis: ChassisInfo {
                chassis_types: vec![10],
                manufacturer: s("Dell Inc."),
                has_battery: true,
                has_lid: true,
            },
            firmware: FirmwareInfo {
                uefi: Some(true),
                secure_boot: Some(false),
                bios_vendor: s("Dell Inc."),
                bios_version: s("1.20.0"),
                bios_date: s("06/08/2022"),
            },
            acpi_tables_dir: None,
            hypervisor: None,
            warnings: vec![],
        }
    }

    /// Windows on an MSI B550 board: Ryzen 5 5600X, RX 6600 XT, ALCS1200A,
    /// RTL8125B, Intel AX200 + BT, NVMe.
    pub fn amd_b550() -> DetectedHardware {
        DetectedHardware {
            host_os: "windows".into(),
            cpu: CpuInfo {
                name: "AMD Ryzen 5 5600X 6-Core Processor".into(),
                vendor: "AuthenticAMD".into(),
                family: Some(0x19),
                model: Some(0x21),
                stepping: Some(0),
                cores: 6,
                threads: 12,
                packages: 1,
                base_clock_mhz: Some(3700),
                features: ["sse4_1", "sse4_2", "sse4a", "avx", "avx2", "svm"]
                    .iter()
                    .map(|f| f.to_string())
                    .collect(),
            },
            gpus: vec![GpuInfo {
                name: "AMD Radeon RX 6600 XT".into(),
                vendor_id: s("1002"),
                device_id: s("73ff"),
                subsystem_vendor_id: s("1da2"),
                subsystem_device_id: s("e448"),
                revision: s("c1"),
                vram_mb: Some(8192),
                location: loc(
                    "PciRoot(0x0)/Pci(0x3,0x1)/Pci(0x0,0x0)/Pci(0x0,0x0)/Pci(0x0,0x0)",
                    None,
                ),
            }],
            audio: vec![
                AudioDevice {
                    name: "Realtek High Definition Audio".into(),
                    codec_vendor_id: s("10ec"),
                    codec_device_id: s("0b00"),
                    codec_subsystem_id: s("1462cb99"),
                    controller_vendor_id: s("1022"),
                    controller_device_id: s("1487"),
                    location: loc("PciRoot(0x0)/Pci(0x8,0x1)/Pci(0x0,0x4)", None),
                    is_hdmi: false,
                    bus: "hdaudio".into(),
                },
                AudioDevice {
                    name: "AMD High Definition Audio Device".into(),
                    codec_vendor_id: s("1002"),
                    codec_device_id: s("aa01"),
                    controller_vendor_id: s("1002"),
                    controller_device_id: s("ab28"),
                    is_hdmi: true,
                    bus: "hdaudio".into(),
                    ..Default::default()
                },
            ],
            network: vec![
                NetworkDevice {
                    name: "Realtek Gaming 2.5GbE Family Controller".into(),
                    kind: NetworkKind::Ethernet,
                    bus: "pci".into(),
                    vendor_id: s("10ec"),
                    device_id: s("8125"),
                    subsystem_vendor_id: s("1462"),
                    subsystem_device_id: s("7c91"),
                    mac_address: s("00-D8-61-12-34-56"),
                    location: loc(
                        "PciRoot(0x0)/Pci(0x2,0x1)/Pci(0x0,0x2)/Pci(0x9,0x0)/Pci(0x0,0x0)",
                        None,
                    ),
                },
                NetworkDevice {
                    name: "Intel(R) Wi-Fi 6 AX200 160MHz".into(),
                    kind: NetworkKind::Wifi,
                    bus: "pci".into(),
                    vendor_id: s("8086"),
                    device_id: s("2723"),
                    subsystem_vendor_id: s("8086"),
                    subsystem_device_id: s("0084"),
                    mac_address: s("50:e0:85:aa:bb:cc"),
                    location: loc(
                        "PciRoot(0x0)/Pci(0x2,0x1)/Pci(0x0,0x2)/Pci(0x8,0x0)/Pci(0x0,0x0)",
                        None,
                    ),
                },
                NetworkDevice {
                    name: "Intel(R) Wireless Bluetooth(R)".into(),
                    kind: NetworkKind::Bluetooth,
                    bus: "usb".into(),
                    vendor_id: s("8087"),
                    device_id: s("0029"),
                    ..Default::default()
                },
            ],
            input: vec![InputDevice {
                name: "HID Keyboard Device".into(),
                kind: InputKind::Keyboard,
                bus: "usb".into(),
                hardware_id: None,
                vendor: None,
            }],
            memory: MemoryInfo { total_mb: 16384 },
            motherboard: MotherboardInfo {
                manufacturer: s("Micro-Star International Co., Ltd."),
                product: s("MAG B550 TOMAHAWK (MS-7C91)"),
                system_manufacturer: s("Micro-Star International Co., Ltd."),
                system_product: s("MS-7C91"),
                lpc_vendor_id: s("1022"),
                lpc_device_id: s("790e"),
                chipset: None,
            },
            storage: vec![StorageDevice {
                name: "WDS100T3X0C-00SJG0".into(),
                kind: "nvme".into(),
                controller_vendor_id: s("15b7"),
                controller_device_id: s("5006"),
                size_bytes: Some(1_000_204_886_016),
            }],
            usb_controllers: vec![],
            chassis: ChassisInfo {
                chassis_types: vec![3],
                manufacturer: s("Micro-Star International Co., Ltd."),
                has_battery: false,
                has_lid: false,
            },
            firmware: FirmwareInfo {
                uefi: Some(true),
                secure_boot: Some(false),
                bios_vendor: s("American Megatrends International, LLC."),
                bios_version: s("1.B0"),
                bios_date: s("2024-01-15"),
            },
            acpi_tables_dir: None,
            hypervisor: None,
            warnings: vec![],
        }
    }

    /// Linux guest on QEMU/KVM (q35): Skylake CPU model, Bochs VGA, virtio-net.
    pub fn kvm_guest() -> DetectedHardware {
        DetectedHardware {
            host_os: "linux".into(),
            cpu: CpuInfo {
                name: "Intel Core Processor (Skylake, IBRS)".into(),
                vendor: "GenuineIntel".into(),
                family: Some(6),
                model: Some(0x5E),
                stepping: Some(3),
                cores: 4,
                threads: 4,
                packages: 1,
                base_clock_mhz: None,
                features: ["sse4_1", "sse4_2", "avx", "avx2", "hypervisor"]
                    .iter()
                    .map(|f| f.to_string())
                    .collect(),
            },
            gpus: vec![GpuInfo {
                name: "QEMU Standard VGA".into(),
                vendor_id: s("1234"),
                device_id: s("1111"),
                location: loc("PciRoot(0x0)/Pci(0x1,0x0)", None),
                ..Default::default()
            }],
            audio: vec![],
            network: vec![NetworkDevice {
                name: "Virtio network device".into(),
                kind: NetworkKind::Ethernet,
                bus: "pci".into(),
                vendor_id: s("1af4"),
                device_id: s("1000"),
                mac_address: s("52:54:00:12:34:56"),
                location: loc("PciRoot(0x0)/Pci(0x2,0x0)", None),
                ..Default::default()
            }],
            input: vec![],
            memory: MemoryInfo { total_mb: 8_000 },
            motherboard: MotherboardInfo {
                manufacturer: s("QEMU"),
                product: s("Standard PC (Q35 + ICH9, 2009)"),
                system_manufacturer: s("QEMU"),
                system_product: s("Standard PC (Q35 + ICH9, 2009)"),
                lpc_vendor_id: s("8086"),
                lpc_device_id: s("2918"),
                chipset: None,
            },
            storage: vec![StorageDevice {
                name: "QEMU HARDDISK".into(),
                kind: "sata".into(),
                controller_vendor_id: s("8086"),
                controller_device_id: s("2922"),
                size_bytes: Some(128_849_018_880),
            }],
            usb_controllers: vec![],
            chassis: ChassisInfo {
                chassis_types: vec![1],
                manufacturer: s("QEMU"),
                has_battery: false,
                has_lid: false,
            },
            firmware: FirmwareInfo {
                uefi: Some(true),
                ..Default::default()
            },
            acpi_tables_dir: None,
            hypervisor: s("KVM"),
            warnings: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::contracts::{ChassisInfo, DetectedHardware};

    #[test]
    fn windows_z390_desktop() {
        let p = build_profile(&windows_z390());
        assert_eq!(p.cpu.vendor, CpuVendor::Intel);
        assert_eq!(p.cpu.platform, CpuPlatform::CoffeeLake);
        assert_eq!((p.cpu.cores, p.cpu.threads), (8, 16));
        assert_eq!(p.cpu.has_avx2, Some(true));
        assert_eq!(p.cpu.has_sse4_2, Some(true));
        assert!(!p.cpu.is_mobile && !p.cpu.is_hybrid);
        assert_eq!(p.form_factor, FormFactor::Desktop);
        assert_eq!(p.vm, None);
        assert_eq!(p.gpus.len(), 2);
        assert!(p.gpus[0].is_igpu);
        assert_eq!(p.gpus[0].family, GpuFamily::IntelCoffeeLake);
        assert_eq!(
            p.gpus[0].pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x2,0x0)")
        );
        assert_eq!(p.gpus[1].family, GpuFamily::AmdPolaris);
        assert!(!p.gpus[1].is_igpu);
        assert_eq!(p.gpus[1].name, "Radeon RX 580 Series");
        assert_eq!(p.gpus[1].subsystem_id.as_deref(), Some("1da2e366"));
        let audio = p.audio.expect("analog codec");
        assert_eq!(audio.codec_id, Some(0x10EC_1220));
        assert!(audio.codec_name.contains("1220"), "{}", audio.codec_name);
        assert_eq!(
            audio.controller_pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        assert_eq!(p.ethernet.len(), 1);
        assert_eq!(
            p.ethernet[0].mac_address.as_deref(),
            Some("04:92:26:AA:BB:CC")
        );
        assert_eq!(p.ethernet[0].bus, DeviceBus::Pci);
        assert!(p.wifi.is_none() && p.bluetooth.is_none());
        assert_eq!(p.input.keyboard_bus, InputBus::Usb);
        assert_eq!(p.input.touchpad_bus, None);
        assert_eq!(p.storage.len(), 2);
        assert_eq!(p.storage[0].kind, StorageKind::Nvme);
        assert_eq!(p.motherboard_vendor, "ASUSTeK COMPUTER INC.");
        assert_eq!(p.motherboard_model, "ROG STRIX Z390-E GAMING");
        assert_eq!(p.chipset.as_deref(), Some("Z390"));
        assert_eq!(p.ram_gb, 32);
        assert_eq!(p.firmware_uefi, Some(true));
        assert_eq!(p.source, "scan");
        assert!(p.scan_confidence > 0.95, "{}", p.scan_confidence);
    }

    #[test]
    fn linux_laptop_with_unbound_i2c_touchpad() {
        let p = build_profile(&linux_laptop_i2c());
        assert_eq!(p.cpu.platform, CpuPlatform::CoffeeLake);
        assert!(p.cpu.codename.starts_with("Whiskey"), "{}", p.cpu.codename);
        assert!(p.cpu.is_mobile);
        assert_eq!(p.form_factor, FormFactor::Laptop);
        assert!(p.has_battery);
        assert_eq!(p.gpus.len(), 1);
        assert_eq!(p.gpus[0].family, GpuFamily::IntelCoffeeLake);
        assert_eq!(p.gpus[0].name, "Intel UHD Graphics 620");
        let audio = p.audio.expect("codec");
        assert_eq!(audio.codec_id, Some(0x10EC_0299));
        assert_eq!(p.input.keyboard_bus, InputBus::Ps2);
        assert_eq!(p.input.touchpad_bus, Some(InputBus::I2c));
        assert_eq!(p.input.touchpad_vendor, Some(TouchpadVendor::Synaptics));
        assert_eq!(p.input.touchpad_hid.as_deref(), Some("SYNA3602"));
        assert!(!p.input.has_touchscreen);
        let wifi = p.wifi.expect("wifi");
        assert_eq!(wifi.device_id.as_deref(), Some("9df0"));
        assert_eq!(wifi.mac_address.as_deref(), Some("A4:C3:F0:11:22:33"));
        let bt = p.bluetooth.expect("bt");
        assert_eq!(
            (bt.bus, bt.device_id.as_deref()),
            (DeviceBus::Usb, Some("0aaa"))
        );
        assert!(p.ethernet.is_empty());
        assert_eq!(p.motherboard_model, "0KTW76");
        // Family-level LPC match from the mobile PCH.
        assert_eq!(p.chipset.as_deref(), Some("Cannon Point-LP"));
        assert_eq!(p.ram_gb, 16);
        assert!(p.scan_confidence > 0.95, "{}", p.scan_confidence);
    }

    #[test]
    fn amd_b550_desktop() {
        let p = build_profile(&amd_b550());
        assert_eq!(p.cpu.vendor, CpuVendor::Amd);
        assert_eq!(p.cpu.platform, CpuPlatform::AmdZen3);
        assert_eq!(p.cpu.cores, 6);
        assert_eq!(p.form_factor, FormFactor::Desktop);
        assert_eq!(p.gpus.len(), 1);
        assert_eq!(p.gpus[0].family, GpuFamily::AmdNavi23);
        let audio = p.audio.expect("codec");
        assert_eq!(audio.codec_id, Some(0x10EC_0B00));
        assert_eq!(audio.controller_device_id.as_deref(), Some("1487"));
        assert_eq!(p.ethernet[0].device_id.as_deref(), Some("8125"));
        assert_eq!(
            p.ethernet[0].mac_address.as_deref(),
            Some("00:D8:61:12:34:56")
        );
        assert_eq!(
            p.wifi.as_ref().and_then(|w| w.device_id.as_deref()),
            Some("2723")
        );
        assert_eq!(
            p.bluetooth.as_ref().and_then(|b| b.device_id.as_deref()),
            Some("0029")
        );
        // The FCH LPC bridge names no chipset; the board name does.
        assert_eq!(p.chipset.as_deref(), Some("B550"));
        assert_eq!(p.ram_gb, 16);
    }

    #[test]
    fn kvm_guest_is_a_vm() {
        let p = build_profile(&kvm_guest());
        assert_eq!(p.vm, Some(VmKind::Kvm));
        assert_eq!(p.form_factor, FormFactor::Desktop);
        assert_eq!(p.cpu.platform, CpuPlatform::Skylake);
        assert_eq!(p.gpus.len(), 1, "virtual display kept inside a VM");
        assert_eq!(p.gpus[0].family, GpuFamily::VirtualDisplay);
        assert!(p.audio.is_none());
        assert_eq!(p.ethernet.len(), 1);
        assert_eq!(p.chipset.as_deref(), Some("ICH9"));
        assert_eq!(p.ram_gb, 8);
        assert!(p.scan_confidence > 0.9, "{}", p.scan_confidence);
    }

    #[test]
    fn virtual_and_bmc_adapters_are_dropped_on_bare_metal() {
        let mut hw = windows_z390();
        hw.gpus.push(crate::contracts::GpuInfo {
            name: "ASPEED Graphics Family".into(),
            vendor_id: Some("1a03".into()),
            device_id: Some("2000".into()),
            ..Default::default()
        });
        hw.gpus.push(crate::contracts::GpuInfo {
            name: "Microsoft Remote Display Adapter".into(),
            ..Default::default()
        });
        let p = build_profile(&hw);
        assert_eq!(p.gpus.len(), 2);
        assert_eq!(p.vm, None);
    }

    #[test]
    fn form_factor_rules() {
        let chassis = |types: &[u32], battery: bool, lid: bool| ChassisInfo {
            chassis_types: types.to_vec(),
            manufacturer: None,
            has_battery: battery,
            has_lid: lid,
        };
        assert_eq!(
            form_factor(&chassis(&[10], true, true), true),
            FormFactor::Laptop
        );
        assert_eq!(
            form_factor(&chassis(&[31], false, false), false),
            FormFactor::Laptop
        );
        assert_eq!(
            form_factor(&chassis(&[13], false, false), false),
            FormFactor::AllInOne
        );
        // Intel NUC (i5-8259U, chassis 35) and Beelink (5800U, chassis 3).
        assert_eq!(
            form_factor(&chassis(&[35], false, false), true),
            FormFactor::MiniPc
        );
        assert_eq!(
            form_factor(&chassis(&[3], false, false), true),
            FormFactor::MiniPc
        );
        // Precision tower with a UPS battery and a desktop CPU.
        assert_eq!(
            form_factor(&chassis(&[7], true, false), false),
            FormFactor::Desktop
        );
        // Laptop firmware reporting a desktop chassis.
        assert_eq!(
            form_factor(&chassis(&[3], true, true), true),
            FormFactor::Laptop
        );
        // Docking station / unknown chassis.
        assert_eq!(
            form_factor(&chassis(&[12], false, false), false),
            FormFactor::Desktop
        );
        assert_eq!(
            form_factor(&chassis(&[2], true, false), true),
            FormFactor::Laptop
        );
        assert_eq!(
            form_factor(&chassis(&[], false, false), false),
            FormFactor::Desktop
        );
    }

    #[test]
    fn ps2_mouse_is_the_touchpad_on_laptops() {
        let mut hw = linux_laptop_i2c();
        hw.input.retain(|d| d.bus != "i2c");
        let p = build_profile(&hw);
        assert_eq!(p.input.touchpad_bus, Some(InputBus::Ps2));
        assert_eq!(p.input.touchpad_hid.as_deref(), Some("DLL08AF"));
        // A PS/2 mouse on a desktop stays a mouse.
        let mut hw = windows_z390();
        hw.input.push(crate::contracts::InputDevice {
            name: "PS/2 Compatible Mouse".into(),
            kind: InputKind::Mouse,
            bus: "ps2".into(),
            hardware_id: Some("PNP0F03".into()),
            vendor: None,
        });
        assert_eq!(build_profile(&hw).input.touchpad_bus, None);
    }

    #[test]
    fn desktop_chipset_names_are_not_trusted_on_laptops() {
        let mut hw = linux_laptop_i2c();
        hw.motherboard.lpc_device_id = None;
        hw.motherboard.product = Some("X570ZD".into());
        let p = build_profile(&hw);
        assert_eq!(p.form_factor, FormFactor::Laptop);
        assert_eq!(p.chipset, None);
    }

    #[test]
    fn partial_scan_never_panics_and_lowers_confidence() {
        let p = build_profile(&DetectedHardware::default());
        assert_eq!(p.cpu.platform, CpuPlatform::Unknown);
        assert!(p.gpus.is_empty() && p.audio.is_none());
        assert!(p.scan_confidence < 0.2, "{}", p.scan_confidence);
        let mut hw = windows_z390();
        hw.cpu.family = None;
        hw.cpu.model = None;
        hw.cpu.features.clear();
        hw.gpus[1].device_id = None;
        hw.audio.clear();
        let p = build_profile(&hw);
        assert_eq!(
            p.cpu.platform,
            CpuPlatform::CoffeeLake,
            "brand string fallback"
        );
        assert_eq!(p.cpu.has_avx2, Some(true), "platform default");
        assert!(p.scan_confidence < 0.7, "{}", p.scan_confidence);
    }

    #[test]
    fn hdmi_only_codecs_give_no_audio() {
        let mut hw = windows_z390();
        hw.audio.retain(|a| a.is_hdmi);
        assert!(build_profile(&hw).audio.is_none());
    }

    #[test]
    fn vm_from_virtual_gpu_without_hypervisor_string() {
        let mut hw = kvm_guest();
        hw.hypervisor = None;
        hw.gpus[0].vendor_id = Some("15ad".into());
        hw.gpus[0].device_id = Some("0405".into());
        assert_eq!(build_profile(&hw).vm, Some(VmKind::Vmware));
    }

    #[test]
    fn multi_socket_cores_are_per_package() {
        let mut hw = windows_z390();
        hw.cpu.cores = 24;
        hw.cpu.threads = 48;
        hw.cpu.packages = 2;
        assert_eq!(build_profile(&hw).cpu.cores, 12);
    }

    #[test]
    fn refresh_reidentifies_gpus_and_keeps_user_choices() {
        let mut p = build_profile(&windows_z390());
        p.gpus[1].family = GpuFamily::Unknown;
        p.gpus[1].disabled = true;
        p.audio.as_mut().expect("audio").layout_id = Some(11);
        p.form_factor = FormFactor::AllInOne;
        p.gpus.push(ProfileGpu {
            name: "My card".into(),
            family: GpuFamily::AmdNavi21,
            ..Default::default()
        });
        let p = refresh_profile(p);
        assert_eq!(p.gpus[1].family, GpuFamily::AmdPolaris);
        assert!(p.gpus[1].disabled);
        // A hand-picked family of the same vendor is a correction and stays;
        // one of another vendor means the ids were edited, so they decide.
        let mut q = p.clone();
        q.gpus[1].family = GpuFamily::AmdVega10;
        let q = refresh_profile(q);
        assert_eq!(q.gpus[1].family, GpuFamily::AmdVega10);
        assert_eq!(q.gpus[1].vendor, GpuVendor::Amd);
        let mut q = q;
        q.gpus[1].family = GpuFamily::NvidiaPascal;
        let q = refresh_profile(q);
        assert_eq!(q.gpus[1].family, GpuFamily::AmdPolaris);
        assert_eq!(
            p.gpus[2].family,
            GpuFamily::AmdNavi21,
            "manual family without ids kept"
        );
        assert_eq!(p.gpus[2].vendor, GpuVendor::Amd);
        assert_eq!(p.audio.and_then(|a| a.layout_id), Some(11));
        assert_eq!(p.form_factor, FormFactor::AllInOne);
    }

    #[test]
    fn refresh_fills_flags_for_a_manually_chosen_platform() {
        let mut p = build_profile(&DetectedHardware::default());
        p.cpu.name = "Mystery CPU".into();
        p.cpu.platform = CpuPlatform::IvyBridge;
        let p = refresh_profile(p);
        assert_eq!(p.cpu.vendor, CpuVendor::Intel);
        assert_eq!(p.cpu.has_avx2, Some(false));
        assert_eq!(p.cpu.has_sse4_2, Some(true));
        assert!(!p.cpu.codename.is_empty());

        // CPUID facts that differ from the auto platform survive a change.
        let mut p = build_profile(&windows_z390());
        p.cpu.has_avx2 = Some(false);
        p.cpu.platform = CpuPlatform::CometLake;
        let p = refresh_profile(p);
        assert_eq!(p.cpu.has_avx2, Some(false));
        assert_eq!(p.cpu.platform, CpuPlatform::CometLake);

        let mut p = build_profile(&windows_z390());
        p.cpu.platform = CpuPlatform::AlderLake;
        let p = refresh_profile(p);
        assert!(p.cpu.is_hybrid);
    }

    #[test]
    fn refresh_resolves_codec_and_chipset() {
        let mut p = build_profile(&windows_z390());
        p.audio = Some(ProfileAudio {
            codec_name: "ALC897".into(),
            ..Default::default()
        });
        p.chipset = None;
        p.ethernet[0].mac_address = Some("aa-bb-cc-dd-ee-ff".into());
        let p = refresh_profile(p);
        assert_eq!(p.audio.and_then(|a| a.codec_id), Some(0x10EC_0897));
        assert_eq!(p.chipset.as_deref(), Some("Z390"));
        assert_eq!(
            p.ethernet[0].mac_address.as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
    }

    #[test]
    fn mac_normalisation() {
        assert_eq!(
            normalize_mac("aabbccddeeff").as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(normalize_mac("00:00:00:00:00:00"), None);
        assert_eq!(normalize_mac("FF-FF-FF-FF-FF-FF"), None);
        assert_eq!(normalize_mac("zz:bb"), None);
    }
}
