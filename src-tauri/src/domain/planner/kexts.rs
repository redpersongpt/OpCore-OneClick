//! Every non-GPU kext: Lilu/VirtualSMC + sensors, audio (AppleALC + layout-id),
//! Ethernet, Wi-Fi, Bluetooth, input, USB, storage, CPU helpers, OTA helpers.
//!
//! Catalog ids and bundle names come from `kext_catalog`; MinKernel/MaxKernel
//! values follow the Darwin ranges in the kext fact sheet (research-kexts §3,
//! §5) so one EFI stays safe when the user boots or updates to another
//! release. Besides `plan.kexts` this stage writes the audio `layout-id`, NIC
//! `device-id` spoofs and `built-in`, the Bluetooth NVRAM variables, the
//! IOSkywalkFamily block of the legacy wireless stack and the kext boot-args
//! (`revpatch=`, `alcid=`, `e1000=0`, `-amfipassbeta`, ...).

use crate::domain::chipset_db::ChipsetInfo;
use crate::domain::device_db::{
    self, BluetoothDriver, EthernetDriver, EthernetInfo, TouchpadDriver, WifiDriver, WifiInfo,
};
use crate::domain::kext_catalog;
use crate::domain::model::{
    BinaryPatch, BuildPlan, CpuPlatform, CpuVendor, DeviceBus, DeviceProperty, DevicePropertyEntry, FormFactor,
    GpuFamily, InputBus, IntelWifiStrategy, KernelBlock, KextSelection, MacOsVersion, NoteLevel, NvramVariable,
    PlanNote, PlistScalar, PluginSelection, ProfileNic, VmKind,
};
use crate::domain::{codec_db, cpu_db, gpu_db};

use super::{DisplayPlan, PlanContext};

/// NVRAM GUID of the Apple boot variables (Bluetooth controller info lives here).
pub const APPLE_NVRAM_GUID: &str = "7C436110-AB2A-4BBB-A880-FE41995C9F82";

/// Catalog id of AMFIPass; its presence in `plan.kexts` means the plan expects
/// OCLP-style root patches after installation (SIP must allow them).
pub const AMFIPASS_ID: &str = "AMFIPass";

/// True when the plan expects post-install root patches (legacy wireless
/// stack, GPU drivers removed from the target): AMFIPass is selected. Later
/// stages use this to relax SIP (`csr-active-config` 0x803).
pub fn plans_root_patch(plan: &BuildPlan) -> bool {
    plan.kexts.iter().any(|k| k.catalog_id == AMFIPASS_ID && k.enabled)
}

/// Select every non-GPU kext for `ctx` and write the properties, boot-args,
/// NVRAM variables, kernel blocks/patches and notes that come with them.
pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    let mut st = State::default();
    core(plan);
    sensors(ctx, display, plan);
    audio(ctx, plan);
    cpu_helpers(ctx, plan);
    vm(ctx, plan, &mut st);
    ethernet(ctx, plan, &mut st);
    wifi(ctx, plan, &mut st);
    bluetooth(ctx, plan);
    usb(ctx, plan);
    input(ctx, plan);
    storage(ctx, plan);
    if let Some(gpu) = gpu_root_patch(ctx, display) {
        st.root_patch.push(format!("{gpu} graphics drivers are restored by an OCLP root patch"));
    }
    root_patch_support(ctx, plan, &st);
    restrict_events(ctx, plan);
    network_check(plan, &st);
}

/// Facts collected across sections.
#[derive(Default)]
struct State {
    /// A wired NIC works on the target (installer and Recovery).
    ethernet_ok: bool,
    /// The Wi-Fi card works inside macOS Recovery (native IO80211 driver).
    wifi_recovery_ok: bool,
    /// The Wi-Fi card works once macOS is installed.
    wifi_ok: bool,
    /// A NIC already carries `built-in` (only the primary interface gets it).
    builtin_set: bool,
    /// Why AMFIPass is needed (root patches planned).
    root_patch: Vec<String>,
}

// ── Darwin ranges ───────────────────────────────────────────────────────────

const DARWIN_10_14: &str = "18.0.0";
const DARWIN_10_15: &str = "19.0.0";
const DARWIN_11: &str = "20.0.0";
const DARWIN_12: &str = "21.0.0";
const DARWIN_13: &str = "22.0.0";
const DARWIN_14: &str = "23.0.0";
const DARWIN_15: &str = "24.0.0";
const DARWIN_26: &str = "25.0.0";
const MAX_10_14: &str = "18.99.99";
const MAX_10_15: &str = "19.99.99";
const MAX_11: &str = "20.99.99";
const MAX_15: &str = "24.99.99";

// ── Small builders ──────────────────────────────────────────────────────────

fn kext(catalog: &str, bundle: &str, reason: impl Into<String>) -> KextSelection {
    KextSelection {
        catalog_id: catalog.to_string(),
        bundle: bundle.to_string(),
        plugins: Vec::new(),
        enabled: true,
        min_kernel: None,
        max_kernel: None,
        required: true,
        reason: reason.into(),
    }
}

trait SelectionExt: Sized {
    fn optional(self) -> Self;
    fn disabled(self) -> Self;
    fn required_if(self, required: bool) -> Self;
    fn min(self, kernel: &str) -> Self;
    fn max(self, kernel: &str) -> Self;
    fn plugin(self, bundle: &str, enabled: bool, min: Option<&str>, max: Option<&str>) -> Self;
}

impl SelectionExt for KextSelection {
    fn optional(mut self) -> Self {
        self.required = false;
        self
    }
    fn disabled(mut self) -> Self {
        self.enabled = false;
        self.required = false;
        self
    }
    fn required_if(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
    fn min(mut self, kernel: &str) -> Self {
        self.min_kernel = Some(kernel.to_string());
        self
    }
    fn max(mut self, kernel: &str) -> Self {
        self.max_kernel = Some(kernel.to_string());
        self
    }
    fn plugin(mut self, bundle: &str, enabled: bool, min: Option<&str>, max: Option<&str>) -> Self {
        self.plugins.push(PluginSelection {
            bundle: bundle.to_string(),
            enabled,
            min_kernel: min.map(str::to_string),
            max_kernel: max.map(str::to_string),
        });
        self
    }
}

/// Append a selection unless the same bundle of the same archive is already
/// planned (the first decision wins).
fn push(plan: &mut BuildPlan, sel: KextSelection) {
    debug_assert!(
        kext_catalog::entry(&sel.catalog_id).is_some_and(|e| e.provides(&sel.bundle)),
        "catalog {} does not provide {}",
        sel.catalog_id,
        sel.bundle
    );
    if plan.kexts.iter().any(|k| k.catalog_id == sel.catalog_id && k.bundle.eq_ignore_ascii_case(&sel.bundle)) {
        tracing::debug!(catalog = %sel.catalog_id, bundle = %sel.bundle, "kext already planned");
        return;
    }
    plan.kexts.push(sel);
}

fn note(plan: &mut BuildPlan, level: NoteLevel, component: &str, title: impl Into<String>, detail: impl Into<String>) {
    let (title, detail) = (title.into(), detail.into());
    if plan.notes.iter().any(|n| n.component == component && n.title == title && n.detail == detail) {
        return;
    }
    plan.notes.push(PlanNote { level, component: component.to_string(), title, detail });
}

fn post_install(plan: &mut BuildPlan, component: &str, title: impl Into<String>, detail: impl Into<String>) {
    let (title, detail) = (title.into(), detail.into());
    if plan.post_install.iter().any(|n| n.component == component && n.title == title) {
        return;
    }
    plan.post_install.push(PlanNote { level: NoteLevel::Info, component: component.to_string(), title, detail });
}

fn boot_arg(plan: &mut BuildPlan, arg: &str) {
    if !plan.boot_args.iter().any(|a| a == arg) {
        plan.boot_args.push(arg.to_string());
    }
}

/// Add comma-separated values to a `key=a,b` boot-arg, merging with one that
/// an earlier stage wrote (finalize keeps only the first `key=`).
fn merge_list_arg(plan: &mut BuildPlan, key: &str, values: &[&str]) {
    if values.is_empty() {
        return;
    }
    let prefix = format!("{key}=");
    match plan.boot_args.iter_mut().find(|a| a.starts_with(&prefix)) {
        Some(existing) => {
            let mut items: Vec<String> =
                existing[prefix.len()..].split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
            for v in values {
                if !items.iter().any(|i| i == v) {
                    items.push((*v).to_string());
                }
            }
            *existing = format!("{prefix}{}", items.join(","));
        }
        None => plan.boot_args.push(format!("{prefix}{}", values.join(","))),
    }
}

/// Set one DeviceProperties key on `path`, reusing this path's entry.
fn property(plan: &mut BuildPlan, path: &str, key: &str, value: PlistScalar, reason: &str) {
    let path = path.trim();
    match plan.device_properties.iter_mut().find(|e| e.path.eq_ignore_ascii_case(path)) {
        Some(entry) => {
            match entry.properties.iter_mut().find(|p| p.key == key) {
                Some(p) => p.value = value,
                None => entry.properties.push(DeviceProperty { key: key.to_string(), value }),
            }
            if !entry.reason.contains(reason) {
                entry.reason =
                    if entry.reason.is_empty() { reason.to_string() } else { format!("{}; {reason}", entry.reason) };
            }
        }
        None => plan.device_properties.push(DevicePropertyEntry {
            path: path.to_string(),
            properties: vec![DeviceProperty { key: key.to_string(), value }],
            reason: reason.to_string(),
        }),
    }
}

fn nic_path(nic: &ProfileNic) -> Option<&str> {
    nic.pci_path.as_deref().map(str::trim).filter(|p| !p.is_empty())
}

fn nic_device(nic: &ProfileNic) -> Option<u16> {
    nic.device_id.as_deref().and_then(device_db::parse_id16)
}

fn darwin_range(target: MacOsVersion) -> (String, String) {
    (target.min_kernel(), target.max_kernel())
}

// ── Core ────────────────────────────────────────────────────────────────────

/// Lilu and VirtualSMC go to the front of the list (Kernel->Add order; the
/// GPU stage may already have added its kext). A copy an earlier stage
/// planned is moved there instead of being duplicated.
fn core(plan: &mut BuildPlan) {
    let core = [
        ("Lilu", "Lilu.kext", "Kernel patching engine every Lilu plugin needs; loads first."),
        ("VirtualSMC", "VirtualSMC.kext", "SMC emulator: macOS does not boot without an SMC."),
    ];
    let front: Vec<KextSelection> = core
        .into_iter()
        .map(|(catalog, bundle, reason)| {
            match plan.kexts.iter().position(|k| k.catalog_id == catalog && k.bundle.eq_ignore_ascii_case(bundle)) {
                Some(i) => plan.kexts.remove(i),
                None => kext(catalog, bundle, reason),
            }
        })
        .collect();
    plan.kexts.splice(0..0, front);
}

// ── Sensors, battery, laptop helpers ────────────────────────────────────────

fn is_amd_zen(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::AmdZen | CpuPlatform::AmdZen2 | CpuPlatform::AmdZen3 | CpuPlatform::AmdZen4 | CpuPlatform::AmdZen5
    )
}

fn is_dell(ctx: &PlanContext) -> bool {
    let v = ctx.profile.motherboard_vendor.to_ascii_lowercase();
    v.contains("dell") || v.contains("alienware")
}

/// The internal panel's backlight runs through a supported iGPU, which is
/// when the acpi stage adds SSDT-PNLF and, from 10.15 on, SSDT-ALS0
/// (Dortania backlight.md; OpenCorePkg SSDT-ALS0.dsl: "Starting with macOS
/// 10.15 Ambient Light Sensor presence is required for backlight
/// functioning"). Mirrors that stage's rule, which runs later.
fn panel_has_light_sensor(ctx: &PlanContext, display: &DisplayPlan) -> bool {
    use CpuPlatform as P;
    use GpuFamily as F;
    if !ctx.has_panel || ctx.is_vm || ctx.target < MacOsVersion::Catalina {
        return false;
    }
    let gpus = &ctx.profile.gpus;
    match gpus.iter().position(|g| g.is_igpu) {
        Some(i) => {
            let igpu = &gpus[i];
            let backlight_family = matches!(
                igpu.family,
                F::IntelIronLake
                    | F::IntelSandyBridge
                    | F::IntelIvyBridge
                    | F::IntelHaswell
                    | F::IntelBroadwell
                    | F::IntelSkylake
                    | F::IntelKabyLake
                    | F::IntelCoffeeLake
                    | F::IntelCometLake
                    | F::IntelIceLake
                    | F::AmdApuVega
            );
            backlight_family
                && !igpu.disabled
                && !display.disabled.contains(&i)
                && gpu_db::support(igpu).display_capable
        }
        // Manual profile without GPUs: go by the CPU generation.
        None => {
            gpus.is_empty()
                && ctx.is_intel()
                && matches!(
                    ctx.platform(),
                    P::Arrandale
                        | P::SandyBridge
                        | P::IvyBridge
                        | P::Haswell
                        | P::Broadwell
                        | P::Skylake
                        | P::KabyLake
                        | P::CoffeeLake
                        | P::CometLake
                        | P::IceLake
                )
        }
    }
}

fn sensors(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    if ctx.is_vm {
        return;
    }
    // Dortania ktext.md: SMCProcessor is Intel only, SMCSuperIO reads desktop
    // Super I/O fan chips (laptop fans sit behind the EC), neither on AMD.
    if ctx.is_intel() {
        push(plan, kext("VirtualSMC", "SMCProcessor.kext", "Intel CPU temperature and power readings.").optional());
        if !ctx.is_laptop {
            push(plan, kext("VirtualSMC", "SMCSuperIO.kext", "Fan speeds from the board's Super I/O chip.").optional());
        }
    }
    if ctx.is_amd() && is_amd_zen(ctx.platform()) {
        // research-amd §7.1: lowers single-core boost and causes random
        // panics on some Zen 4/5 systems, so it ships disabled there.
        let newer = matches!(ctx.platform(), CpuPlatform::AmdZen4 | CpuPlatform::AmdZen5);
        let pm = kext(
            "AMDRyzenCPUPowerManagement",
            "AMDRyzenCPUPowerManagement.kext",
            if newer {
                "AMD CPU power management (disabled: it lowers single-core boost on Zen 4/5; enable it to try \
                 AMD Power Gadget)."
            } else {
                "AMD Zen CPU power management and frequency reporting (AMD Power Gadget)."
            },
        )
        .optional();
        let smc = kext(
            "SMCAMDProcessor",
            "SMCAMDProcessor.kext",
            "AMD CPU temperature through VirtualSMC; needs AMDRyzenCPUPowerManagement.",
        )
        .optional();
        if newer {
            push(plan, pm.disabled());
            push(plan, smc.disabled());
        } else {
            push(plan, pm);
            push(plan, smc);
        }
        if ctx.target == MacOsVersion::Tahoe {
            note(
                plan,
                NoteLevel::Info,
                "cpu",
                "AMD power management on macOS 26",
                "AMDRyzenCPUPowerManagement has no release since 2024; on macOS 26 some users report that AMD \
                 Power Gadget finds no power management. Disable it and SMCAMDProcessor if the system misbehaves.",
            );
        }
    }
    if ctx.is_laptop {
        push(plan, kext("VirtualSMC", "SMCBatteryManager.kext", "Battery status and charging on laptops."));
        push(
            plan,
            kext(
                "ECEnabler",
                "ECEnabler.kext",
                "Reads battery EC fields wider than 8 bits, so battery status works without DSDT patches.",
            )
            .optional(),
        );
        push(
            plan,
            kext("BrightnessKeys", "BrightnessKeys.kext", "Fn brightness keys without DSDT patches.").optional(),
        );
    }
    // Dortania ktext.md: SMCLightSensor only where an ambient light sensor
    // exists ("can cause issues otherwise"): the real one or SSDT-ALS0.
    if panel_has_light_sensor(ctx, display) {
        push(
            plan,
            kext(
                "VirtualSMC",
                "SMCLightSensor.kext",
                "Ambient light sensor (the real one or SSDT-ALS0's), which macOS 10.15+ needs for backlight control.",
            )
            .optional(),
        );
    }
    if is_dell(ctx) {
        push(
            plan,
            kext("VirtualSMC", "SMCDellSensors.kext", "Fan readings and control through Dell System Management Mode.")
                .optional(),
        );
    }
}

// ── Audio ───────────────────────────────────────────────────────────────────

/// PCI subsystem vendor of the board/laptop maker: codec_db prefers layouts
/// written for that vendor. Only the vendor half is known (the scan has no
/// codec subsystem id), so exact-subsystem matches never trigger.
fn oem_subsystem(vendor: &str) -> Option<u32> {
    const OEMS: &[(&str, u16)] = &[
        ("dell", 0x1028),
        ("alienware", 0x1028),
        ("hewlett", 0x103C),
        ("hp", 0x103C),
        ("lenovo", 0x17AA),
        ("asus", 0x1043),
        ("acer", 0x1025),
        ("micro-star", 0x1462),
        ("msi", 0x1462),
        ("gigabyte", 0x1458),
        ("asrock", 0x1849),
        ("samsung", 0x144D),
        ("toshiba", 0x1179),
        ("dynabook", 0x1179),
        ("clevo", 0x1558),
        ("razer", 0x1A58),
        ("microsoft", 0x1414),
        ("huawei", 0x19E5),
        ("xiaomi", 0x1D72),
        ("fujitsu", 0x10CF),
        ("sony", 0x104D),
        ("medion", 0x17C0),
    ];
    let lower = vendor.to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    OEMS.iter()
        .find(|(token, _)| if token.len() <= 3 { words.contains(token) } else { lower.contains(token) })
        .map(|(_, id)| u32::from(*id) << 16)
}

/// The chipset. A desktop chipset guessed from a laptop's model name is
/// ignored (ASUS "X570ZD" is a laptop); one the profile names is trusted.
fn chipset<'a>(ctx: &'a PlanContext) -> Option<&'a ChipsetInfo> {
    ctx.chipset.as_ref().filter(|c| ctx.profile.chipset.is_some() || !ctx.is_laptop || c.is_mobile)
}

/// Intel PCH series (100 = Sunrise Point and newer) from the chipset, or
/// from the CPU platform when the chipset is unknown.
fn intel_pch_series(ctx: &PlanContext) -> Option<u32> {
    if let Some(c) = chipset(ctx).filter(|c| c.vendor == CpuVendor::Intel) {
        return Some(c.series);
    }
    use CpuPlatform as P;
    Some(match ctx.platform() {
        P::Penryn | P::Lynnfield | P::Arrandale | P::NehalemHedt => 5,
        P::SandyBridge | P::SandyBridgeE | P::IvyBridgeE => 6,
        P::IvyBridge => 7,
        P::Haswell | P::Broadwell => 8,
        P::HaswellE | P::BroadwellE => 9,
        P::Skylake => 100,
        P::KabyLake | P::SkylakeX | P::CascadeLakeX => 200,
        P::CoffeeLake => 300,
        P::CometLake | P::IceLake => 400,
        P::RocketLake | P::TigerLake => 500,
        P::AlderLake => 600,
        P::RaptorLake => 700,
        P::ArrowLake => 800,
        _ => return None,
    })
}

/// Default HDA controller path: 00:1f.3 on Intel 100-series and newer PCHs,
/// 00:1b.0 before (Dortania DeviceProperties pages). AMD has no fixed path.
fn default_hda_path(ctx: &PlanContext) -> Option<&'static str> {
    if !ctx.is_intel() || ctx.is_vm {
        return None;
    }
    intel_pch_series(ctx).map(|s| if s >= 100 { "PciRoot(0x0)/Pci(0x1f,0x3)" } else { "PciRoot(0x0)/Pci(0x1b,0x0)" })
}

fn audio(ctx: &PlanContext, plan: &mut BuildPlan) {
    let Some(audio) = ctx.profile.audio.as_ref() else {
        if !ctx.is_vm {
            note(
                plan,
                NoteLevel::Info,
                "audio",
                "No audio codec detected",
                "AppleALC was not added. Add the codec in the hardware editor to get onboard audio.",
            );
        }
        return;
    };
    let codec = audio.codec_id.or_else(|| codec_db::find_codec_by_name(&audio.codec_name));
    let Some(codec) = codec else {
        note(
            plan,
            if ctx.is_vm { NoteLevel::Info } else { NoteLevel::Warning },
            "audio",
            "Unknown audio codec",
            format!(
                "The codec \"{}\" could not be identified, so AppleALC was not added. Pick the codec in the hardware \
                 editor, or use a USB audio adapter.",
                audio.codec_name.trim()
            ),
        );
        return;
    };
    let name = codec_db::codec_name(codec);
    if codec_db::is_hdmi_codec_id(codec) {
        note(
            plan,
            NoteLevel::Info,
            "audio",
            "Only an HDMI/DisplayPort codec",
            format!("{name} is a digital codec served by the GPU driver; AppleALC is not needed for it."),
        );
        return;
    }
    if !codec_db::is_supported(codec) {
        note(
            plan,
            NoteLevel::Warning,
            "audio",
            format!("{name} is not supported by AppleALC"),
            "Onboard analog audio will not work with AppleHDA. After installing, VoodooHDA (installed to \
             /Library/Extensions) or a USB audio adapter are the options.",
        );
        return;
    }
    let subsystem = oem_subsystem(&ctx.profile.motherboard_vendor);
    let Some(layout) = audio.layout_id.or_else(|| codec_db::default_layout(codec, subsystem, ctx.is_laptop)) else {
        note(
            plan,
            NoteLevel::Warning,
            "audio",
            "No AppleALC layout",
            format!("AppleALC has no layout for {name}; audio was not configured."),
        );
        return;
    };
    let source = if audio.layout_id.is_some() { "chosen in the hardware editor" } else { "default for this codec" };
    push(plan, kext("AppleALC", "AppleALC.kext", format!("Onboard audio for {name} (layout-id {layout}, {source}).")));

    let path =
        audio.controller_pci_path.as_deref().map(str::trim).filter(|p| !p.is_empty()).or_else(|| default_hda_path(ctx));
    match path {
        Some(path) => property(
            plan,
            path,
            "layout-id",
            PlistScalar::data(&layout.to_le_bytes()),
            &format!("AppleALC layout {layout} for {name}"),
        ),
        // No known controller path (AMD boards, VMs): the boot-arg works on any path.
        None => boot_arg(plan, &format!("alcid={layout}")),
    }
    let others: Vec<String> = codec_db::ranked_layouts(codec, subsystem, ctx.is_laptop)
        .into_iter()
        .filter(|l| *l != layout)
        .take(6)
        .map(|l| l.to_string())
        .collect();
    if audio.layout_id.is_none() && !others.is_empty() {
        post_install(
            plan,
            "audio",
            "Try other layout-ids if a jack or the microphone is silent",
            format!(
                "Layout {layout} was picked for {name}. Other candidates, best first: {}. Test one with the boot-arg \
                 alcid=<id>, then set it as the layout-id in the hardware editor.",
                others.join(", ")
            ),
        );
    }

    if ctx.target == MacOsVersion::Tahoe {
        // research-kexts §4.1: AppleHDA.kext was removed in 26.0 beta 2;
        // AppleALC stays because the restored AppleHDA needs it.
        note(
            plan,
            NoteLevel::Warning,
            "audio",
            "No analog audio on macOS 26 out of the box",
            "macOS 26 removed AppleHDA, so speakers, headphone jack and microphone stay silent until AppleHDA is \
             restored. AppleALC is kept in the EFI for that. HDMI/DisplayPort and USB audio are unaffected.",
        );
        post_install(
            plan,
            "audio",
            "Restore AppleHDA on macOS 26",
            "Restore AppleHDA from macOS 15 with a root-patch tool (OpenCore Legacy Patcher's modern audio patch or \
             a community AppleHDA installer). It needs SIP partly disabled (csr-active-config 03080000), \
             SecureBootModel Disabled and must be redone after every macOS update. Alternative: VoodooHDA \
             installed to /Library/Extensions (then disable AppleALC).",
        );
    }
}

// ── CPU helpers ─────────────────────────────────────────────────────────────

/// Intel platforms that run with a CPUID spoof, so the CPU name macOS shows
/// is wrong without RestrictEvents.
fn intel_cpuid_spoofed(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::RocketLake
            | CpuPlatform::TigerLake
            | CpuPlatform::AlderLake
            | CpuPlatform::RaptorLake
            | CpuPlatform::ArrowLake
            | CpuPlatform::HaswellE
            | CpuPlatform::BroadwellE
    )
}

fn is_intel_hybrid(ctx: &PlanContext) -> bool {
    ctx.is_intel()
        && ctx.profile.cpu.is_hybrid
        && matches!(ctx.platform(), CpuPlatform::AlderLake | CpuPlatform::RaptorLake | CpuPlatform::ArrowLake)
}

const MCE_BOARD_MODELS: &[&str] = &["MacPro6,1", "MacPro7,1", "iMacPro1,1"];

fn cpu_helpers(ctx: &PlanContext, plan: &mut BuildPlan) {
    let target = ctx.target;
    if ctx.needs_cryptexfixup {
        let caveat = cpu_db::ceiling_workaround(ctx.platform())
            .filter(|w| w.kext == Some("CryptexFixup.kext"))
            .map(|w| w.caveat)
            .unwrap_or("No AVX2: macOS 13+ installs only with CryptexFixup. Delta updates are unavailable.");
        push(
            plan,
            kext(
                "CryptexFixup",
                "CryptexFixup.kext",
                "Installs the non-AVX2 Rosetta cryptex: this CPU has no AVX2, which macOS 13+ expects.",
            )
            .min(DARWIN_13),
        );
        note(plan, NoteLevel::Warning, "cpu", "CPU without AVX2", caveat);
    }
    if ctx.platform() == CpuPlatform::Penryn && target >= MacOsVersion::Mojave {
        note(
            plan,
            NoteLevel::Warning,
            "cpu",
            "telemetrap.kext needed",
            "Penryn CPUs lack SSE4.2: macOS 10.14 and newer need telemetrap.kext (and the MacPro6,1 SMBIOS), \
             which is not part of the download catalog. Add it to EFI/OC/Kexts manually.",
        );
    }
    if is_intel_hybrid(ctx) {
        push(
            plan,
            kext(
                "CpuTopologyRebuild",
                "CpuTopologyRebuild.kext",
                "Rebuilds the P-core/E-core topology so the scheduler uses hybrid cores well.",
            )
            .optional(),
        );
    }
    if !ctx.is_vm {
        if ctx.is_intel() && ctx.cpu.hedt {
            // research-kexts §3.5: CpuTscSync is Intel-only, mostly needed on HEDT/server boards.
            push(
                plan,
                kext(
                    "CpuTscSync",
                    "CpuTscSync.kext",
                    "Keeps the TSC in sync across cores; HEDT and server boards often leave it unsynced.",
                )
                .optional(),
            );
        } else if ctx.is_amd() && ctx.is_laptop && is_amd_zen(ctx.platform()) {
            // research-amd §7.1 / §13: ForgedInvariant for AMD laptops.
            push(
                plan,
                kext(
                    "ForgedInvariant",
                    "ForgedInvariant.kext",
                    "Synchronises the TSC on AMD laptops, whose firmware often leaves it unsynced.",
                )
                .optional(),
            );
        }
    }

    // AppleMCEReporterDisabler only matches the MacPro6,1 / iMacPro1,1 /
    // MacPro7,1 board-ids (research-kexts §3.5).
    let mce_model = MCE_BOARD_MODELS.iter().any(|m| m.eq_ignore_ascii_case(&plan.smbios.model));
    if mce_model {
        let reason = if ctx.is_amd() && target >= MacOsVersion::Monterey {
            Some("Stops AppleIntelMCEReporter panics on AMD CPUs (macOS 12.3+).")
        } else if ctx.is_intel() && ctx.platform() == CpuPlatform::ArrowLake {
            Some("Arrow Lake boots reliably only with AppleIntelMCEReporter disabled.")
        } else if is_intel_hybrid(ctx) && target == MacOsVersion::Tahoe {
            Some("Hybrid Intel CPUs are reported to need AppleIntelMCEReporter disabled on macOS 26.")
        } else if ctx.is_intel() && ctx.cpu.hedt && target >= MacOsVersion::Catalina {
            Some("Stops AppleIntelMCEReporter panics on dual-socket boards (macOS 10.15+); harmless on one socket.")
        } else {
            None
        };
        if let Some(reason) = reason {
            push(plan, kext("AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext", reason).min(DARWIN_10_15));
        }
    }
}

// ── RestrictEvents ──────────────────────────────────────────────────────────

/// RestrictEvents and its `revpatch=` list. Setting `revpatch` replaces the
/// default `auto` (memtab,pci,cpuname), so every needed value is listed
/// (RestrictEvents README: `memtab` is the memory tab of MacBookAir /
/// MacBookPro10,x, `pci` the PCI and RAM views of MacPro7,1).
fn restrict_events(ctx: &PlanContext, plan: &mut BuildPlan) {
    let target = ctx.target;
    let model = plan.smbios.model.trim().to_string();
    let mut values: Vec<&str> = Vec::new();
    let mut why: Vec<&str> = Vec::new();
    if model.eq_ignore_ascii_case("MacPro7,1") {
        values.push("pci");
        why.push("PCI and memory warnings of the MacPro7,1 SMBIOS");
    }
    let spoofed_intel = ctx.is_intel() && intel_cpuid_spoofed(ctx.platform());
    if ctx.is_amd() || spoofed_intel {
        values.push("cpuname");
        why.push("real CPU name in About This Mac");
    }
    // Dortania tahoe.md / security.md: OTA updates on 14.4+ need
    // revpatch=sbvmm with SecureBootModel Disabled; also for board-id skips.
    // Dortania monterey.md: from 12 on a T2 SMBIOS without Apple Secure Boot
    // gets no updates either, which sbvmm (11.3+) works around.
    let secure_boot_off = plan.smbios.secure_boot_model.trim().eq_ignore_ascii_case("Disabled");
    if target >= MacOsVersion::Sonoma
        || plan.smbios.board_id_skip
        || (secure_boot_off && target >= MacOsVersion::Monterey)
    {
        values.push("sbvmm");
        why.push("OTA updates (VMM Secure Boot model)");
    }
    // RestrictEvents README: f16c fixes CoreGraphics crashes on Ivy Bridge (13.3+).
    if matches!(ctx.platform(), CpuPlatform::IvyBridge | CpuPlatform::IvyBridgeE) && target >= MacOsVersion::Ventura {
        values.push("f16c");
        why.push("CoreGraphics crash fix for Ivy Bridge");
    }
    // An earlier stage may already rely on RestrictEvents (revblock=media,
    // revpatch=asset, ...).
    let earlier = plan.boot_args.iter().any(|a| {
        let key = a.split('=').next().unwrap_or_default();
        matches!(key, "revpatch" | "revblock" | "revcpu" | "revcpuname")
    });
    if values.is_empty() && !earlier {
        return;
    }
    let replaces_auto = !values.is_empty() || plan.boot_args.iter().any(|a| a.starts_with("revpatch="));
    if replaces_auto && (model.starts_with("MacBookAir") || model.starts_with("MacBookPro10,")) {
        // Keep what `auto` would have done for these models.
        values.push("memtab");
    }
    let reason = if why.is_empty() {
        "Needed by the RestrictEvents boot-args of this build.".to_string()
    } else {
        format!("RestrictEvents: {}.", why.join(", "))
    };
    push(plan, kext("RestrictEvents", "RestrictEvents.kext", reason).optional());
    merge_list_arg(plan, "revpatch", &values);
    if spoofed_intel {
        // revcpu defaults to off on Intel CPUs.
        boot_arg(plan, "revcpu=1");
    }
}

// ── Root patches (AMFIPass) ─────────────────────────────────────────────────

/// The display GPU only has drivers on the target through an OCLP root patch:
/// past its last native release but within root-patch reach, or a
/// Polaris/Vega card on a CPU without AVX2 from macOS 13 on (Dortania
/// ventura.md: their userspace needs AVX2; OCLP restores them). Same rule as
/// the graphics stage's root-patch check.
fn gpu_root_patch(ctx: &PlanContext, display: &DisplayPlan) -> Option<String> {
    let gpu = ctx.profile.gpus.get(display.primary?)?;
    if gpu.disabled {
        return None;
    }
    let s = gpu_db::support(gpu);
    let target = ctx.target;
    let reached = s.min_native.is_some_and(|min| target >= min);
    let beyond_native = s.max_native.is_some_and(|max| target > max);
    let patchable = s.max_with_root_patch.is_some_and(|max| target <= max);
    let without_avx2 = ctx.needs_cryptexfixup
        && gpu_db::natively_supported_on(gpu, target)
        && matches!(gpu.family, GpuFamily::AmdPolaris | GpuFamily::AmdLexa | GpuFamily::AmdVega10 | GpuFamily::AmdVega20);
    let root_patch = s.display_capable && ((reached && beyond_native && patchable) || without_avx2);
    root_patch.then(|| {
        let name = gpu.name.trim();
        if name.is_empty() {
            gpu_db::family_label(gpu.family).to_string()
        } else {
            name.to_string()
        }
    })
}

fn root_patch_support(ctx: &PlanContext, plan: &mut BuildPlan, st: &State) {
    if st.root_patch.is_empty() {
        return;
    }
    push(
        plan,
        kext(
            AMFIPASS_ID,
            "AMFIPass.kext",
            format!("Keeps AMFI enabled after root patching ({}).", st.root_patch.join("; ")),
        )
        .min(DARWIN_11),
    );
    // research-kexts §1.3: AMFIPass needs its beta flag on Darwin 25.
    if ctx.target >= MacOsVersion::Tahoe {
        boot_arg(plan, "-amfipassbeta");
    }
}

// ── Ethernet ────────────────────────────────────────────────────────────────

/// Outcome of the driver decision for one wired NIC.
struct NicChoice {
    kext: Option<KextSelection>,
    /// `device-id` to inject on the NIC path.
    spoof: Option<[u8; 4]>,
    boot_args: Vec<&'static str>,
    patch: Option<BinaryPatch>,
    works: bool,
    notes: Vec<String>,
}

impl NicChoice {
    fn native(works: bool) -> Self {
        Self { kext: None, spoof: None, boot_args: Vec::new(), patch: None, works, notes: Vec::new() }
    }

    fn with(kext: KextSelection) -> Self {
        Self { kext: Some(kext), ..Self::native(true) }
    }

    fn unsupported(note: impl Into<String>) -> Self {
        Self { notes: vec![note.into()], ..Self::native(false) }
    }
}

/// Dortania Comet Lake page: on 10.15-11.3 AppleIntelI210Ethernet needs its
/// mac-type check patched for the I225-V (device-id spoofed to I225-LM).
fn i225v_patch() -> BinaryPatch {
    BinaryPatch {
        comment: "I225-V patch".into(),
        arch: "x86_64".into(),
        identifier: "com.apple.driver.AppleIntelI210Ethernet".into(),
        base: "__Z18e1000_set_mac_typeP8e1000_hw".into(),
        find: "F2150000".into(),
        mask: String::new(),
        replace: "F3150000".into(),
        replace_mask: String::new(),
        count: 1,
        limit: 0,
        skip: 0,
        min_kernel: DARWIN_10_15.into(),
        max_kernel: "20.4.0".into(),
        enabled: true,
    }
}

const I210: u16 = 0x1533;
const I210_SPOOF: [u8; 4] = [0x33, 0x15, 0x00, 0x00];

fn choose_nic(ctx: &PlanContext, nic: &ProfileNic, info: &EthernetInfo) -> NicChoice {
    let target = ctx.target;
    let chip = info.chip.as_str();
    if let Some(min) = info.min_macos {
        if target < min {
            return NicChoice::unsupported(format!(
                "{chip} has no driver before {}; it will not work on {}.",
                min.display_name(),
                target.display_name()
            ));
        }
    }
    let device = nic_device(nic);
    match &info.driver {
        EthernetDriver::IntelMausi => {
            let pch = intel_pch_series(ctx).unwrap_or(0);
            // Dortania ktext: Mieze's fork for VT-d users and 500-series+ boards;
            // it is built for 10.15 and newer.
            let mieze_only = info.preferred_kext == Some("IntelMausiEthernet");
            let mieze = mieze_only || (ctx.is_intel() && pch >= 500 && target >= MacOsVersion::Catalina);
            if mieze_only && target < MacOsVersion::Catalina {
                return NicChoice::unsupported(format!(
                    "{chip}: only Mieze's IntelMausiEthernet drives this revision, and its build needs macOS 10.15 \
                     or newer."
                ));
            }
            if mieze {
                NicChoice::with(
                    kext(
                        "IntelMausiEthernet",
                        "IntelMausiEthernet.kext",
                        format!("{chip} Ethernet (Mieze's IntelMausiEthernet, AppleVTD-capable)."),
                    )
                    .min(DARWIN_10_15),
                )
            } else {
                NicChoice::with(kext("IntelMausi", "IntelMausi.kext", format!("{chip} Ethernet.")))
            }
        }
        EthernetDriver::IntelI211 => {
            // Dortania ktext.md: AppleIGB "Requires macOS 12 and above";
            // SmallTreeIntel82576 1.3.0 "macOS 10.15+" (1.2.5 covers 10.13-10.14).
            if info.preferred_kext == Some("AppleIGB") {
                if target < MacOsVersion::Monterey {
                    return NicChoice::unsupported(format!(
                        "{chip}: only AppleIGB drives this controller, and AppleIGB needs macOS 12 or newer."
                    ));
                }
                NicChoice::with(
                    kext("AppleIGB", "AppleIGB.kext", format!("{chip} Ethernet (only AppleIGB drives it)."))
                        .min(DARWIN_12),
                )
            } else if target < MacOsVersion::Catalina {
                NicChoice::unsupported(format!(
                    "{chip}: the SmallTreeIntel82576 build in the catalog (1.3.0) needs macOS 10.15 or newer; on \
                     10.13-10.14 add SmallTreeIntel82576 1.2.5 manually or use another NIC."
                ))
            } else if target >= MacOsVersion::Monterey {
                NicChoice::with(
                    kext(
                        "AppleIGB",
                        "AppleIGB.kext",
                        format!("{chip} Ethernet; SmallTreeIntel82576 does not load on macOS 12+."),
                    )
                    .min(DARWIN_12),
                )
            } else {
                NicChoice::with(
                    kext(
                        "SmallTreeIntel82576",
                        "SmallTreeIntel82576.kext",
                        format!("{chip} Ethernet (macOS 10.15 to 11)."),
                    )
                    .min(DARWIN_10_15)
                    .max(MAX_11),
                )
            }
        }
        EthernetDriver::IntelI225 => {
            let native_family = matches!(device, Some(0x15F2 | 0x15F3)) && info.preferred_kext.is_none();
            if native_family && target <= MacOsVersion::Monterey {
                // Apple's AppleIntelI210Ethernet up to macOS 12 (Dortania Comet Lake).
                let mut c = NicChoice::native(true);
                c.spoof = info.device_id_spoof;
                if device == Some(0x15F3) && target <= MacOsVersion::BigSur {
                    c.patch = Some(i225v_patch());
                }
                if target == MacOsVersion::Monterey {
                    c.boot_args.push("e1000=0");
                    c.notes.push(format!(
                        "{chip}: e1000=0 makes macOS 12.3+ use the AppleIntelI210Ethernet kext instead of the \
                         DriverKit driver (on 12.2.1 and older the boot-arg is dk.e1000=0)."
                    ));
                }
                c
            } else {
                // macOS 13+ replaced the kext with a DriverKit driver that needs
                // VT-d; AppleIGC works without it (research-kexts §3.6).
                let mut c = NicChoice::with(
                    kext(
                        "AppleIGC",
                        "AppleIGC.kext",
                        format!("{chip} 2.5GbE (AppleIGC; Apple's driver needs VT-d from macOS 13)."),
                    )
                    .min(DARWIN_10_15),
                );
                if info.preferred_kext == Some("AppleIGC") {
                    c.spoof = info.device_id_spoof;
                }
                c.notes.push(format!("{chip}: AppleIGC only supports auto-negotiation."));
                if native_family {
                    c.notes.push(
                        "Alternative with VT-d enabled: Apple's AppleIntelI210Ethernet with a device-id spoof to \
                         I225-LM and e1000=0."
                            .into(),
                    );
                }
                c
            }
        }
        EthernetDriver::IntelLucy => {
            NicChoice::with(kext("IntelLucy", "IntelLucy.kext", format!("{chip} 10GbE (IntelLucy).")).min(DARWIN_10_15))
        }
        EthernetDriver::NativeIntel => {
            let i210_family = device == Some(I210) || info.device_id_spoof == Some(I210_SPOOF);
            let mut c = NicChoice::native(true);
            c.spoof = info.device_id_spoof;
            if i210_family && target >= MacOsVersion::Ventura {
                c.kext = Some(
                    kext(
                        "AppleIntelI210Ethernet",
                        "AppleIntelI210Ethernet.kext",
                        format!(
                            "{chip}: Apple's I210 kext, removed from macOS 13 (its DriverKit successor needs VT-d)."
                        ),
                    )
                    .min(DARWIN_13),
                );
            }
            if i210_family && target >= MacOsVersion::Monterey {
                c.boot_args.push("e1000=0");
            }
            c
        }
        EthernetDriver::AtherosE2200 => {
            NicChoice::with(kext("AtherosE2200Ethernet", "AtherosE2200Ethernet.kext", format!("{chip} Ethernet.")))
        }
        EthernetDriver::RealtekRtl8111 => {
            if target < MacOsVersion::Mojave {
                // Mieze's changelog: 2.3.0 and newer need macOS 10.14.
                return NicChoice::unsupported(format!(
                    "{chip}: the RealtekRTL8111 builds in the catalog need macOS 10.14 or newer; on 10.13 use \
                     RealtekRTL8111 2.2.2 (added manually) or another NIC."
                ));
            }
            // Mieze: AMD CPUs cannot use AppleVTD, keep 2.4.2 there.
            let (id, why) = if ctx.is_amd() {
                ("RealtekRTL8111-2.4.2", "2.4.2: the AppleVTD build is not for AMD systems")
            } else {
                ("RealtekRTL8111", "AppleVTD-capable 3.0.0")
            };
            let mut c = NicChoice::with(
                kext(id, "RealtekRTL8111.kext", format!("{chip} Ethernet (RealtekRTL8111 {why}).")).min(DARWIN_10_14),
            );
            c.spoof = info.device_id_spoof;
            c
        }
        EthernetDriver::RealtekRtl8125 => NicChoice::with(
            kext(
                "RTL812xLucy",
                "RTL812xLucy.kext",
                format!("{chip} Ethernet (RTL812xLucy, works with and without AppleVTD)."),
            )
            .min(DARWIN_10_15),
        ),
        EthernetDriver::RealtekRtl8100 => {
            NicChoice::with(kext("RealtekRTL8100", "RealtekRTL8100.kext", format!("{chip} Fast Ethernet.")))
        }
        EthernetDriver::NativeAquantia => {
            // research-amd §11: from macOS 12 the Aquantia driver relies on
            // AppleVTD, which AMD systems never have: it works there only
            // with CaseySJ's kernel patches, which are not part of the build.
            let amd_needs_patches = ctx.is_amd() && target >= MacOsVersion::Monterey;
            let mut c = NicChoice::native(!amd_needs_patches);
            c.notes.push(format!(
                "{chip} uses Apple's built-in Aquantia driver; the Kernel quirk ForceAquantiaEthernet must be on \
                 (set by the quirks stage)."
            ));
            if amd_needs_patches {
                c.notes.push(
                    "On AMD (no AppleVTD) macOS 12+ also needs CaseySJ's Aquantia kernel patches \
                     (CaseySJ/Aquantia-macOS-Patches), which are not added automatically; until they are added \
                     the port does not work."
                        .into(),
                );
            }
            c
        }
        EthernetDriver::NativeBroadcom => {
            let mut c = NicChoice::native(true);
            c.spoof = info.device_id_spoof;
            if target >= MacOsVersion::BigSur && info.device_id_spoof.is_some() {
                c.notes.push(format!(
                    "{chip}: if the link stays down on macOS 11+, CatalinaBCM5701Ethernet (not in the catalog) is \
                     the usual fix."
                ));
            }
            c
        }
        EthernetDriver::Unsupported => NicChoice::unsupported(String::new()),
    }
}

fn ethernet(ctx: &PlanContext, plan: &mut BuildPlan, st: &mut State) {
    for nic in &ctx.profile.ethernet {
        let info = device_db::ethernet_info(nic);
        let chip = info.chip.clone();
        if nic.bus == DeviceBus::Usb || info.driver == EthernetDriver::Unsupported {
            let level = if nic.bus == DeviceBus::Usb { NoteLevel::Info } else { NoteLevel::Warning };
            let detail = if info.notes.is_empty() {
                "No macOS driver is known for it.".to_string()
            } else {
                info.notes.join(" ")
            };
            note(plan, level, "ethernet", format!("{chip}: not configured"), detail);
            continue;
        }
        let mut choice = choose_nic(ctx, nic, &info);
        let path = nic_path(nic);
        if let Some(spoof) = choice.spoof {
            match path {
                Some(path) => {
                    property(plan, path, "device-id", PlistScalar::data(&spoof), &format!("{chip} device-id spoof"))
                }
                None => {
                    choice.works = false;
                    choice.notes.push(format!(
                        "{chip} needs a device-id spoof, but its PCI path is unknown; add the path in the hardware \
                         editor."
                    ));
                }
            }
        }
        if choice.works {
            if let Some(sel) = choice.kext.take() {
                // The first working NIC carries the installer download; a
                // failed fetch of its driver must stop the build.
                push(plan, sel.required_if(!st.ethernet_ok));
            }
            for arg in &choice.boot_args {
                boot_arg(plan, arg);
            }
            if let Some(patch) = choice.patch.take() {
                if !plan.kernel_patches.iter().any(|p| p.comment == patch.comment) {
                    plan.kernel_patches.push(patch);
                }
            }
            if let (false, Some(path)) = (st.builtin_set, path) {
                property(plan, path, "built-in", PlistScalar::data(&[0x01]), "primary network interface (iServices)");
                st.builtin_set = true;
            }
            st.ethernet_ok = true;
            if info.driver == EthernetDriver::RealtekRtl8125 && matches!(nic_device(nic), Some(0x8125 | 0x3000)) {
                // research-kexts §3.6: LucyRTL8125Ethernet is the RTL8125 fallback; only one may load.
                push(
                    plan,
                    kext(
                        "LucyRTL8125Ethernet",
                        "LucyRTL8125Ethernet.kext",
                        format!(
                            "Fallback driver for the {chip}: enable it and disable RTL812xLucy if the link misbehaves."
                        ),
                    )
                    .disabled()
                    .min(DARWIN_10_15),
                );
            }
        }
        let mut details: Vec<String> = info.notes.clone();
        details.extend(choice.notes.into_iter().filter(|n| !n.is_empty()));
        if !details.is_empty() {
            let level = if choice.works { NoteLevel::Info } else { NoteLevel::Warning };
            let title =
                if choice.works { format!("{chip} Ethernet") } else { format!("{chip}: not working with this build") };
            note(plan, level, "ethernet", title, details.join(" "));
        }
    }
}

// ── Wi-Fi ───────────────────────────────────────────────────────────────────

/// Device ids Apple's AirPortBrcmNIC / AirPortBrcm4360 list natively
/// (AirportBrcmFixup README matrix): no injector needed for them.
const BRCM_NIC_NATIVE: &[u16] = &[0x43A0, 0x43A3, 0x43BA];
const BRCM_4360_NATIVE: &[u16] = &[0x4331, 0x4353, 0x432B];

fn wifi(ctx: &PlanContext, plan: &mut BuildPlan, st: &mut State) {
    let Some(nic) = ctx.profile.wifi.as_ref() else {
        return;
    };
    let info = device_db::wifi_info(nic);
    match info.driver.clone() {
        WifiDriver::IntelItlwm => intel_wifi(ctx, plan, st, &info),
        WifiDriver::Broadcom { native_max, fixup } => broadcom_wifi(ctx, plan, st, nic, &info, native_max, fixup),
        WifiDriver::AtherosLegacy => {
            // AR9485/AR946x/AR9565/AR958x are not in AirPortAtheros40's list
            // even on 10.13; they need a patched copy (device_db notes).
            let patched_only = nic.vendor_id.as_deref().and_then(device_db::parse_id16) == Some(0x168C)
                && matches!(nic_device(nic), Some(0x0032 | 0x0033 | 0x0034 | 0x0036 | 0x0037));
            let native = !patched_only && info.native_max.is_some_and(|max| ctx.target <= max);
            if native {
                if let Some(spoof) = info.device_id_spoof {
                    match nic_path(nic) {
                        Some(path) => property(
                            plan,
                            path,
                            "device-id",
                            PlistScalar::data(&spoof),
                            &format!("{} device-id spoof", info.chip),
                        ),
                        None => note(
                            plan,
                            NoteLevel::Warning,
                            "wifi",
                            format!("{}: device-id spoof not applied", info.chip),
                            "The Wi-Fi card's PCI path is unknown; add it in the hardware editor.",
                        ),
                    }
                }
                st.wifi_ok = true;
                st.wifi_recovery_ok = true;
            }
            let title =
                if native { format!("{} Wi-Fi", info.chip) } else { format!("{}: Wi-Fi not configured", info.chip) };
            note(plan, if native { NoteLevel::Info } else { NoteLevel::Warning }, "wifi", title, info.notes.join(" "));
        }
        WifiDriver::RealtekRtw88 => {
            if info.min_macos.is_some_and(|min| ctx.target < min) {
                note(
                    plan,
                    NoteLevel::Warning,
                    "wifi",
                    format!("{}: no driver on this macOS", info.chip),
                    info.notes.join(" "),
                );
                return;
            }
            push(
                plan,
                kext("Feixiao", "rtw88.kext", format!("{} Wi-Fi (experimental rtw88 driver).", info.chip))
                    .optional()
                    .min(DARWIN_11),
            );
            st.wifi_ok = true;
            note(plan, NoteLevel::Info, "wifi", format!("{} Wi-Fi is experimental", info.chip), info.notes.join(" "));
        }
        WifiDriver::Unsupported => {
            note(plan, NoteLevel::Warning, "wifi", format!("{}: Wi-Fi not supported", info.chip), info.notes.join(" "));
        }
    }
}

fn intel_wifi(ctx: &PlanContext, plan: &mut BuildPlan, st: &mut State, info: &WifiInfo) {
    let target = ctx.target;
    let chip = info.chip.as_str();
    // AirportItlwm needs Apple Secure Boot (SecureBootModel != Disabled) to
    // load the native IO80211 stack (research-kexts §3.7).
    let secure_boot = !plan.smbios.secure_boot_model.eq_ignore_ascii_case("Disabled");
    let native_build = kext_catalog::airport_itlwm_id(target);
    let strategy = ctx.options.intel_wifi;
    if strategy == IntelWifiStrategy::None {
        note(
            plan,
            NoteLevel::Info,
            "wifi",
            format!("{chip}: Wi-Fi left out"),
            "Intel Wi-Fi was turned off in the build options.",
        );
        return;
    }
    let airport = match (strategy, native_build) {
        (IntelWifiStrategy::AirportItlwm | IntelWifiStrategy::Auto, Some(id)) if secure_boot => Some(id),
        (IntelWifiStrategy::AirportItlwm, Some(_)) => {
            note(
                plan,
                NoteLevel::Warning,
                "wifi",
                "AirportItlwm needs Apple Secure Boot",
                "SecureBootModel is Disabled for this build (needed for OTA updates on 14.4+ or root patches), and \
                 AirportItlwm cannot load without it. itlwm with the HeliPort app is used instead.",
            );
            None
        }
        _ => None,
    };
    if let Some(id) = airport {
        let mut sel = kext(
            id,
            "AirportItlwm.kext",
            format!("{chip} Wi-Fi with the native Wi-Fi menu (AirportItlwm build for this release)."),
        )
        .required_if(!st.ethernet_ok);
        if let Some((min, max)) = kext_catalog::airport_itlwm_kernel_range(id) {
            sel = sel.min(min).max(max);
        }
        push(plan, sel);
        st.wifi_ok = true;
        st.wifi_recovery_ok = true;
        return;
    }
    if strategy == IntelWifiStrategy::AirportItlwm && native_build.is_none() {
        legacy_wireless_intel(ctx, plan, st, chip);
        return;
    }
    push(
        plan,
        kext("itlwm", "itlwm.kext", format!("{chip} Wi-Fi as an Ethernet-like interface (itlwm + HeliPort app)."))
            .optional(),
    );
    st.wifi_ok = true;
    post_install(
        plan,
        "wifi",
        "Install HeliPort for Intel Wi-Fi",
        format!(
            "{chip} uses itlwm, which is managed by the HeliPort app instead of the Wi-Fi menu. Install HeliPort \
             after macOS is installed. itlwm does not work inside macOS Recovery and offers no AirDrop or Continuity."
        ),
    );
}

/// Ventura AirportItlwm on macOS 15/26 through the OCLP legacy wireless
/// stack (research-kexts §3.7 option 2): IOSkywalkFamily and
/// IO80211FamilyLegacy injected, the system IOSkywalkFamily excluded,
/// AMFIPass, plus a "Modern Wireless" root patch after install.
fn legacy_wireless_intel(ctx: &PlanContext, plan: &mut BuildPlan, st: &mut State, chip: &str) {
    let (min, max) = darwin_range(ctx.target);
    push(
        plan,
        kext(
            "IOSkywalkFamily",
            "IOSkywalkFamily.kext",
            "Older IOSkywalkFamily for the legacy wireless stack (AirportItlwm on macOS 15/26).",
        )
        .min(DARWIN_15),
    );
    push(
        plan,
        kext(
            "IO80211FamilyLegacy",
            "IO80211FamilyLegacy.kext",
            "Legacy IO80211 family for AirportItlwm on macOS 15/26.",
        )
        .min(DARWIN_15)
        .plugin("AirPortBrcmNIC.kext", false, Some(DARWIN_14), Some(MAX_15)),
    );
    push(
        plan,
        kext(
            "AirportItlwm-Ventura",
            "AirportItlwm.kext",
            format!("{chip} Wi-Fi: the macOS 13 AirportItlwm build on the legacy wireless stack."),
        )
        .optional()
        .min(&min)
        .max(&max),
    );
    block_skywalk(plan, DARWIN_15, "AirportItlwm legacy wireless stack");
    st.root_patch.push("legacy wireless stack for Intel Wi-Fi".into());
    st.wifi_ok = true;
    note(
        plan,
        NoteLevel::Warning,
        "wifi",
        "AirportItlwm on macOS 15/26 is a community method",
        "It needs a Modern Wireless root patch after installing (OCLP-Mod or Wi-Fi Patcher Pro), SIP partly \
         disabled, SecureBootModel Disabled and the Kernel quirk DisableIoMapper. Every macOS update downloads the \
         full installer. itlwm + HeliPort is the safer choice.",
    );
    post_install(
        plan,
        "wifi",
        "Apply the Modern Wireless root patch",
        format!(
            "{chip}: run a Modern Wireless root patch tool after the first boot, then reboot. Redo it after every \
             macOS update."
        ),
    );
}

fn block_skywalk(plan: &mut BuildPlan, min: &str, why: &str) {
    if plan.kernel_blocks.iter().any(|b| b.identifier == "com.apple.iokit.IOSkywalkFamily") {
        return;
    }
    plan.kernel_blocks.push(KernelBlock {
        comment: format!("Allow IOSkywalkFamily downgrade ({why})"),
        identifier: "com.apple.iokit.IOSkywalkFamily".into(),
        strategy: "Exclude".into(),
        min_kernel: min.into(),
        max_kernel: String::new(),
        enabled: true,
    });
}

fn broadcom_wifi(
    ctx: &PlanContext,
    plan: &mut BuildPlan,
    st: &mut State,
    nic: &ProfileNic,
    info: &WifiInfo,
    native_max: MacOsVersion,
    fixup: bool,
) {
    let target = ctx.target;
    let chip = info.chip.as_str();
    let device = nic_device(nic).unwrap_or_default();
    // AirPortBrcmNIC family (BCM4360/4350/43602/4352) vs AirPortBrcm4360/4331.
    let nic_family = native_max >= MacOsVersion::Ventura;
    let native = target <= native_max;
    if !native && !nic_family {
        note(
            plan,
            NoteLevel::Warning,
            "wifi",
            format!("{chip}: Wi-Fi not configured"),
            format!(
                "Apple dropped the driver for this card after {}. {}",
                native_max.display_name(),
                info.notes.join(" ")
            ),
        );
        return;
    }
    if let Some(path) = nic_path(nic) {
        if let Some(spoof) = info.device_id_spoof {
            property(plan, path, "device-id", PlistScalar::data(&spoof), &format!("{chip} device-id spoof"));
        }
        for (key, value) in &info.extra_properties {
            property(plan, path, key, PlistScalar::data(value), &format!("{chip} Wi-Fi"));
        }
    } else if info.device_id_spoof.is_some() || !info.extra_properties.is_empty() {
        note(
            plan,
            NoteLevel::Warning,
            "wifi",
            format!("{chip}: Wi-Fi properties not applied"),
            "The card needs DeviceProperties (device-id spoof or ASPM), but its PCI path is unknown; add it in the \
             hardware editor.",
        );
    }
    if fixup {
        // AirportBrcmFixup README: the injectors are only needed for ids the
        // Apple driver does not list; the 4360 one must stay off on 11+.
        let spoofed = info.device_id_spoof.is_some();
        let need_4360 = !nic_family && !spoofed && !BRCM_4360_NATIVE.contains(&device);
        let need_nic = nic_family && !spoofed && !BRCM_NIC_NATIVE.contains(&device);
        push(
            plan,
            kext(
                "AirportBrcmFixup",
                "AirportBrcmFixup.kext",
                format!("Patches Apple's Broadcom driver for the non-Apple {chip}."),
            )
            .optional()
            .plugin(
                "AirPortBrcm4360_Injector.kext",
                need_4360 && target <= MacOsVersion::Catalina,
                None,
                Some(MAX_10_15),
            )
            .plugin("AirPortBrcmNIC_Injector.kext", need_nic, None, None),
        );
    }
    st.wifi_ok = true;
    if native {
        st.wifi_recovery_ok = true;
        return;
    }
    // macOS 14+ removed AirPortBrcmNIC: OCLP Modern Wireless (research-kexts
    // §3.7 Broadcom, research-opencore-macos §3.5 item 5).
    push(
        plan,
        kext(
            "IOSkywalkFamily",
            "IOSkywalkFamily.kext",
            "Older IOSkywalkFamily for the legacy Broadcom wireless stack (macOS 14+).",
        )
        .min(DARWIN_14),
    );
    push(
        plan,
        kext(
            "IO80211FamilyLegacy",
            "IO80211FamilyLegacy.kext",
            format!("Legacy IO80211 family carrying the {chip} driver (macOS 14+)."),
        )
        .min(DARWIN_14)
        .plugin("AirPortBrcmNIC.kext", true, Some(DARWIN_14), Some(MAX_15)),
    );
    if target >= MacOsVersion::Tahoe {
        push(
            plan,
            kext(
                "AirPortBrcmNIC-Tahoe",
                "AirPortBrcmNIC-Tahoe.kext",
                format!("{chip} driver for macOS 26 on the legacy wireless stack."),
            )
            .min(DARWIN_26),
        );
    }
    block_skywalk(plan, DARWIN_14, "legacy Broadcom wireless stack");
    st.root_patch.push(format!("Modern Wireless patch for the {chip}"));
    note(
        plan,
        NoteLevel::Warning,
        "wifi",
        format!("{chip} needs a root patch on {}", target.display_name()),
        "Apple removed the driver in macOS 14. The legacy wireless kexts are injected, but Wi-Fi works only after \
         the OpenCore Legacy Patcher Modern Wireless root patch (OCLP 3.0+ for macOS 26), which needs SIP partly \
         disabled and SecureBootModel Disabled. Wi-Fi does not work inside macOS Recovery.",
    );
    post_install(
        plan,
        "wifi",
        "Apply the Modern Wireless root patch",
        format!(
            "{chip}: run OpenCore Legacy Patcher's root patch after the first boot, then reboot. Redo it after every \
             macOS update."
        ),
    );
}

// ── Bluetooth ───────────────────────────────────────────────────────────────

/// The Intel Bluetooth build in use: the maintained fork's v2.5.1 tag caps
/// IntelBTPatcher at `KernelVersion::Tahoe` (IntelBTPatcher.cpp), so macOS 26
/// needs no flag; the upstream 2.4.0 release stops at Sequoia and needs
/// `-ibtcompatbeta` there (research-kexts §1.3).
const INTEL_BT_CATALOG: &str = "IntelBluetoothFirmware";

fn intel_bt_needs_beta_flag(catalog: &str, target: MacOsVersion) -> bool {
    catalog == "IntelBluetoothFirmware-2.4.0" && target >= MacOsVersion::Tahoe
}

fn bluetoolfixup(plan: &mut BuildPlan, reason: &str) {
    push(plan, kext("BrcmPatchRAM", "BlueToolFixup.kext", reason.to_string()).optional().min(DARWIN_12));
    // BrcmPatchRAM README (2.7.0+): set these through the bootloader.
    let vars = [("bluetoothExternalDongleFailed", vec![0u8]), ("bluetoothInternalControllerInfo", vec![0u8; 14])];
    for (key, value) in vars {
        if !plan.nvram_add.iter().any(|v| v.guid == APPLE_NVRAM_GUID && v.key == key) {
            plan.nvram_add.push(NvramVariable {
                guid: APPLE_NVRAM_GUID.into(),
                key: key.into(),
                value: PlistScalar::data(&value),
            });
        }
        if !plan.nvram_delete.iter().any(|v| v.guid == APPLE_NVRAM_GUID && v.key == key) {
            plan.nvram_delete.push(NvramVariable {
                guid: APPLE_NVRAM_GUID.into(),
                key: key.into(),
                value: PlistScalar::data(&value),
            });
        }
    }
}

fn bluetooth(ctx: &PlanContext, plan: &mut BuildPlan) {
    let Some(nic) = ctx.profile.bluetooth.as_ref() else {
        return;
    };
    let target = ctx.target;
    let info = device_db::bluetooth_info(nic);
    let chip = info.chip.clone();
    let modern = target >= MacOsVersion::Monterey;
    match info.driver {
        BluetoothDriver::IntelBluetooth => {
            // IntelBluetoothFirmware FAQ: 12+ = Firmware + IntelBTPatcher +
            // BlueToolFixup, no injector; 11 and older use the injector.
            let catalog = INTEL_BT_CATALOG;
            push(
                plan,
                kext(catalog, "IntelBluetoothFirmware.kext", format!("Uploads firmware to the {chip}.")).optional(),
            );
            push(
                plan,
                kext(catalog, "IntelBTPatcher.kext", format!("Fixes the {chip} bring-up in the Bluetooth stack."))
                    .optional(),
            );
            if modern {
                bluetoolfixup(plan, "Lets macOS 12+ use a non-Apple Bluetooth controller.");
            } else {
                push(
                    plan,
                    kext(
                        catalog,
                        "IntelBluetoothInjector.kext",
                        format!("Enables the Bluetooth switch for the {chip} (macOS 11 and older)."),
                    )
                    .optional()
                    .max(MAX_11),
                );
            }
            if intel_bt_needs_beta_flag(catalog, target) {
                boot_arg(plan, "-ibtcompatbeta");
            }
            if info.notes.iter().any(|n| n.contains("Wi-Fi 7")) {
                note(plan, NoteLevel::Info, "bluetooth", chip.clone(), info.notes.join(" "));
            }
        }
        BluetoothDriver::BroadcomPatchRam => {
            // BrcmPatchRAM README / Dortania: Injector -> FirmwareData -> PatchRAM3.
            if target >= MacOsVersion::Catalina && !modern {
                push(
                    plan,
                    kext(
                        "BrcmPatchRAM",
                        "BrcmBluetoothInjector.kext",
                        format!("Native driver match for the {chip} (macOS 10.15-11)."),
                    )
                    .optional()
                    .max(MAX_11),
                );
            }
            push(
                plan,
                kext("BrcmPatchRAM", "BrcmFirmwareData.kext", format!("Firmware store for the {chip}.")).optional(),
            );
            if target >= MacOsVersion::Catalina {
                push(
                    plan,
                    kext("BrcmPatchRAM", "BrcmPatchRAM3.kext", format!("Uploads firmware to the {chip}."))
                        .optional()
                        .min(DARWIN_10_15),
                );
            } else {
                push(
                    plan,
                    kext(
                        "BrcmPatchRAM",
                        "BrcmPatchRAM2.kext",
                        format!("Uploads firmware to the {chip} (macOS 10.14 and older)."),
                    )
                    .optional()
                    .max(MAX_10_14),
                );
            }
            if modern {
                bluetoolfixup(plan, "Lets macOS 12+ use a non-Apple Bluetooth controller.");
            }
        }
        BluetoothDriver::BroadcomNative => {
            if modern && info.needs_bluetoolfixup {
                bluetoolfixup(plan, "Lets macOS 12+ use a non-Apple Bluetooth controller.");
            } else if !modern && info.needs_injector {
                push(
                    plan,
                    kext(
                        "BrcmPatchRAM",
                        "BrcmBluetoothInjector.kext",
                        format!("Native driver match for the {chip} (macOS 11 and older)."),
                    )
                    .optional()
                    .max(MAX_11),
                );
            }
        }
        BluetoothDriver::Realtek => {
            push(
                plan,
                kext(
                    "RealtekBluetoothFirmware",
                    "RealtekBluetoothFirmware.kext",
                    format!("Uploads firmware to the {chip} (experimental)."),
                )
                .optional(),
            );
            if modern {
                bluetoolfixup(plan, "Lets macOS 12+ use a non-Apple Bluetooth controller.");
            }
            note(plan, NoteLevel::Info, "bluetooth", format!("{chip} is experimental"), info.notes.join(" "));
        }
        BluetoothDriver::Unsupported => {
            note(plan, NoteLevel::Info, "bluetooth", format!("{chip}: Bluetooth not supported"), info.notes.join(" "));
        }
    }
}

// ── USB ─────────────────────────────────────────────────────────────────────

fn usb(ctx: &PlanContext, plan: &mut BuildPlan) {
    if ctx.is_vm {
        return;
    }
    // research-kexts §3.10: install with USBToolBox + UTBDefault, map later.
    push(plan, kext("USBToolBox", "USBToolBox.kext", "USB port mapping driver (USBToolBox)."));
    push(
        plan,
        kext(
            "USBToolBox",
            "UTBDefault.kext",
            "Enables every USB port until a USB map is made; replace it with UTBMap.kext.",
        ),
    );
    post_install(
        plan,
        "usb",
        "Map the USB ports",
        "Run the USBToolBox tool (Windows recommended), keep at most 15 ports per controller, build UTBMap.kext, \
         put it in EFI/OC/Kexts and replace UTBDefault.kext with it in config.plist. Without a map some ports may \
         be missing and sleep can misbehave.",
    );
    if let Some(chipset) = chipset(ctx) {
        if chipset.needs_xhci_unsupported_on(ctx.target, Some(&ctx.profile.motherboard_vendor)) {
            push(
                plan,
                kext(
                    "XHCI-unsupported",
                    "XHCI-unsupported.kext",
                    format!("The {} USB 3 controller is not matched by Apple's XHCI driver.", chipset.name),
                ),
            );
        }
    }
    let vega_apu = ctx.profile.gpus.iter().any(|g| g.family == GpuFamily::AmdApuVega);
    if ctx.is_amd() && ctx.is_laptop && vega_apu && is_amd_zen(ctx.platform()) && ctx.target >= MacOsVersion::BigSur
    {
        // GUX-RyzenXHCIFix README: Ryzen APU laptops (its author's is a
        // Picasso) hang while initialising their XHCI controllers on 11+;
        // the fork runs GenericUSBXHCI's early init and then hands the
        // controllers back to Apple's driver.
        push(
            plan,
            kext(
                "GenericUSBXHCI",
                "GenericUSBXHCI.kext",
                "Works around the XHCI boot hang of Ryzen APU laptops on macOS 11+ (Apple's driver keeps the ports).",
            )
            .optional()
            .min(DARWIN_11),
        );
    }
    if matches!(ctx.platform(), CpuPlatform::AmdBulldozer | CpuPlatform::AmdJaguar) {
        push(plan, kext("XLNCUSBFix", "XLNCUSBFix.kext", "USB fix for AMD FX-era chipsets.").optional());
    }
}

// ── Input ───────────────────────────────────────────────────────────────────

fn input(ctx: &PlanContext, plan: &mut BuildPlan) {
    if ctx.is_vm {
        return;
    }
    let inp = &ctx.profile.input;
    let laptop_like = ctx.is_laptop || ctx.profile.form_factor == FormFactor::AllInOne;
    let touchpad = if laptop_like || inp.touchpad_bus.is_some() || inp.touchpad_hid.is_some() {
        device_db::touchpad_driver(inp.touchpad_bus, inp.touchpad_vendor, inp.touchpad_hid.as_deref())
    } else {
        TouchpadDriver::None
    };
    let ps2_touchpad = matches!(touchpad, TouchpadDriver::Ps2 | TouchpadDriver::RmiSmbus | TouchpadDriver::ElanSmbus);
    // Dortania ktext: most laptop keyboards are PS/2, so laptops always get
    // VoodooPS2 unless the keyboard is known to be USB.
    let ps2 = ps2_touchpad || inp.keyboard_bus == InputBus::Ps2 || (ctx.is_laptop && inp.keyboard_bus != InputBus::Usb);
    // Only one VoodooInput may load; the touchpad driver's own copy wins.
    let other_voodooinput = matches!(
        touchpad,
        TouchpadDriver::I2cHid | TouchpadDriver::RmiI2c | TouchpadDriver::RmiSmbus | TouchpadDriver::AlpsHid
    );
    if ps2 {
        let trackpad = ps2_touchpad || ctx.is_laptop;
        // OpCore-Simplify: VoodooPS2Mouse stays off next to the ELAN VoodooSMBus driver.
        let mouse = touchpad != TouchpadDriver::ElanSmbus;
        push(
            plan,
            kext("VoodooPS2Controller", "VoodooPS2Controller.kext", "PS/2 keyboard and trackpad.")
                .plugin("VoodooInput.kext", trackpad && !other_voodooinput, None, None)
                .plugin("VoodooPS2Keyboard.kext", true, None, None)
                .plugin("VoodooPS2Trackpad.kext", trackpad, None, None)
                .plugin("VoodooPS2Mouse.kext", mouse, None, None),
        );
    }
    let voodoo_i2c = |own_input: bool, what: &str| {
        kext("VoodooI2C", "VoodooI2C.kext", format!("I2C controller driver for the {what}."))
            .plugin("VoodooGPIO.kext", true, None, None)
            .plugin("VoodooI2CServices.kext", true, None, None)
            .plugin("VoodooInput.kext", own_input, None, None)
    };
    match touchpad {
        TouchpadDriver::I2cHid => {
            push(plan, voodoo_i2c(true, "touchpad"));
            push(plan, kext("VoodooI2C", "VoodooI2CHID.kext", "I2C HID touchpad (precision touchpad) driver."));
            input_gpio_note(plan);
        }
        TouchpadDriver::RmiI2c => {
            // VoodooRMI README: VoodooRMI + its VoodooInput, VoodooI2C, RMII2C.
            push(
                plan,
                kext("VoodooRMI", "VoodooRMI.kext", "Synaptics RMI4 touchpad over I2C.")
                    .plugin("VoodooInput.kext", true, None, None)
                    .plugin("RMII2C.kext", true, None, None)
                    .plugin("RMISMBus.kext", false, None, None),
            );
            push(plan, voodoo_i2c(false, "touchpad"));
            input_gpio_note(plan);
        }
        TouchpadDriver::RmiSmbus => {
            push(
                plan,
                kext("VoodooRMI", "VoodooRMI.kext", "Synaptics RMI4 touchpad over SMBus.")
                    .plugin("VoodooInput.kext", true, None, None)
                    .plugin("RMISMBus.kext", true, None, None)
                    .plugin("RMII2C.kext", false, None, None),
            );
            push(plan, kext("VoodooRMI", "VoodooSMBus.kext", "SMBus controller driver for the Synaptics touchpad."));
        }
        TouchpadDriver::ElanSmbus => {
            push(
                plan,
                kext(
                    "VoodooSMBus",
                    "VoodooSMBus.kext",
                    "ELAN touchpad over SMBus (falls back to PS/2 mode without it).",
                )
                .optional()
                .min(DARWIN_10_14),
            );
        }
        TouchpadDriver::AlpsHid => {
            push(plan, voodoo_i2c(true, "touchpad"));
            // AlpsHID ships its own VoodooI2CHID; never load two.
            push(plan, kext("AlpsHID", "AlpsHID.kext", "Alps touchpad driver."));
            push(plan, kext("AlpsHID", "VoodooI2CHID.kext", "AlpsHID's own VoodooI2CHID build."));
        }
        TouchpadDriver::Ps2 | TouchpadDriver::None => {}
    }
    // Dortania ktext.md: VoodooI2CHID also drives I2C/USB touchscreens.
    // The I2C HID and Alps touchpad stacks already carry a VoodooI2CHID;
    // with VoodooRMI over I2C it must stay out (VoodooRMI README).
    if inp.has_touchscreen
        && matches!(
            touchpad,
            TouchpadDriver::Ps2 | TouchpadDriver::RmiSmbus | TouchpadDriver::ElanSmbus | TouchpadDriver::None
        )
    {
        let input_taken = plan.kexts.iter().any(|k| {
            k.enabled && k.plugins.iter().any(|p| p.enabled && p.bundle.eq_ignore_ascii_case("VoodooInput.kext"))
        });
        push(plan, voodoo_i2c(!input_taken, "touchscreen").optional());
        push(
            plan,
            kext("VoodooI2C", "VoodooI2CHID.kext", "Touchscreen (HID over I2C/USB) driver.").optional(),
        );
        input_gpio_note(plan);
    }
}

fn input_gpio_note(plan: &mut BuildPlan) {
    note(
        plan,
        NoteLevel::Info,
        "input",
        "I2C input devices",
        "I2C touchpads and touchscreens need their GPIO interrupt pinned (SSDT-GPI0) or polling mode; if one does \
         not respond, add the boot-arg -vi2c-force-polling.",
    );
}

// ── Storage ─────────────────────────────────────────────────────────────────

fn storage(ctx: &PlanContext, plan: &mut BuildPlan) {
    let mut nvmefix = false;
    let mut ctlna = false;
    // RST-mode ids (RAID class code): no AHCI class match before 11 either.
    let mut rst_mode = false;
    for drive in &ctx.profile.storage {
        let advice = device_db::storage_advice(drive);
        nvmefix |= advice.nvmefix;
        ctlna |= advice.ctlna_ahci;
        rst_mode |= advice.ctlna_ahci
            && drive.vendor_id.as_deref().and_then(device_db::parse_id16) == Some(0x8086)
            && matches!(drive.device_id.as_deref().and_then(device_db::parse_id16), Some(0x2822 | 0x282A));
        if !advice.notes.is_empty() {
            let level = if advice.problematic { NoteLevel::Warning } else { NoteLevel::Info };
            let name = if drive.name.trim().is_empty() { "Storage" } else { drive.name.trim() };
            note(plan, level, "storage", name.to_string(), advice.notes.join(" "));
        }
    }
    if nvmefix && ctx.target >= MacOsVersion::Mojave {
        push(
            plan,
            kext(
                "NVMeFix",
                "NVMeFix.kext",
                "Power management (APST) and compatibility fixes for non-Apple NVMe drives.",
            )
            .optional()
            .min(DARWIN_10_14),
        );
    }
    if ctlna {
        if ctx.target >= MacOsVersion::BigSur {
            push(
                plan,
                kext(
                    "CtlnaAHCIPort",
                    "CtlnaAHCIPort.kext",
                    "AHCI driver for a SATA controller macOS 11+ no longer supports.",
                )
                .min(DARWIN_11),
            );
        } else if rst_mode {
            note(
                plan,
                NoteLevel::Warning,
                "storage",
                "SATA controller needs SATA-unsupported.kext",
                "On macOS 10.15 and older this Intel RST-mode SATA controller needs SATA-unsupported.kext, which is \
                 not in the download catalog: switch SATA to AHCI in the BIOS, add the kext manually or use an NVMe \
                 drive.",
            );
        } else {
            // Dortania ktext.md: CtlnaAHCIPort is a Big Sur+ matter, "Catalina
            // and older need not concern"; SATA-unsupported only if needed.
            note(
                plan,
                NoteLevel::Info,
                "storage",
                "SATA drive not visible?",
                "If the installer does not show the SATA drive on macOS 10.15 or older, add SATA-unsupported.kext \
                 (not in the download catalog) to EFI/OC/Kexts.",
            );
        }
    }
}

// ── Virtual machines ────────────────────────────────────────────────────────

fn vm(ctx: &PlanContext, plan: &mut BuildPlan, st: &mut State) {
    if ctx.profile.vm != Some(VmKind::HyperV) {
        return;
    }
    // MacHyperVSupport README: one core kext per release range.
    if ctx.target >= MacOsVersion::Monterey {
        push(
            plan,
            kext("MacHyperVSupport", "MacHyperVSupportMonterey.kext", "Hyper-V integration services (macOS 12+).")
                .min(DARWIN_12),
        );
    } else {
        push(
            plan,
            kext("MacHyperVSupport", "MacHyperVSupport.kext", "Hyper-V integration services (macOS 11 and older).")
                .max(MAX_11),
        );
    }
    // The synthetic network adapter is driven by MacHyperVSupport.
    st.ethernet_ok = true;
}

// ── Network sanity ──────────────────────────────────────────────────────────

fn network_check(plan: &mut BuildPlan, st: &State) {
    if st.ethernet_ok || st.wifi_recovery_ok {
        return;
    }
    let mut detail = String::from(
        "The installer is downloaded from Apple inside macOS Recovery, which needs a network connection, but no \
         Ethernet port and no Wi-Fi card of this machine works there.",
    );
    if st.wifi_ok {
        detail.push_str(" The Wi-Fi card only works after installation.");
    }
    detail.push_str(
        " Use a supported PCIe network card, or a USB Ethernet adapter that macOS drives natively (e.g. ASIX \
         AX88179 or RTL8153 based), during the install.",
    );
    note(plan, NoteLevel::Warning, "network", "No network in macOS Recovery", detail);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        BuildOptions, CpuPlatform as P, GpuVendor, HardwareProfile, ProfileAudio, ProfileCpu, ProfileGpu, ProfileInput,
        ProfileStorage, StorageKind, TouchpadVendor,
    };
    use crate::domain::planner::{empty_plan, DisplayPlan, PlanContext};
    use MacOsVersion::*;

    const ALC1220: u32 = 0x10EC_1220;
    const ALC256: u32 = 0x10EC_0256;
    const ALC897: u32 = 0x10EC_0897;
    const HDA_100: &str = "PciRoot(0x0)/Pci(0x1f,0x3)";
    const NIC_PATH: &str = "PciRoot(0x0)/Pci(0x1C,0x1)/Pci(0x0,0x0)";
    const WIFI_PATH: &str = "PciRoot(0x0)/Pci(0x1C,0x4)/Pci(0x0,0x0)";

    // ── Profile builders ────────────────────────────────────────────────────

    fn cpu(platform: P) -> ProfileCpu {
        let info = cpu_db::platform_info(platform);
        ProfileCpu {
            name: info.label.to_string(),
            vendor: info.vendor,
            platform,
            cores: 8,
            threads: 16,
            has_avx2: Some(info.has_avx2),
            ..Default::default()
        }
    }

    fn machine(platform: P, form: FormFactor, chipset: Option<&str>, vendor: &str) -> HardwareProfile {
        HardwareProfile {
            cpu: cpu(platform),
            form_factor: form,
            chipset: chipset.map(str::to_string),
            motherboard_vendor: vendor.to_string(),
            has_battery: form == FormFactor::Laptop,
            source: "manual".into(),
            ..Default::default()
        }
    }

    fn pci(vendor: &str, device: &str, path: Option<&str>) -> ProfileNic {
        ProfileNic {
            bus: DeviceBus::Pci,
            vendor_id: Some(vendor.into()),
            device_id: Some(device.into()),
            pci_path: path.map(str::to_string),
            ..Default::default()
        }
    }

    fn usb_dev(vendor: &str, product: &str) -> ProfileNic {
        ProfileNic { bus: DeviceBus::Usb, ..pci(vendor, product, None) }
    }

    fn codec(id: u32) -> ProfileAudio {
        ProfileAudio { codec_name: codec_db::codec_name(id), codec_id: Some(id), ..Default::default() }
    }

    fn gpu(name: &str, vendor: GpuVendor, family: GpuFamily, vendor_id: &str, device_id: &str) -> ProfileGpu {
        ProfileGpu {
            name: name.into(),
            vendor,
            family,
            vendor_id: Some(vendor_id.into()),
            device_id: Some(device_id.into()),
            ..Default::default()
        }
    }

    fn options(target: MacOsVersion) -> BuildOptions {
        BuildOptions { target, ..Default::default() }
    }

    /// What the SMBIOS stage would have written: SecureBootModel Disabled
    /// from 14.4 on (Sonoma recovery installs the latest 14.x).
    fn run_with(
        profile: &HardwareProfile,
        opts: &BuildOptions,
        model: &str,
        prefill: impl FnOnce(&mut BuildPlan),
    ) -> BuildPlan {
        // The first dGPU drives the displays, else the iGPU.
        let igpu = profile.gpus.iter().position(|g| g.is_igpu);
        let dgpu = profile.gpus.iter().position(|g| !g.is_igpu);
        let display = DisplayPlan { primary: dgpu.or(igpu), igpu, igpu_headless: dgpu.is_some(), disabled: vec![] };
        run_display(profile, opts, model, &display, prefill)
    }

    fn run_display(
        profile: &HardwareProfile,
        opts: &BuildOptions,
        model: &str,
        display: &DisplayPlan,
        prefill: impl FnOnce(&mut BuildPlan),
    ) -> BuildPlan {
        let ctx = PlanContext::new(profile, opts);
        let mut plan = empty_plan(opts.target);
        plan.smbios.model = model.into();
        plan.smbios.secure_boot_model = if opts.target >= Sonoma { "Disabled".into() } else { "Default".into() };
        prefill(&mut plan);
        apply(&ctx, display, &mut plan);
        check_invariants(&plan);
        plan
    }

    fn run(profile: &HardwareProfile, target: MacOsVersion, model: &str) -> BuildPlan {
        run_with(profile, &options(target), model, |_| {})
    }

    // ── Assertion helpers ───────────────────────────────────────────────────

    /// Plugins each bundle really carries in its Contents/PlugIns
    /// (research-kexts §2, §3.9).
    const PLUGINS: &[(&str, &[&str])] = &[
        (
            "VoodooPS2Controller.kext",
            &["VoodooPS2Keyboard.kext", "VoodooPS2Trackpad.kext", "VoodooPS2Mouse.kext", "VoodooInput.kext"],
        ),
        ("VoodooI2C.kext", &["VoodooGPIO.kext", "VoodooI2CServices.kext", "VoodooInput.kext"]),
        ("VoodooRMI.kext", &["RMII2C.kext", "RMISMBus.kext", "VoodooInput.kext"]),
        ("AirportBrcmFixup.kext", &["AirPortBrcm4360_Injector.kext", "AirPortBrcmNIC_Injector.kext"]),
        ("IO80211FamilyLegacy.kext", &["AirPortBrcmNIC.kext"]),
    ];

    /// OpenCore's Darwin encoding (major*10000 + minor*100 + patch).
    fn darwin(v: &str) -> u32 {
        let mut it = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        let (a, b, c) = (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0));
        a * 10_000 + b.min(99) * 100 + c.min(99)
    }

    fn valid_kernel(v: &str) -> bool {
        let parts: Vec<&str> = v.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    }

    fn check_invariants(plan: &BuildPlan) {
        let mut seen = std::collections::HashSet::new();
        let mut voodoo_inputs = 0;
        for k in &plan.kexts {
            let entry =
                kext_catalog::entry(&k.catalog_id).unwrap_or_else(|| panic!("unknown catalog id {}", k.catalog_id));
            assert!(entry.provides(&k.bundle), "{} does not provide {}", k.catalog_id, k.bundle);
            assert!(seen.insert((k.catalog_id.clone(), k.bundle.clone())), "duplicate {} {}", k.catalog_id, k.bundle);
            assert!(!k.reason.is_empty(), "{} has no reason", k.bundle);
            for v in k.min_kernel.iter().chain(k.max_kernel.iter()) {
                assert!(valid_kernel(v), "{}: bad kernel {v}", k.bundle);
            }
            if !k.enabled {
                assert!(!k.required, "{} disabled but required", k.bundle);
            }
            if !k.plugins.is_empty() {
                let allowed = PLUGINS
                    .iter()
                    .find(|(b, _)| *b == k.bundle)
                    .unwrap_or_else(|| panic!("{} has no plugins", k.bundle))
                    .1;
                for p in &k.plugins {
                    assert!(allowed.contains(&p.bundle.as_str()), "{} has no plugin {}", k.bundle, p.bundle);
                    for v in p.min_kernel.iter().chain(p.max_kernel.iter()) {
                        assert!(valid_kernel(v));
                    }
                    if p.bundle == "VoodooInput.kext" && p.enabled && k.enabled {
                        voodoo_inputs += 1;
                    }
                }
            }
        }
        assert!(voodoo_inputs <= 1, "{voodoo_inputs} VoodooInput copies enabled");
        assert_eq!(plan.kexts.first().map(|k| k.bundle.as_str()), Some("Lilu.kext"));
        assert_eq!(plan.kexts.get(1).map(|k| k.bundle.as_str()), Some("VirtualSMC.kext"));
        // Every enabled kext can load on the target release.
        let (lo, hi) = (darwin(&plan.target.min_kernel()), darwin(&plan.target.max_kernel()));
        for k in plan.kexts.iter().filter(|k| k.enabled) {
            let min = k.min_kernel.as_deref().map_or(0, darwin);
            let max = k.max_kernel.as_deref().map_or(u32::MAX, darwin);
            assert!(min <= hi && lo <= max, "{} ({min}-{max}) cannot load on {:?}", k.bundle, plan.target);
        }
        // One value per boot-arg key, no spaces inside an argument.
        let mut keys = std::collections::HashSet::new();
        for a in &plan.boot_args {
            assert!(!a.is_empty() && !a.contains(' '), "bad boot-arg {a:?}");
            assert!(keys.insert(a.split('=').next().unwrap_or(a)), "boot-arg {a} written twice");
        }
        for p in &plan.kernel_patches {
            assert!(valid_kernel(&p.min_kernel) && valid_kernel(&p.max_kernel), "{}", p.comment);
        }
        let mut blocks = std::collections::HashSet::new();
        for b in &plan.kernel_blocks {
            assert!(blocks.insert(b.identifier.as_str()) && valid_kernel(&b.min_kernel), "{}", b.comment);
            assert!(matches!(b.strategy.as_str(), "Disable" | "Exclude"));
        }
        // Bluetooth variables are added and deleted together.
        assert_eq!(plan.nvram_add.len(), plan.nvram_delete.len());
        for v in &plan.nvram_add {
            assert!(plan.nvram_delete.iter().any(|d| d.guid == v.guid && d.key == v.key), "{}", v.key);
        }
        if plans_root_patch(plan) {
            assert_eq!(find(plan, AMFIPASS_ID, "AMFIPass.kext").min_kernel.as_deref(), Some("20.0.0"));
        }
        for n in plan.notes.iter().chain(plan.post_install.iter()) {
            assert!(!n.component.is_empty() && !n.title.trim().is_empty(), "{n:?}");
        }
        for e in &plan.device_properties {
            for p in &e.properties {
                if let PlistScalar::Data(hex) = &p.value {
                    assert!(hex.len() % 2 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()));
                }
            }
        }
    }

    fn find<'a>(plan: &'a BuildPlan, catalog: &str, bundle: &str) -> &'a KextSelection {
        plan.kexts
            .iter()
            .find(|k| k.catalog_id == catalog && k.bundle == bundle)
            .unwrap_or_else(|| panic!("missing {catalog}/{bundle}: {:?}", ids(plan)))
    }

    fn ids(plan: &BuildPlan) -> Vec<String> {
        plan.kexts.iter().map(|k| format!("{}/{}", k.catalog_id, k.bundle)).collect()
    }

    fn has_bundle(plan: &BuildPlan, bundle: &str) -> bool {
        plan.kexts.iter().any(|k| k.bundle == bundle)
    }

    fn has_catalog(plan: &BuildPlan, catalog: &str) -> bool {
        plan.kexts.iter().any(|k| k.catalog_id == catalog)
    }

    fn plugin<'a>(sel: &'a KextSelection, bundle: &str) -> &'a PluginSelection {
        sel.plugins
            .iter()
            .find(|p| p.bundle == bundle)
            .unwrap_or_else(|| panic!("{} lacks plugin {bundle}", sel.bundle))
    }

    fn prop<'a>(plan: &'a BuildPlan, path: &str, key: &str) -> Option<&'a PlistScalar> {
        plan.device_properties
            .iter()
            .filter(|e| e.path.eq_ignore_ascii_case(path))
            .flat_map(|e| e.properties.iter())
            .find(|p| p.key == key)
            .map(|p| &p.value)
    }

    fn arg<'a>(plan: &'a BuildPlan, prefix: &str) -> Option<&'a str> {
        plan.boot_args.iter().find(|a| a.starts_with(prefix)).map(String::as_str)
    }

    fn range(sel: &KextSelection) -> (Option<&str>, Option<&str>) {
        (sel.min_kernel.as_deref(), sel.max_kernel.as_deref())
    }

    fn data(bytes: &[u8]) -> PlistScalar {
        PlistScalar::data(bytes)
    }

    fn has_note(plan: &BuildPlan, component: &str, needle: &str) -> bool {
        plan.notes.iter().any(|n| n.component == component && (n.title.contains(needle) || n.detail.contains(needle)))
    }

    fn network_warning(plan: &BuildPlan) -> bool {
        plan.notes.iter().any(|n| n.component == "network" && n.level == NoteLevel::Warning)
    }

    // ── Desktops ────────────────────────────────────────────────────────────

    fn z390() -> HardwareProfile {
        let mut p = machine(P::CoffeeLake, FormFactor::Desktop, Some("Z390"), "Gigabyte Technology Co., Ltd.");
        p.ethernet = vec![pci("8086", "15bc", Some(NIC_PATH))];
        p.audio = Some(codec(ALC1220));
        p
    }

    #[test]
    fn z390_i219_alc1220_sequoia() {
        let plan = run(&z390(), Sequoia, "iMac19,1");
        let mausi = find(&plan, "IntelMausi", "IntelMausi.kext");
        assert!(mausi.required && mausi.enabled);
        assert!(!has_catalog(&plan, "IntelMausiEthernet"));
        assert!(!find(&plan, "VirtualSMC", "SMCProcessor.kext").required);
        find(&plan, "VirtualSMC", "SMCSuperIO.kext");
        assert!(!has_bundle(&plan, "SMCBatteryManager.kext"));
        find(&plan, "AppleALC", "AppleALC.kext");
        let layout = codec_db::default_layout(ALC1220, oem_subsystem("Gigabyte"), false).unwrap();
        assert_eq!(prop(&plan, HDA_100, "layout-id"), Some(&data(&layout.to_le_bytes())));
        assert!(arg(&plan, "alcid=").is_none());
        assert_eq!(prop(&plan, NIC_PATH, "built-in"), Some(&data(&[1])));
        find(&plan, "USBToolBox", "USBToolBox.kext");
        find(&plan, "USBToolBox", "UTBDefault.kext");
        assert!(!has_catalog(&plan, "XHCI-unsupported"), "Z390 is native from Mojave");
        assert!(!has_catalog(&plan, "VoodooPS2Controller"));
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=sbvmm"));
        assert!(!find(&plan, "RestrictEvents", "RestrictEvents.kext").required);
        assert!(plan.post_install.iter().any(|n| n.component == "usb"));
        assert!(!network_warning(&plan));
        assert!(!plans_root_patch(&plan));
    }

    #[test]
    fn z390_high_sierra_needs_xhci_unsupported_and_no_ota_patch() {
        let plan = run(&z390(), HighSierra, "iMac18,3");
        find(&plan, "XHCI-unsupported", "XHCI-unsupported.kext");
        assert!(arg(&plan, "revpatch=").is_none());
        assert!(!has_catalog(&plan, "RestrictEvents"));
        assert!(!has_catalog(&plan, "NVMeFix"));
    }

    fn z490_i225v() -> HardwareProfile {
        let mut p = machine(P::CometLake, FormFactor::Desktop, Some("Z490"), "ASUSTeK COMPUTER INC.");
        p.ethernet = vec![pci("8086", "15f3", Some(NIC_PATH))];
        p
    }

    #[test]
    fn z490_i225v_monterey_uses_native_driver_with_spoof() {
        let plan = run(&z490_i225v(), Monterey, "iMac20,1");
        assert_eq!(prop(&plan, NIC_PATH, "device-id"), Some(&data(&[0xF2, 0x15, 0, 0])));
        assert!(plan.boot_args.iter().any(|a| a == "e1000=0"));
        assert!(!has_catalog(&plan, "AppleIGC"));
        assert!(plan.kernel_patches.is_empty(), "12.x needs no I225-V patch");
        assert_eq!(prop(&plan, NIC_PATH, "built-in"), Some(&data(&[1])));
        assert!(!network_warning(&plan));
    }

    #[test]
    fn z490_i225v_sonoma_uses_appleigc() {
        let plan = run(&z490_i225v(), Sonoma, "iMac20,1");
        let igc = find(&plan, "AppleIGC", "AppleIGC.kext");
        assert!(igc.required);
        assert_eq!(range(igc), (Some("19.0.0"), None));
        assert!(prop(&plan, NIC_PATH, "device-id").is_none());
        assert!(arg(&plan, "e1000=").is_none());
        assert_eq!(prop(&plan, NIC_PATH, "built-in"), Some(&data(&[1])));
    }

    #[test]
    fn z490_i225v_catalina_gets_kernel_patch() {
        let plan = run(&z490_i225v(), Catalina, "iMac20,1");
        let patch = plan.kernel_patches.iter().find(|p| p.comment == "I225-V patch").expect("patch");
        assert_eq!(patch.identifier, "com.apple.driver.AppleIntelI210Ethernet");
        assert_eq!((patch.find.as_str(), patch.replace.as_str()), ("F2150000", "F3150000"));
        assert_eq!((patch.min_kernel.as_str(), patch.max_kernel.as_str()), ("19.0.0", "20.4.0"));
        assert_eq!(prop(&plan, NIC_PATH, "device-id"), Some(&data(&[0xF2, 0x15, 0, 0])));
        assert!(arg(&plan, "e1000=").is_none());
    }

    #[test]
    fn i226_always_uses_appleigc() {
        let mut p = z490_i225v();
        p.ethernet = vec![pci("8086", "125c", Some(NIC_PATH))];
        let plan = run(&p, Monterey, "iMac20,1");
        find(&plan, "AppleIGC", "AppleIGC.kext");
        assert!(arg(&plan, "e1000=").is_none());
    }

    fn b550() -> HardwareProfile {
        let mut p = machine(P::AmdZen3, FormFactor::Desktop, Some("B550"), "Micro-Star International Co., Ltd.");
        p.ethernet = vec![pci("10ec", "8168", Some(NIC_PATH))];
        p.wifi = Some(pci("8086", "2723", Some(WIFI_PATH)));
        p.bluetooth = Some(usb_dev("8087", "0029"));
        p.audio = Some(codec(ALC897));
        p
    }

    #[test]
    fn b550_rtl8111_ax200_sequoia() {
        let plan = run(&b550(), Sequoia, "MacPro7,1");
        let rtl = find(&plan, "RealtekRTL8111-2.4.2", "RealtekRTL8111.kext");
        assert!(rtl.required);
        assert_eq!(range(rtl), (Some("18.0.0"), None));
        assert!(!has_catalog(&plan, "RealtekRTL8111"));
        // AMD: no Intel sensors, Zen power management on.
        assert!(!has_bundle(&plan, "SMCProcessor.kext") && !has_bundle(&plan, "SMCSuperIO.kext"));
        assert!(find(&plan, "AMDRyzenCPUPowerManagement", "AMDRyzenCPUPowerManagement.kext").enabled);
        assert!(find(&plan, "SMCAMDProcessor", "SMCAMDProcessor.kext").enabled);
        assert_eq!(
            range(find(&plan, "AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext")),
            (Some("19.0.0"), None)
        );
        // Wi-Fi: no AirportItlwm build for 15 -> itlwm + HeliPort.
        assert!(!find(&plan, "itlwm", "itlwm.kext").required);
        assert!(!plan.kexts.iter().any(|k| k.catalog_id.starts_with("AirportItlwm")));
        assert!(plan.post_install.iter().any(|n| n.title.contains("HeliPort")));
        // Bluetooth on 12+: firmware + patcher + BlueToolFixup, no injector.
        find(&plan, "IntelBluetoothFirmware", "IntelBluetoothFirmware.kext");
        find(&plan, "IntelBluetoothFirmware", "IntelBTPatcher.kext");
        assert_eq!(range(find(&plan, "BrcmPatchRAM", "BlueToolFixup.kext")), (Some("21.0.0"), None));
        assert!(!has_bundle(&plan, "IntelBluetoothInjector.kext"));
        let dongle = plan.nvram_add.iter().find(|v| v.key == "bluetoothExternalDongleFailed").unwrap();
        assert_eq!((dongle.guid.as_str(), &dongle.value), (APPLE_NVRAM_GUID, &data(&[0])));
        let info = plan.nvram_add.iter().find(|v| v.key == "bluetoothInternalControllerInfo").unwrap();
        assert_eq!(info.value, data(&[0; 14]));
        assert_eq!(plan.nvram_delete.len(), 2);
        assert!(arg(&plan, "-ibtcompatbeta").is_none());
        // Audio: AMD has no fixed HDA path.
        let layout = codec_db::default_layout(ALC897, oem_subsystem("Micro-Star International"), false).unwrap();
        assert_eq!(arg(&plan, "alcid="), Some(format!("alcid={layout}").as_str()));
        assert!(plan.device_properties.iter().all(|e| e.properties.iter().all(|p| p.key != "layout-id")));
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=pci,cpuname,sbvmm"));
        assert!(arg(&plan, "revcpu=").is_none(), "revcpu defaults to on for AMD");
        assert!(!network_warning(&plan));
    }

    #[test]
    fn b550_tahoe_keeps_applealc_and_warns() {
        let plan = run(&b550(), Tahoe, "MacPro7,1");
        find(&plan, "AppleALC", "AppleALC.kext");
        assert!(plan
            .notes
            .iter()
            .any(|n| n.component == "audio" && n.level == NoteLevel::Warning && n.title.contains("macOS 26")));
        assert!(plan.post_install.iter().any(|n| n.title.contains("AppleHDA")));
        find(&plan, "itlwm", "itlwm.kext");
        // The maintained IntelBluetoothFirmware fork supports macOS 26 itself.
        assert!(arg(&plan, "-ibtcompatbeta").is_none());
        assert!(arg(&plan, "-amfipassbeta").is_none());
        assert!(has_note(&plan, "cpu", "macOS 26"));
    }

    #[test]
    fn x570_aquantia_needs_casey_patches_from_monterey() {
        let mut p = machine(P::AmdZen2, FormFactor::Desktop, Some("X570"), "ASUSTeK COMPUTER INC.");
        p.ethernet = vec![pci("1d6a", "07b1", Some(NIC_PATH))];
        // Big Sur: Apple's driver works without AppleVTD.
        let big_sur = run(&p, BigSur, "MacPro7,1");
        assert!(!big_sur.kexts.iter().any(|k| k.bundle.contains("Aquantia")));
        assert!(has_note(&big_sur, "ethernet", "ForceAquantiaEthernet"));
        assert_eq!(prop(&big_sur, NIC_PATH, "built-in"), Some(&data(&[1])));
        assert!(!network_warning(&big_sur));
        assert!(!has_catalog(&big_sur, "AppleMCEReporterDisabler"), "AMD needs it from 12.3");
        // Monterey on AMD: no AppleVTD, so the port needs the CaseySJ patches.
        let plan = run(&p, Monterey, "MacPro7,1");
        assert!(plan
            .notes
            .iter()
            .any(|n| n.component == "ethernet" && n.level == NoteLevel::Warning && n.detail.contains("CaseySJ")));
        assert!(prop(&plan, NIC_PATH, "built-in").is_none());
        assert!(network_warning(&plan));
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=pci,cpuname"));
        find(&plan, "AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext");
        // Intel boards keep AppleVTD, so the native driver counts as working.
        let mut intel = machine(P::CometLake, FormFactor::Desktop, Some("Z490"), "ASUS");
        intel.ethernet = p.ethernet.clone();
        assert!(!network_warning(&run(&intel, Monterey, "iMac20,1")));
    }

    #[test]
    fn rtl8125_prefers_rtl812xlucy_with_disabled_fallback() {
        let mut p = machine(P::AlderLake, FormFactor::Desktop, Some("Z690"), "ASRock");
        p.ethernet = vec![pci("10ec", "8125", Some(NIC_PATH))];
        let plan = run(&p, Sonoma, "MacPro7,1");
        let lucy = find(&plan, "RTL812xLucy", "RTL812xLucy.kext");
        assert!(lucy.enabled && lucy.required);
        let fallback = find(&plan, "LucyRTL8125Ethernet", "LucyRTL8125Ethernet.kext");
        assert!(!fallback.enabled && !fallback.required);

        p.ethernet = vec![pci("10ec", "8126", Some(NIC_PATH))];
        let plan = run(&p, Sonoma, "MacPro7,1");
        find(&plan, "RTL812xLucy", "RTL812xLucy.kext");
        assert!(!has_catalog(&plan, "LucyRTL8125Ethernet"));
    }

    #[test]
    fn i211_switches_driver_at_monterey() {
        let mut p = machine(P::CoffeeLake, FormFactor::Desktop, Some("Z370"), "ASUS");
        p.ethernet = vec![pci("8086", "1539", Some(NIC_PATH))];
        let big_sur = run(&p, BigSur, "iMac19,1");
        assert_eq!(
            range(find(&big_sur, "SmallTreeIntel82576", "SmallTreeIntel82576.kext")),
            (Some("19.0.0"), Some("20.99.99"))
        );
        assert!(!has_catalog(&big_sur, "AppleIGB"));
        let monterey = run(&p, Monterey, "iMac19,1");
        assert_eq!(range(find(&monterey, "AppleIGB", "AppleIGB.kext")), (Some("21.0.0"), None));
        assert!(!has_catalog(&monterey, "SmallTreeIntel82576"));
    }

    #[test]
    fn i210_on_ventura_reinjects_apple_kext() {
        let mut p = machine(P::SkylakeX, FormFactor::Desktop, Some("X299"), "Supermicro");
        p.ethernet = vec![pci("8086", "1533", Some(NIC_PATH))];
        let plan = run(&p, Ventura, "iMacPro1,1");
        assert_eq!(range(find(&plan, "AppleIntelI210Ethernet", "AppleIntelI210Ethernet.kext")), (Some("22.0.0"), None));
        assert!(plan.boot_args.iter().any(|a| a == "e1000=0"));
        // Intel HEDT: CpuTscSync, MCE disabler for the iMac Pro board-id.
        assert!(!find(&plan, "CpuTscSync", "CpuTscSync.kext").required);
        find(&plan, "AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext");
        assert!(!has_catalog(&plan, "ForgedInvariant"));
    }

    #[test]
    fn rtl8111_on_high_sierra_has_no_usable_build() {
        let mut p = machine(P::Haswell, FormFactor::Desktop, Some("Z97"), "Gigabyte");
        p.ethernet = vec![pci("10ec", "8168", Some(NIC_PATH))];
        let plan = run(&p, HighSierra, "iMac15,1");
        assert!(!plan.kexts.iter().any(|k| k.bundle == "RealtekRTL8111.kext"));
        assert!(has_note(&plan, "ethernet", "10.14"));
        assert!(network_warning(&plan));
        // Intel desktop: the 3.0.0 build is used from Mojave on.
        let plan = run(&p, Mojave, "iMac15,1");
        find(&plan, "RealtekRTL8111", "RealtekRTL8111.kext");
    }

    #[test]
    fn hybrid_alder_lake_tahoe() {
        let mut p = machine(P::AlderLake, FormFactor::Desktop, Some("Z690"), "MSI");
        p.cpu.is_hybrid = true;
        p.ethernet = vec![pci("8086", "1a1d", Some(NIC_PATH))];
        let plan = run(&p, Tahoe, "MacPro7,1");
        assert!(!find(&plan, "CpuTopologyRebuild", "CpuTopologyRebuild.kext").required);
        find(&plan, "AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext");
        let mausi = find(&plan, "IntelMausiEthernet", "IntelMausiEthernet.kext");
        assert_eq!(range(mausi), (Some("19.0.0"), None));
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=pci,cpuname,sbvmm"));
        assert!(plan.boot_args.iter().any(|a| a == "revcpu=1"));

        // Non-hybrid Alder Lake (P-cores only) needs no topology rebuild.
        p.cpu.is_hybrid = false;
        let plan = run(&p, Sequoia, "MacPro7,1");
        assert!(!has_catalog(&plan, "CpuTopologyRebuild"));
        assert!(!has_catalog(&plan, "AppleMCEReporterDisabler"));
    }

    #[test]
    fn sandy_bridge_cryptexfixup_only_from_ventura() {
        let mut p = machine(P::SandyBridge, FormFactor::Desktop, Some("Z68"), "ASUS");
        p.cpu.has_avx2 = Some(false);
        let monterey = run(&p, Monterey, "iMac13,2");
        assert!(!has_catalog(&monterey, "CryptexFixup"));
        let ventura = run(&p, Ventura, "iMac13,2");
        let fixup = find(&ventura, "CryptexFixup", "CryptexFixup.kext");
        assert!(fixup.required);
        assert_eq!(range(fixup), (Some("22.0.0"), None));
        assert!(has_note(&ventura, "cpu", "AVX2"));
    }

    #[test]
    fn ivy_bridge_ventura_gets_f16c() {
        let mut p = machine(P::IvyBridge, FormFactor::Desktop, Some("Z77"), "ASUS");
        p.cpu.has_avx2 = Some(false);
        let plan = run(&p, Ventura, "MacPro6,1");
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=f16c"));
        find(&plan, "CryptexFixup", "CryptexFixup.kext");
        let plan = run(&p, Monterey, "MacPro6,1");
        assert!(arg(&plan, "revpatch=").is_none());
    }

    #[test]
    fn revpatch_merges_with_earlier_stage() {
        let p = z390();
        let plan = run_with(&p, &options(Sequoia), "iMac19,1", |plan| {
            plan.boot_args.push("revpatch=asset".into());
            plan.kexts.push(kext("WhateverGreen", "WhateverGreen.kext", "GPU"));
        });
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=asset,sbvmm"));
        // Lilu and VirtualSMC move in front of the GPU kext.
        assert_eq!(plan.kexts[2].bundle, "WhateverGreen.kext");
    }

    #[test]
    fn board_id_skip_enables_sbvmm_before_sonoma() {
        let p = z390();
        let plan = run_with(&p, &options(Ventura), "iMac19,1", |plan| plan.smbios.board_id_skip = true);
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=sbvmm"));
    }

    #[test]
    fn desktop_input_only_with_ps2_keyboard() {
        let mut p = z390();
        let plan = run(&p, Sequoia, "iMac19,1");
        assert!(!has_catalog(&plan, "VoodooPS2Controller"));
        p.input.keyboard_bus = InputBus::Ps2;
        let plan = run(&p, Sequoia, "iMac19,1");
        let ps2 = find(&plan, "VoodooPS2Controller", "VoodooPS2Controller.kext");
        assert!(plugin(ps2, "VoodooPS2Keyboard.kext").enabled);
        assert!(plugin(ps2, "VoodooPS2Mouse.kext").enabled);
        assert!(!plugin(ps2, "VoodooPS2Trackpad.kext").enabled);
        assert!(!plugin(ps2, "VoodooInput.kext").enabled);
    }

    // ── Laptops ─────────────────────────────────────────────────────────────

    fn kblr_laptop() -> HardwareProfile {
        let mut p = machine(P::KabyLake, FormFactor::Laptop, None, "Dell Inc.");
        p.input = ProfileInput {
            keyboard_bus: InputBus::Ps2,
            touchpad_bus: Some(InputBus::I2c),
            touchpad_vendor: Some(TouchpadVendor::Elan),
            touchpad_hid: Some("ELAN0662".into()),
            has_touchscreen: false,
        };
        p.wifi = Some(pci("8086", "9df0", Some(WIFI_PATH)));
        p.bluetooth = Some(usb_dev("8087", "0aaa"));
        p.audio = Some(codec(ALC256));
        p
    }

    #[test]
    fn kbl_r_laptop_elan_i2c_9560_alc256_ventura() {
        let plan = run(&kblr_laptop(), Ventura, "MacBookPro14,1");
        // Sensors and laptop helpers.
        find(&plan, "VirtualSMC", "SMCProcessor.kext");
        assert!(!has_bundle(&plan, "SMCSuperIO.kext"));
        assert!(find(&plan, "VirtualSMC", "SMCBatteryManager.kext").required);
        assert!(!find(&plan, "ECEnabler", "ECEnabler.kext").required);
        find(&plan, "VirtualSMC", "SMCLightSensor.kext");
        find(&plan, "BrightnessKeys", "BrightnessKeys.kext");
        find(&plan, "VirtualSMC", "SMCDellSensors.kext");
        // Input: PS/2 keyboard, I2C HID touchpad owns VoodooInput.
        let ps2 = find(&plan, "VoodooPS2Controller", "VoodooPS2Controller.kext");
        assert!(ps2.required);
        assert!(!plugin(ps2, "VoodooInput.kext").enabled);
        for p in ["VoodooPS2Keyboard.kext", "VoodooPS2Trackpad.kext", "VoodooPS2Mouse.kext"] {
            assert!(plugin(ps2, p).enabled, "{p}");
        }
        let i2c = find(&plan, "VoodooI2C", "VoodooI2C.kext");
        for p in ["VoodooGPIO.kext", "VoodooI2CServices.kext", "VoodooInput.kext"] {
            assert!(plugin(i2c, p).enabled, "{p}");
        }
        let hid = find(&plan, "VoodooI2C", "VoodooI2CHID.kext");
        assert!(hid.plugins.is_empty(), "VoodooI2CHID is a top-level bundle");
        // Wi-Fi: per-release AirportItlwm with its kernel range.
        let airport = find(&plan, "AirportItlwm-Ventura", "AirportItlwm.kext");
        assert_eq!(range(airport), (Some("22.0.0"), Some("22.99.99")));
        assert!(airport.required, "only network path in Recovery");
        assert!(!has_catalog(&plan, "itlwm"));
        assert!(!network_warning(&plan));
        // Audio with the Dell vendor preference.
        let layout = codec_db::default_layout(ALC256, Some(0x1028_0000), true).unwrap();
        assert_eq!(prop(&plan, HDA_100, "layout-id"), Some(&data(&layout.to_le_bytes())));
        assert!(plan.post_install.iter().any(|n| n.component == "audio"));
        // Ventura: Intel BT with BlueToolFixup.
        find(&plan, "BrcmPatchRAM", "BlueToolFixup.kext");
        assert!(!has_catalog(&plan, "CpuTscSync") && !has_catalog(&plan, "ForgedInvariant"));
    }

    #[test]
    fn kbl_r_laptop_sonoma_falls_back_to_itlwm() {
        // SecureBootModel is Disabled from 14.4 on, so AirportItlwm cannot load.
        let plan = run(&kblr_laptop(), Sonoma, "MacBookPro15,2");
        find(&plan, "itlwm", "itlwm.kext");
        assert!(!plan.kexts.iter().any(|k| k.catalog_id.starts_with("AirportItlwm")));
        assert!(network_warning(&plan), "laptop has no Ethernet and itlwm does not work in Recovery");
    }

    #[test]
    fn intel_wifi_sonoma_with_secure_boot_uses_14_4_build() {
        let p = kblr_laptop();
        let plan =
            run_with(&p, &options(Sonoma), "MacBookPro15,2", |plan| plan.smbios.secure_boot_model = "Default".into());
        let airport = find(&plan, "AirportItlwm-Sonoma14.4", "AirportItlwm.kext");
        assert_eq!(range(airport), (Some("23.4.0"), Some("23.99.99")));
    }

    #[test]
    fn explicit_airportitlwm_without_secure_boot_falls_back() {
        let p = kblr_laptop();
        let mut opts = options(Ventura);
        opts.intel_wifi = IntelWifiStrategy::AirportItlwm;
        let plan = run_with(&p, &opts, "MacBookPro14,1", |plan| plan.smbios.secure_boot_model = "Disabled".into());
        find(&plan, "itlwm", "itlwm.kext");
        assert!(has_note(&plan, "wifi", "Secure Boot"));
    }

    #[test]
    fn explicit_airportitlwm_on_sequoia_uses_legacy_stack() {
        let p = kblr_laptop();
        let mut opts = options(Sequoia);
        opts.intel_wifi = IntelWifiStrategy::AirportItlwm;
        let plan = run_with(&p, &opts, "MacBookPro16,2", |_| {});
        let airport = find(&plan, "AirportItlwm-Ventura", "AirportItlwm.kext");
        assert_eq!(range(airport), (Some("24.0.0"), Some("24.99.99")));
        assert_eq!(range(find(&plan, "IOSkywalkFamily", "IOSkywalkFamily.kext")), (Some("24.0.0"), None));
        let legacy = find(&plan, "IO80211FamilyLegacy", "IO80211FamilyLegacy.kext");
        assert!(!plugin(legacy, "AirPortBrcmNIC.kext").enabled, "Broadcom plugin off for Intel cards");
        let block = plan.kernel_blocks.iter().find(|b| b.identifier == "com.apple.iokit.IOSkywalkFamily").unwrap();
        assert_eq!((block.strategy.as_str(), block.min_kernel.as_str()), ("Exclude", "24.0.0"));
        assert!(find(&plan, AMFIPASS_ID, "AMFIPass.kext").required);
        assert!(plans_root_patch(&plan));
        assert!(arg(&plan, "-amfipassbeta").is_none());
    }

    #[test]
    fn intel_wifi_can_be_left_out() {
        let p = kblr_laptop();
        let mut opts = options(Ventura);
        opts.intel_wifi = IntelWifiStrategy::None;
        let plan = run_with(&p, &opts, "MacBookPro14,1", |_| {});
        assert!(!has_catalog(&plan, "itlwm"));
        assert!(!plan.kexts.iter().any(|k| k.bundle == "AirportItlwm.kext"));
    }

    #[test]
    fn intel_bluetooth_big_sur_uses_injector() {
        let plan = run(&kblr_laptop(), BigSur, "MacBookPro14,1");
        assert_eq!(
            range(find(&plan, "IntelBluetoothFirmware", "IntelBluetoothInjector.kext")),
            (None, Some("20.99.99"))
        );
        find(&plan, "IntelBluetoothFirmware", "IntelBTPatcher.kext");
        assert!(!has_bundle(&plan, "BlueToolFixup.kext"));
        assert!(plan.nvram_add.is_empty());
        assert_eq!(range(find(&plan, "AirportItlwm-BigSur", "AirportItlwm.kext")), (Some("20.0.0"), Some("20.99.99")));
    }

    fn touchpad_laptop(bus: InputBus, vendor: TouchpadVendor, hid: Option<&str>) -> HardwareProfile {
        let mut p = machine(P::CoffeeLake, FormFactor::Laptop, None, "LENOVO");
        p.input = ProfileInput {
            keyboard_bus: InputBus::Ps2,
            touchpad_bus: Some(bus),
            touchpad_vendor: Some(vendor),
            touchpad_hid: hid.map(str::to_string),
            has_touchscreen: false,
        };
        p
    }

    #[test]
    fn synaptics_smbus_uses_voodoormi() {
        let plan = run(&touchpad_laptop(InputBus::Smbus, TouchpadVendor::Synaptics, None), Monterey, "MacBookPro15,2");
        let rmi = find(&plan, "VoodooRMI", "VoodooRMI.kext");
        assert!(plugin(rmi, "VoodooInput.kext").enabled);
        assert!(plugin(rmi, "RMISMBus.kext").enabled);
        assert!(!plugin(rmi, "RMII2C.kext").enabled);
        find(&plan, "VoodooRMI", "VoodooSMBus.kext");
        let ps2 = find(&plan, "VoodooPS2Controller", "VoodooPS2Controller.kext");
        assert!(!plugin(ps2, "VoodooInput.kext").enabled);
        assert!(plugin(ps2, "VoodooPS2Mouse.kext").enabled && plugin(ps2, "VoodooPS2Trackpad.kext").enabled);
        assert!(!has_catalog(&plan, "VoodooI2C"));
    }

    #[test]
    fn synaptics_i2c_uses_rmi_over_voodooi2c() {
        let plan = run(
            &touchpad_laptop(InputBus::I2c, TouchpadVendor::Synaptics, Some("SYNA2B33")),
            Monterey,
            "MacBookPro15,2",
        );
        let rmi = find(&plan, "VoodooRMI", "VoodooRMI.kext");
        assert!(plugin(rmi, "RMII2C.kext").enabled && !plugin(rmi, "RMISMBus.kext").enabled);
        let i2c = find(&plan, "VoodooI2C", "VoodooI2C.kext");
        assert!(!plugin(i2c, "VoodooInput.kext").enabled);
        assert!(plugin(i2c, "VoodooGPIO.kext").enabled);
        assert!(!has_bundle(&plan, "VoodooI2CHID.kext"));
        assert!(!has_bundle(&plan, "VoodooSMBus.kext"));
    }

    #[test]
    fn elan_smbus_uses_standalone_voodoosmbus() {
        let plan = run(&touchpad_laptop(InputBus::Smbus, TouchpadVendor::Elan, None), Monterey, "MacBookPro15,2");
        let smbus = find(&plan, "VoodooSMBus", "VoodooSMBus.kext");
        assert!(!smbus.required);
        assert_eq!(range(smbus), (Some("18.0.0"), None));
        let ps2 = find(&plan, "VoodooPS2Controller", "VoodooPS2Controller.kext");
        assert!(plugin(ps2, "VoodooInput.kext").enabled);
        assert!(!plugin(ps2, "VoodooPS2Mouse.kext").enabled);
    }

    #[test]
    fn alps_uses_its_own_voodooi2chid() {
        let plan =
            run(&touchpad_laptop(InputBus::I2c, TouchpadVendor::Alps, Some("ALPS0001")), Ventura, "MacBookPro15,2");
        find(&plan, "AlpsHID", "AlpsHID.kext");
        find(&plan, "AlpsHID", "VoodooI2CHID.kext");
        assert!(!plan.kexts.iter().any(|k| k.catalog_id == "VoodooI2C" && k.bundle == "VoodooI2CHID.kext"));
        assert!(plugin(find(&plan, "VoodooI2C", "VoodooI2C.kext"), "VoodooInput.kext").enabled);
    }

    #[test]
    fn ps2_touchpad_keeps_voodooinput_in_voodoops2() {
        let plan = run(
            &touchpad_laptop(InputBus::Ps2, TouchpadVendor::Synaptics, Some("SYN1234")),
            Catalina,
            "MacBookPro15,2",
        );
        let ps2 = find(&plan, "VoodooPS2Controller", "VoodooPS2Controller.kext");
        assert!(ps2.plugins.iter().all(|p| p.enabled));
        assert!(!has_catalog(&plan, "VoodooI2C") && !has_catalog(&plan, "VoodooRMI"));
    }

    #[test]
    fn amd_renoir_laptop() {
        let mut p = machine(P::AmdZen2, FormFactor::Laptop, None, "HP");
        p.gpus = vec![ProfileGpu {
            is_igpu: true,
            ..gpu("AMD Radeon Graphics", GpuVendor::Amd, GpuFamily::AmdApuVega, "1002", "1636")
        }];
        p.input = ProfileInput {
            keyboard_bus: InputBus::Ps2,
            touchpad_bus: Some(InputBus::I2c),
            touchpad_vendor: Some(TouchpadVendor::Elan),
            touchpad_hid: Some("ELAN0712".into()),
            has_touchscreen: false,
        };
        p.audio = Some(codec(ALC256));
        let plan = run(&p, Sonoma, "MacBookPro16,2");
        assert!(!find(&plan, "ForgedInvariant", "ForgedInvariant.kext").required);
        let gux = find(&plan, "GenericUSBXHCI", "GenericUSBXHCI.kext");
        assert_eq!(range(gux), (Some("20.0.0"), None));
        assert!(find(&plan, "AMDRyzenCPUPowerManagement", "AMDRyzenCPUPowerManagement.kext").enabled);
        find(&plan, "VirtualSMC", "SMCBatteryManager.kext");
        assert!(arg(&plan, "alcid=").is_some());
        find(&plan, "VoodooI2C", "VoodooI2CHID.kext");
        assert!(!has_catalog(&plan, "AppleMCEReporterDisabler"), "MacBookPro16,2 board-id is not matched");
        assert!(!has_catalog(&plan, "CpuTscSync"));
    }

    // ── AMD desktop extras ──────────────────────────────────────────────────

    #[test]
    fn zen4_power_management_ships_disabled() {
        let p = machine(P::AmdZen4, FormFactor::Desktop, Some("B650"), "ASUS");
        let plan = run(&p, Sequoia, "MacPro7,1");
        let pm = find(&plan, "AMDRyzenCPUPowerManagement", "AMDRyzenCPUPowerManagement.kext");
        let smc = find(&plan, "SMCAMDProcessor", "SMCAMDProcessor.kext");
        assert!(!pm.enabled && !smc.enabled);
        assert!(!has_catalog(&plan, "GenericUSBXHCI"));
    }

    #[test]
    fn amd_fx_gets_xlnc_usb_fix_and_no_zen_kexts() {
        let mut p = machine(P::AmdBulldozer, FormFactor::Desktop, Some("990FX"), "ASUS");
        p.cpu.has_avx2 = Some(false);
        let plan = run(&p, Monterey, "MacPro6,1");
        find(&plan, "XLNCUSBFix", "XLNCUSBFix.kext");
        assert!(!has_catalog(&plan, "AMDRyzenCPUPowerManagement"));
        find(&plan, "AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext");
    }

    // ── Broadcom Wi-Fi and Bluetooth ────────────────────────────────────────

    fn bcm94360cd(target_has_ethernet: bool) -> HardwareProfile {
        let mut p = machine(P::CometLake, FormFactor::Desktop, Some("Z490"), "Gigabyte");
        p.wifi = Some(ProfileNic { subsystem_id: Some("106b0111".into()), ..pci("14e4", "43a0", Some(WIFI_PATH)) });
        p.bluetooth = Some(usb_dev("05ac", "828d"));
        if target_has_ethernet {
            p.ethernet = vec![pci("8086", "0d4f", Some(NIC_PATH))];
        }
        p
    }

    #[test]
    fn bcm94360cd_ventura_is_native() {
        let plan = run(&bcm94360cd(false), Ventura, "iMac20,1");
        assert!(!has_catalog(&plan, "AirportBrcmFixup"));
        assert!(!has_catalog(&plan, "IOSkywalkFamily"));
        assert!(!has_bundle(&plan, "BlueToolFixup.kext"), "genuine Apple Bluetooth needs nothing");
        assert!(!network_warning(&plan), "native Broadcom works in Recovery");
        assert!(!plans_root_patch(&plan));
    }

    #[test]
    fn bcm94360cd_sonoma_uses_oclp_wireless_stack() {
        let plan = run(&bcm94360cd(true), Sonoma, "iMac20,1");
        assert_eq!(range(find(&plan, "IOSkywalkFamily", "IOSkywalkFamily.kext")), (Some("23.0.0"), None));
        let legacy = find(&plan, "IO80211FamilyLegacy", "IO80211FamilyLegacy.kext");
        let nic = plugin(legacy, "AirPortBrcmNIC.kext");
        assert!(nic.enabled);
        assert_eq!((nic.min_kernel.as_deref(), nic.max_kernel.as_deref()), (Some("23.0.0"), Some("24.99.99")));
        assert!(!has_catalog(&plan, "AirPortBrcmNIC-Tahoe"));
        let block = plan.kernel_blocks.iter().find(|b| b.identifier == "com.apple.iokit.IOSkywalkFamily").unwrap();
        assert_eq!((block.strategy.as_str(), block.min_kernel.as_str(), block.enabled), ("Exclude", "23.0.0", true));
        assert_eq!(range(find(&plan, AMFIPASS_ID, "AMFIPass.kext")), (Some("20.0.0"), None));
        assert!(plans_root_patch(&plan));
        assert!(arg(&plan, "-amfipassbeta").is_none());
        assert!(plan.post_install.iter().any(|n| n.title.contains("Modern Wireless")));
        assert!(!network_warning(&plan), "the I219 covers Recovery");
        let mausi = find(&plan, "IntelMausi", "IntelMausi.kext");
        assert!(mausi.required);
    }

    #[test]
    fn bcm94360cd_tahoe_adds_tahoe_driver_and_beta_flag() {
        let plan = run(&bcm94360cd(false), Tahoe, "iMac20,1");
        assert_eq!(range(find(&plan, "AirPortBrcmNIC-Tahoe", "AirPortBrcmNIC-Tahoe.kext")), (Some("25.0.0"), None));
        assert!(plan.boot_args.iter().any(|a| a == "-amfipassbeta"));
        assert!(network_warning(&plan), "root-patched Wi-Fi does not work in Recovery");
    }

    #[test]
    fn bcm4352_needs_nic_injector_and_4360_injector_stays_off() {
        let mut p = machine(P::Skylake, FormFactor::Laptop, None, "Dell Inc.");
        p.wifi = Some(pci("14e4", "43b1", Some(WIFI_PATH)));
        let plan = run(&p, Monterey, "MacBookPro13,1");
        let fixup = find(&plan, "AirportBrcmFixup", "AirportBrcmFixup.kext");
        let nic = plugin(fixup, "AirPortBrcmNIC_Injector.kext");
        assert!(nic.enabled);
        let old = plugin(fixup, "AirPortBrcm4360_Injector.kext");
        assert!(!old.enabled);
        assert_eq!(old.max_kernel.as_deref(), Some("19.99.99"));
        assert!(!network_warning(&plan));
    }

    #[test]
    fn bcm4350_gets_aspm_property_and_no_injector() {
        let mut p = machine(P::KabyLake, FormFactor::Laptop, None, "Dell Inc.");
        p.wifi = Some(pci("14e4", "43a3", Some(WIFI_PATH)));
        let plan = run(&p, Monterey, "MacBookPro14,1");
        assert_eq!(prop(&plan, WIFI_PATH, "pci-aspm-default"), Some(&data(&[0, 0, 0, 0])));
        assert!(
            !plugin(find(&plan, "AirportBrcmFixup", "AirportBrcmFixup.kext"), "AirPortBrcmNIC_Injector.kext").enabled
        );
    }

    #[test]
    fn legacy_broadcom_stops_at_its_last_native_release() {
        let mut p = machine(P::Haswell, FormFactor::Laptop, None, "HP");
        p.wifi = Some(pci("14e4", "4357", Some(WIFI_PATH)));
        let catalina = run(&p, Catalina, "MacBookPro11,1");
        let fixup = find(&catalina, "AirportBrcmFixup", "AirportBrcmFixup.kext");
        assert!(plugin(fixup, "AirPortBrcm4360_Injector.kext").enabled);
        assert!(!plugin(fixup, "AirPortBrcmNIC_Injector.kext").enabled);
        let big_sur = run(&p, BigSur, "MacBookPro11,4");
        assert!(!has_catalog(&big_sur, "AirportBrcmFixup"));
        assert!(has_note(&big_sur, "wifi", "not configured"));
    }

    #[test]
    fn broadcom_bluetooth_per_release() {
        let mut p = machine(P::CoffeeLake, FormFactor::Laptop, None, "Lenovo");
        p.bluetooth = Some(usb_dev("0a5c", "21e8"));
        let mojave = run(&p, Mojave, "MacBookPro15,2");
        find(&mojave, "BrcmPatchRAM", "BrcmFirmwareData.kext");
        assert_eq!(range(find(&mojave, "BrcmPatchRAM", "BrcmPatchRAM2.kext")), (None, Some("18.99.99")));
        assert!(!has_bundle(&mojave, "BrcmBluetoothInjector.kext") && !has_bundle(&mojave, "BrcmPatchRAM3.kext"));

        let catalina = run(&p, Catalina, "MacBookPro15,2");
        let order: Vec<&str> =
            catalina.kexts.iter().filter(|k| k.catalog_id == "BrcmPatchRAM").map(|k| k.bundle.as_str()).collect();
        assert_eq!(order, ["BrcmBluetoothInjector.kext", "BrcmFirmwareData.kext", "BrcmPatchRAM3.kext"]);
        assert_eq!(range(find(&catalina, "BrcmPatchRAM", "BrcmBluetoothInjector.kext")), (None, Some("20.99.99")));
        assert_eq!(range(find(&catalina, "BrcmPatchRAM", "BrcmPatchRAM3.kext")), (Some("19.0.0"), None));

        let monterey = run(&p, Monterey, "MacBookPro15,2");
        assert!(!has_bundle(&monterey, "BrcmBluetoothInjector.kext"));
        find(&monterey, "BrcmPatchRAM", "BrcmPatchRAM3.kext");
        find(&monterey, "BrcmPatchRAM", "BlueToolFixup.kext");
        assert_eq!(monterey.nvram_add.len(), 2);
    }

    #[test]
    fn realtek_bluetooth_is_optional_and_experimental() {
        let mut p = machine(P::CometLake, FormFactor::Laptop, None, "ASUS");
        p.bluetooth = Some(usb_dev("0bda", "b00c"));
        let plan = run(&p, Sequoia, "MacBookPro16,2");
        assert!(!find(&plan, "RealtekBluetoothFirmware", "RealtekBluetoothFirmware.kext").required);
        find(&plan, "BrcmPatchRAM", "BlueToolFixup.kext");
        assert!(has_note(&plan, "bluetooth", "experimental"));
    }

    // ── VMs ─────────────────────────────────────────────────────────────────

    #[test]
    fn hyperv_vm_gets_support_kext_and_no_bare_metal_extras() {
        let mut p = machine(P::CometLake, FormFactor::Desktop, None, "Microsoft Corporation");
        p.vm = Some(VmKind::HyperV);
        p.input.keyboard_bus = InputBus::Ps2;
        let plan = run(&p, Sequoia, "iMac20,1");
        assert_eq!(range(find(&plan, "MacHyperVSupport", "MacHyperVSupportMonterey.kext")), (Some("21.0.0"), None));
        assert!(!has_bundle(&plan, "MacHyperVSupport.kext"));
        for absent in ["USBToolBox", "VoodooPS2Controller", "CpuTscSync"] {
            assert!(!has_catalog(&plan, absent), "{absent}");
        }
        assert!(!has_bundle(&plan, "SMCProcessor.kext"));
        assert!(!network_warning(&plan));

        let plan = run(&p, BigSur, "iMac20,1");
        assert_eq!(range(find(&plan, "MacHyperVSupport", "MacHyperVSupport.kext")), (None, Some("20.99.99")));
    }

    #[test]
    fn kvm_vmxnet3_counts_as_network() {
        let mut p = machine(P::Skylake, FormFactor::Desktop, None, "QEMU");
        p.vm = Some(VmKind::Kvm);
        p.ethernet = vec![pci("15ad", "07b0", Some("PciRoot(0x0)/Pci(0x3,0x0)"))];
        let plan = run(&p, Sonoma, "iMac20,1");
        assert!(!network_warning(&plan));
        assert!(!has_catalog(&plan, "USBToolBox"));
        assert_eq!(prop(&plan, "PciRoot(0x0)/Pci(0x3,0x0)", "built-in"), Some(&data(&[1])));
    }

    // ── GPU root patches, storage, audio edge cases ─────────────────────────

    #[test]
    fn kepler_beyond_big_sur_plans_root_patch() {
        let mut p = machine(P::Haswell, FormFactor::Desktop, Some("Z97"), "ASUS");
        p.gpus = vec![gpu("GeForce GTX 770", GpuVendor::Nvidia, GpuFamily::NvidiaKepler, "10de", "1184")];
        let big_sur = run(&p, BigSur, "iMac15,1");
        assert!(!plans_root_patch(&big_sur));
        let monterey = run(&p, Monterey, "iMac15,1");
        assert!(find(&monterey, AMFIPASS_ID, "AMFIPass.kext").reason.contains("GTX 770"));
        assert!(plans_root_patch(&monterey));
    }

    #[test]
    fn storage_notes_and_kexts() {
        let mut p = machine(P::KabyLake, FormFactor::Laptop, None, "Lenovo");
        p.storage = vec![
            ProfileStorage {
                name: "SAMSUNG MZVLB512HAJQ".into(),
                kind: StorageKind::Nvme,
                vendor_id: Some("144d".into()),
                device_id: Some("a808".into()),
                size_bytes: None,
            },
            ProfileStorage {
                name: "Intel SATA AHCI".into(),
                kind: StorageKind::Sata,
                vendor_id: Some("8086".into()),
                device_id: Some("9d03".into()),
                size_bytes: None,
            },
        ];
        let plan = run(&p, BigSur, "MacBookPro14,1");
        let nvme = find(&plan, "NVMeFix", "NVMeFix.kext");
        assert!(!nvme.required);
        assert_eq!(range(nvme), (Some("18.0.0"), None));
        assert_eq!(range(find(&plan, "CtlnaAHCIPort", "CtlnaAHCIPort.kext")), (Some("20.0.0"), None));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.component == "storage" && n.level == NoteLevel::Warning && n.detail.contains("PM981")));
        let catalina = run(&p, Catalina, "MacBookPro14,1");
        assert!(!has_catalog(&catalina, "CtlnaAHCIPort"));
        assert!(has_note(&catalina, "storage", "SATA-unsupported"));
    }

    #[test]
    fn unsupported_and_hdmi_codecs_get_no_applealc() {
        let mut p = z390();
        p.audio = Some(ProfileAudio { codec_name: "Mystery codec".into(), ..Default::default() });
        let plan = run(&p, Ventura, "iMac19,1");
        assert!(!has_catalog(&plan, "AppleALC"));
        assert!(has_note(&plan, "audio", "Unknown audio codec"));

        p.audio = Some(codec(0x8086_2807));
        let plan = run(&p, Ventura, "iMac19,1");
        assert!(!has_catalog(&plan, "AppleALC"));
        assert!(has_note(&plan, "audio", "digital codec"));
    }

    #[test]
    fn layout_and_controller_path_overrides_win() {
        let mut p = machine(P::AmdZen3, FormFactor::Desktop, Some("X570"), "ASUS");
        let path = "PciRoot(0x0)/Pci(0x8,0x1)/Pci(0x0,0x4)";
        p.audio = Some(ProfileAudio { layout_id: Some(11), controller_pci_path: Some(path.into()), ..codec(ALC1220) });
        let plan = run(&p, Monterey, "MacPro7,1");
        assert_eq!(prop(&plan, path, "layout-id"), Some(&data(&[11, 0, 0, 0])));
        assert!(arg(&plan, "alcid=").is_none());
        assert!(find(&plan, "AppleALC", "AppleALC.kext").reason.contains("layout-id 11"));
    }

    #[test]
    fn old_intel_pch_uses_1b_path() {
        let mut p = machine(P::Haswell, FormFactor::Desktop, Some("Z97"), "ASUS");
        p.audio = Some(codec(ALC1220));
        let plan = run(&p, Monterey, "iMac15,1");
        assert!(prop(&plan, "PciRoot(0x0)/Pci(0x1b,0x0)", "layout-id").is_some());
    }

    #[test]
    fn missing_nic_path_blocks_spoof_dependent_nic() {
        let mut p = z490_i225v();
        p.ethernet = vec![pci("8086", "15f3", None)];
        let plan = run(&p, Monterey, "iMac20,1");
        assert!(network_warning(&plan));
        assert!(has_note(&plan, "ethernet", "PCI path"));
    }

    #[test]
    fn desktop_chipset_from_a_laptop_model_name_is_ignored() {
        let mut p = machine(P::CoffeeLake, FormFactor::Laptop, None, "ASUS");
        p.motherboard_model = "ROG H370 Edition".into();
        p.audio = Some(codec(ALC256));
        let plan = run(&p, Monterey, "MacBookPro15,1");
        assert!(!has_catalog(&plan, "XHCI-unsupported"));
        // A chipset the profile names explicitly is trusted.
        p.chipset = Some("H370".into());
        let plan = run(&p, Monterey, "MacBookPro15,1");
        find(&plan, "XHCI-unsupported", "XHCI-unsupported.kext");
        assert!(prop(&plan, HDA_100, "layout-id").is_some());
    }

    #[test]
    fn helper_merges_list_args() {
        let mut plan = empty_plan(Sequoia);
        merge_list_arg(&mut plan, "revpatch", &["sbvmm"]);
        merge_list_arg(&mut plan, "revpatch", &["sbvmm", "pci"]);
        assert_eq!(plan.boot_args, ["revpatch=sbvmm,pci"]);
        assert_eq!(oem_subsystem("HP"), Some(0x103C_0000));
        assert_eq!(oem_subsystem("Hewlett-Packard"), Some(0x103C_0000));
        assert_eq!(oem_subsystem("Shopping Co"), None);
        assert_eq!(oem_subsystem("Micro-Star International"), Some(0x1462_0000));
    }

    // ── Release floors, RestrictEvents, light sensor, touchscreen ──────────

    #[test]
    fn core_kexts_move_to_the_front_without_duplicates() {
        let plan = run_with(&z390(), &options(Sequoia), "iMac19,1", |plan| {
            plan.kexts.push(kext("WhateverGreen", "WhateverGreen.kext", "GPU"));
            plan.kexts.push(kext("Lilu", "Lilu.kext", "added early"));
        });
        let order: Vec<&str> = plan.kexts.iter().take(3).map(|k| k.bundle.as_str()).collect();
        assert_eq!(order, ["Lilu.kext", "VirtualSMC.kext", "WhateverGreen.kext"]);
        assert_eq!(plan.kexts.iter().filter(|k| k.bundle == "Lilu.kext").count(), 1);
    }

    #[test]
    fn revpatch_values_follow_the_restrictevents_readme() {
        // memtab belongs to MacBookAir / MacBookPro10,x, pci to MacPro7,1.
        let mut air = machine(P::IceLake, FormFactor::Laptop, None, "Generic");
        air.gpus = vec![ProfileGpu {
            is_igpu: true,
            ..gpu("Iris Plus Graphics", GpuVendor::Intel, GpuFamily::IntelIceLake, "8086", "8a52")
        }];
        let plan = run(&air, Sequoia, "MacBookAir9,1");
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=sbvmm,memtab"));
        // Before 14 nothing replaces `auto`, so no revpatch at all.
        let plan = run(&air, Ventura, "MacBookAir9,1");
        assert!(arg(&plan, "revpatch=").is_none() && !has_catalog(&plan, "RestrictEvents"));
        // A revpatch an earlier stage wrote replaces `auto` as well.
        let plan = run_with(&air, &options(Ventura), "MacBookAir9,1", |plan| plan.boot_args.push("revpatch=asset".into()));
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=asset,memtab"));
        find(&plan, "RestrictEvents", "RestrictEvents.kext");

        let mut x299 = machine(P::CascadeLakeX, FormFactor::Desktop, Some("X299"), "ASUS");
        x299.ethernet = vec![pci("8086", "1533", Some(NIC_PATH))];
        let plan = run(&x299, Ventura, "MacPro7,1");
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=pci"));
        assert!(arg(&plan, "revcpu=").is_none(), "Cascade Lake runs without a CPUID spoof");
    }

    #[test]
    fn sbvmm_also_when_secure_boot_is_off_from_monterey() {
        let plan = run_with(&z390(), &options(Monterey), "iMac19,1", |plan| {
            plan.smbios.secure_boot_model = "Disabled".into();
        });
        assert_eq!(arg(&plan, "revpatch="), Some("revpatch=sbvmm"));
        let plan = run_with(&z390(), &options(BigSur), "iMac19,1", |plan| {
            plan.smbios.secure_boot_model = "Disabled".into();
        });
        assert!(arg(&plan, "revpatch=").is_none());
    }

    #[test]
    fn earlier_restrictevents_boot_args_pull_in_the_kext() {
        let plan = run_with(&z390(), &options(Ventura), "iMac19,1", |plan| {
            plan.boot_args.push("revblock=media".into());
        });
        assert!(!find(&plan, "RestrictEvents", "RestrictEvents.kext").required);
        assert!(arg(&plan, "revpatch=").is_none(), "nothing to add to auto");
        assert_eq!(arg(&plan, "revblock="), Some("revblock=media"));
    }

    #[test]
    fn polaris_on_a_cpu_without_avx2_needs_a_root_patch_from_ventura() {
        let mut p = machine(P::IvyBridge, FormFactor::Desktop, Some("Z77"), "ASUS");
        p.cpu.has_avx2 = Some(false);
        p.gpus = vec![gpu("Radeon RX 580", GpuVendor::Amd, GpuFamily::AmdPolaris, "1002", "67df")];
        let monterey = run(&p, Monterey, "MacPro6,1");
        assert!(!plans_root_patch(&monterey));
        let ventura = run(&p, Ventura, "MacPro6,1");
        assert!(find(&ventura, AMFIPASS_ID, "AMFIPass.kext").reason.contains("RX 580"));
        assert!(arg(&ventura, "-amfipassbeta").is_none());
        // A Navi card has no root-patch path without AVX2.
        p.gpus = vec![gpu("Radeon RX 5700 XT", GpuVendor::Amd, GpuFamily::AmdNavi10, "1002", "731f")];
        assert!(!plans_root_patch(&run(&p, Ventura, "MacPro6,1")));
    }

    #[test]
    fn i211_class_kexts_respect_their_release_floors() {
        let mut p = machine(P::CoffeeLake, FormFactor::Desktop, Some("Z370"), "ASUS");
        p.ethernet = vec![pci("8086", "1539", Some(NIC_PATH))];
        let mojave = run(&p, Mojave, "iMac19,1");
        assert!(!has_catalog(&mojave, "SmallTreeIntel82576") && !has_catalog(&mojave, "AppleIGB"));
        assert!(has_note(&mojave, "ethernet", "1.2.5"));
        assert!(network_warning(&mojave));
        // 82580: only AppleIGB, which needs macOS 12.
        p.ethernet = vec![pci("8086", "150e", Some(NIC_PATH))];
        let big_sur = run(&p, BigSur, "iMac19,1");
        assert!(!has_catalog(&big_sur, "AppleIGB") && network_warning(&big_sur));
        let monterey = run(&p, Monterey, "iMac19,1");
        assert_eq!(range(find(&monterey, "AppleIGB", "AppleIGB.kext")), (Some("21.0.0"), None));
    }

    #[test]
    fn mieze_only_i219_needs_catalina() {
        let mut p = machine(P::CoffeeLake, FormFactor::Desktop, Some("Z390"), "MSI");
        p.ethernet = vec![pci("8086", "0dc6", Some(NIC_PATH))];
        let mojave = run(&p, Mojave, "iMac19,1");
        assert!(!has_catalog(&mojave, "IntelMausiEthernet") && network_warning(&mojave));
        let catalina = run(&p, Catalina, "iMac19,1");
        assert_eq!(range(find(&catalina, "IntelMausiEthernet", "IntelMausiEthernet.kext")), (Some("19.0.0"), None));
    }

    #[test]
    fn light_sensor_only_where_an_als_exists() {
        let laptop = kblr_laptop();
        assert!(!has_bundle(&run(&laptop, Mojave, "MacBookPro14,1"), "SMCLightSensor.kext"), "no ALS0 before 10.15");
        assert!(has_bundle(&run(&laptop, Catalina, "MacBookPro14,1"), "SMCLightSensor.kext"));
        assert!(!has_bundle(&run(&z390(), Catalina, "iMac19,1"), "SMCLightSensor.kext"), "desktop");

        // An all-in-one panel on a supported iGPU gets one too.
        let mut aio = machine(P::CoffeeLake, FormFactor::AllInOne, Some("H370"), "Lenovo");
        aio.gpus = vec![ProfileGpu {
            is_igpu: true,
            ..gpu("UHD Graphics 630", GpuVendor::Intel, GpuFamily::IntelCoffeeLake, "8086", "3e92")
        }];
        assert!(has_bundle(&run(&aio, Ventura, "iMac19,1"), "SMCLightSensor.kext"));

        // A disabled iGPU means no PNLF/ALS0, so no light sensor kext.
        let display = DisplayPlan { primary: None, igpu: None, igpu_headless: false, disabled: vec![0] };
        let plan = run_display(&aio, &options(Ventura), "iMac19,1", &display, |_| {});
        assert!(!has_bundle(&plan, "SMCLightSensor.kext"));
    }

    #[test]
    fn patched_only_atheros_cards_are_not_native() {
        let mut p = machine(P::Haswell, FormFactor::Laptop, None, "Acer");
        p.wifi = Some(pci("168c", "0036", Some(WIFI_PATH)));
        let plan = run(&p, HighSierra, "MacBookPro11,1");
        assert!(network_warning(&plan));
        assert!(has_note(&plan, "wifi", "not configured"));
        // AR9285 works natively on 10.13 with the AR928X spoof.
        p.wifi = Some(pci("168c", "002b", Some(WIFI_PATH)));
        let plan = run(&p, HighSierra, "MacBookPro11,1");
        assert!(!network_warning(&plan));
        assert_eq!(prop(&plan, WIFI_PATH, "device-id"), Some(&data(&[0x2A, 0, 0, 0])));
    }

    #[test]
    fn touchscreens_get_voodooi2chid() {
        let mut p = touchpad_laptop(InputBus::Ps2, TouchpadVendor::Synaptics, Some("SYN1234"));
        p.input.has_touchscreen = true;
        let plan = run(&p, Ventura, "MacBookPro15,2");
        let i2c = find(&plan, "VoodooI2C", "VoodooI2C.kext");
        assert!(!i2c.required && !plugin(i2c, "VoodooInput.kext").enabled, "VoodooPS2 owns VoodooInput");
        assert!(!find(&plan, "VoodooI2C", "VoodooI2CHID.kext").required);
        assert!(plugin(find(&plan, "VoodooPS2Controller", "VoodooPS2Controller.kext"), "VoodooInput.kext").enabled);

        // Synaptics over I2C: VoodooRMI's README keeps VoodooI2CHID out.
        let mut p = touchpad_laptop(InputBus::I2c, TouchpadVendor::Synaptics, Some("SYNA2B33"));
        p.input.has_touchscreen = true;
        assert!(!has_bundle(&run(&p, Ventura, "MacBookPro15,2"), "VoodooI2CHID.kext"));

        // All-in-one without PS/2: the touchscreen stack brings its own VoodooInput.
        let mut aio = machine(P::CometLake, FormFactor::AllInOne, Some("H410"), "HP");
        aio.input.has_touchscreen = true;
        aio.input.keyboard_bus = InputBus::Usb;
        let plan = run(&aio, Sonoma, "iMac20,1");
        assert!(!has_catalog(&plan, "VoodooPS2Controller"));
        assert!(plugin(find(&plan, "VoodooI2C", "VoodooI2C.kext"), "VoodooInput.kext").enabled);
    }

    #[test]
    fn genericusbxhci_for_every_vega_apu_laptop_from_big_sur() {
        let mut p = machine(P::AmdZen, FormFactor::Laptop, None, "Lenovo");
        p.gpus = vec![ProfileGpu {
            is_igpu: true,
            ..gpu("Radeon Vega 8", GpuVendor::Amd, GpuFamily::AmdApuVega, "1002", "15d8")
        }];
        assert_eq!(
            range(find(&run(&p, BigSur, "MacBookPro16,2"), "GenericUSBXHCI", "GenericUSBXHCI.kext")),
            (Some("20.0.0"), None)
        );
        assert!(!has_catalog(&run(&p, Catalina, "MacBookPro16,2"), "GenericUSBXHCI"));
        p.form_factor = FormFactor::Desktop;
        assert!(!has_catalog(&run(&p, BigSur, "iMac20,1"), "GenericUSBXHCI"), "laptops only");
    }

    #[test]
    fn rst_mode_sata_warns_before_big_sur() {
        let mut p = machine(P::CoffeeLake, FormFactor::Laptop, None, "Dell Inc.");
        p.storage = vec![ProfileStorage {
            name: "Intel RST".into(),
            kind: StorageKind::Sata,
            vendor_id: Some("8086".into()),
            device_id: Some("282a".into()),
            size_bytes: None,
        }];
        let plan = run(&p, Catalina, "MacBookPro15,2");
        assert!(plan.notes.iter().any(|n| n.level == NoteLevel::Warning && n.title.contains("SATA-unsupported")));
        p.storage[0].device_id = Some("a353".into());
        let plan = run(&p, Catalina, "MacBookPro15,2");
        assert!(plan.notes.iter().any(|n| n.component == "storage" && n.level == NoteLevel::Info));
        assert!(!plan.notes.iter().any(|n| n.component == "storage" && n.level == NoteLevel::Warning));
        find(&run(&p, BigSur, "MacBookPro15,2"), "CtlnaAHCIPort", "CtlnaAHCIPort.kext");
    }

    #[test]
    fn vms_of_every_kind_skip_bare_metal_kexts() {
        let kinds = [VmKind::Kvm, VmKind::Vmware, VmKind::HyperV, VmKind::VirtualBox, VmKind::Parallels, VmKind::Other];
        for platform in [P::Skylake, P::AmdZen3] {
            for kind in kinds {
                for target in MacOsVersion::ALL {
                    let mut p = machine(platform, FormFactor::Desktop, None, "QEMU");
                    p.vm = Some(kind);
                    p.input.keyboard_bus = InputBus::Ps2;
                    p.audio = Some(codec(ALC897));
                    let plan = run(&p, target, "iMacPro1,1");
                    for absent in ["USBToolBox", "VoodooPS2Controller", "CpuTscSync", "ForgedInvariant", "ECEnabler"] {
                        assert!(!has_catalog(&plan, absent), "{absent} {kind:?} {target:?}");
                    }
                    assert!(!plan.kexts.iter().any(|k| k.bundle.starts_with("SMC")), "{kind:?}");
                    assert_eq!(has_catalog(&plan, "MacHyperVSupport"), kind == VmKind::HyperV);
                    assert!(arg(&plan, "alcid=").is_some(), "no fixed HDA path in a VM");
                }
            }
        }
    }

    /// Every supported platform, form factor and release yields a sane plan,
    /// for three sets of typical devices.
    #[test]
    fn every_supported_platform_and_target_is_consistent() {
        for &platform in cpu_db::all_platforms() {
            let info = cpu_db::platform_info(platform);
            if !info.supported {
                continue;
            }
            let ceiling = match (info.max_macos, cpu_db::ceiling_workaround(platform)) {
                (Some(_), Some(w)) => w.max_macos,
                (max, _) => max,
            };
            for form in [FormFactor::Desktop, FormFactor::Laptop, FormFactor::AllInOne, FormFactor::MiniPc] {
                for target in MacOsVersion::ALL {
                    if info.min_macos.is_some_and(|m| target < m) || ceiling.is_some_and(|m| target > m) {
                        continue;
                    }
                    for kit in 0..3 {
                        check_platform_kit(platform, &info, form, target, kit);
                    }
                }
            }
        }
    }

    fn check_platform_kit(platform: P, info: &cpu_db::PlatformInfo, form: FormFactor, target: MacOsVersion, kit: u8) {
        let what = format!("{platform:?} {form:?} {target:?} kit {kit}");
        let mut p = machine(platform, form, None, "Generic");
        p.storage = vec![ProfileStorage {
            name: "WD SN770".into(),
            kind: StorageKind::Nvme,
            vendor_id: Some("15b7".into()),
            device_id: Some("5017".into()),
            size_bytes: None,
        }];
        match kit {
            0 => {
                p.ethernet = vec![pci("8086", "15b8", Some(NIC_PATH))];
                p.wifi = Some(pci("8086", "2723", Some(WIFI_PATH)));
                p.bluetooth = Some(usb_dev("8087", "0029"));
                p.audio = Some(codec(ALC897));
                p.input.touchpad_bus = (form == FormFactor::Laptop).then_some(InputBus::I2c);
            }
            1 => {
                p.ethernet = vec![pci("10ec", "8168", Some(NIC_PATH))];
                p.wifi = Some(pci("14e4", "43a0", Some(WIFI_PATH)));
                p.bluetooth = Some(usb_dev("0a5c", "21e8"));
                p.audio = Some(codec(ALC1220));
                p.input.touchpad_bus = Some(InputBus::Smbus);
                p.input.touchpad_vendor = Some(TouchpadVendor::Synaptics);
                p.input.has_touchscreen = true;
            }
            _ => {
                p.wifi = Some(ProfileNic { subsystem_id: Some("106b0111".into()), ..pci("14e4", "43a0", None) });
                p.bluetooth = Some(usb_dev("05ac", "828d"));
                p.input.keyboard_bus = InputBus::Ps2;
            }
        }
        let plan = run(&p, target, "iMac20,1");

        assert!(has_catalog(&plan, "USBToolBox"), "{what}");
        assert_eq!(has_bundle(&plan, "SMCBatteryManager.kext"), form == FormFactor::Laptop, "{what}");
        assert_eq!(has_bundle(&plan, "SMCProcessor.kext"), info.vendor == CpuVendor::Intel, "{what}");
        assert!(!(info.vendor == CpuVendor::Amd && has_bundle(&plan, "SMCSuperIO.kext")), "{what}");
        assert!(!(info.vendor == CpuVendor::Intel && has_catalog(&plan, "AMDRyzenCPUPowerManagement")), "{what}");
        assert_eq!(has_catalog(&plan, "CryptexFixup"), target >= Ventura && !info.has_avx2, "{what}");
        assert_eq!(has_note(&plan, "cpu", "telemetrap"), platform == P::Penryn && target >= Mojave, "{what}");
        assert_eq!(has_catalog(&plan, "NVMeFix"), target >= Mojave, "{what}");
        if form == FormFactor::Laptop {
            assert!(has_catalog(&plan, "VoodooPS2Controller"), "{what}");
        }
        if target >= Sonoma {
            assert!(arg(&plan, "revpatch=").is_some_and(|a| a.contains("sbvmm")), "{what}");
        }
        if target == Tahoe && kit != 2 {
            assert!(has_catalog(&plan, "AppleALC") && has_note(&plan, "audio", "macOS 26"), "{what}");
        }
        match kit {
            0 => {
                assert!(has_catalog(&plan, "IntelMausi") || has_catalog(&plan, "IntelMausiEthernet"), "{what}");
                let airport = plan.kexts.iter().find(|k| k.bundle == "AirportItlwm.kext");
                // run() leaves SecureBootModel on before 14.
                assert_eq!(airport.is_some(), target <= Ventura, "{what}");
                assert_eq!(has_catalog(&plan, "itlwm"), target >= Sonoma, "{what}");
                assert_eq!(has_bundle(&plan, "BlueToolFixup.kext"), target >= Monterey, "{what}");
                assert_eq!(has_bundle(&plan, "IntelBluetoothInjector.kext"), target <= BigSur, "{what}");
                assert!(!network_warning(&plan), "{what}");
                assert!(!plans_root_patch(&plan), "{what}");
            }
            1 => {
                let rtl = if info.vendor == CpuVendor::Amd { "RealtekRTL8111-2.4.2" } else { "RealtekRTL8111" };
                assert_eq!(has_catalog(&plan, rtl), target >= Mojave, "{what}");
                assert_eq!(has_catalog(&plan, "IOSkywalkFamily"), target >= Sonoma, "{what}");
                assert_eq!(has_catalog(&plan, "AirPortBrcmNIC-Tahoe"), target == Tahoe, "{what}");
                assert_eq!(plans_root_patch(&plan), target >= Sonoma, "{what}");
                assert_eq!(arg(&plan, "-amfipassbeta").is_some(), target == Tahoe, "{what}");
                assert_eq!(has_bundle(&plan, "BrcmPatchRAM3.kext"), target >= Catalina, "{what}");
                assert_eq!(has_bundle(&plan, "BrcmPatchRAM2.kext"), target <= Mojave, "{what}");
                assert_eq!(
                    has_bundle(&plan, "BrcmBluetoothInjector.kext"),
                    (Catalina..=BigSur).contains(&target),
                    "{what}"
                );
                assert!(has_catalog(&plan, "VoodooRMI") && has_bundle(&plan, "VoodooI2CHID.kext"), "{what}");
                // 10.13 has no RTL8111 build, but the Broadcom card works in Recovery.
                assert!(!network_warning(&plan), "{what}");
            }
            _ => {
                assert!(!has_bundle(&plan, "BlueToolFixup.kext"), "{what}");
                assert!(!has_catalog(&plan, "AirportBrcmFixup"), "{what}");
                assert_eq!(network_warning(&plan), target >= Sonoma, "{what}");
                assert!(has_catalog(&plan, "VoodooPS2Controller"), "{what}");
            }
        }
    }
}
