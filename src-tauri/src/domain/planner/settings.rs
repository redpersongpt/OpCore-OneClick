//! Misc (boot picker, debug, security), NVRAM, PlatformInfo, UEFI drivers,
//! APFS/Output/Input settings, tools and final boot-args / csr-active-config.
//!
//! Runs after every hardware stage: boot-args are only appended, and the
//! SIP / Secure Boot values follow the root-patch rule of
//! [`super::root_patching_planned`].

use crate::domain::model::{
    BuildPlan, CpuPlatform as P, DriverPlan, GpuFamily, MacOsVersion, NoteLevel, PickerStyle,
    PlistScalar, SettingMap,
};

use super::{note, root_patching_planned, PlanContext};

/// csr-active-config for OCLP root patches: CSR_ALLOW_UNTRUSTED_KEXTS |
/// CSR_ALLOW_UNRESTRICTED_FS | CSR_ALLOW_UNAUTHENTICATED_ROOT, NVRAM bytes
/// `03080000` (research-opencore-macos §3.4, OCLP `security.py`).
pub const SIP_ROOT_PATCH: u32 = 0x0000_0803;
/// The same plus CSR_ALLOW_UNAPPROVED_KEXTS for the NVIDIA Web Driver patch
/// set, NVRAM bytes `030A0000` (research-gpu §8).
pub const SIP_ROOT_PATCH_NVIDIA: u32 = 0x0000_0A03;

/// OpenCanopy icon set shipped in OcBinaryData `Resources/Image`.
const PICKER_VARIANT: &str = "Acidanthera\\GoldenGate";

pub fn apply(ctx: &PlanContext, plan: &mut BuildPlan) {
    let root_patch = root_patching_planned(ctx, plan);
    misc_boot(ctx, plan);
    misc_debug(ctx, plan);
    misc_security(ctx, plan, root_patch);
    tools(ctx, plan);
    boot_args(ctx, plan);
    csr_active_config(ctx, plan, root_patch);
    nvram(ctx, plan);
    platform_info(ctx, plan);
    plan.drivers.extend(drivers(ctx));
    uefi(ctx, plan);
    if ctx.legacy_bios {
        plan.notes.push(note(
            NoteLevel::Warning,
            "firmware",
            "Legacy BIOS boot",
            "This firmware has no UEFI. OpenCore starts through OpenDuet, which needs OpenCore's legacy boot \
             sectors (Utilities/LegacyBoot) on the USB drive; the config uses HfsPlusLegacy.efi, OpenUsbKbDxe.efi \
             and OpenDuet's built-in emulated NVRAM.",
        ));
    }
}

fn set(map: &mut SettingMap, key: &str, value: PlistScalar) {
    map.insert(key.to_string(), value);
}

fn flag(map: &mut SettingMap, key: &str, value: bool) {
    set(map, key, PlistScalar::Bool(value));
}

fn text(map: &mut SettingMap, key: &str, value: &str) {
    set(map, key, PlistScalar::Str(value.to_string()));
}

fn int(map: &mut SettingMap, key: &str, value: i64) {
    set(map, key, PlistScalar::Int(value));
}

fn misc_boot(ctx: &PlanContext, plan: &mut BuildPlan) {
    let graphical = ctx.options.picker == PickerStyle::Graphical;
    let m = &mut plan.misc_boot;
    int(m, "ConsoleAttributes", 0);
    text(m, "HibernateMode", "None");
    // The USB installer boots com.apple.recovery.boot, which OpenCore treats
    // as a macOS recovery entry; with HideAuxiliary it is hidden until Space
    // is pressed (Configuration.tex HideAuxiliary, BootEntryManagement.c;
    // research-recovery §6.2).
    flag(m, "HideAuxiliary", false);
    text(m, "LauncherOption", "Disabled");
    text(m, "LauncherPath", "Default");
    // OC_ATTR_USE_VOLUME_ICON | OC_ATTR_USE_POINTER_CONTROL (Sample value).
    int(m, "PickerAttributes", 17);
    flag(m, "PickerAudioAssist", false);
    text(
        m,
        "PickerMode",
        if graphical { "External" } else { "Builtin" },
    );
    text(
        m,
        "PickerVariant",
        if graphical { PICKER_VARIANT } else { "Auto" },
    );
    flag(m, "PollAppleHotKeys", false);
    flag(m, "ShowPicker", true);
    int(m, "TakeoffDelay", 0);
    int(
        m,
        "Timeout",
        i64::from(ctx.options.picker_timeout.unwrap_or(5)),
    );
    plan.post_install.push(note(
        NoteLevel::Info,
        "bootloader",
        "Boot picker after install",
        "Auxiliary entries (recovery, Reset NVRAM, tools) are shown so the installer appears without \
         pressing Space. Once macOS is installed and the EFI is copied to the internal disk, \
         Misc/Boot/HideAuxiliary can be set to true.",
    ));
}

fn misc_debug(ctx: &PlanContext, plan: &mut BuildPlan) {
    let d = &mut plan.misc_debug;
    // Dortania: AppleDebug, ApplePanic, DisableWatchDog on; Target 67 adds
    // file logging, which only the DEBUG build produces in useful detail.
    flag(d, "AppleDebug", true);
    flag(d, "ApplePanic", true);
    flag(d, "DisableWatchDog", true);
    int(d, "DisplayDelay", 0);
    int(d, "DisplayLevel", 2_147_483_650);
    flag(d, "SysReport", false);
    int(d, "Target", if ctx.options.debug_opencore { 67 } else { 3 });
}

fn misc_security(ctx: &PlanContext, plan: &mut BuildPlan, root_patch: bool) {
    let planned = if plan.smbios.secure_boot_model.is_empty() {
        "Disabled"
    } else {
        plan.smbios.secure_boot_model.as_str()
    };
    // Root-patched systems break the sealed system volume, so Apple Secure
    // Boot must be off (OCLP security.py; research-gpu §8).
    let secure_boot_model = if root_patch { "Disabled" } else { planned }.to_string();
    if root_patch && planned != "Disabled" {
        plan.notes.push(note(
            NoteLevel::Info,
            "security",
            "Apple Secure Boot disabled",
            format!(
                "SecureBootModel is Disabled instead of {planned} because root patches modify the system \
                 volume."
            ),
        ));
    }
    let airport_itlwm = plan
        .kexts
        .iter()
        .any(|k| k.enabled && k.catalog_id.starts_with("AirportItlwm"));
    let modern_wireless = plan
        .kexts
        .iter()
        .any(|k| k.enabled && k.catalog_id == "IO80211FamilyLegacy");
    if airport_itlwm && !modern_wireless && secure_boot_model == "Disabled" {
        plan.notes.push(note(
            NoteLevel::Warning,
            "wifi",
            "AirportItlwm with Secure Boot disabled",
            "AirportItlwm loads only with Apple Secure Boot on (or with IO80211Family force-loaded). With \
             SecureBootModel Disabled the Wi-Fi may be missing in Recovery; use Ethernet for the install.",
        ));
    }

    let s = &mut plan.misc_security;
    flag(s, "AllowSetDefault", true);
    int(s, "ApECID", 0);
    flag(s, "AuthRestart", false);
    flag(s, "BlacklistAppleUpdate", true);
    text(s, "DmgLoading", "Signed");
    flag(s, "EnablePassword", false);
    // Bit 0x1 exposes the OpenCore partition UUID that the emulated NVRAM
    // logout hook needs on OpenDuet (Configuration.tex OpenVariableRuntimeDxe).
    int(
        s,
        "ExposeSensitiveData",
        if ctx.legacy_bios { 7 } else { 6 },
    );
    // 0 lets the picker see the USB installer (the failsafe hides USB/FAT).
    int(s, "ScanPolicy", 0);
    text(s, "SecureBootModel", &secure_boot_model);
    text(s, "Vault", "Optional");
}

fn tools(ctx: &PlanContext, plan: &mut BuildPlan) {
    plan.tools.push("OpenShell.efi".into());
    // CFG Lock quirks are on because the lock state is unknown; ControlMsrE2
    // reads (and on some boards unlocks) it.
    let cfg_quirk = ["AppleXcpmCfgLock", "AppleCpuPmCfgLock"]
        .iter()
        .any(|k| matches!(plan.kernel_quirks.get(*k), Some(PlistScalar::Bool(true))));
    if cfg_quirk && ctx.is_intel() && !ctx.is_vm && !ctx.legacy_bios {
        plan.tools.push("ControlMsrE2.efi".into());
    }
}

/// Verbose and user boot-args are appended after everything the hardware
/// stages added; a user argument whose key a stage already set is dropped
/// with a note instead of overriding the stage.
fn boot_args(ctx: &PlanContext, plan: &mut BuildPlan) {
    if ctx.options.verbose {
        for arg in ["-v", "keepsyms=1", "debug=0x100"] {
            plan.boot_args.push(arg.into());
        }
    }
    let Some(extra) = ctx.options.extra_boot_args.as_deref() else {
        return;
    };
    let key = |arg: &str| arg.split('=').next().unwrap_or(arg).to_string();
    let user_start = plan.boot_args.len();
    for token in extra.split_whitespace() {
        if !token.chars().all(|c| c.is_ascii_graphic()) {
            plan.notes.push(note(
                NoteLevel::Warning,
                "boot-args",
                format!("Boot argument {token} ignored"),
                "Boot arguments must be printable ASCII.",
            ));
            continue;
        }
        let existing = plan
            .boot_args
            .iter()
            .enumerate()
            .flat_map(|(i, a)| a.split_whitespace().map(move |t| (i, t)))
            .find(|(_, a)| key(a) == key(token))
            .map(|(i, a)| (i >= user_start, a.to_string()));
        match existing {
            Some((_, same)) if same == token => {}
            Some((true, other)) => plan.notes.push(note(
                NoteLevel::Warning,
                "boot-args",
                format!("Boot argument {token} ignored"),
                format!(
                    "{other} comes earlier in the extra boot arguments; the first value is kept."
                ),
            )),
            Some((false, other)) => plan.notes.push(note(
                NoteLevel::Warning,
                "boot-args",
                format!("Boot argument {token} ignored"),
                format!("The build already sets {other}; it is kept."),
            )),
            None => plan.boot_args.push(token.to_string()),
        }
    }
}

/// SIP stays fully enabled unless OCLP root patching is planned
/// ([`super::root_patching_planned`]): then `03080000`, or `030A0000` when
/// the display runs on an NVIDIA Web Driver card.
fn csr_active_config(ctx: &PlanContext, plan: &mut BuildPlan, root_patch: bool) {
    if !root_patch {
        plan.csr_active_config = 0;
        return;
    }
    let web_driver = ctx
        .display
        .as_ref()
        .and_then(|d| ctx.display_gpu(d))
        .is_some_and(|g| {
            matches!(
                g.family,
                GpuFamily::NvidiaFermi | GpuFamily::NvidiaMaxwell | GpuFamily::NvidiaPascal
            )
        });
    plan.csr_active_config = if web_driver {
        SIP_ROOT_PATCH_NVIDIA
    } else {
        SIP_ROOT_PATCH
    };
    plan.notes.push(note(
        NoteLevel::Warning,
        "security",
        "System Integrity Protection lowered",
        format!(
            "csr-active-config is {:08X} (little endian) so OpenCore Legacy Patcher can apply its root \
             patches after install. Updates then download the full installer and the patches must be \
             re-applied after each update.",
            plan.csr_active_config.swap_bytes()
        ),
    ));
}

fn nvram(ctx: &PlanContext, plan: &mut BuildPlan) {
    // Dortania haswell-e / broadwell-e: X99 NVRAM is unreliable, so variables
    // are not written to flash (LegacyOverwrite YES, WriteFlash NO).
    let x99 = !ctx.is_vm && matches!(ctx.platform(), P::HaswellE | P::BroadwellE);
    let n = &mut plan.nvram_settings;
    flag(n, "LegacyOverwrite", x99);
    flag(n, "WriteFlash", !x99);
}

fn platform_info(ctx: &PlanContext, plan: &mut BuildPlan) {
    let p = &mut plan.platform_info;
    flag(p, "Automatic", true);
    flag(p, "CustomMemory", false);
    flag(p, "UpdateDataHub", true);
    flag(p, "UpdateNVRAM", true);
    flag(p, "UpdateSMBIOS", true);
    // Custom only together with the CustomSMBIOSGuid quirk the quirks stage
    // sets for Dell laptops (Configuration.tex; audit-config-generator H5).
    text(
        p,
        "UpdateSMBIOSMode",
        if ctx.needs_custom_smbios() {
            "Custom"
        } else {
            "Create"
        },
    );
    flag(p, "UseRawUuidEncoding", false);
}

fn driver(path: &str, source: &str, comment: &str) -> DriverPlan {
    DriverPlan {
        path: path.into(),
        load_early: false,
        enabled: true,
        comment: comment.into(),
        source: source.into(),
    }
}

/// CPUs without RDRAND need HfsPlusLegacy.efi (Dortania penryn, clarkdale,
/// sandy-bridge, nehalem pages; research-recovery §6.2: "Sandy Bridge and
/// older, low-end Ivy Bridge"). Sandy Bridge-E and AMD family 15h/16h predate
/// RDRAND as well, and OpenDuet loads the legacy image (critic-gaps §1).
fn needs_legacy_hfs(ctx: &PlanContext) -> bool {
    if ctx.legacy_bios {
        return true;
    }
    let name = ctx.profile.cpu.name.to_ascii_lowercase();
    let low_end = name.contains("pentium") || name.contains("celeron");
    match ctx.platform() {
        P::Penryn
        | P::Lynnfield
        | P::Arrandale
        | P::SandyBridge
        | P::NehalemHedt
        | P::SandyBridgeE
        | P::AmdBulldozer
        | P::AmdJaguar => true,
        P::IvyBridge => low_end,
        _ => false,
    }
}

/// UEFI drivers in Sample.plist order. LoadEarly stays false: ocvalidate
/// allows it only for OpenRuntime with OpenVariableRuntimeDxe, which is not
/// used (OpenDuet has emulated NVRAM built in).
pub fn drivers(ctx: &PlanContext) -> Vec<DriverPlan> {
    let mut list = vec![driver(
        "OpenRuntime.efi",
        "opencore",
        "Memory and NVRAM runtime fixes",
    )];
    // Every recovery BaseSystem.dmg is an HFS+ image (research-recovery §6.2).
    if ctx.is_vm {
        // VMs (OSX-KVM) use the open-source driver; no RDRAND assumption.
        list.push(open_hfs_plus());
    } else if needs_legacy_hfs(ctx) {
        list.push(driver(
            "HfsPlusLegacy.efi",
            "ocbinarydata",
            "HFS+ file system for CPUs without RDRAND",
        ));
    } else {
        list.push(driver("HfsPlus.efi", "ocbinarydata", "HFS+ file system"));
    }
    if ctx.options.picker == PickerStyle::Graphical {
        list.push(driver(
            "OpenCanopy.efi",
            "opencore",
            "Graphical boot picker",
        ));
    }
    if ctx.legacy_bios {
        // Dortania legacy pages: OpenUsbKbDxe "if your firmware does not support UEFI".
        list.push(driver(
            "OpenUsbKbDxe.efi",
            "opencore",
            "USB keyboard in the picker on legacy BIOS",
        ));
    }
    list.push(driver(
        "ResetNvramEntry.efi",
        "opencore",
        "Reset NVRAM picker entry",
    ));
    list
}

/// Open-source HFS+ driver from the OpenCore release: the fallback when
/// OcBinaryData cannot be downloaded (about three times slower).
pub fn open_hfs_plus() -> DriverPlan {
    driver(
        "OpenHfsPlus.efi",
        "opencore",
        "HFS+ file system (open source)",
    )
}

fn uefi(ctx: &PlanContext, plan: &mut BuildPlan) {
    let a = &mut plan.uefi_apfs;
    flag(a, "EnableJumpstart", true);
    flag(a, "GlobalConnect", false);
    flag(a, "HideVerbose", true);
    flag(a, "JumpstartHotPlug", false);
    // OpenCore loads only Big Sur+ APFS drivers by default; older targets
    // need "no restriction" (Dortania, audit-config-generator H3).
    let floor = if ctx.target < MacOsVersion::BigSur {
        -1
    } else {
        0
    };
    int(a, "MinDate", floor);
    int(a, "MinVersion", floor);

    let o = &mut plan.uefi_output;
    flag(o, "DirectGopRendering", false);
    flag(o, "ForceResolution", false);
    text(o, "GopPassThrough", "Disabled");
    // Fixes black pickers on ASUS and other GOP-on-the-wrong-handle firmware.
    flag(o, "ProvideConsoleGop", true);
    flag(o, "ReconnectOnResChange", false);
    text(o, "Resolution", "Max");
    text(o, "TextRenderer", "BuiltinGraphics");
    int(o, "UIScale", 0);

    let i = &mut plan.uefi_input;
    flag(i, "KeyFiltering", false);
    int(i, "KeyForgetThreshold", 5);
    // OpenUsbKbDxe replaces KeySupport on legacy BIOS (ocvalidate rejects both).
    flag(i, "KeySupport", !ctx.legacy_bios);
    text(i, "KeySupportMode", "Auto");
    flag(i, "KeySwap", false);
    flag(i, "PointerSupport", false);
    int(i, "TimerResolution", 50_000);
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{empty_plan, note, quirks, smbios, DisplayPlan, ROOT_PATCH_COMPONENT};
    use super::*;
    use crate::domain::config_writer::{write_config, ConfigInputs};
    use crate::domain::cpu_db;
    use crate::domain::model::{
        BuildOptions, CpuPlatform, FormFactor, HardwareProfile, KextSelection, PlatformIdentity,
        ProfileGpu, VmKind,
    };
    use MacOsVersion::*;

    fn run_full(p: &HardwareProfile, o: &BuildOptions, d: &DisplayPlan) -> BuildPlan {
        let mut ctx = PlanContext::new(p, o);
        ctx.display = Some(d.clone());
        let mut plan = empty_plan(o.target);
        smbios::apply(&ctx, d, &mut plan).unwrap();
        quirks::apply(&ctx, &mut plan);
        apply(&ctx, &mut plan);
        plan
    }

    fn run_settings(p: &HardwareProfile, o: &BuildOptions, plan: &mut BuildPlan) {
        let ctx = PlanContext::new(p, o);
        apply(&ctx, plan);
    }

    fn coffee(gpus: Vec<ProfileGpu>) -> HardwareProfile {
        profile(
            cpu(
                CpuPlatform::CoffeeLake,
                "Intel(R) Core(TM) i7-8700K",
                "Coffee Lake-S",
                6,
            ),
            FormFactor::Desktop,
            gpus,
        )
    }

    fn paths(plan: &BuildPlan) -> Vec<&str> {
        plan.drivers.iter().map(|d| d.path.as_str()).collect()
    }

    #[test]
    fn picker_and_debug() {
        let p = coffee(vec![gpu(GpuFamily::IntelCoffeeLake, true)]);
        let mut o = options(Sequoia);
        let mut plan = empty_plan(Sequoia);
        run_settings(&p, &o, &mut plan);
        assert_eq!(
            str_of(&plan.misc_boot, "PickerMode").as_deref(),
            Some("External")
        );
        assert_eq!(
            str_of(&plan.misc_boot, "PickerVariant").as_deref(),
            Some("Acidanthera\\GoldenGate")
        );
        assert_eq!(int_of(&plan.misc_boot, "Timeout"), Some(5));
        assert!(!on(&plan.misc_boot, "HideAuxiliary"));
        assert_eq!(int_of(&plan.misc_debug, "Target"), Some(3));
        assert!(paths(&plan).contains(&"OpenCanopy.efi"));

        o.picker = PickerStyle::Text;
        o.picker_timeout = Some(0);
        o.debug_opencore = true;
        let mut plan = empty_plan(Sequoia);
        run_settings(&p, &o, &mut plan);
        assert_eq!(
            str_of(&plan.misc_boot, "PickerMode").as_deref(),
            Some("Builtin")
        );
        assert_eq!(int_of(&plan.misc_boot, "Timeout"), Some(0));
        assert_eq!(int_of(&plan.misc_debug, "Target"), Some(67));
        assert!(!paths(&plan).contains(&"OpenCanopy.efi"));
    }

    #[test]
    fn security_follows_smbios_and_root_patches() {
        let p = coffee(vec![gpu(GpuFamily::IntelCoffeeLake, true)]);
        let o = options(Monterey);
        let plan = run_full(&p, &o, &display(Some(0), Some(0), false));
        assert_eq!(
            str_of(&plan.misc_security, "SecureBootModel").as_deref(),
            Some("Default")
        );
        assert_eq!(
            str_of(&plan.misc_security, "Vault").as_deref(),
            Some("Optional")
        );
        assert_eq!(
            str_of(&plan.misc_security, "DmgLoading").as_deref(),
            Some("Signed")
        );
        assert_eq!(int_of(&plan.misc_security, "ScanPolicy"), Some(0));
        assert_eq!(int_of(&plan.misc_security, "ExposeSensitiveData"), Some(6));
        assert_eq!(plan.csr_active_config, 0);

        // A root-patch kext from another stage flips SIP and Secure Boot.
        let ctx = PlanContext::new(&p, &o);
        let mut plan = empty_plan(Monterey);
        plan.smbios.secure_boot_model = "Default".into();
        plan.kexts.push(KextSelection {
            catalog_id: "AMFIPass".into(),
            bundle: "AMFIPass.kext".into(),
            plugins: vec![],
            enabled: true,
            min_kernel: Some("20.0.0".into()),
            max_kernel: None,
            required: false,
            reason: String::new(),
        });
        apply(&ctx, &mut plan);
        assert_eq!(plan.csr_active_config, SIP_ROOT_PATCH);
        assert_eq!(
            plan.csr_active_config.to_le_bytes(),
            [0x03, 0x08, 0x00, 0x00]
        );
        assert_eq!(
            str_of(&plan.misc_security, "SecureBootModel").as_deref(),
            Some("Disabled")
        );
        assert!(plan
            .notes
            .iter()
            .any(|n| n.title == "System Integrity Protection lowered"
                && n.detail.contains("03080000")));

        // NVIDIA Web Driver display on a root-patched release.
        let p = profile(
            cpu(
                CpuPlatform::Haswell,
                "Intel(R) Core(TM) i7-4790K",
                "Haswell",
                4,
            ),
            FormFactor::Desktop,
            vec![gpu(GpuFamily::NvidiaMaxwell, false)],
        );
        let o = options(BigSur);
        let mut ctx = PlanContext::new(&p, &o);
        ctx.display = Some(display(Some(0), None, false));
        let mut plan = empty_plan(BigSur);
        plan.post_install.push(note(
            NoteLevel::Info,
            ROOT_PATCH_COMPONENT,
            "NVIDIA",
            "Web Driver patch",
        ));
        apply(&ctx, &mut plan);
        assert_eq!(plan.csr_active_config, SIP_ROOT_PATCH_NVIDIA);
        assert_eq!(
            plan.csr_active_config.to_le_bytes(),
            [0x03, 0x0A, 0x00, 0x00]
        );

        // Kepler on Monterey predicted from the display decision alone.
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
        let plan = run_full(&p, &options(Monterey), &display(Some(0), Some(1), true));
        assert_eq!(plan.csr_active_config, SIP_ROOT_PATCH);
        assert_eq!(
            str_of(&plan.misc_security, "SecureBootModel").as_deref(),
            Some("Disabled")
        );
    }

    #[test]
    fn boot_args_are_appended() {
        let p = coffee(vec![]);
        let mut o = options(Sequoia);
        o.extra_boot_args = Some("alcid=11 agdpmod=ignore  -wegnoegpu alcid=7".into());
        let mut plan = empty_plan(Sequoia);
        plan.boot_args = vec!["agdpmod=pikera".into(), "-wegnoegpu".into()];
        run_settings(&p, &o, &mut plan);
        assert_eq!(
            plan.boot_args,
            vec![
                "agdpmod=pikera",
                "-wegnoegpu",
                "-v",
                "keepsyms=1",
                "debug=0x100",
                "alcid=11"
            ]
        );
        assert!(plan
            .notes
            .iter()
            .any(|n| n.title.contains("agdpmod=ignore") && n.detail.contains("agdpmod=pikera")));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.title.contains("alcid=7") && n.detail.contains("extra boot arguments")));
        let mut plan = empty_plan(Sequoia);
        o.extra_boot_args = Some("caf\u{e9}=1 -lilubetaall".into());
        run_settings(&p, &o, &mut plan);
        assert_eq!(
            plan.boot_args.last().map(String::as_str),
            Some("-lilubetaall")
        );
        assert!(plan
            .notes
            .iter()
            .any(|n| n.detail.contains("printable ASCII")));
        o.verbose = false;
        o.extra_boot_args = None;
        let mut plan = empty_plan(Sequoia);
        run_settings(&p, &o, &mut plan);
        assert!(plan.boot_args.is_empty());
    }

    #[test]
    fn drivers_per_platform() {
        let o = options(Sequoia);
        let mut plan = empty_plan(Sequoia);
        run_settings(&coffee(vec![]), &o, &mut plan);
        assert_eq!(
            paths(&plan),
            vec![
                "OpenRuntime.efi",
                "HfsPlus.efi",
                "OpenCanopy.efi",
                "ResetNvramEntry.efi"
            ]
        );
        assert!(plan.drivers.iter().all(|d| !d.load_early && d.enabled));
        assert_eq!(plan.drivers[1].source, "ocbinarydata");
        assert_eq!(plan.drivers[0].source, "opencore");

        for (platform, name) in [
            (CpuPlatform::SandyBridge, "Intel(R) Core(TM) i7-2600K"),
            (CpuPlatform::Penryn, "Intel(R) Core(TM)2 Quad Q9550"),
            (CpuPlatform::AmdBulldozer, "AMD FX(tm)-8350"),
            (CpuPlatform::IvyBridge, "Intel(R) Pentium(R) CPU G2020"),
        ] {
            let p = profile(cpu(platform, name, "x", 4), FormFactor::Desktop, vec![]);
            let mut plan = empty_plan(HighSierra);
            run_settings(&p, &options(HighSierra), &mut plan);
            assert!(paths(&plan).contains(&"HfsPlusLegacy.efi"), "{platform:?}");
        }
        let ivy = profile(
            cpu(CpuPlatform::IvyBridge, "Intel(R) Core(TM) i7-3770", "x", 4),
            FormFactor::Desktop,
            vec![],
        );
        let mut plan = empty_plan(Catalina);
        run_settings(&ivy, &options(Catalina), &mut plan);
        assert!(paths(&plan).contains(&"HfsPlus.efi"));

        let mut legacy = profile(
            cpu(
                CpuPlatform::Penryn,
                "Intel(R) Core(TM)2 Duo E8400",
                "Penryn",
                2,
            ),
            FormFactor::Desktop,
            vec![],
        );
        legacy.firmware_uefi = Some(false);
        let mut plan = empty_plan(HighSierra);
        run_settings(&legacy, &options(HighSierra), &mut plan);
        assert!(paths(&plan).contains(&"OpenUsbKbDxe.efi"));
        assert!(!on(&plan.uefi_input, "KeySupport"));
        assert_eq!(int_of(&plan.misc_security, "ExposeSensitiveData"), Some(7));

        let vm = vm_profile(CpuPlatform::Unknown, VmKind::Kvm);
        let mut plan = empty_plan(Sequoia);
        run_settings(&vm, &o, &mut plan);
        assert!(paths(&plan).contains(&"OpenHfsPlus.efi"));
        assert_eq!(open_hfs_plus().source, "opencore");
    }

    #[test]
    fn apfs_nvram_platform_info() {
        let p = coffee(vec![]);
        for (target, floor) in [(HighSierra, -1), (Catalina, -1), (BigSur, 0), (Tahoe, 0)] {
            let mut plan = empty_plan(target);
            run_settings(&p, &options(target), &mut plan);
            assert_eq!(
                int_of(&plan.uefi_apfs, "MinDate"),
                Some(floor),
                "{target:?}"
            );
            assert_eq!(
                int_of(&plan.uefi_apfs, "MinVersion"),
                Some(floor),
                "{target:?}"
            );
            assert!(on(&plan.uefi_apfs, "EnableJumpstart"));
            assert!(on(&plan.nvram_settings, "WriteFlash"));
            assert!(!on(&plan.nvram_settings, "LegacyOverwrite"));
            assert_eq!(
                str_of(&plan.platform_info, "UpdateSMBIOSMode").as_deref(),
                Some("Create")
            );
            assert!(on(&plan.uefi_output, "ProvideConsoleGop"));
            assert!(on(&plan.uefi_input, "KeySupport"));
        }
        let x99 = profile(
            cpu(
                CpuPlatform::HaswellE,
                "Intel(R) Core(TM) i7-5960X",
                "Haswell-E",
                8,
            ),
            FormFactor::Desktop,
            vec![],
        );
        let mut plan = empty_plan(Sequoia);
        run_settings(&x99, &options(Sequoia), &mut plan);
        assert!(!on(&plan.nvram_settings, "WriteFlash"));
        assert!(on(&plan.nvram_settings, "LegacyOverwrite"));

        let mut dell = profile(
            cpu(
                CpuPlatform::KabyLake,
                "Intel(R) Core(TM) i5-8250U",
                "Kaby Lake-R",
                4,
            ),
            FormFactor::Laptop,
            vec![],
        );
        dell.motherboard_vendor = "Dell Inc.".into();
        let mut plan = empty_plan(Ventura);
        run_settings(&dell, &options(Ventura), &mut plan);
        assert_eq!(
            str_of(&plan.platform_info, "UpdateSMBIOSMode").as_deref(),
            Some("Custom")
        );
    }

    #[test]
    fn control_msr_tool_follows_cfg_quirks() {
        let p = coffee(vec![gpu(GpuFamily::IntelCoffeeLake, true)]);
        let plan = run_full(&p, &options(Sequoia), &display(Some(0), Some(0), false));
        assert_eq!(plan.tools, vec!["OpenShell.efi", "ControlMsrE2.efi"]);
        let amd = profile(
            cpu(
                CpuPlatform::AmdZen3,
                "AMD Ryzen 7 5800X 8-Core Processor",
                "Vermeer",
                8,
            ),
            FormFactor::Desktop,
            vec![gpu(GpuFamily::AmdNavi21, false)],
        );
        let plan = run_full(&amd, &options(Sequoia), &display(Some(0), None, false));
        assert_eq!(plan.tools, vec!["OpenShell.efi"]);
    }

    /// The core stages produce configs the OpenCore 1.0.8 writer accepts
    /// (schema, types, patch bytes, picker/driver consistency) for every
    /// supported platform, form factor, firmware type, VMs and a spread of
    /// releases.
    #[test]
    fn config_writer_accepts_every_platform() {
        let forms = [
            FormFactor::Desktop,
            FormFactor::Laptop,
            FormFactor::MiniPc,
            FormFactor::AllInOne,
        ];
        let mut profiles: Vec<HardwareProfile> = Vec::new();
        for &platform in cpu_db::all_platforms() {
            if !cpu_db::platform_info(platform).supported {
                continue;
            }
            for form in forms {
                let mut c = cpu(platform, "Test CPU", "Test", 6);
                c.is_mobile = matches!(form, FormFactor::Laptop | FormFactor::MiniPc);
                let mut p = profile(c, form, vec![gpu(GpuFamily::AmdPolaris, false)]);
                profiles.push(p.clone());
                // Pre-2011 boards without UEFI boot through OpenDuet.
                if matches!(
                    platform,
                    CpuPlatform::Penryn
                        | CpuPlatform::Lynnfield
                        | CpuPlatform::Arrandale
                        | CpuPlatform::NehalemHedt
                        | CpuPlatform::SandyBridge
                        | CpuPlatform::AmdBulldozer
                ) {
                    p.firmware_uefi = Some(false);
                    profiles.push(p);
                }
            }
        }
        for kind in [VmKind::Kvm, VmKind::Vmware, VmKind::HyperV] {
            profiles.push(vm_profile(CpuPlatform::Unknown, kind));
            profiles.push(vm_profile(CpuPlatform::Haswell, kind));
        }
        let mut checked = 0;
        for p in &profiles {
            let platform = p.cpu.platform;
            let form = p.form_factor;
            let valid: Vec<MacOsVersion> = MacOsVersion::ALL
                .into_iter()
                .filter(|t| super::super::validate(p, &options(*t)).is_ok())
                .collect();
            // Oldest, a middle one and the newest release the CPU allows.
            let mut targets = vec![];
            if let (Some(first), Some(last)) = (valid.first(), valid.last()) {
                targets = vec![*first, valid[valid.len() / 2], *last];
                targets.dedup();
            }
            for target in targets {
                let o = options(target);
                let plan = run_full(p, &o, &display(Some(0), None, false));
                if p.firmware_uefi == Some(false) {
                    assert!(!on(&plan.uefi_input, "KeySupport"));
                    assert!(!on(&plan.uefi_quirks, "RequestBootVarRouting"));
                    assert!(plan.drivers.iter().any(|d| d.path == "OpenUsbKbDxe.efi"));
                }
                if p.vm.is_some() {
                    assert!(plan.drivers.iter().any(|d| d.path == "OpenHfsPlus.efi"));
                    assert!(on(&plan.kernel_quirks, "ProvideCurrentCpuInfo"));
                }
                for (section, map) in [
                    ("Booter/Quirks", &plan.booter_quirks),
                    ("Kernel/Quirks", &plan.kernel_quirks),
                    ("Kernel/Emulate", &plan.kernel_emulate),
                    ("Misc/Boot", &plan.misc_boot),
                    ("Misc/Debug", &plan.misc_debug),
                    ("Misc/Security", &plan.misc_security),
                    ("NVRAM", &plan.nvram_settings),
                    ("PlatformInfo", &plan.platform_info),
                    ("UEFI/Quirks", &plan.uefi_quirks),
                    ("UEFI/APFS", &plan.uefi_apfs),
                    ("UEFI/Output", &plan.uefi_output),
                    ("UEFI/Input", &plan.uefi_input),
                ] {
                    assert_schema(section, map);
                }
                let identity = PlatformIdentity {
                    model: plan.smbios.model.clone(),
                    serial: "C02XG0FDH7JY".into(),
                    mlb: "C02839303QXH69FJA".into(),
                    system_uuid: "dbb364d6-44b2-4a02-b922-ab4396f16da8".into(),
                    rom: "112233445566".into(),
                };
                let driver_files: Vec<String> =
                    plan.drivers.iter().map(|d| d.path.clone()).collect();
                let out = write_config(
                    SAMPLE,
                    &ConfigInputs {
                        plan: &plan,
                        kernel_add: &[],
                        identity: &identity,
                        ssdt_files: &[],
                        driver_files: &driver_files,
                        tool_files: &plan.tools,
                    },
                )
                .unwrap_or_else(|e| panic!("{platform:?} {form:?} {target:?}: {e}"));
                let parsed = plist::Value::from_reader(std::io::Cursor::new(out)).unwrap();
                let root = parsed.as_dictionary().unwrap();
                let security = root["Misc"].as_dictionary().unwrap()["Security"]
                    .as_dictionary()
                    .unwrap();
                assert_eq!(
                    security["SecureBootModel"].as_string(),
                    str_of(&plan.misc_security, "SecureBootModel").as_deref()
                );
                let booter = root["Booter"].as_dictionary().unwrap();
                assert_eq!(
                    booter["Patch"].as_array().unwrap().len(),
                    usize::from(plan.smbios.board_id_skip)
                );
                checked += 1;
            }
        }
        assert!(checked > 150, "only {checked} combinations");
    }
}
