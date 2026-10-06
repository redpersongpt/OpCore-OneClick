//! Shared domain model.
//!
//! These types are the contract between the scanner, the knowledge base, the
//! planner, the config writer and the frontend. Everything that crosses the IPC
//! boundary derives `TS` so the TypeScript bindings in `src/bridge/generated`
//! stay in sync with the backend.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ── macOS releases ──────────────────────────────────────────────────────────

/// Every macOS release an Intel/AMD PC can boot through OpenCore.
/// macOS 26 Tahoe is the final x86 release; macOS 27 is Apple silicon only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MacOsVersion {
    #[serde(rename = "10.13")]
    HighSierra,
    #[serde(rename = "10.14")]
    Mojave,
    #[serde(rename = "10.15")]
    Catalina,
    #[serde(rename = "11")]
    BigSur,
    #[serde(rename = "12")]
    Monterey,
    #[serde(rename = "13")]
    Ventura,
    #[serde(rename = "14")]
    Sonoma,
    #[serde(rename = "15")]
    Sequoia,
    #[serde(rename = "26")]
    Tahoe,
}

impl MacOsVersion {
    pub const ALL: [MacOsVersion; 9] = [
        MacOsVersion::HighSierra,
        MacOsVersion::Mojave,
        MacOsVersion::Catalina,
        MacOsVersion::BigSur,
        MacOsVersion::Monterey,
        MacOsVersion::Ventura,
        MacOsVersion::Sonoma,
        MacOsVersion::Sequoia,
        MacOsVersion::Tahoe,
    ];

    /// Newest release first.
    pub fn newest_first() -> impl Iterator<Item = MacOsVersion> {
        Self::ALL.into_iter().rev()
    }

    /// Stable identifier used across IPC ("10.13" .. "26").
    pub fn id(self) -> &'static str {
        match self {
            MacOsVersion::HighSierra => "10.13",
            MacOsVersion::Mojave => "10.14",
            MacOsVersion::Catalina => "10.15",
            MacOsVersion::BigSur => "11",
            MacOsVersion::Monterey => "12",
            MacOsVersion::Ventura => "13",
            MacOsVersion::Sonoma => "14",
            MacOsVersion::Sequoia => "15",
            MacOsVersion::Tahoe => "26",
        }
    }

    pub fn marketing_name(self) -> &'static str {
        match self {
            MacOsVersion::HighSierra => "High Sierra",
            MacOsVersion::Mojave => "Mojave",
            MacOsVersion::Catalina => "Catalina",
            MacOsVersion::BigSur => "Big Sur",
            MacOsVersion::Monterey => "Monterey",
            MacOsVersion::Ventura => "Ventura",
            MacOsVersion::Sonoma => "Sonoma",
            MacOsVersion::Sequoia => "Sequoia",
            MacOsVersion::Tahoe => "Tahoe",
        }
    }

    /// "macOS Sequoia 15"
    pub fn display_name(self) -> String {
        format!("macOS {} {}", self.marketing_name(), self.id())
    }

    /// Darwin kernel major version (Tahoe = 25).
    pub fn darwin_major(self) -> u32 {
        match self {
            MacOsVersion::HighSierra => 17,
            MacOsVersion::Mojave => 18,
            MacOsVersion::Catalina => 19,
            MacOsVersion::BigSur => 20,
            MacOsVersion::Monterey => 21,
            MacOsVersion::Ventura => 22,
            MacOsVersion::Sonoma => 23,
            MacOsVersion::Sequoia => 24,
            MacOsVersion::Tahoe => 25,
        }
    }

    /// "25.0.0" — for Kernel->Add MinKernel.
    pub fn min_kernel(self) -> String {
        format!("{}.0.0", self.darwin_major())
    }

    /// "25.99.99" — for Kernel->Add MaxKernel.
    pub fn max_kernel(self) -> String {
        format!("{}.99.99", self.darwin_major())
    }

    /// Parse any reasonable spelling: "15", "15.7", "macOS Sequoia 15", "sequoia",
    /// "10.15", "Catalina". Returns None for unknown or Apple-silicon-only releases.
    pub fn parse(input: &str) -> Option<MacOsVersion> {
        let lower = input.trim().to_lowercase();
        if lower.is_empty() {
            return None;
        }
        for version in Self::ALL {
            if lower.contains(&version.marketing_name().to_lowercase()) {
                return Some(version);
            }
        }
        let token = lower
            .split(|c: char| !(c.is_ascii_digit() || c == '.')).rfind(|t| !t.is_empty() && t.chars().next().is_some_and(|c| c.is_ascii_digit()))?
            .to_string();
        let mut parts = token.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
        match (major, minor) {
            (10, 13) => Some(MacOsVersion::HighSierra),
            (10, 14) => Some(MacOsVersion::Mojave),
            (10, 15) => Some(MacOsVersion::Catalina),
            (11, _) => Some(MacOsVersion::BigSur),
            (12, _) => Some(MacOsVersion::Monterey),
            (13, _) => Some(MacOsVersion::Ventura),
            (14, _) => Some(MacOsVersion::Sonoma),
            (15, _) => Some(MacOsVersion::Sequoia),
            (26, _) => Some(MacOsVersion::Tahoe),
            _ => None,
        }
    }
}

// ── CPU ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CpuVendor {
    Intel,
    Amd,
    Apple,
    #[default]
    Unknown,
}

/// Microarchitecture / platform the OpenCore guide is organised by.
/// Intel consumer, Intel HEDT/server, low-power Atom-class, AMD families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CpuPlatform {
    // Intel consumer (desktop + mobile)
    Penryn,
    /// Lynnfield + Clarkdale desktop (1st gen Core i3/i5/i7 7xx/8xx/5xx/6xx)
    Lynnfield,
    /// 1st gen mobile Core (i3/i5/i7 3xx/4xx/5xx/6xxM)
    Arrandale,
    SandyBridge,
    IvyBridge,
    Haswell,
    Broadwell,
    Skylake,
    /// Kaby Lake, Kaby Lake-R, Amber Lake
    KabyLake,
    /// Coffee Lake, Coffee Lake-R, Whiskey Lake
    CoffeeLake,
    CometLake,
    IceLake,
    RocketLake,
    TigerLake,
    AlderLake,
    RaptorLake,
    MeteorLake,
    ArrowLake,
    LunarLake,
    // Intel HEDT / workstation / server
    /// Bloomfield / Gulftown / Xeon 55xx-56xx (X58)
    NehalemHedt,
    SandyBridgeE,
    IvyBridgeE,
    HaswellE,
    BroadwellE,
    SkylakeX,
    CascadeLakeX,
    /// Ice Lake-SP/W, Sapphire Rapids, Emerald Rapids, Granite Rapids
    XeonModernUnsupported,
    // Intel low power
    /// Bonnell..Tremont Atom, Celeron/Pentium N/J/Silver, Alder Lake-N/N100
    IntelAtom,
    // AMD
    /// Family 10h and older (Phenom, Athlon II) — no AMD_Vanilla support
    AmdK10,
    /// Family 15h (FX, Bulldozer..Excavator APUs)
    AmdBulldozer,
    /// Family 16h (Jaguar/Puma)
    AmdJaguar,
    /// Family 17h Zen / Zen+ (Ryzen 1000/2000, Raven/Picasso APUs)
    AmdZen,
    /// Family 17h Zen 2 (Ryzen 3000, Renoir/Lucienne APUs, TR 3000)
    AmdZen2,
    /// Family 19h Zen 3 / Zen 3+ (Ryzen 5000, Cezanne/Barcelo/Rembrandt)
    AmdZen3,
    /// Family 19h Zen 4 (Ryzen 7000/8000, Phoenix/Hawk Point, TR 7000)
    AmdZen4,
    /// Family 1Ah Zen 5 (Ryzen 9000, Strix Point)
    AmdZen5,
    AppleSilicon,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FormFactor {
    #[default]
    Desktop,
    Laptop,
    /// All-in-one: desktop-like board with an internal panel.
    AllInOne,
    /// NUC / mini PC with a mobile CPU but no internal panel or battery.
    MiniPc,
}

impl FormFactor {
    pub fn has_internal_panel(self) -> bool {
        matches!(self, FormFactor::Laptop | FormFactor::AllInOne)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VmKind {
    Kvm,
    Vmware,
    HyperV,
    VirtualBox,
    Parallels,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileCpu {
    pub name: String,
    pub vendor: CpuVendor,
    pub platform: CpuPlatform,
    /// Display codename ("Coffee Lake-S", "Vermeer", ...).
    pub codename: String,
    pub family: Option<u32>,
    pub model: Option<u32>,
    pub stepping: Option<u32>,
    /// Physical cores (AMD core-count patch needs physical cores per package).
    pub cores: u32,
    pub threads: u32,
    pub is_mobile: bool,
    /// Actual instruction flags when the scanner exposes them.
    #[serde(default)]
    #[ts(optional)]
    pub has_avx: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub has_rdrand: Option<bool>,
    /// AVX2 is required for macOS 13+ without CryptexFixup.
    pub has_avx2: Option<bool>,
    pub has_sse4_2: Option<bool>,
    /// Hybrid P/E-core design (Alder/Raptor/Arrow Lake).
    pub is_hybrid: bool,
}

impl ProfileCpu {
    pub fn lacks_avx(&self) -> bool {
        self.has_avx.map(|v| !v).unwrap_or_else(|| {
            matches!(
                self.platform,
                CpuPlatform::Penryn
                    | CpuPlatform::Lynnfield
                    | CpuPlatform::Arrandale
                    | CpuPlatform::NehalemHedt
            ) || (self.vendor == CpuVendor::Intel
                && !self.has_avx2.unwrap_or(false)
                && (self.name.to_ascii_lowercase().contains("pentium")
                    || self.name.to_ascii_lowercase().contains("celeron")))
        })
    }

    pub fn lacks_rdrand(&self) -> bool {
        self.has_rdrand.map(|v| !v).unwrap_or_else(|| {
            matches!(
                self.platform,
                CpuPlatform::Penryn
                    | CpuPlatform::Lynnfield
                    | CpuPlatform::Arrandale
                    | CpuPlatform::NehalemHedt
                    | CpuPlatform::SandyBridge
                    | CpuPlatform::SandyBridgeE
            )
        })
    }
}

// ── GPU ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GpuVendor {
    Intel,
    Amd,
    Nvidia,
    Virtual,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GpuFamily {
    // Intel integrated
    IntelGma,
    IntelIronLake,
    IntelSandyBridge,
    IntelIvyBridge,
    IntelHaswell,
    IntelBroadwell,
    IntelSkylake,
    IntelKabyLake,
    IntelCoffeeLake,
    IntelCometLake,
    IntelIceLake,
    /// Gemini Lake / Apollo Lake / Atom-class UHD 600/605 etc.
    IntelLowPower,
    /// Tiger Lake and newer Xe iGPUs, Rocket Lake UHD 750, Alder/Raptor UHD 7xx
    IntelXe,
    /// Arc dGPUs
    IntelArc,
    // AMD
    AmdTeraScale,
    AmdGcn1,
    AmdGcn2,
    AmdGcn3,
    AmdPolaris,
    /// Polaris 12 "Lexa" (RX 540/550 variants) — needs device-id spoof for some IDs
    AmdLexa,
    AmdVega10,
    AmdVega20,
    AmdNavi10,
    AmdNavi12,
    AmdNavi14,
    AmdNavi21,
    AmdNavi22,
    AmdNavi23,
    AmdNavi24,
    /// RDNA 3 / RDNA 4 dGPUs
    AmdRdna3Plus,
    /// Vega-based APU iGPU (Raven, Picasso, Renoir, Lucienne, Cezanne, Barcelo) — NootedRed
    AmdApuVega,
    /// RDNA2+/3 APU iGPU (Rembrandt 680M, Phoenix 780M, Raphael, Strix)
    AmdApuRdna,
    /// Pre-Vega APUs (Kaveri, Carrizo, ...)
    AmdApuLegacy,
    // NVIDIA
    NvidiaTesla,
    NvidiaFermi,
    NvidiaKepler,
    NvidiaMaxwell,
    NvidiaPascal,
    /// Volta, Turing, Ampere, Ada, Blackwell
    NvidiaModern,
    // Other
    VirtualDisplay,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileGpu {
    pub name: String,
    pub vendor: GpuVendor,
    pub family: GpuFamily,
    /// Lowercase 4-digit hex, e.g. "1002".
    pub vendor_id: Option<String>,
    /// Lowercase 4-digit hex, e.g. "67df".
    pub device_id: Option<String>,
    pub subsystem_id: Option<String>,
    pub is_igpu: bool,
    /// OpenCore device path, e.g. "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)".
    pub pci_path: Option<String>,
    /// ACPI path, e.g. "\\_SB.PCI0.PEG0.PEGP".
    pub acpi_path: Option<String>,
    pub vram_mb: Option<u64>,
    /// User or planner decided this GPU must be disabled for macOS.
    pub disabled: bool,
}

// ── Audio / network / input / storage ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileAudio {
    /// "Realtek ALC897"
    pub codec_name: String,
    /// 0xVVVVDDDD, e.g. 0x10ec0897.
    pub codec_id: Option<u32>,
    /// HDA controller PCI ids.
    pub controller_vendor_id: Option<String>,
    pub controller_device_id: Option<String>,
    pub controller_pci_path: Option<String>,
    /// User override of the AppleALC layout-id.
    pub layout_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DeviceBus {
    Pci,
    Usb,
    Sdio,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileNic {
    pub name: String,
    pub bus: DeviceBus,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub subsystem_id: Option<String>,
    pub pci_path: Option<String>,
    /// MAC address "AA:BB:CC:DD:EE:FF" (used for PlatformInfo ROM).
    pub mac_address: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InputBus {
    Ps2,
    I2c,
    Smbus,
    Usb,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TouchpadVendor {
    Synaptics,
    Elan,
    Alps,
    Other,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileInput {
    pub keyboard_bus: InputBus,
    pub touchpad_bus: Option<InputBus>,
    pub touchpad_vendor: Option<TouchpadVendor>,
    /// ACPI _HID of the I2C touchpad (e.g. "SYNA2B33", "ELAN0662").
    pub touchpad_hid: Option<String>,
    pub has_touchscreen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StorageKind {
    Nvme,
    Sata,
    /// Intel VMD / RST RAID — must be disabled for macOS.
    Raid,
    Emmc,
    Usb,
    #[default]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileStorage {
    pub name: String,
    pub kind: StorageKind,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub size_bytes: Option<u64>,
}

// ── ACPI facts ─────────────────────────────────────────────────────────────

/// Facts extracted from the machine's DSDT/SSDTs, used to generate
/// path-correct SSDTs. Every path is an absolute ACPI path ("\\_SB.PCI0.LPCB").
#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpiFacts {
    pub pci_root: Option<String>,
    pub lpc_bridge: Option<String>,
    /// Real embedded controller (PNP0C09), if any.
    pub ec_path: Option<String>,
    pub ec_has_sta: bool,
    /// Processor objects / ACPI0007 devices, in _UID order.
    pub cpu_paths: Vec<String>,
    /// True when CPUs are declared as `Device (ACPI0007)` instead of `Processor`.
    pub cpu_uses_acpi0007: bool,
    pub awac_path: Option<String>,
    /// AWAC uses an `STAS` variable to switch RTC/AWAC.
    pub awac_has_stas: bool,
    pub rtc_path: Option<String>,
    pub hpet_path: Option<String>,
    pub igpu_path: Option<String>,
    pub xhci_paths: Vec<String>,
    /// Root hub devices under XHCI (RHUB/HUBN) — reset target for SSDT-RHUB.
    pub rhub_paths: Vec<String>,
    pub gpio_path: Option<String>,
    pub smbus_path: Option<String>,
    pub pnlf_exists: bool,
    pub has_osi_windows: bool,
    /// OEM table id of the DSDT, used to scope ACPI patches.
    pub dsdt_oem_table_id: Option<String>,
    pub dsdt_length: Option<u32>,
}

// ── Hardware profile ────────────────────────────────────────────────────────

/// Canonical, editable description of the target machine. Built by the backend
/// from a scan (or imported / entered manually) and sent back unchanged by the
/// UI for compatibility checks and builds.
#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HardwareProfile {
    pub cpu: ProfileCpu,
    pub form_factor: FormFactor,
    pub vm: Option<VmKind>,
    pub gpus: Vec<ProfileGpu>,
    pub audio: Option<ProfileAudio>,
    pub ethernet: Vec<ProfileNic>,
    pub wifi: Option<ProfileNic>,
    pub bluetooth: Option<ProfileNic>,
    pub input: ProfileInput,
    pub storage: Vec<ProfileStorage>,
    pub motherboard_vendor: String,
    pub motherboard_model: String,
    #[serde(default)]
    #[ts(optional)]
    pub system_model: Option<String>,
    /// Chipset / PCH name, e.g. "Z390", "B550", "HM370".
    pub chipset: Option<String>,
    pub ram_gb: u32,
    pub has_battery: bool,
    pub firmware_uefi: Option<bool>,
    pub acpi: Option<AcpiFacts>,
    /// Absolute path of the dumped DSDT.aml (and SSDTs alongside), if available.
    pub acpi_tables_dir: Option<String>,
    /// "scan" | "manual" | "imported" | "demo"
    pub source: String,
    /// 0.0..1.0 — how complete the scan was.
    pub scan_confidence: f64,
}

// ── Build options ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IntelWifiStrategy {
    /// Pick the best option for the target macOS.
    #[default]
    Auto,
    /// itlwm.kext + HeliPort app (works on every version, not in Recovery).
    Itlwm,
    /// AirportItlwm.kext (native Wi-Fi menu; only where a build exists).
    AirportItlwm,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PickerStyle {
    /// OpenCanopy graphical picker (needs OcBinaryData resources).
    #[default]
    Graphical,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildOptions {
    pub target: MacOsVersion,
    /// Force a specific SMBIOS model instead of the planner's choice.
    pub smbios_override: Option<String>,
    /// Extra boot-args appended to the generated ones.
    pub extra_boot_args: Option<String>,
    /// Verbose boot (-v keepsyms=1 debug=0x100). Recommended for the first install.
    pub verbose: bool,
    /// Use the DEBUG OpenCore build (more logging, slower).
    pub debug_opencore: bool,
    pub picker: PickerStyle,
    pub intel_wifi: IntelWifiStrategy,
    /// Resolve the newest upstream releases instead of the pinned, tested set.
    pub use_latest_releases: bool,
    /// Keep the serial/MLB/UUID/ROM from a previous build (iServices stability).
    pub identity: Option<PlatformIdentity>,
    /// Disable dGPUs that macOS cannot drive (recommended).
    pub disable_unsupported_gpus: bool,
    /// Picker timeout in seconds (0 = wait forever).
    pub picker_timeout: Option<u32>,
    /// macOS 26: prepare the EFI for restoring analog audio after install
    /// (AppleHDA root patch or VoodooHDA), which partly disables SIP. Off by
    /// default so SIP stays fully enabled unless the user asks for it.
    #[serde(default)]
    pub prepare_audio_patch: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            target: MacOsVersion::Sequoia,
            smbios_override: None,
            extra_boot_args: None,
            verbose: true,
            debug_opencore: false,
            picker: PickerStyle::Graphical,
            intel_wifi: IntelWifiStrategy::Auto,
            use_latest_releases: false,
            identity: None,
            disable_unsupported_gpus: true,
            picker_timeout: None,
            prepare_audio_patch: false,
        }
    }
}

/// SMBIOS identity written to PlatformInfo->Generic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlatformIdentity {
    pub model: String,
    pub serial: String,
    pub mlb: String,
    pub system_uuid: String,
    /// 6 bytes, hex "112233445566".
    pub rom: String,
}

// ── Build plan ──────────────────────────────────────────────────────────────

/// Scalar plist value used for quirk / setting overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[ts(export)]
pub enum PlistScalar {
    Bool(bool),
    Int(i64),
    Str(String),
    /// Raw bytes, hex encoded in JSON ("0300C89B").
    Data(String),
}

impl PlistScalar {
    pub fn data(bytes: &[u8]) -> Self {
        PlistScalar::Data(hex_upper(bytes))
    }
}

pub fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Ordered key → value overrides for one plist dictionary.
pub type SettingMap = BTreeMap<String, PlistScalar>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoteLevel {
    Info,
    Warning,
    /// The build cannot boot this target until resolved.
    Blocking,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanNote {
    pub level: NoteLevel,
    /// Component this note is about: "cpu", "gpu", "audio", "wifi", "usb", ...
    pub component: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SmbiosPlan {
    pub model: String,
    pub reason: String,
    /// Value for Misc->Security->SecureBootModel ("Disabled", "Default", "j185"...).
    pub secure_boot_model: String,
    /// Add the OCLP "Skip Board ID check" booter patches (unsupported model for target).
    pub board_id_skip: bool,
    /// Other models that would also work for this machine and target.
    pub alternatives: Vec<String>,
}

/// One kext bundle (or plugin) to inject.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KextSelection {
    /// Catalog id of the archive that provides this bundle ("VirtualSMC").
    pub catalog_id: String,
    /// Top-level bundle inside the archive ("SMCProcessor.kext").
    pub bundle: String,
    /// Plugins inside the bundle to inject, e.g. ("VoodooPS2Keyboard.kext", enabled).
    pub plugins: Vec<PluginSelection>,
    pub enabled: bool,
    pub min_kernel: Option<String>,
    pub max_kernel: Option<String>,
    /// Build fails if a required kext cannot be fetched.
    pub required: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginSelection {
    pub bundle: String,
    pub enabled: bool,
    pub min_kernel: Option<String>,
    pub max_kernel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum SsdtSource {
    /// AML generated from the machine's own ACPI tables; bytes hex encoded.
    Generated { aml_hex: String, dsl: String },
    /// Prebuilt binary from OpenCorePkg Docs/AcpiSamples/Binaries.
    OcSample { file: String },
    /// Prebuilt binary from Dortania's Getting-Started-With-ACPI repository.
    Dortania { file: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SsdtPlan {
    /// File name written to EFI/OC/ACPI ("SSDT-EC-USBX.aml").
    pub file_name: String,
    pub source: SsdtSource,
    pub required: bool,
    pub reason: String,
}

/// ACPI->Patch entry. Byte fields are hex strings; the fields after
/// `enabled` default to OpenCore's "unused" values (empty / 0).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpiPatch {
    pub comment: String,
    /// Hex bytes.
    pub find: String,
    pub replace: String,
    pub table_signature: Option<String>,
    pub oem_table_id: Option<String>,
    pub count: u32,
    pub enabled: bool,
    /// ACPI path of the object the patch starts at (`\_SB.VMOD`).
    #[serde(default)]
    pub base: String,
    /// Occurrences of `base` to skip.
    #[serde(default)]
    pub base_skip: u32,
    /// Hex bit mask applied to `find` (empty = all bits).
    #[serde(default)]
    pub mask: String,
    /// Hex bit mask applied to `replace` (empty = all bits).
    #[serde(default)]
    pub replace_mask: String,
    /// Bytes to search, 0 = the whole table.
    #[serde(default)]
    pub limit: u32,
    /// Occurrences of `find` to skip.
    #[serde(default)]
    pub skip: u32,
    /// Only tables of this length, 0 = any.
    #[serde(default)]
    pub table_length: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpiDelete {
    pub comment: String,
    /// 4-char table signature ("SSDT").
    pub table_signature: String,
    /// 8-char OEM table id ("CpuPm").
    pub oem_table_id: String,
    pub all: bool,
}

/// Kernel->Patch / Booter->Patch entry. Byte fields are hex strings.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BinaryPatch {
    pub comment: String,
    pub arch: String,
    pub identifier: String,
    pub base: String,
    pub find: String,
    pub mask: String,
    pub replace: String,
    pub replace_mask: String,
    pub count: u32,
    pub limit: u32,
    pub skip: u32,
    pub min_kernel: String,
    pub max_kernel: String,
    pub enabled: bool,
}

/// Booter->MmioWhitelist entry (used with DevirtualiseMmio).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MmioEntry {
    pub address: u64,
    pub comment: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KernelBlock {
    pub comment: String,
    pub identifier: String,
    /// "Disable" | "Exclude"
    pub strategy: String,
    pub min_kernel: String,
    pub max_kernel: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceProperty {
    pub key: String,
    pub value: PlistScalar,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevicePropertyEntry {
    /// OpenCore device path, e.g. "PciRoot(0x0)/Pci(0x2,0x0)".
    pub path: String,
    pub properties: Vec<DeviceProperty>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NvramVariable {
    pub guid: String,
    pub key: String,
    pub value: PlistScalar,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DriverPlan {
    /// File in EFI/OC/Drivers ("OpenRuntime.efi").
    pub path: String,
    pub load_early: bool,
    pub enabled: bool,
    pub comment: String,
    /// Where the binary comes from: "opencore" | "ocbinarydata".
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BiosSetting {
    pub name: String,
    /// "enable" | "disable" | a concrete value ("64MB", "UEFI", "AHCI").
    pub value: String,
    pub required: bool,
    pub reason: String,
    pub location_hint: Option<String>,
}

/// Everything needed to materialise an EFI for one machine and one target.
/// Pure data: produced by `domain::planner`, consumed by the build pipeline
/// and `domain::config_writer`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildPlan {
    pub target: MacOsVersion,
    pub smbios: SmbiosPlan,

    pub ssdts: Vec<SsdtPlan>,
    pub acpi_patches: Vec<AcpiPatch>,
    pub acpi_deletes: Vec<AcpiDelete>,
    pub acpi_quirks: SettingMap,

    pub booter_quirks: SettingMap,
    pub booter_patches: Vec<BinaryPatch>,
    pub mmio_whitelist: Vec<MmioEntry>,

    pub device_properties: Vec<DevicePropertyEntry>,

    pub kexts: Vec<KextSelection>,
    pub kernel_patches: Vec<BinaryPatch>,
    /// AMD CPUs: physical cores per package for the AMD_Vanilla core-count
    /// patches. The build pipeline downloads the pinned AMD_Vanilla
    /// `patches.plist` and appends its patches to `kernel_patches`.
    pub amd_core_count: Option<u32>,
    pub kernel_blocks: Vec<KernelBlock>,
    pub kernel_quirks: SettingMap,
    /// Kernel->Emulate overrides (Cpuid1Data/Cpuid1Mask as Data, DummyPowerManagement).
    pub kernel_emulate: SettingMap,

    /// Misc->Boot overrides.
    pub misc_boot: SettingMap,
    /// Misc->Debug overrides.
    pub misc_debug: SettingMap,
    /// Misc->Security overrides (SecureBootModel, Vault, ScanPolicy, ...).
    pub misc_security: SettingMap,
    /// Tools to keep in Misc->Tools / EFI/OC/Tools ("OpenShell.efi", ...).
    pub tools: Vec<String>,

    pub boot_args: Vec<String>,
    /// csr-active-config as a little-endian u32 value.
    pub csr_active_config: u32,
    pub nvram_add: Vec<NvramVariable>,
    pub nvram_delete: Vec<NvramVariable>,
    /// NVRAM->WriteFlash, LegacyOverwrite, ... overrides.
    pub nvram_settings: SettingMap,
    #[serde(default)]
    pub nvram_legacy_schema: BTreeMap<String, Vec<String>>,

    /// PlatformInfo top-level overrides (UpdateSMBIOSMode, CustomMemory, ...).
    pub platform_info: SettingMap,

    pub drivers: Vec<DriverPlan>,
    pub uefi_quirks: SettingMap,
    pub uefi_apfs: SettingMap,
    pub uefi_output: SettingMap,
    pub uefi_input: SettingMap,

    pub bios_settings: Vec<BiosSetting>,
    pub notes: Vec<PlanNote>,
    /// Steps the user must do after installing (USB map, root patches, ...).
    pub post_install: Vec<PlanNote>,
}
