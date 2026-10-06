//! Display path selection, iGPU framebuffer properties, dGPU handling and GPU
//! kexts (WhateverGreen / NootRX / NootedRed).
//!
//! Rules follow the Dortania OpenCore Install Guide (desktop, laptop and AMD
//! config.plist pages, Ventura and Tahoe pages), the Dortania GPU Buyers
//! Guide and Getting-Started-With-ACPI (laptop dGPU, GPU disable), the
//! WhateverGreen README / FAQs and the NootRX / NootedRed release notes as
//! collected in `gpu_db`.

mod igpu;
#[cfg(test)]
mod tests;

use serde_json::json;

use crate::domain::gpu_db::{self, GpuRequirement, GpuSupport};
use crate::domain::model::{
    BinaryPatch, BuildPlan, DeviceProperty, DevicePropertyEntry, FormFactor, GpuFamily, GpuVendor,
    KextSelection, MacOsVersion, NoteLevel, NvramVariable, PlanNote, PlistScalar, ProfileGpu,
};
use crate::error::AppError;

use super::{DisplayPlan, PlanContext};
use igpu::{Role, ONE};

/// OpenCore device path of the Intel iGPU when the scan did not report one.
pub const IGPU_PATH: &str = "PciRoot(0x0)/Pci(0x2,0x0)";
const IMEI_PATH: &str = "PciRoot(0x0)/Pci(0x16,0x0)";
const APPLE_NVRAM_GUID: &str = "7C436110-AB2A-4BBB-A880-FE41995C9F82";

/// The GPU Lilu plugin of a build. They exclude each other: NootRX and
/// NootedRed replace WhateverGreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuKext {
    WhateverGreen,
    NootRx,
    NootedRed,
    None,
}

impl GpuKext {
    /// Catalog id and bundle in `kext_catalog`.
    pub fn catalog(self) -> Option<(&'static str, &'static str)> {
        match self {
            GpuKext::WhateverGreen => Some(("WhateverGreen", "WhateverGreen.kext")),
            GpuKext::NootRx => Some(("NootRX", "NootRX.kext")),
            GpuKext::NootedRed => Some(("NootedRed", "NootedRed.kext")),
            GpuKext::None => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            GpuKext::WhateverGreen => "WhateverGreen",
            GpuKext::NootRx => "NootRX",
            GpuKext::NootedRed => "NootedRed",
            GpuKext::None => "no GPU kext",
        }
    }
}

/// The GPU kext `apply` adds for this display plan.
pub fn gpu_kext(ctx: &PlanContext, display: &DisplayPlan) -> GpuKext {
    resolve_kext(display, &views(ctx))
}

/// True when the GPU driving the displays has no native driver on the target:
/// macOS installs and boots unaccelerated, and OCLP root patches restore
/// acceleration after install (legacy iGPUs, Kepler, GCN 1-3, Polaris/Vega
/// on a CPU without AVX2). The single source of the graphics part of
/// [`super::root_patching_planned`], which decides AMFIPass,
/// csr-active-config, SecureBootModel and `ipc_control_port_options=0`.
pub fn needs_root_patch_graphics(ctx: &PlanContext, display: &DisplayPlan) -> bool {
    display
        .primary
        .and_then(|i| ctx.profile.gpus.get(i))
        .is_some_and(|gpu| usability(ctx, gpu, &gpu_db::support(gpu)) == Usability::RootPatch)
}

/// True when an enabled NVIDIA Web Driver era card runs on the web driver:
/// on High Sierra any active Maxwell/Pascal card (driving the displays or
/// kept next to the display GPU), on later releases a Fermi/Maxwell/Pascal
/// display GPU, which only OCLP's web-driver patch set can drive. Secure Boot
/// must be off with the web driver, and its root patch needs SIP `030A0000`.
pub fn uses_nvidia_web_driver(ctx: &PlanContext, display: &DisplayPlan) -> bool {
    let views = views(ctx);
    if ctx.target == MacOsVersion::HighSierra {
        return active(display, &views).iter().any(|v| {
            matches!(
                v.family(),
                GpuFamily::NvidiaMaxwell | GpuFamily::NvidiaPascal
            )
        });
    }
    display
        .primary
        .and_then(|i| views.get(i))
        .is_some_and(|v| {
            !v.gpu.disabled
                && matches!(
                    v.family(),
                    GpuFamily::NvidiaFermi | GpuFamily::NvidiaMaxwell | GpuFamily::NvidiaPascal
                )
        })
}

/// `device-id` (little endian) of the IMEI device at
/// `PciRoot(0x0)/Pci(0x16,0x0)` when the plan drives an Intel iGPU whose
/// generation differs from the PCH's: Sandy Bridge on a 7-series board gets
/// the 6-series id 0x1C3A, Ivy Bridge on a 6-series board the 7-series id
/// 0x1E3A (Dortania sandy-bridge.md / ivy-bridge.md). `apply` injects it, and
/// the acpi stage adds SSDT-IMEI for exactly these plans. Reads
/// `plan.smbios.model` (an iGPU hidden for MacPro/iMacPro gets nothing).
pub fn imei_device_id(
    ctx: &PlanContext,
    display: &DisplayPlan,
    plan: &BuildPlan,
) -> Option<[u8; 4]> {
    let views = views(ctx);
    let v = display.igpu.and_then(|i| views.get(i))?;
    if !v.is_intel_igpu() || hides_igpu(display, &views, &plan.smbios.model) {
        return None;
    }
    igpu::recipe(v.gpu, igpu_role(ctx, display), ctx.target)?;
    let hex = igpu::imei_device_id(v.family(), ctx.chipset.as_ref())?;
    let id = u32::from_str_radix(hex, 16).ok()?;
    Some(id.to_be_bytes())
}

/// iMacPro1,1 / MacPro7,1 expect no iGPU: a headless one is hidden with
/// `-wegnoigpu` instead of configured (WhateverGreen README).
fn hides_igpu(display: &DisplayPlan, views: &[View], model: &str) -> bool {
    display.igpu_headless
        && resolve_kext(display, views) == GpuKext::WhateverGreen
        && igpu_less_model(model)
}

// ── Display selection ───────────────────────────────────────────────────────

/// Decide which GPU drives the displays on the target and which GPUs are
/// disabled. Error when no GPU can show a picture on the target (except VMs).
///
/// Desktops prefer a natively supported dGPU (the iGPU then runs headless
/// when it has a driver, else it is disabled), then the iGPU, then a GPU that
/// needs OCLP root patches after install. Laptops always drive the internal
/// panel from the iGPU: macOS has no Optimus / switchable graphics, so their
/// dGPUs are disabled; a supported dGPU is used only when no iGPU can drive
/// the panel (MUX in discrete mode). A Vega APU uses NootedRed only without
/// a supported dGPU, since NootedRed and WhateverGreen exclude each other.
/// Navi 22 needs NootRX; the RX 6950 XT / 6900 XT XTXH / 6650 XT class takes
/// WhateverGreen with a device-id spoof, and NootRX only when its device path
/// is unknown or a Navi 22 card stays enabled next to it.
///
/// A GPU past its native range but inside OCLP's root-patch range (Kepler
/// after Big Sur, Haswell after Monterey, ...) is used only when nothing
/// drives the target natively; picking such a release is the user's choice
/// of the OCLP path (`compatibility` lists it as needing root patches).
pub fn choose_display(ctx: &PlanContext) -> Result<DisplayPlan, AppError> {
    let views = views(ctx);
    if ctx.is_vm {
        return Ok(choose_vm(ctx, &views));
    }
    if views.iter().all(View::is_virtual) {
        return Err(AppError::new("NO_GPU", "No graphics device was detected.")
            .recoverable()
            .with_suggestion(
                "Add the graphics card or the integrated graphics in the hardware editor.",
            ));
    }
    let chosen = if ctx.is_laptop {
        choose_laptop(ctx, &views)
    } else {
        choose_desktop(ctx, &views)
    };
    match chosen {
        Some(chosen) => {
            tracing::debug!(display = ?chosen, target = ctx.target.id(), "display path chosen");
            Ok(chosen)
        }
        None => Err(no_display_error(ctx, &views)),
    }
}

/// How macOS can use one GPU on the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Usability {
    /// Drives displays with Apple's driver, a documented spoof or a Lilu plugin.
    Native,
    /// Unaccelerated picture until OCLP root patches are applied after install.
    RootPatch,
    /// Driver attaches but the GPU has no display path (Quick Sync / compute).
    Headless,
    Unusable,
}

struct View<'a> {
    index: usize,
    gpu: &'a ProfileGpu,
    support: GpuSupport,
    usability: Usability,
}

fn views<'a>(ctx: &PlanContext<'a>) -> Vec<View<'a>> {
    ctx.profile
        .gpus
        .iter()
        .enumerate()
        .map(|(index, gpu)| {
            let support = gpu_db::support(gpu);
            let usability = usability(ctx, gpu, &support);
            View {
                index,
                gpu,
                support,
                usability,
            }
        })
        .collect()
}

impl View<'_> {
    fn family(&self) -> GpuFamily {
        self.gpu.family
    }

    fn is_virtual(&self) -> bool {
        self.gpu.family == GpuFamily::VirtualDisplay || self.gpu.vendor == GpuVendor::Virtual
    }

    fn is_igpu(&self) -> bool {
        !self.is_virtual() && (self.gpu.is_igpu || igpu_family(self.gpu.family))
    }

    fn is_dgpu(&self) -> bool {
        !self.is_virtual() && !self.is_igpu()
    }

    fn is_intel_igpu(&self) -> bool {
        self.is_igpu() && intel_igpu_family(self.gpu.family)
    }

    /// Can be picked to drive the displays.
    fn selectable(&self) -> bool {
        !self.gpu.disabled && !self.is_virtual()
    }

    /// The Lilu plugin this GPU asks for on its own.
    fn kext(&self) -> GpuKext {
        if self.is_virtual() {
            return GpuKext::None;
        }
        match self.support.requirement {
            GpuRequirement::NootRx => GpuKext::NootRx,
            GpuRequirement::NootedRed => GpuKext::NootedRed,
            _ => GpuKext::WhateverGreen,
        }
    }

    /// NootRX GPUs that also run on WhateverGreen with a device-id spoof
    /// (RX 6950 XT, 6900 XT XTXH, 6650 XT, W6600M): the spoof id.
    fn weg_alternative(&self) -> Option<[u8; 4]> {
        if self.support.requirement == GpuRequirement::NootRx {
            gpu_db::weg_spoof_alternative(self.gpu)
        } else {
            None
        }
    }

    /// Only NootRX can drive it: Navi 22 and the variants without a spoof,
    /// and the spoofable ones whose device path is unknown (the device-id
    /// property cannot be injected, NootRX matches the real id).
    fn needs_nootrx(&self) -> bool {
        self.kext() == GpuKext::NootRx
            && (self.weg_alternative().is_none() || self.pci_path().is_none())
    }

    /// The kext this GPU forces on the build when it drives the displays.
    /// The spoofable RDNA 2 variants take the documented WhateverGreen
    /// device-id spoof (Dortania GPU Buyers Guide; research-gpu §3.3, §9);
    /// NootRX has no tagged releases and is the fallback.
    fn primary_kext(&self) -> GpuKext {
        if self.kext() == GpuKext::NootRx && !self.needs_nootrx() {
            GpuKext::WhateverGreen
        } else {
            self.kext()
        }
    }

    fn label(&self) -> String {
        gpu_label(self.gpu)
    }

    fn pci_path(&self) -> Option<String> {
        self.gpu
            .pci_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
    }
}

fn gpu_label(gpu: &ProfileGpu) -> String {
    let name = gpu.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    gpu_db::identify(gpu.vendor_id.as_deref(), gpu.device_id.as_deref(), "")
        .model_name
        .unwrap_or_else(|| gpu_db::family_label(gpu.family).to_string())
}

fn usability(ctx: &PlanContext, gpu: &ProfileGpu, s: &GpuSupport) -> Usability {
    let target = ctx.target;
    let reached = s.min_native.is_some_and(|min| target >= min);
    let within = s.max_native.is_none_or(|max| target <= max);
    if s.display_capable && gpu_db::natively_supported_on(gpu, target) {
        // Polaris/Vega/Navi userspace needs AVX2 from macOS 13 on (Dortania
        // ventura.md); OCLP restores Polaris/Vega on such CPUs, Navi only
        // behind a developer flag.
        if ctx.lacks_avx2_for_target() && needs_avx2(gpu.family) {
            return if oclp_patches_without_avx2(gpu.family) {
                Usability::RootPatch
            } else {
                Usability::Unusable
            };
        }
        return Usability::Native;
    }
    // OCLP root patches run on Big Sur and newer only (research-gpu §8).
    if s.display_capable
        && reached
        && !within
        && target >= MacOsVersion::BigSur
        && s.max_with_root_patch.is_some_and(|max| target <= max)
    {
        return Usability::RootPatch;
    }
    if !s.display_capable && reached && within {
        return Usability::Headless;
    }
    Usability::Unusable
}

/// Selection rules that differ between desktops, laptops and VMs.
#[derive(Clone, Copy)]
struct Rules {
    /// Natively supported secondary dGPUs stay enabled.
    keep_native_dgpus: bool,
    /// An Intel iGPU may stay enabled headless next to a dGPU.
    allow_headless: bool,
}

fn choose_desktop(ctx: &PlanContext, views: &[View]) -> Option<DisplayPlan> {
    use Usability::{Native, RootPatch};
    let pick = |dgpu: bool, level: Usability| {
        best(
            views
                .iter()
                .filter(|v| v.selectable() && v.is_dgpu() == dgpu && v.usability == level),
        )
    };
    let primary = pick(true, Native)
        .or_else(|| pick(false, Native))
        .or_else(|| pick(true, RootPatch))
        .or_else(|| pick(false, RootPatch))?;
    let rules = Rules {
        keep_native_dgpus: true,
        allow_headless: true,
    };
    Some(assemble(ctx, views, primary, rules))
}

fn choose_laptop(ctx: &PlanContext, views: &[View]) -> Option<DisplayPlan> {
    use Usability::{Native, RootPatch};
    let pick = |dgpu: bool, level: Usability| {
        best(
            views
                .iter()
                .filter(|v| v.selectable() && v.is_dgpu() == dgpu && v.usability == level),
        )
    };
    // The internal panel hangs off the iGPU; a dGPU only drives it on MUX
    // laptops switched to discrete mode (Dortania GPU Buyers Guide), which
    // the profile shows as a disabled or missing iGPU. Same rule as
    // `compatibility::can_drive_display`.
    let igpu_enabled = views.iter().any(|v| v.is_igpu() && !v.gpu.disabled);
    let primary = if igpu_enabled {
        pick(false, Native).or_else(|| pick(false, RootPatch))?
    } else {
        pick(true, Native).or_else(|| pick(true, RootPatch))?
    };
    let rules = Rules {
        keep_native_dgpus: false,
        allow_headless: false,
    };
    Some(assemble(ctx, views, primary, rules))
}

fn choose_vm(ctx: &PlanContext, views: &[View]) -> DisplayPlan {
    let passthrough = best(
        views
            .iter()
            .filter(|v| v.selectable() && v.usability == Usability::Native),
    )
    .or_else(|| {
        best(
            views
                .iter()
                .filter(|v| v.selectable() && v.usability == Usability::RootPatch),
        )
    });
    if let Some(primary) = passthrough {
        let rules = Rules {
            keep_native_dgpus: true,
            allow_headless: false,
        };
        return assemble(ctx, views, primary, rules);
    }
    DisplayPlan {
        primary: views
            .iter()
            .find(|v| v.is_virtual() && !v.gpu.disabled)
            .map(|v| v.index),
        igpu: None,
        igpu_headless: false,
        disabled: views
            .iter()
            .filter(|v| !v.is_virtual() && (v.gpu.disabled || ctx.options.disable_unsupported_gpus))
            .map(|v| v.index)
            .collect(),
    }
}

/// Lowest rank wins; ties go to the first GPU in the profile.
fn best<'v, 'a>(candidates: impl Iterator<Item = &'v View<'a>>) -> Option<&'v View<'a>>
where
    'a: 'v,
{
    candidates.min_by_key(|v| (rank(v), v.index))
}

fn rank(v: &View) -> u8 {
    use GpuFamily::*;
    if v.needs_nootrx() {
        return 2;
    }
    if v.weg_alternative().is_some() {
        return 1;
    }
    match v.family() {
        AmdPolaris | AmdLexa | AmdVega10 | AmdVega20 | AmdNavi10 | AmdNavi12 | AmdNavi14
        | AmdNavi21 | AmdNavi23 => 0,
        AmdGcn1 | AmdGcn2 | AmdGcn3 => 1,
        NvidiaKepler => 3,
        _ => 4,
    }
}

fn assemble(ctx: &PlanContext, views: &[View], primary: &View, rules: Rules) -> DisplayPlan {
    let kext = display_kext(views, primary, rules);
    let mut display = DisplayPlan {
        primary: Some(primary.index),
        igpu: primary.is_igpu().then_some(primary.index),
        igpu_headless: false,
        disabled: Vec::new(),
    };
    for v in views {
        if v.index == primary.index || v.is_virtual() {
            continue;
        }
        if v.gpu.disabled {
            display.disabled.push(v.index);
            continue;
        }
        if v.is_igpu() {
            if rules.allow_headless && display.igpu.is_none() && can_be_headless(ctx, v, kext) {
                display.igpu = Some(v.index);
                display.igpu_headless = true;
            } else {
                display.disabled.push(v.index);
            }
            continue;
        }
        let conflict = conflicts(kext, v);
        if rules.keep_native_dgpus && v.usability == Usability::Native && !conflict {
            continue;
        }
        if conflict || ctx.options.disable_unsupported_gpus {
            display.disabled.push(v.index);
        }
    }
    display
}

/// The GPU kext the selection is built around. A spoofable RDNA 2 primary
/// switches to NootRX when a second card that only NootRX drives stays
/// enabled and no other kept card needs WhateverGreen, so both keep working.
fn display_kext(views: &[View], primary: &View, rules: Rules) -> GpuKext {
    let kext = primary.primary_kext();
    if kext != GpuKext::WhateverGreen
        || primary.weg_alternative().is_none()
        || !rules.keep_native_dgpus
    {
        return kext;
    }
    let kept: Vec<&View> = views
        .iter()
        .filter(|v| {
            v.index != primary.index
                && v.selectable()
                && v.is_dgpu()
                && v.usability == Usability::Native
        })
        .collect();
    let nootrx_only = kept.iter().any(|v| v.needs_nootrx());
    let weg_only = kept.iter().any(|v| v.kext() == GpuKext::WhateverGreen);
    if nootrx_only && !weg_only {
        GpuKext::NootRx
    } else {
        kext
    }
}

/// A secondary GPU that cannot stay enabled next to the primary's kext.
fn conflicts(primary: GpuKext, other: &View) -> bool {
    let driven = matches!(other.usability, Usability::Native | Usability::RootPatch);
    match primary {
        // NootedRed refuses GCN 5 / RDNA dGPUs and cannot share the system
        // with WhateverGreen (NootedRed release notes, research-amd §13).
        GpuKext::NootedRed => gcn5_or_rdna(other.family()) || driven,
        GpuKext::NootRx => driven && other.kext() != GpuKext::NootRx,
        GpuKext::WhateverGreen => driven && other.needs_nootrx(),
        GpuKext::None => false,
    }
}

/// An Intel iGPU with a native driver and a connector-less framebuffer stays
/// enabled for Quick Sync while a dGPU drives the displays.
fn can_be_headless(ctx: &PlanContext, v: &View, primary: GpuKext) -> bool {
    v.is_intel_igpu()
        && matches!(v.usability, Usability::Native | Usability::Headless)
        && igpu::recipe(v.gpu, Role::Headless, ctx.target).is_some()
        && !skylake_needs_weg(ctx, v, primary)
}

/// Skylake runs as Kaby Lake on macOS 13+ only through WhateverGreen, which
/// cannot load next to NootRX.
fn skylake_needs_weg(ctx: &PlanContext, v: &View, primary: GpuKext) -> bool {
    primary == GpuKext::NootRx
        && v.family() == GpuFamily::IntelSkylake
        && ctx.target >= MacOsVersion::Ventura
}

fn no_display_error(ctx: &PlanContext, views: &[View]) -> AppError {
    let target = ctx.target;
    let reasons: Vec<String> = views
        .iter()
        .filter(|v| !v.is_virtual())
        .map(|v| format!("{} ({})", v.label(), why_unusable(ctx, v)))
        .collect();
    let message = format!(
        "No graphics device in this machine can show a picture on {}: {}.",
        target.display_name(),
        reasons.join("; ")
    );
    AppError::new("NO_DISPLAY_PATH", message)
        .recoverable()
        .with_suggestion(suggestion(ctx, views))
        .with_context(json!({ "target": target.id(), "gpus": reasons }))
}

/// Short reason why `v` cannot drive a display on the target.
fn why_unusable(ctx: &PlanContext, v: &View) -> String {
    let s = &v.support;
    let target = ctx.target;
    if v.gpu.disabled {
        return "disabled in the hardware editor".into();
    }
    if v.family() == GpuFamily::Unknown {
        return "it could not be identified".into();
    }
    if s.display_capable
        && ctx.lacks_avx2_for_target()
        && needs_avx2(v.family())
        && gpu_db::natively_supported_on(v.gpu, target)
    {
        return "its drivers need a CPU with AVX2 from macOS 13 on".into();
    }
    if !s.display_capable {
        return if s.min_native.is_some() {
            "it cannot drive a display under macOS".into()
        } else {
            "macOS has no driver for it".into()
        };
    }
    if let Some(min) = s.min_native.filter(|min| target < *min) {
        return format!("its drivers start with {}", min.display_name());
    }
    if let Some(max) = s.max_native.filter(|max| target > *max) {
        return format!("its drivers end with {}", max.display_name());
    }
    if v.is_dgpu() && ctx.is_laptop {
        return "laptop discrete GPUs cannot drive the internal panel".into();
    }
    "macOS cannot drive it on this release".into()
}

fn suggestion(ctx: &PlanContext, views: &[View]) -> String {
    let target = ctx.target;
    let candidates: Vec<&View> = views
        .iter()
        .filter(|v| v.selectable() && v.support.display_capable && (!ctx.is_laptop || v.is_igpu()))
        .collect();
    let cpu_allows = |version: MacOsVersion| {
        ctx.cpu.min_macos.is_none_or(|min| version >= min)
            && ctx.cpu.max_macos.is_none_or(|max| version <= max)
    };
    let works = |version: MacOsVersion| {
        cpu_allows(version)
            && candidates
                .iter()
                .any(|v| gpu_db::natively_supported_on(v.gpu, version))
    };
    let mut parts = Vec::new();
    if let Some(older) = MacOsVersion::newest_first().find(|v| *v < target && works(*v)) {
        parts.push(format!("Choose {} or older.", older.display_name()));
    } else if let Some(newer) = MacOsVersion::ALL
        .into_iter()
        .find(|v| *v > target && works(*v))
    {
        parts.push(format!("Choose {} or newer.", newer.display_name()));
    }
    if views
        .iter()
        .any(|v| v.gpu.disabled && v.support.display_capable)
    {
        parts.push(
            "Re-enable a GPU you disabled in the hardware editor if it should drive the display."
                .into(),
        );
    }
    if ctx.is_laptop {
        parts.push(
            "A laptop needs integrated graphics macOS supports (Intel HD/UHD/Iris up to Ice Lake, or an AMD \
             Vega APU)."
                .into(),
        );
        let dgpu_works = views.iter().any(|v| {
            v.is_dgpu() && !v.gpu.disabled && matches!(v.usability, Usability::Native | Usability::RootPatch)
        });
        if dgpu_works {
            parts.push(
                "If the laptop has a MUX switch, set it to discrete-only mode in the BIOS and scan again (or \
                 disable the integrated graphics in the hardware editor) to drive the displays from the dGPU."
                    .into(),
            );
        }
    } else {
        parts.push(
            "Otherwise install a graphics card macOS supports natively, such as an AMD Radeon RX 580, \
             RX 5700 XT or RX 6600 XT."
                .into(),
        );
    }
    parts.join(" ")
}

// ── Build plan ──────────────────────────────────────────────────────────────

/// Add GPU device properties (AAPL,ig-platform-id / AAPL,snb-platform-id,
/// device-id spoofs, framebuffer patches, headless ids, disable-gpu), GPU
/// boot-args (-wegnoegpu, agdpmod=..., -igfxcdc, ...), and the GPU kext
/// (exactly one of WhateverGreen / NootRX / NootedRed, plus SMCRadeonSensors
/// where useful).
pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    let views = views(ctx);
    let kext = resolve_kext(display, &views);
    let primary = display.primary.and_then(|i| views.get(i));
    let imei = imei_device_id(ctx, display, plan);

    let mut hide_igpu = false;
    if let Some(v) = display.igpu.and_then(|i| views.get(i)) {
        if v.is_intel_igpu() {
            if hides_igpu(display, &views, &plan.smbios.model) {
                hide_igpu = true;
            } else {
                apply_intel_igpu(ctx, display, v, plan);
            }
        } else if v.family() == GpuFamily::AmdApuVega {
            apply_nootedred(v, plan);
        }
    }
    if let Some(id) = imei {
        plan.device_properties.push(DevicePropertyEntry {
            path: IMEI_PATH.to_string(),
            properties: vec![data_prop("device-id", &id)],
            reason: "IMEI: graphics driver needs the id matching the CPU generation (SSDT-IMEI)"
                .into(),
        });
    }

    for v in active(display, &views).into_iter().filter(|v| v.is_dgpu()) {
        if amd_dgpu_family(v.family()) {
            apply_amd(ctx, display, v, kext, plan);
        } else if nvidia_family(v.family()) {
            apply_nvidia(v, plan);
        }
    }

    apply_disabled(ctx, display, &views, kext, hide_igpu, plan);
    push_kexts(ctx, primary, &views, display, kext, plan);
    push_display_notes(ctx, display, primary, kext, plan);
}

/// GPUs macOS drives: the primary, the kept iGPU and natively supported
/// secondary dGPUs.
fn active<'v, 'a>(display: &DisplayPlan, views: &'v [View<'a>]) -> Vec<&'v View<'a>> {
    views
        .iter()
        .filter(|v| {
            !display.disabled.contains(&v.index)
                && (display.primary == Some(v.index)
                    || display.igpu == Some(v.index)
                    || (v.is_dgpu() && v.usability == Usability::Native))
        })
        .collect()
}

fn resolve_kext(display: &DisplayPlan, views: &[View]) -> GpuKext {
    let active = active(display, views);
    if active.iter().any(|v| v.kext() == GpuKext::NootedRed) {
        return GpuKext::NootedRed;
    }
    if active.iter().any(|v| v.needs_nootrx()) {
        // An Intel iGPU kept next to Navi 22 runs on Apple's driver alone;
        // RX 6950 XT class cards next to it run on NootRX by their real id.
        return GpuKext::NootRx;
    }
    // Everything else, RX 6950 XT class cards included (device-id spoof),
    // runs with WhateverGreen. disable-gpu, -wegnoegpu and -wegnoigpu are
    // WhateverGreen features too.
    let real = |i: &usize| views.get(*i).is_some_and(|v| !v.is_virtual());
    let drives = active.iter().any(|v| !v.is_virtual());
    if drives || display.disabled.iter().any(real) {
        GpuKext::WhateverGreen
    } else {
        GpuKext::None
    }
}

fn igpu_role(ctx: &PlanContext, display: &DisplayPlan) -> Role {
    if display.igpu_headless {
        Role::Headless
    } else if ctx.has_panel {
        Role::Panel
    } else if ctx.profile.form_factor == FormFactor::MiniPc && ctx.profile.cpu.is_mobile {
        Role::Nuc
    } else {
        Role::Desktop
    }
}

fn apply_intel_igpu(ctx: &PlanContext, display: &DisplayPlan, v: &View, plan: &mut BuildPlan) {
    let role = igpu_role(ctx, display);
    let label = v.label();
    let Some(recipe) = igpu::recipe(v.gpu, role, ctx.target) else {
        plan.notes.push(note(
            NoteLevel::Warning,
            format!("No framebuffer settings for the {label}"),
            "No documented framebuffer exists for this graphics generation and use; the iGPU gets no \
             properties.",
        ));
        return;
    };

    let mut properties = Vec::new();
    let mut summary = Vec::new();
    if let Some((key, id)) = recipe.platform {
        properties.push(data_prop(key, &id.to_le_bytes()));
        summary.push(format!("{key} 0x{id:08X}"));
    }
    if let Some(id) = recipe.device_id {
        properties.push(data_prop("device-id", &device_id_bytes(id)));
        summary.push(format!("device-id 0x{id:04X}"));
    }
    for (key, value) in igpu::mem_properties(recipe.mem) {
        properties.push(hex_prop(key, value));
    }
    let backlight = if role == Role::Panel {
        igpu::backlight_property(v.family(), ctx.target)
    } else {
        None
    };
    if let Some(key) = backlight {
        properties.push(hex_prop(key, ONE));
    }
    let what = match role {
        Role::Headless => "headless for Quick Sync",
        Role::Panel => "drives the internal panel",
        Role::Desktop | Role::Nuc => "drives the displays",
    };
    plan.device_properties.push(DevicePropertyEntry {
        path: v.pci_path().unwrap_or_else(|| IGPU_PATH.to_string()),
        properties,
        reason: format!("{label}: {what} ({})", summary.join(", ")),
    });

    if role != Role::Headless {
        // Ice Lake: -igfxcdc and -igfxdvmt (Dortania laptop icelake.md).
        for arg in &v.support.boot_args {
            push_arg(plan, arg);
        }
    }

    igpu_notes(ctx, display, v, role, recipe.device_id, plan);
}

fn igpu_notes(
    ctx: &PlanContext,
    display: &DisplayPlan,
    v: &View,
    role: Role,
    device_id: Option<u16>,
    plan: &mut BuildPlan,
) {
    use GpuFamily::*;
    let label = v.label();
    if role == Role::Headless {
        let primary = display
            .primary
            .and_then(|i| ctx.profile.gpus.get(i))
            .map(gpu_label)
            .unwrap_or_else(|| "graphics card".into());
        plan.notes.push(note(
            NoteLevel::Info,
            format!("{label} runs headless"),
            format!(
                "The {primary} drives the displays, so connect the monitors to it. The iGPU stays enabled without \
                 display outputs for Quick Sync video encoding and decoding."
            ),
        ));
    }
    let kaby_spoof =
        v.family() == IntelSkylake && device_id.is_some_and(|id| (0x5900..=0x59FF).contains(&id));
    if kaby_spoof {
        plan.notes.push(note(
            NoteLevel::Info,
            "Skylake graphics run as Kaby Lake",
            "macOS 13 and newer have no Skylake graphics driver, so the iGPU is presented as the closest Kaby Lake \
             model. Add -igfxsklaskbl if this EFI also boots macOS 12 or older.",
        ));
    }
    if role != Role::Panel {
        return;
    }
    match v.family() {
        IntelSandyBridge => plan.notes.push(note(
            NoteLevel::Info,
            "Sandy Bridge panel",
            "Panels of 1600x900 or more may also need AAPL00,DualLink = 01000000 on the iGPU.",
        )),
        IntelIvyBridge => plan.notes.push(note(
            NoteLevel::Info,
            "Ivy Bridge panel",
            "The framebuffer 0x01660003 suits panels up to 1366x768. For 1600x900 or more use 0x01660004 with \
             Dortania's connector patches (or 0x01660009 for some eDP panels).",
        )),
        // WhateverGreen scopes the backlight register fixes to Kaby Lake and
        // newer; Dortania applies them by default from Coffee Lake on.
        IntelKabyLake | IntelSkylake if v.family() == IntelKabyLake || kaby_spoof => {
            plan.notes.push(note(
                NoteLevel::Info,
                "Backlight",
                if ctx.target >= MacOsVersion::Ventura {
                    "If the panel turns black or dim after boot, add enable-backlight-registers-alternative-fix \
                     (01000000) to the iGPU properties."
                } else {
                    "If the panel turns black or dim after boot, add enable-backlight-registers-fix (01000000) \
                     to the iGPU properties."
                },
            ))
        }
        IntelCoffeeLake | IntelCometLake => plan.notes.push(note(
            NoteLevel::Info,
            "Backlight",
            "The backlight register fix is set on the iGPU. For smoother brightness steps add -igfxbls \
             (enable-backlight-smoother).",
        )),
        IntelIceLake => plan.notes.push(note(
            NoteLevel::Info,
            "Ice Lake graphics",
            "Apple's Ice Lake driver does not support HDMI outputs; use DisplayPort or USB-C. Set DVMT \
             pre-allocated to 256 MB if the firmware allows it. Add -igfxdbeo for a garbled panel after boot, \
             -noDC9 for a black screen or panic after wake and -igfxbls for smoother brightness steps.",
        )),
        _ => {}
    }
}

fn apply_nootedred(v: &View, plan: &mut BuildPlan) {
    let model = plan.smbios.model.clone();
    if model.starts_with("MacPro") {
        plan.notes.push(note(
            NoteLevel::Warning,
            "SMBIOS for NootedRed",
            format!(
                "NootedRed may show a black screen with {model}; MacBookPro16,2 or iMac20,1 are the recommended \
                 models for the {}.",
                v.label()
            ),
        ));
    }
}

fn apply_amd(
    ctx: &PlanContext,
    display: &DisplayPlan,
    v: &View,
    kext: GpuKext,
    plan: &mut BuildPlan,
) {
    use GpuFamily::*;
    let label = v.label();
    let via_weg = kext == GpuKext::WhateverGreen;
    let alternative = if via_weg { v.weg_alternative() } else { None };
    // NootRX drives Navi 2x by its real id; WhateverGreen needs the spoof.
    let spoof = alternative.or_else(|| {
        (v.kext() == GpuKext::WhateverGreen)
            .then(|| gpu_db::device_id_for(v.gpu, ctx.target))
            .flatten()
    });

    let mut properties = Vec::new();
    if let Some(bytes) = spoof {
        properties.push(data_prop("device-id", &bytes));
        properties.push(DeviceProperty {
            key: "model".into(),
            value: PlistScalar::Str(label.clone()),
        });
        // -radcodec lets the spoofed card use its hardware encoder.
        push_arg(plan, "-radcodec");
    }
    for arg in v
        .support
        .boot_args
        .iter()
        .filter(|a| !a.starts_with("agdpmod="))
    {
        push_arg(plan, arg);
    }

    if via_weg {
        // Most Navi cards black-screen on iMac / iMacPro / Macmini board-ids
        // without agdpmod=pikera; Polaris and Vega must not use it, MacPro7,1
        // needs nothing (Dortania GPU Buyers Guide and coffee-lake.md
        // boot-args; research-gpu §3.5, audit M18).
        let navi = alternative.is_some()
            || v.support
                .boot_args
                .iter()
                .any(|a| a.starts_with("agdpmod="));
        let model = plan.smbios.model.as_str();
        let agdp_board = model.starts_with("iMac") || model.starts_with("Macmini");
        if ctx.target == MacOsVersion::Tahoe {
            // WhateverGreen maintainer, Aug 2026: agdpmod=ignore on macOS 26
            // (research-gpu §0.3). WhateverGreen then leaves AGDP alone, so the
            // board-id → board-ix kernel patch Dortania tahoe.md links (Pike R.
            // Alpha's patch, values in research-gpu §3.7) keeps the pikera effect.
            push_arg(plan, "agdpmod=ignore");
            if navi && agdp_board {
                push_agdp_patch(plan);
            }
            push_note_once(&mut plan.notes, note(
                NoteLevel::Info,
                "AMD graphics on macOS 26",
                "agdpmod=ignore is set as the WhateverGreen maintainer advises for macOS 26; Lilu 1.7.2 and \
                 WhateverGreen 1.7.1 or newer are required. If WhateverGreen still panics, remove it; with an iMac \
                 SMBIOS and a Navi card, Dortania's AppleGraphicsDevicePolicy board-id patch then replaces \
                 agdpmod=pikera.",
            ));
        } else if navi && agdp_board {
            push_arg(plan, "agdpmod=pikera");
        }
    }

    let drives_panel = display.primary == Some(v.index) && ctx.has_panel;
    if drives_panel && matches!(v.family(), AmdNavi10 | AmdNavi12 | AmdNavi14) {
        // PWM backlight for an RX 5000 GPU wired to the internal panel (WhateverGreen README).
        push_arg(plan, "applbkl=3");
    }

    if properties.is_empty() {
        return;
    }
    match v.pci_path() {
        Some(path) => plan.device_properties.push(DevicePropertyEntry {
            path,
            properties,
            reason: format!("{label}: device-id spoof to a model macOS supports"),
        }),
        None => plan.notes.push(note(
            NoteLevel::Warning,
            format!("{label} needs a device-id spoof"),
            "Its PCI device path is unknown, so the device-id property cannot be injected and macOS will not \
             accelerate the card. Enter the device path in the hardware editor or use an SSDT-GPU-SPOOF.",
        )),
    }
}

fn push_agdp_patch(plan: &mut BuildPlan) {
    const COMMENT: &str =
        "AppleGraphicsDevicePolicy board-id to board-ix (agdpmod=pikera) | macOS 26";
    if plan.kernel_patches.iter().any(|p| p.comment == COMMENT) {
        return;
    }
    plan.kernel_patches.push(BinaryPatch {
        comment: COMMENT.into(),
        arch: "x86_64".into(),
        identifier: "com.apple.driver.AppleGraphicsDevicePolicy".into(),
        base: String::new(),
        // "board-id" → "board-ix"
        find: "626F6172642D6964".into(),
        mask: String::new(),
        replace: "626F6172642D6978".into(),
        replace_mask: String::new(),
        count: 1,
        limit: 0,
        skip: 0,
        min_kernel: MacOsVersion::Tahoe.min_kernel(),
        max_kernel: String::new(),
        enabled: true,
    });
}

fn apply_nvidia(v: &View, plan: &mut BuildPlan) {
    for arg in &v.support.boot_args {
        push_arg(plan, arg);
    }
    if matches!(
        v.family(),
        GpuFamily::NvidiaMaxwell | GpuFamily::NvidiaPascal
    ) {
        // NVIDIA Web Driver on 10.13.6 (Dortania GPU Buyers Guide): nvda_drv_vrl=1
        // plus nvda_drv = "1" in NVRAM.
        if !plan.nvram_add.iter().any(|n| n.key == "nvda_drv") {
            plan.nvram_add.push(NvramVariable {
                guid: APPLE_NVRAM_GUID.into(),
                key: "nvda_drv".into(),
                value: PlistScalar::Data("31".into()),
            });
        }
        plan.post_install.push(note(
            NoteLevel::Info,
            "Install the NVIDIA Web Driver",
            format!(
                "The {} only works with NVIDIA Web Driver 387.10.10.10.40.140, which requires macOS 10.13.6 build \
                 17G14042. Update to that build, install the driver and reboot; until then the card runs \
                 unaccelerated.",
                v.label()
            ),
        ));
    }
}

fn apply_disabled(
    ctx: &PlanContext,
    display: &DisplayPlan,
    views: &[View],
    kext: GpuKext,
    hide_igpu: bool,
    plan: &mut BuildPlan,
) {
    let primary = display.primary.and_then(|i| views.get(i));
    let disabled: Vec<&View> = display
        .disabled
        .iter()
        .filter_map(|&i| views.get(i))
        .filter(|v| !v.is_virtual())
        .collect();
    let dgpus: Vec<&View> = disabled.iter().copied().filter(|v| v.is_dgpu()).collect();
    let dgpu_enabled = views
        .iter()
        .any(|v| v.is_dgpu() && !display.disabled.contains(&v.index));

    if !dgpus.is_empty() {
        let path_missing = dgpus.iter().any(|v| v.pci_path().is_none());
        if !dgpu_enabled && (ctx.is_laptop || path_missing) {
            // -wegnoegpu hides every dGPU; only usable while no dGPU drives a
            // display (Dortania laptop guides and desktop GPU disable page).
            push_arg(plan, "-wegnoegpu");
        } else {
            for v in &dgpus {
                match v.pci_path() {
                    Some(path) => plan.device_properties.push(DevicePropertyEntry {
                        path,
                        properties: vec![hex_prop("disable-gpu", ONE)],
                        reason: format!("{}: disabled for macOS", v.label()),
                    }),
                    None => plan.notes.push(note(
                        NoteLevel::Warning,
                        format!("Cannot disable the {}", v.label()),
                        "Its PCI device path is unknown, so the disable-gpu property cannot be set, and -wegnoegpu \
                         would also hide the GPU that drives the displays. Enter the device path in the hardware \
                         editor, or disable or remove the card.",
                    )),
                }
            }
        }
        if kext == GpuKext::NootRx {
            plan.notes.push(note(
                NoteLevel::Info,
                "GPU disabling without WhateverGreen",
                "disable-gpu is a WhateverGreen property and WhateverGreen cannot load next to NootRX. If macOS \
                 hangs on a disabled card, disable it in the firmware or remove it.",
            ));
        }
        if ctx.is_laptop {
            plan.post_install.push(note(
                NoteLevel::Info,
                "Power off the discrete GPU",
                "The discrete GPU is only hidden from macOS and still draws power. After installing, add \
                 SSDT-dGPU-Off (or SSDT-NoHybGfx) for its ACPI path to turn it off and save battery.",
            ));
        }
    }

    for v in disabled.iter().filter(|v| v.is_igpu()) {
        if v.is_intel_igpu() {
            if kext == GpuKext::WhateverGreen {
                push_arg(plan, "-wegnoigpu");
            } else {
                plan.notes.push(note(
                    NoteLevel::Info,
                    format!("Disable the {} in the firmware", v.label()),
                    format!(
                        "-wegnoigpu needs WhateverGreen, which cannot load next to {}. Turn the iGPU off in the \
                         BIOS (or set iGPU multi-monitor off) if macOS misbehaves with it.",
                        kext.name()
                    ),
                ));
            }
        } else if let Some(path) = v.pci_path() {
            plan.device_properties.push(DevicePropertyEntry {
                path,
                properties: vec![hex_prop("disable-gpu", ONE)],
                reason: format!("{}: disabled for macOS", v.label()),
            });
        }
    }
    if hide_igpu {
        push_arg(plan, "-wegnoigpu");
        plan.notes.push(note(
            NoteLevel::Info,
            "iGPU hidden",
            format!(
                "{} expects no integrated graphics, so the iGPU is hidden with -wegnoigpu and the dGPU does all \
                 the work.",
                plan.smbios.model
            ),
        ));
    }

    for v in &disabled {
        plan.notes.push(note(
            NoteLevel::Info,
            format!("{} disabled", v.label()),
            disabled_reason(ctx, primary, v, kext),
        ));
    }

    if !ctx.options.disable_unsupported_gpus {
        // A laptop dGPU macOS has a driver for still cannot reach the panel.
        for v in views.iter().filter(|v| {
            !v.is_virtual()
                && !display.disabled.contains(&v.index)
                && display.primary != Some(v.index)
                && display.igpu != Some(v.index)
                && (v.usability != Usability::Native || (ctx.is_laptop && v.is_dgpu()))
        }) {
            plan.notes.push(note(
                NoteLevel::Warning,
                format!("{} left enabled", v.label()),
                format!(
                    "macOS cannot drive it ({}), but disabling unsupported GPUs is turned off. If the boot hangs or \
                     the screen stays black, turn that option back on.",
                    why_unusable(ctx, v)
                ),
            ));
        }
    }
}

fn disabled_reason(ctx: &PlanContext, primary: Option<&View>, v: &View, kext: GpuKext) -> String {
    if v.gpu.disabled {
        return "Disabled in the hardware editor.".into();
    }
    let primary_label = primary
        .map(View::label)
        .unwrap_or_else(|| "virtual display".into());
    if v.is_dgpu() && ctx.is_laptop {
        return format!(
            "macOS has no switchable graphics (NVIDIA Optimus / AMD PowerXpress), so the internal panel stays on the \
             {primary_label}. Display outputs wired to the discrete GPU will not work in macOS."
        );
    }
    if primary.is_some() {
        if kext == GpuKext::NootedRed && v.is_dgpu() && gcn5_or_rdna(v.family()) {
            return format!(
                "NootedRed, which drives the {primary_label}, refuses to run while a GCN 5 or RDNA graphics card is \
                 enabled."
            );
        }
        if conflicts(kext, v) {
            return format!(
                "It cannot run next to the {primary_label}: {} and {} exclude each other.",
                kext.name(),
                v.kext().name()
            );
        }
        if v.is_igpu() && !v.is_intel_igpu() && v.usability == Usability::Native {
            return format!(
                "The {primary_label} drives the displays; NootedRed (needed for this APU graphics) cannot load \
                 next to WhateverGreen. Turning the iGPU off in the BIOS also frees its memory."
            );
        }
        if v.is_intel_igpu() && skylake_needs_weg(ctx, v, kext) {
            return "macOS 13 and newer drive Skylake graphics only as Kaby Lake through WhateverGreen, which \
                    cannot load next to NootRX."
                .into();
        }
    }
    let why = why_unusable(ctx, v);
    let mut text = format!(
        "macOS cannot use it on {}: {why}.",
        ctx.target.display_name()
    );
    if v.is_dgpu() {
        text.push_str(&format!(" Connect the monitors to the {primary_label}."));
    }
    text
}

fn push_kexts(
    ctx: &PlanContext,
    primary: Option<&View>,
    views: &[View],
    display: &DisplayPlan,
    kext: GpuKext,
    plan: &mut BuildPlan,
) {
    if let Some((id, bundle)) = kext.catalog() {
        let names: Vec<String> = active(display, views)
            .iter()
            .filter(|v| kext == GpuKext::WhateverGreen || v.kext() == kext)
            .map(|v| v.label())
            .collect();
        let reason = match kext {
            GpuKext::WhateverGreen if names.is_empty() => {
                "Hides GPUs macOS cannot drive".to_string()
            }
            GpuKext::WhateverGreen => format!("Graphics fixes for the {}", names.join(" and ")),
            _ => format!(
                "Graphics driver for the {} (replaces WhateverGreen)",
                names.join(" and ")
            ),
        };
        plan.kexts.push(selection(id, bundle, true, reason));
    }
    // GPU temperatures for VirtualSMC (SMCRadeonSensors needs 10.14+).
    let amd_display =
        primary.is_some_and(|v| amd_dgpu_family(v.family()) || v.family() == GpuFamily::AmdApuVega);
    if amd_display && kext != GpuKext::None && ctx.target >= MacOsVersion::Mojave {
        plan.kexts.push(selection(
            "SMCRadeonSensors",
            "SMCRadeonSensors.kext",
            false,
            "AMD GPU temperature sensors".into(),
        ));
    }
}

fn push_display_notes(
    ctx: &PlanContext,
    display: &DisplayPlan,
    primary: Option<&View>,
    kext: GpuKext,
    plan: &mut BuildPlan,
) {
    let Some(p) = primary else {
        plan.notes.push(note(
            NoteLevel::Info,
            "No display adapter",
            "The VM exposes no display adapter; use a serial console or add a virtual display.",
        ));
        return;
    };
    let label = p.label();
    if p.is_virtual() {
        plan.notes.push(note(
            NoteLevel::Info,
            format!("Display: {label}"),
            "macOS drives the virtual display as a plain framebuffer without Metal acceleration; pass a supported \
             GPU through for acceleration.",
        ));
        return;
    }

    let mut detail = match p.kext() {
        GpuKext::NootRx if kext == GpuKext::NootRx => {
            "Driven by NootRX, which replaces WhateverGreen.".to_string()
        }
        GpuKext::NootedRed => "Driven by NootedRed, which replaces WhateverGreen.".to_string(),
        _ if kext == GpuKext::WhateverGreen => {
            "Apple's driver with WhateverGreen patches.".to_string()
        }
        _ => "Apple's driver.".to_string(),
    };
    for text in &p.support.notes {
        detail.push(' ');
        detail.push_str(text);
    }
    let mux = ctx.is_laptop && p.is_dgpu();
    let level = if p.usability == Usability::RootPatch || mux {
        NoteLevel::Warning
    } else {
        NoteLevel::Info
    };
    plan.notes
        .push(note(level, format!("Display: {label}"), detail));

    if mux {
        plan.notes.push(note(
            NoteLevel::Warning,
            "Discrete GPU drives the laptop panel",
            format!(
                "The integrated graphics is off or not listed, so the {label} is used. This only works when the \
                 panel is wired to it: keep the MUX switch in discrete-only mode in the BIOS, or use external \
                 displays on ports wired to the dGPU."
            ),
        ));
    }

    if needs_root_patch_graphics(ctx, display) {
        let tahoe = if ctx.target == MacOsVersion::Tahoe {
            " macOS 26 needs OpenCore Legacy Patcher 3.0 or newer."
        } else {
            ""
        };
        plan.notes.push(note(
            NoteLevel::Warning,
            "Graphics acceleration needs OCLP root patches",
            format!(
                "The {label} has no native driver on {}: macOS installs and boots without acceleration (slow, \
                 glitchy UI) until OpenCore Legacy Patcher root patches are applied. Root patches need lowered SIP, \
                 AMFIPass and SecureBootModel Disabled, and must be reapplied after every update.{tahoe}",
                ctx.target.display_name()
            ),
        ));
        plan.post_install.push(note(
            NoteLevel::Warning,
            "Apply OpenCore Legacy Patcher root patches",
            format!(
                "Install OpenCore Legacy Patcher in macOS and run \"Post-Install Root Patch\" to restore graphics \
                 acceleration for the {label}; repeat it after each macOS update."
            ),
        ));
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn igpu_less_model(model: &str) -> bool {
    model.starts_with("MacPro") || model.starts_with("iMacPro")
}

fn igpu_family(family: GpuFamily) -> bool {
    intel_igpu_family(family)
        || matches!(
            family,
            GpuFamily::AmdApuVega | GpuFamily::AmdApuRdna | GpuFamily::AmdApuLegacy
        )
}

fn intel_igpu_family(family: GpuFamily) -> bool {
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
    )
}

fn amd_dgpu_family(family: GpuFamily) -> bool {
    use GpuFamily::*;
    matches!(
        family,
        AmdTeraScale
            | AmdGcn1
            | AmdGcn2
            | AmdGcn3
            | AmdPolaris
            | AmdLexa
            | AmdVega10
            | AmdVega20
            | AmdNavi10
            | AmdNavi12
            | AmdNavi14
            | AmdNavi21
            | AmdNavi22
            | AmdNavi23
            | AmdNavi24
            | AmdRdna3Plus
    )
}

fn nvidia_family(family: GpuFamily) -> bool {
    use GpuFamily::*;
    matches!(
        family,
        NvidiaTesla | NvidiaFermi | NvidiaKepler | NvidiaMaxwell | NvidiaPascal | NvidiaModern
    )
}

fn gcn5_or_rdna(family: GpuFamily) -> bool {
    use GpuFamily::*;
    matches!(
        family,
        AmdVega10
            | AmdVega20
            | AmdNavi10
            | AmdNavi12
            | AmdNavi14
            | AmdNavi21
            | AmdNavi22
            | AmdNavi23
            | AmdNavi24
            | AmdRdna3Plus
    )
}

fn needs_avx2(family: GpuFamily) -> bool {
    oclp_patches_without_avx2(family)
        || matches!(
            family,
            GpuFamily::AmdNavi10
                | GpuFamily::AmdNavi12
                | GpuFamily::AmdNavi14
                | GpuFamily::AmdNavi21
                | GpuFamily::AmdNavi22
                | GpuFamily::AmdNavi23
        )
}

fn oclp_patches_without_avx2(family: GpuFamily) -> bool {
    matches!(
        family,
        GpuFamily::AmdPolaris | GpuFamily::AmdLexa | GpuFamily::AmdVega10 | GpuFamily::AmdVega20
    )
}

fn device_id_bytes(id: u16) -> [u8; 4] {
    let [lo, hi] = id.to_le_bytes();
    [lo, hi, 0, 0]
}

fn data_prop(key: &str, bytes: &[u8]) -> DeviceProperty {
    DeviceProperty {
        key: key.to_string(),
        value: PlistScalar::data(bytes),
    }
}

fn hex_prop(key: &str, hex: &str) -> DeviceProperty {
    DeviceProperty {
        key: key.to_string(),
        value: PlistScalar::Data(hex.to_string()),
    }
}

/// Append a boot-arg unless one with the same key is already there.
fn push_arg(plan: &mut BuildPlan, arg: &str) {
    let key = arg.split('=').next().unwrap_or(arg);
    if !plan
        .boot_args
        .iter()
        .any(|a| a.split('=').next().unwrap_or(a) == key)
    {
        plan.boot_args.push(arg.to_string());
    }
}

fn selection(id: &str, bundle: &str, required: bool, reason: String) -> KextSelection {
    KextSelection {
        catalog_id: id.to_string(),
        bundle: bundle.to_string(),
        plugins: Vec::new(),
        enabled: true,
        min_kernel: None,
        max_kernel: None,
        required,
        reason,
    }
}

fn push_note_once(notes: &mut Vec<PlanNote>, new: PlanNote) {
    if !notes.iter().any(|n| n.title == new.title) {
        notes.push(new);
    }
}

fn note(level: NoteLevel, title: impl Into<String>, detail: impl Into<String>) -> PlanNote {
    PlanNote {
        level,
        component: "gpu".into(),
        title: title.into(),
        detail: detail.into(),
    }
}
