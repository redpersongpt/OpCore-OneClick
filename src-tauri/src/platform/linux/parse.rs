//! Text formats read by the Linux scanner: `/proc/cpuinfo`,
//! `/proc/asound/card*/codec#*`, `/proc/bus/input/devices`, `pci.ids` and
//! a few one-line sysfs attributes.

use std::collections::{BTreeSet, HashMap};

use crate::platform::common::parse_hex_u32;

/// Summary of `/proc/cpuinfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuinfoSummary {
    pub vendor: Option<String>,
    pub model_name: Option<String>,
    pub family: Option<u32>,
    pub model: Option<u32>,
    pub stepping: Option<u32>,
    pub flags: Vec<String>,
    /// Number of `processor` entries.
    pub logical: u32,
    /// Unique (physical id, core id) pairs, when the kernel reports them.
    pub physical_cores: Option<u32>,
    pub packages: Option<u32>,
}

pub fn parse_cpuinfo(text: &str) -> CpuinfoSummary {
    let mut summary = CpuinfoSummary::default();
    let mut pairs = BTreeSet::new();
    let mut packages = BTreeSet::new();
    let mut physical_id: Option<u32> = None;
    let mut core_id: Option<u32> = None;
    let mut flush = |physical_id: &mut Option<u32>, core_id: &mut Option<u32>| {
        if let (Some(p), Some(c)) = (*physical_id, *core_id) {
            pairs.insert((p, c));
        }
        if let Some(p) = *physical_id {
            packages.insert(p);
        }
        *physical_id = None;
        *core_id = None;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            if line.trim().is_empty() {
                flush(&mut physical_id, &mut core_id);
            }
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "processor" => summary.logical += 1,
            "vendor_id" if summary.vendor.is_none() => summary.vendor = Some(value.to_string()),
            "model name" if summary.model_name.is_none() => {
                summary.model_name = Some(value.to_string())
            }
            "cpu family" if summary.family.is_none() => summary.family = value.parse().ok(),
            "model" if summary.model.is_none() => summary.model = value.parse().ok(),
            "stepping" if summary.stepping.is_none() => summary.stepping = value.parse().ok(),
            "flags" if summary.flags.is_empty() => {
                summary.flags = value.split_whitespace().map(str::to_string).collect();
            }
            "physical id" => physical_id = value.parse().ok(),
            "core id" => core_id = value.parse().ok(),
            _ => {}
        }
    }
    flush(&mut physical_id, &mut core_id);
    summary.physical_cores = (!pairs.is_empty()).then_some(pairs.len() as u32);
    summary.packages = (!packages.is_empty()).then_some(packages.len() as u32);
    summary
}

/// Count unique (package, core) pairs from `/sys/devices/system/cpu/cpu*/topology`.
pub fn count_topology(entries: &[(u32, u32)]) -> (u32, u32) {
    let cores: BTreeSet<_> = entries.iter().collect();
    let packages: BTreeSet<_> = entries.iter().map(|(p, _)| p).collect();
    (cores.len() as u32, packages.len() as u32)
}

/// Map `/proc/cpuinfo` flag names to the scanner's feature names.
pub fn features_from_flags(flags: &[String]) -> Vec<String> {
    const MAP: &[(&str, &str)] = &[
        ("pni", "sse3"),
        ("ssse3", "ssse3"),
        ("sse4_1", "sse4_1"),
        ("sse4_2", "sse4_2"),
        ("sse4a", "sse4a"),
        ("avx", "avx"),
        ("avx2", "avx2"),
        ("avx512f", "avx512f"),
        ("vmx", "vmx"),
        ("svm", "svm"),
        ("hypervisor", "hypervisor"),
    ];
    MAP.iter()
        .filter(|(flag, _)| flags.iter().any(|f| f == flag))
        .map(|(_, name)| (*name).to_string())
        .collect()
}

/// One codec from `/proc/asound/cardN/codec#M`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsoundCodec {
    pub name: Option<String>,
    pub vendor_id: u32,
    pub subsystem_id: Option<u32>,
    pub revision_id: Option<u32>,
    /// The codec has an audio function group (modem-only codecs do not).
    pub has_audio_function: bool,
}

pub fn parse_asound_codec(text: &str) -> Option<AsoundCodec> {
    let mut codec = AsoundCodec::default();
    let mut vendor = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Codec" => codec.name = Some(value.to_string()),
            "Vendor Id" => vendor = parse_hex_u32(value),
            "Subsystem Id" => codec.subsystem_id = parse_hex_u32(value),
            "Revision Id" => codec.revision_id = parse_hex_u32(value),
            "AFG Function Id" => codec.has_audio_function = true,
            _ => {}
        }
    }
    codec.vendor_id = vendor?;
    if !text.contains("Function Id") {
        // Older kernels print no function-group lines; assume audio.
        codec.has_audio_function = true;
    }
    Some(codec)
}

/// One block of `/proc/bus/input/devices`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcInputDevice {
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
    pub name: String,
    pub phys: String,
    pub sysfs: String,
    pub handlers: Vec<String>,
    /// `B: PROP=` bitmap (bit 0 pointer, 1 direct, 2 buttonpad).
    pub props: u64,
    /// `B: EV=` bitmap.
    pub ev: u64,
}

pub const BUS_USB: u16 = 0x03;
pub const BUS_BLUETOOTH: u16 = 0x05;
pub const BUS_I8042: u16 = 0x11;
pub const BUS_I2C: u16 = 0x18;
pub const BUS_RMI: u16 = 0x1d;

pub fn parse_input_devices(text: &str) -> Vec<ProcInputDevice> {
    let mut out = Vec::new();
    let mut current: Option<ProcInputDevice> = None;
    for line in text.lines().chain(std::iter::once("")) {
        let line = line.trim_end();
        if line.is_empty() {
            if let Some(device) = current.take() {
                out.push(device);
            }
            continue;
        }
        let device = current.get_or_insert_with(ProcInputDevice::default);
        let Some((tag, rest)) = line.split_once(": ") else {
            continue;
        };
        match tag {
            "I" => {
                for field in rest.split_whitespace() {
                    let Some((key, value)) = field.split_once('=') else {
                        continue;
                    };
                    let value = u16::from_str_radix(value, 16).unwrap_or(0);
                    match key {
                        "Bus" => device.bus = value,
                        "Vendor" => device.vendor = value,
                        "Product" => device.product = value,
                        _ => {}
                    }
                }
            }
            "N" => {
                device.name = rest
                    .trim_start_matches("Name=")
                    .trim_matches('"')
                    .to_string()
            }
            "P" => device.phys = rest.trim_start_matches("Phys=").to_string(),
            "S" => device.sysfs = rest.trim_start_matches("Sysfs=").to_string(),
            "H" => {
                device.handlers = rest
                    .trim_start_matches("Handlers=")
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
            }
            "B" => {
                if let Some((key, value)) = rest.split_once('=') {
                    // Multi-word bitmaps list the most significant word first.
                    let low = value.split_whitespace().last().unwrap_or("0");
                    let bits = u64::from_str_radix(low, 16).unwrap_or(0);
                    match key {
                        "PROP" => device.props = bits,
                        "EV" => device.ev = bits,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// ACPI id inside an I2C client name or sysfs path ("i2c-SYNA2393:00" → "SYNA2393").
pub fn hid_from_i2c_name(text: &str) -> Option<String> {
    text.split('/').find_map(|segment| {
        let name = segment.strip_prefix("i2c-").unwrap_or(segment);
        let (hid, instance) = name.split_once(':')?;
        let ok = (4..=9).contains(&hid.len())
            && hid
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && instance.chars().all(|c| c.is_ascii_digit());
        ok.then(|| hid.to_string())
    })
}

/// "PNP: SYN3286 PNP0f13" (serio `firmware_id`) → ["SYN3286", "PNP0F13"].
pub fn serio_firmware_ids(text: &str) -> Vec<String> {
    text.trim()
        .strip_prefix("PNP:")
        .unwrap_or(text)
        .split_whitespace()
        .map(str::to_ascii_uppercase)
        .collect()
}

/// UEFI variable file: 4 attribute bytes followed by the value.
pub fn secure_boot_from_efivar(bytes: &[u8]) -> Option<bool> {
    bytes.get(4).map(|b| *b == 1)
}

/// Vendor and device names from a `pci.ids` database.
#[derive(Debug, Clone, Default)]
pub struct PciIds {
    vendors: HashMap<u16, String>,
    devices: HashMap<(u16, u16), String>,
}

impl PciIds {
    pub fn parse(text: &str) -> Self {
        let mut ids = Self::default();
        let mut vendor: Option<u16> = None;
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if line.starts_with("C ") {
                // Device class section: no vendors below.
                break;
            }
            if let Some(rest) = line.strip_prefix('\t') {
                if rest.starts_with('\t') {
                    continue;
                }
                if let (Some(v), Some((id, name))) = (vendor, split_id(rest)) {
                    ids.devices.insert((v, id), name.to_string());
                }
            } else if let Some((id, name)) = split_id(line) {
                vendor = Some(id);
                ids.vendors.insert(id, name.to_string());
            }
        }
        ids
    }

    pub fn is_empty(&self) -> bool {
        self.vendors.is_empty()
    }

    /// "Intel Corporation CoffeeLake-S GT2 [UHD Graphics 630]", like lspci.
    pub fn name(&self, vendor: &str, device: &str) -> Option<String> {
        let v = u16::from_str_radix(vendor, 16).ok()?;
        let d = u16::from_str_radix(device, 16).ok()?;
        let device_name = self.devices.get(&(v, d))?;
        Some(match self.vendors.get(&v) {
            Some(vendor_name) => format!("{vendor_name} {device_name}"),
            None => device_name.clone(),
        })
    }
}

fn split_id(line: &str) -> Option<(u16, &str)> {
    let (id, name) = line.split_once("  ")?;
    Some((u16::from_str_radix(id.trim(), 16).ok()?, name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO_RYZEN: &str = "processor\t: 0
vendor_id\t: AuthenticAMD
cpu family\t: 25
model\t\t: 33
model name\t: AMD Ryzen 7 5800X 8-Core Processor
stepping\t: 0
physical id\t: 0
core id\t\t: 0
cpu cores\t: 8
flags\t\t: fpu vme pni ssse3 sse4_1 sse4_2 avx avx2 svm sse4a

processor\t: 1
vendor_id\t: AuthenticAMD
physical id\t: 0
core id\t\t: 1

processor\t: 2
vendor_id\t: AuthenticAMD
physical id\t: 0
core id\t\t: 0

processor\t: 3
vendor_id\t: AuthenticAMD
physical id\t: 0
core id\t\t: 1
";

    #[test]
    fn cpuinfo_counts_physical_cores() {
        let s = parse_cpuinfo(CPUINFO_RYZEN);
        assert_eq!(s.vendor.as_deref(), Some("AuthenticAMD"));
        assert_eq!(
            (s.family, s.model, s.stepping),
            (Some(25), Some(33), Some(0))
        );
        assert_eq!(s.logical, 4);
        assert_eq!(s.physical_cores, Some(2));
        assert_eq!(s.packages, Some(1));
        assert_eq!(
            features_from_flags(&s.flags),
            ["sse3", "ssse3", "sse4_1", "sse4_2", "sse4a", "avx", "avx2", "svm"]
        );
        assert_eq!(count_topology(&[(0, 0), (0, 0), (0, 1), (1, 0)]), (3, 2));
    }

    #[test]
    fn cpuinfo_without_topology() {
        let s = parse_cpuinfo("processor : 0\nBogoMIPS : 48.00\n\nprocessor : 1\n");
        assert_eq!(s.logical, 2);
        assert_eq!(s.physical_cores, None);
        assert_eq!(s.vendor, None);
    }

    #[test]
    fn asound_codec() {
        let text = "Codec: Realtek ALC897
Address: 0
AFG Function Id: 0x1 (unsol 1)
Vendor Id: 0x10ec0897
Subsystem Id: 0x10438698
Revision Id: 0x100402
No Modem Function Group found
Default PCM:
";
        let codec = parse_asound_codec(text).unwrap();
        assert_eq!(codec.vendor_id, 0x10ec_0897);
        assert_eq!(codec.subsystem_id, Some(0x1043_8698));
        assert_eq!(codec.revision_id, Some(0x10_0402));
        assert!(codec.has_audio_function);
        let modem = parse_asound_codec(
            "Codec: Conexant ID 2c06\nMFG Function Id: 0x2\nVendor Id: 0x14f12c06\n",
        )
        .unwrap();
        assert!(!modem.has_audio_function);
        assert_eq!(parse_asound_codec("garbage"), None);
    }

    const INPUT_DEVICES: &str = r#"I: Bus=0011 Vendor=0001 Product=0001 Version=ab41
N: Name="AT Translated Set 2 keyboard"
P: Phys=isa0060/serio0/input0
S: Sysfs=/devices/platform/i8042/serio0/input/input0
U: Uniq=
H: Handlers=sysrq kbd event0 leds
B: PROP=0
B: EV=120013
B: KEY=402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe

I: Bus=0018 Vendor=06cb Product=7e7e Version=0100
N: Name="SYNA2393:00 06CB:7E7E Touchpad"
P: Phys=i2c-SYNA2393:00
S: Sysfs=/devices/pci0000:00/0000:00:15.1/i2c_designware.1/i2c-1/i2c-SYNA2393:00/0018:06CB:7E7E.0002/input/input12
U: Uniq=
H: Handlers=mouse1 event7
B: PROP=5
B: EV=1b

I: Bus=0019 Vendor=0000 Product=0005 Version=0000
N: Name="Lid Switch"
P: Phys=PNP0C0D/button/input0
S: Sysfs=/devices/LNXSYSTM:00/LNXSYBUS:00/PNP0C0D:00/input/input2
U: Uniq=
H: Handlers=event2
B: PROP=0
B: EV=21
"#;

    #[test]
    fn input_devices() {
        let devices = parse_input_devices(INPUT_DEVICES);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].bus, BUS_I8042);
        assert_eq!(devices[0].ev, 0x120013);
        assert!(devices[0].handlers.contains(&"kbd".to_string()));
        assert_eq!(devices[1].bus, BUS_I2C);
        assert_eq!(devices[1].vendor, 0x06cb);
        assert_eq!(devices[1].props, 5);
        assert_eq!(
            hid_from_i2c_name(&devices[1].sysfs).as_deref(),
            Some("SYNA2393")
        );
        assert_eq!(devices[2].name, "Lid Switch");
    }

    #[test]
    fn small_attributes() {
        assert_eq!(
            hid_from_i2c_name("i2c-ELAN0662:00").as_deref(),
            Some("ELAN0662")
        );
        assert_eq!(hid_from_i2c_name("i2c-1"), None);
        assert_eq!(
            serio_firmware_ids("PNP: SYN3286 PNP0f13"),
            ["SYN3286", "PNP0F13"]
        );
        assert_eq!(secure_boot_from_efivar(&[6, 0, 0, 0, 1]), Some(true));
        assert_eq!(secure_boot_from_efivar(&[6, 0, 0, 0, 0]), Some(false));
        assert_eq!(secure_boot_from_efivar(&[6, 0]), None);
    }

    #[test]
    fn pci_ids_names() {
        let db = PciIds::parse(
            "# comment\n8086  Intel Corporation\n\t3e92  CoffeeLake-S GT2 [UHD Graphics 630]\n\t\t1043 8694  PRIME\n\
             10ec  Realtek Semiconductor Co., Ltd.\n\t8168  RTL8111/8168/8211/8411 PCI Express Gigabit Ethernet Controller\n\
             C 03  Display controller\n\t00  VGA compatible controller\n",
        );
        assert_eq!(
            db.name("8086", "3e92").as_deref(),
            Some("Intel Corporation CoffeeLake-S GT2 [UHD Graphics 630]")
        );
        assert!(db.name("10ec", "8168").unwrap().contains("RTL8111"));
        assert_eq!(db.name("8086", "ffff"), None);
        assert!(!db.is_empty());
    }
}
