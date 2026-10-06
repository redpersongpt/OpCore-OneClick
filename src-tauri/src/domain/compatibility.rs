//! Compatibility assessment: per-component support and the list of macOS
//! releases this machine can run, with reasons.
//!
//! The release verdict follows the planner. A release is supported when
//! `planner::validate` accepts the CPU for it (platform supported, native
//! floor and ceiling, AMD core count) and a GPU that macOS drives natively
//! can show the picture ([`display_path`], the rule
//! `planner::graphics::choose_display` applies). Releases that only work
//! through a workaround (OCLP graphics root patches, CryptexFixup or
//! telemetrap past the CPU's native ceiling) are never "supported": they are
//! listed with their caveats as expert options, and a report for such a
//! target is `Partial` so the UI can offer an explicit override instead of a
//! dead end (audit-compat-rules R3/R12, audit-frontend G4). The
//! recommendation is the newest supported release that loses nothing on this
//! machine: macOS 26 removed AppleHDA, Broadcom Wi-Fi needs a root patch from
//! Sonoma on (research-opencore-macos §3.5, audit-compat-rules E1).

use crate::contracts::{note, CompatibilityReport, ComponentAssessment, MacOsOption, SupportLevel};

use super::cpu_db::{self, CpuIdentity};
use super::device_db::{self, BluetoothDriver, EthernetDriver, TouchpadDriver, WifiDriver};
use super::gpu_db::{self, GpuRequirement};
use super::model::{
    CpuPlatform, CpuVendor, FormFactor, GpuFamily, HardwareProfile, InputBus, MacOsVersion,
    NoteLevel, PlanNote, ProfileGpu, StorageKind, VmKind,
};
use super::{codec_db, kext_catalog, macos_db, planner};

// ── Display path (shared with the planner and the BIOS checklist) ───────────

/// How one GPU fares on one release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuStatus {
    /// Native drivers (a documented spoof or NootRX/NootedRed counts as native).
    Native,
    /// Only through OCLP root patches after install.
    RootPatch,
    /// Cannot show a picture on this release.
    Unavailable,
}

/// Which GPU drives the displays on one release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayPath {
    /// `profile.gpus[index]` drives the displays with native drivers.
    Native(usize),
    /// `profile.gpus[index]` needs OCLP graphics root patches.
    RootPatch(usize),
    /// A VM without a usable GPU: plain framebuffer, no acceleration.
    Virtual,
    None,
}

/// AMD Metal dGPU families whose drivers need AVX2 from macOS 13 on
/// (Dortania Ventura notes, research-gpu §3.6; OCLP only patches
/// Polaris/Vega back). Same list as the planner's graphics stage.
fn needs_avx2_driver(family: GpuFamily) -> bool {
    use GpuFamily::*;
    matches!(
        family,
        AmdPolaris
            | AmdLexa
            | AmdVega10
            | AmdVega20
            | AmdNavi10
            | AmdNavi12
            | AmdNavi14
            | AmdNavi21
            | AmdNavi22
            | AmdNavi23
    )
}

/// Native, root-patch or no support of `gpu` on `version` for this machine.
pub fn gpu_status(profile: &HardwareProfile, gpu: &ProfileGpu, version: MacOsVersion) -> GpuStatus {
    let s = gpu_db::support(gpu);
    if !s.display_capable || s.min_native.is_some_and(|min| version < min) {
        return GpuStatus::Unavailable;
    }
    if gpu_db::natively_supported_on(gpu, version) {
        if macos_db::requires_avx2(version) && !has_avx2(profile) && needs_avx2_driver(gpu.family) {
            // research-gpu §8: OCLP patches Polaris/Vega for non-AVX2 CPUs;
            // Navi needs a developer flag, so it is not offered.
            return match gpu.family {
                GpuFamily::AmdPolaris
                | GpuFamily::AmdLexa
                | GpuFamily::AmdVega10
                | GpuFamily::AmdVega20 => GpuStatus::RootPatch,
                _ => GpuStatus::Unavailable,
            };
        }
        return GpuStatus::Native;
    }
    // OCLP root patches run from Big Sur on (research-gpu §8).
    match s.max_with_root_patch {
        Some(max) if version <= max && version >= MacOsVersion::BigSur => GpuStatus::RootPatch,
        _ => GpuStatus::Unavailable,
    }
}

/// Whether `profile.gpus[index]` may drive a display at all. Laptop dGPUs
/// next to an iGPU are Optimus/PowerXpress designs without a MUX: macOS
/// cannot switch them, the panel stays on the iGPU (Dortania GPU buyers
/// guide, laptop dGPUs; research-gpu §5). An iGPU the user disabled in the
/// profile stands for a MUX laptop switched to discrete-only mode.
pub fn can_drive_display(profile: &HardwareProfile, index: usize) -> bool {
    let Some(gpu) = profile.gpus.get(index) else {
        return false;
    };
    if gpu.disabled {
        return false;
    }
    !(profile.form_factor == FormFactor::Laptop
        && !gpu.is_igpu
        && profile.gpus.iter().any(|g| g.is_igpu && !g.disabled))
}

/// Display candidates in preference order: the dGPU on desktops (the iGPU
/// then runs headless), the iGPU on laptops, a virtual display last.
fn display_candidates(profile: &HardwareProfile) -> Vec<usize> {
    let laptop = profile.form_factor == FormFactor::Laptop;
    let mut order: Vec<usize> = (0..profile.gpus.len())
        .filter(|&i| can_drive_display(profile, i))
        .collect();
    order.sort_by_key(|&i| {
        let g = &profile.gpus[i];
        (
            g.family == GpuFamily::VirtualDisplay,
            if laptop { !g.is_igpu } else { g.is_igpu },
            i,
        )
    });
    order
}

/// The display path on `version` (VMs always have at least a framebuffer).
pub fn display_path(profile: &HardwareProfile, version: MacOsVersion) -> DisplayPath {
    let order = display_candidates(profile);
    let status = |i: usize| gpu_status(profile, &profile.gpus[i], version);
    if let Some(i) = order
        .iter()
        .copied()
        .find(|&i| status(i) == GpuStatus::Native)
    {
        return DisplayPath::Native(i);
    }
    if let Some(i) = order
        .iter()
        .copied()
        .find(|&i| status(i) == GpuStatus::RootPatch)
    {
        return DisplayPath::RootPatch(i);
    }
    if profile.vm.is_some() {
        DisplayPath::Virtual
    } else {
        DisplayPath::None
    }
}

// ── CPU facts ───────────────────────────────────────────────────────────────

/// The `cpu_db` identity behind the (possibly edited) profile, the same one
/// the planner works with ([`planner::cpu_identity`]).
pub fn cpu_identity(profile: &HardwareProfile) -> CpuIdentity {
    planner::cpu_identity(&profile.cpu)
}

fn has_avx2(profile: &HardwareProfile) -> bool {
    cpu_identity(profile).has_avx2
}

fn is_apple(profile: &HardwareProfile) -> bool {
    profile.cpu.platform == CpuPlatform::AppleSilicon || profile.cpu.vendor == CpuVendor::Apple
}

/// The machine has an analog codec AppleALC can drive (or one not yet
/// identified): it loses that audio on macOS 26, where AppleHDA is gone.
pub fn has_analog_audio(profile: &HardwareProfile) -> bool {
    profile.vm.is_none()
        && profile
            .audio
            .as_ref()
            .is_some_and(|a| a.codec_id.is_none_or(codec_db::is_supported))
}

/// How likely the laptop only offers Modern Standby (S0ix) instead of S3
/// sleep, which macOS needs (critic-gaps §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModernStandby {
    Unlikely,
    /// Common on this generation (Kaby Lake-R to Comet Lake, Ryzen 4000+).
    Possible,
    /// The norm from Ice Lake on.
    Likely,
}

pub fn modern_standby(profile: &HardwareProfile) -> ModernStandby {
    use CpuPlatform as P;
    if profile.form_factor != FormFactor::Laptop || profile.vm.is_some() {
        return ModernStandby::Unlikely;
    }
    match profile.cpu.platform {
        P::IceLake
        | P::TigerLake
        | P::AlderLake
        | P::RaptorLake
        | P::MeteorLake
        | P::ArrowLake
        | P::LunarLake
        | P::AmdZen4
        | P::AmdZen5 => ModernStandby::Likely,
        P::KabyLake | P::CoffeeLake | P::CometLake | P::AmdZen2 | P::AmdZen3 => {
            ModernStandby::Possible
        }
        _ => ModernStandby::Unlikely,
    }
}

/// Platforms that only boot with community recipes (no Dortania guide).
fn community_platform(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::RocketLake
            | CpuPlatform::TigerLake
            | CpuPlatform::AlderLake
            | CpuPlatform::RaptorLake
            | CpuPlatform::ArrowLake
            | CpuPlatform::AmdZen5
    )
}

/// Outcome of the CPU check for one release.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CpuVerdict {
    Ok,
    /// Past the CPU's native ceiling, reachable only through a documented
    /// workaround (CryptexFixup, telemetrap): an expert option that loses
    /// features, never "supported" (`cpu_db::CeilingWorkaround`).
    Workaround(String),
    Refused(String),
}

/// Physical cores the AMD core-count patch is built for (family 15h modules
/// count as two cores, as the planner does).
fn amd_cores(profile: &HardwareProfile) -> u32 {
    let cpu = &profile.cpu;
    if cpu.platform == CpuPlatform::AmdBulldozer {
        cpu.cores.max(cpu.threads)
    } else {
        cpu.cores
    }
}

/// Why the AMD kernel patches cannot be made for this CPU, if they cannot.
fn amd_core_problem(profile: &HardwareProfile, ident: &CpuIdentity) -> Option<String> {
    if ident.vendor != CpuVendor::Amd {
        return None;
    }
    let cores = amd_cores(profile);
    if cores == 0 {
        Some(
            "The AMD kernel patches need the number of physical CPU cores, which is unknown. Enter it in the \
             hardware editor."
                .into(),
        )
    } else if cores > 64 {
        Some(format!(
            "macOS supports at most 64 CPU cores; this CPU has {cores}."
        ))
    } else {
        None
    }
}

/// The CPU check of the planner (`planner::validate`), at least as strict as
/// it: platform supported (VMs skip it for a guest CPU model that is not a
/// bare-metal platform, `cpu_db::usable_as_vm_guest`), per-model floor
/// (Whiskey/Amber Lake need 10.14.1), native ceiling of this
/// part (`cpu_db::max_macos_for`), and an AMD core count the kernel patches
/// can use. Past the native ceiling a `cpu_db` workaround makes it an expert
/// option.
fn cpu_verdict(
    profile: &HardwareProfile,
    ident: &CpuIdentity,
    version: MacOsVersion,
) -> CpuVerdict {
    let platform = profile.cpu.platform;
    if is_apple(profile) {
        return CpuVerdict::Refused(
            "Apple silicon Macs run macOS natively; OpenCore is for Intel and AMD PCs.".into(),
        );
    }
    let info = cpu_db::platform_info(platform);
    if profile.vm.is_some() && !info.supported && cpu_db::usable_as_vm_guest(platform) {
        // A guest only sees the CPU model the hypervisor exposes; one macOS
        // does not know is not checked against platform limits, but without
        // AVX2 macOS 13+ still needs CryptexFixup (`planner::validate`).
        return if macos_db::requires_avx2(version) && !ident.has_avx2 {
            CpuVerdict::Workaround(CRYPTEX_DEFAULT.into())
        } else {
            CpuVerdict::Ok
        };
    }
    if platform == CpuPlatform::Unknown {
        return CpuVerdict::Refused("The CPU platform is unknown. Pick it in the hardware editor.".into());
    }
    if !info.supported {
        let why = info.notes.first().copied().unwrap_or_default();
        return CpuVerdict::Refused(
            format!("{} CPUs cannot run macOS. {why}", info.label)
                .trim_end()
                .to_string(),
        );
    }
    if let Some(problem) = amd_core_problem(profile, ident) {
        return CpuVerdict::Refused(problem);
    }
    if let Some(min) = info.min_macos.max(cpu_db::min_macos_for(ident)) {
        if version < min {
            return CpuVerdict::Refused(format!(
                "{} needs {} or newer.",
                info.label,
                min.display_name()
            ));
        }
    }
    let Some(max) = cpu_db::max_macos_for(ident) else {
        return CpuVerdict::Ok;
    };
    if version <= max {
        return CpuVerdict::Ok;
    }
    let native = if info.max_macos.is_none() {
        // An AVX2 platform, but this part (Pentium/Celeron) lacks AVX2.
        format!(
            "This {} model has no AVX2 and runs up to {} natively.",
            info.label,
            max.display_name()
        )
    } else {
        format!("{} runs up to {} natively.", info.label, max.display_name())
    };
    match cpu_db::ceiling_workaround_for(ident) {
        Some(w) if version >= w.from && w.max_macos.is_none_or(|m| version <= m) => {
            CpuVerdict::Workaround(format!("{native} {}", w.caveat))
        }
        Some(w) if w.max_macos.is_some_and(|m| m > max) => {
            let reach = w.max_macos.unwrap_or(max);
            CpuVerdict::Refused(format!(
                "{native} Workarounds reach {} at most.",
                reach.display_name()
            ))
        }
        _ => CpuVerdict::Refused(native),
    }
}

// ── Per-release evaluation ──────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Eval {
    /// Why the release cannot run at all.
    blockers: Vec<(&'static str, String)>,
    /// Why the release only runs through a workaround (root patches,
    /// CryptexFixup/telemetrap past the native ceiling): expert option.
    workarounds: Vec<(&'static str, String)>,
    /// Features lost or patched on this release (not recommended).
    major: Vec<(&'static str, String)>,
    /// Caveats that do not affect the recommendation.
    minor: Vec<(&'static str, String)>,
    root_patch: bool,
}

impl Eval {
    fn supported(&self) -> bool {
        self.blockers.is_empty() && self.workarounds.is_empty()
    }

    /// Reachable only as an explicit expert choice.
    fn expert_only(&self) -> bool {
        self.blockers.is_empty() && !self.workarounds.is_empty()
    }

    fn notes(&self) -> Vec<String> {
        self.blockers
            .iter()
            .chain(&self.workarounds)
            .chain(&self.major)
            .chain(&self.minor)
            .map(|(_, t)| t.clone())
            .collect()
    }
}

fn gpu_label(gpu: &ProfileGpu) -> String {
    if gpu.name.trim().is_empty() {
        gpu_db::family_label(gpu.family).to_string()
    } else {
        gpu.name.trim().to_string()
    }
}

fn native_range(min: Option<MacOsVersion>, max: Option<MacOsVersion>) -> String {
    let lo = min.unwrap_or(MacOsVersion::HighSierra);
    let hi = max.unwrap_or(MacOsVersion::Tahoe);
    if lo == hi {
        lo.display_name()
    } else {
        format!("{} to {}", lo.display_name(), hi.display_name())
    }
}

fn gpu_unavailable_reason(
    profile: &HardwareProfile,
    index: usize,
    version: MacOsVersion,
) -> String {
    let gpu = &profile.gpus[index];
    let name = gpu_label(gpu);
    if gpu.disabled {
        return format!("{name} is disabled in the profile");
    }
    if !can_drive_display(profile, index) {
        return format!("{name} is a laptop dGPU that cannot drive the screen");
    }
    let s = gpu_db::support(gpu);
    if !s.display_capable {
        return if gpu.family == GpuFamily::Unknown {
            format!("{name} could not be identified")
        } else if s.min_native.is_some() {
            format!("{name} cannot drive a display (headless only)")
        } else {
            format!("{name} has no macOS driver")
        };
    }
    match (s.min_native, s.max_native) {
        (Some(min), _) if version < min => format!("{name} needs {} or newer", min.display_name()),
        (_, Some(max)) if version > max => {
            format!("{name} is supported up to {}", max.display_name())
        }
        _ if macos_db::requires_avx2(version) && !has_avx2(profile) => {
            format!("{name} needs an AVX2 CPU on {}", version.display_name())
        }
        _ => format!("{name} is not supported on {}", version.display_name()),
    }
}

/// Intel models without graphics: "F" SKUs, Xeon E3-12x0/12x1 and Xeon
/// E-2100/2200 without the "G" suffix.
fn intel_model_without_igpu(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let tokens: Vec<&str> = upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let digits_then =
        |t: &str, n: usize| t.len() >= n && t[..n].chars().all(|c| c.is_ascii_digit());
    let f_sku = tokens
        .iter()
        .any(|t| t.len() >= 5 && digits_then(t, 4) && t.ends_with('F'));
    let xeon = upper.contains("XEON")
        && tokens.windows(2).any(|w| match w {
            ["E3", model] => {
                model.len() == 4
                    && model.starts_with("12")
                    && digits_then(model, 4)
                    && (model.ends_with('0') || model.ends_with('1'))
            }
            ["E", model] => {
                digits_then(model, 4) && model.starts_with('2') && !model.ends_with('G')
            }
            _ => false,
        });
    f_sku || xeon
}

/// Desktop Intel CPUs before Rocket Lake have a usable iGPU unless the model
/// has none; a dGPU-only scan then usually means it is off in the BIOS.
pub(crate) fn igpu_may_be_disabled(profile: &HardwareProfile) -> bool {
    use CpuPlatform as P;
    let platform_has_igpu = matches!(
        profile.cpu.platform,
        P::SandyBridge
            | P::IvyBridge
            | P::Haswell
            | P::Broadwell
            | P::Skylake
            | P::KabyLake
            | P::CoffeeLake
            | P::CometLake
    );
    platform_has_igpu
        && !intel_model_without_igpu(&profile.cpu.name)
        && !profile.gpus.iter().any(|g| g.is_igpu)
}

fn display_blocker(profile: &HardwareProfile, version: MacOsVersion) -> String {
    if profile.gpus.is_empty() {
        return "No GPU was detected, so nothing can show a picture. Add the GPU in the hardware editor.".into();
    }
    let reasons: Vec<String> = (0..profile.gpus.len())
        .map(|i| gpu_unavailable_reason(profile, i, version))
        .collect();
    let mut text = format!(
        "No GPU can drive a display on {}: {}.",
        version.display_name(),
        reasons.join("; ")
    );
    if igpu_may_be_disabled(profile) {
        text.push_str(
            " The CPU has an Intel iGPU that is not listed: enable it in the BIOS (iGPU Multi-Monitor) if no \
             supported dGPU is available.",
        );
    }
    text
}

fn evaluate(profile: &HardwareProfile, ident: &CpuIdentity, version: MacOsVersion) -> Eval {
    let mut e = Eval::default();
    match cpu_verdict(profile, ident, version) {
        CpuVerdict::Refused(reason) => {
            e.blockers.push(("cpu", reason));
            return e;
        }
        CpuVerdict::Workaround(reason) => e.workarounds.push(("cpu", reason)),
        CpuVerdict::Ok => {}
    }
    cpu_caveats(profile, ident, version, &mut e);
    display_caveats(profile, version, &mut e);
    if profile.vm.is_none() {
        audio_caveats(profile, version, &mut e);
        network_caveats(profile, version, &mut e);
    }
    e
}

const CRYPTEX_DEFAULT: &str =
    "No AVX2: macOS 13+ installs only with CryptexFixup. Delta updates are \
     unavailable and AMD Polaris/Vega/Navi GPUs lose acceleration.";

fn cpu_caveats(
    profile: &HardwareProfile,
    ident: &CpuIdentity,
    version: MacOsVersion,
    e: &mut Eval,
) {
    // Known platforms past their AVX2 ceiling are already an expert option;
    // this covers VMs with an unidentified CPU model.
    if macos_db::requires_avx2(version) && !ident.has_avx2 && e.workarounds.is_empty() {
        let caveat = cpu_db::ceiling_workaround_for(ident)
            .map(|w| w.caveat)
            .unwrap_or(CRYPTEX_DEFAULT);
        e.major.push(("cpu", caveat.to_string()));
    }
    match profile.cpu.platform {
        CpuPlatform::ArrowLake => match version {
            MacOsVersion::Sequoia => e.minor.push((
                "cpu",
                "Arrow Lake is experimental; Sequoia is the release with working reports.".into(),
            )),
            MacOsVersion::Tahoe => e.major.push((
                "cpu",
                "Arrow Lake on macOS 26 is hit-and-miss and needs AppleMCEReporterDisabler.".into(),
            )),
            _ => e
                .major
                .push(("cpu", "Arrow Lake is untested on this release.".into())),
        },
        CpuPlatform::AmdZen5 => {
            if version >= MacOsVersion::Sequoia {
                e.minor.push((
                    "cpu",
                    "Zen 5 is experimental; it is confirmed on Sequoia and Tahoe.".into(),
                ));
            } else {
                e.major
                    .push(("cpu", "Zen 5 is untested on this release.".into()));
            }
        }
        _ => {}
    }
}

fn display_caveats(profile: &HardwareProfile, version: MacOsVersion, e: &mut Eval) {
    match display_path(profile, version) {
        DisplayPath::Native(i) => {
            let gpu = &profile.gpus[i];
            let name = gpu_label(gpu);
            match gpu.family {
                GpuFamily::VirtualDisplay => e.minor.push((
                    "gpu",
                    "No GPU is passed through: the VM display has no graphics acceleration.".into(),
                )),
                GpuFamily::NvidiaMaxwell | GpuFamily::NvidiaPascal => e.minor.push((
                    "gpu",
                    format!("{name} is only accelerated with the NVIDIA Web Driver, installed after macOS."),
                )),
                GpuFamily::IntelSkylake if version >= MacOsVersion::Ventura => e.minor.push((
                    "gpu",
                    format!("{name} runs with a Kaby Lake device-id spoof on this release."),
                )),
                GpuFamily::AmdApuVega if version == MacOsVersion::Tahoe => e.minor.push((
                    "gpu",
                    "If the macOS 26 installer stalls, install with NootedRed disabled and enable it afterwards."
                        .into(),
                )),
                _ => {}
            }
            if matches!(
                gpu_db::support(gpu).requirement,
                GpuRequirement::NootRx | GpuRequirement::NootedRed
            ) {
                e.minor.push((
                    "gpu",
                    format!("{name} relies on a community driver (NootRX/NootedRed) instead of WhateverGreen."),
                ));
            }
        }
        DisplayPath::RootPatch(i) => {
            let name = gpu_label(&profile.gpus[i]);
            // research-gpu §8: root-patched systems need lowered SIP,
            // AMFIPass and SecureBootModel Disabled; macOS 26 needs OCLP 3.0.
            let oclp = if version == MacOsVersion::Tahoe {
                " macOS 26 needs OpenCore Legacy Patcher 3.0 or newer."
            } else {
                ""
            };
            e.workarounds.push((
                "gpu",
                format!(
                    "{name} has no native driver on this release: graphics acceleration only comes back with \
                     OpenCore Legacy Patcher root patches after install (lowered SIP, AMFIPass, SecureBootModel \
                     Disabled), repeated after every update. Not recommended for daily use.{oclp}"
                ),
            ));
            e.root_patch = true;
        }
        DisplayPath::Virtual => e.minor.push((
            "gpu",
            "No GPU is passed through: the VM display has no graphics acceleration.".into(),
        )),
        DisplayPath::None => e.blockers.push(("gpu", display_blocker(profile, version))),
    }
}

const TAHOE_AUDIO: &str =
    "macOS 26 removed AppleHDA: the analog outputs need VoodooHDA or an AppleHDA root \
     patch after install. HDMI/DP audio from AMD GPUs and USB audio still work.";

fn audio_caveats(profile: &HardwareProfile, version: MacOsVersion, e: &mut Eval) {
    if version == MacOsVersion::Tahoe && has_analog_audio(profile) {
        e.major.push(("audio", TAHOE_AUDIO.into()));
        e.root_patch = true;
    }
}

fn network_caveats(profile: &HardwareProfile, version: MacOsVersion, e: &mut Eval) {
    if let Some(nic) = &profile.wifi {
        let info = device_db::wifi_info(nic);
        if let Some(min) = info.min_macos.filter(|m| version < *m) {
            e.minor.push((
                "wifi",
                format!(
                    "{}: no Wi-Fi driver before {}.",
                    info.chip,
                    min.display_name()
                ),
            ));
        }
        // AirportItlwm (native Wi-Fi menu) has no build for 15 and 26, so Intel
        // cards lose the menu on Sequoia already: it never tips Sequoia vs Tahoe.
        match info.driver {
            WifiDriver::IntelItlwm if kext_catalog::airport_itlwm_id(version).is_none() => e.minor.push((
                "wifi",
                "No AirportItlwm build for this release: Intel Wi-Fi runs through itlwm and the HeliPort app \
                 (no native Wi-Fi menu, AirDrop or Continuity)."
                    .into(),
            )),
            WifiDriver::Broadcom { native_max, .. } if version > native_max => {
                let oclp = if version == MacOsVersion::Tahoe {
                    " (OCLP 3.0 or newer for macOS 26)"
                } else {
                    ""
                };
                e.major.push((
                    "wifi",
                    format!(
                        "{} is native up to {}; here Wi-Fi needs the OCLP wireless root patch after install{oclp}.",
                        info.chip,
                        native_max.display_name()
                    ),
                ));
                e.root_patch = true;
            }
            // AirPortAtheros40 can be injected up to Big Sur (device_db).
            WifiDriver::AtherosLegacy if version > MacOsVersion::BigSur => {
                e.major.push((
                    "wifi",
                    format!("{} needs the OCLP legacy wireless root patch after install.", info.chip),
                ));
                e.root_patch = true;
            }
            WifiDriver::RealtekRtw88 => e.minor.push((
                "wifi",
                format!("{} uses the experimental rtw88 driver.", info.chip),
            )),
            _ => {}
        }
    }
    if let Some(bt) = &profile.bluetooth {
        let info = device_db::bluetooth_info(bt);
        if info.driver == BluetoothDriver::IntelBluetooth && version == MacOsVersion::Tahoe {
            e.minor.push((
                "bluetooth",
                "Intel Bluetooth on macOS 26 needs the -ibtcompatbeta boot argument.".into(),
            ));
        }
    }
    for nic in &profile.ethernet {
        let info = device_db::ethernet_info(nic);
        if let Some(min) = info.min_macos.filter(|m| version < *m) {
            e.minor.push((
                "ethernet",
                format!("{}: no driver before {}.", info.chip, min.display_name()),
            ));
        }
    }
}

// ── Components ──────────────────────────────────────────────────────────────

struct Component {
    assessment: ComponentAssessment,
    issues: Vec<PlanNote>,
}

fn component(
    component: &str,
    name: impl Into<String>,
    level: SupportLevel,
    notes: Vec<String>,
) -> Component {
    Component {
        assessment: ComponentAssessment {
            component: component.into(),
            name: name.into(),
            level,
            notes,
        },
        issues: Vec::new(),
    }
}

fn cpu_component(profile: &HardwareProfile, ident: &CpuIdentity, focus: MacOsVersion) -> Component {
    let info = cpu_db::platform_info(profile.cpu.platform);
    let name = if profile.cpu.name.trim().is_empty() {
        info.label.to_string()
    } else {
        profile.cpu.name.trim().to_string()
    };
    let vm = profile.vm.is_some();
    let mut notes = Vec::new();
    if !profile.cpu.codename.trim().is_empty() && profile.cpu.codename != info.label {
        notes.push(format!("{} ({}).", info.label, profile.cpu.codename.trim()));
    }
    let level = if is_apple(profile) {
        SupportLevel::Unsupported
    } else if vm && !info.supported && cpu_db::usable_as_vm_guest(profile.cpu.platform) {
        // A VM's CPU model does not need to be one macOS knows.
        SupportLevel::Partial
    } else if profile.cpu.platform == CpuPlatform::Unknown {
        SupportLevel::Unknown
    } else if !info.supported {
        SupportLevel::Unsupported
    } else {
        let range = native_range(
            cpu_db::min_macos_for(ident).or(info.min_macos),
            cpu_db::max_macos_for(ident),
        );
        notes.push(format!("Native range: {range}."));
        if amd_core_problem(profile, ident).is_some() {
            SupportLevel::Unknown
        } else {
            let degraded = (macos_db::requires_avx2(focus) && !ident.has_avx2)
                || cpu_verdict(profile, ident, focus) != CpuVerdict::Ok;
            if community_platform(profile.cpu.platform) || degraded {
                SupportLevel::Partial
            } else {
                SupportLevel::Supported
            }
        }
    };
    notes.extend(info.notes.iter().map(|n| n.to_string()));
    if info.has_avx2 && !ident.has_avx2 && info.supported {
        if let Some(w) = cpu_db::ceiling_workaround_for(ident) {
            notes.push(w.caveat.to_string());
        }
    }
    if community_platform(profile.cpu.platform) {
        notes.push(
            "Not covered by the Dortania guide: the configuration follows community recipes."
                .into(),
        );
    }
    let mut c = component("cpu", name, level, notes);
    // An unusable core count is a blocker of every release (see cpu_verdict).
    if ident.vendor == CpuVendor::Amd
        && info.supported
        && amd_core_problem(profile, ident).is_none()
    {
        c.assessment.notes.push(format!(
            "The AMD kernel patches use {} physical cores.",
            amd_cores(profile)
        ));
    }
    if profile.cpu.threads > 64 {
        c.issues.push(note(
            NoteLevel::Warning,
            "cpu",
            "Too many threads",
            &format!(
                "macOS handles at most 64 threads; this CPU has {}. Disable Hyper-Threading/SMT in the BIOS.",
                profile.cpu.threads
            ),
        ));
    }
    c
}

fn gpu_components(profile: &HardwareProfile, focus: MacOsVersion) -> Vec<Component> {
    let primary = match display_path(profile, focus) {
        DisplayPath::Native(i) | DisplayPath::RootPatch(i) => Some(i),
        _ => None,
    };
    profile
        .gpus
        .iter()
        .enumerate()
        .map(|(i, gpu)| {
            let s = gpu_db::support(gpu);
            let mut notes = Vec::new();
            let level = if gpu.disabled {
                notes.push("Disabled in the profile: macOS will not use it.".into());
                if s.display_capable {
                    SupportLevel::Partial
                } else {
                    SupportLevel::Unsupported
                }
            } else if !can_drive_display(profile, i) {
                notes.push(
                    "Laptop dGPU without a display MUX: macOS cannot drive the internal screen through it, so it \
                     is turned off."
                        .into(),
                );
                SupportLevel::Unsupported
            } else {
                match gpu_status(profile, gpu, focus) {
                    GpuStatus::Native => {
                        if primary == Some(i) {
                            notes.push(format!("Drives the displays on {}.", focus.display_name()));
                        } else if gpu.is_igpu {
                            notes.push("Runs headless next to the dGPU (Quick Sync, hardware video decoding).".into());
                        } else {
                            notes.push("Supported, but another GPU drives the displays.".into());
                        }
                        if gpu.family == GpuFamily::VirtualDisplay
                            || matches!(s.requirement, GpuRequirement::NootRx | GpuRequirement::NootedRed)
                        {
                            SupportLevel::Partial
                        } else {
                            SupportLevel::Supported
                        }
                    }
                    GpuStatus::RootPatch => {
                        notes.push(format!(
                            "On {} only through OCLP root patches after install.",
                            focus.display_name()
                        ));
                        SupportLevel::Partial
                    }
                    GpuStatus::Unavailable => {
                        if s.display_capable {
                            notes.push(format!(
                                "Not available on {}; native range {}.",
                                focus.display_name(),
                                native_range(s.min_native, s.max_native)
                            ));
                            SupportLevel::Partial
                        } else if gpu.family == GpuFamily::Unknown {
                            SupportLevel::Unknown
                        } else if s.min_native.is_some() && gpu.is_igpu {
                            notes.push("Cannot drive a display; usable headless only.".into());
                            SupportLevel::Partial
                        } else {
                            notes.push("No macOS driver: it is disabled in the build.".into());
                            SupportLevel::Unsupported
                        }
                    }
                }
            };
            notes.extend(s.notes);
            component("gpu", gpu_label(gpu), level, notes)
        })
        .collect()
}

fn audio_component(profile: &HardwareProfile, focus: MacOsVersion) -> Option<Component> {
    let Some(audio) = &profile.audio else {
        if profile.vm.is_some() {
            return None;
        }
        return Some(component(
            "audio",
            "No analog codec",
            SupportLevel::Unknown,
            vec![
                "No analog HD Audio codec was detected (HDMI/DP audio only, USB audio, or a missing codec driver)."
                    .into(),
            ],
        ));
    };
    let name = if audio.codec_name.trim().is_empty() {
        "HD Audio codec".to_string()
    } else {
        audio.codec_name.trim().to_string()
    };
    let mut c = match audio.codec_id {
        Some(id) if codec_db::is_supported(id) => {
            let laptop = profile.form_factor == FormFactor::Laptop;
            let ranked = codec_db::ranked_layouts(id, None, laptop);
            let shown: Vec<String> = ranked.iter().take(8).map(u32::to_string).collect();
            let mut notes = vec![format!("AppleALC layouts, best first: {}.", shown.join(", "))];
            let mut issues = Vec::new();
            if let Some(layout) = audio.layout_id {
                if ranked.contains(&layout) {
                    notes.push(format!("Layout {layout} is set in the profile."));
                } else {
                    issues.push(note(
                        NoteLevel::Warning,
                        "audio",
                        "Layout-id not available",
                        &format!("AppleALC has no layout {layout} for {name}; pick one of {}.", shown.join(", ")),
                    ));
                }
            }
            let level = if focus == MacOsVersion::Tahoe {
                notes.push(TAHOE_AUDIO.into());
                SupportLevel::Partial
            } else {
                SupportLevel::Supported
            };
            let mut c = component("audio", name, level, notes);
            c.issues = issues;
            c
        }
        Some(_) => {
            let mut c = component(
                "audio",
                name.clone(),
                SupportLevel::Unsupported,
                vec!["AppleALC has no layouts for this codec: VoodooHDA or a USB audio device are the alternatives.".into()],
            );
            c.issues.push(note(
                NoteLevel::Warning,
                "audio",
                "Codec not supported by AppleALC",
                &format!("{name} has no AppleALC layout, so onboard audio will not work out of the box."),
            ));
            c
        }
        None => component(
            "audio",
            name,
            SupportLevel::Unknown,
            vec!["The codec is unknown: pick it in the hardware editor so a valid layout-id can be chosen.".into()],
        ),
    };
    if profile.vm.is_some() {
        c.assessment.level = SupportLevel::Unknown;
    }
    Some(c)
}

fn ethernet_works(profile: &HardwareProfile, focus: MacOsVersion) -> bool {
    profile.ethernet.iter().any(|nic| {
        let info = device_db::ethernet_info(nic);
        info.driver != EthernetDriver::Unsupported && info.min_macos.is_none_or(|m| focus >= m)
    })
}

fn network_components(profile: &HardwareProfile, focus: MacOsVersion) -> Vec<Component> {
    let mut out = Vec::new();
    for nic in &profile.ethernet {
        let info = device_db::ethernet_info(nic);
        let level = if info.driver == EthernetDriver::Unsupported {
            SupportLevel::Unsupported
        } else if info.min_macos.is_some_and(|m| focus < m) {
            SupportLevel::Partial
        } else {
            SupportLevel::Supported
        };
        out.push(component(
            "ethernet",
            info.chip.clone(),
            level,
            info.notes.clone(),
        ));
    }
    let mut wifi_in_recovery = false;
    if let Some(nic) = &profile.wifi {
        let info = device_db::wifi_info(nic);
        let mut notes = info.notes.clone();
        let level = match info.driver {
            WifiDriver::IntelItlwm => {
                let airport = kext_catalog::airport_itlwm_id(focus).is_some();
                wifi_in_recovery = airport;
                notes.insert(
                    0,
                    if airport {
                        "AirportItlwm gives the native Wi-Fi menu; AirDrop and Continuity do not work.".into()
                    } else {
                        "itlwm with the HeliPort app on this release (no native Wi-Fi menu, not available in macOS \
                         Recovery)."
                            .to_string()
                    },
                );
                SupportLevel::Partial
            }
            WifiDriver::Broadcom { native_max, .. } => {
                if focus <= native_max {
                    wifi_in_recovery = true;
                    SupportLevel::Supported
                } else {
                    notes.insert(
                        0,
                        format!(
                            "Native up to {}; newer releases need the OCLP wireless root patch.",
                            native_max.display_name()
                        ),
                    );
                    SupportLevel::Partial
                }
            }
            // AirPortAtheros40 is native or injected by OpenCore up to Big
            // Sur, so it also works in Recovery there (device_db).
            WifiDriver::AtherosLegacy => {
                if focus <= MacOsVersion::BigSur {
                    wifi_in_recovery = true;
                    SupportLevel::Supported
                } else {
                    SupportLevel::Partial
                }
            }
            WifiDriver::RealtekRtw88 => SupportLevel::Partial,
            WifiDriver::Unsupported => SupportLevel::Unsupported,
        };
        let mut c = component("wifi", info.chip.clone(), level, notes);
        if level == SupportLevel::Unsupported {
            c.issues.push(note(
                NoteLevel::Warning,
                "wifi",
                "Wi-Fi card not supported",
                &format!(
                    "{} has no macOS driver. Use Ethernet or replace it with a supported card (Broadcom BCM94360 / \
                     BCM94352 family, or an Intel card for itlwm).",
                    info.chip
                ),
            ));
        }
        out.push(c);
    }
    if profile.vm.is_none() && !ethernet_works(profile, focus) && !wifi_in_recovery {
        let mut issue = note(
            NoteLevel::Warning,
            "network",
            "No network in macOS Recovery",
            "The installer downloads macOS from Apple, but no network adapter here works in macOS Recovery. \
             Connect a supported Ethernet adapter (USB adapters with ASIX/Realtek chips work) for the install.",
        );
        if profile.wifi.is_some() && profile.ethernet.is_empty() {
            issue.detail.push_str(" Wi-Fi works after installation.");
        }
        if let Some(first) = out.first_mut() {
            first.issues.push(issue);
        } else {
            let mut c = component(
                "ethernet",
                "No network adapter",
                SupportLevel::Unknown,
                vec!["No Ethernet or Wi-Fi adapter was detected.".into()],
            );
            c.issues.push(issue);
            out.push(c);
        }
    }
    if let Some(bt) = &profile.bluetooth {
        let info = device_db::bluetooth_info(bt);
        let level = match info.driver {
            BluetoothDriver::Unsupported => SupportLevel::Unsupported,
            BluetoothDriver::Realtek => SupportLevel::Partial,
            _ => SupportLevel::Supported,
        };
        out.push(component(
            "bluetooth",
            info.chip.clone(),
            level,
            info.notes.clone(),
        ));
    }
    out
}

fn input_component(profile: &HardwareProfile) -> Option<Component> {
    let input = &profile.input;
    let laptop = profile.form_factor == FormFactor::Laptop;
    if profile.vm.is_some() {
        return None;
    }
    let mut notes = Vec::new();
    let mut level = SupportLevel::Supported;
    match input.keyboard_bus {
        InputBus::Ps2 => notes.push("PS/2 keyboard: VoodooPS2Controller.".into()),
        InputBus::Usb => notes.push("USB keyboard: works natively.".into()),
        InputBus::I2c | InputBus::Smbus => {
            notes.push("I2C keyboard: VoodooI2C.".into());
            level = SupportLevel::Partial;
        }
        InputBus::Unknown => {
            if laptop {
                notes.push("Keyboard bus unknown; laptop keyboards are almost always PS/2.".into());
            }
        }
    }
    let touchpad = device_db::touchpad_driver(
        input.touchpad_bus,
        input.touchpad_vendor,
        input.touchpad_hid.as_deref(),
    );
    match touchpad {
        TouchpadDriver::Ps2 => notes.push("PS/2 touchpad: VoodooPS2Trackpad.".into()),
        TouchpadDriver::I2cHid | TouchpadDriver::RmiI2c | TouchpadDriver::AlpsHid => {
            let stack = match touchpad {
                TouchpadDriver::RmiI2c => "VoodooRMI over VoodooI2C",
                TouchpadDriver::AlpsHid => "AlpsHID over VoodooI2C",
                _ => "VoodooI2C + VoodooI2CHID",
            };
            notes.push(format!(
                "I2C touchpad ({}): {stack}. It needs a working GPIO interrupt; polling mode is the fallback.",
                input.touchpad_hid.as_deref().unwrap_or("unknown id")
            ));
            level = SupportLevel::Partial;
        }
        TouchpadDriver::RmiSmbus | TouchpadDriver::ElanSmbus => {
            notes.push("SMBus touchpad: VoodooRMI / VoodooSMBus.".into());
            level = SupportLevel::Partial;
        }
        TouchpadDriver::None => {
            if input.touchpad_bus == Some(InputBus::Usb) {
                notes.push("USB touchpad: works as a plain HID device.".into());
            } else if laptop {
                notes.push(
                    "No touchpad was detected; set its bus and id in the hardware editor.".into(),
                );
                level = SupportLevel::Unknown;
            }
        }
    }
    if input.has_touchscreen {
        notes.push("Touchscreen: basic touch through VoodooI2C when it is an I2C device.".into());
    }
    if !laptop && input.touchpad_bus.is_none() && notes.is_empty() {
        notes.push("USB keyboards and mice work natively.".into());
    }
    let name = match (laptop, input.touchpad_bus) {
        (true, _) | (_, Some(_)) => "Keyboard and touchpad",
        _ => "Keyboard and mouse",
    };
    Some(component("input", name, level, notes))
}

fn storage_component(profile: &HardwareProfile) -> Component {
    if profile.storage.is_empty() {
        return component(
            "storage",
            "Drives",
            SupportLevel::Unknown,
            vec!["No drives were detected.".into()],
        );
    }
    let mut notes = Vec::new();
    let mut issues = Vec::new();
    let mut problematic = 0;
    let mut nvmefix = false;
    for drive in &profile.storage {
        let advice = device_db::storage_advice(drive);
        nvmefix |= advice.nvmefix;
        let name = if drive.name.trim().is_empty() {
            "Drive".to_string()
        } else {
            drive.name.trim().to_string()
        };
        if advice.problematic {
            problematic += 1;
            issues.push(note(
                NoteLevel::Warning,
                "storage",
                &format!("{name} cannot be used"),
                &advice.notes.join(" "),
            ));
        } else {
            notes.extend(advice.notes.iter().map(|n| format!("{name}: {n}")));
        }
    }
    if nvmefix {
        notes.push("NVMeFix improves power management of non-Apple NVMe drives.".into());
    }
    let usable = profile.storage.len() - problematic;
    let usable_internal = profile
        .storage
        .iter()
        .filter(|d| d.kind != StorageKind::Usb && !device_db::storage_advice(d).problematic)
        .count();
    if usable_internal == 0 && usable > 0 {
        notes.push("Only external USB drives are usable as macOS targets.".into());
    }
    let level = if problematic == 0 {
        SupportLevel::Supported
    } else if usable > 0 {
        SupportLevel::Partial
    } else {
        SupportLevel::Unsupported
    };
    let summary = format!("{} drive(s)", profile.storage.len());
    let mut c = component("storage", summary, level, notes);
    c.issues = issues;
    c
}

/// Platforms whose boards often have no UEFI at all (critic-gaps §1;
/// Dortania Penryn, Clarkdale, Nehalem and Sandy Bridge pages).
fn legacy_era(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::Penryn
            | CpuPlatform::Lynnfield
            | CpuPlatform::Arrandale
            | CpuPlatform::NehalemHedt
            | CpuPlatform::SandyBridge
    )
}

/// The scan ran in legacy BIOS mode on a platform whose boards often lack
/// UEFI: OpenCore then starts through OpenDuet. A legacy (CSM) boot on a
/// newer board means "switch the firmware to UEFI" instead (critic-gaps §1).
pub fn legacy_boot_only(profile: &HardwareProfile) -> bool {
    profile.vm.is_none() && profile.firmware_uefi == Some(false) && legacy_era(profile.cpu.platform)
}

fn vm_label(kind: VmKind) -> &'static str {
    match kind {
        VmKind::Kvm => "QEMU/KVM",
        VmKind::Vmware => "VMware",
        VmKind::HyperV => "Hyper-V",
        VmKind::VirtualBox => "VirtualBox",
        VmKind::Parallels => "Parallels",
        VmKind::Other => "other hypervisor",
    }
}

fn vm_notes(kind: VmKind) -> &'static str {
    match kind {
        VmKind::Kvm => {
            "QEMU/KVM: use OVMF (UEFI), a CPU model macOS knows (host or Haswell/Skylake class; AMD hosts need an \
             Intel model) and vmxnet3, e1000-82545em or VirtIO networking."
        }
        VmKind::Vmware => "VMware: macOS guests need a patched (unlocked) VMware Workstation/Player on PCs.",
        VmKind::HyperV => "Hyper-V: a Generation 2 VM with Secure Boot off; MacHyperVSupport provides the drivers.",
        VmKind::VirtualBox => "VirtualBox: macOS guests run without graphics acceleration and are slow.",
        VmKind::Parallels => "Parallels: macOS guests are only supported on Mac hosts.",
        VmKind::Other => "Virtual machine: only the emulated devices are available to macOS.",
    }
}

fn platform_component(profile: &HardwareProfile, focus: MacOsVersion) -> Component {
    let mut notes = Vec::new();
    let mut issues = Vec::new();
    let mut level = SupportLevel::Supported;
    let name = match profile.vm {
        Some(kind) => {
            notes.push(vm_notes(kind).to_string());
            level = SupportLevel::Partial;
            format!("Virtual machine ({})", vm_label(kind))
        }
        None => {
            let board = format!(
                "{} {}",
                profile.motherboard_vendor.trim(),
                profile.motherboard_model.trim()
            )
            .trim()
            .to_string();
            match (&profile.chipset, board.is_empty()) {
                (Some(chipset), false) => format!("{board} ({chipset})"),
                (Some(chipset), true) => chipset.clone(),
                (None, false) => board,
                (None, true) => "Motherboard".to_string(),
            }
        }
    };
    if profile.firmware_uefi == Some(false) && profile.vm.is_none() {
        level = SupportLevel::Partial;
        let detail = if legacy_boot_only(profile) {
            "The system booted in legacy BIOS mode and the board may have no UEFI. Boards without UEFI need \
             OpenCore's legacy boot sector (OpenDuet, Utilities/LegacyBoot) on the USB drive."
        } else {
            "The OS was started in legacy (CSM) mode. Switch the firmware to UEFI boot and turn CSM off before \
             booting OpenCore."
        };
        issues.push(note(
            NoteLevel::Warning,
            "platform",
            "Legacy BIOS boot",
            detail,
        ));
    }
    if profile.vm.is_none() && !is_apple(profile) {
        issues.push(note(
            NoteLevel::Warning,
            "platform",
            "BitLocker and dual boot",
            "Turning Secure Boot off, changing the boot order or repartitioning makes Windows BitLocker / Device \
             Encryption ask for its 48-digit recovery key. Save the key (aka.ms/myrecoverykey) and suspend \
             BitLocker before changing BIOS settings, and install macOS on its own drive.",
        ));
        notes.push(
            "Secure Boot must be off for OpenCore (or OpenCore signed with your own keys); turn it back on only \
             after switching back to the firmware's Windows Boot Manager."
                .into(),
        );
    }
    match modern_standby(profile) {
        ModernStandby::Likely => {
            level = SupportLevel::Partial;
            issues.push(note(
                NoteLevel::Warning,
                "platform",
                "Sleep",
                "Laptops of this generation usually only offer Modern Standby (S0ix). macOS needs S3 sleep, so \
                 sleep will not work unless the BIOS has an S3 / \"Linux\" sleep option; disable sleep and \
                 hibernation otherwise.",
            ));
        }
        ModernStandby::Possible => notes.push(
            "Many laptops of this generation only offer Modern Standby (S0ix); macOS sleep needs S3. Look for an \
             S3 / \"Linux\" sleep option in the BIOS."
                .into(),
        ),
        ModernStandby::Unlikely => {}
    }
    if profile.ram_gb > 0 && profile.ram_gb < 4 {
        level = SupportLevel::Partial;
        issues.push(note(
            NoteLevel::Warning,
            "platform",
            "Not enough memory",
            &format!(
                "Apple requires 4 GB of RAM from Big Sur on; this machine reports {} GB.",
                profile.ram_gb
            ),
        ));
    } else if profile.ram_gb > 0 && profile.ram_gb < 8 && focus >= MacOsVersion::Sonoma {
        notes.push(format!(
            "{} GB of RAM works, but 8 GB or more is recommended for current releases.",
            profile.ram_gb
        ));
    }
    let mut c = component("platform", name, level, notes);
    c.issues = issues;
    c
}

// ── Report ──────────────────────────────────────────────────────────────────

/// Contiguous runs: "macOS High Sierra 10.13 to macOS Monterey 12, macOS Tahoe 26".
fn describe_versions(versions: &[MacOsVersion]) -> String {
    let mut runs: Vec<(MacOsVersion, MacOsVersion)> = Vec::new();
    for &v in versions {
        let idx = MacOsVersion::ALL.iter().position(|x| *x == v).unwrap_or(0);
        match runs.last_mut() {
            Some((_, end))
                if MacOsVersion::ALL
                    .iter()
                    .position(|x| x == end)
                    .is_some_and(|e| e + 1 == idx) =>
            {
                *end = v;
            }
            _ => runs.push((v, v)),
        }
    }
    runs.iter()
        .map(|(a, b)| native_range(Some(*a), Some(*b)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn confidence(profile: &HardwareProfile) -> f64 {
    let mut c = if profile.scan_confidence > 0.0 {
        profile.scan_confidence.min(1.0)
    } else {
        0.6
    };
    if profile.cpu.platform == CpuPlatform::Unknown && profile.vm.is_none() {
        c *= 0.5;
    }
    if profile.gpus.iter().any(|g| g.family == GpuFamily::Unknown) {
        c *= 0.8;
    }
    if community_platform(profile.cpu.platform) {
        c *= 0.85;
    }
    if profile.vm.is_some() {
        c *= 0.9;
    }
    if profile.audio.is_none() && profile.vm.is_none() {
        c *= 0.95;
    }
    ((c * 100.0).round() / 100.0).clamp(0.0, 1.0)
}

/// Why releases newer than `rec` are passed over: their feature losses
/// (supported) or the workaround they need (expert options). Releases with
/// the same reason are grouped ("macOS Ventura 13 to macOS Tahoe 26: ...").
fn passed_over(evals: &[(MacOsVersion, Eval)], rec: MacOsVersion) -> Vec<String> {
    let mut groups: Vec<(Vec<MacOsVersion>, String)> = Vec::new();
    for (v, e) in evals.iter().filter(|(v, _)| *v > rec) {
        let reasons: Vec<&str> = if e.supported() {
            e.major.iter().map(|(_, t)| t.as_str()).collect()
        } else if e.expert_only() {
            e.workarounds.iter().map(|(_, t)| t.as_str()).collect()
        } else {
            Vec::new()
        };
        if reasons.is_empty() {
            continue;
        }
        let text = reasons.join(" ");
        match groups.last_mut() {
            Some((versions, last)) if *last == text => versions.push(*v),
            _ => groups.push((vec![*v], text)),
        }
    }
    groups
        .into_iter()
        .map(|(versions, text)| format!("{}: {text}", describe_versions(&versions)))
        .collect()
}

/// Assess `profile`. When `target` is None the report is computed for the
/// recommended release. A release is "supported" when the CPU allows it
/// natively (the planner's CPU check) AND at least one display path (iGPU or
/// dGPU, not disabled) is natively supported on it (VMs excepted). Releases
/// that only work through OCLP root patches or a CPU workaround are listed as
/// unsupported with `needs_root_patch` / notes, and a report targeting one of
/// them is `Partial` (expert option) rather than `Unsupported`. The
/// recommended release is the newest fully supported one, preferring Sequoia
/// over Tahoe when Tahoe loses features this machine needs (analog audio via
/// AppleHDA, Broadcom Wi-Fi without root patch). Intel Wi-Fi does not tip
/// the balance: neither release has an AirportItlwm build.
pub fn assess(profile: &HardwareProfile, target: Option<MacOsVersion>) -> CompatibilityReport {
    let ident = cpu_identity(profile);
    let evals: Vec<(MacOsVersion, Eval)> = MacOsVersion::ALL
        .iter()
        .map(|&v| (v, evaluate(profile, &ident, v)))
        .collect();
    let supported: Vec<MacOsVersion> = evals
        .iter()
        .filter(|(_, e)| e.supported())
        .map(|(v, _)| *v)
        .collect();
    let expert: Vec<MacOsVersion> = evals
        .iter()
        .filter(|(_, e)| e.expert_only())
        .map(|(v, _)| *v)
        .collect();
    let recommended = evals
        .iter()
        .rev()
        .find(|(_, e)| e.supported() && e.major.is_empty())
        .or_else(|| evals.iter().rev().find(|(_, e)| e.supported()))
        .map(|(v, _)| *v);

    let selected = target.or(recommended);
    let focus = selected
        .or_else(|| {
            evals
                .iter()
                .rev()
                .find(|(_, e)| !e.blockers.iter().any(|(c, _)| *c == "cpu"))
                .map(|(v, _)| *v)
        })
        .unwrap_or(MacOsVersion::Sequoia);
    let eval_of = |v: MacOsVersion| evals.iter().find(|(x, _)| *x == v).map(|(_, e)| e);

    let mut versions: Vec<MacOsOption> = evals
        .iter()
        .map(|(v, e)| MacOsOption {
            version: *v,
            name: v.display_name(),
            supported: e.supported(),
            recommended: Some(*v) == recommended,
            notes: e.notes(),
            needs_root_patch: e.root_patch,
        })
        .collect();

    // Why newer releases are not recommended.
    let why_not_newer = recommended
        .map(|rec| passed_over(&evals, rec))
        .unwrap_or_default();
    if let Some(rec) = recommended.filter(|_| !why_not_newer.is_empty()) {
        if let Some(option) = versions.iter_mut().find(|o| o.version == rec) {
            option.notes.insert(
                0,
                "Recommended: newer releases lose features or need workarounds on this machine (see their notes)."
                    .into(),
            );
        }
    }

    let mut components: Vec<Component> = vec![cpu_component(profile, &ident, focus)];
    components.extend(gpu_components(profile, focus));
    components.extend(audio_component(profile, focus));
    components.extend(network_components(profile, focus));
    components.extend(input_component(profile));
    components.push(storage_component(profile));
    components.push(platform_component(profile, focus));

    // Notes for the selected (or focus) release: blocking, then warnings, then info.
    let mut notes: Vec<PlanNote> = Vec::new();
    if let Some(e) = eval_of(focus) {
        let title = |kind: &str| format!("{kind} on {}", focus.display_name());
        for (component, text) in &e.blockers {
            notes.push(note(
                NoteLevel::Blocking,
                component,
                &title("Not supported"),
                text,
            ));
        }
        for (component, text) in &e.workarounds {
            notes.push(note(
                NoteLevel::Warning,
                component,
                &title("Only with workarounds"),
                text,
            ));
        }
        for (component, text) in &e.major {
            notes.push(note(
                NoteLevel::Warning,
                component,
                &title("Limitation"),
                text,
            ));
        }
    }
    for c in &components {
        notes.extend(
            c.issues
                .iter()
                .filter(|n| n.level != NoteLevel::Info)
                .cloned(),
        );
    }
    if let Some(e) = eval_of(focus) {
        for (component, text) in &e.minor {
            notes.push(note(
                NoteLevel::Info,
                component,
                &format!("Caveat on {}", focus.display_name()),
                text,
            ));
        }
    }
    for c in &components {
        notes.extend(
            c.issues
                .iter()
                .filter(|n| n.level == NoteLevel::Info)
                .cloned(),
        );
    }
    if let Some(rec) = recommended {
        if !why_not_newer.is_empty() && selected.is_none_or(|s| s >= rec) {
            notes.push(note(
                NoteLevel::Info,
                "macos",
                &format!("Why {} is recommended", rec.display_name()),
                &why_not_newer.join(" "),
            ));
        }
    }
    dedupe_notes(&mut notes);
    notes.sort_by_key(|n| match n.level {
        NoteLevel::Blocking => 0,
        NoteLevel::Warning => 1,
        NoteLevel::Info => 2,
    });

    let selected_eval = selected.and_then(eval_of);
    let selected_supported = selected_eval.is_some_and(Eval::supported);
    let selected_expert = selected_eval.is_some_and(Eval::expert_only);
    let platform_unknown =
        profile.cpu.platform == CpuPlatform::Unknown && profile.vm.is_none() && !is_apple(profile);
    let cores_unknown = cpu_db::platform_info(profile.cpu.platform).supported
        && ident.vendor == CpuVendor::Amd
        && amd_cores(profile) == 0;
    let caveats = selected_eval.is_some_and(|e| !e.major.is_empty())
        || components.iter().any(|c| {
            matches!(
                c.assessment.level,
                SupportLevel::Partial | SupportLevel::Unsupported
            )
        });
    let level = if platform_unknown || cores_unknown {
        SupportLevel::Unknown
    } else if selected_supported {
        if caveats {
            SupportLevel::Partial
        } else {
            SupportLevel::Supported
        }
    } else if selected_expert {
        SupportLevel::Partial
    } else {
        SupportLevel::Unsupported
    };

    let first_reason = eval_of(focus)
        .and_then(|e| e.blockers.first().or(e.workarounds.first()))
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
    let fully = |list: &[MacOsVersion]| {
        if list.is_empty() {
            " No release is fully supported.".to_string()
        } else {
            format!(" Fully supported: {}.", describe_versions(list))
        }
    };
    let summary = if is_apple(profile) {
        "This Mac already runs macOS natively; OpenCore EFIs are for Intel and AMD PCs.".to_string()
    } else if platform_unknown {
        "The CPU could not be identified. Pick its platform in the hardware editor to see which macOS releases \
         fit."
            .to_string()
    } else if cores_unknown {
        "The number of CPU cores is unknown; the AMD kernel patches need it. Enter it in the hardware editor to \
         see which macOS releases fit."
            .to_string()
    } else if selected_supported {
        let verdict = if level == SupportLevel::Supported {
            "is fully supported"
        } else {
            "works with caveats"
        };
        let rec = match recommended {
            Some(r) if Some(r) != selected => format!(" Recommended: {}.", r.display_name()),
            Some(_) => " It is the recommended release.".to_string(),
            None => String::new(),
        };
        format!(
            "{} {verdict}. This machine can run {}.{rec}",
            focus.display_name(),
            describe_versions(&supported)
        )
    } else if selected_expert {
        format!(
            "{} only works on this machine with workarounds (an expert option, not recommended). {first_reason}{}",
            focus.display_name(),
            fully(&supported)
        )
    } else if supported.is_empty() {
        let mut text = format!("No macOS release runs natively on this machine. {first_reason}");
        if !expert.is_empty() {
            text.push_str(&format!(
                " Expert options with workarounds: {}.",
                describe_versions(&expert)
            ));
        }
        text
    } else {
        format!(
            "{} cannot run on this machine. {first_reason} Supported: {}.",
            focus.display_name(),
            describe_versions(&supported)
        )
    };

    CompatibilityReport {
        level,
        summary,
        target: selected,
        recommended,
        versions,
        components: components.into_iter().map(|c| c.assessment).collect(),
        notes,
        confidence: confidence(profile),
    }
}

/// Keep the first note per (component, title, detail).
fn dedupe_notes(notes: &mut Vec<PlanNote>) {
    let mut seen = std::collections::HashSet::new();
    notes.retain(|n| seen.insert((n.component.clone(), n.title.clone(), n.detail.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        DeviceBus, ProfileAudio, ProfileCpu, ProfileInput, ProfileNic, ProfileStorage,
    };
    use crate::domain::profile::{build_profile, fixtures};

    fn cpu(platform: CpuPlatform, name: &str) -> ProfileCpu {
        let info = cpu_db::platform_info(platform);
        ProfileCpu {
            name: name.into(),
            vendor: info.vendor,
            platform,
            codename: String::new(),
            cores: 4,
            threads: 8,
            has_avx2: Some(info.has_avx2),
            has_sse4_2: Some(platform != CpuPlatform::Penryn),
            ..Default::default()
        }
    }

    fn gpu(vendor: &str, device: &str, name: &str, path: &str) -> ProfileGpu {
        let id = gpu_db::identify(Some(vendor), Some(device), name);
        ProfileGpu {
            name: name.into(),
            vendor: id.vendor,
            family: id.family,
            vendor_id: Some(vendor.into()),
            device_id: Some(device.into()),
            is_igpu: id.is_igpu,
            pci_path: Some(path.into()),
            ..Default::default()
        }
    }

    fn igpu(device: &str, name: &str) -> ProfileGpu {
        gpu("8086", device, name, "PciRoot(0x0)/Pci(0x2,0x0)")
    }

    fn dgpu(vendor: &str, device: &str, name: &str) -> ProfileGpu {
        gpu(
            vendor,
            device,
            name,
            "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)",
        )
    }

    fn desktop(platform: CpuPlatform, gpus: Vec<ProfileGpu>) -> HardwareProfile {
        HardwareProfile {
            cpu: cpu(platform, "Test CPU"),
            form_factor: FormFactor::Desktop,
            gpus,
            ram_gb: 16,
            firmware_uefi: Some(true),
            source: "manual".into(),
            ..Default::default()
        }
    }

    fn alc(codec: u32) -> Option<ProfileAudio> {
        Some(ProfileAudio {
            codec_name: codec_db::codec_name(codec),
            codec_id: Some(codec),
            controller_pci_path: Some("PciRoot(0x0)/Pci(0x1f,0x3)".into()),
            ..Default::default()
        })
    }

    fn option(report: &CompatibilityReport, v: MacOsVersion) -> &MacOsOption {
        report
            .versions
            .iter()
            .find(|o| o.version == v)
            .expect("version listed")
    }

    fn supported(report: &CompatibilityReport) -> Vec<MacOsVersion> {
        report
            .versions
            .iter()
            .filter(|o| o.supported)
            .map(|o| o.version)
            .collect()
    }

    /// The CPU checks of `planner::validate` (the full planner is compared in
    /// `planner::consistency_tests`).
    fn planner_accepts(profile: &HardwareProfile, target: MacOsVersion) -> bool {
        let options = crate::domain::model::BuildOptions {
            target,
            ..Default::default()
        };
        crate::domain::planner::validate(profile, &options).is_ok()
    }

    #[test]
    fn z390_with_alc1220_prefers_sequoia() {
        let p = build_profile(&fixtures::windows_z390());
        let r = assess(&p, None);
        // The RX 580 drives the displays, so the UHD 630's 10.14 floor does not apply.
        assert_eq!(supported(&r).first(), Some(&MacOsVersion::HighSierra));
        let mut igpu_only = p.clone();
        igpu_only.gpus.truncate(1);
        assert_eq!(
            supported(&assess(&igpu_only, None)).first(),
            Some(&MacOsVersion::Mojave)
        );
        assert!(option(&r, MacOsVersion::Tahoe).supported);
        assert!(option(&r, MacOsVersion::Tahoe).needs_root_patch, "AppleHDA");
        assert_eq!(r.recommended, Some(MacOsVersion::Sequoia));
        assert_eq!(r.target, Some(MacOsVersion::Sequoia));
        assert!(option(&r, MacOsVersion::Sequoia).recommended);
        assert!(r
            .notes
            .iter()
            .any(|n| n.title.contains("Why") && n.detail.contains("AppleHDA")));
        assert_ne!(r.level, SupportLevel::Unsupported);
        let gpus: Vec<_> = r
            .components
            .iter()
            .filter(|c| c.component == "gpu")
            .collect();
        assert_eq!(gpus.len(), 2);
        assert!(
            gpus[1].notes[0].starts_with("Drives the displays"),
            "{:?}",
            gpus[1].notes
        );
        assert!(gpus[0].notes[0].contains("headless"), "{:?}", gpus[0].notes);
        assert!(r.confidence > 0.8);
        assert!(r.summary.contains("Sequoia"), "{}", r.summary);

        let tahoe = assess(&p, Some(MacOsVersion::Tahoe));
        assert_eq!(tahoe.target, Some(MacOsVersion::Tahoe));
        assert_eq!(tahoe.level, SupportLevel::Partial);
        assert!(tahoe
            .notes
            .iter()
            .any(|n| n.level == NoteLevel::Warning && n.component == "audio"));
    }

    #[test]
    fn whiskey_lake_laptop_with_pm981() {
        let p = build_profile(&fixtures::linux_laptop_i2c());
        let r = assess(&p, None);
        // Whiskey Lake starts with 10.14.1.
        assert!(!option(&r, MacOsVersion::HighSierra).supported);
        assert!(option(&r, MacOsVersion::Mojave).supported);
        assert_eq!(r.recommended, Some(MacOsVersion::Sequoia));
        let storage = r
            .components
            .iter()
            .find(|c| c.component == "storage")
            .expect("storage");
        assert_eq!(storage.level, SupportLevel::Unsupported);
        assert!(r
            .notes
            .iter()
            .any(|n| n.component == "storage" && n.level == NoteLevel::Warning));
        let input = r
            .components
            .iter()
            .find(|c| c.component == "input")
            .expect("input");
        assert_eq!(input.level, SupportLevel::Partial);
        assert!(input.notes.iter().any(|n| n.contains("SYNA3602")));
        // Intel Wi-Fi without Ethernet: no network in Recovery on Sequoia.
        assert!(r.notes.iter().any(|n| n.component == "network"));
        let sonoma = assess(&p, Some(MacOsVersion::Sonoma));
        assert!(!sonoma.notes.iter().any(|n| n.component == "network"));
        assert!(r
            .notes
            .iter()
            .any(|n| n.title == "Sleep" || n.title == "BitLocker and dual boot"));
    }

    #[test]
    fn amd_b550_rx6600() {
        let p = build_profile(&fixtures::amd_b550());
        let r = assess(&p, None);
        // Navi 23 is native from 12.1.
        assert_eq!(supported(&r).first(), Some(&MacOsVersion::Monterey));
        assert!(!option(&r, MacOsVersion::BigSur).supported);
        assert!(option(&r, MacOsVersion::BigSur).notes[0].contains("needs macOS Monterey 12"));
        assert_eq!(r.recommended, Some(MacOsVersion::Sequoia));
        let cpu = &r.components[0];
        assert_eq!(cpu.level, SupportLevel::Supported);
        assert!(cpu.notes.iter().any(|n| n.contains("6 physical cores")));
        let wifi = r
            .components
            .iter()
            .find(|c| c.component == "wifi")
            .expect("wifi");
        assert_eq!(wifi.level, SupportLevel::Partial);
    }

    #[test]
    fn vm_supports_everything_the_cpu_allows() {
        let p = build_profile(&fixtures::kvm_guest());
        let r = assess(&p, None);
        assert_eq!(supported(&r).len(), 9);
        assert_eq!(r.recommended, Some(MacOsVersion::Tahoe));
        assert!(option(&r, MacOsVersion::Tahoe)
            .notes
            .iter()
            .any(|n| n.contains("acceleration")));
        assert!(r.components.iter().all(|c| c.component != "audio"));
        assert_eq!(r.level, SupportLevel::Partial);
    }

    #[test]
    fn haswell_igpu_only_stops_at_monterey() {
        let p = desktop(
            CpuPlatform::Haswell,
            vec![igpu("0412", "Intel HD Graphics 4600")],
        );
        let r = assess(&p, None);
        assert_eq!(r.recommended, Some(MacOsVersion::Monterey));
        let ventura = option(&r, MacOsVersion::Ventura);
        assert!(!ventura.supported && ventura.needs_root_patch);
        // Releases passed over for the same reason are grouped.
        assert!(r.notes.iter().any(|n| n.title.starts_with("Why")
            && n.detail
                .starts_with("macOS Ventura 13 to macOS Sequoia 15: ")
            && n.detail.contains("macOS Tahoe 26: ")));
        // The root-patch path is an expert option, not a dead end.
        let expert = assess(&p, Some(MacOsVersion::Sequoia));
        assert_eq!(expert.level, SupportLevel::Partial);
        assert!(expert.summary.contains("workarounds"), "{}", expert.summary);
        assert!(expert.notes.iter().all(|n| n.level != NoteLevel::Blocking));
        assert!(expert.notes.iter().any(|n| n.level == NoteLevel::Warning
            && n.component == "gpu"
            && n.detail.contains("Legacy Patcher")));
        assert!(
            !expert
                .notes
                .iter()
                .any(|n| n.component == "gpu" && n.detail.contains("OpenCore Legacy Patcher 3.0")),
            "the OCLP 3.0 hint is for macOS 26 only"
        );
        let tahoe = assess(&p, Some(MacOsVersion::Tahoe));
        assert!(tahoe
            .notes
            .iter()
            .any(|n| n.detail.contains("OpenCore Legacy Patcher 3.0")));
    }

    #[test]
    fn old_igpu_with_new_dgpu_is_not_capped() {
        let p = desktop(
            CpuPlatform::Haswell,
            vec![
                igpu("0412", "Intel HD Graphics 4600"),
                dgpu("1002", "73df", "AMD Radeon RX 6700 XT"),
            ],
        );
        let r = assess(&p, None);
        assert!(option(&r, MacOsVersion::Tahoe).supported);
        assert_eq!(
            display_path(&p, MacOsVersion::Tahoe),
            DisplayPath::Native(1)
        );
        assert_eq!(
            display_path(&p, MacOsVersion::Catalina),
            DisplayPath::Native(0)
        );
    }

    #[test]
    fn ivy_bridge_cpu_ceiling_is_monterey() {
        let p = desktop(
            CpuPlatform::IvyBridge,
            vec![
                igpu("0162", "Intel HD Graphics 4000"),
                dgpu("1002", "67df", "Radeon RX 580"),
            ],
        );
        let r = assess(&p, None);
        assert_eq!(supported(&r).last(), Some(&MacOsVersion::Monterey));
        let ventura = option(&r, MacOsVersion::Ventura);
        assert!(
            ventura.notes[0].contains("CryptexFixup"),
            "{:?}",
            ventura.notes
        );
        assert_eq!(r.recommended, Some(MacOsVersion::Monterey));
    }

    #[test]
    fn broadcom_bcm4360_recommends_ventura() {
        let mut p = desktop(
            CpuPlatform::CoffeeLake,
            vec![igpu("3e92", "Intel UHD Graphics 630")],
        );
        p.wifi = Some(ProfileNic {
            name: "BCM4360".into(),
            bus: DeviceBus::Pci,
            vendor_id: Some("14e4".into()),
            device_id: Some("43a0".into()),
            ..Default::default()
        });
        let r = assess(&p, None);
        assert_eq!(r.recommended, Some(MacOsVersion::Ventura));
        let sonoma = option(&r, MacOsVersion::Sonoma);
        assert!(sonoma.supported && sonoma.needs_root_patch);
    }

    #[test]
    fn tahoe_recommended_without_analog_audio_or_broadcom() {
        let p = desktop(
            CpuPlatform::CometLake,
            vec![igpu("9bc5", "Intel UHD Graphics 630")],
        );
        let r = assess(&p, None);
        assert_eq!(r.recommended, Some(MacOsVersion::Tahoe));
        assert_eq!(supported(&r).first(), Some(&MacOsVersion::Catalina));
        let mut with_codec = p.clone();
        with_codec.audio = alc(0x10EC_0897);
        assert_eq!(
            assess(&with_codec, None).recommended,
            Some(MacOsVersion::Sequoia)
        );
        // A codec AppleALC cannot drive loses nothing on Tahoe.
        let mut unsupported_codec = p.clone();
        unsupported_codec.audio = alc(0x1234_5678);
        assert!(!codec_db::is_supported(0x1234_5678));
        assert_eq!(
            assess(&unsupported_codec, None).recommended,
            Some(MacOsVersion::Tahoe)
        );
    }

    #[test]
    fn tiger_lake_laptop_with_optimus_dgpu_has_no_display() {
        let mut p = desktop(
            CpuPlatform::TigerLake,
            vec![
                igpu("9a49", "Intel Iris Xe Graphics"),
                dgpu("10de", "1f99", "NVIDIA GeForce GTX 1650 Mobile"),
            ],
        );
        p.form_factor = FormFactor::Laptop;
        p.cpu.is_mobile = true;
        let r = assess(&p, None);
        assert!(supported(&r).is_empty());
        assert_eq!(r.recommended, None);
        assert_eq!(r.level, SupportLevel::Unsupported);
        assert!(r.summary.starts_with("No macOS release"), "{}", r.summary);
        // Even a supported laptop dGPU cannot drive the panel next to an iGPU.
        let mut amd = p.clone();
        amd.gpus[1] = dgpu("1002", "7340", "AMD Radeon RX 5500M");
        assert!(supported(&assess(&amd, None)).is_empty());
    }

    #[test]
    fn alder_lake_needs_a_supported_dgpu() {
        let rtx = desktop(
            CpuPlatform::AlderLake,
            vec![
                igpu("4680", "Intel UHD Graphics 770"),
                dgpu("10de", "2504", "NVIDIA GeForce RTX 3060"),
            ],
        );
        assert!(supported(&assess(&rtx, None)).is_empty());
        let rx = desktop(
            CpuPlatform::AlderLake,
            vec![
                igpu("4680", "Intel UHD Graphics 770"),
                dgpu("1002", "67df", "Radeon RX 580"),
            ],
        );
        let r = assess(&rx, None);
        assert_eq!(supported(&r).first(), Some(&MacOsVersion::Catalina));
        assert_eq!(
            r.components[0].level,
            SupportLevel::Partial,
            "community recipe"
        );
    }

    #[test]
    fn f_sku_hint_only_for_igpu_cpus() {
        let mut p = desktop(
            CpuPlatform::CoffeeLake,
            vec![dgpu("10de", "2504", "NVIDIA GeForce RTX 3060")],
        );
        p.cpu.name = "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz".into();
        let r = assess(&p, Some(MacOsVersion::Sequoia));
        assert!(r
            .notes
            .iter()
            .any(|n| n.detail.contains("iGPU Multi-Monitor")));
        p.cpu.name = "Intel(R) Core(TM) i5-9400F CPU @ 2.90GHz".into();
        let r = assess(&p, Some(MacOsVersion::Sequoia));
        assert!(!r
            .notes
            .iter()
            .any(|n| n.detail.contains("iGPU Multi-Monitor")));
        for (name, without) in [
            ("Intel(R) Core(TM) i9-9900KF CPU @ 3.60GHz", true),
            ("Intel(R) Xeon(R) CPU E3-1231 v3 @ 3.40GHz", true),
            ("Intel(R) Xeon(R) CPU E3-1245 v3 @ 3.40GHz", false),
            ("Intel(R) Xeon(R) E-2124 CPU @ 3.30GHz", true),
            ("Intel(R) Xeon(R) E-2124G CPU @ 3.40GHz", false),
            ("Intel(R) Core(TM) i7-4790K CPU @ 4.00GHz", false),
        ] {
            assert_eq!(intel_model_without_igpu(name), without, "{name}");
        }
    }

    #[test]
    fn apple_silicon_and_unknown_cpus() {
        let mut p = desktop(CpuPlatform::AppleSilicon, vec![]);
        p.cpu.vendor = CpuVendor::Apple;
        let r = assess(&p, None);
        assert!(supported(&r).is_empty());
        assert!(r.summary.contains("natively"));
        let p = desktop(
            CpuPlatform::Unknown,
            vec![igpu("3e92", "Intel UHD Graphics 630")],
        );
        let r = assess(&p, None);
        assert_eq!(r.level, SupportLevel::Unknown);
        assert!(supported(&r).is_empty());
    }

    #[test]
    fn penryn_and_pentium_ceilings() {
        let p = desktop(
            CpuPlatform::Penryn,
            vec![dgpu("10de", "0640", "NVIDIA GeForce 9500 GT")],
        );
        let r = assess(&p, None);
        assert_eq!(supported(&r), vec![MacOsVersion::HighSierra]);
        // Haswell Pentium without AVX2: CryptexFixup path is supported but not recommended.
        let mut p = desktop(
            CpuPlatform::Haswell,
            vec![dgpu("1002", "6810", "AMD Radeon R9 270X")],
        );
        p.cpu.has_avx2 = Some(false);
        let r = assess(&p, None);
        assert_eq!(r.recommended, Some(MacOsVersion::Monterey));
        // GCN is native up to Monterey only.
        assert!(!option(&r, MacOsVersion::Ventura).supported);
        // Polaris on a non-AVX2 CPU needs the OCLP patch from Ventura on.
        let mut p = desktop(
            CpuPlatform::Haswell,
            vec![dgpu("1002", "67df", "Radeon RX 580")],
        );
        p.cpu.has_avx2 = Some(false);
        let r = assess(&p, None);
        let v = option(&r, MacOsVersion::Ventura);
        assert!(!v.supported && v.needs_root_patch);
    }

    #[test]
    fn arrow_lake_and_zen5_prefer_tested_releases() {
        let p = desktop(
            CpuPlatform::ArrowLake,
            vec![dgpu("1002", "73ff", "AMD Radeon RX 6600")],
        );
        assert_eq!(assess(&p, None).recommended, Some(MacOsVersion::Sequoia));
        let p = desktop(
            CpuPlatform::AmdZen5,
            vec![dgpu("1002", "73ff", "AMD Radeon RX 6600")],
        );
        assert_eq!(assess(&p, None).recommended, Some(MacOsVersion::Tahoe));
    }

    #[test]
    fn vmd_and_low_ram_warnings() {
        let mut p = desktop(
            CpuPlatform::CometLake,
            vec![igpu("9bc5", "Intel UHD Graphics 630")],
        );
        p.storage = vec![ProfileStorage {
            name: "Intel RST VMD".into(),
            kind: StorageKind::Raid,
            vendor_id: Some("8086".into()),
            device_id: Some("9a0b".into()),
            size_bytes: None,
        }];
        p.ram_gb = 3;
        p.firmware_uefi = Some(false);
        let r = assess(&p, None);
        assert!(r
            .notes
            .iter()
            .any(|n| n.component == "storage" && n.detail.contains("VMD")));
        assert!(r.notes.iter().any(|n| n.title == "Not enough memory"));
        assert!(r.notes.iter().any(|n| n.title == "Legacy BIOS boot"));
        assert_eq!(r.level, SupportLevel::Partial);
    }

    #[test]
    fn report_notes_are_ordered_by_level() {
        let p = desktop(
            CpuPlatform::Haswell,
            vec![igpu("0412", "Intel HD Graphics 4600")],
        );
        let r = assess(&p, Some(MacOsVersion::Tahoe));
        let ranks: Vec<u8> = r
            .notes
            .iter()
            .map(|n| match n.level {
                NoteLevel::Blocking => 0,
                NoteLevel::Warning => 1,
                NoteLevel::Info => 2,
            })
            .collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
    }

    #[test]
    fn describes_version_runs() {
        use MacOsVersion::*;
        assert_eq!(
            describe_versions(&[HighSierra, Monterey, Ventura]),
            "macOS High Sierra 10.13, macOS Monterey 12 to macOS Ventura 13"
        );
    }

    #[test]
    fn cryptexfixup_and_telemetrap_paths_are_expert_options() {
        use MacOsVersion::*;
        // Ivy Bridge + RX 580: Ventura+ needs CryptexFixup and the OCLP
        // non-AVX2 Polaris patch.
        let p = desktop(
            CpuPlatform::IvyBridge,
            vec![dgpu("1002", "67df", "Radeon RX 580")],
        );
        let r = assess(&p, None);
        assert_eq!(r.recommended, Some(Monterey));
        let v = option(&r, Ventura);
        assert!(!v.supported && v.needs_root_patch);
        assert!(v.notes.iter().any(|n| n.contains("CryptexFixup")));
        let opt_in = assess(&p, Some(Sonoma));
        assert_eq!(opt_in.level, SupportLevel::Partial);
        assert!(opt_in.notes.iter().all(|n| n.level != NoteLevel::Blocking));
        // Navi needs AVX2 and OCLP only patches it behind a developer flag.
        let navi = desktop(
            CpuPlatform::IvyBridge,
            vec![dgpu("1002", "73ff", "AMD Radeon RX 6600")],
        );
        let r = assess(&navi, Some(Ventura));
        assert_eq!(r.level, SupportLevel::Unsupported);
        assert!(r
            .notes
            .iter()
            .any(|n| n.level == NoteLevel::Blocking && n.component == "gpu"));

        // Penryn: High Sierra natively, Mojave to Monterey with telemetrap,
        // nothing past Monterey.
        let p = desktop(
            CpuPlatform::Penryn,
            vec![dgpu("1002", "67df", "Radeon RX 580")],
        );
        let r = assess(&p, None);
        assert_eq!(supported(&r), vec![HighSierra]);
        assert_eq!(r.recommended, Some(HighSierra));
        assert!(option(&r, Mojave).notes[0].contains("telemetrap"));
        assert_eq!(assess(&p, Some(Monterey)).level, SupportLevel::Partial);
        let ventura = assess(&p, Some(Ventura));
        assert_eq!(ventura.level, SupportLevel::Unsupported);
        assert!(option(&ventura, Ventura).notes[0].contains("at most"));

        // A Pentium on an AVX2 platform without CPUID flags: the brand
        // string says it has no AVX2.
        let mut p = desktop(
            CpuPlatform::Haswell,
            vec![dgpu("1002", "6810", "AMD Radeon R9 270X")],
        );
        p.cpu.name = "Intel(R) Pentium(R) CPU G3258 @ 3.20GHz".into();
        p.cpu.has_avx2 = None;
        assert!(!cpu_identity(&p).has_avx2);
        let r = assess(&p, None);
        assert_eq!(supported(&r).last(), Some(&Monterey));
        assert!(option(&r, Ventura)
            .notes
            .iter()
            .any(|n| n.contains("no AVX2")));
    }

    #[test]
    fn amd_core_count_limits() {
        let mut p = desktop(
            CpuPlatform::AmdZen3,
            vec![dgpu("1002", "73ff", "AMD Radeon RX 6600")],
        );
        p.cpu.cores = 0;
        p.cpu.threads = 0;
        let r = assess(&p, None);
        assert!(supported(&r).is_empty());
        assert_eq!(r.level, SupportLevel::Unknown);
        assert!(r.summary.contains("cores"), "{}", r.summary);
        assert_eq!(r.components[0].level, SupportLevel::Unknown);
        p.cpu.cores = 96;
        p.cpu.threads = 192;
        let r = assess(&p, None);
        assert!(supported(&r).is_empty());
        assert!(option(&r, MacOsVersion::Sonoma).notes[0].contains("64"));
        // Family 15h modules count as cores.
        let mut fx = desktop(
            CpuPlatform::AmdBulldozer,
            vec![dgpu("1002", "67df", "Radeon RX 580")],
        );
        fx.cpu.cores = 0;
        fx.cpu.threads = 8;
        assert!(!supported(&assess(&fx, None)).is_empty());
    }

    #[test]
    fn vm_guest_cpu_models_are_not_held_to_bare_metal_limits() {
        let mut p = desktop(
            CpuPlatform::MeteorLake,
            vec![gpu(
                "1234",
                "1111",
                "QEMU Standard VGA",
                "PciRoot(0x0)/Pci(0x1,0x0)",
            )],
        );
        // Bare metal: the Arc iGPU platform has no working configuration.
        assert!(supported(&assess(&p, None)).is_empty());
        // As a guest CPU model it runs every release.
        p.vm = Some(VmKind::Kvm);
        assert_eq!(supported(&assess(&p, None)).len(), 9);
        p.cpu.platform = CpuPlatform::Unknown;
        assert_eq!(supported(&assess(&p, None)).len(), 9);
        // Without AVX2, macOS 13+ is an expert option (CryptexFixup).
        p.cpu.has_avx2 = Some(false);
        let r = assess(&p, Some(MacOsVersion::Ventura));
        assert_eq!(supported(&r).len(), 5);
        assert_eq!(r.level, SupportLevel::Partial);
        // K10 lacks the instruction set macOS needs, VM or not.
        p.cpu.platform = CpuPlatform::AmdK10;
        p.cpu.has_avx2 = None;
        assert!(supported(&assess(&p, None)).is_empty());
    }

    #[test]
    fn mux_laptop_with_disabled_igpu_uses_the_dgpu() {
        let mut p = desktop(
            CpuPlatform::CoffeeLake,
            vec![
                igpu("3e9b", "Intel UHD Graphics 630"),
                dgpu("1002", "7340", "AMD Radeon RX 5500M"),
            ],
        );
        p.form_factor = FormFactor::Laptop;
        assert_eq!(
            display_path(&p, MacOsVersion::Sonoma),
            DisplayPath::Native(0)
        );
        p.gpus[0].disabled = true;
        assert_eq!(
            display_path(&p, MacOsVersion::Sonoma),
            DisplayPath::Native(1)
        );
        assert!(can_drive_display(&p, 1));
    }

    /// Every platform × form factor × release: no supported release the
    /// planner refuses, the recommendation is supported, nothing panics.
    #[test]
    fn sweep_agrees_with_the_planner() {
        let forms = [
            FormFactor::Desktop,
            FormFactor::Laptop,
            FormFactor::AllInOne,
            FormFactor::MiniPc,
        ];
        for &platform in cpu_db::all_platforms() {
            for form in forms {
                for gpus in [
                    vec![dgpu("1002", "67df", "Radeon RX 580")],
                    vec![igpu("3e92", "Intel UHD Graphics 630")],
                    vec![gpu(
                        "1002",
                        "15d8",
                        "AMD Radeon Vega 8 Graphics",
                        "PciRoot(0x0)/Pci(0x8,0x1)/Pci(0x0,0x0)",
                    )],
                    vec![],
                ] {
                    for vm in [None, Some(VmKind::Kvm)] {
                        let mut p = desktop(platform, gpus.clone());
                        p.form_factor = form;
                        p.vm = vm;
                        p.audio = alc(0x10EC_0897);
                        p.input = ProfileInput {
                            keyboard_bus: InputBus::Ps2,
                            ..Default::default()
                        };
                        for target in std::iter::once(None).chain(MacOsVersion::ALL.map(Some)) {
                            let r = assess(&p, target);
                            assert_eq!(r.versions.len(), 9);
                            for o in &r.versions {
                                if o.supported {
                                    assert!(
                                        planner_accepts(&p, o.version),
                                        "{platform:?} {form:?} {vm:?} {:?} supported but refused",
                                        o.version
                                    );
                                }
                            }
                            if let Some(t) = target {
                                let o = option(&r, t);
                                // A target the report does not support is never "Supported".
                                if !o.supported {
                                    assert_ne!(r.level, SupportLevel::Supported);
                                    let blocking =
                                        r.notes.iter().any(|n| n.level == NoteLevel::Blocking);
                                    // Expert options carry no blocking note and stay Partial.
                                    if r.level == SupportLevel::Partial {
                                        assert!(!blocking, "{platform:?} {form:?} {t:?}");
                                    }
                                }
                            }
                            if let Some(rec) = r.recommended {
                                assert!(option(&r, rec).supported);
                            }
                            assert!(!r.summary.is_empty());
                            assert!((0.0..=1.0).contains(&r.confidence));
                        }
                    }
                }
            }
        }
    }
}
