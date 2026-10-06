//! Network, Bluetooth, input and storage device knowledge: which kext (if any)
//! drives a given PCI/USB id under which macOS release.
//!
//! Id lists are taken from the `IOPCIMatch` / `IONameMatch` / USB personalities
//! of the kexts themselves (IntelMausi 1.0.8 and Mieze's IntelMausiEthernet,
//! AppleIGC 1.9, AppleIGB 5.11, IntelLucy 1.1.6, AtherosE2200Ethernet 2.4.0,
//! RealtekRTL8111 3.0.0, RTL812xLucy 1.1.1, RealtekRTL8100 2.0.1, AirportItlwm
//! 2.3.0, AirportBrcmFixup 2.2.1, BrcmPatchRAM 2.7.2, IntelBluetoothFirmware,
//! RealtekBluetoothFirmware 1.0.2, Feixiao rtw88, CtlnaAHCIPort /
//! SATA-unsupported), cross-checked against pci.ids, the Linux e1000e / vmd
//! tables, the AirportBrcmFixup per-release driver matrix and Dortania.

use super::model::{
    DeviceBus, InputBus, MacOsVersion, ProfileNic, ProfileStorage, StorageKind, TouchpadVendor,
};

// ── Id parsing ──────────────────────────────────────────────────────────────

const VENDOR_APPLE: u16 = 0x106B;
const VENDOR_INTEL: u16 = 0x8086;
const VENDOR_AMD: u16 = 0x1022;
const VENDOR_ATI: u16 = 0x1002;
const VENDOR_REALTEK: u16 = 0x10EC;
const VENDOR_DLINK: u16 = 0x1186;
const VENDOR_ATHEROS_ETH: u16 = 0x1969;
const VENDOR_ATHEROS_WIFI: u16 = 0x168C;
const VENDOR_BROADCOM: u16 = 0x14E4;
const VENDOR_AQUANTIA: u16 = 0x1D6A;
const VENDOR_MARVELL: u16 = 0x11AB;
const VENDOR_VMWARE: u16 = 0x15AD;
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VENDOR_MEDIATEK: u16 = 0x14C3;
const VENDOR_QUALCOMM: u16 = 0x17CB;
const VENDOR_SAMSUNG: u16 = 0x144D;
const VENDOR_MICRON: u16 = 0x1344;
const VENDOR_SK_HYNIX: u16 = 0x1C5C;

const USB_INTEL: u16 = 0x8087;
const USB_APPLE: u16 = 0x05AC;
const USB_BROADCOM: u16 = 0x0A5C;
const USB_REALTEK: u16 = 0x0BDA;
const USB_ATHEROS: u16 = 0x0CF3;
const USB_MEDIATEK: u16 = 0x0E8D;

/// Parse a 16-bit PCI/USB id written as "8086", "0x8086" or "8086h"
/// (case-insensitive, surrounding whitespace ignored).
pub fn parse_id16(text: &str) -> Option<u16> {
    let t = text.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    let t = t
        .strip_suffix('h')
        .or_else(|| t.strip_suffix('H'))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 4 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(t, 16).ok()
}

/// Parse a subsystem id into `(vendor, device)`. The profile stores it vendor
/// first ("106b0117", like the HDA `SUBSYS_` field); "106b:0117", "106b-0117"
/// and a "0x" prefix are accepted too.
pub fn parse_subsystem(text: &str) -> Option<(u16, u16)> {
    let t = text.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    let compact: String = t
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '_' | ' '))
        .collect();
    if compact.len() != 8 || !compact.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let vendor = u16::from_str_radix(&compact[..4], 16).ok()?;
    let device = u16::from_str_radix(&compact[4..], 16).ok()?;
    Some((vendor, device))
}

fn nic_ids(nic: &ProfileNic) -> Option<(u16, u16)> {
    let vendor = parse_id16(nic.vendor_id.as_deref()?)?;
    let device = parse_id16(nic.device_id.as_deref()?)?;
    Some((vendor, device))
}

/// True when the card carries an Apple subsystem id (genuine Apple AirPort
/// cards such as BCM94360CD/CS2 on adapters). Both halves are checked because
/// scanners differ in subsystem byte order.
fn has_apple_subsystem(nic: &ProfileNic) -> Option<bool> {
    let (a, b) = parse_subsystem(nic.subsystem_id.as_deref()?)?;
    Some(a == VENDOR_APPLE || b == VENDOR_APPLE)
}

fn lookup<'a>(table: &'a [(u16, &'a str)], device: u16) -> Option<&'a str> {
    table
        .iter()
        .find(|(id, _)| *id == device)
        .map(|(_, name)| *name)
}

fn contains(table: &[u16], device: u16) -> bool {
    table.contains(&device)
}

fn pair_lookup<'a>(table: &'a [(u16, u16, &'a str)], vendor: u16, device: u16) -> Option<&'a str> {
    table
        .iter()
        .find(|(v, d, _)| *v == vendor && *d == device)
        .map(|(_, _, name)| *name)
}

fn le_id(device: u16) -> [u8; 4] {
    let [lo, hi] = device.to_le_bytes();
    [lo, hi, 0, 0]
}

fn display_name(nic: &ProfileNic, fallback: &str) -> String {
    if nic.name.trim().is_empty() {
        fallback.to_string()
    } else {
        nic.name.trim().to_string()
    }
}

// ── Ethernet ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EthernetDriver {
    /// Intel 82577..I219 — IntelMausi (or Mieze IntelMausiEthernet with AppleVTD
    /// and for the 700/800-series I219 ids only that fork matches).
    IntelMausi,
    /// Intel I211 / 82576 class — SmallTreeIntel82576 (10.15-11) / AppleIGB (12+).
    /// 82575/82580/I354/DH89xx are AppleIGB only (see `EthernetInfo::preferred_kext`).
    IntelI211,
    /// Intel I225/I226 — AppleIGC kext, or native AppleIntelI210 with a device-id spoof.
    IntelI225,
    /// Intel X520/X540/X550/82598/82599 10GbE — IntelLucy.
    IntelLucy,
    /// No kext: a driver shipped with macOS matches (Intel I210/I350/82574L/
    /// 82566DC/82571EB/80003ES2LAN/82545EM, VMware vmxnet3, VirtIO), possibly
    /// after a `device-id` spoof (`EthernetInfo::device_id_spoof`).
    NativeIntel,
    /// Killer E220x/E2400/E2500, Atheros AR816x/AR817x — AtherosE2200Ethernet.
    AtherosE2200,
    /// Realtek RTL8111/8168 (incl. Killer E2500v2/E2600) — RealtekRTL8111
    /// (2.4.2 on AMD: no AppleVTD).
    RealtekRtl8111,
    /// Realtek RTL8125/8126 (incl. Killer E3000/E5000) — RTL812xLucy
    /// (LucyRTL8125Ethernet fallback for RTL8125).
    RealtekRtl8125,
    /// Realtek RTL8100/8101 — RealtekRTL8100.
    RealtekRtl8100,
    /// Aquantia AQC107/113 — native AppleEthernetAquantiaAqtion.
    NativeAquantia,
    /// Broadcom BCM57xx — native AppleBCM5701Ethernet (sometimes needs spoof).
    NativeBroadcom,
    Unsupported,
}

/// Everything known about one wired NIC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EthernetInfo {
    pub driver: EthernetDriver,
    /// Controller name ("Intel I219-V", "Realtek RTL8125").
    pub chip: String,
    /// Catalog id of the kext to use instead of the default one for `driver`
    /// ("IntelMausiEthernet", "AppleIGB", "AppleIGC").
    pub preferred_kext: Option<&'static str>,
    /// `device-id` DeviceProperties value (little endian) that makes the driver
    /// match (I225-V → I225-LM, I350 → I210, BCM57xx → BCM57765, I225-K →
    /// I225-V for AppleIGC, ...). Inject it whenever the NIC is used; for
    /// I225-V it is only needed on the native path but harmless with AppleIGC.
    pub device_id_spoof: Option<[u8; 4]>,
    /// First release the driver supports; None = every supported release.
    pub min_macos: Option<MacOsVersion>,
    pub notes: Vec<String>,
}

impl EthernetInfo {
    fn new(driver: EthernetDriver, chip: impl Into<String>) -> Self {
        Self {
            driver,
            chip: chip.into(),
            preferred_kext: None,
            device_id_spoof: None,
            min_macos: None,
            notes: Vec::new(),
        }
    }

    fn kext(mut self, kext: &'static str) -> Self {
        self.preferred_kext = Some(kext);
        self
    }

    fn spoof(mut self, device: u16) -> Self {
        self.device_id_spoof = Some(le_id(device));
        self
    }

    fn min(mut self, version: MacOsVersion) -> Self {
        self.min_macos = Some(version);
        self
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// IntelMausi 1.0.8 `IOPCIMatch` (82577/82578/82579/I217/I218/I219).
const INTEL_MAUSI: &[(u16, &str)] = &[
    (0x10EA, "82577LM"),
    (0x10EB, "82577LC"),
    (0x10EF, "82578DM"),
    (0x10F0, "82578DC"),
    (0x1502, "82579LM"),
    (0x1503, "82579V"),
    (0x153A, "I217-LM"),
    (0x153B, "I217-V"),
    (0x155A, "I218-LM"),
    (0x1559, "I218-V"),
    (0x15A0, "I218-LM"),
    (0x15A1, "I218-V"),
    (0x15A2, "I218-LM"),
    (0x15A3, "I218-V"),
    (0x156F, "I219-LM"),
    (0x1570, "I219-V"),
    (0x15B7, "I219-LM"),
    (0x15B8, "I219-V"),
    (0x15B9, "I219-LM"),
    (0x15D7, "I219-LM"),
    (0x15D8, "I219-V"),
    (0x15E3, "I219-LM"),
    (0x15D6, "I219-V"),
    (0x15BD, "I219-LM"),
    (0x15BE, "I219-V"),
    (0x15BB, "I219-LM"),
    (0x15BC, "I219-V"),
    (0x15DF, "I219-LM"),
    (0x15E0, "I219-V"),
    (0x15E1, "I219-LM"),
    (0x15E2, "I219-V"),
    (0x0D4E, "I219-LM"),
    (0x0D4F, "I219-V"),
    (0x0D4C, "I219-LM"),
    (0x0D4D, "I219-V"),
    (0x0D53, "I219-LM"),
    (0x0D55, "I219-V"),
    (0x15FB, "I219-LM"),
    (0x15FC, "I219-V"),
    (0x15F9, "I219-LM"),
    (0x15FA, "I219-V"),
    (0x15F4, "I219-LM"),
    (0x15F5, "I219-V"),
    (0x1A1E, "I219-LM"),
    (0x1A1F, "I219-V"),
    (0x1A1C, "I219-LM"),
    (0x1A1D, "I219-V"),
    (0x550A, "I219-LM"),
    (0x550B, "I219-V"),
    (0x550C, "I219-LM"),
    (0x550D, "I219-V"),
];

/// I219 ids matched only by Mieze's IntelMausiEthernet 3.0.x (Raptor Lake /
/// Lunar Lake PCH).
const INTEL_MAUSI_MIEZE_ONLY: &[(u16, &str)] = &[
    (0x0DC5, "I219-LM"),
    (0x0DC6, "I219-V"),
    (0x0DC7, "I219-LM"),
    (0x0DC8, "I219-V"),
    (0x550E, "I219-LM"),
    (0x550F, "I219-V"),
    (0x5510, "I219-LM"),
    (0x5511, "I219-V"),
];

/// Newest e1000e I219 ids (Arrow Lake, Panther Lake, Nova Lake PCH) that no
/// macOS driver matches yet.
const INTEL_I219_NO_DRIVER: &[(u16, &str)] = &[
    (0x57A0, "I219-LM"),
    (0x57A1, "I219-V"),
    (0x57B3, "I219-LM"),
    (0x57B4, "I219-V"),
    (0x57B7, "I219-LM"),
    (0x57B8, "I219-V"),
    (0x57B9, "I219-LM"),
    (0x57BA, "I219-V"),
];

/// Older e1000e parts (ICH8-10 82566/82567/82562, 82571-82574/82583,
/// 80003ES2LAN) with no driver in macOS or IntelMausi.
const INTEL_E1000E_LEGACY: &[(u16, &str)] = &[
    (0x1049, "82566MM"),
    (0x104A, "82566DM"),
    (0x104C, "82562V"),
    (0x104D, "82566MC"),
    (0x105F, "82571EB"),
    (0x1060, "82571EB"),
    (0x107D, "82572EI"),
    (0x107E, "82572EI"),
    (0x107F, "82572EI"),
    (0x108B, "82573V"),
    (0x108C, "82573E"),
    (0x1098, "80003ES2LAN"),
    (0x109A, "82573L"),
    (0x10A4, "82571EB"),
    (0x10A5, "82571EB"),
    (0x10B9, "82572EI"),
    (0x10BA, "80003ES2LAN"),
    (0x10BB, "80003ES2LAN"),
    (0x10BC, "82571EB"),
    (0x10BD, "82566DM-2"),
    (0x10BF, "82567LF"),
    (0x10C0, "82562V-2"),
    (0x10C2, "82562G-2"),
    (0x10C3, "82562GT-2"),
    (0x10C4, "82562GT"),
    (0x10C5, "82562G"),
    (0x10CB, "82567V"),
    (0x10CC, "82567LM-2"),
    (0x10CD, "82567LF-2"),
    (0x10CE, "82567V-2"),
    (0x10D3, "82574L"),
    (0x10D5, "82571PT"),
    (0x10D9, "82571EB"),
    (0x10DA, "82571EB"),
    (0x10DE, "82567LM-3"),
    (0x10DF, "82567LF-3"),
    (0x10E5, "82567LM-4"),
    (0x10F5, "82567LM"),
    (0x1501, "82567V-3"),
    (0x150C, "82583V"),
    (0x1525, "82567V-4"),
    (0x294C, "82566DC-2"),
];

/// Intel parts with a driver inside macOS (AppleIntel8254XEthernet,
/// Intel82574L, AppleIntelI210Ethernet) — Dortania "Native Ethernet Controllers".
const INTEL_NATIVE: &[(u16, &str)] = &[
    (0x100F, "82545EM"),
    (0x104B, "82566DC"),
    (0x105E, "82571EB"),
    (0x1096, "80003ES2LAN"),
    (0x10F6, "82574L"),
    (0x1533, "I210"),
];

/// I210 variants that AppleIntelI210Ethernet does not list; a device-id spoof
/// to 0x1533 makes it match.
const INTEL_I210_VARIANTS: &[(u16, &str)] = &[
    (0x1534, "I210 (OEM)"),
    (0x1535, "I210-IT"),
    (0x1536, "I210 (fiber)"),
    (0x1537, "I210 (backplane)"),
    (0x1538, "I210 (SGMII)"),
    (0x157B, "I210 (flashless)"),
    (0x157C, "I210 (backplane, flashless)"),
    (0x15F6, "I210"),
];

/// I350 — native through a device-id spoof to I210 (Dortania HEDT).
const INTEL_I350: &[(u16, &str)] = &[
    (0x1521, "I350"),
    (0x1522, "I350 (fiber)"),
    (0x1523, "I350 (backplane)"),
    (0x1524, "I350"),
    (0x1546, "I350"),
];

/// I211 and the 82576 family (SmallTree's own controller): SmallTreeIntel82576
/// up to Big Sur, AppleIGB from Monterey.
const INTEL_I211_CLASS: &[(u16, &str)] = &[
    (0x1539, "I211"),
    (0x10C9, "82576"),
    (0x10E6, "82576"),
    (0x10E7, "82576"),
    (0x10E8, "82576"),
    (0x1526, "82576"),
    (0x150A, "82576NS"),
    (0x150D, "82576"),
    (0x1518, "82576NS"),
];

/// Parts only AppleIGB matches.
const INTEL_IGB_ONLY: &[(u16, &str)] = &[
    (0x10A7, "82575EB"),
    (0x10A9, "82575EB"),
    (0x10D6, "82575GB"),
    (0x150E, "82580"),
    (0x150F, "82580"),
    (0x1510, "82580"),
    (0x1511, "82580"),
    (0x1516, "82580"),
    (0x1527, "82580"),
    (0x0438, "DH8900CC"),
    (0x034A, "DH8900CC"),
    (0x043C, "DH8900CC"),
    (0x0440, "DH8900CC"),
    (0x1F40, "I354"),
    (0x1F41, "I354"),
    (0x1F45, "I354"),
];

/// I225/I226 variants that AppleIGC's `igc_set_mac_type` supports but its
/// `IOPCIMatch` omits: (device, chip, device-id to spoof for matching).
const INTEL_IGC_SPOOF: &[(u16, &str, u16)] = &[
    (0x0D9F, "I225-IT", 0x15F3),
    (0x15F7, "I220-V", 0x15F3),
    (0x3100, "I225-K (Killer E3100)", 0x15F3),
    (0x3101, "I225-K2 (Killer E3100X)", 0x15F3),
    (0x5502, "I225-LMvP", 0x15F3),
    (0x5503, "I226-LMvP", 0x125C),
    (0x125E, "I221-V", 0x125C),
];

/// IntelLucy 1.1.6 `IOPCIPrimaryMatch`.
const INTEL_LUCY: &[(u16, &str)] = &[
    (0x10B6, "82598"),
    (0x10C6, "82598EB"),
    (0x10C7, "82598EB"),
    (0x10C8, "82598EB"),
    (0x10DB, "82598EB"),
    (0x10DD, "82598EB"),
    (0x10E1, "82598EB"),
    (0x10EC, "82598EB"),
    (0x10F1, "82598EB"),
    (0x1508, "82598EB"),
    (0x150B, "82598EB"),
    (0x10F7, "82599"),
    (0x10F8, "82599"),
    (0x10F9, "82599"),
    (0x10FB, "82599ES"),
    (0x10FC, "82599"),
    (0x1507, "X520"),
    (0x1514, "X520"),
    (0x1517, "82599ES"),
    (0x151C, "82599"),
    (0x1529, "82599"),
    (0x152A, "82599"),
    (0x154A, "X520"),
    (0x154D, "X520"),
    (0x154F, "82599"),
    (0x1557, "82599"),
    (0x1558, "X520"),
    (0x1528, "X540"),
    (0x1560, "X540"),
    (0x1563, "X550"),
    (0x15D1, "X550"),
    (0x15AA, "X552"),
    (0x15AB, "X552"),
    (0x15AC, "X552"),
    (0x15AD, "X552"),
    (0x15AE, "X552"),
    (0x15B0, "X552"),
    (0x15C2, "X553"),
    (0x15C3, "X553"),
    (0x15C4, "X553"),
    (0x15C6, "X553"),
    (0x15C7, "X553"),
    (0x15C8, "X553"),
    (0x15CE, "X553"),
    (0x15E4, "X553"),
    (0x15E5, "X553"),
];

/// AtherosE2200Ethernet 2.4.0 `IOPCIMatch`.
const ATHEROS_E2200: &[(u16, &str)] = &[
    (0x1090, "AR8162"),
    (0x1091, "AR8161"),
    (0x10A0, "QCA8172"),
    (0x10A1, "QCA8171"),
    (0xE091, "Killer E220x"),
    (0xE0A1, "Killer E2400"),
    (0xE0B1, "Killer E2500"),
];

/// Older Atheros/Attansic parts (AR813x/AR815x, L1/L2) — only the legacy
/// third-party AtherosL1cEthernet ever drove them.
const ATHEROS_LEGACY: &[(u16, &str)] = &[
    (0x1026, "AR8121/AR8113/AR8114"),
    (0x1048, "Attansic L1"),
    (0x1062, "AR8132"),
    (0x1063, "AR8131"),
    (0x1066, "Attansic L2c"),
    (0x1067, "Attansic L1c"),
    (0x1073, "AR8151 v1.0"),
    (0x1083, "AR8151 v2.0"),
    (0x2048, "Attansic L2"),
    (0x2060, "AR8152 v1.1"),
    (0x2062, "AR8152 v2.0"),
];

/// Realtek RTL8111/8168 family (RealtekRTL8111 3.0.0 `IOPCIMatch`).
const REALTEK_RTL8111: &[(u16, u16, &str)] = &[
    (VENDOR_REALTEK, 0x8168, "RTL8111/8168"),
    (VENDOR_DLINK, 0x8168, "RTL8111/8168 (D-Link)"),
    (VENDOR_REALTEK, 0x2502, "Killer E2500v2 (RTL8111)"),
    (VENDOR_REALTEK, 0x2600, "Killer E2600 (RTL8111)"),
];

/// RTL812xLucy 1.1.1 `IOPCIPrimaryMatch`.
const REALTEK_RTL812X: &[(u16, u16, &str)] = &[
    (VENDOR_REALTEK, 0x8125, "RTL8125 2.5GbE"),
    (VENDOR_DLINK, 0x8125, "RTL8125 2.5GbE (D-Link)"),
    (VENDOR_REALTEK, 0x3000, "Killer E3000 2.5GbE (RTL8125)"),
    (VENDOR_REALTEK, 0x8126, "RTL8126 5GbE"),
    (VENDOR_REALTEK, 0x5000, "Killer E5000 5GbE (RTL8126)"),
];

/// Ids LucyRTL8125Ethernet also matches (fallback for RTL8125 only).
const LUCY_RTL8125_FALLBACK: &[(u16, u16)] = &[
    (VENDOR_REALTEK, 0x8125),
    (VENDOR_DLINK, 0x8125),
    (VENDOR_REALTEK, 0x3000),
];

/// Aquantia AQtion ids in the `IONameMatch` of AppleEthernetAquantiaAqtion;
/// `true` = AQC113 generation (Monterey+).
const AQUANTIA: &[(u16, &str, bool)] = &[
    (0x0001, "AQC107", false),
    (0xD107, "AQC107", false),
    (0x07B1, "AQC107", false),
    (0x87B1, "AQC107S", false),
    (0x88B1, "AQC107", false),
    (0x89B1, "AQC107", false),
    (0x91B1, "AQC107", false),
    (0x92B1, "AQC107", false),
    (0x80B1, "AQC100S", false),
    (0x00C0, "AQC113", true),
    (0x04C0, "AQC113", true),
    (0x34C0, "AQC113", true),
    (0x93C0, "AQC114CS", true),
    (0x94C0, "AQC113CS", true),
];

/// Broadcom NetXtreme parts AppleBCM5701Ethernet matches natively.
const BROADCOM_NATIVE: &[(u16, &str)] = &[
    (0x1682, "BCM57762"),
    (0x1684, "BCM5764M"),
    (0x1686, "BCM57766"),
    (0x16B0, "BCM57761"),
    (0x16B4, "BCM57765"),
];

/// Broadcom NetXtreme parts that work with AppleBCM5701Ethernet after a
/// device-id spoof to BCM57765 (the FakePCIID_BCM57XX list).
const BROADCOM_SPOOF: &[(u16, &str)] = &[
    (0x1641, "BCM57787"),
    (0x1642, "BCM57764"),
    (0x1643, "BCM5725"),
    (0x1644, "BCM5700"),
    (0x1645, "BCM5701"),
    (0x1646, "BCM5702"),
    (0x1647, "BCM5703"),
    (0x1655, "BCM5717"),
    (0x1656, "BCM5718"),
    (0x1657, "BCM5719"),
    (0x1665, "BCM5717"),
    (0x1683, "BCM57767"),
    (0x1687, "BCM5762"),
    (0x1688, "BCM5761"),
    (0x1689, "BCM5761"),
    (0x1690, "BCM57760"),
    (0x1691, "BCM57788"),
    (0x1692, "BCM57780"),
    (0x1693, "BCM5787M"),
    (0x1694, "BCM57790"),
    (0x1699, "BCM5785"),
    (0x16A0, "BCM5785"),
    (0x16B1, "BCM57781"),
    (0x16B2, "BCM57791"),
    (0x16B3, "BCM57786"),
    (0x16B5, "BCM57785"),
    (0x16B6, "BCM57795"),
    (0x16B7, "BCM57782"),
    (0x16F3, "BCM5727"),
];

/// Choose the Ethernet driver for a NIC by PCI id.
pub fn ethernet_driver(nic: &ProfileNic) -> EthernetDriver {
    ethernet_info(nic).driver
}

/// Full Ethernet decision for one NIC (driver, chip name, spoofs, notes).
pub fn ethernet_info(nic: &ProfileNic) -> EthernetInfo {
    use EthernetDriver as D;
    if nic.bus == DeviceBus::Usb {
        return EthernetInfo::new(D::Unsupported, display_name(nic, "USB Ethernet adapter")).note(
            "USB Ethernet adapters are not configured by the EFI. Many (RTL8153, AX88179, CDC-ECM class) work \
             natively once macOS is installed, but they cannot be relied on for the installer.",
        );
    }
    let Some((vendor, device)) = nic_ids(nic) else {
        return EthernetInfo::new(
            D::Unsupported,
            display_name(nic, "Unknown Ethernet controller"),
        )
        .note("The PCI vendor/device id is unknown, so no driver can be chosen.");
    };

    match vendor {
        VENDOR_INTEL => intel_ethernet(nic, device),
        VENDOR_REALTEK | VENDOR_DLINK => realtek_ethernet(nic, vendor, device),
        VENDOR_ATHEROS_ETH => {
            if let Some(chip) = lookup(ATHEROS_E2200, device) {
                EthernetInfo::new(D::AtherosE2200, format!("Atheros {chip}"))
            } else if let Some(chip) = lookup(ATHEROS_LEGACY, device) {
                EthernetInfo::new(D::Unsupported, format!("Atheros {chip}")).note(
                    "No maintained driver: only the old third-party AtherosL1cEthernet kext supported this \
                     controller.",
                )
            } else {
                unknown_ethernet(nic, vendor, device)
            }
        }
        VENDOR_AQUANTIA => match AQUANTIA.iter().find(|(id, _, _)| *id == device) {
            Some((_, chip, aqc113)) => {
                let mut info = EthernetInfo::new(D::NativeAquantia, format!("Aquantia {chip}")).note(
                    "Many add-in cards ship an old Aquantia firmware; update it from Windows/Linux if the link \
                     does not come up.",
                );
                info = info.note(
                    "From macOS 12 the Aquantia driver relies on a working IOMMU (AppleVTD); without VT-d the \
                     CaseySJ Aquantia kernel patches are needed.",
                );
                if *aqc113 {
                    info = info.min(MacOsVersion::Monterey);
                }
                if device == 0x80B1 {
                    info = info.note(
                        "AQC100S (SFP+) is matched by the Apple driver but no Mac uses it; support is not \
                         guaranteed.",
                    );
                }
                info
            }
            None => EthernetInfo::new(D::Unsupported, display_name(nic, "Aquantia controller")).note(
                "This Aquantia model (e.g. AQC100, AQC108, AQC111, AQC112, AQC113C, AQC115C) is not matched by \
                 the macOS Aquantia driver.",
            ),
        },
        VENDOR_BROADCOM => {
            if let Some(chip) = lookup(BROADCOM_NATIVE, device) {
                EthernetInfo::new(D::NativeBroadcom, format!("Broadcom {chip}"))
            } else if let Some(chip) = lookup(BROADCOM_SPOOF, device) {
                EthernetInfo::new(D::NativeBroadcom, format!("Broadcom {chip}"))
                    .spoof(0x16B4)
                    .note("Needs a device-id spoof to BCM57765 (14e4:16b4) so AppleBCM5701Ethernet attaches.")
            } else {
                unknown_ethernet(nic, vendor, device)
            }
        }
        VENDOR_VMWARE if device == 0x07B0 => EthernetInfo::new(D::NativeIntel, "VMware vmxnet3")
            .min(MacOsVersion::BigSur)
            .note("vmxnet3 has a built-in driver from macOS 11; use the e1000e/82545EM adapter type for older guests."),
        VENDOR_VIRTIO if matches!(device, 0x1000 | 0x1041) => EthernetInfo::new(D::NativeIntel, "VirtIO network")
            .min(MacOsVersion::BigSur)
            .note("VirtIO networking has a built-in driver from macOS 11; use vmxnet3 or e1000-82545em for older guests."),
        VENDOR_MARVELL => EthernetInfo::new(D::Unsupported, display_name(nic, "Marvell Ethernet controller"))
            .note("Marvell Yukon/Alaska controllers have no driver in current macOS releases."),
        _ => unknown_ethernet(nic, vendor, device),
    }
}

fn unknown_ethernet(nic: &ProfileNic, vendor: u16, device: u16) -> EthernetInfo {
    EthernetInfo::new(
        EthernetDriver::Unsupported,
        display_name(
            nic,
            &format!("Ethernet controller {vendor:04x}:{device:04x}"),
        ),
    )
    .note("No macOS driver is known for this controller; use a supported PCIe or USB adapter.")
}

fn intel_ethernet(nic: &ProfileNic, device: u16) -> EthernetInfo {
    use EthernetDriver as D;
    if let Some(chip) = lookup(INTEL_MAUSI, device) {
        return EthernetInfo::new(D::IntelMausi, format!("Intel {chip}"));
    }
    if let Some(chip) = lookup(INTEL_MAUSI_MIEZE_ONLY, device) {
        return EthernetInfo::new(D::IntelMausi, format!("Intel {chip}"))
            .kext("IntelMausiEthernet")
            .note("This I219 revision is only matched by Mieze's IntelMausiEthernet, not acidanthera IntelMausi.");
    }
    if let Some(chip) = lookup(INTEL_I219_NO_DRIVER, device) {
        return EthernetInfo::new(D::Unsupported, format!("Intel {chip}")).note(
            "This I219 revision (Arrow Lake or newer PCH) is not matched by any IntelMausi build yet.",
        );
    }
    if let Some(chip) = lookup(INTEL_I211_CLASS, device) {
        return EthernetInfo::new(D::IntelI211, format!("Intel {chip}")).note(
            "SmallTreeIntel82576 up to macOS 11, AppleIGB from macOS 12 (AppleIGB is deprecated upstream and \
             only ships debug builds).",
        );
    }
    if let Some(chip) = lookup(INTEL_IGB_ONLY, device) {
        return EthernetInfo::new(D::IntelI211, format!("Intel {chip}"))
            .kext("AppleIGB")
            .note("Only AppleIGB drives this controller (SmallTreeIntel82576 does not match it).");
    }
    match device {
        0x15F2 => {
            return EthernetInfo::new(D::IntelI225, "Intel I225-LM")
                .min(MacOsVersion::Catalina)
                .note(i225_note());
        }
        0x15F3 => {
            return EthernetInfo::new(D::IntelI225, "Intel I225-V")
                .spoof(0x15F2)
                .min(MacOsVersion::Catalina)
                .note("Native path: device-id spoof to I225-LM (F2150000); on 10.15-11.3 also the I225-V kernel patch.")
                .note(i225_note());
        }
        0x15F8 => {
            return EthernetInfo::new(D::IntelI225, "Intel I225-I")
                .kext("AppleIGC")
                .min(MacOsVersion::Catalina);
        }
        0x125B | 0x125C | 0x125D | 0x3102 => {
            let chip = match device {
                0x125B => "Intel I226-LM",
                0x125C => "Intel I226-V",
                0x125D => "Intel I226-IT",
                _ => "Intel I226-K",
            };
            return EthernetInfo::new(D::IntelI225, chip)
                .kext("AppleIGC")
                .min(MacOsVersion::Catalina)
                .note("AppleIGC (auto-negotiation only); the native I225 spoof is not reliable on I226.");
        }
        _ => {}
    }
    if let Some((_, chip, target)) = INTEL_IGC_SPOOF.iter().find(|e| e.0 == device) {
        return EthernetInfo::new(D::IntelI225, format!("Intel {chip}"))
            .kext("AppleIGC")
            .spoof(*target)
            .min(MacOsVersion::Catalina)
            .note(format!(
                "Not in AppleIGC's match list: a device-id spoof to 8086:{target:04x} lets it attach (its igc \
                 core handles this variant by the real id). The native I210 driver does not support it."
            ));
    }
    if let Some(chip) = lookup(INTEL_LUCY, device) {
        return EthernetInfo::new(D::IntelLucy, format!("Intel {chip} 10GbE"))
            .min(MacOsVersion::Catalina)
            .note("IntelLucy is tested on macOS 10.15-14; AppleVTD is optional.");
    }
    if let Some(chip) = lookup(INTEL_NATIVE, device) {
        let info = EthernetInfo::new(D::NativeIntel, format!("Intel {chip}"));
        return if device == 0x1533 {
            info.note(i210_note())
        } else {
            info
        };
    }
    if let Some(chip) = lookup(INTEL_I210_VARIANTS, device) {
        return EthernetInfo::new(D::NativeIntel, format!("Intel {chip}"))
            .spoof(0x1533)
            .note("Needs a device-id spoof to I210 (33150000) for AppleIntelI210Ethernet.")
            .note(i210_note());
    }
    if let Some(chip) = lookup(INTEL_I350, device) {
        return EthernetInfo::new(D::NativeIntel, format!("Intel {chip}"))
            .spoof(0x1533)
            .note("Native through a device-id spoof to I210 (33150000) on every I350 port path.")
            .note(i210_note());
    }
    if device == 0x100E {
        return EthernetInfo::new(D::Unsupported, "Intel 82540EM").note(
            "The emulated 82540EM has no macOS driver; switch the VM NIC to e1000-82545em or vmxnet3.",
        );
    }
    if let Some(chip) = lookup(INTEL_E1000E_LEGACY, device) {
        let info = EthernetInfo::new(D::Unsupported, format!("Intel {chip}")).note(
            "No maintained driver; the old third-party AppleIntelE1000e kext may work on older releases.",
        );
        return if device == 0x10D3 {
            info.note(
                "Apple's Intel82574L driver only matches the 8086:10f6 variant; users report that re-flashing \
                 the card to that id (or a device-id spoof) makes it work.",
            )
        } else {
            info
        };
    }
    unknown_ethernet(nic, VENDOR_INTEL, device)
}

fn i225_note() -> &'static str {
    "macOS 13+ replaced the AppleIntelI210Ethernet kext with a DriverKit extension that needs VT-d: use \
     AppleIGC, or inject the old AppleIntelI210Ethernet kext with boot-arg e1000=0 (dk.e1000=0 on 12.2.1 and \
     older)."
}

/// Same DriverKit caveat for I210/I350 on the native path (AppleIGC does not
/// drive them; AppleIGB matches both).
fn i210_note() -> &'static str {
    "macOS 13+ replaced the AppleIntelI210Ethernet kext with a DriverKit extension that needs VT-d: without \
     it inject the old AppleIntelI210Ethernet kext with boot-arg e1000=0 (dk.e1000=0 on 12.2.1 and older), or \
     use AppleIGB."
}

fn realtek_ethernet(nic: &ProfileNic, vendor: u16, device: u16) -> EthernetInfo {
    use EthernetDriver as D;
    if let Some(chip) = pair_lookup(REALTEK_RTL8111, vendor, device) {
        return EthernetInfo::new(D::RealtekRtl8111, format!("Realtek {chip}"))
            .note("RealtekRTL8111 2.5+ is built around AppleVTD; on AMD (no AppleVTD) its author recommends 2.4.2.");
    }
    if vendor == VENDOR_REALTEK && device == 0x8161 {
        return EthernetInfo::new(D::RealtekRtl8111, "Realtek RTL8111/8168 (8161)")
            .spoof(0x8168)
            .note("Needs a device-id spoof to 10ec:8168 so RealtekRTL8111 matches.");
    }
    if let Some(chip) = pair_lookup(REALTEK_RTL812X, vendor, device) {
        let mut info = EthernetInfo::new(D::RealtekRtl8125, format!("Realtek {chip}"))
            .min(MacOsVersion::Catalina);
        if LUCY_RTL8125_FALLBACK.contains(&(vendor, device)) {
            info = info.note(
                "If RTL812xLucy misbehaves, LucyRTL8125Ethernet is the fallback (RTL8125 only).",
            );
        }
        return info;
    }
    if vendor == VENDOR_REALTEK && device == 0x8136 {
        return EthernetInfo::new(D::RealtekRtl8100, "Realtek RTL810xE");
    }
    if vendor == VENDOR_REALTEK && device == 0x8127 {
        return EthernetInfo::new(D::Unsupported, "Realtek RTL8127 10GbE")
            .note("RTL8127 is not matched by RTL812xLucy yet.");
    }
    if vendor == VENDOR_REALTEK && matches!(device, 0x8169 | 0x8167 | 0x8139 | 0x8129 | 0x8138) {
        return EthernetInfo::new(D::Unsupported, display_name(nic, "Realtek PCI Ethernet"))
            .note("Legacy PCI Realtek controllers have no macOS driver.");
    }
    unknown_ethernet(nic, vendor, device)
}

// ── Wi-Fi ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiDriver {
    /// Intel Wi-Fi supported by OpenIntelWireless (itlwm / AirportItlwm).
    IntelItlwm,
    /// Broadcom that works natively (+AirportBrcmFixup) up to `native_max`,
    /// and with OCLP root patches after that.
    Broadcom {
        native_max: MacOsVersion,
        fixup: bool,
    },
    /// Atheros AR9xxx — native up to High Sierra, injected AirPortAtheros40 up
    /// to Big Sur, OCLP root patch after.
    AtherosLegacy,
    /// Realtek rtw88 PCIe (RTL8822BE/CE, 8821CE, 8812AE, 8814AE) — experimental rtw88.kext.
    RealtekRtw88,
    /// USB dongles and everything else — no working driver.
    Unsupported,
}

/// Everything known about one Wi-Fi card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiInfo {
    pub driver: WifiDriver,
    /// Card / chipset name ("Broadcom BCM4360", "Intel Wi-Fi 6 AX200").
    pub chip: String,
    /// First release with a working driver; None = every supported release.
    pub min_macos: Option<MacOsVersion>,
    /// Last release that works without a root patch; None = no limit (or unsupported).
    pub native_max: Option<MacOsVersion>,
    /// `device-id` DeviceProperties value (little endian) the native driver needs.
    pub device_id_spoof: Option<[u8; 4]>,
    /// Extra DeviceProperties as (key, 4-byte data), e.g. `pci-aspm-default`.
    pub extra_properties: Vec<(&'static str, [u8; 4])>,
    pub notes: Vec<String>,
}

impl WifiInfo {
    fn new(driver: WifiDriver, chip: impl Into<String>) -> Self {
        let native_max = match &driver {
            WifiDriver::Broadcom { native_max, .. } => Some(*native_max),
            WifiDriver::AtherosLegacy => Some(MacOsVersion::HighSierra),
            _ => None,
        };
        Self {
            driver,
            chip: chip.into(),
            min_macos: None,
            native_max,
            device_id_spoof: None,
            extra_properties: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    fn spoof(mut self, device: u16) -> Self {
        self.device_id_spoof = Some(le_id(device));
        self
    }
}

/// Intel ids matched by AirportItlwm/itlwm 2.3.0 (`IOPCIMatch`).
const INTEL_ITLWM: &[u16] = &[
    0x0060, 0x0064, 0x0082, 0x0083, 0x0084, 0x0085, 0x0087, 0x0089, 0x008A, 0x008B, 0x0090, 0x0091,
    0x00A0, 0x00A4, 0x0260, 0x0264, 0x02A0, 0x02F0, 0x06F0, 0x0887, 0x0888, 0x088E, 0x088F, 0x0890,
    0x0891, 0x0892, 0x0893, 0x0894, 0x0895, 0x0896, 0x0897, 0x08AE, 0x08AF, 0x08B1, 0x08B2, 0x08B3,
    0x08B4, 0x095A, 0x095B, 0x24F3, 0x24F4, 0x24F5, 0x24F6, 0x24FB, 0x24FD, 0x2526, 0x271B, 0x271C,
    0x2720, 0x2723, 0x2725, 0x2726, 0x2729, 0x30DC, 0x3165, 0x3166, 0x31DC, 0x34F0, 0x3DF0, 0x40A4,
    0x4229, 0x422B, 0x422C, 0x4230, 0x4232, 0x4235, 0x4236, 0x4237, 0x4238, 0x4239, 0x423A, 0x423B,
    0x423C, 0x423D, 0x42A4, 0x43F0, 0x4DF0, 0x51F0, 0x51F1, 0x54F0, 0x7A70, 0x7AF0, 0x7E40, 0x7F70,
    0x9DF0, 0xA0F0, 0xA370,
];

/// CNVi ids that can also host a Wi-Fi 7 (BE201/BE202) companion module.
const INTEL_CNVI_SHARED: &[u16] = &[0x51F0, 0x51F1, 0x54F0, 0x7A70, 0x7AF0, 0x7E40, 0x7F70];

/// Intel Wi-Fi without any macOS driver (Wi-Fi 7 and very old parts).
const INTEL_WIFI_UNSUPPORTED: &[(u16, &str)] = &[
    (0x272B, "Intel Wi-Fi 7 BE200"),
    (0xA840, "Intel Wi-Fi 7 BE200"),
    (0x7740, "Intel Wi-Fi 7 (CNVi)"),
    (0x4D40, "Intel Wi-Fi 7 (CNVi)"),
    (0xE340, "Intel Wi-Fi 7 (CNVi)"),
    (0xE440, "Intel Wi-Fi 7 (CNVi)"),
    (0xD340, "Intel Wi-Fi 7 (CNVi)"),
    (0xD240, "Intel Wi-Fi 7 (CNVi)"),
    (0x6E70, "Intel Wi-Fi 7 (CNVi)"),
    (0x9327, "Intel Wi-Fi 7 (CNVi)"),
    (0x0885, "Intel Centrino Wireless-N + WiMAX 6150"),
    (0x0886, "Intel Centrino Wireless-N + WiMAX 6150"),
    (0x4220, "Intel PRO/Wireless 2200BG"),
    (0x4222, "Intel PRO/Wireless 3945ABG"),
    (0x4227, "Intel PRO/Wireless 3945ABG"),
    (0x4223, "Intel PRO/Wireless 2915ABG"),
    (0x4224, "Intel PRO/Wireless 2915ABG"),
    (0x093C, "Intel WiGig 802.11ad"),
];

fn intel_wifi_name(device: u16) -> &'static str {
    match device {
        0x4229 | 0x4230 => "Intel Wireless WiFi Link 4965AGN",
        0x4232 | 0x4237 => "Intel WiFi Link 5100",
        0x4235 | 0x4236 => "Intel Ultimate N WiFi Link 5300",
        0x423A | 0x423B => "Intel WiFi Link 5350",
        0x423C | 0x423D => "Intel WiMAX/WiFi Link 5150",
        0x422B | 0x4238 => "Intel Centrino Ultimate-N 6300",
        0x422C | 0x4239 => "Intel Centrino Advanced-N 6200",
        0x0082 | 0x0085 => "Intel Centrino Advanced-N 6205",
        0x0087 | 0x0089 => "Intel Centrino Advanced-N + WiMAX 6250",
        0x0083 | 0x0084 => "Intel Centrino Wireless-N 1000",
        0x008A | 0x008B => "Intel Centrino Wireless-N 1030",
        0x0090 | 0x0091 => "Intel Centrino Advanced-N 6230",
        0x0887 | 0x0888 => "Intel Centrino Wireless-N 2230",
        0x0890 | 0x0891 => "Intel Centrino Wireless-N 2200",
        0x088E | 0x088F => "Intel Centrino Advanced-N 6235",
        0x0892 | 0x0893 => "Intel Centrino Wireless-N 135",
        0x0894 | 0x0895 => "Intel Centrino Wireless-N 105",
        0x0896 | 0x0897 => "Intel Centrino Wireless-N 130",
        0x08AE | 0x08AF => "Intel Centrino Wireless-N 100",
        0x08B1 | 0x08B2 => "Intel Wireless-AC 7260",
        0x08B3 | 0x08B4 => "Intel Wireless-AC 3160",
        0x095A | 0x095B => "Intel Wireless-AC 7265",
        0x3165 | 0x3166 => "Intel Wireless-AC 3165",
        0x24FB => "Intel Wireless-AC 3168",
        0x24F3..=0x24F6 => "Intel Wireless-AC 8260",
        0x24FD => "Intel Wireless-AC 8265",
        0x2526 | 0x271B | 0x271C => "Intel Wireless-AC 9260",
        0x9DF0 | 0xA370 | 0x31DC | 0x30DC => "Intel Wireless-AC 9560 (CNVi)",
        0x02F0 | 0x06F0 | 0x34F0 | 0x3DF0 | 0x4DF0 | 0x43F0 | 0xA0F0 => {
            "Intel Wi-Fi 6 AX201 (CNVi)"
        }
        0x2723 => "Intel Wi-Fi 6 AX200",
        0x2725 => "Intel Wi-Fi 6E AX210",
        0x7A70 | 0x7AF0 | 0x51F0 | 0x51F1 | 0x54F0 | 0x7F70 | 0x7E40 => {
            "Intel Wi-Fi 6E AX211 (CNVi)"
        }
        _ => "Intel Wireless",
    }
}

/// Broadcom Wi-Fi: (device, chip, native_max, always needs fixup, device-id spoof).
/// native_max per the AirportBrcmFixup driver matrix: AirPortBrcm4331 (432b)
/// gone in 10.15, AirPortBrcm4360 (4331/4353) gone in 11, AirPortBrcmNIC
/// (43a0/43a3/43ba) gone in 14. "Always needs fixup" marks ids that no Apple
/// driver lists (only the AirportBrcmFixup injectors or a spoof match them);
/// the natively listed ids need the fixup only without an Apple subsystem.
const BROADCOM_WIFI: &[(u16, &str, MacOsVersion, bool, Option<u16>)] = &[
    (0x43A0, "BCM4360", MacOsVersion::Ventura, false, None),
    (
        0x43A1,
        "BCM4360 (2.4 GHz)",
        MacOsVersion::Ventura,
        true,
        Some(0x43A0),
    ),
    (
        0x43A2,
        "BCM4360 (5 GHz)",
        MacOsVersion::Ventura,
        true,
        Some(0x43A0),
    ),
    (0x43BA, "BCM43602", MacOsVersion::Ventura, false, None),
    (
        0x43BB,
        "BCM43602 (2.4 GHz)",
        MacOsVersion::Ventura,
        true,
        Some(0x43BA),
    ),
    (
        0x43BC,
        "BCM43602 (5 GHz)",
        MacOsVersion::Ventura,
        true,
        Some(0x43BA),
    ),
    (0x43A3, "BCM4350", MacOsVersion::Ventura, false, None),
    (0x43B1, "BCM4352", MacOsVersion::Ventura, true, None),
    (
        0x43B2,
        "BCM4352 (2.4 GHz)",
        MacOsVersion::Ventura,
        true,
        None,
    ),
    (0x4331, "BCM4331", MacOsVersion::Catalina, false, None),
    (0x4353, "BCM43224", MacOsVersion::Catalina, false, None),
    (0x4357, "BCM43225", MacOsVersion::Catalina, true, None),
    (0x432B, "BCM4322", MacOsVersion::Mojave, false, None),
];

/// Broadcom Wi-Fi parts with no driver in any supported release (dropped in
/// Sierra or never supported).
const BROADCOM_WIFI_UNSUPPORTED: &[(u16, &str)] = &[
    (0x4311, "BCM4311"),
    (0x4312, "BCM4311"),
    (0x4313, "BCM4311"),
    (0x4315, "BCM4312"),
    (0x4318, "BCM4318"),
    (0x4319, "BCM4318"),
    (0x431A, "BCM4318"),
    (0x4320, "BCM4306"),
    (0x4324, "BCM4309"),
    (0x4325, "BCM4306"),
    (0x4328, "BCM4321"),
    (0x4329, "BCM4321"),
    (0x432A, "BCM4321"),
    (0x432C, "BCM4322"),
    (0x432D, "BCM4322"),
    (0x4358, "BCM43227"),
    (0x4359, "BCM43228"),
    (0x4365, "BCM43142"),
    (0x43A9, "BCM43217"),
    (0x43AA, "BCM43131"),
    (0x43AE, "BCM43162"),
    (0x43D3, "BCM43567"),
    (0x43D9, "BCM43570"),
    (0x43DF, "BCM4354"),
    (0x43E9, "BCM4358"),
    (0x43EC, "BCM4356"),
    (0x4727, "BCM4313"),
    (0x4415, "BCM4359"),
    (0x441F, "BCM4361"),
    (0x449D, "BCM43752"),
];

/// Broadcom chips only found in T2 and Apple-silicon Macs. macOS has a driver
/// (AppleBCMWLANCore), but it needs Apple platform data (module-instance,
/// firmware selection) that a PC does not provide.
const BROADCOM_WIFI_APPLE_T2: &[(u16, &str)] = &[
    (0x43DC, "BCM4355"),
    (0x4425, "BCM4378"),
    (0x4433, "BCM4387"),
    (0x4464, "BCM4364"),
    (0x4488, "BCM4377"),
];

/// Atheros cards served by AirPortAtheros40: (device, chip, device-id spoof).
/// Its `IONameMatch` lists 168c:1c/23/24/2a/30 (and Apple's 106b:86); AR9285
/// and AR9287 work with a spoof to AR928X, the AR946x/AR9485/AR9565/AR958x
/// generation needs a patched AirPortAtheros40 (`ATHEROS_WIFI_PATCHED`).
const ATHEROS_WIFI: &[(u16, &str, Option<u16>)] = &[
    (0x001C, "AR242x/AR542x", None),
    (0x0023, "AR5416", None),
    (0x0024, "AR5418", None),
    (0x002A, "AR928X", None),
    (0x002B, "AR9285", Some(0x002A)),
    (0x002E, "AR9287", Some(0x002A)),
    (0x0030, "AR93xx", None),
    (0x0032, "AR9485", None),
    (0x0033, "AR958x", None),
    (0x0034, "AR9462", None),
    (0x0036, "QCA9565/AR9565", None),
    (0x0037, "AR9485", None),
];

/// AR9xxx ids that AirPortAtheros40 only drives after binary patching.
const ATHEROS_WIFI_PATCHED: &[u16] = &[0x0032, 0x0033, 0x0034, 0x0036, 0x0037];

/// rtw88.kext (Feixiao 1.0.x) PCIe personalities.
const REALTEK_RTW88: &[(u16, &str)] = &[
    (0xB822, "RTL8822BE"),
    (0xC822, "RTL8822CE"),
    (0xC821, "RTL8821CE"),
    (0x8812, "RTL8812AE"),
    (0x8813, "RTL8814AE"),
];

pub fn wifi_driver(nic: &ProfileNic) -> WifiDriver {
    wifi_info(nic).driver
}

/// Full Wi-Fi decision for one card.
pub fn wifi_info(nic: &ProfileNic) -> WifiInfo {
    use WifiDriver as D;
    if matches!(nic.bus, DeviceBus::Usb | DeviceBus::Sdio) {
        return WifiInfo::new(D::Unsupported, display_name(nic, "USB Wi-Fi adapter")).note(
            "USB and SDIO Wi-Fi adapters have no driver that works in the installer; use Ethernet or a \
             supported PCIe/M.2 card.",
        );
    }
    let Some((vendor, device)) = nic_ids(nic) else {
        return WifiInfo::new(D::Unsupported, display_name(nic, "Unknown Wi-Fi card"))
            .note("The PCI vendor/device id is unknown, so no driver can be chosen.");
    };
    match vendor {
        VENDOR_INTEL => intel_wifi(device),
        VENDOR_BROADCOM => broadcom_wifi(nic, device),
        VENDOR_ATHEROS_WIFI => atheros_wifi(nic, device),
        VENDOR_APPLE if device == 0x0086 => {
            WifiInfo::new(D::AtherosLegacy, "Apple AirPort Extreme (Atheros)").note(atheros_note())
        }
        VENDOR_REALTEK => match lookup(REALTEK_RTW88, device) {
            Some(chip) => {
                let mut info = WifiInfo::new(D::RealtekRtw88, format!("Realtek {chip}"));
                info.min_macos = Some(MacOsVersion::BigSur);
                info.note(
                    "Experimental rtw88.kext: the card shows up as an Ethernet-like interface managed by its \
                     companion app; no AirDrop/Continuity.",
                )
            }
            None => WifiInfo::new(D::Unsupported, display_name(nic, "Realtek Wi-Fi"))
                .note("No macOS driver for this Realtek Wi-Fi chip (rtw89/rtl8xxx families); replace the card."),
        },
        VENDOR_MEDIATEK => WifiInfo::new(D::Unsupported, display_name(nic, "MediaTek Wi-Fi"))
            .note("MediaTek Wi-Fi has no macOS driver; replace the card or use Ethernet."),
        VENDOR_QUALCOMM => WifiInfo::new(D::Unsupported, display_name(nic, "Qualcomm Wi-Fi"))
            .note("Qualcomm Wi-Fi has no macOS driver; replace the card or use Ethernet."),
        _ => WifiInfo::new(D::Unsupported, display_name(nic, &format!("Wi-Fi card {vendor:04x}:{device:04x}")))
            .note("No macOS driver is known for this Wi-Fi card."),
    }
}

fn intel_wifi(device: u16) -> WifiInfo {
    if let Some(chip) = lookup(INTEL_WIFI_UNSUPPORTED, device) {
        return WifiInfo::new(WifiDriver::Unsupported, chip).note(
            "itlwm does not support this Intel card (Wi-Fi 7, WiMAX/WiGig or pre-4965 generation); replace it \
             with a supported Intel or Broadcom card.",
        );
    }
    if contains(INTEL_ITLWM, device) {
        let mut info = WifiInfo::new(WifiDriver::IntelItlwm, intel_wifi_name(device)).note(
            "Official AirportItlwm builds cover macOS 10.13-14 (separate 14.0 and 14.4 builds); macOS 15/26 \
             need itlwm + HeliPort (no Recovery, no AirDrop) or a community AirportItlwm build.",
        );
        if contains(INTEL_CNVI_SHARED, device) {
            info = info.note(
                "This CNVi id is shared with Wi-Fi 7 (BE201/BE202) modules, which itlwm does not support.",
            );
        }
        return info;
    }
    WifiInfo::new(
        WifiDriver::Unsupported,
        format!("Intel Wi-Fi 8086:{device:04x}"),
    )
    .note("This Intel Wi-Fi id is not in itlwm's supported list.")
}

fn broadcom_wifi(nic: &ProfileNic, device: u16) -> WifiInfo {
    if let Some((_, chip, native_max, always_fixup, spoof)) =
        BROADCOM_WIFI.iter().find(|e| e.0 == device)
    {
        let apple = has_apple_subsystem(nic);
        let fixup = *always_fixup || apple != Some(true);
        let mut info = WifiInfo::new(
            WifiDriver::Broadcom {
                native_max: *native_max,
                fixup,
            },
            format!("Broadcom {chip}"),
        );
        if let Some(target) = spoof {
            info = info.spoof(*target);
        }
        if device == 0x43A3 {
            info.extra_properties
                .push(("pci-aspm-default", [0, 0, 0, 0]));
            info = info.note("BCM4350 (DW1820A) needs pci-aspm-default = 0 to avoid freezes.");
        }
        if matches!(device, 0x43BA | 0x43A3) {
            info = info.note(
                "On macOS 15/26 AppleBCMWLANCompanion (beta, needs VT-d) can drive this chip without root patches.",
            );
        }
        let after = match native_max {
            MacOsVersion::Ventura => {
                "macOS 14 and newer need the OCLP Modern Wireless root patch (IOSkywalkFamily block, \
                 AMFIPass, SIP partly disabled)."
            }
            _ => "Newer releases need an OCLP Legacy Wireless root patch.",
        };
        return info.note(after);
    }
    if let Some(chip) = lookup(BROADCOM_WIFI_APPLE_T2, device) {
        return WifiInfo::new(WifiDriver::Unsupported, format!("Broadcom {chip}")).note(
            "Apple-only Wi-Fi module (T2 / Apple-silicon Macs): the macOS driver needs Apple platform data a \
             PC does not provide; replace it with a BCM94360/BCM94352 card.",
        );
    }
    let chip = lookup(BROADCOM_WIFI_UNSUPPORTED, device)
        .map(|c| format!("Broadcom {c}"))
        .unwrap_or_else(|| display_name(nic, &format!("Broadcom Wi-Fi 14e4:{device:04x}")));
    WifiInfo::new(WifiDriver::Unsupported, chip)
        .note("This Broadcom chip has no driver in macOS 10.13 or newer; replace it with a BCM94360/BCM94352 card.")
}

fn atheros_note() -> &'static str {
    "Native up to macOS 10.13; 10.14-11 need the High Sierra AirPortAtheros40 injected; 12+ only through an \
     OCLP root patch. Atheros cards get no Continuity features."
}

fn atheros_wifi(nic: &ProfileNic, device: u16) -> WifiInfo {
    match ATHEROS_WIFI.iter().find(|e| e.0 == device) {
        Some((_, chip, spoof)) => {
            let mut info = WifiInfo::new(WifiDriver::AtherosLegacy, format!("Atheros {chip}"))
                .note(atheros_note());
            if let Some(target) = spoof {
                info = info.spoof(*target);
            }
            if contains(ATHEROS_WIFI_PATCHED, device) {
                info = info.note(
                    "AR9485/AR946x/AR9565/AR958x are not in AirPortAtheros40's list: they need a patched \
                     AirPortAtheros40 (and a compatible spoof to pci168c,30) even on 10.13.",
                );
            }
            info
        }
        None => WifiInfo::new(
            WifiDriver::Unsupported,
            display_name(nic, &format!("Qualcomm Atheros 168c:{device:04x}")),
        )
        .note("Qualcomm Atheros 802.11ac/ax cards (QCA6174, QCA9377, ...) have no macOS driver."),
    }
}

// ── Bluetooth ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BluetoothDriver {
    /// Intel — IntelBluetoothFirmware + IntelBTPatcher (+ BlueToolFixup 12+).
    IntelBluetooth,
    /// Broadcom needing firmware upload — BrcmPatchRAM3 + BrcmFirmwareData (+ BlueToolFixup 12+).
    BroadcomPatchRam,
    /// Apple/Broadcom modules with native firmware (BCM94360/20702 Apple ids) — BlueToolFixup only on 12+.
    BroadcomNative,
    /// Realtek — RealtekBluetoothFirmware (experimental).
    Realtek,
    Unsupported,
}

/// Everything known about one Bluetooth controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothInfo {
    pub driver: BluetoothDriver,
    pub chip: String,
    /// BrcmBluetoothInjector (or IntelBluetoothInjector) is needed on 10.13-11
    /// because the USB id is not an Apple one.
    pub needs_injector: bool,
    /// BlueToolFixup is needed on macOS 12+ (every supported non-Apple id;
    /// genuine Apple modules work without it).
    pub needs_bluetoolfixup: bool,
    pub notes: Vec<String>,
}

impl BluetoothInfo {
    fn new(driver: BluetoothDriver, chip: impl Into<String>, needs_injector: bool) -> Self {
        let needs_bluetoolfixup = driver != BluetoothDriver::Unsupported;
        Self {
            driver,
            chip: chip.into(),
            needs_injector,
            needs_bluetoolfixup,
            notes: Vec::new(),
        }
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// IntelBluetoothFirmware personalities (USB 8087:xxxx).
const INTEL_BT: &[(u16, &str)] = &[
    (0x07DC, "Intel Wireless 7260 Bluetooth"),
    (0x0A2A, "Intel Wireless 7265/3165 Bluetooth"),
    (0x0AA7, "Intel Wireless-AC 3168 Bluetooth"),
    (0x0A2B, "Intel Wireless 8260/8265 Bluetooth"),
    (0x0AAA, "Intel Wireless-AC 9460/9560 Bluetooth"),
    (0x0025, "Intel Wireless-AC 9260 Bluetooth"),
    (0x0026, "Intel AX201 Bluetooth"),
    (0x0029, "Intel AX200 Bluetooth"),
    (0x0032, "Intel AX210 Bluetooth"),
    (0x0033, "Intel AX211 Bluetooth"),
    (0x0035, "Intel Bluetooth (Wi-Fi 7 generation)"),
    (0x0036, "Intel BE200 Bluetooth"),
    (0x0038, "Intel Bluetooth (Wi-Fi 7 generation)"),
];

/// BrcmPatchRAM3 2.7.2 personalities: devices that need a firmware upload.
#[rustfmt::skip]
const BROADCOM_PATCHRAM: &[(u16, u16)] = &[
    (0x0489, 0xE032), (0x0489, 0xE042), (0x0489, 0xE046), (0x0489, 0xE047), (0x0489, 0xE04F),
    (0x0489, 0xE052), (0x0489, 0xE055), (0x0489, 0xE059), (0x0489, 0xE062), (0x0489, 0xE079),
    (0x0489, 0xE07A), (0x0489, 0xE087), (0x0489, 0xE096), (0x0489, 0xE0A1), (0x04CA, 0x2003),
    (0x04CA, 0x2004), (0x04CA, 0x2005), (0x04CA, 0x2006), (0x04CA, 0x2007), (0x04CA, 0x2009),
    (0x04CA, 0x200A), (0x04CA, 0x200B), (0x04CA, 0x200C), (0x04CA, 0x200E), (0x04CA, 0x200F),
    (0x04CA, 0x2012), (0x04CA, 0x2016), (0x04F2, 0xB49D), (0x04F2, 0xB4A1), (0x050D, 0x065A),
    (0x0930, 0x021E), (0x0930, 0x021F), (0x0930, 0x0221), (0x0930, 0x0223), (0x0930, 0x0225),
    (0x0930, 0x0226), (0x0930, 0x0229), (0x0A5C, 0x2167), (0x0A5C, 0x2168), (0x0A5C, 0x2169),
    (0x0A5C, 0x216A), (0x0A5C, 0x216B), (0x0A5C, 0x216C), (0x0A5C, 0x216D), (0x0A5C, 0x216E),
    (0x0A5C, 0x216F), (0x0A5C, 0x21D3), (0x0A5C, 0x21D6), (0x0A5C, 0x21D7), (0x0A5C, 0x21D8),
    (0x0A5C, 0x21DC), (0x0A5C, 0x21DE), (0x0A5C, 0x21E1), (0x0A5C, 0x21E3), (0x0A5C, 0x21E6),
    (0x0A5C, 0x21E8), (0x0A5C, 0x21EC), (0x0A5C, 0x21F1), (0x0A5C, 0x21F3), (0x0A5C, 0x21F4),
    (0x0A5C, 0x21FB), (0x0A5C, 0x21FD), (0x0A5C, 0x21FE), (0x0A5C, 0x640B), (0x0A5C, 0x6410),
    (0x0A5C, 0x6412), (0x0A5C, 0x6413), (0x0A5C, 0x6414), (0x0A5C, 0x6417), (0x0A5C, 0x6418),
    (0x0A5C, 0x7460), (0x0B05, 0x17B5), (0x0B05, 0x17CB), (0x0B05, 0x17CF), (0x0B05, 0x180A),
    (0x0BB4, 0x0306), (0x105B, 0xE065), (0x105B, 0xE066), (0x13D3, 0x3384), (0x13D3, 0x3388),
    (0x13D3, 0x3389), (0x13D3, 0x3392), (0x13D3, 0x3404), (0x13D3, 0x3411), (0x13D3, 0x3413),
    (0x13D3, 0x3418), (0x13D3, 0x3427), (0x13D3, 0x3435), (0x13D3, 0x3456), (0x13D3, 0x3482),
    (0x13D3, 0x3484), (0x13D3, 0x3504), (0x13D3, 0x3508), (0x13D3, 0x3517), (0x145F, 0x01A3),
    (0x185F, 0x2167), (0x19FF, 0x0239), (0x413C, 0x8143), (0x413C, 0x8197),
];

/// Broadcom modules with firmware in ROM (BrcmNonPatchRAM2 and the
/// "no firmware" BrcmBluetoothInjector personalities).
const BROADCOM_BT_NATIVE: &[(u16, u16)] = &[
    (0x03F0, 0x231D),
    (0x0489, 0xE030),
    (0x0B05, 0x1788),
    (0x0B05, 0x178A),
    (0x13D3, 0x3295),
    (0x0A5C, 0x219A),
    (0x0A5C, 0x217D),
    (0x0A5C, 0x22BE),
    (0x0A5C, 0x828D),
    (0x04B4, 0xF901),
    (0x33BA, 0x03E8),
    (0x33BA, 0x03E9),
];

/// RealtekBluetoothFirmware 1.0.2 personalities.
#[rustfmt::skip]
const REALTEK_BT: &[(u16, u16)] = &[
    (0x0489, 0xE085), (0x0489, 0xE08B), (0x0489, 0xE112), (0x0489, 0xE122), (0x0489, 0xE123),
    (0x0489, 0xE125), (0x0489, 0xE12F), (0x0489, 0xE130), (0x04C5, 0x161F), (0x04C5, 0x165C),
    (0x04C5, 0x1675), (0x04CA, 0x4005), (0x04CA, 0x4006), (0x04CA, 0x4007), (0x04F2, 0xB49F),
    (0x0930, 0x021D), (0x0B05, 0x17DC), (0x0B05, 0x185C), (0x0B05, 0x18EF), (0x0B05, 0x190E),
    (0x0BDA, 0x2852), (0x0BDA, 0x385A), (0x0BDA, 0x4852), (0x0BDA, 0x4853), (0x0BDA, 0x8520),
    (0x0BDA, 0x8771), (0x0BDA, 0x887B), (0x0BDA, 0x8922), (0x0BDA, 0xB009), (0x0BDA, 0xB00C),
    (0x0BDA, 0xB850), (0x0BDA, 0xB85B), (0x0BDA, 0xC123), (0x0BDA, 0xC822), (0x0BDA, 0xC852),
    (0x0CB5, 0xC547), (0x0CB8, 0xC549), (0x0CB8, 0xC558), (0x0CB8, 0xC559), (0x1358, 0xC123),
    (0x13D3, 0x3394), (0x13D3, 0x3410), (0x13D3, 0x3414), (0x13D3, 0x3416), (0x13D3, 0x3458),
    (0x13D3, 0x3459), (0x13D3, 0x3461), (0x13D3, 0x3462), (0x13D3, 0x3494), (0x13D3, 0x3526),
    (0x13D3, 0x3529), (0x13D3, 0x3533), (0x13D3, 0x3548), (0x13D3, 0x3549), (0x13D3, 0x3553),
    (0x13D3, 0x3555), (0x13D3, 0x3570), (0x13D3, 0x3571), (0x13D3, 0x3572), (0x13D3, 0x3586),
    (0x13D3, 0x3587), (0x13D3, 0x3591), (0x13D3, 0x3592), (0x13D3, 0x3600), (0x13D3, 0x3601),
    (0x13D3, 0x3612), (0x13D3, 0x3616), (0x13D3, 0x3617), (0x13D3, 0x3618), (0x13D3, 0x3619),
    (0x2001, 0x332A), (0x2357, 0x0604), (0x2550, 0x8761), (0x2B89, 0x6275), (0x2B89, 0x8761),
    (0x2C0A, 0x8761), (0x2FF8, 0x3051), (0x2FF8, 0xB011), (0x3625, 0x010B), (0x6655, 0x8771),
    (0x7392, 0xA611), (0x7392, 0xC611), (0x7392, 0xE611),
];

pub fn bluetooth_driver(nic: &ProfileNic) -> BluetoothDriver {
    bluetooth_info(nic).driver
}

/// Full Bluetooth decision by USB VID:PID.
pub fn bluetooth_info(nic: &ProfileNic) -> BluetoothInfo {
    use BluetoothDriver as D;
    let Some((vendor, product)) = nic_ids(nic) else {
        return BluetoothInfo::new(
            D::Unsupported,
            display_name(nic, "Unknown Bluetooth controller"),
            false,
        )
        .note("The USB vendor/product id is unknown, so no driver can be chosen.");
    };
    if vendor == USB_INTEL {
        if let Some(chip) = lookup(INTEL_BT, product) {
            let mut info = BluetoothInfo::new(D::IntelBluetooth, chip, true).note(
                "macOS 26 needs IntelBTPatcher with -ibtcompatbeta, or the IntelBluetoothFirmware fork 2.5.1+.",
            );
            if matches!(product, 0x0035 | 0x0036 | 0x0038) {
                info = info.note("Wi-Fi 7 generation Bluetooth firmware is only in the IntelBluetoothFirmware fork 2.5.1+.");
            }
            return info;
        }
        let info = BluetoothInfo::new(
            D::Unsupported,
            format!("Intel Bluetooth 8087:{product:04x}"),
            false,
        )
        .note("This Intel Bluetooth id is not supported by IntelBluetoothFirmware.");
        return if product == 0x07DA {
            info.note(
                "8087:07da (Centrino 6235 generation) is only covered by IntelBluetoothInjector's CSR \
                 personality on macOS 10.13-11.",
            )
        } else {
            info
        };
    }
    if vendor == USB_APPLE {
        let mut info = BluetoothInfo::new(D::BroadcomNative, "Apple Bluetooth (Broadcom)", false).note(
            "Genuine Apple module: works natively, no firmware upload, injector or BlueToolFixup needed \
             (map its USB port as internal).",
        );
        info.needs_bluetoolfixup = false;
        return info;
    }
    if BROADCOM_PATCHRAM.contains(&(vendor, product)) {
        return BluetoothInfo::new(
            D::BroadcomPatchRam,
            display_name(nic, "Broadcom Bluetooth"),
            true,
        );
    }
    if BROADCOM_BT_NATIVE.contains(&(vendor, product)) {
        return BluetoothInfo::new(
            D::BroadcomNative,
            display_name(nic, "Broadcom Bluetooth"),
            true,
        )
        .note("Firmware is in ROM: BrcmBluetoothInjector on 10.13-11, BlueToolFixup on 12+.");
    }
    if REALTEK_BT.contains(&(vendor, product)) {
        return BluetoothInfo::new(D::Realtek, display_name(nic, "Realtek Bluetooth"), false)
            .note("RealtekBluetoothFirmware is experimental.");
    }
    let (chip, note) = match vendor {
        USB_BROADCOM => (
            "Broadcom Bluetooth",
            "This Broadcom id is not in BrcmPatchRAM's device list.",
        ),
        USB_REALTEK => (
            "Realtek Bluetooth",
            "This Realtek id is not in RealtekBluetoothFirmware's device list.",
        ),
        USB_ATHEROS => (
            "Qualcomm Atheros Bluetooth",
            "Atheros/Qualcomm Bluetooth has no current driver (Ath3kBT only covered a few AR3011/AR3012 \
             modules up to macOS 11).",
        ),
        USB_MEDIATEK => ("MediaTek Bluetooth", "MediaTek Bluetooth has no macOS driver."),
        _ => ("Bluetooth controller", "No macOS driver is known for this Bluetooth controller."),
    };
    BluetoothInfo::new(D::Unsupported, display_name(nic, chip), false).note(note)
}

// ── Touchpad ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchpadDriver {
    /// VoodooPS2Trackpad (+ VoodooInput).
    Ps2,
    /// VoodooI2C + VoodooI2CHID (precision touchpads, most ELAN/Synaptics HID).
    I2cHid,
    /// VoodooRMI over I2C (Synaptics RMI4).
    RmiI2c,
    /// VoodooRMI + VoodooSMBus (Synaptics SMBus).
    RmiSmbus,
    /// VoodooSMBus (ELAN SMBus) via VoodooPS2/ELAN.
    ElanSmbus,
    /// AlpsHID on VoodooI2C.
    AlpsHid,
    None,
}

/// Vendor implied by an ACPI/PnP hardware id ("SYNA2B33", "ELAN0662", "ALPS0001").
pub fn touchpad_vendor_from_hid(hid: &str) -> Option<TouchpadVendor> {
    let h = hid.trim().to_ascii_uppercase();
    if h.is_empty() {
        return None;
    }
    if h.starts_with("SYN") || h.contains("VID_06CB") {
        Some(TouchpadVendor::Synaptics)
    } else if h.starts_with("ELAN")
        || h.starts_with("ETD")
        || h.starts_with("ELN")
        || h.contains("VID_04F3")
    {
        Some(TouchpadVendor::Elan)
    } else if h.starts_with("ALP") || h.contains("VID_044E") {
        Some(TouchpadVendor::Alps)
    } else {
        Some(TouchpadVendor::Other)
    }
}

/// PnP ids of PS/2 pointing devices (as opposed to ACPI ids of I2C HID devices).
fn is_ps2_pnp_id(hid: &str) -> bool {
    let h = hid.trim().to_ascii_uppercase();
    h.starts_with("PNP0F")
        || (h.starts_with("SYN") && !h.starts_with("SYNA"))
        || ["ETD", "LEN", "HPQ", "IBM", "FUJ", "TOS", "SNY"]
            .iter()
            .any(|p| h.starts_with(p))
}

/// Pick the touchpad driver. Rules (Dortania ktext "Laptop input", VoodooRMI
/// README, VoodooI2C docs): PS/2 → VoodooPS2; Synaptics SMBus → VoodooRMI +
/// VoodooSMBus; ELAN SMBus → VoodooSMBus; Synaptics I2C → VoodooRMI over
/// VoodooI2C; Alps I2C/USB → AlpsHID; any other I2C HID (PNP0C50) → VoodooI2CHID.
/// USB touchpads other than Alps work as plain HID devices without a kext.
/// The bus is inferred from the ACPI/PnP id when the scanner could not tell.
pub fn touchpad_driver(
    bus: Option<InputBus>,
    vendor: Option<TouchpadVendor>,
    hid: Option<&str>,
) -> TouchpadDriver {
    let hid = hid.map(str::trim).filter(|h| !h.is_empty());
    let vendor = vendor
        .filter(|v| *v != TouchpadVendor::Unknown)
        .or_else(|| hid.and_then(touchpad_vendor_from_hid))
        .unwrap_or(TouchpadVendor::Unknown);
    let i2c = |vendor: TouchpadVendor| match vendor {
        TouchpadVendor::Synaptics => TouchpadDriver::RmiI2c,
        TouchpadVendor::Alps => TouchpadDriver::AlpsHid,
        _ => TouchpadDriver::I2cHid,
    };
    match bus {
        Some(InputBus::Ps2) => TouchpadDriver::Ps2,
        Some(InputBus::Smbus) => match vendor {
            TouchpadVendor::Synaptics => TouchpadDriver::RmiSmbus,
            TouchpadVendor::Elan => TouchpadDriver::ElanSmbus,
            _ => TouchpadDriver::Ps2,
        },
        Some(InputBus::I2c) => i2c(vendor),
        Some(InputBus::Usb) => match vendor {
            TouchpadVendor::Alps => TouchpadDriver::AlpsHid,
            _ => TouchpadDriver::None,
        },
        Some(InputBus::Unknown) | None => match hid {
            Some(h) if is_ps2_pnp_id(h) => TouchpadDriver::Ps2,
            Some(_) => i2c(vendor),
            None if bus.is_some() || vendor != TouchpadVendor::Unknown => TouchpadDriver::Ps2,
            None => TouchpadDriver::None,
        },
    }
}

// ── Storage ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAdvice {
    /// The drive cannot be used by macOS (Samsung PM981/PM991, Micron 2200S,
    /// Intel 600p, SK hynix PC601/PC611, Optane cache) or the controller is RAID/VMD.
    pub problematic: bool,
    /// NVMeFix.kext recommended (power management on non-Apple NVMe).
    pub nvmefix: bool,
    /// SATA controller needs CtlnaAHCIPort.kext (macOS 11+; SATA-unsupported.kext on 10.13-10.15).
    pub ctlna_ahci: bool,
    pub notes: Vec<String>,
}

/// Intel VMD host bridges (Linux `drivers/pci/controller/vmd.c`) plus the
/// "RST VMD Managed Controller" child (09AB).
const INTEL_VMD: &[u16] = &[
    0x09AB, 0x201D, 0x28C0, 0x28C1, 0x467F, 0x4C3D, 0x9A0B, 0xA77F, 0x7D0B, 0xAD0B, 0xB60B, 0xB06F,
    0xB07F, 0xD70B, 0xD73B,
];

/// Intel SATA controllers in RAID / RST / Optane-caching mode (pci.ids) that no
/// macOS driver attaches to. 2822/282A are handled by CtlnaAHCIPort instead.
const INTEL_SATA_RAID: &[u16] = &[
    0x02D5, 0x02D7, 0x06D5, 0x06D6, 0x06D7, 0x06DE, 0x1BA6, 0x1BD6, 0x1BF6, 0x1C04, 0x1C05, 0x1C06,
    0x1D04, 0x1D06, 0x1E04, 0x1E05, 0x1E06, 0x1E07, 0x1E0E, 0x1F24, 0x1F25, 0x1F26, 0x1F27, 0x1F2E,
    0x1F2F, 0x1F34, 0x1F35, 0x1F36, 0x1F37, 0x1F3E, 0x1F3F, 0x25B0, 0x2682, 0x2683, 0x27C3, 0x27C6,
    0x2823, 0x2826, 0x2827, 0x282B, 0x282F, 0x2925, 0x292C, 0x3A05, 0x3A25, 0x3B25, 0x3B2C, 0x43D4,
    0x43D5, 0x43D6, 0x43D7, 0x7767, 0x7F66, 0x8C04, 0x8C05, 0x8C06, 0x8C07, 0x8C0E, 0x8C0F, 0x8C84,
    0x8C85, 0x8C86, 0x8C87, 0x8C8E, 0x8C8F, 0x8D04, 0x8D06, 0x8D0E, 0x8D64, 0x8D66, 0x8D6E, 0x9C04,
    0x9C05, 0x9C06, 0x9C07, 0x9C0E, 0x9C0F, 0x9C85, 0x9C87, 0x9C8F, 0xA0D5, 0xA0D7, 0xA105, 0xA106,
    0xA107, 0xA10F, 0xA186, 0xA1D6, 0xA206, 0xA256, 0xA286, 0xA28E, 0xA355, 0xA356, 0xA357, 0xA35E,
    0xA384, 0xA386, 0xA38E,
];

/// AMD SATA RAID modes and the RAIDXpert NVMe RAID device.
const AMD_RAID: &[(u16, u16)] = &[
    (VENDOR_AMD, 0x7802),
    (VENDOR_AMD, 0x7803),
    (VENDOR_AMD, 0x7805),
    (VENDOR_AMD, 0x7902),
    (VENDOR_AMD, 0x7903),
    (VENDOR_AMD, 0xB000),
    (VENDOR_ATI, 0x4381),
    (VENDOR_ATI, 0x4392),
    (VENDOR_ATI, 0x4393),
];

/// Intel SATA controllers listed by SATA-unsupported 0.9.2 (and kept by
/// CtlnaAHCIPort): mobile 100/200/300-series AHCI and the RST "in-box
/// compatible" RAID ids.
const INTEL_SATA_CTLNA: &[u16] = &[0x2822, 0x282A, 0x9D03, 0xA103, 0xA282, 0xA353, 0x9DD3];

/// Intel Optane Memory cache devices (Optane Memory M10 and the Optane half of
/// an H10/H20, which enumerates separately).
const INTEL_OPTANE_CACHE: &[u16] = &[0x2522];

/// QLC NAND half of the Optane H10 (Teton Glacier) and H20 (Pyramid Glacier).
/// Intel: on platforms without Optane support "only the Intel QLC 3D NAND side
/// of the device will be recognized", which is the half macOS can use.
const INTEL_OPTANE_HYBRID_NAND: &[u16] = &[0x0975, 0x09AD];

/// Model-name fragments of NVMe drives macOS cannot use reliably (Dortania
/// "Storage Support"): Samsung PM981/PM981a/SM981 (MZVLB), PM991/PM991a
/// (MZVLQ/MZALQ/MZ9LQ), Micron 2200S, Intel 600p (SSDPEKKW).
const BAD_NVME_MODELS: &[(&str, &str)] = &[
    ("PM981", "Samsung PM981"),
    ("SM981", "Samsung SM981"),
    ("MZVLB", "Samsung PM981/PM981a"),
    ("PM991", "Samsung PM991"),
    ("MZVLQ", "Samsung PM991/PM991a"),
    ("MZALQ", "Samsung PM991"),
    ("MZ9LQ", "Samsung PM991"),
    ("2200S", "Micron 2200S"),
    (" 600P", "Intel 600p"),
    ("SSDPEKKW", "Intel 600p"),
    ("SSDPEKKF", "Intel Pro 6000p"),
];

/// Micron 2200 OEM part numbers look like "MTFDHBA512TCK"; other Micron
/// client drives share the MTFDHBA prefix with a different suffix.
fn is_micron_2200(model: &str) -> bool {
    model
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token.starts_with("MTFDHBA") && token.contains("TCK"))
}

pub fn is_intel_vmd(vendor: u16, device: u16) -> bool {
    vendor == VENDOR_INTEL && contains(INTEL_VMD, device)
}

pub fn is_raid_controller(vendor: u16, device: u16) -> bool {
    (vendor == VENDOR_INTEL && contains(INTEL_SATA_RAID, device))
        || AMD_RAID.contains(&(vendor, device))
}

pub fn storage_advice(drive: &ProfileStorage) -> StorageAdvice {
    let mut advice = StorageAdvice {
        problematic: false,
        nvmefix: false,
        ctlna_ahci: false,
        notes: Vec::new(),
    };
    let vendor = drive.vendor_id.as_deref().and_then(parse_id16);
    let device = drive.device_id.as_deref().and_then(parse_id16);
    let ids = vendor.zip(device);
    let model = drive.name.to_ascii_uppercase();

    if let Some((v, d)) = ids {
        if is_intel_vmd(v, d) {
            advice.problematic = true;
            advice.notes.push(
                "Intel VMD is enabled: macOS cannot see drives behind it. Disable VMD (and switch Intel RST to AHCI) \
                 in the BIOS; Windows needs the storage driver switched first or it will not boot."
                    .into(),
            );
        } else if is_raid_controller(v, d) {
            advice.problematic = true;
            advice.notes.push(
                "The SATA controller is in RAID/RST/Optane mode. Set SATA mode to AHCI in the BIOS (switch Windows \
                 to the AHCI driver first)."
                    .into(),
            );
        }
        if v == VENDOR_INTEL && contains(INTEL_SATA_CTLNA, d) {
            advice.ctlna_ahci = true;
            if matches!(d, 0x2822 | 0x282A) {
                advice.notes.push(
                    "Intel RST mode controller: AHCI mode in the BIOS is preferred; CtlnaAHCIPort lets macOS use it \
                     as long as no RAID volume is configured."
                        .into(),
                );
            }
        }
    }

    match drive.kind {
        StorageKind::Raid => {
            if !advice.problematic {
                advice.problematic = true;
                advice.notes.push(
                    "RAID / Intel RST / VMD storage is invisible to macOS. Switch the controller to AHCI and disable \
                     VMD in the BIOS."
                        .into(),
                );
            }
        }
        StorageKind::Nvme => nvme_advice(&mut advice, ids, &model),
        StorageKind::Emmc => {
            advice.problematic = true;
            advice.notes.push(
                "eMMC storage is only driven by the experimental EmeraldSDHC kext; install macOS to an NVMe/SATA \
                 drive or an external USB drive."
                    .into(),
            );
        }
        StorageKind::Usb => {
            advice.notes.push(
                "External USB drive: usable as a macOS target, not cached by the firmware.".into(),
            );
        }
        StorageKind::Sata | StorageKind::Other => {}
    }
    advice
}

fn nvme_advice(advice: &mut StorageAdvice, ids: Option<(u16, u16)>, model: &str) {
    let vendor = ids.map(|(v, _)| v);
    advice.nvmefix = vendor != Some(VENDOR_APPLE);

    let bad_model = BAD_NVME_MODELS
        .iter()
        .find(|(fragment, _)| model.contains(fragment))
        .map(|(_, name)| *name)
        .or_else(|| is_micron_2200(model).then_some("Micron 2200"));
    if let Some(name) = bad_model {
        advice.problematic = true;
        advice.notes.push(format!(
            "{name} SSDs are known to kernel panic or fail to boot macOS even with NVMeFix; use another drive for \
             macOS."
        ));
    } else if let Some((v, d)) = ids {
        let known_bad = match (v, d) {
            (VENDOR_MICRON, 0x5410) => Some("Micron 2200S"),
            (VENDOR_INTEL, 0xF1A5) => Some("Intel 600p"),
            (VENDOR_SK_HYNIX, 0x1627) => Some("SK hynix PC601"),
            (VENDOR_SK_HYNIX, 0x1639) => Some("SK hynix PC611"),
            _ => None,
        };
        if let Some(name) = known_bad {
            advice.problematic = true;
            advice.notes.push(format!(
                "{name} SSDs are known to kernel panic or fail to boot macOS; use another drive for macOS."
            ));
        }
    }

    if let Some((v, d)) = ids {
        if v == VENDOR_INTEL && contains(INTEL_OPTANE_CACHE, d) {
            advice.problematic = true;
            advice.notes.push(
                "Intel Optane Memory (cache) device: macOS does not support Optane caching. Disable Optane \
                 acceleration in RST and do not install macOS on it; on an H10/H20 use the QLC SSD half."
                    .into(),
            );
        }
        if v == VENDOR_INTEL && contains(INTEL_OPTANE_HYBRID_NAND, d) {
            advice.notes.push(
                "QLC SSD half of an Intel Optane H10/H20: usable by macOS once Optane acceleration is turned off \
                 in RST; the Optane half (8086:2522) must stay unused."
                    .into(),
            );
        }
        if v == VENDOR_SAMSUNG && d == 0xA808 && bad_model.is_none() && model.trim().is_empty() {
            advice.notes.push(
                "Samsung 970 EVO/EVO Plus and the OEM PM981/PM981a share this controller id; a PM981/PM981a \
                 (model MZVLB...) cannot run macOS."
                    .into(),
            );
        }
        if v == VENDOR_SK_HYNIX && d == 0x174A {
            advice
                .notes
                .push("SK hynix Gold P31 / PC711 works from macOS 11 only.".into());
        }
        if v == VENDOR_SAMSUNG && model.contains("970 EVO PLUS") {
            advice.notes.push(
                "Samsung 970 EVO Plus: update the firmware with Samsung Magician first; early firmware panics macOS."
                    .into(),
            );
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pci(vendor: &str, device: &str) -> ProfileNic {
        ProfileNic {
            name: String::new(),
            bus: DeviceBus::Pci,
            vendor_id: Some(vendor.into()),
            device_id: Some(device.into()),
            ..Default::default()
        }
    }

    fn pci_sub(vendor: &str, device: &str, subsystem: &str) -> ProfileNic {
        ProfileNic {
            subsystem_id: Some(subsystem.into()),
            ..pci(vendor, device)
        }
    }

    fn usb(vendor: &str, product: &str) -> ProfileNic {
        ProfileNic {
            bus: DeviceBus::Usb,
            ..pci(vendor, product)
        }
    }

    fn drive(name: &str, kind: StorageKind, vendor: &str, device: &str) -> ProfileStorage {
        ProfileStorage {
            name: name.into(),
            kind,
            vendor_id: Some(vendor.into()),
            device_id: Some(device.into()),
            size_bytes: None,
        }
    }

    fn hex(id: u16) -> String {
        format!("{id:04x}")
    }

    #[test]
    fn id_parsing_accepts_common_spellings() {
        assert_eq!(parse_id16("8086"), Some(0x8086));
        assert_eq!(parse_id16("0x15F3"), Some(0x15F3));
        assert_eq!(parse_id16(" 0X10ec "), Some(0x10EC));
        assert_eq!(parse_id16("a0f0h"), Some(0xA0F0));
        assert_eq!(parse_id16("1"), Some(1));
        assert_eq!(parse_id16(""), None);
        assert_eq!(parse_id16("0x"), None);
        assert_eq!(parse_id16("12345"), None);
        assert_eq!(parse_id16("+123"), None);
        assert_eq!(parse_id16("zz"), None);
        assert_eq!(parse_subsystem("106b0117"), Some((0x106B, 0x0117)));
        assert_eq!(parse_subsystem("0x1028:0798"), Some((0x1028, 0x0798)));
        assert_eq!(parse_subsystem("1043-8698"), Some((0x1043, 0x8698)));
        assert_eq!(parse_subsystem("106b"), None);
        assert_eq!(parse_subsystem("nothex00"), None);
    }

    #[test]
    fn tables_match_upstream_kext_lists() {
        assert_eq!(INTEL_MAUSI.len(), 51);
        assert_eq!(INTEL_MAUSI.len() + INTEL_MAUSI_MIEZE_ONLY.len(), 59);
        assert_eq!(INTEL_LUCY.len(), 46);
        assert_eq!(ATHEROS_E2200.len(), 7);
        assert_eq!(INTEL_ITLWM.len(), 87);
        assert_eq!(BROADCOM_PATCHRAM.len(), 99);
        assert_eq!(REALTEK_BT.len(), 83);
        assert_eq!(INTEL_BT.len(), 13);
        for table in [INTEL_ITLWM, INTEL_VMD, INTEL_SATA_RAID, INTEL_SATA_CTLNA] {
            let mut sorted = table.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), table.len(), "duplicate id");
        }
        for table in [BROADCOM_PATCHRAM, BROADCOM_BT_NATIVE, REALTEK_BT] {
            let mut sorted = table.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), table.len(), "duplicate USB id");
        }
    }

    #[test]
    fn missing_ids_never_panic() {
        let empty = ProfileNic::default();
        assert_eq!(ethernet_driver(&empty), EthernetDriver::Unsupported);
        assert_eq!(wifi_driver(&empty), WifiDriver::Unsupported);
        assert_eq!(bluetooth_driver(&empty), BluetoothDriver::Unsupported);
        let garbage = pci("intel", "??");
        assert_eq!(ethernet_driver(&garbage), EthernetDriver::Unsupported);
        assert!(!ethernet_info(&garbage).notes.is_empty());
    }

    #[test]
    fn every_intel_mausi_id_maps_to_mausi() {
        for (id, _) in INTEL_MAUSI {
            let info = ethernet_info(&pci("8086", &hex(*id)));
            assert_eq!(info.driver, EthernetDriver::IntelMausi, "{id:04x}");
            assert_eq!(info.preferred_kext, None, "{id:04x}");
        }
        for (id, _) in INTEL_MAUSI_MIEZE_ONLY {
            let info = ethernet_info(&pci("8086", &hex(*id)));
            assert_eq!(info.driver, EthernetDriver::IntelMausi, "{id:04x}");
            assert_eq!(info.preferred_kext, Some("IntelMausiEthernet"), "{id:04x}");
        }
        for (id, _) in INTEL_I219_NO_DRIVER {
            assert_eq!(
                ethernet_driver(&pci("8086", &hex(*id))),
                EthernetDriver::Unsupported,
                "{id:04x}"
            );
        }
        // Spot checks across generations, with and without 0x / uppercase.
        assert_eq!(ethernet_info(&pci("8086", "0x15BC")).chip, "Intel I219-V");
        assert_eq!(
            ethernet_driver(&pci("8086", "1502")),
            EthernetDriver::IntelMausi
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "153b")),
            EthernetDriver::IntelMausi
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "1a1c")),
            EthernetDriver::IntelMausi
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "0dc8")),
            EthernetDriver::IntelMausi
        );
    }

    #[test]
    fn intel_igb_i225_lucy_and_native() {
        assert_eq!(
            ethernet_driver(&pci("8086", "1539")),
            EthernetDriver::IntelI211
        );
        assert_eq!(ethernet_info(&pci("8086", "1539")).preferred_kext, None);
        let igb = ethernet_info(&pci("8086", "150e"));
        assert_eq!(igb.driver, EthernetDriver::IntelI211);
        assert_eq!(igb.preferred_kext, Some("AppleIGB"));

        let lm = ethernet_info(&pci("8086", "15f2"));
        assert_eq!(lm.driver, EthernetDriver::IntelI225);
        assert_eq!(lm.device_id_spoof, None);
        assert_eq!(lm.min_macos, Some(MacOsVersion::Catalina));
        let v = ethernet_info(&pci("8086", "15f3"));
        assert_eq!(v.driver, EthernetDriver::IntelI225);
        assert_eq!(v.device_id_spoof, Some([0xF2, 0x15, 0x00, 0x00]));
        for id in ["125b", "125c", "125d", "3102", "15f8"] {
            let info = ethernet_info(&pci("8086", id));
            assert_eq!(info.driver, EthernetDriver::IntelI225, "{id}");
            assert_eq!(info.preferred_kext, Some("AppleIGC"), "{id}");
        }
        // Variants outside AppleIGC's match list attach through a device-id spoof.
        for (id, spoof) in [
            ("0d9f", [0xF3, 0x15, 0, 0]),
            ("3100", [0xF3, 0x15, 0, 0]),
            ("3101", [0xF3, 0x15, 0, 0]),
            ("5502", [0xF3, 0x15, 0, 0]),
            ("15f7", [0xF3, 0x15, 0, 0]),
            ("5503", [0x5C, 0x12, 0, 0]),
            ("125e", [0x5C, 0x12, 0, 0]),
        ] {
            let info = ethernet_info(&pci("8086", id));
            assert_eq!(info.driver, EthernetDriver::IntelI225, "{id}");
            assert_eq!(info.preferred_kext, Some("AppleIGC"), "{id}");
            assert_eq!(info.device_id_spoof, Some(spoof), "{id}");
        }
        assert_eq!(
            ethernet_info(&pci("8086", "3100")).chip,
            "Intel I225-K (Killer E3100)"
        );
        for (_, _, target) in INTEL_IGC_SPOOF {
            assert!(matches!(target, 0x15F3 | 0x125C));
        }

        for (id, _) in INTEL_LUCY {
            assert_eq!(
                ethernet_driver(&pci("8086", &hex(*id))),
                EthernetDriver::IntelLucy,
                "{id:04x}"
            );
        }
        assert_eq!(ethernet_info(&pci("8086", "1528")).chip, "Intel X540 10GbE");

        assert_eq!(
            ethernet_driver(&pci("8086", "1533")),
            EthernetDriver::NativeIntel
        );
        let i210 = ethernet_info(&pci("8086", "1533"));
        assert_eq!(i210.device_id_spoof, None);
        assert!(i210.notes.iter().any(|n| n.contains("DriverKit")));
        assert!(!i210.notes.iter().any(|n| n.contains("AppleIGC")));
        assert_eq!(
            ethernet_info(&pci("8086", "1535")).device_id_spoof,
            Some([0x33, 0x15, 0, 0])
        );
        let i350 = ethernet_info(&pci("8086", "1521"));
        assert_eq!(i350.driver, EthernetDriver::NativeIntel);
        assert_eq!(i350.device_id_spoof, Some([0x33, 0x15, 0, 0]));
        assert_eq!(
            ethernet_info(&pci("8086", "157b")).device_id_spoof,
            Some([0x33, 0x15, 0, 0])
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "10f6")),
            EthernetDriver::NativeIntel
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "100f")),
            EthernetDriver::NativeIntel
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "100e")),
            EthernetDriver::Unsupported
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "10de")),
            EthernetDriver::Unsupported
        );
        assert_eq!(
            ethernet_driver(&pci("8086", "10d3")),
            EthernetDriver::Unsupported
        );
    }

    #[test]
    fn intel_tables_do_not_overlap() {
        let tables: [&[(u16, &str)]; 10] = [
            INTEL_MAUSI,
            INTEL_MAUSI_MIEZE_ONLY,
            INTEL_I219_NO_DRIVER,
            INTEL_E1000E_LEGACY,
            INTEL_NATIVE,
            INTEL_I210_VARIANTS,
            INTEL_I350,
            INTEL_I211_CLASS,
            INTEL_IGB_ONLY,
            INTEL_LUCY,
        ];
        let igc: Vec<u16> = INTEL_IGC_SPOOF
            .iter()
            .map(|(id, _, _)| *id)
            .chain([
                0x15F2, 0x15F3, 0x15F8, 0x125B, 0x125C, 0x125D, 0x3102, 0x100E,
            ])
            .collect();
        let mut seen = std::collections::HashSet::new();
        for id in tables
            .iter()
            .flat_map(|t| t.iter().map(|(id, _)| *id))
            .chain(igc)
        {
            assert!(seen.insert(id), "duplicate Intel Ethernet id {id:04x}");
        }
    }

    #[test]
    fn realtek_atheros_killer() {
        assert_eq!(
            ethernet_driver(&pci("10ec", "8168")),
            EthernetDriver::RealtekRtl8111
        );
        assert_eq!(
            ethernet_driver(&pci("1186", "8168")),
            EthernetDriver::RealtekRtl8111
        );
        assert_eq!(
            ethernet_driver(&pci("10ec", "2502")),
            EthernetDriver::RealtekRtl8111
        );
        assert_eq!(
            ethernet_driver(&pci("10ec", "2600")),
            EthernetDriver::RealtekRtl8111
        );
        let r8161 = ethernet_info(&pci("10ec", "8161"));
        assert_eq!(r8161.driver, EthernetDriver::RealtekRtl8111);
        assert_eq!(r8161.device_id_spoof, Some([0x68, 0x81, 0, 0]));
        for id in ["8125", "3000", "8126", "5000"] {
            let info = ethernet_info(&pci("10ec", id));
            assert_eq!(info.driver, EthernetDriver::RealtekRtl8125, "{id}");
            assert_eq!(info.min_macos, Some(MacOsVersion::Catalina));
        }
        assert_eq!(
            ethernet_driver(&pci("1186", "8125")),
            EthernetDriver::RealtekRtl8125
        );
        assert_eq!(
            ethernet_driver(&pci("10ec", "8136")),
            EthernetDriver::RealtekRtl8100
        );
        assert_eq!(
            ethernet_driver(&pci("10ec", "8127")),
            EthernetDriver::Unsupported
        );
        assert_eq!(
            ethernet_driver(&pci("10ec", "8169")),
            EthernetDriver::Unsupported
        );
        assert_eq!(
            ethernet_driver(&pci("1186", "4300")),
            EthernetDriver::Unsupported
        );

        for id in ["1090", "1091", "10a0", "10a1", "e091", "e0a1", "e0b1"] {
            assert_eq!(
                ethernet_driver(&pci("1969", id)),
                EthernetDriver::AtherosE2200,
                "{id}"
            );
        }
        assert_eq!(
            ethernet_driver(&pci("1969", "1083")),
            EthernetDriver::Unsupported
        );
        assert_eq!(
            ethernet_driver(&pci("1969", "ffff")),
            EthernetDriver::Unsupported
        );
    }

    #[test]
    fn aquantia_broadcom_vm_and_others() {
        let aqc107 = ethernet_info(&pci("1d6a", "07b1"));
        assert_eq!(aqc107.driver, EthernetDriver::NativeAquantia);
        assert_eq!(aqc107.min_macos, None);
        let aqc113 = ethernet_info(&pci("1d6a", "04c0"));
        assert_eq!(aqc113.driver, EthernetDriver::NativeAquantia);
        assert_eq!(aqc113.min_macos, Some(MacOsVersion::Monterey));
        assert_eq!(
            ethernet_driver(&pci("1d6a", "80b1")),
            EthernetDriver::NativeAquantia
        );
        // Not in the Apple driver's IONameMatch.
        for id in ["11b1", "00b1", "14c0", "12c0", "08b1"] {
            assert_eq!(
                ethernet_driver(&pci("1d6a", id)),
                EthernetDriver::Unsupported,
                "{id}"
            );
        }

        assert_eq!(
            ethernet_driver(&pci("14e4", "1686")),
            EthernetDriver::NativeBroadcom
        );
        assert_eq!(ethernet_info(&pci("14e4", "16b4")).device_id_spoof, None);
        let spoofed = ethernet_info(&pci("14e4", "1691"));
        assert_eq!(spoofed.driver, EthernetDriver::NativeBroadcom);
        assert_eq!(spoofed.device_id_spoof, Some([0xB4, 0x16, 0, 0]));
        assert_eq!(
            ethernet_driver(&pci("14e4", "165f")),
            EthernetDriver::Unsupported
        );

        assert_eq!(
            ethernet_driver(&pci("15ad", "07b0")),
            EthernetDriver::NativeIntel
        );
        assert_eq!(
            ethernet_info(&pci("1af4", "1041")).min_macos,
            Some(MacOsVersion::BigSur)
        );
        assert_eq!(
            ethernet_driver(&pci("11ab", "4362")),
            EthernetDriver::Unsupported
        );
        assert_eq!(
            ethernet_driver(&pci("1234", "5678")),
            EthernetDriver::Unsupported
        );
    }

    #[test]
    fn usb_ethernet_is_unsupported_with_note() {
        let info = ethernet_info(&usb("0bda", "8153"));
        assert_eq!(info.driver, EthernetDriver::Unsupported);
        assert!(info.notes[0].contains("USB"));
    }

    #[test]
    fn intel_wifi() {
        for id in INTEL_ITLWM {
            assert_eq!(
                wifi_driver(&pci("8086", &hex(*id))),
                WifiDriver::IntelItlwm,
                "{id:04x}"
            );
        }
        assert_eq!(wifi_info(&pci("8086", "2723")).chip, "Intel Wi-Fi 6 AX200");
        assert_eq!(
            wifi_info(&pci("8086", "a0f0")).chip,
            "Intel Wi-Fi 6 AX201 (CNVi)"
        );
        assert_eq!(wifi_info(&pci("8086", "2725")).chip, "Intel Wi-Fi 6E AX210");
        assert_eq!(
            wifi_info(&pci("8086", "24fd")).chip,
            "Intel Wireless-AC 8265"
        );
        assert!(wifi_info(&pci("8086", "7af0"))
            .notes
            .iter()
            .any(|n| n.contains("Wi-Fi 7")));
        for (id, _) in INTEL_WIFI_UNSUPPORTED {
            assert_eq!(
                wifi_driver(&pci("8086", &hex(*id))),
                WifiDriver::Unsupported,
                "{id:04x}"
            );
            assert!(!contains(INTEL_ITLWM, *id), "{id:04x} in both lists");
        }
        assert_eq!(wifi_driver(&pci("8086", "272b")), WifiDriver::Unsupported);
        assert_eq!(wifi_driver(&pci("8086", "1234")), WifiDriver::Unsupported);
    }

    #[test]
    fn broadcom_wifi_versions_and_fixup() {
        let third_party = wifi_driver(&pci_sub("14e4", "43a0", "10438698"));
        assert_eq!(
            third_party,
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: true
            }
        );
        let apple = wifi_driver(&pci_sub("14e4", "43a0", "106b0117"));
        assert_eq!(
            apple,
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: false
            }
        );
        // Device-first subsystem order is recognised too.
        let apple_swapped = wifi_driver(&pci_sub("14e4", "43a0", "0117106b"));
        assert_eq!(
            apple_swapped,
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: false
            }
        );
        // Unknown subsystem → fixup to be safe.
        assert_eq!(
            wifi_driver(&pci("14e4", "43ba")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: true
            }
        );
        // BCM4352 always needs the fixup, even with an Apple subsystem.
        assert_eq!(
            wifi_driver(&pci_sub("14e4", "43b1", "106b0000")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: true
            }
        );
        let bcm4350 = wifi_info(&pci_sub("14e4", "43a3", "10280023"));
        assert_eq!(
            bcm4350.driver,
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: true
            }
        );
        assert_eq!(bcm4350.native_max, Some(MacOsVersion::Ventura));
        assert_eq!(
            bcm4350.extra_properties,
            vec![("pci-aspm-default", [0, 0, 0, 0])]
        );
        assert_eq!(
            wifi_info(&pci("14e4", "43a1")).device_id_spoof,
            Some([0xA0, 0x43, 0, 0])
        );
        assert_eq!(
            wifi_driver(&pci_sub("14e4", "4331", "106b00f5")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Catalina,
                fixup: false
            }
        );
        assert_eq!(
            wifi_driver(&pci("14e4", "4353")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Catalina,
                fixup: true
            }
        );
        assert_eq!(
            wifi_driver(&pci("14e4", "432b")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Mojave,
                fixup: true
            }
        );
        // Natively listed ids skip the fixup on a genuine Apple card; injector-only ids never do.
        assert_eq!(
            wifi_driver(&pci_sub("14e4", "43a3", "106b0000")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Ventura,
                fixup: false
            }
        );
        assert_eq!(
            wifi_driver(&pci_sub("14e4", "4357", "106b0000")),
            WifiDriver::Broadcom {
                native_max: MacOsVersion::Catalina,
                fixup: true
            }
        );
        for (id, _) in BROADCOM_WIFI_UNSUPPORTED
            .iter()
            .chain(BROADCOM_WIFI_APPLE_T2)
        {
            assert_eq!(
                wifi_driver(&pci("14e4", &hex(*id))),
                WifiDriver::Unsupported,
                "{id:04x}"
            );
            assert!(!BROADCOM_WIFI.iter().any(|e| e.0 == *id), "{id:04x}");
        }
        assert!(wifi_info(&pci("14e4", "4464")).notes[0].contains("Apple-only"));
        assert!(wifi_info(&pci("14e4", "4727")).notes[0].contains("no driver"));
    }

    #[test]
    fn atheros_realtek_and_unsupported_wifi() {
        for id in ["002a", "002b", "002e", "0030", "0032", "0034", "0036"] {
            let info = wifi_info(&pci("168c", id));
            assert_eq!(info.driver, WifiDriver::AtherosLegacy, "{id}");
            assert_eq!(info.native_max, Some(MacOsVersion::HighSierra));
        }
        assert_eq!(
            wifi_info(&pci("168c", "002b")).device_id_spoof,
            Some([0x2A, 0, 0, 0])
        );
        assert_eq!(
            wifi_info(&pci("168c", "002e")).device_id_spoof,
            Some([0x2A, 0, 0, 0])
        );
        // Natively listed by AirPortAtheros40: no spoof, no patch note.
        for id in ["001c", "0023", "0024", "002a", "0030"] {
            let info = wifi_info(&pci("168c", id));
            assert_eq!(info.driver, WifiDriver::AtherosLegacy, "{id}");
            assert_eq!(info.device_id_spoof, None, "{id}");
            assert_eq!(info.notes.len(), 1, "{id}");
        }
        for id in ["0032", "0033", "0034", "0036", "0037"] {
            let info = wifi_info(&pci("168c", id));
            assert_eq!(info.driver, WifiDriver::AtherosLegacy, "{id}");
            assert!(info.notes.iter().any(|n| n.contains("patched")), "{id}");
        }
        assert_eq!(wifi_driver(&pci("106b", "0086")), WifiDriver::AtherosLegacy);
        assert_eq!(wifi_driver(&pci("168c", "003e")), WifiDriver::Unsupported);
        assert_eq!(wifi_driver(&pci("168c", "0042")), WifiDriver::Unsupported);

        for id in ["b822", "c822", "c821", "8812", "8813"] {
            let info = wifi_info(&pci("10ec", id));
            assert_eq!(info.driver, WifiDriver::RealtekRtw88, "{id}");
            assert_eq!(info.min_macos, Some(MacOsVersion::BigSur));
        }
        assert_eq!(wifi_driver(&pci("10ec", "8852")), WifiDriver::Unsupported);
        assert_eq!(wifi_driver(&pci("14c3", "0616")), WifiDriver::Unsupported);
        assert_eq!(wifi_driver(&pci("17cb", "1103")), WifiDriver::Unsupported);
        // USB dongles are never supported, even with a supported chip id.
        assert_eq!(wifi_driver(&usb("8086", "2723")), WifiDriver::Unsupported);
        assert_eq!(wifi_driver(&usb("0bda", "b822")), WifiDriver::Unsupported);
    }

    #[test]
    fn bluetooth() {
        for (id, _) in INTEL_BT {
            assert_eq!(
                bluetooth_driver(&usb("8087", &hex(*id))),
                BluetoothDriver::IntelBluetooth,
                "{id:04x}"
            );
        }
        assert_eq!(
            bluetooth_driver(&usb("8087", "07da")),
            BluetoothDriver::Unsupported
        );
        for (v, p) in BROADCOM_PATCHRAM {
            assert_eq!(
                bluetooth_driver(&usb(&hex(*v), &hex(*p))),
                BluetoothDriver::BroadcomPatchRam
            );
            assert!(!BROADCOM_BT_NATIVE.contains(&(*v, *p)));
            assert!(
                !REALTEK_BT.contains(&(*v, *p)),
                "{v:04x}:{p:04x} in two lists"
            );
        }
        let native = bluetooth_info(&usb("0a5c", "828d"));
        assert_eq!(native.driver, BluetoothDriver::BroadcomNative);
        assert!(native.needs_injector && native.needs_bluetoolfixup);
        let apple = bluetooth_info(&usb("05ac", "828d"));
        assert_eq!(apple.driver, BluetoothDriver::BroadcomNative);
        assert!(!apple.needs_injector && !apple.needs_bluetoolfixup);
        let intel = bluetooth_info(&usb("8087", "0029"));
        assert!(intel.needs_injector && intel.needs_bluetoolfixup);
        let patchram = bluetooth_info(&usb("0a5c", "21e8"));
        assert_eq!(patchram.driver, BluetoothDriver::BroadcomPatchRam);
        assert!(patchram.needs_injector && patchram.needs_bluetoolfixup);
        let realtek = bluetooth_info(&usb("0bda", "b00c"));
        assert!(!realtek.needs_injector && realtek.needs_bluetoolfixup);
        let csr = bluetooth_info(&usb("8087", "07da"));
        assert_eq!(csr.driver, BluetoothDriver::Unsupported);
        assert!(!csr.needs_bluetoolfixup && csr.notes.len() == 2);
        assert_eq!(
            bluetooth_driver(&usb("0a5c", "ffff")),
            BluetoothDriver::Unsupported
        );
        for (v, p) in REALTEK_BT {
            assert_eq!(
                bluetooth_driver(&usb(&hex(*v), &hex(*p))),
                BluetoothDriver::Realtek
            );
        }
        assert_eq!(
            bluetooth_driver(&usb("0bda", "0001")),
            BluetoothDriver::Unsupported
        );
        assert_eq!(
            bluetooth_driver(&usb("0cf3", "3004")),
            BluetoothDriver::Unsupported
        );
        assert_eq!(
            bluetooth_driver(&usb("0e8d", "0616")),
            BluetoothDriver::Unsupported
        );
    }

    #[test]
    fn touchpads() {
        use InputBus as B;
        use TouchpadVendor as V;
        assert_eq!(touchpad_driver(None, None, None), TouchpadDriver::None);
        assert_eq!(
            touchpad_driver(Some(B::Ps2), Some(V::Synaptics), None),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(Some(B::Ps2), Some(V::Elan), Some("ETD0108")),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(Some(B::Smbus), Some(V::Synaptics), None),
            TouchpadDriver::RmiSmbus
        );
        assert_eq!(
            touchpad_driver(Some(B::Smbus), Some(V::Elan), None),
            TouchpadDriver::ElanSmbus
        );
        assert_eq!(
            touchpad_driver(Some(B::Smbus), Some(V::Alps), None),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(Some(B::I2c), Some(V::Synaptics), None),
            TouchpadDriver::RmiI2c
        );
        assert_eq!(
            touchpad_driver(Some(B::I2c), None, Some("SYNA2B33")),
            TouchpadDriver::RmiI2c
        );
        assert_eq!(
            touchpad_driver(Some(B::I2c), Some(V::Elan), Some("ELAN0662")),
            TouchpadDriver::I2cHid
        );
        assert_eq!(
            touchpad_driver(Some(B::I2c), Some(V::Alps), None),
            TouchpadDriver::AlpsHid
        );
        assert_eq!(
            touchpad_driver(Some(B::I2c), Some(V::Other), Some("MSFT0001")),
            TouchpadDriver::I2cHid
        );
        assert_eq!(
            touchpad_driver(Some(B::I2c), Some(V::Unknown), Some("FTE1001")),
            TouchpadDriver::I2cHid
        );
        assert_eq!(
            touchpad_driver(Some(B::Usb), Some(V::Alps), None),
            TouchpadDriver::AlpsHid
        );
        assert_eq!(
            touchpad_driver(Some(B::Usb), Some(V::Synaptics), None),
            TouchpadDriver::None
        );
        // Unknown bus: infer from the hardware id.
        assert_eq!(
            touchpad_driver(Some(B::Unknown), None, Some("SYNA3602")),
            TouchpadDriver::RmiI2c
        );
        assert_eq!(
            touchpad_driver(None, None, Some("ELAN1200")),
            TouchpadDriver::I2cHid
        );
        assert_eq!(
            touchpad_driver(None, None, Some("SYN1219")),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(None, None, Some("PNP0F13")),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(None, None, Some("LEN0068")),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(Some(B::Unknown), None, None),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(None, Some(V::Elan), None),
            TouchpadDriver::Ps2
        );
        assert_eq!(
            touchpad_driver(None, None, Some("  ")),
            TouchpadDriver::None
        );
    }

    #[test]
    fn touchpad_vendor_inference() {
        assert_eq!(
            touchpad_vendor_from_hid("syna2b33"),
            Some(TouchpadVendor::Synaptics)
        );
        assert_eq!(
            touchpad_vendor_from_hid("HID\\VID_06CB&PID_CD8B"),
            Some(TouchpadVendor::Synaptics)
        );
        assert_eq!(
            touchpad_vendor_from_hid("ELAN0662"),
            Some(TouchpadVendor::Elan)
        );
        assert_eq!(
            touchpad_vendor_from_hid("ALPS0001"),
            Some(TouchpadVendor::Alps)
        );
        assert_eq!(
            touchpad_vendor_from_hid("MSFT0001"),
            Some(TouchpadVendor::Other)
        );
        assert_eq!(touchpad_vendor_from_hid(""), None);
    }

    #[test]
    fn storage_nvme() {
        let pm981 = storage_advice(&drive(
            "SAMSUNG MZVLB512HAJQ-000L7",
            StorageKind::Nvme,
            "144d",
            "a808",
        ));
        assert!(pm981.problematic && pm981.nvmefix);
        let evo = storage_advice(&drive(
            "Samsung SSD 970 EVO Plus 1TB",
            StorageKind::Nvme,
            "144d",
            "a808",
        ));
        assert!(!evo.problematic && evo.nvmefix);
        assert!(evo.notes.iter().any(|n| n.contains("firmware")));
        assert!(
            storage_advice(&drive(
                "SAMSUNG MZVLQ512HBLU-00B00",
                StorageKind::Nvme,
                "144d",
                "a809"
            ))
            .problematic
        );
        assert!(
            !storage_advice(&drive(
                "Samsung SSD 980 1TB",
                StorageKind::Nvme,
                "144d",
                "a809"
            ))
            .problematic
        );
        assert!(
            storage_advice(&drive(
                "Micron 2200S NVMe 512GB",
                StorageKind::Nvme,
                "1344",
                "5410"
            ))
            .problematic
        );
        assert!(
            storage_advice(&drive("MTFDHBA512TCK-1AS1AABHA", StorageKind::Nvme, "", ""))
                .problematic
        );
        assert!(
            !storage_advice(&drive(
                "MTFDHBA512TDV-1AZ1AABHA",
                StorageKind::Nvme,
                "1344",
                "5405"
            ))
            .problematic
        );
        assert!(storage_advice(&drive("", StorageKind::Nvme, "1344", "5410")).problematic);
        assert!(
            storage_advice(&drive(
                "INTEL SSDPEKKW256G7",
                StorageKind::Nvme,
                "8086",
                "f1a5"
            ))
            .problematic
        );
        assert!(
            !storage_advice(&drive(
                "INTEL SSDPEKNW512G8",
                StorageKind::Nvme,
                "8086",
                "f1a8"
            ))
            .problematic
        );
        assert!(
            storage_advice(&drive("SK hynix PC601", StorageKind::Nvme, "1c5c", "1627")).problematic
        );
        let p31 = storage_advice(&drive("SHGP31-1000GM", StorageKind::Nvme, "1c5c", "174a"));
        assert!(!p31.problematic && p31.notes.iter().any(|n| n.contains("macOS 11")));
        // Optane H10: the QLC half (0975) is usable, the Optane half (2522) is not.
        let h10_nand = storage_advice(&drive(
            "INTEL HBRPEKNX0101AH",
            StorageKind::Nvme,
            "8086",
            "0975",
        ));
        assert!(!h10_nand.problematic && h10_nand.nvmefix);
        assert!(h10_nand.notes.iter().any(|n| n.contains("2522")));
        assert!(
            storage_advice(&drive(
                "INTEL HBRPEKNX0101AHO",
                StorageKind::Nvme,
                "8086",
                "2522"
            ))
            .problematic
        );
        let unnamed_samsung = storage_advice(&drive("", StorageKind::Nvme, "144d", "a808"));
        assert!(!unnamed_samsung.problematic);
        assert!(unnamed_samsung.notes.iter().any(|n| n.contains("PM981")));
        let wd = storage_advice(&drive("WDC WDS500G2B0C", StorageKind::Nvme, "15b7", "5009"));
        assert!(!wd.problematic && wd.nvmefix && !wd.ctlna_ahci);
        let apple = storage_advice(&drive(
            "APPLE SSD AP0256M",
            StorageKind::Nvme,
            "106b",
            "2003",
        ));
        assert!(!apple.nvmefix);
        let unknown = storage_advice(&ProfileStorage {
            kind: StorageKind::Nvme,
            ..Default::default()
        });
        assert!(unknown.nvmefix && !unknown.problematic);
    }

    #[test]
    fn storage_vmd_raid_sata() {
        for id in INTEL_VMD {
            let a = storage_advice(&drive("RST VMD", StorageKind::Nvme, "8086", &hex(*id)));
            assert!(a.problematic, "{id:04x}");
            assert!(a.notes.iter().any(|n| n.contains("VMD")));
        }
        for id in INTEL_SATA_RAID {
            assert!(
                storage_advice(&drive("RAID", StorageKind::Sata, "8086", &hex(*id))).problematic,
                "{id:04x}"
            );
            assert!(!contains(INTEL_SATA_CTLNA, *id));
        }
        assert!(storage_advice(&drive("RAID", StorageKind::Raid, "1022", "7905")).problematic);
        assert!(storage_advice(&drive("RAIDXpert", StorageKind::Raid, "1022", "b000")).problematic);
        assert!(
            storage_advice(&ProfileStorage {
                kind: StorageKind::Raid,
                ..Default::default()
            })
            .problematic
        );

        for id in INTEL_SATA_CTLNA {
            let a = storage_advice(&drive("SATA", StorageKind::Sata, "8086", &hex(*id)));
            assert!(a.ctlna_ahci && !a.problematic, "{id:04x}");
            assert!(!a.nvmefix);
        }
        let z390 = storage_advice(&drive(
            "Samsung SSD 860 EVO",
            StorageKind::Sata,
            "8086",
            "a352",
        ));
        assert!(!z390.problematic && !z390.ctlna_ahci && !z390.nvmefix);
        assert!(!storage_advice(&drive("SATA", StorageKind::Sata, "1022", "7901")).ctlna_ahci);
        assert!(storage_advice(&drive("eMMC", StorageKind::Emmc, "", "")).problematic);
        assert!(!storage_advice(&drive("USB", StorageKind::Usb, "", "")).problematic);
    }
}
