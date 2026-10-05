//! IPC contracts between the Rust backend and the React frontend.
//!
//! Every type here derives `TS`; `cargo test` regenerates the TypeScript
//! bindings in `src/bridge/generated`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::domain::model::{
    BuildPlan, HardwareProfile, MacOsVersion, NoteLevel, PlanNote, PlatformIdentity,
};

// ─── Hardware detection (raw scanner output) ────────────────────────────────

/// Raw facts collected by a platform scanner. IDs are lowercase 4-digit hex
/// without prefix ("8086", "3e92"). Interpretation happens in
/// `domain::profile`, never in the scanners.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DetectedHardware {
    /// "windows" | "linux" | "macos"
    pub host_os: String,
    pub cpu: CpuInfo,
    pub gpus: Vec<GpuInfo>,
    pub audio: Vec<AudioDevice>,
    pub network: Vec<NetworkDevice>,
    pub input: Vec<InputDevice>,
    pub memory: MemoryInfo,
    pub motherboard: MotherboardInfo,
    pub storage: Vec<StorageDevice>,
    pub usb_controllers: Vec<UsbControllerInfo>,
    pub chassis: ChassisInfo,
    pub firmware: FirmwareInfo,
    /// Directory where the ACPI tables were dumped (DSDT.aml, SSDT*.aml).
    pub acpi_tables_dir: Option<String>,
    /// Hypervisor name when running inside a VM ("KVM", "VMware", "Microsoft Hyper-V").
    pub hypervisor: Option<String>,
    /// Non-fatal problems hit while scanning.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CpuInfo {
    /// Brand string ("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz").
    pub name: String,
    /// "GenuineIntel" | "AuthenticAMD" | "Apple" | other vendor string.
    pub vendor: String,
    /// CPUID display family (base + extended).
    pub family: Option<u32>,
    /// CPUID display model (base + extended << 4).
    pub model: Option<u32>,
    pub stepping: Option<u32>,
    /// Physical cores across all packages.
    pub cores: u32,
    pub threads: u32,
    /// Number of CPU packages (sockets).
    pub packages: u32,
    pub base_clock_mhz: Option<u32>,
    /// Lowercase feature flags of interest: "sse4_2", "avx", "avx2", "avx512f", "vmx", "svm".
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PciLocation {
    /// OpenCore device path ("PciRoot(0x0)/Pci(0x1f,0x3)").
    pub pci_path: Option<String>,
    /// ACPI path ("\\_SB.PCI0.HDAS").
    pub acpi_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GpuInfo {
    pub name: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub revision: Option<String>,
    pub vram_mb: Option<u64>,
    pub location: PciLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioDevice {
    pub name: String,
    /// HDA codec vendor id ("10ec").
    pub codec_vendor_id: Option<String>,
    /// HDA codec device id ("0897").
    pub codec_device_id: Option<String>,
    /// HDA codec subsystem ("10438698").
    pub codec_subsystem_id: Option<String>,
    /// HD Audio controller PCI ids.
    pub controller_vendor_id: Option<String>,
    pub controller_device_id: Option<String>,
    pub location: PciLocation,
    /// HDMI / DisplayPort audio function (not the analog codec).
    pub is_hdmi: bool,
    /// "hdaudio" | "usb" | "sst" | "other"
    pub bus: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NetworkKind {
    #[default]
    Ethernet,
    Wifi,
    Bluetooth,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NetworkDevice {
    pub name: String,
    pub kind: NetworkKind,
    /// "pci" | "usb" | "sdio" | "other"
    pub bus: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub mac_address: Option<String>,
    pub location: PciLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InputKind {
    Keyboard,
    Touchpad,
    Mouse,
    Touchscreen,
    #[default]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InputDevice {
    pub name: String,
    pub kind: InputKind,
    /// "ps2" | "i2c" | "smbus" | "usb" | "bluetooth" | "unknown"
    pub bus: String,
    /// ACPI _HID / PnP id ("SYNA2B33", "ELAN0662", "PNP0303").
    pub hardware_id: Option<String>,
    /// "synaptics" | "elan" | "alps" | ...
    pub vendor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MemoryInfo {
    pub total_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MotherboardInfo {
    /// Baseboard manufacturer / product.
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    /// System (OEM) manufacturer / product name ("Dell Inc." / "XPS 13 9370").
    pub system_manufacturer: Option<String>,
    pub system_product: Option<String>,
    /// LPC/eSPI bridge PCI device id — identifies the PCH ("a305" = Z390).
    pub lpc_vendor_id: Option<String>,
    pub lpc_device_id: Option<String>,
    /// Chipset name if the scanner could resolve it ("Z390").
    pub chipset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorageDevice {
    pub name: String,
    /// "nvme" | "sata" | "raid" | "emmc" | "usb" | "other"
    pub kind: String,
    pub controller_vendor_id: Option<String>,
    pub controller_device_id: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsbPortInfo {
    /// 1-based port number on the controller's root hub.
    pub index: u32,
    pub name: Option<String>,
    /// "usb2" | "usb3"
    pub speed_class: String,
    /// AppleUSB connector type guess (0 Type-A, 3 USB3 Type-A, 9 Type-C, 255 internal).
    pub connector: Option<u32>,
    pub user_connectable: Option<bool>,
    /// Index of the companion port (USB2<->USB3 pair), if known.
    pub companion: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsbControllerInfo {
    pub name: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    /// "xhci" | "ehci" | "ohci" | "uhci" | "other"
    pub kind: String,
    pub location: PciLocation,
    pub ports: Vec<UsbPortInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChassisInfo {
    /// SMBIOS chassis type codes (3 = desktop, 9/10 = laptop/notebook, 13 = all in one, ...).
    pub chassis_types: Vec<u32>,
    pub manufacturer: Option<String>,
    pub has_battery: bool,
    pub has_lid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FirmwareInfo {
    pub uefi: Option<bool>,
    pub secure_boot: Option<bool>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
}

/// Result of `scan_hardware`: raw facts plus the backend's interpretation.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanResult {
    pub detected: DetectedHardware,
    pub profile: HardwareProfile,
}

// ─── Catalog (for manual editing in the UI) ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogOption {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Catalog {
    pub macos_versions: Vec<CatalogOption>,
    pub cpu_platforms: Vec<CatalogOption>,
    pub gpu_families: Vec<CatalogOption>,
    pub smbios_models: Vec<CatalogOption>,
    pub form_factors: Vec<CatalogOption>,
    pub opencore_version: String,
}

// ─── Compatibility ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SupportLevel {
    /// Works natively / with standard kexts.
    Supported,
    /// Works with caveats (extra kexts, root patches, missing features).
    Partial,
    /// Will not work under macOS; must be disabled/replaced.
    Unsupported,
    /// Not enough information.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComponentAssessment {
    /// "cpu" | "gpu" | "audio" | "ethernet" | "wifi" | "bluetooth" | "input" | "storage" | "platform"
    pub component: String,
    pub name: String,
    pub level: SupportLevel,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MacOsOption {
    pub version: MacOsVersion,
    pub name: String,
    pub supported: bool,
    pub recommended: bool,
    /// Why it is unsupported, or caveats for this version.
    pub notes: Vec<String>,
    /// Needs post-install root patching (OCLP) for graphics/Wi-Fi/audio.
    pub needs_root_patch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompatibilityReport {
    /// Overall result for the selected (or recommended) target.
    pub level: SupportLevel,
    pub summary: String,
    pub target: Option<MacOsVersion>,
    pub recommended: Option<MacOsVersion>,
    pub versions: Vec<MacOsOption>,
    pub components: Vec<ComponentAssessment>,
    /// Blocking problems and warnings for the selected target.
    pub notes: Vec<PlanNote>,
    pub confidence: f64,
}

// ─── EFI build ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ArtifactStatus {
    Downloaded,
    Cached,
    Bundled,
    Generated,
    /// Optional component that could not be fetched and was left out.
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KextResult {
    /// BundlePath as written to config.plist.
    pub name: String,
    pub catalog_id: String,
    pub version: Option<String>,
    pub status: ArtifactStatus,
    pub enabled: bool,
    pub reason: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SsdtResult {
    pub file_name: String,
    /// "generated" | "oc_sample" | "dortania"
    pub source: String,
    pub status: ArtifactStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildResult {
    pub build_id: String,
    /// Directory that contains the `EFI` folder.
    pub efi_path: String,
    pub config_plist_path: String,
    pub target: MacOsVersion,
    pub opencore_version: String,
    pub identity: PlatformIdentity,
    pub plan: BuildPlan,
    pub kexts: Vec<KextResult>,
    pub ssdts: Vec<SsdtResult>,
    pub validation: ValidationResult,
    pub warnings: Vec<String>,
}

// ─── Validation ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationIssue {
    pub level: NoteLevel,
    /// "ocvalidate" | "layout" | "kext" | "acpi" | "config"
    pub source: String,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationResult {
    /// True when there is no blocking issue.
    pub valid: bool,
    /// ocvalidate ran (false when no binary for this host was available).
    pub ocvalidate_ran: bool,
    pub ocvalidate_output: Option<String>,
    pub issues: Vec<ValidationIssue>,
}

// ─── Disk / USB ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskInfo {
    /// "\\\\.\\PhysicalDrive2" | "/dev/sdb" | "/dev/disk4"
    pub device_path: String,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub serial_number: Option<String>,
    pub size_bytes: u64,
    pub size_display: String,
    /// "usb" | "sata" | "nvme" | "sd" | ...
    pub transport: Option<String>,
    pub removable: bool,
    pub partition_table: Option<String>,
    pub partitions: Vec<PartitionInfo>,
    /// Holds the running OS, the boot loader or a page file — never writable.
    pub is_system_disk: bool,
    /// Why the disk is not offered as a target (if it is not).
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PartitionInfo {
    pub number: u32,
    pub label: Option<String>,
    pub filesystem: Option<String>,
    pub size_bytes: u64,
    pub mount_point: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FlashConfirmation {
    pub token: String,
    pub device: String,
    /// Unix milliseconds.
    pub expires_at: i64,
    pub disk_display: String,
    pub efi_hash: String,
    /// Recovery image that will be written next to the EFI, if any.
    pub recovery: Option<MacOsVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrivilegeStatus {
    pub elevated: bool,
    /// The app can ask for elevation for each disk operation (pkexec / UAC / osascript).
    pub can_elevate: bool,
    pub detail: String,
}

/// Payload of the `flash:progress` event.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FlashProgress {
    pub task_id: String,
    /// "prepare" | "partition" | "format" | "copy-efi" | "copy-recovery" | "verify" | "complete" | "failed"
    pub phase: String,
    /// 0.0..1.0 overall.
    pub progress: f64,
    pub message: String,
    pub error: Option<String>,
}

// ─── Task / Progress ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskUpdate {
    pub task_id: String,
    pub kind: String,
    pub status: TaskStatus,
    pub progress: Option<f64>,
    pub message: Option<String>,
    #[ts(type = "unknown")]
    pub detail: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

// ─── BIOS / Firmware ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FirmwareReport {
    pub uefi_mode: FirmwareCheck,
    pub secure_boot: FirmwareCheck,
    pub vt_x: FirmwareCheck,
    pub vt_d: FirmwareCheck,
    pub above_4g: FirmwareCheck,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FirmwareCheck {
    pub name: String,
    /// "ok" | "action" | "unknown" | "not_applicable"
    pub status: String,
    pub evidence: String,
    pub required: bool,
}

// ─── Recovery ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveryCacheInfo {
    pub available: bool,
    pub version: Option<MacOsVersion>,
    pub dmg_path: Option<String>,
    pub chunklist_path: Option<String>,
    pub size_bytes: Option<u64>,
    /// Chunklist verification passed.
    pub verified: bool,
}

/// Payload of the `recovery:progress` event.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveryProgress {
    pub task_id: String,
    pub version: MacOsVersion,
    /// "resolving" | "downloading" | "verifying" | "complete" | "failed"
    pub phase: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// 0.0..1.0 when total is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
}

// ─── App State ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersistedState {
    pub current_step: Option<String>,
    pub profile: Option<HardwareProfile>,
    pub target: Option<MacOsVersion>,
    pub identity: Option<PlatformIdentity>,
    pub efi_path: Option<String>,
    pub timestamp: Option<i64>,
}

// ─── Updates ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppVersionInfo {
    pub version: String,
    pub opencore_version: String,
    pub host_os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub url: Option<String>,
    pub notes: Option<String>,
}

/// Small helper so commands can attach notes uniformly.
pub fn note(level: NoteLevel, component: &str, title: &str, detail: &str) -> PlanNote {
    PlanNote {
        level,
        component: component.into(),
        title: title.into(),
        detail: detail.into(),
    }
}
