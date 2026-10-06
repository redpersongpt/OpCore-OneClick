//! SMBIOS model, Secure Boot model and board-id handling.
//!
//! Every hardware class has a ranked list of models (Dortania per-platform
//! pages, `extras/smbios-support`, research-intel-desktop §6 and
//! research-intel-laptop §3): the first model Apple supports on the target
//! wins, so a machine moves to a newer Mac only when the older one was
//! dropped. Platforms whose ceiling is passed with CryptexFixup or root
//! patches (Sandy/Ivy Bridge, 1st gen Core, X58/X79) keep the model of their
//! own generation and skip Apple's board-id check instead, like the OCLP
//! Hackintosh guides do.

use crate::domain::model::{
    BinaryPatch, BuildPlan, CpuPlatform as P, FormFactor, GpuFamily, MacOsVersion, NoteLevel,
};
use crate::domain::smbios_db::{self, SmbiosModel};
use crate::error::AppError;

use super::{graphics, note, DisplayPlan, MobileClass, PlanContext};

/// How the displays are driven, as far as the SMBIOS choice is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayPath {
    /// The iGPU drives the displays (Intel iGPU or a NootedRed APU).
    Igpu,
    /// A dGPU drives the displays, a supported iGPU stays on for compute.
    DgpuWithIgpu,
    /// A dGPU drives the displays and there is no usable iGPU.
    DgpuOnly,
    /// Virtual machine display.
    Virtual,
}

/// Ranked SMBIOS candidates for one hardware class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidates {
    /// Preference order; the first one Apple supports on the target wins.
    pub models: Vec<&'static str>,
    /// Kept with the board-id skip when none of `models` supports the target.
    pub skip_model: &'static str,
    /// Hardware class, used in the reason text.
    pub class: &'static str,
}

fn c(models: &[&'static str], skip_model: &'static str, class: &'static str) -> Candidates {
    Candidates {
        models: models.to_vec(),
        skip_model,
        class,
    }
}

/// The model chosen for a target.
#[derive(Debug, Clone)]
pub struct Choice {
    pub model: &'static SmbiosModel,
    /// The model does not support the target: skip Apple's board-id check.
    pub board_id_skip: bool,
    /// Other candidates Apple supports on the target.
    pub alternatives: Vec<&'static SmbiosModel>,
}

/// Choose `plan.smbios` (model, reason, alternatives, secure_boot_model,
/// board_id_skip). Honour `options.smbios_override` (warn when it does not
/// support the target). The chosen model must support the target per
/// `smbios_db` whenever a reasonable one exists; otherwise use the closest
/// model plus the board-id skip booter patches (and RestrictEvents
/// `revpatch=sbvmm` for updates) and explain it in a note.
pub fn apply(
    ctx: &PlanContext,
    display: &DisplayPlan,
    plan: &mut BuildPlan,
) -> Result<(), AppError> {
    let target = ctx.target;
    let path = display_path(ctx, display);
    let cands = candidates(ctx, display);
    let recommended = choose(&cands, target).ok_or_else(|| {
        AppError::new(
            "SMBIOS_NO_CANDIDATE",
            format!("No Mac model is known for this {}.", cands.class),
        )
    })?;

    let override_model = ctx
        .options
        .smbios_override
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let (choice, reason) = match override_model {
        Some(name) => {
            let model = smbios_db::find(name).ok_or_else(|| {
                AppError::new(
                    "SMBIOS_UNKNOWN",
                    format!("{name} is not a Mac model that can run macOS High Sierra or newer."),
                )
                .recoverable()
                .with_suggestion(format!(
                    "Leave the SMBIOS on automatic ({}) or pick a model from the list.",
                    recommended.model.model
                ))
            })?;
            let supported = smbios_db::supports(model.model, target);
            let mut alternatives = vec![recommended.model];
            alternatives.extend(recommended.alternatives.iter().copied());
            alternatives.retain(|m| m.model != model.model);
            let choice = Choice {
                model,
                board_id_skip: !supported && target > model.max_os,
                alternatives,
            };
            if !supported {
                plan.notes.push(note(
                    NoteLevel::Warning,
                    "smbios",
                    format!("{} does not support {}", model.model, target.display_name()),
                    format!(
                        "The chosen SMBIOS {} ({}) is supported by Apple from {} to {}. {} The recommended \
                         model for this machine is {}.",
                        model.model,
                        model.description,
                        model.min_release,
                        model.max_release,
                        if choice.board_id_skip {
                            "The build skips Apple's board-id check so the installer still runs."
                        } else {
                            "The installer of this older release does not know the model and may refuse \
                             to install or boot."
                        },
                        recommended.model.model
                    ),
                ));
            }
            let reason = format!(
                "Chosen manually: {} ({}){}.",
                model.model,
                model.description,
                if supported {
                    String::new()
                } else {
                    format!(", not supported by {}", target.display_name())
                }
            );
            (choice, reason)
        }
        None => {
            let reason = reason_text(&cands, &recommended, target);
            (recommended, reason)
        }
    };

    let secure_boot_model = secure_boot_model(ctx, display, choice.model, choice.board_id_skip);
    plan.smbios.model = choice.model.model.to_string();
    plan.smbios.reason = reason;
    plan.smbios.secure_boot_model = secure_boot_model.to_string();
    plan.smbios.board_id_skip = choice.board_id_skip;
    plan.smbios.alternatives = choice
        .alternatives
        .iter()
        .map(|m| m.model.to_string())
        .collect();

    if choice.board_id_skip {
        plan.booter_patches.push(board_id_skip_patch());
        plan.notes.push(note(
            NoteLevel::Warning,
            "smbios",
            format!("Board-id check skipped for {}", choice.model.model),
            format!(
                "{} The \"Skip Board ID check\" booter patch from OpenCore Legacy Patcher lets the \
                 installer and the system boot with it anyway; RestrictEvents (revpatch=sbvmm) keeps \
                 software updates working from macOS 11.3 on. Apple Secure Boot is disabled.{}",
                if override_model.is_some() {
                    format!(
                        "The chosen {} stops at macOS {}.",
                        choice.model.model, choice.model.max_release
                    )
                } else {
                    format!(
                        "No Mac model that matches this {} supports {} (the {} stops at macOS {}).",
                        cands.class,
                        target.display_name(),
                        choice.model.model,
                        choice.model.max_release
                    )
                },
                if target < MacOsVersion::BigSur {
                    " Should the installer still refuse the model, add -no_compat_check to the boot \
                     arguments (it also blocks software updates)."
                } else {
                    ""
                }
            ),
        ));
    } else if override_model.is_none()
        && target < MacOsVersion::Tahoe
        && choice.model.max_os < MacOsVersion::Tahoe
    {
        if let Some(next) = choice
            .alternatives
            .iter()
            .find(|m| m.max_os == MacOsVersion::Tahoe)
        {
            plan.notes.push(note(
                NoteLevel::Info,
                "smbios",
                format!("{} stops at macOS {}", choice.model.model, choice.model.max_release),
                format!(
                    "{} is the closest Mac for this machine but Apple ends its support with {}. {} also \
                     supports {} and macOS 26; choose it now if you plan to update to macOS 26 later, since \
                     changing the SMBIOS afterwards changes the serial numbers.",
                    choice.model.model,
                    choice.model.max_release,
                    next.model,
                    target.display_name()
                ),
            ));
        }
    }
    // Recent iMacs always had an iGPU (Dortania smbios-support).
    let imac = choice.model.model.starts_with("iMac") && !choice.model.model.starts_with("iMacPro");
    if path == DisplayPath::DgpuOnly && imac && choice.model.max_os >= MacOsVersion::Catalina {
        plan.notes.push(note(
            NoteLevel::Warning,
            "smbios",
            "iMac SMBIOS without an iGPU",
            "iMac models expect a working iGPU; without one Quick Look, Preview and DRM video can fail. \
             iMacPro1,1 or MacPro7,1 avoid this.",
        ));
    }
    Ok(())
}

/// Classify the display decision.
pub fn display_path(ctx: &PlanContext, display: &DisplayPlan) -> DisplayPath {
    if ctx.is_vm {
        return DisplayPath::Virtual;
    }
    match ctx.display_gpu(display) {
        Some(gpu) if gpu.is_igpu => DisplayPath::Igpu,
        Some(_) if display.igpu.is_some() => DisplayPath::DgpuWithIgpu,
        _ => DisplayPath::DgpuOnly,
    }
}

/// Ranked SMBIOS candidates for this machine.
pub fn candidates(ctx: &PlanContext, display: &DisplayPlan) -> Candidates {
    let path = display_path(ctx, display);
    let family = ctx.display_gpu(display).map(|g| g.family);
    if path == DisplayPath::Virtual {
        // research-amd §17 (OSX-PROXMOX): iMacPro1,1 up to macOS 15, then
        // MacPro7,1, the only workstation model left on macOS 26.
        return c(&["iMacPro1,1", "MacPro7,1"], "MacPro7,1", "virtual machine");
    }
    if ctx.is_amd() {
        return amd(ctx, path, family);
    }
    if ctx.cpu.hedt {
        return intel_hedt(ctx.platform());
    }
    match ctx.profile.form_factor {
        FormFactor::Laptop => intel_laptop(ctx, path),
        _ if path == DisplayPath::DgpuOnly && !pre_haswell(ctx.platform()) => workstation(family),
        FormFactor::MiniPc if ctx.profile.cpu.is_mobile => intel_mini(ctx, path, family),
        // The Ice Lake framebuffer only matches the Ice Lake MacBooks.
        _ if ctx.platform() == P::IceLake => intel_laptop(ctx, path),
        _ => intel_desktop(ctx, path, family),
    }
}

fn pre_haswell(platform: P) -> bool {
    matches!(
        platform,
        P::Penryn | P::Lynnfield | P::Arrandale | P::SandyBridge | P::IvyBridge
    )
}

/// The first candidate Apple supports on `target`, else the class's own
/// model with the board-id skip. None only when no candidate is a known model.
pub fn choose(cands: &Candidates, target: MacOsVersion) -> Option<Choice> {
    let models: Vec<&'static SmbiosModel> = cands
        .models
        .iter()
        .filter_map(|m| smbios_db::find(m))
        .collect();
    let supported: Vec<&'static SmbiosModel> = models
        .iter()
        .copied()
        .filter(|m| smbios_db::supports(m.model, target))
        .collect();
    if let Some((first, rest)) = supported.split_first() {
        return Some(Choice {
            model: first,
            board_id_skip: false,
            alternatives: rest.to_vec(),
        });
    }
    smbios_db::find(cands.skip_model)
        .or_else(|| models.iter().copied().max_by_key(|m| m.max_os))
        .map(|model| Choice {
            model,
            board_id_skip: target > model.max_os,
            alternatives: Vec::new(),
        })
}

fn reason_text(cands: &Candidates, choice: &Choice, target: MacOsVersion) -> String {
    let m = choice.model;
    let status = if choice.board_id_skip {
        format!(
            "Apple supports it only up to {}, so Apple's board-id check is skipped for {}",
            m.max_release,
            target.display_name()
        )
    } else if smbios_db::supports(m.model, target) {
        format!("the closest Mac that supports {}", target.display_name())
    } else {
        format!("not supported by {}", target.display_name())
    };
    format!(
        "{}: {} ({}), {}.",
        capitalise(cands.class),
        m.model,
        m.description,
        status
    )
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ── Candidate tables ────────────────────────────────────────────────────────

/// Machines whose display GPU is a dGPU and whose CPU has no usable iGPU:
/// Dortania (smbios-support, AMD/fx "PlatformInfo") wants iMacPro1,1 or
/// MacPro7,1, MacPro6,1 for GCN 1-3 cards (native up to macOS 12) and
/// iMac14,2 as the NVIDIA Web Driver fallback on High Sierra.
fn workstation(family: Option<GpuFamily>) -> Candidates {
    use GpuFamily as G;
    match family {
        Some(G::AmdGcn1 | G::AmdGcn2 | G::AmdGcn3 | G::AmdTeraScale) => c(
            &["MacPro6,1", "MacPro7,1", "iMacPro1,1"],
            "MacPro7,1",
            "system without a usable iGPU and an AMD GCN 1-3 graphics card",
        ),
        Some(G::NvidiaMaxwell | G::NvidiaPascal | G::NvidiaFermi) => c(
            &["MacPro7,1", "iMacPro1,1", "iMac14,2"],
            "MacPro7,1",
            "system without a usable iGPU and an NVIDIA graphics card",
        ),
        _ => c(
            &["MacPro7,1", "iMacPro1,1"],
            "MacPro7,1",
            "system whose display runs on a dGPU without a usable iGPU",
        ),
    }
}

fn amd(ctx: &PlanContext, path: DisplayPath, family: Option<GpuFamily>) -> Candidates {
    let laptop = ctx.profile.form_factor == FormFactor::Laptop;
    match (laptop, path) {
        // NootedRed APUs: MacBookPro16,2 / iMac20,1 (research-amd §8, §13).
        (true, DisplayPath::Igpu) => c(
            &["MacBookPro16,2", "MacBookPro15,2"],
            "MacBookPro16,2",
            "AMD laptop with a Vega APU",
        ),
        (true, _) => laptop_dgpu(),
        (false, DisplayPath::Igpu) => c(
            &["iMac20,1", "iMacPro1,1"],
            "iMac20,1",
            "AMD desktop with a Vega APU",
        ),
        // MacPro7,1 for 10.15+, iMacPro1,1 for 10.13/10.14 (research-amd §8).
        (false, _) => {
            let mut cands = workstation(family);
            cands.class = if ctx.is_hedt {
                "AMD Threadripper workstation"
            } else if cands.models[0] == "MacPro6,1" {
                "AMD desktop with a GCN 1-3 graphics card"
            } else {
                "AMD desktop"
            };
            cands
        }
    }
}

/// Dortania HEDT pages: MacPro5,1/6,1 for X58, MacPro6,1 for X79,
/// iMacPro1,1 for X99/X299 (MacPro7,1 on macOS 26, and first for the Cascade
/// Lake-W family the Mac Pro 2019 uses).
fn intel_hedt(platform: P) -> Candidates {
    match platform {
        P::NehalemHedt => c(&["MacPro5,1", "MacPro6,1"], "MacPro6,1", "X58 workstation"),
        P::SandyBridgeE | P::IvyBridgeE => c(&["MacPro6,1"], "MacPro6,1", "X79 workstation"),
        P::CascadeLakeX => c(
            &["MacPro7,1", "iMacPro1,1"],
            "MacPro7,1",
            "Cascade Lake-X/W workstation",
        ),
        P::HaswellE | P::BroadwellE => {
            c(&["iMacPro1,1", "MacPro7,1"], "MacPro7,1", "X99 workstation")
        }
        _ => c(
            &["iMacPro1,1", "MacPro7,1"],
            "MacPro7,1",
            "X299 workstation",
        ),
    }
}

/// research-intel-desktop §6, per display path.
fn intel_desktop(ctx: &PlanContext, path: DisplayPath, family: Option<GpuFamily>) -> Candidates {
    use DisplayPath::{DgpuOnly, DgpuWithIgpu, Igpu};
    let platform = ctx.platform();
    match (platform, path) {
        (P::Penryn, _) => c(&["iMac10,1", "MacPro6,1"], "MacPro6,1", "Core 2 desktop"),
        // Arrandale on a desktop board is the Clarkdale die.
        (P::Lynnfield | P::Arrandale, _) => {
            let clarkdale = platform == P::Arrandale
                || ctx
                    .profile
                    .cpu
                    .codename
                    .to_ascii_lowercase()
                    .contains("clarkdale");
            let imac = if clarkdale { "iMac11,2" } else { "iMac11,1" };
            c(&[imac, "MacPro6,1"], "MacPro6,1", "1st gen Core desktop")
        }
        (P::SandyBridge, Igpu) => c(
            &["iMac12,2", "Macmini5,1"],
            "iMac12,2",
            "Sandy Bridge desktop on HD 3000",
        ),
        (P::SandyBridge, DgpuWithIgpu) => c(
            &["iMac12,2", "MacPro6,1"],
            "MacPro6,1",
            "Sandy Bridge desktop with a dGPU",
        ),
        (P::SandyBridge, _) => c(
            &["MacPro6,1"],
            "MacPro6,1",
            "Sandy Bridge desktop with a dGPU",
        ),
        (P::IvyBridge, Igpu) => c(
            &["iMac13,1", "iMac14,4"],
            "iMac13,1",
            "Ivy Bridge desktop on HD 4000",
        ),
        (P::IvyBridge, DgpuWithIgpu) => c(
            &["iMac13,2", "iMac15,1", "MacPro6,1"],
            "iMac13,2",
            "Ivy Bridge desktop with a dGPU",
        ),
        (P::IvyBridge, _) => c(
            &["MacPro6,1"],
            "MacPro6,1",
            "Ivy Bridge desktop with a dGPU",
        ),
        (_, DgpuOnly) => workstation(family),
        (P::Haswell, Igpu) => c(
            &["iMac14,4", "iMac16,2", "iMac18,1", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Haswell desktop on the iGPU",
        ),
        (P::Haswell, _) => c(
            &["iMac15,1", "iMac17,1", "iMac18,2", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Haswell desktop with a dGPU",
        ),
        (P::Broadwell, Igpu) => c(
            &["iMac16,2", "iMac18,1", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Broadwell desktop on the iGPU",
        ),
        (P::Broadwell, _) => c(
            &["iMac16,2", "iMac17,1", "iMac18,2", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Broadwell desktop with a dGPU",
        ),
        // Ventura+ runs the Skylake iGPU as Kaby Lake, so Kaby Lake iMacs follow.
        (P::Skylake, Igpu) => c(
            &["iMac17,1", "iMac18,1", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Skylake desktop on the iGPU",
        ),
        (P::Skylake, _) => c(
            &["iMac17,1", "iMac18,3", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Skylake desktop with a dGPU",
        ),
        (P::KabyLake, Igpu) => c(
            &["iMac18,1", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Kaby Lake desktop on the iGPU",
        ),
        (P::KabyLake, _) => c(
            &["iMac18,3", "iMac19,1", "iMac20,1"],
            "iMac20,1",
            "Kaby Lake desktop with a dGPU",
        ),
        // iMac19,1 from Mojave (iMac18,x on High Sierra), iMac20,1 on macOS 26.
        (P::CoffeeLake, Igpu) => c(
            &["iMac19,1", "iMac20,1", "iMac18,1"],
            "iMac20,1",
            "Coffee Lake desktop on the iGPU",
        ),
        (P::CoffeeLake, _) => c(
            &["iMac19,1", "iMac20,1", "iMac18,3"],
            "iMac20,1",
            "Coffee Lake desktop with a dGPU",
        ),
        // iMac20,2 is the 10-core i9 model (Dortania comet-lake).
        (P::CometLake, _) if ctx.profile.cpu.cores >= 10 => c(
            &["iMac20,2", "iMac20,1"],
            "iMac20,2",
            "Comet Lake desktop with a 10-core CPU",
        ),
        (P::CometLake, _) => c(&["iMac20,1", "iMac20,2"], "iMac20,1", "Comet Lake desktop"),
        // Rocket Lake and newer have no usable iGPU: workstation models.
        _ => workstation(family),
    }
}

/// NUCs and mini PCs with mobile CPUs (Dortania laptop tables, "NUC" rows):
/// Mac mini where one exists, iMac18,1 for unsupported iGPUs on Ventura and
/// Macmini8,1 until macOS 15.
fn intel_mini(ctx: &PlanContext, path: DisplayPath, family: Option<GpuFamily>) -> Candidates {
    let quad = ctx.profile.cpu.cores >= 4;
    match ctx.platform() {
        P::SandyBridge if quad => c(&["Macmini5,3"], "Macmini5,3", "Sandy Bridge mini PC"),
        P::SandyBridge => c(&["Macmini5,1"], "Macmini5,1", "Sandy Bridge mini PC"),
        P::IvyBridge => {
            let mini = if quad { "Macmini6,2" } else { "Macmini6,1" };
            c(&[mini, "Macmini7,1"], mini, "Ivy Bridge mini PC")
        }
        P::Haswell => c(
            &["Macmini7,1", "iMac18,1", "Macmini8,1", "iMac20,1"],
            "iMac20,1",
            "Haswell mini PC",
        ),
        P::Broadwell => c(
            &["iMac16,1", "iMac18,1", "Macmini8,1", "iMac20,1"],
            "iMac20,1",
            "Broadwell mini PC",
        ),
        P::Skylake => c(
            &["iMac17,1", "iMac18,1", "Macmini8,1", "iMac20,1"],
            "iMac20,1",
            "Skylake mini PC",
        ),
        P::KabyLake => c(
            &["iMac18,1", "Macmini8,1", "iMac20,1"],
            "iMac20,1",
            "Kaby Lake mini PC",
        ),
        P::CoffeeLake => c(
            &["Macmini8,1", "iMac20,1", "iMac18,1"],
            "iMac20,1",
            "Coffee Lake mini PC",
        ),
        P::CometLake => c(
            &["Macmini8,1", "iMac20,1"],
            "iMac20,1",
            "Comet Lake mini PC",
        ),
        P::IceLake => intel_laptop(ctx, path),
        _ => intel_desktop(ctx, path, family),
    }
}

/// Laptops whose display is driven by a dGPU (MUX designs, 11th gen and newer
/// with an AMD dGPU): the 16" and 15" MacBook Pro with Radeon graphics.
fn laptop_dgpu() -> Candidates {
    c(
        &[
            "MacBookPro16,1",
            "MacBookPro15,3",
            "MacBookPro15,1",
            "MacBookPro16,4",
        ],
        "MacBookPro16,1",
        "laptop whose display runs on the dGPU",
    )
}

/// research-intel-laptop §3 by CPU class. Ventura and newer without a native
/// model follow Dortania (`extras/ventura`: "All unsupported laptops should
/// use MacBookPro14,1"), then MacBookAir8,1 (Y) / MacBookPro15,2 (U) /
/// MacBookPro15,1 (H) for 14-15 and MacBookPro16,2 / 16,1 for 26.
fn intel_laptop(ctx: &PlanContext, path: DisplayPath) -> Candidates {
    use MobileClass::{H, U, Y};
    let platform = ctx.platform();
    let dgpu = matches!(path, DisplayPath::DgpuOnly | DisplayPath::DgpuWithIgpu);
    if dgpu
        && !pre_haswell(platform)
        && !matches!(
            platform,
            P::Haswell | P::Broadwell | P::Skylake | P::KabyLake
        )
    {
        return laptop_dgpu();
    }
    let class = if dgpu { H } else { ctx.mobile_class() };
    let amber = ctx
        .profile
        .cpu
        .codename
        .to_ascii_lowercase()
        .contains("amber");
    match (platform, class) {
        (P::Penryn, Y) => c(&["MacBookAir3,2"], "MacBookAir3,2", "Core 2 ULV laptop"),
        (P::Penryn, _) => c(&["MacBookPro7,1"], "MacBookPro7,1", "Core 2 laptop"),
        // Dortania arrandale: MacBookPro6,2 (15" dual), MacBookPro6,1 (17"/quad, Clarksfield).
        (P::Arrandale | P::Lynnfield, H) => c(
            &["MacBookPro6,1", "MacBookPro6,2"],
            "MacBookPro6,1",
            "1st gen Core quad-core laptop",
        ),
        (P::Arrandale | P::Lynnfield, _) => c(
            &["MacBookPro6,2", "MacBookPro6,1"],
            "MacBookPro6,2",
            "1st gen Core laptop",
        ),
        (P::SandyBridge, Y) => c(
            &["MacBookAir4,2", "MacBookAir4,1"],
            "MacBookAir4,2",
            "Sandy Bridge ULV laptop",
        ),
        (P::SandyBridge, U) => c(
            &["MacBookPro8,1"],
            "MacBookPro8,1",
            "Sandy Bridge dual-core laptop",
        ),
        (P::SandyBridge, H) => c(
            &["MacBookPro8,2", "MacBookPro8,3"],
            "MacBookPro8,2",
            "Sandy Bridge quad-core laptop",
        ),
        (P::IvyBridge, Y) => c(
            &[
                "MacBookAir5,2",
                "MacBookAir5,1",
                "MacBookAir6,2",
                "MacBookAir7,2",
            ],
            "MacBookAir5,2",
            "Ivy Bridge ULV laptop",
        ),
        (P::IvyBridge, U) => c(
            &[
                "MacBookPro10,2",
                "MacBookPro9,2",
                "MacBookPro11,1",
                "MacBookPro11,4",
            ],
            "MacBookPro10,2",
            "Ivy Bridge dual-core laptop",
        ),
        (P::IvyBridge, H) => c(
            &[
                "MacBookPro10,1",
                "MacBookPro9,1",
                "MacBookPro11,2",
                "MacBookPro11,4",
            ],
            "MacBookPro10,1",
            "Ivy Bridge quad-core laptop",
        ),
        (P::Haswell, Y) => c(
            &[
                "MacBookAir6,2",
                "MacBookPro11,1",
                "MacBookPro11,4",
                "MacBookPro14,1",
                "MacBookAir8,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Haswell low-power laptop",
        ),
        (P::Haswell, U) => c(
            &[
                "MacBookPro11,1",
                "MacBookAir6,2",
                "MacBookPro11,4",
                "MacBookPro14,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Haswell U-series laptop",
        ),
        (P::Haswell, H) => c(
            &[
                "MacBookPro11,2",
                "MacBookPro11,3",
                "MacBookPro11,4",
                "MacBookPro11,5",
                "MacBookPro14,1",
                "MacBookPro14,3",
                "MacBookPro15,1",
                "MacBookPro16,1",
            ],
            "MacBookPro16,1",
            "Haswell quad-core laptop",
        ),
        (P::Broadwell, Y) => c(
            &[
                "MacBook8,1",
                "MacBookAir7,2",
                "MacBookPro14,1",
                "MacBookAir8,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Broadwell Core M laptop",
        ),
        (P::Broadwell, U) => c(
            &[
                "MacBookPro12,1",
                "MacBookAir7,2",
                "MacBookPro14,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Broadwell U-series laptop",
        ),
        (P::Broadwell, H) => c(
            &[
                "MacBookPro11,4",
                "MacBookPro11,5",
                "MacBookPro14,1",
                "MacBookPro15,1",
                "MacBookPro16,1",
            ],
            "MacBookPro16,1",
            "Broadwell quad-core laptop",
        ),
        (P::Skylake, Y) => c(
            &[
                "MacBook9,1",
                "MacBookPro14,1",
                "MacBookAir8,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Skylake Core m laptop",
        ),
        (P::Skylake, U) => c(
            &[
                "MacBookPro13,1",
                "MacBookPro13,2",
                "MacBookPro14,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Skylake U-series laptop",
        ),
        (P::Skylake, H) => c(
            &[
                "MacBookPro13,3",
                "MacBookPro14,1",
                "MacBookPro14,3",
                "MacBookPro15,1",
                "MacBookPro16,1",
            ],
            "MacBookPro16,1",
            "Skylake quad-core laptop",
        ),
        (P::KabyLake, Y) if amber => c(
            &[
                "MacBookAir8,1",
                "MacBookAir8,2",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Amber Lake laptop",
        ),
        (P::KabyLake, Y) => c(
            &[
                "MacBook10,1",
                "MacBookAir8,1",
                "MacBookPro15,2",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Kaby Lake Core m laptop",
        ),
        (P::KabyLake, U) => c(
            &[
                "MacBookPro14,1",
                "MacBookPro14,2",
                "MacBookPro15,2",
                "MacBookPro15,4",
                "MacBookPro16,2",
            ],
            "MacBookPro16,2",
            "Kaby Lake U-series laptop",
        ),
        (P::KabyLake, H) => c(
            &["MacBookPro14,3", "MacBookPro15,1", "MacBookPro16,1"],
            "MacBookPro16,1",
            "Kaby Lake quad-core laptop",
        ),
        // Dortania coffee-lake-plus: 9th gen H uses the 16" MacBookPro16,1
        // from Catalina on; the 8th gen page keeps MacBookPro15,1.
        (P::CoffeeLake, H) if ctx.intel_generation() == Some(9) => c(
            &["MacBookPro16,1", "MacBookPro15,1", "MacBookPro15,3"],
            "MacBookPro16,1",
            "Coffee Lake Plus H-series laptop",
        ),
        (P::CoffeeLake, H) => c(
            &["MacBookPro15,1", "MacBookPro15,3", "MacBookPro16,1"],
            "MacBookPro16,1",
            "Coffee Lake H-series laptop",
        ),
        (P::CoffeeLake, _) => c(
            &["MacBookPro15,2", "MacBookPro15,4", "MacBookPro16,2"],
            "MacBookPro16,2",
            "Coffee Lake / Whiskey Lake U-series laptop",
        ),
        (P::CometLake, H) => c(
            &["MacBookPro16,1", "MacBookPro16,4"],
            "MacBookPro16,1",
            "Comet Lake H-series laptop",
        ),
        (P::CometLake, _) => c(
            &["MacBookPro16,3", "MacBookPro16,2"],
            "MacBookPro16,2",
            "Comet Lake U-series laptop",
        ),
        (P::IceLake, _) => c(
            &["MacBookAir9,1", "MacBookPro16,2"],
            "MacBookPro16,2",
            "Ice Lake laptop",
        ),
        _ => laptop_dgpu(),
    }
}

// ── Secure Boot ─────────────────────────────────────────────────────────────

/// `Misc/Security/SecureBootModel` per research-opencore-macos §5.4:
/// - `Disabled` for macOS 14+ (OTA updates from 14.4 on fail with a T2
///   model and Apple Secure Boot), with the board-id skip and when root
///   patches are expected (they break the sealed system volume);
/// - `Default` for 11-13 (resolves to the T2 model or x86legacy);
/// - for 10.13-10.15 `Default` only with a T2 model the installer knows
///   (x86legacy needs 11.0.1), never with NVIDIA Web Drivers.
///
/// "The installer knows the model" means the model shipped with an older
/// point release than the one the recovery server hands out: High Sierra
/// recovery is 10.13.6 17G65 (research-recovery §4.3), older than the
/// 10.13.6 17G2112 build the 2018 MacBookPro15,1/15,2 need (j680/j132,
/// research-opencore-macos §5.1).
pub fn secure_boot_model(
    ctx: &PlanContext,
    display: &DisplayPlan,
    model: &SmbiosModel,
    board_id_skip: bool,
) -> &'static str {
    const DEFAULT: &str = "Default";
    const DISABLED: &str = "Disabled";
    let target = ctx.target;
    // The graphics part of the root-patch decision is all that is known
    // before the kexts stage; Wi-Fi and audio root patches only exist from
    // macOS 14 on, where Secure Boot is off anyway.
    let root_patch = graphics::needs_root_patch_graphics(ctx, display)
        || graphics::uses_nvidia_web_driver(ctx, display);
    if board_id_skip || target >= MacOsVersion::Sonoma || root_patch {
        return DISABLED;
    }
    if target >= MacOsVersion::BigSur {
        return DEFAULT;
    }
    // A hypervisor makes Default resolve to x86legacy, which needs 11.0.1.
    if ctx.is_vm {
        return DISABLED;
    }
    let installer_knows_model = recovery_release(target)
        .is_some_and(|installer| release_before(model.min_release, installer));
    if model.secure_boot_model.is_some()
        && smbios_db::supports(model.model, target)
        && installer_knows_model
    {
        DEFAULT
    } else {
        DISABLED
    }
}

/// Point release Apple's recovery server installs for releases before Big
/// Sur (research-recovery §4.3: 17G65, 18G87, 19H2).
fn recovery_release(target: MacOsVersion) -> Option<&'static str> {
    match target {
        MacOsVersion::HighSierra => Some("10.13.6"),
        MacOsVersion::Mojave => Some("10.14.6"),
        MacOsVersion::Catalina => Some("10.15.7"),
        _ => None,
    }
}

/// `a` is an older dotted release than `b` ("10.13.2" < "10.13.6").
fn release_before(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a) < parse(b)
}

/// OCLP's "Skip Board ID check" Booter patch (research-opencore-macos §3.6,
/// research-intel-desktop §7.1; same bytes in OCLP's config payload and the
/// 5T33Z0 board-id guide): boot.efi looks for "PlatformSupport.plist"
/// (UTF-16) and finds nothing once every character is a dot.
///
/// OCLP's companion "Reroute HW_BID to OC_BID" patch is not added: it makes
/// boot.efi read the board-id from an `OC_BID` NVRAM variable that only OCLP's
/// own SMBIOS spoofing sets, and OCLP itself dropped it in favour of this
/// patch (changelog: "Drop usage of HW_BID rerouting in boot.efi, patch out
/// PlatformSupport.plist instead"); it ships disabled in OCLP's config.
pub fn board_id_skip_patch() -> BinaryPatch {
    BinaryPatch {
        comment: "Skip Board ID check".into(),
        arch: "x86_64".into(),
        identifier: "Apple".into(),
        base: String::new(),
        find:
            "0050006C006100740066006F0072006D0053007500700070006F00720074002E0070006C006900730074"
                .into(),
        mask: String::new(),
        replace:
            "002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E"
                .into(),
        replace_mask: String::new(),
        count: 0,
        limit: 0,
        skip: 0,
        min_kernel: String::new(),
        max_kernel: String::new(),
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{empty_plan, validate};
    use super::*;
    use crate::domain::cpu_db;
    use crate::domain::model::{BuildOptions, HardwareProfile, ProfileGpu, VmKind};
    use MacOsVersion::*;

    fn run_with(p: &HardwareProfile, o: &BuildOptions, d: &DisplayPlan) -> BuildPlan {
        let ctx = PlanContext::new(p, o);
        let mut plan = empty_plan(o.target);
        apply(&ctx, d, &mut plan).unwrap();
        plan
    }

    fn run(p: &HardwareProfile, target: MacOsVersion, d: &DisplayPlan) -> BuildPlan {
        run_with(p, &options(target), d)
    }

    fn igpu_family(platform: P) -> Option<GpuFamily> {
        use GpuFamily as G;
        Some(match platform {
            P::Penryn => G::IntelGma,
            P::Lynnfield | P::Arrandale => G::IntelIronLake,
            P::SandyBridge => G::IntelSandyBridge,
            P::IvyBridge => G::IntelIvyBridge,
            P::Haswell => G::IntelHaswell,
            P::Broadwell => G::IntelBroadwell,
            P::Skylake => G::IntelSkylake,
            P::KabyLake => G::IntelKabyLake,
            P::CoffeeLake => G::IntelCoffeeLake,
            P::CometLake => G::IntelCometLake,
            P::IceLake => G::IntelIceLake,
            P::AmdZen | P::AmdZen2 | P::AmdZen3 => G::AmdApuVega,
            _ => return None,
        })
    }

    fn desktop(
        platform: P,
        name: &str,
        codename: &str,
        cores: u32,
        gpus: Vec<ProfileGpu>,
    ) -> HardwareProfile {
        profile(
            cpu(platform, name, codename, cores),
            FormFactor::Desktop,
            gpus,
        )
    }

    fn laptop(platform: P, name: &str, codename: &str, cores: u32) -> HardwareProfile {
        let family = igpu_family(platform).unwrap_or(GpuFamily::IntelXe);
        profile(
            cpu(platform, name, codename, cores),
            FormFactor::Laptop,
            vec![gpu(family, true)],
        )
    }

    fn igpu_only() -> DisplayPlan {
        display(Some(0), Some(0), false)
    }

    fn dgpu_headless() -> DisplayPlan {
        display(Some(0), Some(1), true)
    }

    fn dgpu_only() -> DisplayPlan {
        display(Some(0), None, false)
    }

    fn with_dgpu(
        platform: P,
        name: &str,
        codename: &str,
        cores: u32,
        dgpu: GpuFamily,
    ) -> HardwareProfile {
        let mut gpus = vec![gpu(dgpu, false)];
        if let Some(f) = igpu_family(platform) {
            gpus.push(gpu(f, true));
        }
        desktop(platform, name, codename, cores, gpus)
    }

    #[test]
    fn coffee_lake_desktop_follows_releases() {
        let p = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8700K",
            "Coffee Lake-S",
            6,
            vec![gpu(GpuFamily::IntelCoffeeLake, true)],
        );
        let expect = [
            (HighSierra, "iMac18,1"),
            (Mojave, "iMac19,1"),
            (BigSur, "iMac19,1"),
            (Sequoia, "iMac19,1"),
            (Tahoe, "iMac20,1"),
        ];
        for (target, model) in expect {
            let plan = run(&p, target, &igpu_only());
            assert_eq!(plan.smbios.model, model, "{target:?}");
            assert!(!plan.smbios.board_id_skip);
            assert!(plan.booter_patches.is_empty());
        }
        // Sequoia: iMac20,1 is offered as the Tahoe-proof alternative.
        let plan = run(&p, Sequoia, &igpu_only());
        assert!(plan.smbios.alternatives.contains(&"iMac20,1".to_string()));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.component == "smbios" && n.detail.contains("iMac20,1")));
        // dGPU + headless iGPU.
        let p = with_dgpu(
            P::CoffeeLake,
            "Intel(R) Core(TM) i9-9900K",
            "Coffee Lake-S",
            8,
            GpuFamily::AmdPolaris,
        );
        assert_eq!(
            run(&p, HighSierra, &dgpu_headless()).smbios.model,
            "iMac18,3"
        );
        assert_eq!(run(&p, Sonoma, &dgpu_headless()).smbios.model, "iMac19,1");
        // F CPU: no iGPU.
        let p = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i5-9400F",
            "Coffee Lake-S",
            6,
            vec![gpu(GpuFamily::AmdNavi10, false)],
        );
        assert_eq!(run(&p, Catalina, &dgpu_only()).smbios.model, "MacPro7,1");
        assert_eq!(run(&p, Mojave, &dgpu_only()).smbios.model, "iMacPro1,1");
        assert_eq!(run(&p, Tahoe, &dgpu_only()).smbios.model, "MacPro7,1");
    }

    #[test]
    fn older_intel_desktops() {
        let p = desktop(
            P::Haswell,
            "Intel(R) Core(TM) i7-4770",
            "Haswell",
            4,
            vec![gpu(GpuFamily::IntelHaswell, true)],
        );
        let expect = [
            (BigSur, "iMac14,4"),
            (Monterey, "iMac16,2"),
            (Ventura, "iMac18,1"),
            (Sonoma, "iMac19,1"),
            (Tahoe, "iMac20,1"),
        ];
        for (target, model) in expect {
            assert_eq!(
                run(&p, target, &igpu_only()).smbios.model,
                model,
                "Haswell iGPU {target:?}"
            );
        }
        let p = with_dgpu(
            P::Haswell,
            "Intel(R) Core(TM) i7-4790K",
            "Haswell",
            4,
            GpuFamily::AmdPolaris,
        );
        assert_eq!(run(&p, Catalina, &dgpu_headless()).smbios.model, "iMac15,1");
        assert_eq!(run(&p, Monterey, &dgpu_headless()).smbios.model, "iMac17,1");
        assert_eq!(run(&p, Ventura, &dgpu_only()).smbios.model, "MacPro7,1");
        let p = desktop(
            P::Skylake,
            "Intel(R) Core(TM) i7-6700K",
            "Skylake-S",
            4,
            vec![gpu(GpuFamily::IntelSkylake, true)],
        );
        assert_eq!(run(&p, Monterey, &igpu_only()).smbios.model, "iMac17,1");
        assert_eq!(run(&p, Ventura, &igpu_only()).smbios.model, "iMac18,1");
        let p = desktop(
            P::KabyLake,
            "Intel(R) Core(TM) i7-7700K",
            "Kaby Lake-S",
            4,
            vec![gpu(GpuFamily::IntelKabyLake, true)],
        );
        assert_eq!(run(&p, Ventura, &igpu_only()).smbios.model, "iMac18,1");
        assert_eq!(run(&p, Sonoma, &igpu_only()).smbios.model, "iMac19,1");
        assert_eq!(run(&p, Tahoe, &igpu_only()).smbios.model, "iMac20,1");
        let p = desktop(
            P::CometLake,
            "Intel(R) Core(TM) i9-10900K",
            "Comet Lake-S",
            10,
            vec![gpu(GpuFamily::IntelCometLake, true)],
        );
        assert_eq!(run(&p, Tahoe, &igpu_only()).smbios.model, "iMac20,2");
        let p = desktop(
            P::CometLake,
            "Intel(R) Core(TM) i7-10700K",
            "Comet Lake-S",
            8,
            vec![gpu(GpuFamily::IntelCometLake, true)],
        );
        let plan = run(&p, Catalina, &igpu_only());
        assert_eq!(plan.smbios.model, "iMac20,1");
        assert_eq!(plan.smbios.alternatives, vec!["iMac20,2".to_string()]);
    }

    #[test]
    fn pre_haswell_desktops_and_cryptexfixup_paths() {
        let p = desktop(
            P::IvyBridge,
            "Intel(R) Core(TM) i7-3770",
            "Ivy Bridge",
            4,
            vec![gpu(GpuFamily::IntelIvyBridge, true)],
        );
        assert_eq!(run(&p, Catalina, &igpu_only()).smbios.model, "iMac13,1");
        assert_eq!(run(&p, BigSur, &igpu_only()).smbios.model, "iMac14,4");
        let plan = run(&p, Monterey, &igpu_only());
        assert_eq!(plan.smbios.model, "iMac13,1");
        assert!(plan.smbios.board_id_skip);
        assert_eq!(plan.smbios.secure_boot_model, "Disabled");
        assert_eq!(plan.booter_patches.len(), 1);
        assert_eq!(plan.booter_patches[0].comment, "Skip Board ID check");

        let p = with_dgpu(
            P::IvyBridge,
            "Intel(R) Core(TM) i5-3570K",
            "Ivy Bridge",
            4,
            GpuFamily::AmdPolaris,
        );
        assert_eq!(
            run(&p, Monterey, &dgpu_headless()).smbios.model,
            "MacPro6,1"
        );
        let plan = run(&p, Ventura, &dgpu_headless());
        assert_eq!(
            (plan.smbios.model.as_str(), plan.smbios.board_id_skip),
            ("iMac13,2", true)
        );
        let plan = run(&p, Tahoe, &dgpu_only());
        assert_eq!(
            (plan.smbios.model.as_str(), plan.smbios.board_id_skip),
            ("MacPro6,1", true)
        );

        let p = with_dgpu(
            P::SandyBridge,
            "Intel(R) Core(TM) i7-2600K",
            "Sandy Bridge",
            4,
            GpuFamily::AmdGcn1,
        );
        assert_eq!(
            run(&p, HighSierra, &dgpu_headless()).smbios.model,
            "iMac12,2"
        );
        assert_eq!(run(&p, Mojave, &dgpu_only()).smbios.model, "MacPro6,1");
        let p = with_dgpu(
            P::Lynnfield,
            "Intel(R) Core(TM) i5-750",
            "Lynnfield",
            4,
            GpuFamily::AmdPolaris,
        );
        assert_eq!(run(&p, HighSierra, &dgpu_only()).smbios.model, "iMac11,1");
        assert_eq!(run(&p, Monterey, &dgpu_only()).smbios.model, "MacPro6,1");
        let p = with_dgpu(
            P::Penryn,
            "Intel(R) Core(TM)2 Quad Q9550",
            "Penryn",
            4,
            GpuFamily::NvidiaKepler,
        );
        assert_eq!(run(&p, HighSierra, &dgpu_only()).smbios.model, "iMac10,1");
        assert_eq!(run(&p, Catalina, &dgpu_only()).smbios.model, "MacPro6,1");
    }

    #[test]
    fn hedt() {
        let hedt = |platform, name, codename| {
            desktop(
                platform,
                name,
                codename,
                8,
                vec![gpu(GpuFamily::AmdPolaris, false)],
            )
        };
        let p = hedt(P::NehalemHedt, "Intel(R) Core(TM) i7 CPU 920", "Bloomfield");
        assert_eq!(run(&p, Mojave, &dgpu_only()).smbios.model, "MacPro5,1");
        assert_eq!(run(&p, Catalina, &dgpu_only()).smbios.model, "MacPro6,1");
        let p = hedt(P::IvyBridgeE, "Intel(R) Core(TM) i7-4930K", "Ivy Bridge-E");
        assert_eq!(run(&p, Monterey, &dgpu_only()).smbios.model, "MacPro6,1");
        assert!(run(&p, Sonoma, &dgpu_only()).smbios.board_id_skip);
        let p = hedt(P::HaswellE, "Intel(R) Core(TM) i7-5960X", "Haswell-E");
        assert_eq!(run(&p, Sequoia, &dgpu_only()).smbios.model, "iMacPro1,1");
        assert_eq!(run(&p, Tahoe, &dgpu_only()).smbios.model, "MacPro7,1");
        let p = hedt(P::SkylakeX, "Intel(R) Core(TM) i9-7900X", "Skylake-X");
        assert_eq!(run(&p, HighSierra, &dgpu_only()).smbios.model, "iMacPro1,1");
        assert_eq!(run(&p, Tahoe, &dgpu_only()).smbios.model, "MacPro7,1");
        let p = hedt(
            P::CascadeLakeX,
            "Intel(R) Core(TM) i9-10980XE",
            "Cascade Lake-X",
        );
        assert_eq!(run(&p, Catalina, &dgpu_only()).smbios.model, "MacPro7,1");
        assert_eq!(run(&p, Mojave, &dgpu_only()).smbios.model, "iMacPro1,1");
    }

    #[test]
    fn hybrid_and_newer_intel_desktops_use_workstation_models() {
        for (platform, name) in [
            (P::RocketLake, "Intel(R) Core(TM) i7-11700K"),
            (P::AlderLake, "12th Gen Intel(R) Core(TM) i9-12900K"),
            (P::RaptorLake, "13th Gen Intel(R) Core(TM) i7-13700K"),
            (P::ArrowLake, "Intel(R) Core(TM) Ultra 9 285K"),
        ] {
            let p = desktop(
                platform,
                name,
                "x",
                8,
                vec![
                    gpu(GpuFamily::AmdNavi23, false),
                    gpu(GpuFamily::IntelXe, true),
                ],
            );
            for target in [Monterey, Sequoia, Tahoe] {
                let plan = run(&p, target, &dgpu_only());
                assert_eq!(plan.smbios.model, "MacPro7,1", "{platform:?} {target:?}");
                assert!(
                    plan.smbios.alternatives.contains(&"iMacPro1,1".to_string()) || target == Tahoe
                );
            }
        }
    }

    #[test]
    fn intel_laptops_by_class() {
        let cases: &[(P, &str, &str, u32, MacOsVersion, &str)] = &[
            (
                P::Arrandale,
                "Intel(R) Core(TM) i5 CPU M 520",
                "Arrandale",
                2,
                HighSierra,
                "MacBookPro6,2",
            ),
            (
                P::SandyBridge,
                "Intel(R) Core(TM) i5-2520M",
                "Sandy Bridge",
                2,
                HighSierra,
                "MacBookPro8,1",
            ),
            (
                P::SandyBridge,
                "Intel(R) Core(TM) i7-2720QM",
                "Sandy Bridge",
                4,
                HighSierra,
                "MacBookPro8,2",
            ),
            (
                P::SandyBridge,
                "Intel(R) Core(TM) i7-2677M",
                "Sandy Bridge",
                2,
                HighSierra,
                "MacBookAir4,2",
            ),
            (
                P::IvyBridge,
                "Intel(R) Core(TM) i5-3320M",
                "Ivy Bridge",
                2,
                Catalina,
                "MacBookPro10,2",
            ),
            (
                P::IvyBridge,
                "Intel(R) Core(TM) i7-3720QM",
                "Ivy Bridge",
                4,
                BigSur,
                "MacBookPro11,2",
            ),
            (
                P::IvyBridge,
                "Intel(R) Core(TM) i7-3720QM",
                "Ivy Bridge",
                4,
                Monterey,
                "MacBookPro11,4",
            ),
            (
                P::Haswell,
                "Intel(R) Core(TM) i5-4200U",
                "Haswell-ULT",
                2,
                BigSur,
                "MacBookPro11,1",
            ),
            (
                P::Haswell,
                "Intel(R) Core(TM) i5-4200U",
                "Haswell-ULT",
                2,
                Monterey,
                "MacBookPro11,4",
            ),
            (
                P::Haswell,
                "Intel(R) Core(TM) i7-4710HQ",
                "Haswell-H",
                4,
                Ventura,
                "MacBookPro14,1",
            ),
            (
                P::Haswell,
                "Intel(R) Core(TM) i7-4710HQ",
                "Haswell-H",
                4,
                Sonoma,
                "MacBookPro15,1",
            ),
            (
                P::Broadwell,
                "Intel(R) Core(TM) i5-5200U",
                "Broadwell-U",
                2,
                Monterey,
                "MacBookPro12,1",
            ),
            (
                P::Broadwell,
                "Intel(R) Core(TM) M-5Y10c",
                "Broadwell-Y",
                2,
                BigSur,
                "MacBook8,1",
            ),
            (
                P::Skylake,
                "Intel(R) Core(TM) i5-6200U",
                "Skylake-U",
                2,
                Monterey,
                "MacBookPro13,1",
            ),
            (
                P::Skylake,
                "Intel(R) Core(TM) i5-6200U",
                "Skylake-U",
                2,
                Ventura,
                "MacBookPro14,1",
            ),
            (
                P::Skylake,
                "Intel(R) Core(TM) i5-6200U",
                "Skylake-U",
                2,
                Sequoia,
                "MacBookPro15,2",
            ),
            (
                P::Skylake,
                "Intel(R) Core(TM) m3-6Y30",
                "Skylake-Y",
                2,
                Sonoma,
                "MacBookAir8,1",
            ),
            (
                P::Skylake,
                "Intel(R) Core(TM) i7-6700HQ",
                "Skylake-H",
                4,
                Monterey,
                "MacBookPro13,3",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) i5-7200U",
                "Kaby Lake-U",
                2,
                Ventura,
                "MacBookPro14,1",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) i5-8250U",
                "Kaby Lake-R",
                4,
                Sonoma,
                "MacBookPro15,2",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) i5-8250U",
                "Kaby Lake-R",
                4,
                Tahoe,
                "MacBookPro16,2",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) i5-8200Y",
                "Amber Lake-Y",
                2,
                Sonoma,
                "MacBookAir8,1",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) i5-8200Y",
                "Amber Lake-Y",
                2,
                Sequoia,
                "MacBookPro15,2",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) m3-7Y30",
                "Kaby Lake-Y",
                2,
                Ventura,
                "MacBook10,1",
            ),
            (
                P::KabyLake,
                "Intel(R) Core(TM) i7-7700HQ",
                "Kaby Lake-H",
                4,
                Ventura,
                "MacBookPro14,3",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-8750H",
                "Coffee Lake-H",
                6,
                HighSierra,
                "MacBookPro15,1",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-9750H",
                "Coffee Lake-H",
                6,
                Tahoe,
                "MacBookPro16,1",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-9750H",
                "Coffee Lake-H",
                6,
                Catalina,
                "MacBookPro16,1",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-9750H",
                "Coffee Lake-H",
                6,
                Mojave,
                "MacBookPro15,1",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-8850H",
                "Coffee Lake-H",
                6,
                Sequoia,
                "MacBookPro15,1",
            ),
            (
                P::IvyBridge,
                "Intel(R) Core(TM) i5-3317U",
                "Ivy Bridge",
                2,
                Catalina,
                "MacBookAir5,2",
            ),
            (
                P::IvyBridge,
                "Intel(R) Core(TM) i5-3317U",
                "Ivy Bridge",
                2,
                BigSur,
                "MacBookAir6,2",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-8565U",
                "Whiskey Lake-U",
                4,
                Sequoia,
                "MacBookPro15,2",
            ),
            (
                P::CoffeeLake,
                "Intel(R) Core(TM) i7-8565U",
                "Whiskey Lake-U",
                4,
                Tahoe,
                "MacBookPro16,2",
            ),
            (
                P::CometLake,
                "Intel(R) Core(TM) i5-10210U",
                "Comet Lake-U",
                4,
                Sequoia,
                "MacBookPro16,3",
            ),
            (
                P::CometLake,
                "Intel(R) Core(TM) i5-10210U",
                "Comet Lake-U",
                4,
                Tahoe,
                "MacBookPro16,2",
            ),
            (
                P::CometLake,
                "Intel(R) Core(TM) i7-10750H",
                "Comet Lake-H",
                6,
                Catalina,
                "MacBookPro16,1",
            ),
            (
                P::IceLake,
                "Intel(R) Core(TM) i5-1035G7",
                "Ice Lake-U",
                4,
                Sequoia,
                "MacBookAir9,1",
            ),
            (
                P::IceLake,
                "Intel(R) Core(TM) i5-1035G7",
                "Ice Lake-U",
                4,
                Tahoe,
                "MacBookPro16,2",
            ),
        ];
        for (platform, name, codename, cores, target, model) in cases {
            let p = laptop(*platform, name, codename, *cores);
            let plan = run(&p, *target, &igpu_only());
            assert_eq!(plan.smbios.model, *model, "{name} on {target:?}");
            assert!(!plan.smbios.board_id_skip, "{name} on {target:?}");
        }
        // Sandy/Ivy laptops on Ventura+ (CryptexFixup) keep their own model.
        let p = laptop(P::IvyBridge, "Intel(R) Core(TM) i5-3320M", "Ivy Bridge", 2);
        let plan = run(&p, Sonoma, &igpu_only());
        assert_eq!(
            (plan.smbios.model.as_str(), plan.smbios.board_id_skip),
            ("MacBookPro10,2", true)
        );
        let p = laptop(
            P::SandyBridge,
            "Intel(R) Core(TM) i5-2520M",
            "Sandy Bridge",
            2,
        );
        let plan = run(&p, Catalina, &igpu_only());
        assert_eq!(
            (plan.smbios.model.as_str(), plan.smbios.board_id_skip),
            ("MacBookPro8,1", true)
        );
        // Tiger Lake laptop whose panel runs on an AMD dGPU.
        let mut p = laptop(
            P::TigerLake,
            "11th Gen Intel(R) Core(TM) i7-11800H",
            "Tiger Lake-H",
            8,
        );
        p.gpus.insert(0, gpu(GpuFamily::AmdNavi23, false));
        assert_eq!(
            run(&p, Sequoia, &dgpu_only()).smbios.model,
            "MacBookPro16,1"
        );
    }

    #[test]
    fn mini_pcs() {
        let mini = |platform, name: &str, codename: &str, cores| {
            let mut c = cpu(platform, name, codename, cores);
            c.is_mobile = true;
            profile(
                c,
                FormFactor::MiniPc,
                vec![gpu(igpu_family(platform).unwrap(), true)],
            )
        };
        let p = mini(P::KabyLake, "Intel(R) Core(TM) i5-7260U", "Kaby Lake-U", 2);
        assert_eq!(run(&p, Ventura, &igpu_only()).smbios.model, "iMac18,1");
        assert_eq!(run(&p, Sonoma, &igpu_only()).smbios.model, "Macmini8,1");
        assert_eq!(run(&p, Tahoe, &igpu_only()).smbios.model, "iMac20,1");
        let p = mini(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8559U",
            "Coffee Lake-U",
            4,
        );
        assert_eq!(run(&p, Mojave, &igpu_only()).smbios.model, "Macmini8,1");
        assert_eq!(run(&p, HighSierra, &igpu_only()).smbios.model, "iMac18,1");
        let p = mini(P::IvyBridge, "Intel(R) Core(TM) i5-3427U", "Ivy Bridge", 2);
        assert_eq!(run(&p, Monterey, &igpu_only()).smbios.model, "Macmini7,1");
        // A desktop CPU in a mini PC uses the desktop table.
        let p = profile(
            cpu(
                P::CometLake,
                "Intel(R) Core(TM) i5-10500T",
                "Comet Lake-S",
                6,
            ),
            FormFactor::MiniPc,
            vec![gpu(GpuFamily::IntelCometLake, true)],
        );
        assert_eq!(run(&p, Sequoia, &igpu_only()).smbios.model, "iMac20,1");
    }

    #[test]
    fn amd_systems() {
        let p = desktop(
            P::AmdZen3,
            "AMD Ryzen 7 5800X 8-Core Processor",
            "Vermeer",
            8,
            vec![gpu(GpuFamily::AmdNavi21, false)],
        );
        assert_eq!(run(&p, Mojave, &dgpu_only()).smbios.model, "iMacPro1,1");
        assert_eq!(run(&p, Sequoia, &dgpu_only()).smbios.model, "MacPro7,1");
        assert_eq!(run(&p, Tahoe, &dgpu_only()).smbios.model, "MacPro7,1");
        let p = desktop(
            P::AmdZen2,
            "AMD Ryzen 5 3600 6-Core Processor",
            "Matisse",
            6,
            vec![gpu(GpuFamily::AmdGcn2, false)],
        );
        assert_eq!(run(&p, Monterey, &dgpu_only()).smbios.model, "MacPro6,1");
        let p = desktop(
            P::AmdBulldozer,
            "AMD FX(tm)-8350 Eight-Core Processor",
            "Vishera",
            4,
            vec![gpu(GpuFamily::AmdPolaris, false)],
        );
        let plan = run(&p, Sonoma, &dgpu_only());
        assert_eq!(plan.smbios.model, "MacPro7,1");
        assert!(!plan.smbios.board_id_skip);
        let p = desktop(
            P::AmdZen2,
            "AMD Ryzen 5 PRO 4650G with Radeon Graphics",
            "Renoir",
            6,
            vec![gpu(GpuFamily::AmdApuVega, true)],
        );
        let plan = run(&p, Monterey, &igpu_only());
        assert_eq!(plan.smbios.model, "iMac20,1");
        assert_eq!(plan.smbios.secure_boot_model, "Default");
        let p = profile(
            cpu(
                P::AmdZen3,
                "AMD Ryzen 7 5700U with Radeon Graphics",
                "Lucienne",
                8,
            ),
            FormFactor::Laptop,
            vec![gpu(GpuFamily::AmdApuVega, true)],
        );
        assert_eq!(run(&p, Tahoe, &igpu_only()).smbios.model, "MacBookPro16,2");
        let p = desktop(
            P::AmdZen4,
            "AMD Ryzen Threadripper 7960X 24-Cores",
            "Storm Peak",
            24,
            vec![gpu(GpuFamily::AmdNavi21, false)],
        );
        let ctx_o = options(Tahoe);
        let ctx = PlanContext::new(&p, &ctx_o);
        assert_eq!(
            candidates(&ctx, &dgpu_only()).class,
            "AMD Threadripper workstation"
        );
    }

    #[test]
    fn virtual_machines() {
        let p = vm_profile(P::Unknown, VmKind::Kvm);
        let d = display(None, None, false);
        let plan = run(&p, Sequoia, &d);
        assert_eq!(plan.smbios.model, "iMacPro1,1");
        assert_eq!(plan.smbios.secure_boot_model, "Disabled");
        let plan = run(&p, Monterey, &d);
        assert_eq!(plan.smbios.secure_boot_model, "Default");
        let plan = run(&p, Catalina, &d);
        assert_eq!(
            plan.smbios.secure_boot_model, "Disabled",
            "x86legacy needs 11.0.1"
        );
        assert_eq!(run(&p, Tahoe, &d).smbios.model, "MacPro7,1");
    }

    #[test]
    fn secure_boot_models() {
        let p = desktop(
            P::CometLake,
            "Intel(R) Core(TM) i7-10700K",
            "Comet Lake-S",
            8,
            vec![gpu(GpuFamily::IntelCometLake, true)],
        );
        assert_eq!(
            run(&p, Catalina, &igpu_only()).smbios.secure_boot_model,
            "Default",
            "T2 iMac20,1 on 10.15"
        );
        assert_eq!(
            run(&p, Ventura, &igpu_only()).smbios.secure_boot_model,
            "Default"
        );
        assert_eq!(
            run(&p, Sonoma, &igpu_only()).smbios.secure_boot_model,
            "Disabled"
        );
        assert_eq!(
            run(&p, Tahoe, &igpu_only()).smbios.secure_boot_model,
            "Disabled"
        );
        let p = desktop(
            P::KabyLake,
            "Intel(R) Core(TM) i7-7700K",
            "Kaby Lake-S",
            4,
            vec![gpu(GpuFamily::IntelKabyLake, true)],
        );
        assert_eq!(
            run(&p, Mojave, &igpu_only()).smbios.secure_boot_model,
            "Disabled",
            "iMac18,1 has no T2"
        );
        assert_eq!(
            run(&p, Monterey, &igpu_only()).smbios.secure_boot_model,
            "Default"
        );
        // Kepler on Monterey needs a root patch.
        let p = with_dgpu(
            P::Haswell,
            "Intel(R) Core(TM) i7-4790K",
            "Haswell",
            4,
            GpuFamily::NvidiaKepler,
        );
        assert_eq!(
            run(&p, Monterey, &dgpu_headless()).smbios.secure_boot_model,
            "Disabled"
        );
        assert_eq!(
            run(&p, BigSur, &dgpu_headless()).smbios.secure_boot_model,
            "Default"
        );
        // Web drivers on High Sierra with a T2 SMBIOS.
        let p = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i5-9400F",
            "Coffee Lake-S",
            6,
            vec![gpu(GpuFamily::NvidiaPascal, false)],
        );
        let plan = run(&p, HighSierra, &dgpu_only());
        assert_eq!(plan.smbios.model, "iMacPro1,1");
        assert_eq!(plan.smbios.secure_boot_model, "Disabled");
        let p = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i5-9400F",
            "Coffee Lake-S",
            6,
            vec![gpu(GpuFamily::AmdPolaris, false)],
        );
        assert_eq!(
            run(&p, HighSierra, &dgpu_only()).smbios.secure_boot_model,
            "Default"
        );
        // MacBookPro15,1/15,2 need 10.13.6 17G2112; recovery installs 17G65.
        let p = laptop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8750H",
            "Coffee Lake-H",
            6,
        );
        let plan = run(&p, HighSierra, &igpu_only());
        assert_eq!(plan.smbios.model, "MacBookPro15,1");
        assert_eq!(plan.smbios.secure_boot_model, "Disabled");
        assert_eq!(
            run(&p, Mojave, &igpu_only()).smbios.secure_boot_model,
            "Default",
            "10.14.6 knows the 2018 MacBook Pro"
        );
        let p = laptop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i5-8259U",
            "Coffee Lake-U",
            4,
        );
        let plan = run(&p, HighSierra, &igpu_only());
        assert_eq!(
            (
                plan.smbios.model.as_str(),
                plan.smbios.secure_boot_model.as_str()
            ),
            ("MacBookPro15,2", "Disabled")
        );
        assert!(release_before("10.13.2", "10.13.6"));
        assert!(!release_before("10.13.6", "10.13.6"));
        assert!(release_before("10.15.6", "10.15.7"));
        assert!(!release_before("11", "10.15.7"));
    }

    #[test]
    fn override_is_honoured() {
        let p = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8700K",
            "Coffee Lake-S",
            6,
            vec![gpu(GpuFamily::IntelCoffeeLake, true)],
        );
        let mut o = options(Tahoe);
        o.smbios_override = Some("imac19,1".into());
        let plan = run_with(&p, &o, &igpu_only());
        assert_eq!(plan.smbios.model, "iMac19,1");
        assert!(plan.smbios.board_id_skip);
        assert_eq!(plan.booter_patches.len(), 1);
        assert_eq!(plan.smbios.secure_boot_model, "Disabled");
        assert!(plan.smbios.alternatives.contains(&"iMac20,1".to_string()));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.level == NoteLevel::Warning && n.title.contains("does not support")));
        let skip = plan
            .notes
            .iter()
            .find(|n| n.title.starts_with("Board-id check skipped"))
            .expect("board-id note");
        assert!(skip
            .detail
            .starts_with("The chosen iMac19,1 stops at macOS 15.8.1."));
        assert!(!skip.detail.contains("-no_compat_check"));

        // An automatic skip before Big Sur names the hardware class and the
        // -no_compat_check fallback.
        let p_snb = laptop(
            P::SandyBridge,
            "Intel(R) Core(TM) i5-2520M",
            "Sandy Bridge",
            2,
        );
        let plan = run(&p_snb, Catalina, &igpu_only());
        let skip = plan
            .notes
            .iter()
            .find(|n| n.title.starts_with("Board-id check skipped"))
            .expect("board-id note");
        assert!(skip
            .detail
            .starts_with("No Mac model that matches this Sandy Bridge"));
        assert!(skip.detail.contains("-no_compat_check"));

        o.smbios_override = Some("MacPro7,1".into());
        o.target = Mojave;
        let plan = run_with(&p, &o, &igpu_only());
        assert_eq!(plan.smbios.model, "MacPro7,1");
        assert!(
            !plan.smbios.board_id_skip,
            "too-old target cannot be skipped"
        );

        o.smbios_override = Some("Nonsense9,9".into());
        let ctx = PlanContext::new(&p, &o);
        let err = apply(&ctx, &igpu_only(), &mut empty_plan(Mojave)).unwrap_err();
        assert_eq!(err.code, "SMBIOS_UNKNOWN");
    }

    #[test]
    fn board_id_patch_bytes() {
        let patch = board_id_skip_patch();
        let decode = |hex: &str| -> Vec<u8> {
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect()
        };
        let find = decode(&patch.find);
        let replace = decode(&patch.replace);
        assert_eq!(find.len(), replace.len());
        assert_eq!(find.len(), 42);
        // "\0P\0l\0a..." = UTF-16 "PlatformSupport.plist" seen one byte early.
        let text: String = find.iter().skip(1).step_by(2).map(|b| *b as char).collect();
        assert_eq!(text, "PlatformSupport.plist");
        assert!(replace.iter().skip(1).step_by(2).all(|b| *b == b'.'));
        assert_eq!(patch.identifier, "Apple");
        assert_eq!(patch.arch, "x86_64");
    }

    /// Every supported platform × form factor × display path × release the
    /// CPU allows gets a model from `smbios_db`; the board-id skip is used
    /// only when no candidate supports the release, and never for Haswell
    /// and newer, AMD or VMs.
    #[test]
    fn every_platform_and_release() {
        use GpuFamily as G;
        let forms = [
            FormFactor::Desktop,
            FormFactor::Laptop,
            FormFactor::MiniPc,
            FormFactor::AllInOne,
        ];
        for &platform in cpu_db::all_platforms() {
            let info = cpu_db::platform_info(platform);
            if !info.supported {
                continue;
            }
            for form in forms {
                let mut c = cpu(platform, "Test CPU", "Test", 4);
                c.is_mobile = form != FormFactor::Desktop;
                let igpu = igpu_family(platform);
                let layouts: Vec<(Vec<ProfileGpu>, DisplayPlan)> = {
                    let mut l = vec![(vec![gpu(G::AmdPolaris, false)], dgpu_only())];
                    if let Some(f) = igpu {
                        l.push((vec![gpu(f, true)], igpu_only()));
                        l.push((
                            vec![gpu(G::AmdPolaris, false), gpu(f, true)],
                            dgpu_headless(),
                        ));
                    }
                    l
                };
                for (gpus, d) in layouts {
                    let p = profile(c.clone(), form, gpus);
                    for target in MacOsVersion::ALL {
                        let o = options(target);
                        if validate(&p, &o).is_err() {
                            continue;
                        }
                        let plan = run_with(&p, &o, &d);
                        let ctx = PlanContext::new(&p, &o);
                        for m in candidates(&ctx, &d).models {
                            assert!(smbios_db::find(m).is_some(), "{m} is not in smbios_db");
                        }
                        let model = smbios_db::find(&plan.smbios.model).unwrap_or_else(|| {
                            panic!("{platform:?} {form:?} {target:?}: unknown model")
                        });
                        let supported = smbios_db::supports(model.model, target);
                        let label = format!("{platform:?} {form:?} {target:?} -> {}", model.model);
                        assert_eq!(
                            plan.smbios.board_id_skip,
                            !supported && target > model.max_os,
                            "{label}"
                        );
                        assert_eq!(
                            plan.booter_patches.len(),
                            usize::from(plan.smbios.board_id_skip),
                            "{label}"
                        );
                        if !pre_haswell(platform)
                            && !matches!(platform, P::NehalemHedt | P::SandyBridgeE | P::IvyBridgeE)
                        {
                            assert!(supported, "{label}: a supported model exists");
                        }
                        if target == Tahoe && supported {
                            assert_eq!(model.max_os, Tahoe, "{label}");
                        }
                        if target >= Sonoma || plan.smbios.board_id_skip {
                            assert_eq!(plan.smbios.secure_boot_model, "Disabled", "{label}");
                        }
                        if target < BigSur && plan.smbios.secure_boot_model == "Default" {
                            assert!(model.secure_boot_model.is_some(), "{label}: T2 only");
                            assert!(
                                model.model != "MacBookPro15,1" || target > HighSierra,
                                "{label}"
                            );
                        }
                        assert!(["Default", "Disabled"]
                            .contains(&plan.smbios.secure_boot_model.as_str()));
                        assert!(
                            !plan.smbios.alternatives.contains(&plan.smbios.model),
                            "{label}"
                        );
                        for alt in &plan.smbios.alternatives {
                            assert!(
                                smbios_db::supports(alt, target) || plan.smbios.board_id_skip,
                                "{label}: {alt}"
                            );
                        }
                        assert!(!plan.smbios.reason.is_empty());
                        let laptop_model = model.model.starts_with("MacBook");
                        if form == FormFactor::Laptop && !info.hedt {
                            assert!(laptop_model, "{label}: laptops get MacBook models");
                        }
                        if form == FormFactor::Desktop && platform != P::IceLake {
                            assert!(!laptop_model, "{label}: desktops never get MacBook models");
                        }
                    }
                }
            }
        }
    }
}
