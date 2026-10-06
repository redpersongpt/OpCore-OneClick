//! PCI-attached and USB devices from sysfs: GPUs, HD Audio codecs, network
//! and Bluetooth, storage, USB controllers with their root hub ports.

use std::collections::HashSet;

use crate::contracts::{
    AudioDevice, GpuInfo, NetworkDevice, NetworkKind, StorageDevice, UsbControllerInfo, UsbPortInfo,
};
use crate::platform::common::{
    hex16, is_hdmi_codec_vendor, normalize_mac, parse_hex_u32, storage_kind_from_class,
    usb_controller_kind,
};

use super::parse::{parse_asound_codec, PciIds};
use super::sysfs::{owning_pci_address, pci_functions, usb_device_dir, PciFunction, SysRoot};

const PCI_IDS_PATHS: &[&str] = &[
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
    "/usr/local/share/hwdata/pci.ids",
];

#[derive(Debug, Clone, Default)]
pub struct DeviceScan {
    pub gpus: Vec<GpuInfo>,
    pub audio: Vec<AudioDevice>,
    pub network: Vec<NetworkDevice>,
    pub storage: Vec<StorageDevice>,
    pub usb_controllers: Vec<UsbControllerInfo>,
    /// ISA/LPC bridge (vendor, device): identifies the chipset.
    pub lpc: Option<(String, String)>,
    pub warnings: Vec<String>,
}

pub fn collect(sys: &SysRoot) -> DeviceScan {
    let ids = PCI_IDS_PATHS
        .iter()
        .find_map(|p| std::fs::read_to_string(sys.path(p)).ok())
        .map(|text| PciIds::parse(&text))
        .unwrap_or_default();
    let pci = pci_functions(sys, &ids);
    let mut warnings = Vec::new();
    if pci.is_empty() {
        warnings.push(
            "No PCI devices are visible in /sys/bus/pci (container or restricted sysfs)"
                .to_string(),
        );
    }
    DeviceScan {
        gpus: gpus(sys, &pci),
        audio: audio(sys, &pci),
        network: network(sys, &pci),
        storage: storage(sys, &pci),
        usb_controllers: usb_controllers(sys, &pci),
        lpc: pci
            .iter()
            .find(|f| f.is(0x06, 0x01))
            .map(|f| (f.vendor_id.clone(), f.device_id.clone())),
        warnings,
    }
}

fn find<'a>(pci: &'a [PciFunction], address: &str) -> Option<&'a PciFunction> {
    pci.iter().find(|f| f.address == address)
}

fn gpus(sys: &SysRoot, pci: &[PciFunction]) -> Vec<GpuInfo> {
    pci.iter()
        .filter(|f| f.class.base == 0x03)
        .map(|f| GpuInfo {
            name: f.display_name("Display controller"),
            vendor_id: Some(f.vendor_id.clone()),
            device_id: Some(f.device_id.clone()),
            subsystem_vendor_id: f.subsystem_vendor_id.clone(),
            subsystem_device_id: f.subsystem_device_id.clone(),
            revision: f.revision.clone(),
            // amdgpu exposes the dedicated memory size; other DRM drivers do not.
            vram_mb: sys
                .read(&format!("{}/mem_info_vram_total", f.dir))
                .and_then(|v| v.parse::<u64>().ok())
                .map(|bytes| bytes / (1024 * 1024))
                .filter(|mb| *mb > 0),
            location: f.location.clone(),
        })
        .collect()
}

/// Codec directories on the hdaudio bus, else below each sound card.
fn codec_dirs(sys: &SysRoot) -> Vec<String> {
    let bus: Vec<String> = sys
        .list("/sys/bus/hdaudio/devices")
        .into_iter()
        .map(|n| format!("/sys/bus/hdaudio/devices/{n}"))
        .collect();
    if !bus.is_empty() {
        return bus;
    }
    sys.list("/sys/class/sound")
        .into_iter()
        .filter(|card| card.starts_with("card"))
        .flat_map(|card| {
            let device = format!("/sys/class/sound/{card}/device");
            sys.list(&device)
                .into_iter()
                .filter(|n| n.contains("hdaudio"))
                .map(move |n| format!("{device}/{n}"))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn audio(sys: &SysRoot, pci: &[PciFunction]) -> Vec<AudioDevice> {
    let mut out = Vec::new();
    let mut used = HashSet::new();
    let mut push_codec = |out: &mut Vec<AudioDevice>,
                          name: Option<String>,
                          id: u32,
                          subsystem: Option<u32>,
                          controller: Option<&PciFunction>| {
        let vendor = hex16(id >> 16);
        let device = hex16(id);
        if let Some(c) = controller {
            used.insert(c.address.clone());
        }
        out.push(AudioDevice {
            bus: if controller.is_some_and(is_intel_dsp) {
                "sst"
            } else {
                "hdaudio"
            }
            .into(),
            name: name.unwrap_or_else(|| format!("HD Audio codec {vendor}:{device}")),
            is_hdmi: is_hdmi_codec_vendor(&vendor),
            codec_vendor_id: Some(vendor),
            codec_device_id: Some(device),
            codec_subsystem_id: subsystem.map(|s| format!("{s:08x}")),
            controller_vendor_id: controller.map(|c| c.vendor_id.clone()),
            controller_device_id: controller.map(|c| c.device_id.clone()),
            location: controller.map(|c| c.location.clone()).unwrap_or_default(),
        });
    };

    for dir in codec_dirs(sys) {
        let Some(id) = sys
            .read(&format!("{dir}/vendor_id"))
            .and_then(|v| parse_hex_u32(&v))
        else {
            continue;
        };
        // Modem-only codecs have no audio function group.
        if sys
            .read(&format!("{dir}/afg"))
            .and_then(|v| parse_hex_u32(&v))
            == Some(0)
        {
            continue;
        }
        let name = match (
            sys.read(&format!("{dir}/vendor_name")),
            sys.read(&format!("{dir}/chip_name")),
        ) {
            (Some(v), Some(c)) => Some(format!("{v} {c}")),
            (None, Some(c)) => Some(c),
            _ => None,
        };
        let subsystem = sys
            .read(&format!("{dir}/subsystem_id"))
            .and_then(|v| parse_hex_u32(&v));
        let controller = sys
            .real(&dir)
            .and_then(|r| owning_pci_address(&r))
            .and_then(|a| find(pci, &a));
        push_codec(&mut out, name, id, subsystem, controller);
    }

    if out.is_empty() {
        // Kernels without the hdaudio bus: /proc/asound/cardN/codec#M.
        for card in sys
            .list("/proc/asound")
            .into_iter()
            .filter(|c| c.starts_with("card"))
        {
            let controller = sys
                .real(&format!("/sys/class/sound/{card}/device"))
                .and_then(|r| owning_pci_address(&r))
                .and_then(|a| find(pci, &a));
            for file in sys
                .list(&format!("/proc/asound/{card}"))
                .into_iter()
                .filter(|f| f.starts_with("codec#"))
            {
                let Ok(bytes) = sys.read_bytes(&format!("/proc/asound/{card}/{file}")) else {
                    continue;
                };
                let Some(codec) = parse_asound_codec(&String::from_utf8_lossy(&bytes)) else {
                    continue;
                };
                if codec.has_audio_function {
                    push_codec(
                        &mut out,
                        codec.name,
                        codec.vendor_id,
                        codec.subsystem_id,
                        controller,
                    );
                }
            }
        }
    }

    // Controllers whose codecs are not visible (no driver, or a DSP in SST/SOF mode).
    for f in pci
        .iter()
        .filter(|f| f.class.base == 0x04 && !used.contains(&f.address))
    {
        let bus = if f.is(0x04, 0x03) {
            "hdaudio"
        } else if is_intel_dsp(f) {
            "sst"
        } else {
            continue;
        };
        out.push(AudioDevice {
            name: f.display_name("Audio controller"),
            controller_vendor_id: Some(f.vendor_id.clone()),
            controller_device_id: Some(f.device_id.clone()),
            location: f.location.clone(),
            bus: bus.into(),
            ..Default::default()
        });
    }
    out
}

/// Intel audio controller in DSP mode (SST / SOF: class 0x0401 or 0x0480
/// instead of plain HD Audio 0x0403).
fn is_intel_dsp(f: &PciFunction) -> bool {
    f.vendor_id == "8086" && (f.is(0x04, 0x01) || f.is(0x04, 0x80))
}

struct UsbIds {
    vendor_id: Option<String>,
    product_id: Option<String>,
    name: Option<String>,
}

fn usb_ids(sys: &SysRoot, dir: &str) -> UsbIds {
    let name = match (
        sys.read(&format!("{dir}/manufacturer")),
        sys.read(&format!("{dir}/product")),
    ) {
        (Some(m), Some(p)) if !p.contains(&m) => Some(format!("{m} {p}")),
        (_, Some(p)) => Some(p),
        (m, None) => m,
    };
    UsbIds {
        vendor_id: sys.read_hex(&format!("{dir}/idVendor"), 4),
        product_id: sys.read_hex(&format!("{dir}/idProduct"), 4),
        name,
    }
}

fn network(sys: &SysRoot, pci: &[PciFunction]) -> Vec<NetworkDevice> {
    let mut out: Vec<(Option<String>, NetworkDevice)> = pci
        .iter()
        .filter(|f| f.class.base == 0x02)
        .map(|f| {
            let kind = match f.class.sub {
                0x00 => NetworkKind::Ethernet,
                0x80 => NetworkKind::Wifi,
                _ => NetworkKind::Other,
            };
            let device = NetworkDevice {
                name: f.display_name("Network controller"),
                kind,
                bus: "pci".into(),
                vendor_id: Some(f.vendor_id.clone()),
                device_id: Some(f.device_id.clone()),
                subsystem_vendor_id: f.subsystem_vendor_id.clone(),
                subsystem_device_id: f.subsystem_device_id.clone(),
                mac_address: None,
                location: f.location.clone(),
            };
            (Some(f.address.clone()), device)
        })
        .collect();

    for iface in sys.list("/sys/class/net") {
        let base = format!("/sys/class/net/{iface}");
        let Some(real) = sys.real(&format!("{base}/device")) else {
            continue;
        };
        // ARPHRD_ETHER covers Ethernet and Wi-Fi; skip modems, CAN, InfiniBand.
        if sys.read(&format!("{base}/type")).as_deref() != Some("1") {
            continue;
        }
        let wireless =
            sys.exists(&format!("{base}/wireless")) || sys.exists(&format!("{base}/phy80211"));
        let mac = sys
            .read(&format!("{base}/address"))
            .and_then(|m| normalize_mac(&m));
        match sys
            .link_name(&format!("{base}/device/subsystem"))
            .as_deref()
        {
            Some("pci") => {
                let address = owning_pci_address(&real);
                if let Some((_, device)) =
                    out.iter_mut().find(|(a, _)| a.is_some() && *a == address)
                {
                    if device.mac_address.is_none() {
                        device.mac_address = mac;
                    }
                    if wireless {
                        device.kind = NetworkKind::Wifi;
                    }
                }
            }
            Some(bus @ ("usb" | "sdio")) => {
                let (ids, dir) = if bus == "usb" {
                    let Some(dir) = usb_device_dir(sys, &real) else {
                        continue;
                    };
                    (usb_ids(sys, &dir), dir)
                } else {
                    let ids = UsbIds {
                        vendor_id: sys.read_hex(&format!("{real}/vendor"), 4),
                        product_id: sys.read_hex(&format!("{real}/device"), 4),
                        name: None,
                    };
                    (ids, real.clone())
                };
                if out
                    .iter()
                    .any(|(key, _)| key.as_deref() == Some(dir.as_str()))
                {
                    continue;
                }
                let device = NetworkDevice {
                    name: ids.name.unwrap_or_else(|| iface.clone()),
                    kind: if wireless {
                        NetworkKind::Wifi
                    } else {
                        NetworkKind::Ethernet
                    },
                    bus: bus.into(),
                    vendor_id: ids.vendor_id,
                    device_id: ids.product_id,
                    mac_address: mac,
                    ..Default::default()
                };
                out.push((Some(dir), device));
            }
            _ => {}
        }
    }

    let mut devices: Vec<NetworkDevice> = out.into_iter().map(|(_, d)| d).collect();
    devices.extend(bluetooth(sys, pci, &devices));
    devices
}

fn bluetooth(sys: &SysRoot, pci: &[PciFunction], known: &[NetworkDevice]) -> Vec<NetworkDevice> {
    let mut out: Vec<NetworkDevice> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut add_usb = |out: &mut Vec<NetworkDevice>, dir: &str| {
        if !seen.insert(dir.to_string()) {
            return;
        }
        let ids = usb_ids(sys, dir);
        out.push(NetworkDevice {
            name: ids.name.unwrap_or_else(|| "Bluetooth".into()),
            kind: NetworkKind::Bluetooth,
            bus: "usb".into(),
            vendor_id: ids.vendor_id,
            device_id: ids.product_id,
            ..Default::default()
        });
    };

    for hci in sys
        .list("/sys/class/bluetooth")
        .into_iter()
        .filter(|h| h.starts_with("hci") && !h.contains(':'))
    {
        let Some(real) = sys.real(&format!("/sys/class/bluetooth/{hci}/device")) else {
            continue;
        };
        if let Some(dir) = usb_device_dir(sys, &real) {
            add_usb(&mut out, &dir);
        } else if let Some(dir) = sdio_function_dir(&real) {
            out.push(NetworkDevice {
                name: "Bluetooth".into(),
                kind: NetworkKind::Bluetooth,
                bus: "sdio".into(),
                vendor_id: sys.read_hex(&format!("{dir}/vendor"), 4),
                device_id: sys.read_hex(&format!("{dir}/device"), 4),
                ..Default::default()
            });
        } else if let Some(f) = owning_pci_address(&real)
            .and_then(|a| find(pci, &a))
            // A UART radio (serdev) sits below an LPSS UART, whose ids are
            // not the radio's.
            .filter(|f| matches!(f.class.base, 0x02 | 0x0d))
        {
            let already = known.iter().any(|k| {
                k.location.pci_path.is_some() && k.location.pci_path == f.location.pci_path
            });
            if !already {
                out.push(NetworkDevice {
                    name: f.display_name("Bluetooth"),
                    kind: NetworkKind::Bluetooth,
                    bus: "pci".into(),
                    vendor_id: Some(f.vendor_id.clone()),
                    device_id: Some(f.device_id.clone()),
                    location: f.location.clone(),
                    ..Default::default()
                });
            }
        } else {
            out.push(NetworkDevice {
                name: "Bluetooth".into(),
                kind: NetworkKind::Bluetooth,
                bus: "other".into(),
                ..Default::default()
            });
        }
    }

    // Radios without a bound driver: wireless controller class E0/01/01.
    for device in sys
        .list("/sys/bus/usb/devices")
        .into_iter()
        .filter(|d| !d.contains(':') && !d.starts_with("usb"))
    {
        let dir = format!("/sys/bus/usb/devices/{device}");
        let radio = sys
            .list(&dir)
            .into_iter()
            .filter(|i| i.starts_with(&format!("{device}:")))
            .any(|intf| {
                let attr = |name: &str| sys.read(&format!("{dir}/{intf}/{name}"));
                attr("bInterfaceClass").as_deref() == Some("e0")
                    && attr("bInterfaceSubClass").as_deref() == Some("01")
                    && attr("bInterfaceProtocol").as_deref() == Some("01")
            });
        if radio {
            if let Some(real) = sys.real(&dir) {
                add_usb(&mut out, &real);
            }
        }
    }
    out
}

/// Directory of the SDIO function ("mmc1:0001:2") in a resolved path.
fn sdio_function_dir(real: &str) -> Option<String> {
    let segments: Vec<&str> = real.split('/').collect();
    let at = segments
        .iter()
        .position(|s| s.starts_with("mmc") && s.matches(':').count() == 2)?;
    Some(segments[..=at].join("/"))
}

fn is_ignored_block(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd", "nullb", "zd",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
        || name.ends_with("boot0")
        || name.ends_with("boot1")
        || name.ends_with("rpmb")
}

fn storage(sys: &SysRoot, pci: &[PciFunction]) -> Vec<StorageDevice> {
    let mut out = Vec::new();
    let mut used = HashSet::new();
    for name in sys
        .list("/sys/block")
        .into_iter()
        .filter(|n| !is_ignored_block(n))
    {
        let base = format!("/sys/block/{name}");
        let Some(real) = sys.real(&base) else {
            continue;
        };
        // Native NVMe multipath puts the namespace below the (virtual)
        // subsystem; its controllers are linked from there.
        let multipath = real.contains("/virtual/nvme-subsystem/");
        if real.contains("/virtual/") && !multipath {
            continue;
        }
        let device_path = if multipath {
            nvme_subsystem_controller(sys, &format!("{base}/device"))
        } else {
            Some(real.clone())
        };
        let size_bytes = sys
            .read(&format!("{base}/size"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|sectors| sectors * 512)
            .filter(|b| *b > 0);
        let on_usb = real.split('/').any(|s| {
            s.len() > 3 && s.starts_with("usb") && s[3..].chars().all(|c| c.is_ascii_digit())
        });
        let controller = if on_usb {
            None
        } else {
            device_path
                .as_deref()
                .and_then(owning_pci_address)
                .and_then(|a| find(pci, &a))
        };
        let kind = if on_usb {
            "usb"
        } else if name.starts_with("nvme") {
            "nvme"
        } else if name.starts_with("mmcblk") {
            if sys.read(&format!("{base}/device/type")).as_deref() == Some("MMC") {
                "emmc"
            } else {
                "other"
            }
        } else {
            controller
                .and_then(|c| storage_kind_from_class(c.class))
                .unwrap_or("other")
        };
        let model = sys.read(&format!("{base}/device/model"));
        let vendor = sys
            .read(&format!("{base}/device/vendor"))
            .filter(|v| v != "ATA");
        let display = match (vendor, model) {
            (Some(v), Some(m)) if !m.starts_with(&v) => format!("{v} {m}"),
            (_, Some(m)) => m,
            _ => name.clone(),
        };
        if let Some(c) = controller {
            used.insert(c.address.clone());
        }
        out.push(StorageDevice {
            name: display,
            kind: kind.into(),
            controller_vendor_id: controller.map(|c| c.vendor_id.clone()),
            controller_device_id: controller.map(|c| c.device_id.clone()),
            size_bytes,
        });
    }
    for f in pci
        .iter()
        .filter(|f| f.class.base == 0x01 && !used.contains(&f.address))
    {
        out.push(StorageDevice {
            name: f.display_name("Storage controller"),
            kind: storage_kind_from_class(f.class).unwrap_or("other").into(),
            controller_vendor_id: Some(f.vendor_id.clone()),
            controller_device_id: Some(f.device_id.clone()),
            size_bytes: None,
        });
    }
    out
}

/// Resolved path of the first controller ("nvme0") linked from an NVMe
/// subsystem directory.
fn nvme_subsystem_controller(sys: &SysRoot, subsystem: &str) -> Option<String> {
    sys.list(subsystem)
        .into_iter()
        .find(|n| {
            n.strip_prefix("nvme")
                .is_some_and(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()))
        })
        .and_then(|controller| sys.real(&format!("{subsystem}/{controller}")))
}

fn usb_controllers(sys: &SysRoot, pci: &[PciFunction]) -> Vec<UsbControllerInfo> {
    pci.iter()
        .filter(|f| f.is(0x0c, 0x03))
        .map(|f| {
            let name = f.display_name("USB controller");
            UsbControllerInfo {
                kind: usb_controller_kind(f.class.prog_if, &name).into(),
                name,
                vendor_id: Some(f.vendor_id.clone()),
                device_id: Some(f.device_id.clone()),
                location: f.location.clone(),
                ports: root_hub_ports(sys, &f.dir),
            }
        })
        .collect()
}

/// Ports of every root hub of a host controller (an xHCI has one USB 2 and one
/// USB 3 root hub). Port numbers are per root hub; `peer` links pair them.
fn root_hub_ports(sys: &SysRoot, controller_dir: &str) -> Vec<UsbPortInfo> {
    let mut ports = Vec::new();
    for hub in sys.list(controller_dir) {
        let Some(bus) = hub
            .strip_prefix("usb")
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        else {
            continue;
        };
        let speed: f64 = sys
            .read(&format!("{controller_dir}/{hub}/speed"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let usb3 = speed >= 5000.0;
        let interface = format!("{controller_dir}/{hub}/{bus}-0:1.0");
        let prefix = format!("{hub}-port");
        for port in sys.list(&interface) {
            let Some(index) = port
                .strip_prefix(&prefix)
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let dir = format!("{interface}/{port}");
            let connect_type = sys.read(&format!("{dir}/connect_type")).unwrap_or_default();
            if connect_type == "not used" {
                continue;
            }
            let companion = sys
                .link_name(&format!("{dir}/peer"))
                .and_then(|p| p.rsplit_once("-port").and_then(|(_, n)| n.parse().ok()));
            let name = sys
                .read(&format!("{dir}/firmware_node/path"))
                .and_then(|p| {
                    p.rsplit('.')
                        .next()
                        .map(|s| s.trim_end_matches('_').to_string())
                });
            // Ports wired to a USB Type-C connector link to it (typec class).
            let type_c = sys.exists(&format!("{dir}/connector"));
            let (connector, user_connectable) = match connect_type.as_str() {
                "hardwired" => (Some(255), Some(false)),
                "hotplug" if type_c => (Some(9), Some(true)),
                "hotplug" if usb3 || companion.is_some() => (Some(3), Some(true)),
                "hotplug" => (Some(0), Some(true)),
                _ => (None, None),
            };
            ports.push(UsbPortInfo {
                index,
                name,
                speed_class: if usb3 { "usb3" } else { "usb2" }.into(),
                connector,
                user_connectable,
                companion,
            });
        }
    }
    ports
}

#[cfg(test)]
mod tests {
    use super::super::sysfs::fixture::Tree;
    use super::*;

    fn desktop_tree() -> Tree {
        let t = Tree::new("devices");
        t.pci("pci0000:00", "0000:00:02.0", "8086", "3e92", "030000");
        t.pci(
            "pci0000:00/0000:00:01.0",
            "0000:01:00.0",
            "1002",
            "67df",
            "030000",
        );
        t.file(
            "/sys/devices/pci0000:00/0000:00:01.0/0000:01:00.0/mem_info_vram_total",
            "8589934592\n",
        );
        // HDA controller with an analog codec and an HDMI codec.
        let hda = t.pci("pci0000:00", "0000:00:1f.3", "8086", "a348", "040300");
        for (codec, vendor_id, chip, vendor) in [
            ("hdaudioC0D0", "0x10ec0897", "ALC897", "Realtek"),
            ("hdaudioC0D2", "0x8086280b", "Kabylake HDMI", "Intel"),
        ] {
            let dir = format!("{hda}/{codec}");
            t.file(&format!("{dir}/vendor_id"), vendor_id)
                .file(&format!("{dir}/subsystem_id"), "0x10438698")
                .file(&format!("{dir}/chip_name"), chip)
                .file(&format!("{dir}/vendor_name"), vendor)
                .file(&format!("{dir}/afg"), "0x1")
                .link(&format!("/sys/bus/hdaudio/devices/{codec}"), &dir);
        }
        // GPU HDMI function without a visible codec.
        t.pci(
            "pci0000:00/0000:00:01.0",
            "0000:01:00.1",
            "1002",
            "aaf0",
            "040300",
        );
        // Ethernet + Wi-Fi.
        let lan = t.pci("pci0000:00", "0000:00:1f.6", "8086", "15bc", "020000");
        t.dir("/sys/bus/pci");
        t.file("/sys/class/net/eno1/type", "1")
            .file("/sys/class/net/eno1/address", "a4:bb:6d:12:34:56\n");
        t.link("/sys/class/net/eno1/device", &lan);
        t.link(&format!("{lan}/subsystem"), "/sys/bus/pci");
        let wifi = t.pci(
            "pci0000:00/0000:00:1c.4",
            "0000:02:00.0",
            "8086",
            "2723",
            "028000",
        );
        t.file("/sys/class/net/wlp2s0/type", "1")
            .dir("/sys/class/net/wlp2s0/wireless");
        t.link("/sys/class/net/wlp2s0/device", &wifi);
        t.link(&format!("{wifi}/subsystem"), "/sys/bus/pci");
        t.file("/sys/class/net/docker0/type", "1");
        // XHCI with a USB 2 and a USB 3 root hub and Bluetooth on port 14.
        let xhci = t.pci("pci0000:00", "0000:00:14.0", "8086", "a36d", "0c0330");
        t.file(&format!("{xhci}/usb1/speed"), "480")
            .file(&format!("{xhci}/usb2/speed"), "10000");
        let hs1 = format!("{xhci}/usb1/1-0:1.0/usb1-port1");
        let ss1 = format!("{xhci}/usb2/2-0:1.0/usb2-port1");
        t.file(&format!("{hs1}/connect_type"), "hotplug")
            .file(
                &format!("{hs1}/firmware_node/path"),
                "\\_SB_.PCI0.XHC_.RHUB.HS01",
            )
            .file(&format!("{ss1}/connect_type"), "hotplug");
        t.link(&format!("{hs1}/peer"), &ss1);
        t.link(&format!("{ss1}/peer"), &hs1);
        t.file(
            &format!("{xhci}/usb1/1-0:1.0/usb1-port14/connect_type"),
            "hardwired",
        );
        t.file(
            &format!("{xhci}/usb2/2-0:1.0/usb2-port3/connect_type"),
            "hotplug",
        );
        t.link(
            &format!("{xhci}/usb2/2-0:1.0/usb2-port3/connector"),
            "/sys/devices/platform/USBC000:00/typec/port0",
        );
        t.dir("/sys/devices/platform/USBC000:00/typec/port0");
        t.file(
            &format!("{xhci}/usb1/1-0:1.0/usb1-port9/connect_type"),
            "not used",
        );
        let bt = format!("{xhci}/usb1/1-14");
        t.file(&format!("{bt}/idVendor"), "8087")
            .file(&format!("{bt}/idProduct"), "0aaa");
        t.file(&format!("{bt}/1-14:1.0/bInterfaceClass"), "e0");
        t.link(
            "/sys/class/bluetooth/hci0/device",
            &format!("{bt}/1-14:1.0"),
        );
        // NVMe + SATA disk + controller without disks + USB stick.
        let nvme = t.pci(
            "pci0000:00/0000:00:1d.0",
            "0000:3d:00.0",
            "144d",
            "a808",
            "010802",
        );
        t.file(
            &format!("{nvme}/nvme/nvme0/model"),
            "Samsung SSD 970 EVO Plus 1TB",
        );
        t.file(&format!("{nvme}/nvme/nvme0/nvme0n1/size"), "1953525168");
        t.link(
            &format!("{nvme}/nvme/nvme0/nvme0n1/device"),
            &format!("{nvme}/nvme/nvme0"),
        );
        t.link("/sys/block/nvme0n1", &format!("{nvme}/nvme/nvme0/nvme0n1"));
        let ahci = t.pci("pci0000:00", "0000:00:17.0", "8086", "a352", "010601");
        let sda = format!("{ahci}/ata1/host0/target0:0:0/0:0:0:0/block/sda");
        t.file(&format!("{sda}/size"), "976773168")
            .file(
                &format!("{ahci}/ata1/host0/target0:0:0/0:0:0:0/model"),
                "Samsung SSD 860 EVO 500GB",
            )
            .file(
                &format!("{ahci}/ata1/host0/target0:0:0/0:0:0:0/vendor"),
                "ATA",
            );
        t.link(
            &format!("{sda}/device"),
            &format!("{ahci}/ata1/host0/target0:0:0/0:0:0:0"),
        );
        t.link("/sys/block/sda", &sda);
        t.pci("pci0000:00", "0000:00:0e.0", "8086", "9a0b", "010400");
        let sdb = format!("{xhci}/usb2/2-1/2-1:1.0/host1/target1:0:0/1:0:0:0/block/sdb");
        t.file(&format!("{sdb}/size"), "60063744");
        t.link("/sys/block/sdb", &sdb);
        t.dir("/sys/devices/virtual/block/loop0");
        t.link("/sys/block/loop0", "/sys/devices/virtual/block/loop0");
        // LPC
        t.pci("pci0000:00", "0000:00:1f.0", "8086", "a305", "060100");
        t
    }

    #[test]
    fn collects_desktop_devices() {
        let tree = desktop_tree();
        let scan = collect(&SysRoot::new(&tree.root));

        assert_eq!(scan.gpus.len(), 2);
        let dgpu = scan
            .gpus
            .iter()
            .find(|g| g.device_id.as_deref() == Some("67df"))
            .unwrap();
        assert_eq!(dgpu.vram_mb, Some(8192));
        assert_eq!(
            dgpu.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)")
        );

        let analog = scan
            .audio
            .iter()
            .find(|a| a.codec_device_id.as_deref() == Some("0897"))
            .unwrap();
        assert_eq!(analog.name, "Realtek ALC897");
        assert_eq!(analog.codec_subsystem_id.as_deref(), Some("10438698"));
        assert_eq!(analog.controller_device_id.as_deref(), Some("a348"));
        assert_eq!(
            analog.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        assert!(!analog.is_hdmi);
        assert!(scan
            .audio
            .iter()
            .any(|a| a.codec_vendor_id.as_deref() == Some("8086") && a.is_hdmi));
        let hdmi_controller = scan
            .audio
            .iter()
            .find(|a| a.controller_device_id.as_deref() == Some("aaf0"))
            .unwrap();
        assert_eq!(hdmi_controller.codec_vendor_id, None);
        assert_eq!(scan.audio.len(), 3);

        let lan = scan
            .network
            .iter()
            .find(|n| n.kind == NetworkKind::Ethernet)
            .unwrap();
        assert_eq!(lan.mac_address.as_deref(), Some("a4:bb:6d:12:34:56"));
        let wifi = scan
            .network
            .iter()
            .find(|n| n.kind == NetworkKind::Wifi)
            .unwrap();
        assert_eq!(wifi.device_id.as_deref(), Some("2723"));
        let bt: Vec<_> = scan
            .network
            .iter()
            .filter(|n| n.kind == NetworkKind::Bluetooth)
            .collect();
        assert_eq!(bt.len(), 1);
        assert_eq!(
            (bt[0].vendor_id.as_deref(), bt[0].device_id.as_deref()),
            (Some("8087"), Some("0aaa"))
        );

        let names: Vec<(&str, &str)> = scan
            .storage
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("Samsung SSD 970 EVO Plus 1TB", "nvme")));
        assert!(names.contains(&("Samsung SSD 860 EVO 500GB", "sata")));
        assert!(names.contains(&("sdb", "usb")));
        assert!(names.contains(&("Storage controller 8086:9a0b", "raid")));
        assert!(!names.iter().any(|(n, _)| n.starts_with("loop")));
        let nvme = scan.storage.iter().find(|s| s.kind == "nvme").unwrap();
        assert_eq!(nvme.size_bytes, Some(1_953_525_168 * 512));
        assert_eq!(nvme.controller_device_id.as_deref(), Some("a808"));

        let xhci = &scan.usb_controllers[0];
        assert_eq!(xhci.kind, "xhci");
        assert_eq!(xhci.ports.len(), 4);
        let type_c = xhci
            .ports
            .iter()
            .find(|p| p.speed_class == "usb3" && p.index == 3)
            .unwrap();
        assert_eq!(type_c.connector, Some(9));
        let hs1 = xhci
            .ports
            .iter()
            .find(|p| p.speed_class == "usb2" && p.index == 1)
            .unwrap();
        assert_eq!(hs1.name.as_deref(), Some("HS01"));
        assert_eq!(hs1.companion, Some(1));
        assert_eq!(hs1.connector, Some(3));
        let internal = xhci.ports.iter().find(|p| p.index == 14).unwrap();
        assert_eq!(internal.user_connectable, Some(false));
        assert!(xhci.ports.iter().any(|p| p.speed_class == "usb3"));

        assert_eq!(scan.lpc, Some(("8086".into(), "a305".into())));
        assert!(scan.warnings.is_empty());
    }

    #[test]
    fn empty_sysfs_is_not_fatal() {
        let tree = Tree::new("empty");
        let scan = collect(&SysRoot::new(&tree.root));
        assert!(scan.gpus.is_empty() && scan.audio.is_empty() && scan.network.is_empty());
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn laptop_specific_layouts() {
        let t = Tree::new("laptop-devices");
        // NVMe namespace under native multipath: the block device hangs off
        // the virtual subsystem, which links the PCI controller.
        let nvme = t.pci(
            "pci0000:00/0000:00:1d.0",
            "0000:3d:00.0",
            "1e0f",
            "0001",
            "010802",
        );
        let subsys = "/sys/devices/virtual/nvme-subsystem/nvme-subsys0";
        t.dir(&format!("{nvme}/nvme/nvme0"))
            .file(&format!("{subsys}/model"), "KXG60ZNV512G KIOXIA")
            .file(&format!("{subsys}/nvme0n1/size"), "1000215216");
        t.link(&format!("{subsys}/nvme0"), &format!("{nvme}/nvme/nvme0"));
        t.link(&format!("{subsys}/nvme0n1/device"), subsys);
        t.link("/sys/block/nvme0n1", &format!("{subsys}/nvme0n1"));
        // Audio DSP in SST/SOF mode (class 0x0401) with its codec.
        let dsp = t.pci("pci0000:00", "0000:00:1f.3", "8086", "9dc8", "040100");
        t.file(&format!("{dsp}/ehdaudio0D0/vendor_id"), "0x10ec0257");
        t.link(
            "/sys/bus/hdaudio/devices/ehdaudio0D0",
            &format!("{dsp}/ehdaudio0D0"),
        );
        // Bluetooth on the LPSS UART (serdev) and on SDIO.
        let uart = t.pci("pci0000:00", "0000:00:1e.0", "8086", "9da8", "118000");
        t.dir(&format!("{uart}/dw-apb-uart.0/serial0/serial0-0"));
        t.link(
            "/sys/class/bluetooth/hci0/device",
            &format!("{uart}/dw-apb-uart.0/serial0/serial0-0"),
        );
        let sd = t.pci("pci0000:00", "0000:00:14.5", "8086", "9df5", "080501");
        let func = format!("{sd}/mmc_host/mmc1/mmc1:0001/mmc1:0001:2");
        t.file(&format!("{func}/vendor"), "0x02d0")
            .file(&format!("{func}/device"), "0xa9a6");
        t.link("/sys/class/bluetooth/hci1/device", &func);

        let scan = collect(&SysRoot::new(&t.root));
        let disk = scan.storage.iter().find(|s| s.kind == "nvme").unwrap();
        assert_eq!(disk.name, "KXG60ZNV512G KIOXIA");
        assert_eq!(disk.size_bytes, Some(1_000_215_216 * 512));
        assert_eq!(disk.controller_device_id.as_deref(), Some("0001"));
        assert_eq!(scan.storage.len(), 1, "the controller owns the disk");

        assert_eq!(scan.audio.len(), 1);
        assert_eq!(scan.audio[0].bus, "sst");
        assert_eq!(scan.audio[0].codec_device_id.as_deref(), Some("0257"));
        assert_eq!(scan.audio[0].controller_device_id.as_deref(), Some("9dc8"));

        let bt: Vec<(&str, Option<&str>)> = scan
            .network
            .iter()
            .filter(|n| n.kind == NetworkKind::Bluetooth)
            .map(|n| (n.bus.as_str(), n.device_id.as_deref()))
            .collect();
        assert_eq!(bt, [("other", None), ("sdio", Some("a9a6"))]);
    }

    #[test]
    fn asound_fallback() {
        let t = Tree::new("asound");
        let hda = t.pci("pci0000:00", "0000:00:1b.0", "8086", "1c20", "040300");
        t.link("/sys/class/sound/card0/device", &hda);
        t.file(
            "/proc/asound/card0/codec#0",
            "Codec: Realtek ALC892\nAFG Function Id: 0x1 (unsol 1)\nVendor Id: 0x10ec0892\nSubsystem Id: 0x1458a002\n",
        );
        let scan = collect(&SysRoot::new(&t.root));
        assert_eq!(scan.audio.len(), 1);
        assert_eq!(scan.audio[0].codec_device_id.as_deref(), Some("0892"));
        assert_eq!(scan.audio[0].controller_device_id.as_deref(), Some("1c20"));
    }
}
