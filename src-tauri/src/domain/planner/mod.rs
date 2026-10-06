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

use crate::domain::chipset_db::{self, ChipsetInfo};
use crate::domain::cpu_db::{self, PlatformInfo};
use crate::domain::macos_db;
use crate::domain::model::{
    AcpiFacts, BuildOptions, BuildPlan, CpuPlatform, CpuVendor, FormFactor, HardwareProfile, MacOsVersion,
    SettingMap, SmbiosPlan,
};
use crate::domain::{bios, model};
use crate::error::AppError;

/// Facts every stage needs, derived once from the profile and options.
pub struct PlanContext<'a> {
    pub profile: &'a HardwareProfile,
    pub options: &'a BuildOptions,
    pub target: MacOsVersion,
    pub cpu: PlatformInfo,
    pub chipset: Option<ChipsetInfo>,
    pub is_vm: bool,
    pub is_laptop: bool,
    /// Laptop or all-in-one: internal panel needs backlight handling.
    pub has_panel: bool,
    /// Target needs AVX2 the CPU lacks → CryptexFixup.
    pub needs_cryptexfixup: bool,
    /// Parsed ACPI facts (profile.acpi, or parsed from profile.acpi_tables_dir).
    pub acpi: Option<AcpiFacts>,
}

impl<'a> PlanContext<'a> {
    pub fn new(profile: &'a HardwareProfile, options: &'a BuildOptions) -> Self {
        let cpu = cpu_db::platform_info(profile.cpu.platform);
        let chipset = profile
            .chipset
            .as_deref()
            .and_then(chipset_db::from_board_name)
            .or_else(|| chipset_db::from_board_name(&profile.motherboard_model));
        let has_avx2 = profile.cpu.has_avx2.unwrap_or(cpu.has_avx2);
        let acpi = profile.acpi.clone().or_else(|| {
            profile
                .acpi_tables_dir
                .as_deref()
                .and_then(|dir| crate::domain::acpi::parse_tables(std::path::Path::new(dir)).ok())
        });
        Self {
            profile,
            options,
            target: options.target,
            cpu,
            chipset,
            is_vm: profile.vm.is_some(),
            is_laptop: profile.form_factor == FormFactor::Laptop,
            has_panel: profile.form_factor.has_internal_panel(),
            needs_cryptexfixup: macos_db::requires_avx2(options.target) && !has_avx2,
            acpi,
        }
    }

    pub fn is_intel(&self) -> bool {
        self.profile.cpu.vendor == CpuVendor::Intel
    }

    pub fn is_amd(&self) -> bool {
        self.profile.cpu.vendor == CpuVendor::Amd
    }

    pub fn platform(&self) -> CpuPlatform {
        self.profile.cpu.platform
    }
}

/// Which GPU drives the displays, decided before anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    let ctx = PlanContext::new(profile, options);
    let display = graphics::choose_display(&ctx)?;

    let mut plan = empty_plan(options.target);
    smbios::apply(&ctx, &display, &mut plan)?;
    graphics::apply(&ctx, &display, &mut plan);
    kexts::apply(&ctx, &display, &mut plan);
    acpi::apply(&ctx, &mut plan);
    quirks::apply(&ctx, &mut plan);
    settings::apply(&ctx, &mut plan);
    notes::apply(&ctx, &display, &mut plan);
    plan.bios_settings = bios::recommended_settings(profile, options.target);
    finalize(&mut plan);
    Ok(plan)
}

fn validate(profile: &HardwareProfile, options: &BuildOptions) -> Result<(), AppError> {
    let platform = profile.cpu.platform;
    if platform == CpuPlatform::AppleSilicon || profile.cpu.vendor == CpuVendor::Apple {
        return Err(AppError::new(
            "APPLE_SILICON",
            "This Mac already runs macOS natively; OpenCore EFIs are for Intel/AMD PCs.",
        ));
    }
    if platform == CpuPlatform::Unknown && profile.vm.is_none() {
        return Err(AppError::new("CPU_UNKNOWN", "The CPU platform is unknown. Pick it manually in the hardware editor.")
            .recoverable()
            .with_suggestion("Open the hardware editor and choose the CPU generation."));
    }
    let info = cpu_db::platform_info(platform);
    if !info.supported && profile.vm.is_none() {
        return Err(AppError::new(
            "CPU_UNSUPPORTED",
            format!("{} CPUs cannot run macOS through OpenCore.", info.label),
        ));
    }
    if let Some(max) = info.max_macos {
        if options.target > max {
            return Err(AppError::new(
                "TARGET_ABOVE_CPU_LIMIT",
                format!("{} supports up to {}.", info.label, max.display_name()),
            )
            .recoverable()
            .with_suggestion("Choose an older macOS version."));
        }
    }
    if let Some(min) = info.min_macos {
        if options.target < min {
            return Err(AppError::new(
                "TARGET_BELOW_CPU_MINIMUM",
                format!("{} needs {} or newer.", info.label, min.display_name()),
            )
            .recoverable()
            .with_suggestion("Choose a newer macOS version."));
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

/// Remove duplicates while keeping first occurrence order.
fn finalize(plan: &mut BuildPlan) {
    let mut seen = std::collections::HashSet::new();
    plan.boot_args.retain(|arg| {
        let key = arg.split('=').next().unwrap_or(arg).to_string();
        seen.insert(key)
    });
    let mut seen = std::collections::HashSet::new();
    plan.kexts.retain(|k| seen.insert((k.catalog_id.clone(), k.bundle.clone())));
    let mut seen = std::collections::HashSet::new();
    plan.ssdts.retain(|s| seen.insert(s.file_name.clone()));
    let mut seen = std::collections::HashSet::new();
    plan.drivers.retain(|d| seen.insert(d.path.clone()));
    let _ = model::NoteLevel::Info;
}
