//! The planner turns a hardware profile + build options into a complete,
//! declarative `BuildPlan`, following the Dortania OpenCore Install Guide (and
//! its laptop / AMD / HEDT variants) updated for OpenCore 1.0.8 and macOS 26.
//! Pure: no network or disk writes. ACPI tables referenced by the profile are
//! read through `domain::acpi`.
//!
//! Stages run in a fixed order and may read what earlier stages wrote:
//! `graphics::choose_display` → `smbios` → `graphics` → `kexts` → `acpi` →
//! `quirks` → `settings` → `notes`.

pub mod acpi;
pub mod graphics;
pub mod kexts;
pub mod notes;
pub mod quirks;
pub mod settings;
pub mod smbios;

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::domain::chipset_db::{self, ChipsetInfo};
use crate::domain::cpu_db::{self, CpuIdentity, PlatformInfo};
use crate::domain::device_db::{self, EthernetDriver};
use crate::domain::model::{
    AcpiFacts, BuildOptions, BuildPlan, CpuPlatform, CpuVendor, FormFactor, HardwareProfile,
    MacOsVersion, NoteLevel, PlanNote, ProfileCpu, ProfileGpu, ProfileNic, SettingMap, SmbiosPlan,
};
use crate::domain::{bios, compatibility, macos_db};
use crate::error::AppError;

/// Facts every stage needs, derived once from the profile and options.
pub struct PlanContext<'a> {
    pub profile: &'a HardwareProfile,
    pub options: &'a BuildOptions,
    pub target: MacOsVersion,
    pub cpu: PlatformInfo,
    /// The CPU as `cpu_db` sees it (platform, codename, segment, AVX2).
    pub identity: CpuIdentity,
    pub chipset: Option<ChipsetInfo>,
    pub is_vm: bool,
    pub is_laptop: bool,
    /// Laptop or all-in-one: internal panel needs backlight handling.
    pub has_panel: bool,
    /// Target needs AVX2 the CPU lacks → CryptexFixup.
    pub needs_cryptexfixup: bool,
    /// Parsed ACPI facts (profile.acpi, or parsed from profile.acpi_tables_dir).
    pub acpi: Option<AcpiFacts>,
    /// HEDT / workstation platform (Intel X58..X299/W790, Threadripper).
    pub is_hedt: bool,
    /// The firmware has no UEFI: OpenCore boots through OpenDuet. Same rule
    /// as the compatibility report and the BIOS checklist
    /// ([`compatibility::legacy_boot_only`]): a CSM boot on a newer board
    /// means "switch the firmware to UEFI", not OpenDuet.
    pub legacy_bios: bool,
    pub is_hp: bool,
    pub is_dell: bool,
    pub is_lenovo: bool,
    pub is_asus: bool,
    pub is_gigabyte: bool,
    pub is_msi: bool,
    pub is_asrock: bool,
    pub is_microsoft: bool,
    /// Display decision, filled by `plan` right after `graphics::choose_display`
    /// so the later stages can read it without changing their signatures.
    pub display: Option<DisplayPlan>,
}

/// Power class of a mobile CPU, which decides the closest MacBook model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileClass {
    /// 4.5-17 W: Core M, m3/m5/m7, Y-series, Amber Lake, Sandy/Ivy ULV.
    Y,
    /// 15-28 W: U-series (dual or quad core), Ice Lake G1/G4/G7.
    U,
    /// 35-45 W: H/HQ/HK/QM/XM and desktop parts in laptops.
    H,
}

impl<'a> PlanContext<'a> {
    pub fn new(profile: &'a HardwareProfile, options: &'a BuildOptions) -> Self {
        let cpu = cpu_db::platform_info(profile.cpu.platform);
        let identity = cpu_identity(&profile.cpu);
        let chipset = resolve_chipset(profile);
        let acpi = profile.acpi.clone().or_else(|| {
            profile
                .acpi_tables_dir
                .as_deref()
                .and_then(|dir| crate::domain::acpi::parse_tables(std::path::Path::new(dir)).ok())
        });
        let tokens = vendor_tokens(profile);
        let has = |names: &[&str]| tokens.iter().any(|t| names.contains(&t.as_str()));
        let joined = tokens.join(" ");
        Self {
            profile,
            options,
            target: options.target,
            cpu,
            needs_cryptexfixup: needs_cryptexfixup(profile, &identity, options.target),
            is_hedt: cpu_db::is_hedt(&identity) || chipset.as_ref().is_some_and(|c| c.is_hedt),
            identity,
            chipset,
            is_vm: profile.vm.is_some(),
            is_laptop: profile.form_factor == FormFactor::Laptop,
            has_panel: profile.form_factor.has_internal_panel(),
            acpi,
            legacy_bios: compatibility::legacy_boot_only(profile),
            is_hp: has(&["hp", "hpe", "hewlett"]),
            is_dell: has(&["dell", "alienware"]),
            is_lenovo: has(&["lenovo", "thinkpad", "ideapad", "thinkcentre", "legion"]),
            is_asus: has(&["asus", "asustek", "rog"]),
            is_gigabyte: has(&["gigabyte", "aorus"]),
            is_msi: has(&["msi"]) || joined.contains("micro star"),
            is_asrock: has(&["asrock"]),
            is_microsoft: has(&["microsoft", "surface"]),
            display: None,
        }
    }

    pub fn is_intel(&self) -> bool {
        self.identity.vendor == CpuVendor::Intel
    }

    pub fn is_amd(&self) -> bool {
        self.identity.vendor == CpuVendor::Amd
    }

    pub fn platform(&self) -> CpuPlatform {
        self.profile.cpu.platform
    }

    /// CPU has AVX2 (profile flag, else what `cpu_db` expects for the part).
    pub fn has_avx2(&self) -> bool {
        self.identity.has_avx2
    }

    /// The target expects AVX2 (macOS 13+) and the CPU lacks it: the AMD
    /// Metal drivers need it too (research-gpu §8).
    pub fn lacks_avx2_for_target(&self) -> bool {
        macos_db::requires_avx2(self.target) && !self.has_avx2()
    }

    /// Mobile CPU (laptop, NUC, mobile parts in all-in-ones).
    pub fn mobile_cpu(&self) -> bool {
        self.profile.cpu.is_mobile
            || self.is_laptop
            || matches!(
                self.platform(),
                CpuPlatform::Arrandale | CpuPlatform::IceLake
            )
    }

    /// Generation digit(s) from an Intel Core model number ("i7-9750H" → 9,
    /// "i5-10210U" → 10, "i7-1065G7" → 10, "i5-520M" → 1).
    pub fn intel_generation(&self) -> Option<u32> {
        intel_model(&self.profile.cpu.name).map(|m| m.generation)
    }

    /// Power class of the CPU, for laptop SMBIOS selection.
    pub fn mobile_class(&self) -> MobileClass {
        mobile_class(&self.profile.cpu)
    }

    /// Physical cores per package, as the AMD core-count patch wants them.
    /// Family 15h modules count as two cores (OpenCore uses the thread count).
    pub fn physical_cores(&self) -> u32 {
        let cpu = &self.profile.cpu;
        if self.platform() == CpuPlatform::AmdBulldozer {
            cpu.cores.max(cpu.threads)
        } else {
            cpu.cores
        }
    }

    /// First wired NIC that macOS can drive, else the first one.
    pub fn primary_nic(&self) -> Option<&'a ProfileNic> {
        let nics = &self.profile.ethernet;
        nics.iter()
            .find(|n| device_db::ethernet_driver(n) != EthernetDriver::Unsupported)
            .or_else(|| nics.first())
    }

    /// An Aquantia AQtion NIC that the macOS driver matches.
    pub fn has_aquantia(&self) -> bool {
        self.profile
            .ethernet
            .iter()
            .any(|n| device_db::ethernet_driver(n) == EthernetDriver::NativeAquantia)
    }

    /// GPU driving the displays per the display decision.
    pub fn display_gpu(&self, display: &DisplayPlan) -> Option<&'a ProfileGpu> {
        display.primary.and_then(|i| self.profile.gpus.get(i))
    }

    /// Dell laptops need `UpdateSMBIOSMode = Custom` with the CustomSMBIOSGuid
    /// quirk (Dortania laptop guides).
    pub fn needs_custom_smbios(&self) -> bool {
        self.is_dell && self.is_laptop && !self.is_vm
    }
}

/// CryptexFixup is needed: the target lies on the CPU's CryptexFixup path
/// (`cpu_db`), or a VM guest whose CPU model macOS does not know (and so
/// has no `cpu_db` path) lacks AVX2 on macOS 13+.
pub fn needs_cryptexfixup(
    profile: &HardwareProfile,
    identity: &CpuIdentity,
    target: MacOsVersion,
) -> bool {
    cpu_db::needs_cryptexfixup(identity, target)
        || (profile.vm.is_some()
            && !cpu_db::platform_info(identity.platform).supported
            && macos_db::requires_avx2(target)
            && !identity.has_avx2)
}

/// `cpu_db`'s view of the (possibly edited) profile CPU, shared with the
/// compatibility report. Without a CPUID AVX2 flag the brand string decides
/// when it names the same platform (Pentium/Celeron parts of AVX2 platforms
/// lack AVX2), else the platform default.
pub fn cpu_identity(cpu: &ProfileCpu) -> CpuIdentity {
    let info = cpu_db::platform_info(cpu.platform);
    let vendor = if cpu.vendor == CpuVendor::Unknown {
        info.vendor
    } else {
        cpu.vendor
    };
    let has_avx2 = cpu.has_avx2.unwrap_or_else(|| {
        let vendor_id = match vendor {
            CpuVendor::Intel => "GenuineIntel",
            CpuVendor::Amd => "AuthenticAMD",
            _ => "",
        };
        let guess = cpu_db::identify(&cpu.name, vendor_id, cpu.family, cpu.model, cpu.stepping);
        if guess.platform == cpu.platform {
            guess.has_avx2
        } else {
            info.has_avx2 && !low_end_intel(&cpu.name, cpu.platform)
        }
    });
    CpuIdentity {
        vendor,
        platform: cpu.platform,
        codename: cpu.codename.clone(),
        is_mobile: cpu.is_mobile,
        is_hybrid: cpu.is_hybrid,
        has_avx2,
    }
}

fn low_end_intel(name: &str, platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    let lower = name.to_ascii_lowercase();
    (lower.contains("pentium") || lower.contains("celeron"))
        && matches!(
            platform,
            P::Haswell | P::Broadwell | P::Skylake | P::KabyLake | P::CoffeeLake | P::CometLake
        )
}

/// Chipset from the profile (LPC-derived name), refined or completed by the
/// board name the way `chipset_db::resolve` refines an LPC match. A desktop
/// chipset guessed from a laptop's model name is ignored (ASUS laptop names
/// look like AMD desktop boards).
fn resolve_chipset(profile: &HardwareProfile) -> Option<ChipsetInfo> {
    let from_profile = profile
        .chipset
        .as_deref()
        .and_then(|name| chipset_db::from_name(name).or_else(|| chipset_db::from_board_name(name)));
    let laptop = profile.form_factor == FormFactor::Laptop;
    let from_board =
        chipset_db::from_board_name(&profile.motherboard_model).filter(|c| !laptop || c.is_mobile);
    match (from_profile, from_board) {
        (Some(id), Some(board)) if board_refines(&id, &board) => Some(board),
        (Some(id), _) => Some(id),
        (None, board) => board,
    }
}

/// A board-name match replaces the profile's chipset when the profile only
/// names the family, or when the profile's id belongs to silicon another
/// chipset reuses (X58 boards report ICH10, TRX40/WRX80 the X570 Promontory,
/// B840 the A620 one).
fn board_refines(id: &ChipsetInfo, board: &ChipsetInfo) -> bool {
    let same_silicon = matches!(
        (id.name.as_str(), board.name.as_str()),
        ("ICH10", "X58") | ("X570", "TRX40" | "WRX80") | ("A620", "B840")
    );
    same_silicon || (is_family_name(&id.name) && same_family(id, board))
}

/// Family placeholders ("AMD 500 Series", "Sunrise Point-LP") cannot appear
/// in a board name and are refined by it.
fn is_family_name(name: &str) -> bool {
    !name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn same_family(a: &ChipsetInfo, b: &ChipsetInfo) -> bool {
    let am5 = |c: &ChipsetInfo| c.vendor == CpuVendor::Amd && matches!(c.series, 600 | 800);
    a.vendor == b.vendor
        && a.is_mobile == b.is_mobile
        && (a.series == b.series || (am5(a) && am5(b)))
}

fn vendor_tokens(profile: &HardwareProfile) -> Vec<String> {
    format!(
        "{} {}",
        profile.motherboard_vendor, profile.motherboard_model
    )
    .to_ascii_lowercase()
    .split(|c: char| !c.is_ascii_alphanumeric())
    .filter(|t| !t.is_empty())
    .map(str::to_string)
    .collect()
}

/// Parsed Intel model number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntelModel {
    pub generation: u32,
    /// Digits of the SKU ("8750", "10210", "1065").
    pub number: String,
    /// Upper-case suffix ("H", "HQ", "U", "G7", "M", "Y30" for m3-7Y30).
    pub suffix: String,
}

static CORE_MODEL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:i[3579]|m[357]?|core\s+[3579])[\s-]+(\d{3,5})([a-z]{0,2}\d{0,2})\b")
        .expect("static regex")
});
static PLAIN_MODEL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(\d{4,5})([uyhm]|g\d)\b").expect("static regex"));
static CORE_M_Y: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:m[357]?)[\s-]*(\d)y(\d{2})").expect("static regex"));

/// Parse "Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz" and friends.
pub fn intel_model(name: &str) -> Option<IntelModel> {
    if let Some(c) = CORE_M_Y.captures(name) {
        let generation = c[1].parse().ok()?;
        return Some(IntelModel {
            generation,
            number: format!("{}Y{}", &c[1], &c[2]),
            suffix: format!("Y{}", &c[2]),
        });
    }
    let caps = CORE_MODEL
        .captures(name)
        .or_else(|| PLAIN_MODEL.captures(name))?;
    let number = caps[1].to_string();
    let suffix = caps[2].to_ascii_uppercase();
    let digits: Vec<u32> = number.chars().filter_map(|c| c.to_digit(10)).collect();
    let generation = match digits.len() {
        3 => 1,
        4 if digits[0] == 1 && digits[1] == 0 && suffix.starts_with('G') => 10,
        4 if digits[0] == 1 && suffix.starts_with('G') => digits[0] * 10 + digits[1],
        4 => digits[0],
        5 => digits[0] * 10 + digits[1],
        _ => return None,
    };
    Some(IntelModel {
        generation,
        number,
        suffix,
    })
}

/// Classify a mobile CPU by its model suffix, then codename, then core count.
/// Dual-core 35 W parts ("i5-2520M", "i5-3320M") count as `U`: they map to
/// the dual-core 13" MacBook Pro rows of the Dortania laptop tables.
pub fn mobile_class(cpu: &ProfileCpu) -> MobileClass {
    use CpuPlatform as P;
    if cpu.vendor == CpuVendor::Amd {
        let h = cpu
            .name
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|t| {
                t.starts_with(|c: char| c.is_ascii_digit())
                    && (t.ends_with('h') || t.ends_with("hs") || t.ends_with("hx"))
            });
        return if h { MobileClass::H } else { MobileClass::U };
    }
    let quad_is_h = matches!(
        cpu.platform,
        P::Arrandale | P::SandyBridge | P::IvyBridge | P::Haswell | P::Broadwell | P::Skylake
    );
    if let Some(m) = intel_model(&cpu.name) {
        let s = m.suffix.as_str();
        let class = match s {
            _ if s.starts_with('Y') => Some(MobileClass::Y),
            // 1st gen ULV parts are "UM"; Sandy Bridge ULV ones end in 7M
            // (i5-2467M, i7-2677M).
            "UM" => Some(MobileClass::Y),
            "M" if cpu.platform == P::SandyBridge && m.number.ends_with('7') => {
                Some(MobileClass::Y)
            }
            // Ivy Bridge U parts are the 17 W ULV chips of the MacBook Air
            // rows (Dortania ivy-bridge laptop table: "Dual Core 17w").
            _ if s.starts_with('U') && cpu.platform == P::IvyBridge => Some(MobileClass::Y),
            "M" | "LM" if cpu.cores >= 4 => Some(MobileClass::H),
            "M" | "LM" | "P" => Some(MobileClass::U),
            _ if s.starts_with('U') || s.starts_with('G') => Some(MobileClass::U),
            _ if s.starts_with('H') => Some(MobileClass::H),
            "QM" | "XM" | "MQ" | "MX" | "K" | "KF" | "F" | "T" | "S" => Some(MobileClass::H),
            "" if m.generation >= 2 => Some(MobileClass::H),
            _ => None,
        };
        if let Some(class) = class {
            return class;
        }
    }
    let codename = cpu.codename.to_ascii_lowercase();
    if codename.contains("amber") || codename.ends_with("-y") {
        return MobileClass::Y;
    }
    if codename.ends_with("-u")
        || codename.ends_with("-u/y")
        || codename.contains("ult")
        || codename.contains("whiskey")
        || codename.starts_with("ice lake")
    {
        return MobileClass::U;
    }
    if codename.ends_with("-h")
        || codename.contains("crystal well")
        || codename.contains("clarksfield")
    {
        return MobileClass::H;
    }
    // Before Kaby Lake-R every mobile quad-core was a 45 W part.
    if cpu.cores >= 6 || (cpu.cores >= 4 && quad_is_h) {
        MobileClass::H
    } else {
        MobileClass::U
    }
}

/// Component of a `PlanNote` (in `notes` or `post_install`) that marks a
/// planned OpenCore Legacy Patcher style root patch that the display GPU and
/// the kext set do not reveal (the macOS 26 AppleHDA restore). `settings`
/// then lowers SIP and disables Apple Secure Boot.
pub const ROOT_PATCH_COMPONENT: &str = "root-patch";

/// Component of a `PlanNote` that marks VoodooHDA as the planned audio driver,
/// installed to /Library/Extensions after the install (it cannot be injected
/// on macOS 26, research-kexts §4.1). `settings` lowers SIP to `03000000`.
pub const VOODOOHDA_COMPONENT: &str = "voodoohda";

/// Kexts of OCLP's Modern Wireless / legacy wireless stack (catalog ids).
const LEGACY_WIRELESS_KEXTS: &[&str] = &[
    "IOSkywalkFamily",
    "IO80211FamilyLegacy",
    "AirPortBrcmNIC-Tahoe",
];

/// Bundle id whose Kernel->Block entry belongs to the legacy wireless stack.
pub const IOSKYWALK_ID: &str = "com.apple.iokit.IOSkywalkFamily";

/// csr-active-config for VoodooHDA in /Library/Extensions:
/// CSR_ALLOW_UNTRUSTED_KEXTS | CSR_ALLOW_UNRESTRICTED_FS, NVRAM bytes
/// `03000000` (Dortania tahoe.md; research-kexts §4.1).
pub const SIP_VOODOOHDA: u32 = 0x0000_0003;
/// csr-active-config for OCLP root patches: the VoodooHDA bits plus
/// CSR_ALLOW_UNAUTHENTICATED_ROOT, NVRAM bytes `03080000`
/// (research-opencore-macos §3.4, OCLP `security.py`).
pub const SIP_ROOT_PATCH: u32 = 0x0000_0803;
/// The same plus CSR_ALLOW_UNAPPROVED_KEXTS for the NVIDIA Web Driver patch
/// set, NVRAM bytes `030A0000` (research-gpu §8).
pub const SIP_ROOT_PATCH_NVIDIA: u32 = 0x0000_0A03;

/// What the plan expects to change on the system volume after the install,
/// by source. [`root_patching_planned`] is the only place that decides it:
/// `kexts` (AMFIPass, `ipc_control_port_options=0`), `settings`
/// (csr-active-config, SecureBootModel) and `notes` read it; `smbios`, which
/// runs before the Wi-Fi and audio kexts are known, uses its graphics part
/// (the same `graphics` functions) for SecureBootModel.
///
/// Precedence for csr-active-config: an NVIDIA Web Driver card with any
/// system-volume patch (`030A0000`) over a system-volume patch from graphics,
/// Wi-Fi or the AppleHDA restore (`03080000`) over VoodooHDA alone
/// (`03000000`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RootPatchPlan {
    /// The display GPU's drivers come back only through an OCLP root patch
    /// ([`graphics::needs_root_patch_graphics`]).
    pub graphics: bool,
    /// An active card runs on the NVIDIA Web Driver
    /// ([`graphics::uses_nvidia_web_driver`]): natively on High Sierra, past
    /// it through OCLP's web-driver patch set.
    pub nvidia_web_driver: bool,
    /// OCLP Modern Wireless: the legacy wireless kexts or the
    /// IOSkywalkFamily block (Broadcom on 14+, AirportItlwm on 15+).
    pub wireless: bool,
    /// Another stage noted a root patch (component [`ROOT_PATCH_COMPONENT`]),
    /// such as the AppleHDA restore on macOS 26.
    pub noted: bool,
    /// VoodooHDA goes to /Library/Extensions (component [`VOODOOHDA_COMPONENT`]).
    pub voodoo_hda: bool,
}

impl RootPatchPlan {
    /// The sealed system volume gets patched (OCLP or a compatible tool).
    pub fn oclp(&self) -> bool {
        self.graphics || self.wireless || self.noted
    }

    /// Anything lowers SIP.
    pub fn any(&self) -> bool {
        self.oclp() || self.voodoo_hda
    }

    /// csr-active-config, by precedence: OCLP with an NVIDIA Web Driver card
    /// (`030A0000`) over OCLP (`03080000`) over VoodooHDA alone (`03000000`).
    /// Each value contains every bit of the ones after it, so the strongest
    /// source wins without dropping what a weaker one needs.
    pub fn csr_active_config(&self) -> u32 {
        if self.oclp() && self.nvidia_web_driver {
            SIP_ROOT_PATCH_NVIDIA
        } else if self.oclp() {
            SIP_ROOT_PATCH
        } else if self.voodoo_hda {
            SIP_VOODOOHDA
        } else {
            0
        }
    }

    /// Apple Secure Boot must be off: root patches break the sealed system
    /// volume (OCLP `security.py`), and the NVIDIA Web Driver never works
    /// with it (research-opencore-macos §5.4).
    pub fn secure_boot_disabled(&self) -> bool {
        self.oclp() || self.nvidia_web_driver
    }

    /// AMFIPass keeps AMFI on with patched graphics or wireless drivers
    /// (research-gpu §8, research-kexts §3.7). The AppleHDA restore and
    /// VoodooHDA do not need it.
    pub fn needs_amfipass(&self) -> bool {
        self.graphics || self.wireless
    }

    /// `ipc_control_port_options=0`: OCLP adds it whenever SIP is lowered,
    /// for the macOS 12.3+ Electron/Firefox crashes (research-gpu §2).
    pub fn needs_ipc_control_port_options(&self, target: MacOsVersion) -> bool {
        self.any() && target >= MacOsVersion::Monterey
    }
}

/// The root-patch decision for this plan. Graphics comes from the display
/// decision; the other sources come from what the `kexts` stage planned, so
/// the answer is final once `kexts` has run (`smbios`, which runs first, only
/// sees the graphics part).
pub fn root_patching_planned(
    ctx: &PlanContext,
    display: &DisplayPlan,
    plan: &BuildPlan,
) -> RootPatchPlan {
    let noted = |component: &str| {
        plan.notes
            .iter()
            .chain(&plan.post_install)
            .any(|n| n.component == component)
    };
    let wireless_kext = plan
        .kexts
        .iter()
        .any(|k| k.enabled && LEGACY_WIRELESS_KEXTS.contains(&k.catalog_id.as_str()));
    RootPatchPlan {
        graphics: graphics::needs_root_patch_graphics(ctx, display),
        nvidia_web_driver: graphics::uses_nvidia_web_driver(ctx, display),
        wireless: wireless_kext
            || plan
                .kernel_blocks
                .iter()
                .any(|b| b.enabled && b.identifier == IOSKYWALK_ID),
        noted: noted(ROOT_PATCH_COMPONENT),
        voodoo_hda: noted(VOODOOHDA_COMPONENT),
    }
}

/// Shorthand for a `PlanNote`.
pub fn note(
    level: NoteLevel,
    component: &str,
    title: impl Into<String>,
    detail: impl Into<String>,
) -> PlanNote {
    PlanNote {
        level,
        component: component.to_string(),
        title: title.into(),
        detail: detail.into(),
    }
}

/// Which GPU drives the displays, decided before anything else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayPlan {
    /// Index into `profile.gpus` of the GPU that drives the displays
    /// (None only for VMs without a passed-through GPU).
    pub primary: Option<usize>,
    /// Index of a supported iGPU that stays enabled (display or headless).
    pub igpu: Option<usize>,
    /// The iGPU is enabled for compute/QuickSync only (a dGPU drives displays).
    pub igpu_headless: bool,
    /// GPUs macOS cannot drive on the target; they get disabled.
    pub disabled: Vec<usize>,
}

/// Produce the full plan. Errors only when the target cannot work at all;
/// soft problems become `PlanNote`s.
pub fn plan(profile: &HardwareProfile, options: &BuildOptions) -> Result<BuildPlan, AppError> {
    validate(profile, options)?;
    let mut ctx = PlanContext::new(profile, options);
    let display = graphics::choose_display(&ctx)?;
    ctx.display = Some(display.clone());

    let mut plan = empty_plan(options.target);
    smbios::apply(&ctx, &display, &mut plan)?;
    graphics::apply(&ctx, &display, &mut plan);
    kexts::apply(&ctx, &display, &mut plan);
    acpi::apply(&ctx, &display, &mut plan);
    quirks::apply(&ctx, &mut plan);
    settings::apply(&ctx, &mut plan);
    notes::apply(&ctx, &display, &mut plan);
    plan.bios_settings = bios::recommended_settings(profile, options.target);
    finalize(&mut plan);
    Ok(plan)
}

/// Reject targets the CPU cannot run, using the same limits as the
/// compatibility report: the native range of `cpu_db`, extended by the
/// platform's ceiling workaround (CryptexFixup, telemetrap, untested
/// Excavator) which the later stages implement.
pub fn validate(profile: &HardwareProfile, options: &BuildOptions) -> Result<(), AppError> {
    let platform = profile.cpu.platform;
    let target = options.target;
    if platform == CpuPlatform::AppleSilicon || profile.cpu.vendor == CpuVendor::Apple {
        return Err(AppError::new(
            "APPLE_SILICON",
            "This Mac already runs macOS natively; OpenCore EFIs are for Intel/AMD PCs.",
        ));
    }
    let is_vm = profile.vm.is_some();
    let info = cpu_db::platform_info(platform);
    // A guest sees whatever CPU model the hypervisor exposes; only a known,
    // supported model is checked against its limits (the compatibility
    // report applies the same rule).
    if is_vm && !info.supported && cpu_db::usable_as_vm_guest(platform) {
        return Ok(());
    }
    if platform == CpuPlatform::Unknown {
        return Err(AppError::new(
            "CPU_UNKNOWN",
            "The CPU platform is unknown. Pick it manually in the hardware editor.",
        )
        .recoverable()
        .with_suggestion("Open the hardware editor and choose the CPU generation."));
    }
    if !info.supported {
        return Err(AppError::new(
            "CPU_UNSUPPORTED",
            format!("{} CPUs cannot run macOS through OpenCore.", info.label),
        ));
    }
    let identity = cpu_identity(&profile.cpu);
    if let Some(max) = cpu_db::max_macos_for(&identity) {
        if target > max {
            let workaround = cpu_db::ceiling_workaround_for(&identity)
                .filter(|w| target >= w.from && !matches!(w.max_macos, Some(m) if target > m));
            if workaround.is_none() {
                let reach = cpu_db::ceiling_workaround_for(&identity)
                    .and_then(|w| w.max_macos)
                    .map_or(max, |m| m.max(max));
                return Err(AppError::new(
                    "TARGET_ABOVE_CPU_LIMIT",
                    format!("{} supports up to {}.", info.label, reach.display_name()),
                )
                .recoverable()
                .with_suggestion("Choose an older macOS version."));
            }
        }
    }
    if let Some(min) = cpu_db::min_macos_for(&identity) {
        if target < min {
            return Err(AppError::new(
                "TARGET_BELOW_CPU_MINIMUM",
                format!("{} needs {} or newer.", info.label, min.display_name()),
            )
            .recoverable()
            .with_suggestion("Choose a newer macOS version."));
        }
    }
    if identity.vendor == CpuVendor::Amd {
        let cores = if platform == CpuPlatform::AmdBulldozer {
            profile.cpu.cores.max(profile.cpu.threads)
        } else {
            profile.cpu.cores
        };
        if cores == 0 {
            return Err(AppError::new(
                "AMD_CORE_COUNT_UNKNOWN",
                "The AMD kernel patches need the number of physical CPU cores, which is unknown.",
            )
            .recoverable()
            .with_suggestion("Enter the physical core count in the hardware editor."));
        }
        if cores > 64 {
            return Err(AppError::new(
                "AMD_TOO_MANY_CORES",
                format!("macOS supports at most 64 CPU cores; this CPU has {cores}."),
            ));
        }
    }
    Ok(())
}

pub fn empty_plan(target: MacOsVersion) -> BuildPlan {
    BuildPlan {
        target,
        smbios: SmbiosPlan {
            model: String::new(),
            reason: String::new(),
            secure_boot_model: "Disabled".into(),
            board_id_skip: false,
            alternatives: vec![],
        },
        ssdts: vec![],
        acpi_patches: vec![],
        acpi_deletes: vec![],
        acpi_quirks: SettingMap::new(),
        booter_quirks: SettingMap::new(),
        booter_patches: vec![],
        mmio_whitelist: vec![],
        device_properties: vec![],
        kexts: vec![],
        kernel_patches: vec![],
        amd_core_count: None,
        kernel_blocks: vec![],
        kernel_quirks: SettingMap::new(),
        kernel_emulate: SettingMap::new(),
        misc_boot: SettingMap::new(),
        misc_debug: SettingMap::new(),
        misc_security: SettingMap::new(),
        tools: vec![],
        boot_args: vec![],
        csr_active_config: 0,
        nvram_add: vec![],
        nvram_delete: vec![],
        nvram_settings: SettingMap::new(),
        platform_info: SettingMap::new(),
        drivers: vec![],
        uefi_quirks: SettingMap::new(),
        uefi_apfs: SettingMap::new(),
        uefi_output: SettingMap::new(),
        uefi_input: SettingMap::new(),
        bios_settings: vec![],
        notes: vec![],
        post_install: vec![],
    }
}

/// Remove duplicates while keeping first occurrence order; DeviceProperties
/// entries for the same path are merged (first value of a key wins).
fn finalize(plan: &mut BuildPlan) {
    let mut seen = HashSet::new();
    plan.boot_args.retain(|arg| {
        let key = arg.split('=').next().unwrap_or(arg).to_string();
        seen.insert(key)
    });
    let mut seen = HashSet::new();
    plan.kexts
        .retain(|k| seen.insert((k.catalog_id.clone(), k.bundle.clone())));
    let mut seen = HashSet::new();
    plan.ssdts.retain(|s| seen.insert(s.file_name.clone()));
    let mut seen = HashSet::new();
    plan.drivers
        .retain(|d| seen.insert(d.path.to_ascii_lowercase()));
    let mut seen = HashSet::new();
    plan.tools.retain(|t| seen.insert(t.to_ascii_lowercase()));
    let mut seen = HashSet::new();
    plan.booter_patches.retain(|p| {
        seen.insert((
            p.identifier.clone(),
            p.find.to_ascii_uppercase(),
            p.replace.to_ascii_uppercase(),
        ))
    });
    let mut seen = HashSet::new();
    plan.mmio_whitelist.retain(|m| seen.insert(m.address));
    let mut seen = HashSet::new();
    plan.kernel_blocks.retain(|b| {
        seen.insert((
            b.identifier.clone(),
            b.strategy.clone(),
            b.min_kernel.clone(),
        ))
    });
    let mut seen = HashSet::new();
    plan.nvram_add
        .retain(|v| seen.insert((v.guid.to_ascii_uppercase(), v.key.clone())));
    let mut seen = HashSet::new();
    plan.nvram_delete
        .retain(|v| seen.insert((v.guid.to_ascii_uppercase(), v.key.clone())));
    // Only exact repeats go: two stages may share a title with different detail.
    for list in [&mut plan.notes, &mut plan.post_install] {
        let mut seen = HashSet::new();
        list.retain(|n| seen.insert((n.component.clone(), n.title.clone(), n.detail.clone())));
    }

    let mut merged: Vec<crate::domain::model::DevicePropertyEntry> = Vec::new();
    for entry in std::mem::take(&mut plan.device_properties) {
        match merged
            .iter_mut()
            .find(|e| e.path.eq_ignore_ascii_case(&entry.path))
        {
            Some(existing) => {
                for prop in entry.properties {
                    if !existing.properties.iter().any(|p| p.key == prop.key) {
                        existing.properties.push(prop);
                    }
                }
                if !entry.reason.is_empty() && !existing.reason.contains(&entry.reason) {
                    existing.reason = format!("{}; {}", existing.reason, entry.reason);
                }
            }
            None => merged.push(entry),
        }
    }
    plan.device_properties = merged;
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Profile builders shared by the planner stage tests.

    use super::*;
    use crate::domain::model::{
        DeviceBus, GpuFamily, GpuVendor, PlistScalar, ProfileNic, ProfileStorage, StorageKind,
        VmKind,
    };

    pub const SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/Sample-1.0.8.plist");

    pub fn cpu(platform: CpuPlatform, name: &str, codename: &str, cores: u32) -> ProfileCpu {
        let info = cpu_db::platform_info(platform);
        ProfileCpu {
            name: name.to_string(),
            vendor: info.vendor,
            platform,
            codename: codename.to_string(),
            family: None,
            model: None,
            stepping: None,
            cores,
            threads: cores * 2,
            is_mobile: false,
            has_avx2: None,
            has_sse4_2: Some(true),
            is_hybrid: matches!(
                platform,
                CpuPlatform::AlderLake | CpuPlatform::RaptorLake | CpuPlatform::ArrowLake
            ),
        }
    }

    pub fn gpu(family: GpuFamily, is_igpu: bool) -> ProfileGpu {
        let vendor = match family {
            GpuFamily::NvidiaKepler
            | GpuFamily::NvidiaMaxwell
            | GpuFamily::NvidiaPascal
            | GpuFamily::NvidiaFermi
            | GpuFamily::NvidiaTesla
            | GpuFamily::NvidiaModern => GpuVendor::Nvidia,
            GpuFamily::VirtualDisplay => GpuVendor::Virtual,
            f if format!("{f:?}").starts_with("Intel") => GpuVendor::Intel,
            _ => GpuVendor::Amd,
        };
        ProfileGpu {
            name: format!("{family:?}"),
            vendor,
            family,
            vendor_id: None,
            device_id: None,
            subsystem_id: None,
            is_igpu,
            pci_path: Some(if is_igpu {
                "PciRoot(0x0)/Pci(0x2,0x0)".to_string()
            } else {
                "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)".to_string()
            }),
            acpi_path: None,
            vram_mb: None,
            disabled: false,
        }
    }

    pub fn profile(
        cpu: ProfileCpu,
        form_factor: FormFactor,
        gpus: Vec<ProfileGpu>,
    ) -> HardwareProfile {
        let is_mobile = form_factor == FormFactor::Laptop;
        HardwareProfile {
            cpu: ProfileCpu {
                is_mobile: cpu.is_mobile || is_mobile,
                ..cpu
            },
            form_factor,
            vm: None,
            gpus,
            audio: None,
            ethernet: vec![],
            wifi: None,
            bluetooth: None,
            input: Default::default(),
            storage: vec![ProfileStorage {
                name: "Generic NVMe SSD".into(),
                kind: StorageKind::Nvme,
                vendor_id: None,
                device_id: None,
                size_bytes: None,
            }],
            motherboard_vendor: String::new(),
            motherboard_model: String::new(),
            chipset: None,
            ram_gb: 16,
            has_battery: is_mobile,
            firmware_uefi: Some(true),
            acpi: None,
            acpi_tables_dir: None,
            source: "manual".into(),
            scan_confidence: 1.0,
        }
    }

    pub fn vm_profile(platform: CpuPlatform, kind: VmKind) -> HardwareProfile {
        let mut p = profile(
            cpu(platform, "Common KVM processor", "Unknown", 4),
            FormFactor::Desktop,
            vec![gpu(GpuFamily::VirtualDisplay, false)],
        );
        p.vm = Some(kind);
        p
    }

    pub fn nic(vendor: &str, device: &str) -> ProfileNic {
        ProfileNic {
            name: String::new(),
            bus: DeviceBus::Pci,
            vendor_id: Some(vendor.into()),
            device_id: Some(device.into()),
            subsystem_id: None,
            pci_path: None,
            mac_address: None,
        }
    }

    pub fn options(target: MacOsVersion) -> BuildOptions {
        BuildOptions {
            target,
            ..BuildOptions::default()
        }
    }

    /// Display plan from the GPU indices (`igpu` with `headless` = compute only).
    pub fn display(primary: Option<usize>, igpu: Option<usize>, headless: bool) -> DisplayPlan {
        DisplayPlan {
            primary,
            igpu,
            igpu_headless: headless,
            disabled: vec![],
        }
    }

    pub fn on(map: &SettingMap, key: &str) -> bool {
        matches!(map.get(key), Some(PlistScalar::Bool(true)))
    }

    pub fn int_of(map: &SettingMap, key: &str) -> Option<i64> {
        match map.get(key) {
            Some(PlistScalar::Int(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn str_of(map: &SettingMap, key: &str) -> Option<String> {
        match map.get(key) {
            Some(PlistScalar::Str(v)) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn data_of(map: &SettingMap, key: &str) -> Option<String> {
        match map.get(key) {
            Some(PlistScalar::Data(v)) => Some(v.clone()),
            _ => None,
        }
    }

    /// Every override key must exist in Sample.plist under `section` with the
    /// same type (config_writer rejects anything else).
    static SAMPLE_ROOT: Lazy<plist::Dictionary> =
        Lazy::new(
            || match plist::Value::from_reader(std::io::Cursor::new(SAMPLE)) {
                Ok(plist::Value::Dictionary(d)) => d,
                _ => panic!("Sample.plist fixture does not parse"),
            },
        );

    pub fn assert_schema(section: &str, map: &SettingMap) {
        let mut dict: &plist::Dictionary = &SAMPLE_ROOT;
        for part in section.split('/') {
            dict = dict
                .get(part)
                .and_then(plist::Value::as_dictionary)
                .unwrap_or_else(|| panic!("{section} is not a dictionary in Sample.plist"));
        }
        for (key, value) in map {
            let mut node = dict;
            let parts: Vec<&str> = key.split('/').collect();
            for part in &parts[..parts.len() - 1] {
                node = node
                    .get(part)
                    .and_then(plist::Value::as_dictionary)
                    .unwrap_or_else(|| panic!("{section}/{key}: {part} missing"));
            }
            let leaf = node
                .get(parts[parts.len() - 1])
                .unwrap_or_else(|| panic!("{section}/{key} is not in Sample.plist"));
            let ok = matches!(
                (value, leaf),
                (PlistScalar::Bool(_), plist::Value::Boolean(_))
                    | (PlistScalar::Int(_), plist::Value::Integer(_))
                    | (PlistScalar::Str(_), plist::Value::String(_))
                    | (PlistScalar::Data(_), plist::Value::Data(_))
            );
            assert!(ok, "{section}/{key}: type mismatch ({value:?} vs {leaf:?})");
        }
    }
}

#[cfg(test)]
mod consistency_tests;

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::domain::model::{GpuFamily, VmKind};
    use MacOsVersion::*;

    #[test]
    fn intel_model_numbers() {
        let cases = [
            ("Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz", 9, "H"),
            ("Intel(R) Core(TM) i5-10210U CPU @ 1.60GHz", 10, "U"),
            ("Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz", 10, "G7"),
            ("Intel(R) Core(TM) i7-1165G7 @ 2.80GHz", 11, "G7"),
            ("Intel(R) Core(TM) i5-520M CPU @ 2.40GHz", 1, "M"),
            ("Intel(R) Core(TM) i7-2677M CPU @ 1.80GHz", 2, "M"),
            ("Intel(R) Core(TM) i7-4980HQ CPU @ 2.80GHz", 4, "HQ"),
            ("Intel(R) Core(TM) i9-10900K CPU @ 3.70GHz", 10, "K"),
            ("Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz", 8, ""),
        ];
        for (name, generation, suffix) in cases {
            let m = intel_model(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(
                (m.generation, m.suffix.as_str()),
                (generation, suffix),
                "{name}"
            );
        }
        let celeron = intel_model("Intel(R) Celeron(R) CPU 3867U @ 1.80GHz").unwrap();
        assert_eq!(celeron.suffix, "U");
        let m = intel_model("Intel(R) Core(TM) m3-7Y30 CPU @ 1.00GHz").unwrap();
        assert_eq!((m.generation, m.suffix.as_str()), (7, "Y30"));
        assert!(intel_model("AMD Ryzen 7 5800X 8-Core Processor").is_none());
    }

    #[test]
    fn mobile_classes() {
        let class = |platform, name: &str, codename: &str, cores| {
            let mut c = cpu(platform, name, codename, cores);
            c.is_mobile = true;
            mobile_class(&c)
        };
        use CpuPlatform as P;
        assert_eq!(
            class(
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-8750H",
                "Coffee Lake-H",
                6
            ),
            MobileClass::H
        );
        assert_eq!(
            class(
                P::CometLake,
                "Intel(R) Core(TM) i5-10210U",
                "Comet Lake-U",
                4
            ),
            MobileClass::U
        );
        assert_eq!(
            class(P::KabyLake, "Intel(R) Core(TM) m3-7Y30", "Kaby Lake-Y", 2),
            MobileClass::Y
        );
        assert_eq!(
            class(P::KabyLake, "Intel(R) Core(TM) i5-8200Y", "Amber Lake-Y", 2),
            MobileClass::Y
        );
        assert_eq!(
            class(
                P::SandyBridge,
                "Intel(R) Core(TM) i7-2677M",
                "Sandy Bridge",
                2
            ),
            MobileClass::Y
        );
        assert_eq!(
            class(
                P::SandyBridge,
                "Intel(R) Core(TM) i5-2520M",
                "Sandy Bridge",
                2
            ),
            MobileClass::U
        );
        assert_eq!(
            class(P::IvyBridge, "Intel(R) Core(TM) i7-3720QM", "Ivy Bridge", 4),
            MobileClass::H
        );
        assert_eq!(
            class(P::IvyBridge, "Intel(R) Core(TM) i5-3317U", "Ivy Bridge", 2),
            MobileClass::Y,
            "Ivy Bridge U parts are 17 W ULV"
        );
        assert_eq!(
            class(P::IvyBridge, "Intel(R) Core(TM) i5-3210M", "Ivy Bridge", 2),
            MobileClass::U
        );
        assert_eq!(
            class(P::Haswell, "Intel(R) Core(TM) i5-4200U", "Haswell-ULT", 2),
            MobileClass::U
        );
        assert_eq!(
            class(
                P::Arrandale,
                "Intel(R) Core(TM) i7 CPU Q 720",
                "Clarksfield",
                4
            ),
            MobileClass::H
        );
        assert_eq!(
            class(
                P::Arrandale,
                "Intel(R) Core(TM) i5 CPU M 520",
                "Arrandale",
                2
            ),
            MobileClass::U
        );
        assert_eq!(
            class(P::IceLake, "Intel(R) Core(TM) i5-1035G1", "Ice Lake-U", 4),
            MobileClass::U
        );
        assert_eq!(
            class(P::Broadwell, "Intel(R) Core(TM) M-5Y10c", "Broadwell-Y", 2),
            MobileClass::Y
        );
        assert_eq!(
            class(
                P::AmdZen2,
                "AMD Ryzen 7 4800H with Radeon Graphics",
                "Renoir",
                8
            ),
            MobileClass::H
        );
        assert_eq!(
            class(
                P::AmdZen3,
                "AMD Ryzen 5 5600U with Radeon Graphics",
                "Cezanne",
                6
            ),
            MobileClass::U
        );
    }

    #[test]
    fn context_vendor_flags_and_chipset() {
        let mut p = profile(
            cpu(
                CpuPlatform::CoffeeLake,
                "Intel(R) Core(TM) i7-8700",
                "Coffee Lake-S",
                6,
            ),
            FormFactor::Desktop,
            vec![],
        );
        p.motherboard_vendor = "Hewlett-Packard".into();
        p.motherboard_model = "8434".into();
        p.chipset = Some("Z390".into());
        let o = options(Sequoia);
        let ctx = PlanContext::new(&p, &o);
        assert!(ctx.is_hp && !ctx.is_dell);
        assert_eq!(ctx.chipset.as_ref().map(|c| c.name.as_str()), Some("Z390"));

        p.chipset = Some("AMD 500 Series".into());
        p.motherboard_vendor = "ASUSTeK COMPUTER INC.".into();
        p.motherboard_model = "ROG STRIX B550-F GAMING".into();
        let ctx = PlanContext::new(&p, &o);
        assert!(ctx.is_asus);
        assert_eq!(ctx.chipset.as_ref().map(|c| c.name.as_str()), Some("B550"));

        // Chipsets that reuse another chipset's silicon take the board's name.
        for (lpc, board, expect) in [
            ("X570", "TRX40 AORUS XTREME", "TRX40"),
            ("X570", "Pro WS WRX80E-SAGE SE WIFI", "WRX80"),
            ("ICH10", "P6X58D-E", "X58"),
            ("X570", "ROG CROSSHAIR VIII HERO", "X570"),
            ("Z390", "PRIME B360M-A", "Z390"),
        ] {
            p.chipset = Some(lpc.into());
            p.motherboard_model = board.into();
            let ctx = PlanContext::new(&p, &o);
            assert_eq!(
                ctx.chipset.as_ref().map(|c| c.name.as_str()),
                Some(expect),
                "{lpc} + {board}"
            );
        }
        let ctx = PlanContext::new(&p, &o);
        assert!(!ctx.is_hedt);
        p.chipset = Some("X570".into());
        p.motherboard_model = "TRX40 AORUS XTREME".into();
        assert!(
            PlanContext::new(&p, &o).is_hedt,
            "TRX40 is a workstation chipset"
        );
        p.motherboard_model = "ROG STRIX B550-F GAMING".into();

        p.chipset = None;
        p.form_factor = FormFactor::Laptop;
        p.motherboard_model = "ROG Zephyrus G14 GA401QM X570ZD".into();
        let ctx = PlanContext::new(&p, &o);
        assert!(
            ctx.chipset.is_none(),
            "desktop chipset guessed from a laptop name"
        );

        p.motherboard_vendor = "Dell Inc.".into();
        let ctx = PlanContext::new(&p, &o);
        assert!(ctx.needs_custom_smbios());
    }

    #[test]
    fn cryptexfixup_follows_cpu_db() {
        let p = profile(
            cpu(
                CpuPlatform::IvyBridge,
                "Intel(R) Core(TM) i7-3770",
                "Ivy Bridge",
                4,
            ),
            FormFactor::Desktop,
            vec![],
        );
        assert!(PlanContext::new(&p, &options(Ventura)).needs_cryptexfixup);
        assert!(!PlanContext::new(&p, &options(Monterey)).needs_cryptexfixup);
        let p = profile(
            cpu(
                CpuPlatform::CoffeeLake,
                "Intel(R) Pentium(R) Gold G5400",
                "Coffee Lake-S",
                2,
            ),
            FormFactor::Desktop,
            vec![],
        );
        let o = options(Sonoma);
        let ctx = PlanContext::new(&p, &o);
        assert!(!ctx.has_avx2() && ctx.needs_cryptexfixup);
    }

    #[test]
    fn validate_uses_cpu_limits_and_workarounds() {
        let ivy = profile(
            cpu(
                CpuPlatform::IvyBridge,
                "Intel(R) Core(TM) i7-3770",
                "Ivy Bridge",
                4,
            ),
            FormFactor::Desktop,
            vec![],
        );
        // CryptexFixup path: allowed through Tahoe.
        assert!(validate(&ivy, &options(Tahoe)).is_ok());
        let penryn = profile(
            cpu(
                CpuPlatform::Penryn,
                "Intel(R) Core(TM)2 Quad Q9550",
                "Penryn",
                4,
            ),
            FormFactor::Desktop,
            vec![],
        );
        assert!(validate(&penryn, &options(Monterey)).is_ok());
        let err = validate(&penryn, &options(Ventura)).unwrap_err();
        assert_eq!(err.code, "TARGET_ABOVE_CPU_LIMIT");
        assert!(err.message.contains("Monterey"));
        let comet = profile(
            cpu(
                CpuPlatform::CometLake,
                "Intel(R) Core(TM) i7-10700K",
                "Comet Lake-S",
                8,
            ),
            FormFactor::Desktop,
            vec![],
        );
        assert_eq!(
            validate(&comet, &options(Mojave)).unwrap_err().code,
            "TARGET_BELOW_CPU_MINIMUM"
        );
        let mut whiskey = profile(
            cpu(
                CpuPlatform::CoffeeLake,
                "Intel(R) Core(TM) i7-8565U",
                "Whiskey Lake-U",
                4,
            ),
            FormFactor::Laptop,
            vec![],
        );
        assert_eq!(
            validate(&whiskey, &options(HighSierra)).unwrap_err().code,
            "TARGET_BELOW_CPU_MINIMUM"
        );
        whiskey.cpu.codename = "Coffee Lake-U".into();
        assert!(validate(&whiskey, &options(HighSierra)).is_ok());
        let zen4 = profile(
            cpu(
                CpuPlatform::AmdZen4,
                "AMD Ryzen 5 7600X 6-Core Processor",
                "Raphael",
                6,
            ),
            FormFactor::Desktop,
            vec![],
        );
        assert_eq!(
            validate(&zen4, &options(BigSur)).unwrap_err().code,
            "TARGET_BELOW_CPU_MINIMUM"
        );
        assert!(validate(&zen4, &options(Tahoe)).is_ok());
        let mut zen = zen4.clone();
        zen.cpu.cores = 0;
        assert_eq!(
            validate(&zen, &options(Tahoe)).unwrap_err().code,
            "AMD_CORE_COUNT_UNKNOWN"
        );
        zen.cpu.cores = 96;
        assert_eq!(
            validate(&zen, &options(Tahoe)).unwrap_err().code,
            "AMD_TOO_MANY_CORES"
        );
        let fx = profile(
            cpu(
                CpuPlatform::AmdBulldozer,
                "AMD FX(tm)-8350 Eight-Core Processor",
                "Vishera",
                4,
            ),
            FormFactor::Desktop,
            vec![],
        );
        assert!(
            validate(&fx, &options(Sonoma)).is_ok(),
            "CryptexFixup path on FX"
        );
        let pentium = profile(
            cpu(
                CpuPlatform::Haswell,
                "Intel(R) Pentium(R) CPU G3258",
                "Haswell",
                2,
            ),
            FormFactor::Desktop,
            vec![],
        );
        assert!(
            validate(&pentium, &options(Tahoe)).is_ok(),
            "low-end CryptexFixup path"
        );
        let alder_laptop = profile(
            cpu(
                CpuPlatform::MeteorLake,
                "Intel(R) Core(TM) Ultra 7 155H",
                "Meteor Lake-H",
                16,
            ),
            FormFactor::Laptop,
            vec![],
        );
        assert_eq!(
            validate(&alder_laptop, &options(Sonoma)).unwrap_err().code,
            "CPU_UNSUPPORTED"
        );
        let vm = vm_profile(CpuPlatform::Unknown, VmKind::Kvm);
        assert!(validate(&vm, &options(Tahoe)).is_ok());
    }

    #[test]
    fn root_patch_prediction() {
        let p = profile(
            cpu(
                CpuPlatform::Haswell,
                "Intel(R) Core(TM) i7-4790K",
                "Haswell",
                4,
            ),
            FormFactor::Desktop,
            vec![
                gpu(GpuFamily::NvidiaKepler, false),
                gpu(GpuFamily::IntelHaswell, true),
            ],
        );
        let d = display(Some(0), Some(1), true);
        let o = options(Monterey);
        let ctx = PlanContext::new(&p, &o);
        let rp = root_patching_planned(&ctx, &d, &empty_plan(Monterey));
        assert!(rp.graphics && rp.oclp(), "Kepler on Monterey");
        assert!(rp.needs_amfipass() && rp.secure_boot_disabled());
        assert_eq!(rp.csr_active_config(), SIP_ROOT_PATCH);
        assert!(rp.needs_ipc_control_port_options(Monterey));

        let o = options(BigSur);
        let ctx = PlanContext::new(&p, &o);
        let rp = root_patching_planned(&ctx, &d, &empty_plan(BigSur));
        assert_eq!(rp, RootPatchPlan::default());
        assert_eq!(rp.csr_active_config(), 0);

        let o = options(Sonoma);
        let ctx = PlanContext::new(&p, &o);
        let igpu = display(Some(1), Some(1), false);
        assert!(
            root_patching_planned(&ctx, &igpu, &empty_plan(Sonoma)).graphics,
            "Haswell iGPU on Sonoma"
        );

        // Signals from the kexts stage.
        let p = profile(
            cpu(
                CpuPlatform::CometLake,
                "Intel(R) Core(TM) i7-10700K",
                "Comet Lake-S",
                8,
            ),
            FormFactor::Desktop,
            vec![gpu(GpuFamily::IntelCometLake, true)],
        );
        let o = options(Tahoe);
        let ctx = PlanContext::new(&p, &o);
        let d = display(Some(0), Some(0), false);
        let mut plan = empty_plan(Tahoe);
        assert!(!root_patching_planned(&ctx, &d, &plan).any());

        plan.post_install.push(note(
            NoteLevel::Info,
            VOODOOHDA_COMPONENT,
            "VoodooHDA",
            "",
        ));
        let rp = root_patching_planned(&ctx, &d, &plan);
        assert!(rp.voodoo_hda && !rp.oclp() && !rp.secure_boot_disabled());
        assert!(!rp.needs_amfipass());
        assert_eq!(rp.csr_active_config().to_le_bytes(), [0x03, 0x00, 0x00, 0x00]);

        plan.post_install.push(note(
            NoteLevel::Info,
            ROOT_PATCH_COMPONENT,
            "AppleHDA",
            "",
        ));
        let rp = root_patching_planned(&ctx, &d, &plan);
        assert!(rp.noted && rp.oclp() && !rp.needs_amfipass());
        assert_eq!(rp.csr_active_config(), SIP_ROOT_PATCH, "OCLP wins over VoodooHDA");

        plan.kernel_blocks.push(crate::domain::model::KernelBlock {
            comment: String::new(),
            identifier: IOSKYWALK_ID.into(),
            strategy: "Exclude".into(),
            min_kernel: "23.0.0".into(),
            max_kernel: String::new(),
            enabled: true,
        });
        let rp = root_patching_planned(&ctx, &d, &plan);
        assert!(rp.wireless && rp.needs_amfipass());

        // The web-driver patch set needs CSR_ALLOW_UNAPPROVED_KEXTS on top.
        let web = RootPatchPlan {
            graphics: true,
            nvidia_web_driver: true,
            ..RootPatchPlan::default()
        };
        assert_eq!(web.csr_active_config(), SIP_ROOT_PATCH_NVIDIA);
        // High Sierra's web driver alone: Secure Boot off, SIP untouched.
        let hs = RootPatchPlan {
            nvidia_web_driver: true,
            ..RootPatchPlan::default()
        };
        assert!(hs.secure_boot_disabled() && hs.csr_active_config() == 0);
        // Every stronger value keeps the bits of the weaker ones.
        assert_eq!(SIP_ROOT_PATCH_NVIDIA & SIP_ROOT_PATCH, SIP_ROOT_PATCH);
        assert_eq!(SIP_ROOT_PATCH & SIP_VOODOOHDA, SIP_VOODOOHDA);
    }

    #[test]
    fn legacy_bios_follows_the_compatibility_rule() {
        let mut p = profile(
            cpu(
                CpuPlatform::CoffeeLake,
                "Intel(R) Core(TM) i7-8700",
                "Coffee Lake-S",
                6,
            ),
            FormFactor::Desktop,
            vec![],
        );
        p.firmware_uefi = Some(false);
        let o = options(Sequoia);
        assert!(
            !PlanContext::new(&p, &o).legacy_bios,
            "a CSM boot on a UEFI-era board is not OpenDuet"
        );
        p.cpu = cpu(CpuPlatform::Penryn, "Intel(R) Core(TM)2 Quad Q9550", "Penryn", 4);
        let o = options(HighSierra);
        assert!(PlanContext::new(&p, &o).legacy_bios);
        p.vm = Some(VmKind::Kvm);
        assert!(!PlanContext::new(&p, &o).legacy_bios);
    }

    #[test]
    fn finalize_merges_and_dedupes() {
        use crate::domain::model::{DeviceProperty, DevicePropertyEntry, MmioEntry, PlistScalar};
        let mut plan = empty_plan(Sequoia);
        plan.boot_args = vec![
            "-v".into(),
            "agdpmod=pikera".into(),
            "agdpmod=ignore".into(),
            "-v".into(),
        ];
        plan.device_properties = vec![
            DevicePropertyEntry {
                path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
                properties: vec![DeviceProperty {
                    key: "a".into(),
                    value: PlistScalar::Int(1),
                }],
                reason: "gpu".into(),
            },
            DevicePropertyEntry {
                path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
                properties: vec![
                    DeviceProperty {
                        key: "a".into(),
                        value: PlistScalar::Int(2),
                    },
                    DeviceProperty {
                        key: "b".into(),
                        value: PlistScalar::Int(3),
                    },
                ],
                reason: "other".into(),
            },
        ];
        plan.mmio_whitelist = vec![
            MmioEntry {
                address: 0xFD00_0000,
                comment: "a".into(),
                enabled: true,
            },
            MmioEntry {
                address: 0xFD00_0000,
                comment: "b".into(),
                enabled: true,
            },
        ];
        plan.notes = vec![
            note(NoteLevel::Info, "cpu", "x", "1"),
            note(NoteLevel::Info, "cpu", "x", "2"),
            note(NoteLevel::Info, "cpu", "x", "1"),
        ];
        plan.post_install = vec![
            note(NoteLevel::Info, "usb", "Map", "a"),
            note(NoteLevel::Info, "usb", "Map", "a"),
        ];
        finalize(&mut plan);
        assert_eq!(plan.boot_args, vec!["-v", "agdpmod=pikera"]);
        assert_eq!(plan.device_properties.len(), 1);
        let props = &plan.device_properties[0].properties;
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].value, PlistScalar::Int(1));
        assert_eq!(props[1].key, "b");
        assert_eq!(plan.device_properties[0].reason, "gpu; other");
        assert_eq!(plan.mmio_whitelist.len(), 1);
        assert_eq!(plan.mmio_whitelist[0].comment, "a");
        // Same title with a different detail is kept; exact repeats go.
        let details: Vec<&str> = plan.notes.iter().map(|n| n.detail.as_str()).collect();
        assert_eq!(details, vec!["1", "2"]);
        assert_eq!(plan.post_install.len(), 1);
    }
}
