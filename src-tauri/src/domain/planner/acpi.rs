//! SSDT selection per platform (Dortania ssdt-platform matrix), generated from
//! the machine's DSDT when available, prebuilt fallback otherwise; ACPI
//! renames and deletes.
//!
//! Every SSDT is first requested from `domain::acpi` with the best source
//! available: the indexed tables (exact `_STA` renames), the extracted facts
//! (guarded definitions), or nothing. A table the generator calls not needed
//! is skipped; one it cannot build falls back to the OpenCore sample or the
//! Dortania prebuilt table when a generic one is safe on that machine.

use std::path::Path;

use crate::domain::acpi::{self, AcpiTables, GeneratedSsdt, SsdtKind};
use crate::domain::gpu_db;
use crate::domain::model::{
    hex_upper, AcpiDelete, AcpiFacts, AcpiPatch, BuildPlan, CpuPlatform, CpuVendor, GpuFamily,
    InputBus, MacOsVersion, NoteLevel, PlanNote, PlistScalar, ProfileGpu, SsdtPlan, SsdtSource,
    VmKind,
};
use crate::error::AppError;

use super::{graphics, DisplayPlan, PlanContext};

const COMPONENT: &str = "acpi";

/// ACPI > Quirks: Dortania's config.plist pages leave the Sample.plist
/// defaults, where only ResetLogoStatus is on.
const QUIRKS: [(&str, bool); 6] = [
    ("FadtEnableReset", false),
    ("NormalizeHeaders", false),
    ("RebaseRegions", false),
    ("ResetHwSig", false),
    ("ResetLogoStatus", true),
    ("SyncTableIds", false),
];

pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    let tables = match load_tables(ctx) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "ACPI tables could not be loaded, using facts only");
            plan.notes.push(note(
                NoteLevel::Warning,
                "ACPI tables could not be read",
                format!(
                    "{} The SSDTs are built from the scanned facts or generic prebuilt tables instead.",
                    e.message
                ),
            ));
            None
        }
    };
    let facts = tables
        .as_ref()
        .map(AcpiTables::facts)
        .or_else(|| ctx.acpi.clone());
    let source = match (&tables, &facts) {
        (Some(t), _) => Source::Tables(t),
        (None, Some(f)) => Source::Facts(f),
        (None, None) => Source::Unknown,
    };

    let selection = select(ctx, display, plan, facts.as_ref(), tables.is_some());
    let mut built = Built::default();
    for request in selection.requests {
        built.build(request, &source);
    }

    plan.ssdts.extend(built.ssdts);
    for patch in built.patches {
        add_patch(&mut plan.acpi_patches, patch);
    }
    if ctx.profile.vm == Some(VmKind::HyperV) {
        hyper_v(plan);
    }
    if intel_800_series(ctx) {
        add_patch(&mut plan.acpi_patches, remove_conditional_scope());
    }
    for delete in deletes(ctx) {
        if !plan.acpi_deletes.iter().any(|d| {
            d.table_signature == delete.table_signature && d.oem_table_id == delete.oem_table_id
        }) {
            plan.acpi_deletes.push(delete);
        }
    }
    for (key, value) in QUIRKS {
        plan.acpi_quirks
            .insert(key.to_string(), PlistScalar::Bool(value));
    }

    if !built.prebuilt.is_empty() {
        plan.notes.push(note(
            NoteLevel::Info,
            "Generic prebuilt SSDTs",
            format!(
                "{} {} generic prebuilt (OpenCore sample / Dortania) because {}. Run the scan on the \
                 target machine with its ACPI tables dumped to get tables built for its exact paths.",
                built.prebuilt.join(", "),
                if built.prebuilt.len() == 1 { "is" } else { "are" },
                if matches!(source, Source::Unknown) {
                    "the machine's ACPI tables are not available"
                } else {
                    "the ACPI facts were not enough to build them"
                },
            ),
        ));
    }
    plan.notes.extend(selection.notes);
    plan.notes.extend(built.notes);
    plan.post_install.extend(post_install(ctx));
}

/// Backlight objects for an internal panel driven by a supported iGPU:
/// SSDT-PNLF with this `_UID`, plus SSDT-ALS0 from macOS 10.15 on (Dortania
/// backlight.md; OpenCorePkg SSDT-ALS0.dsl: "Starting with macOS 10.15
/// Ambient Light Sensor presence is required for backlight functioning").
/// The kexts stage adds SMCLightSensor exactly when `als0` is set (Dortania
/// ktext.md: only where an ambient light sensor exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelBacklight {
    pub pnlf_uid: u32,
    pub als0: bool,
}

/// The backlight decision for this plan; None without an internal panel, in
/// VMs, and when the panel is not on a usable iGPU (a dGPU drives it, the
/// iGPU is headless, disabled or turned off by the graphics stage).
pub fn panel_backlight(
    ctx: &PlanContext,
    display: &DisplayPlan,
    plan: &BuildPlan,
) -> Option<PanelBacklight> {
    if !ctx.has_panel || ctx.is_vm {
        return None;
    }
    Some(PanelBacklight {
        pnlf_uid: panel_pnlf_uid(ctx, display, plan)?,
        als0: ctx.target >= MacOsVersion::Catalina,
    })
}

/// SSDT-PNLF `_UID` for an iGPU generation, the WhateverGreen backlight
/// profile (OpenCore SSDT-PNLF.dsl): 14 Arrandale/Sandy/Ivy, 15 Haswell/
/// Broadwell, 16 Skylake/Kaby Lake, 19 Coffee Lake and newer. Vega APUs
/// driven by NootedRed use 19 as well (research-amd §6).
pub fn pnlf_uid(family: GpuFamily) -> Option<u32> {
    use GpuFamily as F;
    match family {
        F::IntelIronLake | F::IntelSandyBridge | F::IntelIvyBridge => Some(14),
        F::IntelHaswell | F::IntelBroadwell => Some(15),
        F::IntelSkylake | F::IntelKabyLake => Some(16),
        F::IntelCoffeeLake | F::IntelCometLake | F::IntelIceLake | F::AmdApuVega => Some(19),
        _ => None,
    }
}

// ── Sources ─────────────────────────────────────────────────────────────────

/// What the SSDTs are generated from.
enum Source<'a> {
    /// The machine's indexed DSDT/SSDTs: exact `_STA` renames are possible.
    Tables(&'a AcpiTables),
    /// Extracted facts only: existing objects are guarded, not renamed.
    Facts(&'a AcpiFacts),
    /// Nothing is known about the machine's ACPI.
    Unknown,
}

fn load_tables(ctx: &PlanContext) -> Result<Option<AcpiTables>, AppError> {
    match ctx.profile.acpi_tables_dir.as_deref().map(str::trim) {
        Some(dir) if !dir.is_empty() => acpi::load_tables(Path::new(dir)).map(Some),
        _ => Ok(None),
    }
}

/// Tables that need nothing from the machine: PNLF and XOSI define new
/// root objects, GPI0 and ALS0 guard every reference with `CondRefOf`.
fn self_contained(kind: &SsdtKind) -> bool {
    matches!(
        kind,
        SsdtKind::Pnlf { .. } | SsdtKind::Xosi | SsdtKind::Gpi0 | SsdtKind::Als0
    )
}

fn generate(kind: &SsdtKind, source: &Source) -> Result<GeneratedSsdt, AppError> {
    match source {
        Source::Tables(t) => acpi::generate_with_tables(kind, t),
        Source::Facts(f) => acpi::generate(kind, f),
        Source::Unknown if self_contained(kind) => acpi::generate(kind, &AcpiFacts::default()),
        Source::Unknown => Err(AppError::new(
            acpi::ERR_INSUFFICIENT,
            "the machine's ACPI tables are not available",
        )
        .recoverable()),
    }
}

// ── Requests ────────────────────────────────────────────────────────────────

/// One SSDT the platform needs, before it is built.
#[derive(Debug, Clone)]
struct Request {
    kind: SsdtKind,
    required: bool,
    reason: String,
    fallback: Fallback,
}

impl Request {
    fn new(kind: SsdtKind, required: bool, reason: impl Into<String>) -> Self {
        Self {
            kind,
            required,
            reason: reason.into(),
            fallback: Fallback::Generic { acpi0007: false },
        }
    }

    fn fallback(mut self, fallback: Fallback) -> Self {
        self.fallback = fallback;
        self
    }
}

/// What to use when the generator cannot build the table.
#[derive(Debug, Clone)]
enum Fallback {
    /// [`acpi::fallback_prebuilt`], or its variant for CPUs declared as
    /// ACPI0007 devices.
    Generic { acpi0007: bool },
    /// This prebuilt table.
    Table(SsdtSource),
    /// No generic table is safe here: leave it out and say why.
    Unavailable(&'static str),
    /// Most machines of this kind need nothing: leave it out silently.
    Skip,
}

#[derive(Default)]
struct Selection {
    requests: Vec<Request>,
    notes: Vec<PlanNote>,
}

impl Selection {
    fn push(&mut self, request: Request) {
        self.requests.push(request);
    }
}

fn select(
    ctx: &PlanContext,
    display: &DisplayPlan,
    plan: &BuildPlan,
    facts: Option<&AcpiFacts>,
    have_tables: bool,
) -> Selection {
    let mut s = Selection::default();
    if ctx.is_vm {
        select_vm(&mut s);
        return s;
    }
    cpu(ctx, facts, &mut s);
    embedded_controller(ctx, plan, &mut s);
    clock(ctx, facts, &mut s);
    nvram(ctx, &mut s);
    uncore(ctx, &mut s);
    usb_reset(ctx, facts, &mut s);
    imei(ctx, display, plan, &mut s);
    laptop_input(ctx, have_tables, &mut s);
    backlight(ctx, display, plan, &mut s);
    s
}

/// VMs get only what macOS cannot boot without: an EC device for Catalina
/// and newer (the generator skips it when the VM already has one). Hyper-V
/// adds OpenCore's samples on top ([`hyper_v`]).
fn select_vm(s: &mut Selection) {
    s.push(Request::new(
        SsdtKind::EcUsbx { laptop: false },
        true,
        "Fake EC device (macOS Catalina and newer look for one) and USBX USB power properties.",
    ));
}

const HID: &str = "5F484944";
const XHID: &str = "58484944";
const STA: &str = "5F535441";
const XSTA: &str = "58535441";

/// A `Base`-scoped DSDT rename: (base, comment, find, replace).
type ScopedRename = (&'static str, &'static str, &'static str, &'static str);

/// OpenCore's Hyper-V samples in the order MacHyperVSupport's README asks
/// for, each with the renames its source lists (OpenCorePkg 1.0.8
/// Docs/AcpiSamples/Source/SSDT-HV-*.dsl): (file, reason, renames).
const HYPER_V_TABLES: &[(&str, &str, &[ScopedRename])] = &[
    (
        "SSDT-HV-VMBUS.aml",
        "Standard _HID values for the Hyper-V VMBus, which AppleACPIPlatform needs for EFI device paths \
         (Startup Disk).",
        &[
            ("\\_SB.VMOD", "_HID to XHID rename (Hyper-V VMOD)", HID, XHID),
            ("\\_SB.VMOD.VMBS", "_HID to XHID rename (Hyper-V VMBus)", HID, XHID),
        ],
    ),
    (
        "SSDT-HV-DEV.aml",
        "Processor objects macOS can use and _STA methods that hide the virtual devices it cannot handle \
         (Windows 10 / Server 2019 and newer hosts).",
        &[
            ("\\_SB.VMOD.TPM2", "_STA to XSTA rename (Hyper-V TPM)", STA, XSTA),
            ("\\_SB.NVDR", "_STA to XSTA rename (Hyper-V NVDIMM)", STA, XSTA),
            ("\\_SB.EPC", "_STA to XSTA rename (Hyper-V EPC)", STA, XSTA),
            ("\\_SB.VMOD.BAT1", "_STA to XSTA rename (Hyper-V battery)", STA, XSTA),
        ],
    ),
    (
        "SSDT-HV-PLUG.aml",
        "Loads VMPlatformPlugin on the first CPU (macOS 11 and newer); it must follow SSDT-HV-DEV.",
        &[],
    ),
];

/// The Hyper-V tables and their renames (MacHyperVSupport README, "OpenCore
/// configuration").
fn hyper_v(plan: &mut BuildPlan) {
    for (file, reason, renames) in HYPER_V_TABLES {
        plan.ssdts.push(SsdtPlan {
            file_name: file.to_string(),
            source: SsdtSource::OcSample {
                file: file.to_string(),
            },
            required: false,
            reason: reason.to_string(),
        });
        for (base, comment, find, replace) in *renames {
            let mut comment = comment.to_string();
            acpi::name_required_table(&mut comment, file);
            add_patch(
                &mut plan.acpi_patches,
                AcpiPatch {
                    comment,
                    find: find.to_string(),
                    replace: replace.to_string(),
                    table_signature: Some("DSDT".to_string()),
                    oem_table_id: None,
                    count: 1,
                    enabled: true,
                    base: base.to_string(),
                    ..AcpiPatch::default()
                },
            );
        }
    }
    plan.notes.push(note(
        NoteLevel::Info,
        "Hyper-V ACPI tables",
        "OpenCore's SSDT-HV-VMBUS, SSDT-HV-DEV and SSDT-HV-PLUG are included with the renames their sources \
         list. On a Windows 8.1 / Server 2012 R2 host, disable SSDT-HV-DEV and the four renames that name it.",
    ));
}

/// Intel 800-series board (Arrow Lake desktops), by chipset or, when it is
/// unknown, by a desktop Arrow Lake CPU.
fn intel_800_series(ctx: &PlanContext) -> bool {
    if !ctx.is_intel() || ctx.is_vm {
        return false;
    }
    match ctx.chipset.as_ref() {
        Some(c) => c.vendor == CpuVendor::Intel && c.series == 800,
        None => ctx.platform() == CpuPlatform::ArrowLake && !ctx.is_laptop,
    }
}

/// OpCore-Simplify's DSDT patch for Intel 800-series boards
/// (lzhoang2801/OpCore-Simplify 52766d2, Scripts/acpi_guru.py
/// `remove_conditional_scope`): the header of `If (PCHA != Zero)` (IfOp, a
/// masked 3-byte PkgLength, LNot LEqual PCHA Zero) becomes NoOps, so the
/// scope it guards is declared unconditionally.
fn remove_conditional_scope() -> AcpiPatch {
    AcpiPatch {
        comment: "Remove conditional ACPI scope declaration (Intel 800-series)".to_string(),
        find: "A000000092935043484100".to_string(),
        replace: "A3A3A3A3A3A3A3A3A3A3A3".to_string(),
        mask: "FF000000FFFFFFFFFFFFFF".to_string(),
        table_signature: Some("DSDT".to_string()),
        oem_table_id: None,
        count: 1,
        enabled: true,
        ..AcpiPatch::default()
    }
}

/// Processor objects: SSDT-PLUG (XCPM plugin-type, Haswell and newer) or
/// SSDT-CPUR on AMD boards that declare the CPUs as ACPI0007 devices.
fn cpu(ctx: &PlanContext, facts: Option<&AcpiFacts>, s: &mut Selection) {
    let platform = ctx.platform();
    if ctx.is_amd() {
        amd_cpu(ctx, facts, s);
        return;
    }
    if !ctx.is_intel() || !intel_xcpm(platform) {
        // Penryn to Ivy Bridge and Nehalem/Sandy/Ivy-E HEDT use
        // AppleIntelCPUPowerManagement: no SSDT-PLUG (ssdt-platform.md).
        return;
    }
    // Alder Lake and newer declare the CPUs as ACPI0007 devices; the
    // generator then emits SSDT-PLUG-ALT (research-intel-desktop §2.3).
    // Facts that list the CPU objects are authoritative.
    let acpi0007 = match facts {
        Some(f) if !f.cpu_paths.is_empty() => f.cpu_uses_acpi0007,
        _ => acpi0007_platform(platform) || facts.is_some_and(|f| f.cpu_uses_acpi0007),
    };
    let reason = if acpi0007 {
        "Processor objects for the CPUs the firmware declares as ACPI0007 devices, with \
         plugin-type = 1 on the first so XCPM native power management loads."
    } else {
        "plugin-type = 1 on the first CPU so XCPM native power management loads \
         (not needed from macOS 12.3 on, harmless there)."
    };
    s.push(Request::new(SsdtKind::Plug, true, reason).fallback(Fallback::Generic { acpi0007 }));
}

fn amd_cpu(ctx: &PlanContext, facts: Option<&AcpiFacts>, s: &mut Selection) {
    let chipset = ctx.chipset.as_ref();
    // Dortania: SSDT-CPUR for B550 and A520 only (CPUs under \_SB.PLTF);
    // never on X570/B450/A320 (research-amd §6).
    let b550 = chipset
        .is_some_and(|c| c.vendor == CpuVendor::Amd && matches!(c.name.as_str(), "B550" | "A520"));
    // AM5 and TRX50/WRX90 boards declare ACPI0007 CPUs under other names;
    // their CPUR flavour is OpenCore's SSDT-PLUG-ALT (\_SB.CP00..CP3F,
    // research-amd §6). The plugin-type it sets on CP00 is harmless on AMD.
    let am5_like = match chipset {
        Some(c) => c.vendor == CpuVendor::Amd && c.series >= 600,
        None => {
            matches!(ctx.platform(), CpuPlatform::AmdZen4 | CpuPlatform::AmdZen5)
                && !ctx.is_laptop
                && !ctx.profile.cpu.is_mobile
        }
    };
    let fallback = if b550 {
        Fallback::Generic { acpi0007: false }
    } else if am5_like {
        Fallback::Table(SsdtSource::OcSample {
            file: "SSDT-PLUG-ALT.aml".to_string(),
        })
    } else if facts.is_some_and(|f| f.cpu_uses_acpi0007) {
        // Neither prebuilt matches this board's processor objects.
        Fallback::Unavailable(
            "The CPUs are ACPI0007 devices and no generic table covers this board; macOS stalls \
             at boot without Processor objects.",
        )
    } else {
        Fallback::Skip
    };
    if facts.is_none() && !(b550 || am5_like) {
        return;
    }
    s.push(
        Request::new(
            SsdtKind::Cpur,
            true,
            "The firmware declares the CPUs as ACPI0007 devices; macOS only finds Processor \
             objects and stalls at boot without them.",
        )
        .fallback(fallback),
    );
}

/// Fake EC + USBX on every machine (ssdt-platform.md). Laptops keep the
/// real EC for battery and keys; desktops have it disabled in macOS.
fn embedded_controller(ctx: &PlanContext, plan: &BuildPlan, s: &mut Selection) {
    let laptop = ctx.is_laptop;
    // Broadwell and older use SSDT-EC without USBX, unless a Skylake-era
    // SMBIOS is used, which needs USBX (research-intel-desktop §3.5). The
    // generated table always carries USBX, which is harmless on old models.
    let usbx =
        ctx.is_amd() || usbx_platform(ctx.platform()) || skylake_era_smbios(&plan.smbios.model);
    let fallback = if usbx {
        Fallback::Generic { acpi0007: false }
    } else {
        Fallback::Table(SsdtSource::Dortania {
            file: if laptop {
                "SSDT-EC-LAPTOP.aml"
            } else {
                "SSDT-EC-DESKTOP.aml"
            }
            .to_string(),
        })
    };
    let reason = if laptop {
        "Keeps the laptop's real EC (battery, keys), adds a fake EC named EC only when none \
         exists, plus the USBX USB power properties."
    } else {
        "Fake EC for macOS with the real desktop EC disabled in macOS (AppleACPIEC is not \
         compatible with desktop ECs), plus the USBX USB power properties."
    };
    s.push(Request::new(SsdtKind::EcUsbx { laptop }, true, reason).fallback(fallback));
}

/// AWAC → legacy RTC on 300-series and newer Intel, RTC0-RANGE on X99/X299
/// (never both: Dortania awac.md / OpenCore SSDT-RTC0-RANGE.dsl).
fn clock(ctx: &PlanContext, facts: Option<&AcpiFacts>, s: &mut Selection) {
    let platform = ctx.platform();
    let target = ctx.target;
    if ctx.is_intel() {
        let rtc_range = match platform {
            // Dortania haswell-e.md: "required for all Big Sur users".
            CpuPlatform::HaswellE | CpuPlatform::BroadwellE => target >= MacOsVersion::BigSur,
            // Dortania skylake-x.md: needed to enable the legacy RTC.
            CpuPlatform::SkylakeX | CpuPlatform::CascadeLakeX => true,
            _ => false,
        };
        if rtc_range {
            s.push(Request::new(
                SsdtKind::Rtc0Range,
                true,
                "New RTC device covering the whole 0x70-0x77 range; many HEDT boards map only \
                 part of it and macOS 11+ halts early at boot.",
            ));
        }
    }
    let facts_awac = facts.is_some_and(|f| f.awac_path.is_some());
    let listed = ctx.is_intel() && awac_listed(ctx, facts.is_some(), s);
    if listed || facts_awac {
        s.push(Request::new(
            SsdtKind::Awac,
            true,
            "macOS needs the legacy RTC (PNP0B00); the AWAC clock (ACPI000E) is switched off \
             or a fake RTC0 is added.",
        ));
    }
}

/// Dortania lists SSDT-AWAC for 300-series and newer (awac.md, coffee-lake.md,
/// comet-lake.md, the laptop table and the Rocket/Alder Lake community pages).
fn awac_listed(ctx: &PlanContext, have_facts: bool, s: &mut Selection) -> bool {
    use CpuPlatform as P;
    match ctx.platform() {
        P::CoffeeLake if !mobile(ctx) => match ctx.chipset.as_ref() {
            // "required for most B360, B365, H310, H370, Z390 and some Z370".
            Some(c) if c.name == "Z370" => {
                if !have_facts {
                    s.notes.push(note(
                        NoteLevel::Info,
                        "SSDT-AWAC left out on Z370",
                        "Most Z370 boards still have the legacy RTC. Some with newer firmware \
                         switched to the AWAC clock; if macOS stalls early at boot, dump the ACPI \
                         tables on this machine and rebuild.",
                    ));
                }
                false
            }
            _ => true,
        },
        P::CoffeeLake
        | P::CometLake
        | P::IceLake
        | P::RocketLake
        | P::TigerLake
        | P::AlderLake
        | P::RaptorLake
        | P::ArrowLake => true,
        _ => false,
    }
}

/// SSDT-PMC: NVRAM on true 300-series desktop boards (B360/B365/H310/H370/
/// Z390, not Z370) and on 9th-gen Coffee Lake-H laptops (Dortania nvram.md,
/// laptop ssdt-platform table).
fn nvram(ctx: &PlanContext, s: &mut Selection) {
    if !ctx.is_intel() || ctx.platform() != CpuPlatform::CoffeeLake {
        return;
    }
    let reason = "PMCR device mapping the PMC region so native NVRAM works on 300-series chipsets.";
    if mobile(ctx) {
        let pch_ok = ctx
            .chipset
            .as_ref()
            .is_none_or(|c| c.series == 300 && c.is_mobile && c.name != "Cannon Point-LP");
        if ninth_gen_coffee_lake(&ctx.profile.cpu.name) && pch_ok {
            s.push(Request::new(SsdtKind::Pmc, true, reason));
        }
        return;
    }
    match ctx.chipset.as_ref() {
        Some(c) if c.needs_pmc() => s.push(Request::new(SsdtKind::Pmc, true, reason)),
        Some(_) => {}
        None => s.notes.push(note(
            NoteLevel::Warning,
            "Chipset unknown: SSDT-PMC not added",
            "B360, B365, H310, H370 and Z390 boards need SSDT-PMC for NVRAM; Z370 does not. \
             Set the chipset in the hardware editor to get it.",
        )),
    }
}

/// SSDT-UNC: X79/X99 uncore bridges of absent sockets crash IOPCIFamily
/// from macOS 11 on (OpenCore SSDT-UNC.dsl, ssdt-platform.md HEDT table).
fn uncore(ctx: &PlanContext, s: &mut Selection) {
    let hedt_unc = matches!(
        ctx.platform(),
        CpuPlatform::SandyBridgeE
            | CpuPlatform::IvyBridgeE
            | CpuPlatform::HaswellE
            | CpuPlatform::BroadwellE
    );
    if ctx.is_intel() && hedt_unc && ctx.target >= MacOsVersion::BigSur {
        s.push(Request::new(
            SsdtKind::Unc,
            true,
            "Hides the uncore PCI bridges of empty CPU sockets; IOPCIFamily panics on them \
             from macOS 11 on.",
        ));
    }
}

/// SSDT-RHUB: Ice Lake laptops ("root device" errors) and ASUS 400-series
/// and newer desktops (Dortania rhub.md, comet-lake.md).
fn usb_reset(ctx: &PlanContext, facts: Option<&AcpiFacts>, s: &mut Selection) {
    use CpuPlatform as P;
    if !ctx.is_intel() {
        return;
    }
    let platform = ctx.platform();
    let ice_lake = platform == P::IceLake;
    let asus_400_plus = !ctx.is_laptop
        && matches!(
            platform,
            P::CometLake | P::RocketLake | P::AlderLake | P::RaptorLake | P::ArrowLake
        )
        && is_asus(ctx);
    if !(ice_lake || asus_400_plus) {
        return;
    }
    // Dortania's prebuilt only covers root hubs under \_SB.PCI0, a no-op on
    // PC00 boards (audit-efi-pipeline M8).
    let fallback = if pci0_root(ctx, facts) {
        Fallback::Generic { acpi0007: false }
    } else {
        Fallback::Unavailable(
            "Dortania's prebuilt SSDT-RHUB only covers \\_SB.PCI0 controllers and this board uses \
             another PCI root.",
        )
    };
    let reason = if ice_lake {
        "Disables the USB root hubs in macOS so ports are built from the controller \
         (root device errors on many Ice Lake laptops)."
    } else {
        "Disables the USB root hubs in macOS so ports are built from the controller \
         (needed on ASUS 400-series and newer boards)."
    };
    s.push(Request::new(SsdtKind::RhubReset, true, reason).fallback(fallback));
}

/// SSDT-IMEI: Sandy Bridge CPU on a 7-series board or Ivy Bridge CPU on a
/// 6-series board, "needed" per Dortania sandy-bridge.md / ivy-bridge.md;
/// planned together with the IMEI `device-id` of the graphics stage
/// ([`graphics::imei_device_id`]).
fn imei(ctx: &PlanContext, display: &DisplayPlan, plan: &BuildPlan, s: &mut Selection) {
    if graphics::imei_device_id(ctx, display, plan).is_none() {
        return;
    }
    s.push(Request::new(
        SsdtKind::Imei,
        true,
        "IMEI device at 0x00160000 so the Intel ME gets the device-id the iGPU driver expects \
         (CPU and chipset generations differ).",
    ));
}

/// XOSI / GPI0 for laptops (Dortania laptop pages and trackpad.md; AMD:
/// research-amd §13).
fn laptop_input(ctx: &PlanContext, have_tables: bool, s: &mut Selection) {
    use CpuPlatform as P;
    if !ctx.is_laptop {
        return;
    }
    let xosi = Request::new(
        SsdtKind::Xosi,
        false,
        "With the _OSI to XOSI rename, macOS answers the firmware's Windows checks so devices \
         gated on Windows (I2C trackpads, keys) are enabled.",
    );
    let i2c = ctx.profile.input.touchpad_bus == Some(InputBus::I2c);
    if ctx.is_amd() {
        s.push(xosi);
        return;
    }
    if !ctx.is_intel() {
        return;
    }
    match ctx.platform() {
        // Arrandale, Sandy and Ivy Bridge laptop pages always add SSDT-XOSI.
        P::Arrandale | P::SandyBridge | P::IvyBridge => s.push(xosi),
        _ if i2c => {
            // From Haswell on Dortania uses SSDT-GPI0 and keeps XOSI as the
            // alternative: its blind _OSI rename "is common to break Windows
            // booting" (trackpad-methods/prebuilt.md), e.g. on an OSID method.
            // With the dumped tables it is built like SSDTTime's, with those
            // names renamed first and the Windows versions the firmware checks.
            if have_tables {
                s.push(xosi);
            }
            s.push(Request::new(
                SsdtKind::Gpi0,
                false,
                "Enables the GPIO controller in macOS so VoodooI2C can use GPIO interrupts for \
                 the I2C trackpad.",
            ));
            if !have_tables {
                s.notes.push(note(
                    NoteLevel::Info,
                    "SSDT-XOSI left out",
                    "If the I2C trackpad is not detected after installing, its controller is \
                     probably enabled for Windows only. Run the scan on this laptop with its \
                     ACPI tables dumped to get an SSDT-XOSI built for it.",
                ));
            }
        }
        _ => {}
    }
}

/// PNLF (+ ALS0 on Catalina and newer) for an internal panel driven by a
/// supported iGPU ([`panel_backlight`]).
fn backlight(ctx: &PlanContext, display: &DisplayPlan, plan: &BuildPlan, s: &mut Selection) {
    let Some(panel) = panel_backlight(ctx, display, plan) else {
        return;
    };
    let uid = panel.pnlf_uid;
    s.push(Request::new(
        SsdtKind::Pnlf { uid },
        false,
        format!("Backlight device for the internal panel (WhateverGreen profile {uid})."),
    ));
    if panel.als0 {
        s.push(Request::new(
            SsdtKind::Als0,
            false,
            "Ambient light sensor device; from macOS 10.15 on the backlight only works when one \
             exists (a real enabled sensor makes this unnecessary).",
        ));
    }
}

/// `_UID` of the panel's iGPU: the display decision must drive the displays
/// from the iGPU itself (an all-in-one whose panel hangs off a dGPU, with the
/// iGPU headless, gets none), and neither the profile, the display decision
/// nor the graphics stage may have disabled it.
fn panel_pnlf_uid(ctx: &PlanContext, display: &DisplayPlan, plan: &BuildPlan) -> Option<u32> {
    let gpus = &ctx.profile.gpus;
    if let Some(i) = gpus.iter().position(|g| g.is_igpu) {
        let igpu = &gpus[i];
        let drives_panel = display.primary == Some(i) && !display.igpu_headless;
        let usable = drives_panel
            && !igpu.disabled
            && !display.disabled.contains(&i)
            && !igpu_disabled_in_plan(igpu, plan)
            && gpu_db::support(igpu).display_capable;
        return if usable { pnlf_uid(igpu.family) } else { None };
    }
    if !gpus.is_empty() || !ctx.is_intel() {
        // The panel is on a dGPU (e.g. Clarksfield), which needs no PNLF here.
        return None;
    }
    // No GPU listed at all (manual profile): go by the CPU generation.
    use CpuPlatform as P;
    match ctx.platform() {
        P::Arrandale | P::SandyBridge | P::IvyBridge => Some(14),
        P::Haswell | P::Broadwell => Some(15),
        P::Skylake | P::KabyLake => Some(16),
        P::CoffeeLake | P::CometLake | P::IceLake => Some(19),
        _ => None,
    }
}

/// The graphics stage turned the iGPU off (`-wegnoigpu`, `disable-gpu` or a
/// `class-code` of FFFFFFFF).
fn igpu_disabled_in_plan(igpu: &ProfileGpu, plan: &BuildPlan) -> bool {
    if plan.boot_args.iter().any(|a| a == "-wegnoigpu") {
        return true;
    }
    let path = igpu
        .pci_path
        .as_deref()
        .unwrap_or("PciRoot(0x0)/Pci(0x2,0x0)");
    plan.device_properties
        .iter()
        .filter(|e| e.path.eq_ignore_ascii_case(path))
        .flat_map(|e| e.properties.iter())
        .any(|p| match (p.key.as_str(), &p.value) {
            ("disable-gpu", _) => true,
            ("class-code", PlistScalar::Data(hex)) => hex.eq_ignore_ascii_case("FFFFFFFF"),
            _ => false,
        })
}

/// ACPI > Delete: the OEM CpuPm / Cpu0Ist tables on Sandy and Ivy Bridge,
/// "temporary until we've made our SSDT-PM" (Dortania sandy-bridge.md).
fn deletes(ctx: &PlanContext) -> Vec<AcpiDelete> {
    if !ctx.is_intel()
        || ctx.is_vm
        || !matches!(
            ctx.platform(),
            CpuPlatform::SandyBridge | CpuPlatform::IvyBridge
        )
    {
        return Vec::new();
    }
    vec![
        AcpiDelete {
            comment: "Delete CpuPm".to_string(),
            table_signature: "SSDT".to_string(),
            oem_table_id: "CpuPm".to_string(),
            all: true,
        },
        AcpiDelete {
            comment: "Delete Cpu0Ist".to_string(),
            table_signature: "SSDT".to_string(),
            oem_table_id: "Cpu0Ist".to_string(),
            all: true,
        },
    ]
}

/// Post-install steps for decisions this stage leaves to the user.
fn post_install(ctx: &PlanContext) -> Vec<PlanNote> {
    use CpuPlatform as P;
    let mut out = Vec::new();
    if !ctx.is_intel() || ctx.is_vm {
        return out;
    }
    let platform = ctx.platform();
    let ssdt_pm = |detail: &str| PlanNote {
        level: NoteLevel::Info,
        component: "cpu".to_string(),
        title: "Create SSDT-PM for CPU power management".to_string(),
        detail: detail.to_string(),
    };
    if matches!(platform, P::SandyBridge | P::IvyBridge) {
        out.push(ssdt_pm(
            "After installing, generate SSDT-PM with ssdtPRGen.sh, add it to EFI/OC/ACPI and \
             config.plist, then disable the \"Delete CpuPm\" and \"Delete Cpu0Ist\" ACPI > \
             Delete entries (Dortania post-install pm.md).",
        ));
    }
    // pm.md: Ivy Bridge-E has no XCPM past 10.11 and still needs ssdtPRGen;
    // the X79 config pages drop no tables.
    if matches!(platform, P::SandyBridgeE | P::IvyBridgeE) {
        out.push(ssdt_pm(
            "After installing, generate SSDT-PM with ssdtPRGen.sh and add it to EFI/OC/ACPI and \
             config.plist (Dortania post-install pm.md).",
        ));
    }
    if ctx.is_laptop
        && matches!(
            platform,
            P::Arrandale | P::SandyBridge | P::IvyBridge | P::Haswell | P::Broadwell
        )
    {
        out.push(note(
            NoteLevel::Info,
            "IRQ conflicts (no audio)",
            "Laptops of this generation often need the HPET IRQ fix: if audio or other devices \
             do not work, generate SSDT-HPET with SSDTTime's FixHPET option on this machine and \
             add it with its ACPI patches.",
        ));
    }
    out
}

// ── Building ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Built {
    ssdts: Vec<SsdtPlan>,
    patches: Vec<AcpiPatch>,
    planned: Vec<SsdtKind>,
    /// File names of generic prebuilt tables used.
    prebuilt: Vec<String>,
    notes: Vec<PlanNote>,
}

impl Built {
    fn build(&mut self, request: Request, source: &Source) {
        if self.conflicts(&request.kind) {
            tracing::debug!(
                ssdt = request.kind.file_name(),
                "skipped: conflicts with a planned SSDT"
            );
            return;
        }
        match generate(&request.kind, source) {
            Ok(g) => self.push_generated(request, g),
            Err(e) if acpi::is_not_needed(&e) => {
                tracing::debug!(ssdt = request.kind.file_name(), reason = %e.message, "not needed");
            }
            Err(e) => self.push_fallback(request, &e, source),
        }
    }

    /// SSDT-AWAC and SSDT-RTC0-RANGE both redefine the RTC `_STA`.
    fn conflicts(&self, kind: &SsdtKind) -> bool {
        let planned = |k: &SsdtKind| self.planned.contains(k);
        match kind {
            SsdtKind::Awac => planned(&SsdtKind::Rtc0Range),
            SsdtKind::Rtc0Range => planned(&SsdtKind::Awac),
            other => planned(other),
        }
    }

    fn push_generated(&mut self, request: Request, g: GeneratedSsdt) {
        for mut patch in g.patches {
            acpi::name_required_table(&mut patch.comment, &g.file_name);
            add_patch(&mut self.patches, patch);
        }
        self.ssdts.push(SsdtPlan {
            file_name: g.file_name,
            source: SsdtSource::Generated {
                aml_hex: hex_upper(&g.aml),
                dsl: g.dsl,
            },
            required: request.required,
            reason: request.reason,
        });
        self.planned.push(request.kind);
    }

    fn push_fallback(&mut self, request: Request, err: &AppError, source: &Source) {
        let table = match &request.fallback {
            Fallback::Generic { acpi0007: true } => acpi::fallback_prebuilt_acpi0007(&request.kind),
            Fallback::Generic { acpi0007: false } => acpi::fallback_prebuilt(&request.kind),
            Fallback::Table(t) => Some(t.clone()),
            Fallback::Unavailable(_) | Fallback::Skip => None,
        };
        let name = request.kind.file_name();
        let Some(table) = table else {
            match request.fallback {
                Fallback::Skip => {
                    tracing::debug!(ssdt = name, reason = %err.message, "left out");
                }
                Fallback::Unavailable(why) => self.notes.push(missing_note(name, why, err, source)),
                _ => self.notes.push(missing_note(
                    name,
                    "There is no generic prebuilt version of this table.",
                    err,
                    source,
                )),
            }
            return;
        };
        let file = match &table {
            SsdtSource::OcSample { file } | SsdtSource::Dortania { file } => file.clone(),
            SsdtSource::Generated { .. } => name.to_string(),
        };
        self.ssdts.push(SsdtPlan {
            file_name: file.clone(),
            source: table,
            required: request.required,
            reason: format!("{} Generic prebuilt table.", request.reason),
        });
        for mut patch in acpi::fallback_patches(&request.kind) {
            acpi::name_required_table(&mut patch.comment, &file);
            add_patch(&mut self.patches, patch);
        }
        self.prebuilt.push(file);
        self.planned.push(request.kind);
    }
}

fn missing_note(name: &str, why: &str, err: &AppError, source: &Source) -> PlanNote {
    let cause = if matches!(source, Source::Unknown) {
        "The machine's ACPI tables are not available.".to_string()
    } else {
        format!("{}.", err.message.trim_end_matches('.'))
    };
    note(
        NoteLevel::Warning,
        &format!("{name} not added"),
        format!(
            "{cause} {why} Run the scan on the target machine with its ACPI tables dumped, or add \
             the table by hand (SSDTTime)."
        ),
    )
}

/// Append unless the same rename (find, replace and scope) is already listed;
/// order is kept because table-specific renames depend on the ones before.
/// A rename one table needs stays enabled even if another listed it as
/// optional (disabled), and its comment names every table that needs it
/// (the build disables it only when all of them are missing).
fn add_patch(list: &mut Vec<AcpiPatch>, patch: AcpiPatch) {
    let duplicate = list.iter_mut().find(|p| {
        p.find.eq_ignore_ascii_case(&patch.find)
            && p.replace.eq_ignore_ascii_case(&patch.replace)
            && p.mask.eq_ignore_ascii_case(&patch.mask)
            && p.base == patch.base
            && p.table_signature == patch.table_signature
            && p.oem_table_id == patch.oem_table_id
            && p.count == patch.count
    });
    match duplicate {
        Some(_) if !patch.enabled => {}
        Some(existing) if !existing.enabled => *existing = patch,
        Some(existing) => {
            for table in acpi::tables_named_in(&patch.comment) {
                acpi::name_required_table(&mut existing.comment, table);
            }
        }
        None => list.push(patch),
    }
}

// ── Platform facts ──────────────────────────────────────────────────────────

fn note(level: NoteLevel, title: &str, detail: impl Into<String>) -> PlanNote {
    PlanNote {
        level,
        component: COMPONENT.to_string(),
        title: title.to_string(),
        detail: detail.into(),
    }
}

/// Intel platforms with XCPM (Haswell and newer, consumer and HEDT).
fn intel_xcpm(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::Haswell
            | P::Broadwell
            | P::Skylake
            | P::KabyLake
            | P::CoffeeLake
            | P::CometLake
            | P::IceLake
            | P::RocketLake
            | P::TigerLake
            | P::AlderLake
            | P::RaptorLake
            | P::ArrowLake
            | P::HaswellE
            | P::BroadwellE
            | P::SkylakeX
            | P::CascadeLakeX
    )
}

/// Platforms whose firmware declares the CPUs as ACPI0007 devices.
fn acpi0007_platform(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::AlderLake | CpuPlatform::RaptorLake | CpuPlatform::ArrowLake
    )
}

/// Dortania uses SSDT-EC-USBX from Skylake on (and on X99/X299 HEDT).
fn usbx_platform(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::Skylake
            | P::KabyLake
            | P::CoffeeLake
            | P::CometLake
            | P::IceLake
            | P::RocketLake
            | P::TigerLake
            | P::AlderLake
            | P::RaptorLake
            | P::ArrowLake
            | P::HaswellE
            | P::BroadwellE
            | P::SkylakeX
            | P::CascadeLakeX
    )
}

/// Skylake-era and newer Mac models read USB power from a USBX device
/// ("Skylake+ SMBIOS will also require a USBX device", smbios-support.md).
fn skylake_era_smbios(model: &str) -> bool {
    let Some(split) = model.find(|c: char| c.is_ascii_digit()) else {
        return false;
    };
    let (family, numbers) = model.split_at(split);
    let major: u32 = numbers
        .split(',')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    match family {
        "iMac" => major >= 17,
        "iMacPro" => true,
        "MacPro" => major >= 7,
        "MacBookPro" => major >= 13,
        "MacBookAir" => major >= 8,
        "MacBook" => major >= 9,
        "Macmini" => major >= 8,
        _ => false,
    }
}

/// Mobile CPU or mobile chipset (laptops, NUC-style mini PCs).
fn mobile(ctx: &PlanContext) -> bool {
    ctx.is_laptop || ctx.profile.cpu.is_mobile || ctx.chipset.as_ref().is_some_and(|c| c.is_mobile)
}

/// 9th-gen Coffee Lake-H ("i7-9750H", "i9-9980HK", Xeon "E-2276M") as
/// opposed to 8th gen ("i7-8750H", "E-2176M").
fn ninth_gen_coffee_lake(name: &str) -> bool {
    name.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .any(|token| {
            let t = token.to_ascii_uppercase();
            let core = ["I3-", "I5-", "I7-", "I9-"]
                .iter()
                .find_map(|p| t.strip_prefix(p))
                .is_some_and(|rest| {
                    rest.len() >= 4
                        && rest.starts_with('9')
                        && rest.chars().take(4).all(|c| c.is_ascii_digit())
                });
            let xeon = t.starts_with("E-22") && t.ends_with('M');
            core || xeon
        })
}

fn is_asus(ctx: &PlanContext) -> bool {
    let vendor = ctx.profile.motherboard_vendor.to_ascii_lowercase();
    if vendor.contains("asus") {
        return true;
    }
    let model = ctx.profile.motherboard_model.to_ascii_uppercase();
    let first = model.split_whitespace().next().unwrap_or_default();
    matches!(first, "ROG" | "TUF" | "PRIME" | "PROART") || first.starts_with("ROG-")
}

/// The main PCI root is `\_SB.PCI0` (known from the facts, else guessed:
/// 500-series boards and newer use PC00, so do Rocket Lake, Tiger Lake,
/// Alder Lake and newer CPUs and X299 when the chipset is unknown; a Rocket
/// Lake CPU on a 400-series board keeps that board's PCI0).
fn pci0_root(ctx: &PlanContext, facts: Option<&AcpiFacts>) -> bool {
    use CpuPlatform as P;
    if let Some(root) = facts.and_then(|f| f.pci_root.as_deref()) {
        return root.trim_end_matches('_').ends_with("PCI0");
    }
    if let Some(c) = ctx.chipset.as_ref() {
        if c.vendor == CpuVendor::Intel && !c.is_hedt && c.series >= 100 {
            return c.series < 500;
        }
    }
    !matches!(
        ctx.platform(),
        P::RocketLake
            | P::TigerLake
            | P::AlderLake
            | P::RaptorLake
            | P::ArrowLake
            | P::SkylakeX
            | P::CascadeLakeX
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::acpi::aml::{self, encode_body, Resource, Term};
    use crate::domain::cpu_db;
    use crate::domain::kext_catalog;
    use crate::domain::model::{
        BuildOptions, DeviceProperty, DevicePropertyEntry, FormFactor, GpuVendor, HardwareProfile,
        ProfileCpu,
    };
    use crate::domain::planner::empty_plan;

    /// OpenCore 1.0.8 `Docs/AcpiSamples/Binaries`.
    const OC_SAMPLES: &[&str] = &[
        "SSDT-ALS0.aml",
        "SSDT-AWAC-DISABLE.aml",
        "SSDT-BRG0.aml",
        "SSDT-EC-USBX.aml",
        "SSDT-EC.aml",
        "SSDT-EHCx-DISABLE.aml",
        "SSDT-HV-DEV.aml",
        "SSDT-HV-PLUG.aml",
        "SSDT-HV-VMBUS.aml",
        "SSDT-IMEI.aml",
        "SSDT-PLUG-ALT.aml",
        "SSDT-PLUG.aml",
        "SSDT-PMC.aml",
        "SSDT-PNLF.aml",
        "SSDT-RTC0-RANGE.aml",
        "SSDT-RTC0.aml",
        "SSDT-SBUS-MCHC.aml",
        "SSDT-UNC.aml",
    ];

    fn machine(platform: CpuPlatform, form_factor: FormFactor) -> HardwareProfile {
        HardwareProfile {
            cpu: ProfileCpu {
                vendor: cpu_db::platform_info(platform).vendor,
                platform,
                is_mobile: form_factor == FormFactor::Laptop,
                ..Default::default()
            },
            form_factor,
            ..Default::default()
        }
    }

    fn with_chipset(mut profile: HardwareProfile, chipset: &str) -> HardwareProfile {
        profile.chipset = Some(chipset.to_string());
        profile
    }

    fn plan_for(profile: &HardwareProfile, target: MacOsVersion, smbios: &str) -> BuildPlan {
        let options = BuildOptions {
            target,
            ..BuildOptions::default()
        };
        let ctx = PlanContext::new(profile, &options);
        let mut plan = empty_plan(target);
        plan.smbios.model = smbios.to_string();
        apply(&ctx, &display_for(&ctx), &mut plan);
        plan
    }

    /// The graphics stage's display decision, or none for profiles without
    /// a GPU (manual profiles).
    fn display_for(ctx: &PlanContext) -> DisplayPlan {
        graphics::choose_display(ctx).unwrap_or(DisplayPlan {
            primary: None,
            igpu: None,
            igpu_headless: false,
            disabled: vec![],
        })
    }

    fn names(plan: &BuildPlan) -> Vec<&str> {
        plan.ssdts.iter().map(|s| s.file_name.as_str()).collect()
    }

    fn ssdt<'a>(plan: &'a BuildPlan, name: &str) -> &'a SsdtPlan {
        plan.ssdts
            .iter()
            .find(|s| s.file_name == name)
            .unwrap_or_else(|| panic!("{name} missing from {:?}", names(plan)))
    }

    /// DSL of a generated SSDT (panics when it is a prebuilt one).
    fn dsl<'a>(plan: &'a BuildPlan, name: &str) -> &'a str {
        match &ssdt(plan, name).source {
            SsdtSource::Generated { dsl, aml_hex } => {
                assert!(
                    aml_hex.starts_with("53534454"),
                    "{name} is not an SSDT image"
                );
                dsl
            }
            other => panic!("{name} should be generated, got {other:?}"),
        }
    }

    fn dortania(plan: &BuildPlan, name: &str) {
        match &ssdt(plan, name).source {
            SsdtSource::Dortania { file } => assert_eq!(file, name),
            other => panic!("{name} should be a Dortania prebuilt, got {other:?}"),
        }
    }

    fn oc_sample(plan: &BuildPlan, name: &str) {
        match &ssdt(plan, name).source {
            SsdtSource::OcSample { file } => assert_eq!(file, name),
            other => panic!("{name} should be an OpenCore sample, got {other:?}"),
        }
    }

    fn has_note(plan: &BuildPlan, title_part: &str) -> bool {
        plan.notes.iter().any(|n| n.title.contains(title_part))
    }

    fn patch_comments(plan: &BuildPlan) -> Vec<&str> {
        plan.acpi_patches
            .iter()
            .map(|p| p.comment.as_str())
            .collect()
    }

    /// Intel PCH facts: `\_SB.<root>.<lpc>` with a real EC0 (no `_STA`),
    /// Processor objects in `\_PR`, AWAC switched by STAS, a legacy RTC and
    /// one root hub.
    fn intel_facts(root: &str, lpc: &str) -> AcpiFacts {
        let lpc_path = format!("\\_SB.{root}.{lpc}");
        AcpiFacts {
            pci_root: Some(format!("\\_SB.{root}")),
            lpc_bridge: Some(lpc_path.clone()),
            ec_path: Some(format!("{lpc_path}.EC0")),
            cpu_paths: (0..4).map(|i| format!("\\_PR.CPU{i}")).collect(),
            awac_path: Some(format!("{lpc_path}.AWAC")),
            awac_has_stas: true,
            rtc_path: Some(format!("{lpc_path}.RTC")),
            igpu_path: Some(format!("\\_SB.{root}.GFX0")),
            xhci_paths: vec![format!("\\_SB.{root}.XHC")],
            rhub_paths: vec![format!("\\_SB.{root}.XHC.RHUB")],
            dsdt_oem_table_id: Some("TESTDSDT".to_string()),
            ..Default::default()
        }
    }

    /// AMD AM4 facts: SBRG without an EC, CPUs as ACPI0007 devices under
    /// `\_SB.PLTF` (B550/A520 firmware) or Processor objects (older boards).
    fn amd_facts(acpi0007: bool) -> AcpiFacts {
        AcpiFacts {
            pci_root: Some("\\_SB.PCI0".to_string()),
            lpc_bridge: Some("\\_SB.PCI0.SBRG".to_string()),
            cpu_paths: (0..16)
                .map(|i| {
                    if acpi0007 {
                        format!("\\_SB.PLTF.C0{i:02X}")
                    } else {
                        format!("\\_PR.C0{i:02X}")
                    }
                })
                .collect(),
            cpu_uses_acpi0007: acpi0007,
            rtc_path: Some("\\_SB.PCI0.SBRG.RTC".to_string()),
            ..Default::default()
        }
    }

    fn igpu(family: GpuFamily, device_id: &str) -> ProfileGpu {
        ProfileGpu {
            name: "Intel iGPU".to_string(),
            vendor: GpuVendor::Intel,
            family,
            vendor_id: Some("8086".to_string()),
            device_id: Some(device_id.to_string()),
            is_igpu: true,
            pci_path: Some("PciRoot(0x0)/Pci(0x2,0x0)".to_string()),
            ..Default::default()
        }
    }

    // ── Desktop Intel ───────────────────────────────────────────────────────

    #[test]
    fn coffee_lake_z390_from_facts() {
        let mut p = with_chipset(
            machine(CpuPlatform::CoffeeLake, FormFactor::Desktop),
            "Z390",
        );
        p.cpu.name = "Intel(R) Core(TM) i9-9900K CPU @ 3.60GHz".to_string();
        p.acpi = Some(intel_facts("PCI0", "LPCB"));
        let plan = plan_for(&p, MacOsVersion::Sequoia, "iMac19,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX.aml",
                "SSDT-AWAC.aml",
                "SSDT-PMC.aml"
            ]
        );
        assert!(dsl(&plan, "SSDT-PLUG.aml").contains("plugin-type"));
        assert!(dsl(&plan, "SSDT-EC-USBX.aml").contains("\\_SB.PCI0.LPCB"));
        assert!(dsl(&plan, "SSDT-AWAC.aml").contains("STAS"));
        assert!(dsl(&plan, "SSDT-PMC.aml").contains("\\_SB.PCI0.LPCB"));
        // Facts alone never rename: existing objects are guarded instead.
        assert!(plan.acpi_patches.is_empty());
        assert!(plan.acpi_deletes.is_empty());
        assert!(plan.ssdts.iter().all(|s| s.required));
        assert!(!has_note(&plan, "Generic prebuilt"));
    }

    #[test]
    fn coffee_lake_without_tables_uses_prebuilt_tables() {
        let p = with_chipset(
            machine(CpuPlatform::CoffeeLake, FormFactor::Desktop),
            "B360",
        );
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac19,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-DESKTOP.aml",
                "SSDT-AWAC.aml",
                "SSDT-PMC.aml"
            ]
        );
        oc_sample(&plan, "SSDT-PLUG.aml");
        dortania(&plan, "SSDT-EC-USBX-DESKTOP.aml");
        dortania(&plan, "SSDT-AWAC.aml");
        dortania(&plan, "SSDT-PMC.aml");
        assert!(has_note(&plan, "Generic prebuilt"));
    }

    #[test]
    fn z370_has_no_pmc_and_no_awac_without_facts() {
        let p = with_chipset(
            machine(CpuPlatform::CoffeeLake, FormFactor::Desktop),
            "Z370",
        );
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac19,1");
        assert_eq!(names(&plan), ["SSDT-PLUG.aml", "SSDT-EC-USBX-DESKTOP.aml"]);
        assert!(has_note(&plan, "Z370"));

        // A Z370 whose firmware exposes the AWAC clock gets the fix.
        let mut p = with_chipset(
            machine(CpuPlatform::CoffeeLake, FormFactor::Desktop),
            "Z370",
        );
        p.acpi = Some(intel_facts("PCI0", "LPCB"));
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac19,1");
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG.aml", "SSDT-EC-USBX.aml", "SSDT-AWAC.aml"]
        );
    }

    #[test]
    fn coffee_lake_without_chipset_warns_about_pmc() {
        let p = machine(CpuPlatform::CoffeeLake, FormFactor::Desktop);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac19,1");
        assert!(names(&plan).contains(&"SSDT-AWAC.aml"));
        assert!(!names(&plan).contains(&"SSDT-PMC.aml"));
        assert!(has_note(&plan, "SSDT-PMC"));
    }

    #[test]
    fn awac_generator_skips_boards_with_a_legacy_rtc() {
        let mut facts = intel_facts("PCI0", "LPCB");
        facts.awac_path = None;
        facts.awac_has_stas = false;
        let mut p = with_chipset(machine(CpuPlatform::CometLake, FormFactor::Desktop), "Z490");
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Tahoe, "iMac20,1");
        assert_eq!(names(&plan), ["SSDT-PLUG.aml", "SSDT-EC-USBX.aml"]);
    }

    #[test]
    fn comet_lake_rhub_only_on_asus() {
        let mut p = with_chipset(machine(CpuPlatform::CometLake, FormFactor::Desktop), "Z490");
        p.motherboard_vendor = "ASUSTeK COMPUTER INC.".to_string();
        p.motherboard_model = "ROG STRIX Z490-E GAMING".to_string();
        let plan = plan_for(&p, MacOsVersion::Tahoe, "iMac20,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-DESKTOP.aml",
                "SSDT-AWAC.aml",
                "SSDT-RHUB.aml"
            ]
        );
        dortania(&plan, "SSDT-RHUB.aml");
        assert!(!names(&plan).contains(&"SSDT-PMC.aml"));

        let mut p = with_chipset(machine(CpuPlatform::CometLake, FormFactor::Desktop), "Z490");
        p.motherboard_vendor = "Gigabyte Technology Co., Ltd.".to_string();
        p.motherboard_model = "Z490 AORUS ELITE".to_string();
        let plan = plan_for(&p, MacOsVersion::Tahoe, "iMac20,1");
        assert!(!names(&plan).contains(&"SSDT-RHUB.aml"));
    }

    #[test]
    fn rocket_lake_rhub_prebuilt_follows_the_board_generation() {
        // A Rocket Lake CPU on an ASUS Z490 keeps the board's \_SB.PCI0.
        let mut p = with_chipset(
            machine(CpuPlatform::RocketLake, FormFactor::Desktop),
            "Z490",
        );
        p.motherboard_model = "ROG STRIX Z490-E GAMING".to_string();
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac20,1");
        dortania(&plan, "SSDT-RHUB.aml");

        // Z590 boards use PC00, which Dortania's table does not cover.
        let mut p = with_chipset(
            machine(CpuPlatform::RocketLake, FormFactor::Desktop),
            "Z590",
        );
        p.motherboard_model = "PRIME Z590-A".to_string();
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac20,1");
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG.aml", "SSDT-EC-USBX-DESKTOP.aml", "SSDT-AWAC.aml"]
        );
        assert!(has_note(&plan, "SSDT-RHUB.aml not added"));
    }

    #[test]
    fn alder_lake_pc00_from_facts() {
        let mut facts = intel_facts("PC00", "LPCB");
        facts.ec_path = None;
        facts.cpu_paths = (0..4).map(|i| format!("\\_SB.PR0{i}")).collect();
        facts.cpu_uses_acpi0007 = true;
        let mut p = with_chipset(machine(CpuPlatform::AlderLake, FormFactor::Desktop), "Z690");
        p.motherboard_vendor = "ASUSTeK COMPUTER INC.".to_string();
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Sequoia, "MacPro7,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG-ALT.aml",
                "SSDT-EC-USBX.aml",
                "SSDT-AWAC.aml",
                "SSDT-RHUB.aml"
            ]
        );
        let plug = dsl(&plan, "SSDT-PLUG-ALT.aml");
        assert!(plug.contains("ACPI0007") && plug.contains("plugin-type"));
        assert!(dsl(&plan, "SSDT-EC-USBX.aml").contains("\\_SB.PC00.LPCB"));
        assert!(dsl(&plan, "SSDT-RHUB.aml").contains("\\_SB.PC00.XHC.RHUB"));
    }

    #[test]
    fn plug_follows_the_processor_objects_in_the_facts() {
        // An Alder Lake board whose firmware still uses Processor objects.
        let mut facts = intel_facts("PC00", "LPCB");
        facts.cpu_paths = (0..4).map(|i| format!("\\_SB.PR0{i}")).collect();
        let mut p = with_chipset(machine(CpuPlatform::AlderLake, FormFactor::Desktop), "Z690");
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "MacPro7,1");
        assert_eq!(names(&plan)[0], "SSDT-PLUG.aml");
        assert!(dsl(&plan, "SSDT-PLUG.aml").contains("\\_SB.PR00"));
        assert!(!ssdt(&plan, "SSDT-PLUG.aml").reason.contains("ACPI0007"));

        // Facts without CPU objects fall back by generation.
        let mut facts = intel_facts("PC00", "LPCB");
        facts.cpu_paths.clear();
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "MacPro7,1");
        oc_sample(&plan, "SSDT-PLUG-ALT.aml");
    }

    #[test]
    fn alder_lake_without_tables() {
        let mut p = with_chipset(machine(CpuPlatform::AlderLake, FormFactor::Desktop), "Z690");
        p.motherboard_vendor = "ASUSTeK COMPUTER INC.".to_string();
        let plan = plan_for(&p, MacOsVersion::Sequoia, "MacPro7,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG-ALT.aml",
                "SSDT-EC-USBX-DESKTOP.aml",
                "SSDT-AWAC.aml"
            ]
        );
        oc_sample(&plan, "SSDT-PLUG-ALT.aml");
        // Dortania's SSDT-RHUB only knows PCI0 paths: left out with a warning.
        assert!(has_note(&plan, "SSDT-RHUB.aml not added"));
        let warning = plan
            .notes
            .iter()
            .find(|n| n.title.contains("SSDT-RHUB"))
            .map(|n| n.level);
        assert_eq!(warning, Some(NoteLevel::Warning));
    }

    #[test]
    fn haswell_ec_follows_the_smbios_generation() {
        let p = with_chipset(machine(CpuPlatform::Haswell, FormFactor::Desktop), "Z97");
        let plan = plan_for(&p, MacOsVersion::BigSur, "iMac15,1");
        assert_eq!(names(&plan), ["SSDT-PLUG.aml", "SSDT-EC-DESKTOP.aml"]);
        dortania(&plan, "SSDT-EC-DESKTOP.aml");

        let plan = plan_for(&p, MacOsVersion::Tahoe, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-PLUG.aml", "SSDT-EC-USBX-DESKTOP.aml"]);
    }

    #[test]
    fn penryn_and_lynnfield_get_only_an_ec() {
        for platform in [
            CpuPlatform::Penryn,
            CpuPlatform::Lynnfield,
            CpuPlatform::NehalemHedt,
        ] {
            let p = machine(platform, FormFactor::Desktop);
            let plan = plan_for(&p, MacOsVersion::HighSierra, "iMac10,1");
            assert_eq!(names(&plan), ["SSDT-EC-DESKTOP.aml"], "{platform:?}");
            assert!(plan.acpi_deletes.is_empty());
        }
    }

    #[test]
    fn sandy_bridge_on_7_series_needs_imei_and_cpupm_drops() {
        let mut p = with_chipset(
            machine(CpuPlatform::SandyBridge, FormFactor::Desktop),
            "Z77",
        );
        p.gpus = vec![igpu(GpuFamily::IntelSandyBridge, "0122")];
        let plan = plan_for(&p, MacOsVersion::HighSierra, "iMac12,2");
        assert_eq!(names(&plan), ["SSDT-EC-DESKTOP.aml", "SSDT-IMEI.aml"]);
        dortania(&plan, "SSDT-IMEI.aml");
        assert!(ssdt(&plan, "SSDT-IMEI.aml").required);
        let deletes: Vec<(&str, &str, bool)> = plan
            .acpi_deletes
            .iter()
            .map(|d| (d.table_signature.as_str(), d.oem_table_id.as_str(), d.all))
            .collect();
        assert_eq!(
            deletes,
            [("SSDT", "CpuPm", true), ("SSDT", "Cpu0Ist", true)]
        );
        assert!(plan
            .post_install
            .iter()
            .any(|n| n.title.contains("SSDT-PM")));

        let options = BuildOptions {
            target: MacOsVersion::HighSierra,
            ..BuildOptions::default()
        };
        let ctx = PlanContext::new(&p, &options);
        let display = display_for(&ctx);
        assert_eq!(
            graphics::imei_device_id(&ctx, &display, &plan),
            Some([0x3A, 0x1C, 0x00, 0x00])
        );
    }

    #[test]
    fn imei_only_for_mismatched_generations() {
        let options = BuildOptions {
            target: MacOsVersion::Catalina,
            ..BuildOptions::default()
        };
        let imei = |p: &HardwareProfile| {
            let ctx = PlanContext::new(p, &options);
            graphics::imei_device_id(&ctx, &display_for(&ctx), &empty_plan(options.target))
        };
        let mut ivy_on_6 = with_chipset(machine(CpuPlatform::IvyBridge, FormFactor::Desktop), "H61");
        ivy_on_6.gpus = vec![igpu(GpuFamily::IntelIvyBridge, "0162")];
        assert_eq!(imei(&ivy_on_6), Some([0x3A, 0x1E, 0x00, 0x00]));
        let plan = plan_for(&ivy_on_6, MacOsVersion::Catalina, "iMac13,2");
        assert!(names(&plan).contains(&"SSDT-IMEI.aml"));
        // No iGPU in use: neither the IMEI id nor SSDT-IMEI.
        let mut headless = ivy_on_6.clone();
        headless.gpus[0].disabled = true;
        assert_eq!(imei(&headless), None);
        let mut ivy_on_7 = with_chipset(machine(CpuPlatform::IvyBridge, FormFactor::Desktop), "Z77");
        ivy_on_7.gpus = vec![igpu(GpuFamily::IntelIvyBridge, "0162")];
        assert_eq!(imei(&ivy_on_7), None);
        let mut unknown = machine(CpuPlatform::SandyBridge, FormFactor::Desktop);
        unknown.gpus = vec![igpu(GpuFamily::IntelSandyBridge, "0122")];
        assert_eq!(imei(&unknown), None);
        let plan = plan_for(&ivy_on_7, MacOsVersion::Catalina, "iMac13,2");
        assert_eq!(names(&plan), ["SSDT-EC-DESKTOP.aml"]);
        assert_eq!(plan.acpi_deletes.len(), 2);
    }

    // ── HEDT ────────────────────────────────────────────────────────────────

    #[test]
    fn x99_gets_rtc_range_and_unc_from_big_sur() {
        let p = with_chipset(machine(CpuPlatform::HaswellE, FormFactor::Desktop), "X99");
        let plan = plan_for(&p, MacOsVersion::BigSur, "iMacPro1,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-DESKTOP.aml",
                "SSDT-RTC0-RANGE-HEDT.aml",
                "SSDT-UNC.aml"
            ]
        );
        dortania(&plan, "SSDT-RTC0-RANGE-HEDT.aml");
        oc_sample(&plan, "SSDT-UNC.aml");

        let plan = plan_for(&p, MacOsVersion::Catalina, "iMacPro1,1");
        assert_eq!(names(&plan), ["SSDT-PLUG.aml", "SSDT-EC-USBX-DESKTOP.aml"]);
    }

    #[test]
    fn x79_gets_unc_without_plug() {
        let p = with_chipset(machine(CpuPlatform::IvyBridgeE, FormFactor::Desktop), "X79");
        let plan = plan_for(&p, MacOsVersion::Monterey, "MacPro6,1");
        assert_eq!(names(&plan), ["SSDT-EC-DESKTOP.aml", "SSDT-UNC.aml"]);
        assert!(plan.acpi_deletes.is_empty());
        // No XCPM on Ivy Bridge-E: SSDT-PM after installing (pm.md).
        let pm: Vec<&PlanNote> = plan
            .post_install
            .iter()
            .filter(|n| n.title.contains("SSDT-PM"))
            .collect();
        assert_eq!(pm.len(), 1);
        assert!(!pm[0].detail.contains("Delete"));

        let plan = plan_for(&p, MacOsVersion::Catalina, "MacPro6,1");
        assert_eq!(names(&plan), ["SSDT-EC-DESKTOP.aml"]);

        let x58 = with_chipset(
            machine(CpuPlatform::NehalemHedt, FormFactor::Desktop),
            "X58",
        );
        let plan = plan_for(&x58, MacOsVersion::Monterey, "MacPro6,1");
        assert_eq!(names(&plan), ["SSDT-EC-DESKTOP.aml"]);
        assert!(plan.post_install.is_empty());
    }

    #[test]
    fn x299_never_combines_rtc_range_with_awac() {
        let mut facts = intel_facts("PC00", "LPC0");
        facts.cpu_paths = (0..4).map(|i| format!("\\_SB.SCK0.CP0{i}")).collect();
        let mut p = with_chipset(machine(CpuPlatform::SkylakeX, FormFactor::Desktop), "X299");
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Sequoia, "iMacPro1,1");
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG.aml", "SSDT-EC-USBX.aml", "SSDT-RTC0-RANGE.aml"]
        );
        assert!(dsl(&plan, "SSDT-PLUG.aml").contains("\\_SB.SCK0.CP00"));
        assert!(dsl(&plan, "SSDT-RTC0-RANGE.aml").contains("PNP0B00"));
    }

    #[test]
    fn awac_and_rtc_range_exclude_each_other() {
        let mut built = Built::default();
        built.planned.push(SsdtKind::Rtc0Range);
        assert!(built.conflicts(&SsdtKind::Awac));
        let mut built = Built::default();
        built.planned.push(SsdtKind::Awac);
        assert!(built.conflicts(&SsdtKind::Rtc0Range));
        assert!(!built.conflicts(&SsdtKind::Pmc));
    }

    // ── AMD ─────────────────────────────────────────────────────────────────

    #[test]
    fn amd_b550_cpur_from_facts() {
        let mut p = with_chipset(machine(CpuPlatform::AmdZen3, FormFactor::Desktop), "B550");
        p.acpi = Some(amd_facts(true));
        let plan = plan_for(&p, MacOsVersion::Sequoia, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-CPUR.aml", "SSDT-EC-USBX.aml"]);
        let cpur = dsl(&plan, "SSDT-CPUR.aml");
        assert!(cpur.contains("ACPI0007"));
        assert!(!cpur.contains("plugin-type"));
        assert!(dsl(&plan, "SSDT-EC-USBX.aml").contains("\\_SB.PCI0.SBRG"));
    }

    #[test]
    fn amd_without_tables() {
        let b550 = with_chipset(machine(CpuPlatform::AmdZen3, FormFactor::Desktop), "B550");
        let plan = plan_for(&b550, MacOsVersion::Sequoia, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-CPUR.aml", "SSDT-EC-USBX-DESKTOP.aml"]);
        dortania(&plan, "SSDT-CPUR.aml");

        let x570 = with_chipset(machine(CpuPlatform::AmdZen2, FormFactor::Desktop), "X570");
        let plan = plan_for(&x570, MacOsVersion::Sequoia, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-EC-USBX-DESKTOP.aml"]);

        let b650 = with_chipset(machine(CpuPlatform::AmdZen4, FormFactor::Desktop), "B650");
        let plan = plan_for(&b650, MacOsVersion::Tahoe, "MacPro7,1");
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG-ALT.aml", "SSDT-EC-USBX-DESKTOP.aml"]
        );
        oc_sample(&plan, "SSDT-PLUG-ALT.aml");

        let fx = machine(CpuPlatform::AmdBulldozer, FormFactor::Desktop);
        let plan = plan_for(&fx, MacOsVersion::Monterey, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-EC-USBX-DESKTOP.aml"]);
    }

    #[test]
    fn am5_and_threadripper_without_tables() {
        // AM5 desktop whose chipset was not detected.
        let zen4 = machine(CpuPlatform::AmdZen4, FormFactor::Desktop);
        let plan = plan_for(&zen4, MacOsVersion::Sequoia, "MacPro7,1");
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG-ALT.aml", "SSDT-EC-USBX-DESKTOP.aml"]
        );
        oc_sample(&plan, "SSDT-PLUG-ALT.aml");
        assert!(has_note(&plan, "Generic prebuilt"));

        let trx50 = with_chipset(machine(CpuPlatform::AmdZen4, FormFactor::Desktop), "TRX50");
        let plan = plan_for(&trx50, MacOsVersion::Sonoma, "MacPro7,1");
        assert_eq!(names(&plan)[0], "SSDT-PLUG-ALT.aml");

        // Threadripper 3000 (TRX40) keeps its Processor objects.
        let trx40 = with_chipset(machine(CpuPlatform::AmdZen2, FormFactor::Desktop), "TRX40");
        let plan = plan_for(&trx40, MacOsVersion::Sonoma, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-EC-USBX-DESKTOP.aml"]);

        // A Zen 4 laptop is not an AM5 board.
        let laptop = machine(CpuPlatform::AmdZen4, FormFactor::Laptop);
        let plan = plan_for(&laptop, MacOsVersion::Sonoma, "MacBookPro16,3");
        assert!(!names(&plan).iter().any(|n| n.starts_with("SSDT-PLUG")));
    }

    #[test]
    fn amd_acpi0007_board_without_a_generic_table_warns() {
        let mut facts = amd_facts(true);
        facts.cpu_paths.clear();
        let mut p = with_chipset(machine(CpuPlatform::AmdZen3, FormFactor::Desktop), "X570");
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-EC-USBX.aml"]);
        let warning = plan
            .notes
            .iter()
            .find(|n| n.title == "SSDT-CPUR.aml not added")
            .map(|n| n.level);
        assert_eq!(warning, Some(NoteLevel::Warning));

        // The same gap on a B550 falls back to Dortania's table.
        let mut p = with_chipset(machine(CpuPlatform::AmdZen3, FormFactor::Desktop), "B550");
        let mut facts = amd_facts(true);
        facts.cpu_paths.clear();
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "MacPro7,1");
        dortania(&plan, "SSDT-CPUR.aml");
    }

    #[test]
    fn amd_processor_objects_need_no_cpur() {
        let mut p = with_chipset(machine(CpuPlatform::AmdZen2, FormFactor::Desktop), "X570");
        p.acpi = Some(amd_facts(false));
        let plan = plan_for(&p, MacOsVersion::Sonoma, "MacPro7,1");
        assert_eq!(names(&plan), ["SSDT-EC-USBX.aml"]);
        assert!(plan.notes.iter().all(|n| n.level != NoteLevel::Warning));
    }

    #[test]
    fn amd_with_an_awac_clock_gets_the_fix() {
        let mut facts = amd_facts(true);
        facts.awac_path = Some("\\_SB.PCI0.SBRG.AWAC".to_string());
        facts.awac_has_stas = true;
        let mut p = with_chipset(machine(CpuPlatform::AmdZen4, FormFactor::Desktop), "X670E");
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::Tahoe, "MacPro7,1");
        assert_eq!(
            names(&plan),
            ["SSDT-CPUR.aml", "SSDT-EC-USBX.aml", "SSDT-AWAC.aml"]
        );
    }

    #[test]
    fn amd_laptop_with_vega_igpu() {
        let mut p = machine(CpuPlatform::AmdZen2, FormFactor::Laptop);
        p.gpus = vec![ProfileGpu {
            name: "AMD Radeon Graphics".to_string(),
            vendor: GpuVendor::Amd,
            family: GpuFamily::AmdApuVega,
            vendor_id: Some("1002".to_string()),
            device_id: Some("1636".to_string()),
            is_igpu: true,
            ..Default::default()
        }];
        let plan = plan_for(&p, MacOsVersion::Sonoma, "MacBookPro16,3");
        assert_eq!(
            names(&plan),
            [
                "SSDT-EC-USBX-LAPTOP.aml",
                "SSDT-XOSI.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        assert!(dsl(&plan, "SSDT-PNLF.aml").contains("_UID 19"));
        assert!(patch_comments(&plan)
            .iter()
            .any(|c| c.starts_with("_OSI to XOSI")));
    }

    // ── Laptops ─────────────────────────────────────────────────────────────

    #[test]
    fn ivy_bridge_laptop_from_facts() {
        let mut facts = intel_facts("PCI0", "LPCB");
        facts.awac_path = None;
        facts.awac_has_stas = false;
        facts.pnlf_exists = true;
        let mut p = with_chipset(machine(CpuPlatform::IvyBridge, FormFactor::Laptop), "HM65");
        p.gpus = vec![igpu(GpuFamily::IntelIvyBridge, "0166")];
        p.acpi = Some(facts);
        let plan = plan_for(&p, MacOsVersion::BigSur, "MacBookPro10,2");
        assert_eq!(
            names(&plan),
            [
                "SSDT-EC-USBX.aml",
                "SSDT-IMEI.aml",
                "SSDT-XOSI.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        assert!(dsl(&plan, "SSDT-PNLF.aml").contains("_UID 14"));
        assert!(dsl(&plan, "SSDT-IMEI.aml").contains("0x00160000"));
        assert_eq!(
            patch_comments(&plan),
            [
                "_OSI to XOSI rename - requires SSDT-XOSI.aml",
                "PNLF to XNLF rename (SSDT-PNLF.aml)"
            ]
        );
        assert_eq!(plan.acpi_deletes.len(), 2);
        assert!(plan.post_install.iter().any(|n| n.title.contains("IRQ")));
    }

    #[test]
    fn kaby_lake_laptop_with_i2c_trackpad() {
        let mut p = with_chipset(
            machine(CpuPlatform::KabyLake, FormFactor::Laptop),
            "Sunrise Point-LP",
        );
        p.gpus = vec![igpu(GpuFamily::IntelKabyLake, "5916")];
        p.input.touchpad_bus = Some(InputBus::I2c);
        let plan = plan_for(&p, MacOsVersion::Ventura, "MacBookPro14,1");
        // Without the tables only the guarded GPI0 stub: a blind _OSI rename
        // can break OEM names such as OSID.
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-LAPTOP.aml",
                "SSDT-GPI0.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        dortania(&plan, "SSDT-EC-USBX-LAPTOP.aml");
        assert!(dsl(&plan, "SSDT-GPI0.aml").contains("CondRefOf (\\GPEN)"));
        assert!(dsl(&plan, "SSDT-PNLF.aml").contains("_UID 16"));
        assert!(plan.acpi_patches.is_empty());
        let xosi = plan
            .notes
            .iter()
            .find(|n| n.title == "SSDT-XOSI left out")
            .map(|n| n.level);
        assert_eq!(xosi, Some(NoteLevel::Info));
    }

    /// Kaby Lake-R laptop DSDT: Processor objects, EC0 without `_STA`, a
    /// GPI0 gated by GPEN, Windows `_OSI` checks up to 2017 and an OSID
    /// method (the Dell case that breaks a blind `_OSI` rename).
    fn kbl_laptop_dsdt() -> Vec<u8> {
        let io = |port: u16| Resource::Io {
            min: port,
            max: port,
            align: 0,
            len: 1,
        };
        let ec0 = Term::device(
            "EC0",
            vec![
                Term::name("_HID", Term::EisaId("PNP0C09".into())),
                Term::name("_UID", Term::int(1)),
                Term::name("_CRS", Term::Resources(vec![io(0x62), io(0x66)])),
                Term::name("_GPE", Term::int(0x17)),
            ],
        );
        let rtc = Term::device(
            "RTC",
            vec![
                Term::name("_HID", Term::EisaId("PNP0B00".into())),
                Term::name(
                    "_CRS",
                    Term::Resources(vec![
                        Resource::Io {
                            min: 0x70,
                            max: 0x70,
                            align: 1,
                            len: 8,
                        },
                        Resource::IrqNoFlags { mask: 1 << 8 },
                    ]),
                ),
            ],
        );
        let gpi0 = Term::device(
            "GPI0",
            vec![
                Term::name("_HID", Term::str("INT344B")),
                Term::method(
                    "_STA",
                    0,
                    vec![
                        Term::if_then(
                            Term::equal(Term::path("GPEN"), Term::int(0)),
                            vec![Term::ret(Term::int(0))],
                        ),
                        Term::ret(Term::int(0x0F)),
                    ],
                ),
            ],
        );
        let osi_checks = Term::method(
            "_INI",
            0,
            ["Windows 2009", "Windows 2015", "Windows 2017"]
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    Term::if_then(
                        Term::call("_OSI", vec![Term::str(s)]),
                        vec![Term::store(
                            Term::int(0x07D9 + i as u64),
                            Term::path("OSYS"),
                        )],
                    )
                })
                .collect(),
        );
        let terms = vec![
            Term::name("OSYS", Term::int(0)),
            Term::name("GPEN", Term::int(0)),
            Term::method("OSID", 0, vec![Term::ret(Term::int(1))]),
            Term::scope(
                "\\_PR",
                (0..4u8)
                    .map(|i| Term::Processor {
                        name: format!("CPU{i}"),
                        id: i + 1,
                        pblk: 0x1810,
                        pblk_len: 6,
                        body: vec![],
                    })
                    .collect(),
            ),
            Term::scope(
                "\\_SB",
                vec![Term::device(
                    "PCI0",
                    vec![
                        Term::name("_HID", Term::EisaId("PNP0A08".into())),
                        Term::name("_CID", Term::EisaId("PNP0A03".into())),
                        osi_checks,
                        Term::device("GFX0", vec![Term::name("_ADR", Term::int(0x0002_0000))]),
                        gpi0,
                        Term::device(
                            "LPCB",
                            vec![Term::name("_ADR", Term::int(0x001F_0000)), ec0, rtc],
                        ),
                    ],
                )],
            ),
        ];
        aml::table(b"DSDT", 2, "OCTEST", "KBL-R", 1, &encode_body(&terms))
    }

    #[test]
    fn i2c_laptop_from_dumped_tables_gets_xosi() {
        let dir = write_tables("kbl-laptop", &[("DSDT.aml", kbl_laptop_dsdt())]);
        let mut p = with_chipset(
            machine(CpuPlatform::KabyLake, FormFactor::Laptop),
            "Sunrise Point-LP",
        );
        p.gpus = vec![igpu(GpuFamily::IntelKabyLake, "5917")];
        p.input.touchpad_bus = Some(InputBus::I2c);
        p.acpi_tables_dir = Some(dir.0.to_string_lossy().into_owned());
        let plan = plan_for(&p, MacOsVersion::Ventura, "MacBookPro14,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX.aml",
                "SSDT-XOSI.aml",
                "SSDT-GPI0.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        assert!(plan
            .ssdts
            .iter()
            .all(|s| matches!(s.source, SsdtSource::Generated { .. })));
        // Laptop mode: the real EC0 stays, a fake EC is added beside it.
        let ec = dsl(&plan, "SSDT-EC-USBX.aml");
        assert!(ec.contains("ACID0001") && !ec.contains("EC0._STA"));
        let xosi = dsl(&plan, "SSDT-XOSI.aml");
        assert!(xosi.contains("\"Windows 2017\"") && !xosi.contains("\"Windows 2018\""));
        // OSID is renamed before the _OSI rename could split it.
        let finds: Vec<&str> = plan.acpi_patches.iter().map(|p| p.find.as_str()).collect();
        assert_eq!(
            &finds[..2],
            ["4F534944", "5F4F5349"],
            "{:?}",
            patch_comments(&plan)
        );
        assert!(patch_comments(&plan)
            .iter()
            .any(|c| c.starts_with("GPI0 _STA to XSTA")));
        assert!(!has_note(&plan, "SSDT-XOSI left out"));
        assert!(!has_note(&plan, "Generic prebuilt"));
    }

    #[test]
    fn ps2_laptop_skips_trackpad_tables() {
        let mut p = machine(CpuPlatform::Skylake, FormFactor::Laptop);
        p.gpus = vec![igpu(GpuFamily::IntelSkylake, "1916")];
        p.input.touchpad_bus = Some(InputBus::Ps2);
        let plan = plan_for(&p, MacOsVersion::HighSierra, "MacBookPro13,1");
        // No ALS0 before Catalina.
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG.aml", "SSDT-EC-USBX-LAPTOP.aml", "SSDT-PNLF.aml"]
        );
        assert!(plan.acpi_patches.is_empty());
    }

    #[test]
    fn coffee_lake_laptop_pmc_only_for_9th_gen() {
        let mut p = with_chipset(
            machine(CpuPlatform::CoffeeLake, FormFactor::Laptop),
            "HM370",
        );
        p.gpus = vec![igpu(GpuFamily::IntelCoffeeLake, "3e9b")];
        p.cpu.name = "Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz".to_string();
        let plan = plan_for(&p, MacOsVersion::Sequoia, "MacBookPro15,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-LAPTOP.aml",
                "SSDT-AWAC.aml",
                "SSDT-PMC.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        assert!(dsl(&plan, "SSDT-PNLF.aml").contains("_UID 19"));

        p.cpu.name = "Intel(R) Core(TM) i7-8750H CPU @ 2.20GHz".to_string();
        let plan = plan_for(&p, MacOsVersion::Sequoia, "MacBookPro15,1");
        assert!(!names(&plan).contains(&"SSDT-PMC.aml"));
        assert!(names(&plan).contains(&"SSDT-AWAC.aml"));
    }

    #[test]
    fn comet_lake_laptop_needs_no_pmc() {
        let mut p = with_chipset(
            machine(CpuPlatform::CometLake, FormFactor::Laptop),
            "400 Series PCH-LP",
        );
        p.gpus = vec![igpu(GpuFamily::IntelCometLake, "9b41")];
        p.cpu.name = "Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz".to_string();
        p.motherboard_vendor = "ASUSTeK COMPUTER INC.".to_string();
        let plan = plan_for(&p, MacOsVersion::Tahoe, "MacBookPro16,2");
        // RHUB is a desktop ASUS fix; laptops do not get it.
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-LAPTOP.aml",
                "SSDT-AWAC.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        assert!(dsl(&plan, "SSDT-PNLF.aml").contains("_UID 19"));
        assert!(plan.post_install.is_empty());
    }

    #[test]
    fn arrandale_laptop() {
        let mut p = machine(CpuPlatform::Arrandale, FormFactor::Laptop);
        p.gpus = vec![igpu(GpuFamily::IntelIronLake, "0046")];
        let plan = plan_for(&p, MacOsVersion::HighSierra, "MacBookPro6,2");
        assert_eq!(
            names(&plan),
            ["SSDT-EC-LAPTOP.aml", "SSDT-XOSI.aml", "SSDT-PNLF.aml"]
        );
        dortania(&plan, "SSDT-EC-LAPTOP.aml");
        assert!(dsl(&plan, "SSDT-PNLF.aml").contains("_UID 14"));
        assert_eq!(
            patch_comments(&plan),
            ["_OSI to XOSI rename - requires SSDT-XOSI.aml"]
        );
        assert!(plan.acpi_deletes.is_empty());
        assert!(plan.post_install.iter().any(|n| n.title.contains("IRQ")));
    }

    #[test]
    fn mini_pc_gets_no_panel_or_trackpad_tables() {
        let mut p = machine(CpuPlatform::CoffeeLake, FormFactor::MiniPc);
        p.cpu.is_mobile = true;
        p.cpu.name = "Intel(R) Core(TM) i7-8559U CPU @ 2.70GHz".to_string();
        p.chipset = Some("Cannon Point-LP".to_string());
        p.gpus = vec![igpu(GpuFamily::IntelCoffeeLake, "3ea5")];
        p.input.touchpad_bus = Some(InputBus::I2c);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "Macmini8,1");
        assert_eq!(
            names(&plan),
            ["SSDT-PLUG.aml", "SSDT-EC-USBX-DESKTOP.aml", "SSDT-AWAC.aml"]
        );
        assert!(!has_note(&plan, "SSDT-PMC"));
    }

    #[test]
    fn ice_lake_laptop_resets_the_root_hub() {
        let mut p = machine(CpuPlatform::IceLake, FormFactor::Laptop);
        p.gpus = vec![igpu(GpuFamily::IntelIceLake, "8a52")];
        let plan = plan_for(&p, MacOsVersion::Tahoe, "MacBookPro16,2");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-LAPTOP.aml",
                "SSDT-AWAC.aml",
                "SSDT-RHUB.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
        dortania(&plan, "SSDT-RHUB.aml");
    }

    #[test]
    fn disabled_igpu_gets_no_backlight_device() {
        let mut p = machine(CpuPlatform::CoffeeLake, FormFactor::AllInOne);
        p.gpus = vec![igpu(GpuFamily::IntelCoffeeLake, "3e92")];
        let options = BuildOptions {
            target: MacOsVersion::Sonoma,
            ..BuildOptions::default()
        };
        let ctx = PlanContext::new(&p, &options);
        let mut plan = empty_plan(MacOsVersion::Sonoma);
        plan.device_properties.push(DevicePropertyEntry {
            path: "PciRoot(0x0)/Pci(0x2,0x0)".to_string(),
            properties: vec![DeviceProperty {
                key: "class-code".to_string(),
                value: PlistScalar::Data("FFFFFFFF".to_string()),
            }],
            reason: String::new(),
        });
        apply(&ctx, &display_for(&ctx), &mut plan);
        assert!(!names(&plan).contains(&"SSDT-PNLF.aml"));

        // The same all-in-one with its iGPU in use: desktop EC, panel tables.
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMac19,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX-DESKTOP.aml",
                "SSDT-AWAC.aml",
                "SSDT-PNLF.aml",
                "SSDT-ALS0.aml"
            ]
        );
    }

    #[test]
    fn clarksfield_laptop_on_a_dgpu_has_no_pnlf() {
        let mut p = machine(CpuPlatform::Arrandale, FormFactor::Laptop);
        p.gpus = vec![ProfileGpu {
            name: "NVIDIA GeForce GT 330M".to_string(),
            vendor: GpuVendor::Nvidia,
            family: GpuFamily::NvidiaTesla,
            ..Default::default()
        }];
        let plan = plan_for(&p, MacOsVersion::HighSierra, "MacBookPro6,2");
        assert_eq!(names(&plan), ["SSDT-EC-LAPTOP.aml", "SSDT-XOSI.aml"]);
    }

    // ── VMs ─────────────────────────────────────────────────────────────────

    #[test]
    fn vms_stay_minimal() {
        let mut p = machine(CpuPlatform::Skylake, FormFactor::Desktop);
        p.vm = Some(VmKind::Kvm);
        p.chipset = Some("Z390".to_string());
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMacPro1,1");
        assert_eq!(names(&plan), ["SSDT-EC-USBX-DESKTOP.aml"]);
        assert!(plan.acpi_deletes.is_empty());
        assert!(!has_note(&plan, "Hyper-V"));

        p.vm = Some(VmKind::HyperV);
        let plan = plan_for(&p, MacOsVersion::Sonoma, "iMacPro1,1");
        // MacHyperVSupport: VMBUS, DEV, PLUG in that order, from OpenCore.
        assert_eq!(
            names(&plan),
            [
                "SSDT-EC-USBX-DESKTOP.aml",
                "SSDT-HV-VMBUS.aml",
                "SSDT-HV-DEV.aml",
                "SSDT-HV-PLUG.aml"
            ]
        );
        for file in ["SSDT-HV-VMBUS.aml", "SSDT-HV-DEV.aml", "SSDT-HV-PLUG.aml"] {
            let ssdt = plan.ssdts.iter().find(|s| s.file_name == file).unwrap();
            assert!(matches!(&ssdt.source, SsdtSource::OcSample { file: f } if f == file));
        }
        let bases: Vec<(&str, &str)> = plan
            .acpi_patches
            .iter()
            .map(|p| (p.base.as_str(), p.find.as_str()))
            .collect();
        assert_eq!(
            bases,
            [
                ("\\_SB.VMOD", "5F484944"),
                ("\\_SB.VMOD.VMBS", "5F484944"),
                ("\\_SB.VMOD.TPM2", "5F535441"),
                ("\\_SB.NVDR", "5F535441"),
                ("\\_SB.EPC", "5F535441"),
                ("\\_SB.VMOD.BAT1", "5F535441"),
            ]
        );
        for p in &plan.acpi_patches {
            assert_eq!((p.count, p.table_signature.as_deref()), (1, Some("DSDT")));
            assert!(p.enabled && p.comment.contains("(SSDT-HV-"), "{}", p.comment);
        }
        assert!(has_note(&plan, "Hyper-V"));
    }

    #[test]
    fn intel_800_series_gets_the_conditional_scope_patch() {
        let rcsp = |plan: &BuildPlan| {
            plan.acpi_patches
                .iter()
                .any(|p| p.comment.starts_with("Remove conditional ACPI scope"))
        };
        let z890 = with_chipset(machine(CpuPlatform::ArrowLake, FormFactor::Desktop), "Z890");
        let plan = plan_for(&z890, MacOsVersion::Sequoia, "MacPro7,1");
        let patch = plan
            .acpi_patches
            .iter()
            .find(|p| p.comment.starts_with("Remove conditional ACPI scope"))
            .expect("800-series patch");
        assert_eq!(patch.find.len(), patch.mask.len());
        assert_eq!(patch.find.len(), patch.replace.len());
        assert_eq!(patch.table_signature.as_deref(), Some("DSDT"));
        // Arrow Lake desktop without a known chipset: same board family.
        let arrow_lake = machine(CpuPlatform::ArrowLake, FormFactor::Desktop);
        assert!(rcsp(&plan_for(&arrow_lake, MacOsVersion::Sequoia, "MacPro7,1")));
        let z790 = with_chipset(machine(CpuPlatform::RaptorLake, FormFactor::Desktop), "Z790");
        assert!(!rcsp(&plan_for(&z790, MacOsVersion::Sequoia, "MacPro7,1")));
    }

    // ── Indexed tables ──────────────────────────────────────────────────────

    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_tables(name: &str, images: &[(&str, Vec<u8>)]) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "oneclick-planner-acpi-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (file, bytes) in images {
            std::fs::write(dir.join(file), bytes).expect("write table");
        }
        TempDir(dir)
    }

    fn sta_if(var: &str, value: u64) -> Term {
        Term::method(
            "_STA",
            0,
            vec![
                Term::if_then(
                    Term::equal(Term::path(var), Term::int(value)),
                    vec![Term::ret(Term::int(0x0F))],
                ),
                Term::ret(Term::int(0)),
            ],
        )
    }

    /// Z390 DSDT: a valid EC named EC with an `_STA`, AWAC and RTC switched
    /// by STAS, Processor objects in `\_PR`.
    fn z390_dsdt() -> Vec<u8> {
        let io = |port: u16| Resource::Io {
            min: port,
            max: port,
            align: 0,
            len: 1,
        };
        let ec = Term::device(
            "EC",
            vec![
                Term::name("_HID", Term::EisaId("PNP0C09".into())),
                Term::name("_UID", Term::int(1)),
                Term::name("_CRS", Term::Resources(vec![io(0x62), io(0x66)])),
                Term::name("_GPE", Term::int(0x17)),
                sta_if("ECON", 1),
            ],
        );
        let rtc = Term::device(
            "RTC",
            vec![
                Term::name("_HID", Term::EisaId("PNP0B00".into())),
                Term::name(
                    "_CRS",
                    Term::Resources(vec![
                        Resource::Io {
                            min: 0x70,
                            max: 0x70,
                            align: 1,
                            len: 8,
                        },
                        Resource::IrqNoFlags { mask: 1 << 8 },
                    ]),
                ),
                sta_if("STAS", 1),
            ],
        );
        let awac = Term::device(
            "AWAC",
            vec![Term::name("_HID", Term::str("ACPI000E")), sta_if("STAS", 0)],
        );
        let terms = vec![
            Term::name("STAS", Term::int(1)),
            Term::name("ECON", Term::int(1)),
            Term::scope(
                "\\_PR",
                (0..4u8)
                    .map(|i| Term::Processor {
                        name: format!("CPU{i}"),
                        id: i + 1,
                        pblk: 0x1810,
                        pblk_len: 6,
                        body: vec![],
                    })
                    .collect(),
            ),
            Term::scope(
                "\\_SB",
                vec![Term::device(
                    "PCI0",
                    vec![
                        Term::name("_HID", Term::EisaId("PNP0A08".into())),
                        Term::name("_CID", Term::EisaId("PNP0A03".into())),
                        Term::device(
                            "XHC",
                            vec![
                                Term::name("_ADR", Term::int(0x0014_0000)),
                                Term::device("RHUB", vec![Term::name("_ADR", Term::int(0))]),
                            ],
                        ),
                        Term::device(
                            "LPCB",
                            vec![Term::name("_ADR", Term::int(0x001F_0000)), ec, awac, rtc],
                        ),
                    ],
                )],
            ),
        ];
        aml::table(b"DSDT", 2, "OCTEST", "CFL-Z390", 1, &encode_body(&terms))
    }

    #[test]
    fn z390_from_dumped_tables_renames_in_order() {
        let dir = write_tables("z390", &[("DSDT.aml", z390_dsdt())]);
        let mut p = with_chipset(
            machine(CpuPlatform::CoffeeLake, FormFactor::Desktop),
            "Z390",
        );
        p.acpi_tables_dir = Some(dir.0.to_string_lossy().into_owned());
        let plan = plan_for(&p, MacOsVersion::Sequoia, "iMac19,1");
        assert_eq!(
            names(&plan),
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX.aml",
                "SSDT-AWAC.aml",
                "SSDT-PMC.aml"
            ]
        );
        assert!(dsl(&plan, "SSDT-PLUG.aml").contains("\\_PR.CPU0"));
        // The real EC named EC becomes EC0 first, then its _STA is renamed.
        let comments = patch_comments(&plan);
        assert_eq!(comments.len(), 2, "{comments:?}");
        assert!(comments[0].starts_with("EC to EC0"));
        // Both name their table, so the build can drop them with it.
        assert!(comments[0].ends_with("(SSDT-EC-USBX.aml)"), "{comments:?}");
        assert_eq!(comments[1], "EC0 _STA to XSTA rename (SSDT-EC-USBX.aml)");
        assert_eq!(plan.acpi_patches[0].find, "45435F5F");
        assert_eq!(plan.acpi_patches[0].replace, "4543305F");
        assert_eq!(
            plan.acpi_patches[1].table_signature.as_deref(),
            Some("DSDT")
        );
        assert!(dsl(&plan, "SSDT-AWAC.aml").contains("STAS"));
    }

    #[test]
    fn unreadable_tables_fall_back_with_a_warning() {
        let mut p = with_chipset(machine(CpuPlatform::KabyLake, FormFactor::Desktop), "Z270");
        p.acpi_tables_dir = Some(
            std::env::temp_dir()
                .join("oneclick-planner-acpi-missing-dir")
                .to_string_lossy()
                .into_owned(),
        );
        let plan = plan_for(&p, MacOsVersion::Ventura, "iMac18,3");
        assert!(has_note(&plan, "ACPI tables could not be read"));
        assert_eq!(names(&plan), ["SSDT-PLUG.aml", "SSDT-EC-USBX-DESKTOP.aml"]);
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    #[test]
    fn quirks_follow_dortania() {
        let p = machine(CpuPlatform::KabyLake, FormFactor::Desktop);
        let plan = plan_for(&p, MacOsVersion::Ventura, "iMac18,3");
        assert_eq!(plan.acpi_quirks.len(), 6);
        assert_eq!(
            plan.acpi_quirks.get("ResetLogoStatus"),
            Some(&PlistScalar::Bool(true))
        );
        assert_eq!(
            plan.acpi_quirks.get("NormalizeHeaders"),
            Some(&PlistScalar::Bool(false))
        );
    }

    #[test]
    fn patches_are_deduplicated_in_order() {
        let patch = |find: &str| AcpiPatch {
            comment: find.to_string(),
            find: find.to_string(),
            replace: "58534154".to_string(),
            table_signature: Some("DSDT".to_string()),
            oem_table_id: None,
            count: 1,
            enabled: true,
            ..Default::default()
        };
        let mut list = Vec::new();
        add_patch(&mut list, patch("5F535441"));
        add_patch(&mut list, patch("5f535441"));
        add_patch(&mut list, patch("5F534154"));
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].find, "5F534154");

        // An optional (disabled) rename another table needs becomes enabled.
        let mut optional = patch("5F535441");
        optional.enabled = false;
        let mut list = vec![optional.clone()];
        add_patch(&mut list, patch("5F535441"));
        assert_eq!(list.len(), 1);
        assert!(list[0].enabled);
        add_patch(&mut list, optional);
        assert!(list[0].enabled);

        // Every table that needs a shared rename is named in its comment.
        let named = |file: &str, enabled: bool| {
            let mut p = patch("5F535441");
            p.comment = format!("_STA to XSTA rename ({file})");
            p.enabled = enabled;
            p
        };
        let mut list = vec![named("SSDT-A.aml", true)];
        add_patch(&mut list, named("SSDT-B.aml", true));
        add_patch(&mut list, named("SSDT-C.aml", false));
        assert_eq!(list[0].comment, "_STA to XSTA rename (SSDT-A.aml, SSDT-B.aml)");
        let mut list = vec![named("SSDT-C.aml", false)];
        add_patch(&mut list, named("SSDT-B.aml", true));
        assert_eq!(list[0].comment, "_STA to XSTA rename (SSDT-B.aml)");
        // A Base-scoped rename is not the same patch as a table-wide one.
        let mut scoped = patch("5F535441");
        scoped.base = "\\_SB.NVDR".into();
        add_patch(&mut list, scoped);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn smbios_generations() {
        for model in [
            "iMac17,1",
            "iMac20,2",
            "iMacPro1,1",
            "MacPro7,1",
            "MacBookPro13,1",
            "MacBookAir8,1",
            "Macmini8,1",
        ] {
            assert!(skylake_era_smbios(model), "{model}");
        }
        for model in [
            "iMac16,2",
            "MacPro6,1",
            "MacBookPro12,1",
            "MacBookAir7,2",
            "Macmini7,1",
            "",
            "Unknown",
        ] {
            assert!(!skylake_era_smbios(model), "{model}");
        }
    }

    #[test]
    fn ninth_gen_names() {
        assert!(ninth_gen_coffee_lake(
            "Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz"
        ));
        assert!(ninth_gen_coffee_lake("Intel(R) Core(TM) i9-9980HK"));
        assert!(ninth_gen_coffee_lake("Intel(R) Xeon(R) E-2276M CPU"));
        assert!(!ninth_gen_coffee_lake(
            "Intel(R) Core(TM) i7-8750H CPU @ 2.20GHz"
        ));
        assert!(!ninth_gen_coffee_lake("Intel(R) Xeon(R) E-2176M CPU"));
        assert!(!ninth_gen_coffee_lake(""));
    }

    #[test]
    fn pnlf_uids() {
        assert_eq!(pnlf_uid(GpuFamily::IntelSandyBridge), Some(14));
        assert_eq!(pnlf_uid(GpuFamily::IntelBroadwell), Some(15));
        assert_eq!(pnlf_uid(GpuFamily::IntelKabyLake), Some(16));
        assert_eq!(pnlf_uid(GpuFamily::IntelIceLake), Some(19));
        assert_eq!(pnlf_uid(GpuFamily::IntelXe), None);
        assert_eq!(pnlf_uid(GpuFamily::AmdApuRdna), None);
    }

    // ── Every supported platform ────────────────────────────────────────────

    fn check_invariants(plan: &BuildPlan, profile: &HardwareProfile, label: &str) {
        let all = names(plan);
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "{label}: duplicate SSDTs {all:?}");
        let ecs = all.iter().filter(|n| n.starts_with("SSDT-EC")).count();
        assert_eq!(ecs, 1, "{label}: {all:?}");
        let awac = all.contains(&"SSDT-AWAC.aml");
        let rtc_range = all.iter().any(|n| n.starts_with("SSDT-RTC0-RANGE"));
        assert!(!(awac && rtc_range), "{label}: AWAC with RTC0-RANGE");
        for s in &plan.ssdts {
            assert!(s.file_name.ends_with(".aml"), "{label}: {}", s.file_name);
            match &s.source {
                SsdtSource::Generated { aml_hex, dsl } => {
                    assert!(aml_hex.starts_with("53534454"), "{label}: {}", s.file_name);
                    assert!(!dsl.is_empty());
                }
                SsdtSource::Dortania { file } => assert!(
                    kext_catalog::dortania_ssdt(file).is_some(),
                    "{label}: {file} is not pinned"
                ),
                SsdtSource::OcSample { file } => {
                    assert!(OC_SAMPLES.contains(&file.as_str()), "{label}: {file}")
                }
            }
            assert!(!s.reason.is_empty());
        }
        for p in &plan.acpi_patches {
            assert_eq!(p.find.len(), p.replace.len(), "{label}: {}", p.comment);
            assert!(!p.find.is_empty() && p.find.len() % 2 == 0);
        }
        // Renames only come with the table that needs them.
        let renames_to = |hex: &str| plan.acpi_patches.iter().any(|p| p.replace == hex);
        assert_eq!(
            renames_to("584F5349"),
            all.contains(&"SSDT-XOSI.aml"),
            "{label}"
        );
        if renames_to("584E4C46") {
            assert!(all.contains(&"SSDT-PNLF.aml"), "{label}");
        }
        let platform = profile.cpu.platform;
        let laptop = profile.form_factor == FormFactor::Laptop;
        let panel = profile.form_factor.has_internal_panel();
        if profile.vm.is_some() {
            let expected = if profile.vm == Some(VmKind::HyperV) { 4 } else { 1 };
            assert_eq!(all.len(), expected, "{label}: {all:?}");
        } else {
            if profile.cpu.vendor == CpuVendor::Amd {
                assert!(!all.contains(&"SSDT-PLUG.aml"), "{label}: PLUG on AMD");
                assert!(!all.contains(&"SSDT-RHUB.aml"), "{label}");
            } else {
                assert!(!all.contains(&"SSDT-CPUR.aml"), "{label}: CPUR on Intel");
                assert_eq!(
                    all.iter().any(|n| n.starts_with("SSDT-PLUG")),
                    intel_xcpm(platform),
                    "{label}: PLUG only with XCPM"
                );
            }
            for name in ["SSDT-XOSI.aml", "SSDT-GPI0.aml"] {
                assert!(laptop || !all.contains(&name), "{label}: {name}");
            }
            for name in ["SSDT-PNLF.aml", "SSDT-ALS0.aml"] {
                assert!(panel || !all.contains(&name), "{label}: {name}");
            }
            if all.contains(&"SSDT-ALS0.aml") {
                assert!(all.contains(&"SSDT-PNLF.aml"), "{label}");
            }
            if all.contains(&"SSDT-IMEI.aml") {
                assert!(matches!(
                    platform,
                    CpuPlatform::SandyBridge | CpuPlatform::IvyBridge
                ));
            }
        }
        let cpupm = matches!(platform, CpuPlatform::SandyBridge | CpuPlatform::IvyBridge);
        assert_eq!(
            plan.acpi_deletes.len(),
            if cpupm { 2 } else { 0 },
            "{label}"
        );
        assert_eq!(plan.acpi_quirks.len(), QUIRKS.len());
    }

    #[test]
    fn every_supported_platform_form_factor_and_target() {
        let forms = [
            FormFactor::Desktop,
            FormFactor::Laptop,
            FormFactor::AllInOne,
            FormFactor::MiniPc,
        ];
        let mut runs = 0;
        for &platform in cpu_db::all_platforms() {
            let info = cpu_db::platform_info(platform);
            if !info.supported {
                continue;
            }
            let min = info.min_macos.unwrap_or(MacOsVersion::HighSierra);
            let max = info.max_macos.unwrap_or(MacOsVersion::Tahoe);
            for form in forms {
                for target in MacOsVersion::ALL
                    .into_iter()
                    .filter(|t| (min..=max).contains(t))
                {
                    for (facts, smbios) in [
                        (None, "MacPro7,1"),
                        (None, "iMac14,2"),
                        (Some(intel_facts("PCI0", "LPCB")), "MacPro7,1"),
                        (Some(intel_facts("PC00", "LPC0")), "iMac14,2"),
                        (Some(amd_facts(true)), "MacPro7,1"),
                    ] {
                        let mut p = machine(platform, form);
                        p.acpi = facts.clone();
                        p.input.touchpad_bus = Some(InputBus::I2c);
                        let label = format!(
                            "{platform:?} {form:?} {target:?} {smbios} facts={}",
                            facts.is_some()
                        );
                        let plan = plan_for(&p, target, smbios);
                        check_invariants(&plan, &p, &label);
                        runs += 1;
                    }
                }
            }
        }
        // VMs on any platform.
        for vm in [
            VmKind::Kvm,
            VmKind::Vmware,
            VmKind::HyperV,
            VmKind::VirtualBox,
        ] {
            let mut p = machine(CpuPlatform::Unknown, FormFactor::Desktop);
            p.vm = Some(vm);
            let plan = plan_for(&p, MacOsVersion::Sonoma, "iMacPro1,1");
            check_invariants(&plan, &p, &format!("{vm:?}"));
        }
        assert!(runs > 500, "only {runs} combinations");
    }
}
