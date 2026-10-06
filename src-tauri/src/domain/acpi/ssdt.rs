//! SSDT generation (Dortania "Getting started with ACPI" / SSDTTime logic).
//!
//! Every table uses the machine's own paths (PCI0 vs PC00, LPCB vs LPC0 vs
//! SBRG, `\_PR.CPU0` vs `\_SB.PR00` vs `\_SB.PLTF.C000`) and is gated with
//! `_OSI ("Darwin")` where Dortania/SSDTTime gate it.
//!
//! Two entry points:
//! - [`generate_with_tables`] works from the indexed DSDT/SSDTs and matches
//!   SSDTTime, including `_STA` → `XSTA` renames made unique with the
//!   surrounding bytes (scoped to the defining table's signature, and to its
//!   OEM table id for SSDTs), and SSDT-RTC0-RANGE ranges taken from the RTC
//!   `_CRS` with its gaps closed;
//! - [`generate`] works from [`AcpiFacts`] alone. Where an existing `_STA`
//!   would need such a rename, the definition is guarded with `CondRefOf`
//!   (or skipped) instead, and the EC → EC0 rename is avoided by placing the
//!   fake EC under `\_SB` like Dortania's prebuilt tables.
//!
//! Name renames (`_OSI` → `XOSI`, `EC` → `EC0`, `PNLF` → `XNLF`, `OSID` →
//! `XSID`) apply to every table, as SSDTTime and Dortania do, so references
//! from OEM SSDTs stay consistent. Patches are listed in the order they must
//! be applied.
//!
//! One canonical file name per kind (see [`SsdtKind::file_name`]); the only
//! variant is `SSDT-PLUG-ALT.aml`, used by [`SsdtKind::Plug`] when the CPUs
//! are `ACPI0007` devices.
//!
//! Errors carry [`ERR_NOT_NEEDED`] when the machine does not need the table
//! (do not fall back to a prebuilt one) and [`ERR_INSUFFICIENT`] when the
//! facts are not enough (use [`fallback_prebuilt`]).

use crate::domain::model::{AcpiFacts, AcpiPatch, SsdtSource};
use crate::error::AppError;

use super::aml::{self, MatchOp, ObjType, Resource, Term, ONES};
use super::dsdt::{display_path, parse_path, AcpiTables, CpuList, ROOT_HUB_NAMES};
use super::namespace::{NsKind, NsObject};
use super::parse::{find_all, Seg, Value};

/// OEM id written into generated tables.
pub const OEM_ID: &str = "OCLICK";
/// Error code: this machine does not need the SSDT.
pub const ERR_NOT_NEEDED: &str = "ACPI_SSDT_NOT_NEEDED";
/// Error code: the facts are not enough to build the SSDT.
pub const ERR_INSUFFICIENT: &str = "ACPI_FACTS_INSUFFICIENT";

const USBX_PROPERTIES: [(&str, u64); 4] = [
    ("kUSBSleepPowerSupply", 0x13EC),
    ("kUSBSleepPortCurrentLimit", 0x0834),
    ("kUSBWakePowerSupply", 0x13EC),
    ("kUSBWakePortCurrentLimit", 0x0834),
];

/// Windows `_OSI` strings in release order (Microsoft "_OSI strings for
/// Windows operating systems").
const OSI_STRINGS: [&str; 22] = [
    "Windows 2000",
    "Windows 2001",
    "Windows 2001 SP1",
    "Windows 2001.1",
    "Windows 2001 SP2",
    "Windows 2001.1 SP1",
    "Windows 2006",
    "Windows 2006 SP1",
    "Windows 2006.1",
    "Windows 2009",
    "Windows 2012",
    "Windows 2013",
    "Windows 2015",
    "Windows 2016",
    "Windows 2017",
    "Windows 2017.2",
    "Windows 2018",
    "Windows 2018.2",
    "Windows 2019",
    "Windows 2020",
    "Windows 2021",
    "Windows 2022",
];

/// Controller names that clash with Apple's built-in port maps (SSDTTime).
const ILLEGAL_USB_NAMES: [&[u8; 4]; 4] = [b"XHC1", b"EHC1", b"EHC2", b"PXSX"];

/// Name schemes for redefined processor objects (SSDTTime order).
const CPU_NAME_SCHEMES: [&str; 6] = ["C000", "CP00", "P000", "PR00", "CX00", "PX00"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsdtKind {
    /// Fake EC (desktop: disable real EC named EC; laptop: keep real EC) + USBX power properties.
    EcUsbx { laptop: bool },
    /// plugin-type=1 on the first CPU object (SSDT-PLUG / PLUG-ALT for ACPI0007).
    Plug,
    /// AWAC → legacy RTC (STAS=1 or fake RTC0).
    Awac,
    /// PMCR device for NVRAM on 300-series Intel.
    Pmc,
    /// Disable RHUB so macOS rebuilds ports (Comet Lake+/AMD where needed).
    RhubReset,
    /// `_OSI` → XOSI rename + Windows-compatible XOSI method (laptops, I2C).
    Xosi,
    /// Enable GPIO controller for I2C touchpads (GPI0 _STA).
    Gpi0,
    /// Backlight PNLF device; `uid` per iGPU generation (14 SNB/IVB, 15 HSW/BDW, 16 SKL/KBL, 19 CFL+).
    Pnlf { uid: u32 },
    /// Disable unused uncore bridges on X79/X99.
    Unc,
    /// RTC0 with fixed IO ranges for HEDT boards.
    Rtc0Range,
    /// Processor objects for B550/A520 boards declaring CPUs as ACPI0007.
    Cpur,
    /// Fake IMEI for Sandy/Ivy Bridge on 7/6-series mismatched boards.
    Imei,
    /// SBUS / MCHC for SMBus.
    SbusMchc,
    /// Fake ambient light sensor ALS0.
    Als0,
}

impl SsdtKind {
    /// Canonical output file name ("SSDT-EC-USBX.aml"). `Plug` produces
    /// "SSDT-PLUG-ALT.aml" instead when the CPUs are ACPI0007 devices; the
    /// name actually used is [`GeneratedSsdt::file_name`].
    pub fn file_name(&self) -> &'static str {
        match self {
            SsdtKind::EcUsbx { .. } => "SSDT-EC-USBX.aml",
            SsdtKind::Plug => "SSDT-PLUG.aml",
            SsdtKind::Awac => "SSDT-AWAC.aml",
            SsdtKind::Pmc => "SSDT-PMC.aml",
            SsdtKind::RhubReset => "SSDT-RHUB.aml",
            SsdtKind::Xosi => "SSDT-XOSI.aml",
            SsdtKind::Gpi0 => "SSDT-GPI0.aml",
            SsdtKind::Pnlf { .. } => "SSDT-PNLF.aml",
            SsdtKind::Unc => "SSDT-UNC.aml",
            SsdtKind::Rtc0Range => "SSDT-RTC0-RANGE.aml",
            SsdtKind::Cpur => "SSDT-CPUR.aml",
            SsdtKind::Imei => "SSDT-IMEI.aml",
            SsdtKind::SbusMchc => "SSDT-SBUS-MCHC.aml",
            SsdtKind::Als0 => "SSDT-ALS0.aml",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedSsdt {
    pub file_name: String,
    pub aml: Vec<u8>,
    /// Equivalent ASL source, for display and support.
    pub dsl: String,
    /// ACPI renames this SSDT depends on (e.g. `_OSI` → `XOSI`, `EC` → `EC0`).
    pub patches: Vec<AcpiPatch>,
}

/// True when `err` means the machine does not need the SSDT at all.
pub fn is_not_needed(err: &AppError) -> bool {
    err.code == ERR_NOT_NEEDED
}

fn not_needed(kind: &SsdtKind, why: &str) -> AppError {
    let mut e = AppError::new(
        ERR_NOT_NEEDED,
        format!("{} is not needed: {why}", kind.file_name()),
    );
    e.severity = crate::error::Severity::Info;
    e.recoverable = true;
    e
}

fn insufficient(kind: &SsdtKind, why: &str) -> AppError {
    AppError::new(
        ERR_INSUFFICIENT,
        format!("Cannot build {} for this machine: {why}", kind.file_name()),
    )
    .recoverable()
    .with_suggestion("Use the prebuilt table, or dump the ACPI tables from the target machine.")
}

/// Generate one SSDT for this machine. Returns an error when the facts are
/// insufficient (caller then uses `fallback_prebuilt`).
pub fn generate(kind: &SsdtKind, facts: &AcpiFacts) -> Result<GeneratedSsdt, AppError> {
    generate_inner(
        kind,
        &Ctx {
            facts,
            tables: None,
        },
    )
}

/// Generate one SSDT from the machine's indexed ACPI tables; exact renames
/// are only possible this way.
pub fn generate_with_tables(
    kind: &SsdtKind,
    tables: &AcpiTables,
) -> Result<GeneratedSsdt, AppError> {
    generate_inner(
        kind,
        &Ctx {
            facts: tables.facts_ref(),
            tables: Some(tables),
        },
    )
}

fn generate_inner(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    match kind {
        SsdtKind::EcUsbx { laptop } => ec_usbx(kind, ctx, *laptop),
        SsdtKind::Plug => processors(kind, ctx, true),
        SsdtKind::Cpur => processors(kind, ctx, false),
        SsdtKind::Awac => awac(kind, ctx),
        SsdtKind::Pmc => pmc(kind, ctx),
        SsdtKind::RhubReset => rhub(kind, ctx),
        SsdtKind::Xosi => Ok(xosi(ctx)),
        SsdtKind::Gpi0 => gpi0(kind, ctx),
        SsdtKind::Pnlf { uid } => Ok(pnlf(ctx, *uid)),
        SsdtKind::Unc => unc(kind, ctx),
        SsdtKind::Rtc0Range => rtc0_range(kind, ctx),
        SsdtKind::Imei => imei(kind, ctx),
        SsdtKind::SbusMchc => sbus_mchc(kind, ctx),
        SsdtKind::Als0 => als0(kind, ctx),
    }
}

/// Prebuilt fallback (OpenCorePkg AcpiSamples or Dortania compiled) used when
/// no DSDT is available. None when no safe generic table exists.
///
/// Only tables that are harmless on unknown paths are listed: Dortania's
/// prebuilt EC, PMC, RHUB, RTC0-RANGE and IMEI tables and OpenCore's
/// SSDT-PLUG probe each candidate path with `CondRefOf`; Dortania's AWAC
/// only stores to `STAS` from module-level code; the OpenCore ALS0/UNC/PNLF
/// samples touch standard paths only. `Cpur` maps to Dortania's B550/A520
/// table, which the planner must only request on those boards. `Xosi` also
/// needs the rename from [`fallback_patches`]. GPI0 and SBUS-MCHC have no
/// safe generic form.
pub fn fallback_prebuilt(kind: &SsdtKind) -> Option<SsdtSource> {
    let oc = |f: &str| {
        Some(SsdtSource::OcSample {
            file: f.to_string(),
        })
    };
    let dortania = |f: &str| {
        Some(SsdtSource::Dortania {
            file: f.to_string(),
        })
    };
    match kind {
        SsdtKind::EcUsbx { laptop: false } => dortania("SSDT-EC-USBX-DESKTOP.aml"),
        SsdtKind::EcUsbx { laptop: true } => dortania("SSDT-EC-USBX-LAPTOP.aml"),
        SsdtKind::Plug => oc("SSDT-PLUG.aml"),
        SsdtKind::Awac => dortania("SSDT-AWAC.aml"),
        SsdtKind::Pmc => dortania("SSDT-PMC.aml"),
        SsdtKind::RhubReset => dortania("SSDT-RHUB.aml"),
        SsdtKind::Xosi => dortania("SSDT-XOSI.aml"),
        SsdtKind::Gpi0 => None,
        SsdtKind::Pnlf { .. } => oc("SSDT-PNLF.aml"),
        SsdtKind::Unc => oc("SSDT-UNC.aml"),
        SsdtKind::Rtc0Range => dortania("SSDT-RTC0-RANGE-HEDT.aml"),
        SsdtKind::Cpur => dortania("SSDT-CPUR.aml"),
        SsdtKind::Imei => dortania("SSDT-IMEI.aml"),
        SsdtKind::SbusMchc => None,
        SsdtKind::Als0 => oc("SSDT-ALS0.aml"),
    }
}

/// [`fallback_prebuilt`] for a machine whose CPUs are known (from the CPU
/// generation, e.g. Alder Lake and newer) to be ACPI0007 devices: `Plug`
/// maps to OpenCore's SSDT-PLUG-ALT, which declares `\_SB.CP00`-`CP3F`.
pub fn fallback_prebuilt_acpi0007(kind: &SsdtKind) -> Option<SsdtSource> {
    match kind {
        SsdtKind::Plug => Some(SsdtSource::OcSample {
            file: "SSDT-PLUG-ALT.aml".to_string(),
        }),
        other => fallback_prebuilt(other),
    }
}

/// ACPI patches a prebuilt fallback depends on (the `_OSI` → `XOSI` rename
/// for SSDT-XOSI).
pub fn fallback_patches(kind: &SsdtKind) -> Vec<AcpiPatch> {
    match kind {
        SsdtKind::Xosi => vec![global_patch(
            "_OSI to XOSI rename - requires SSDT-XOSI.aml",
            b"_OSI",
            b"XOSI",
            true,
        )],
        _ => Vec::new(),
    }
}

// ── Context and helpers ─────────────────────────────────────────────────────

struct Ctx<'a> {
    facts: &'a AcpiFacts,
    tables: Option<&'a AcpiTables>,
}

impl Ctx<'_> {
    fn path(&self, value: &Option<String>) -> Option<Vec<Seg>> {
        value.as_deref().and_then(parse_path)
    }

    fn lpc(&self) -> Option<Vec<Seg>> {
        self.path(&self.facts.lpc_bridge)
    }

    /// Object at `path` in the tables (real definitions only).
    fn object(&self, path: &[Seg]) -> Option<&NsObject> {
        self.tables.and_then(|t| t.ns().get(path))
    }

    fn exists(&self, path: &[Seg]) -> bool {
        self.object(path).is_some()
    }
}

const SB: Seg = *b"_SB_";
const PREDEFINED_SCOPES: [Seg; 5] = [*b"_SB_", *b"_PR_", *b"_GPE", *b"_SI_", *b"_TZ_"];

fn sp(path: &[Seg]) -> String {
    display_path(path)
}

fn child(path: &[Seg], seg: &[u8; 4]) -> Vec<Seg> {
    let mut p = path.to_vec();
    p.push(*seg);
    p
}

fn parent(path: &[Seg]) -> Vec<Seg> {
    path.split_last()
        .map(|(_, p)| p.to_vec())
        .unwrap_or_default()
}

fn seg_text(seg: &Seg) -> String {
    String::from_utf8_lossy(seg)
        .trim_end_matches('_')
        .to_string()
}

fn last_name(path: &[Seg]) -> String {
    path.last()
        .map(seg_text)
        .unwrap_or_else(|| "\\".to_string())
}

fn darwin() -> Term {
    Term::call("_OSI", vec![Term::str("Darwin")])
}

/// `Method (_STA) { If (_OSI ("Darwin")) { Return (a) } Else { Return (b) } }`
fn sta_method(on_darwin: Term, otherwise: Term) -> Term {
    Term::method(
        "_STA",
        0,
        vec![Term::if_else(
            darwin(),
            vec![Term::ret(on_darwin)],
            vec![Term::ret(otherwise)],
        )],
    )
}

fn sta_enable() -> Term {
    sta_method(Term::int(0x0F), Term::int(0))
}

fn sta_disable() -> Term {
    sta_method(Term::int(0), Term::int(0x0F))
}

/// `_DSM` returning a property package, with the standard function 0 probe.
fn dsm(properties: Vec<Term>) -> Term {
    Term::method(
        "_DSM",
        4,
        vec![
            Term::if_then(
                Term::lnot(Term::Arg(2)),
                vec![Term::ret(Term::Buffer(vec![0x03]))],
            ),
            Term::ret(Term::Package(properties)),
        ],
    )
}

/// `External` for a scope target unless it is a predefined root scope.
fn scope_external(path: &[Seg], out: &mut Vec<Term>) {
    if path.is_empty() || (path.len() == 1 && PREDEFINED_SCOPES.contains(&path[0])) {
        return;
    }
    push_external(out, &sp(path), ObjType::Device);
}

fn push_external(out: &mut Vec<Term>, path: &str, kind: ObjType) {
    let exists = out
        .iter()
        .any(|t| matches!(t, Term::External { path: p, .. } if p == path));
    if !exists {
        out.push(Term::external(path, kind));
    }
}

fn sta_type(obj: &NsObject) -> ObjType {
    if matches!(obj.kind, NsKind::Method { .. }) {
        ObjType::Method
    } else {
        ObjType::Int
    }
}

/// Reference to a renamed `XSTA` (method call or integer read; same AML).
fn xsta_ref(dev: &[Seg]) -> Term {
    Term::path(&format!("{}.XSTA", sp(dev)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn global_patch(comment: &str, find: &[u8; 4], replace: &[u8; 4], enabled: bool) -> AcpiPatch {
    AcpiPatch {
        comment: comment.to_string(),
        find: hex(find),
        replace: hex(replace),
        table_signature: None,
        oem_table_id: None,
        count: 0,
        enabled,
    }
}

/// Replace occurrences like OpenCore does (left to right, non-overlapping).
fn apply_patch(data: &mut [u8], find: &[u8], replace: &[u8], count: u32) {
    if find.is_empty() || find.len() != replace.len() || data.len() < find.len() {
        return;
    }
    let mut done = 0u32;
    let mut i = 0;
    while i + find.len() <= data.len() {
        if &data[i..i + find.len()] == find {
            data[i..i + find.len()].copy_from_slice(replace);
            done += 1;
            if count != 0 && done >= count {
                return;
            }
            i += find.len();
        } else {
            i += 1;
        }
    }
}

const MAX_PAD: usize = 64;

/// Bytes that other patches this module emits may rewrite before a
/// table-specific rename is applied: the names renamed in every table
/// (`EC`, `_OSI`, `PNLF`, `OSI?`) and every other `_STA`. A find pattern
/// that avoids them still matches whichever other patches the config
/// combines it with. `core` is the position of the name being renamed.
fn volatile_mask(data: &[u8], core: usize) -> Vec<bool> {
    let mut mask = vec![false; data.len()];
    for (w, seg) in data.windows(4).enumerate() {
        let volatile = matches!(seg, b"EC__" | b"_OSI" | b"PNLF" | b"_STA")
            || (seg.starts_with(b"OSI") && super::parse::is_name_char(seg[3]));
        if volatile && w != core {
            mask[w..w + 4].fill(true);
        }
    }
    mask
}

/// Smallest (left, right) padding around `data[pos..pos+len]` that occurs
/// exactly once in `data` and never in `others` (left-only, right-only and
/// alternating growth are tried; the shortest wins, left first on ties).
/// Padding never grows into a byte marked in `avoid`.
fn shortest_unique(
    data: &[u8],
    pos: usize,
    len: usize,
    others: &[Vec<u8>],
    avoid: Option<&[bool]>,
) -> Option<(usize, usize)> {
    let blocked = |i: usize| avoid.is_some_and(|m| m.get(i).copied().unwrap_or(false));
    let core = data.get(pos..pos + len)?;
    let base: Vec<usize> = find_all(data, core);
    let base_others: Vec<Vec<usize>> = others.iter().map(|o| find_all(o, core)).collect();
    let mut best: Option<(usize, usize)> = None;
    for mode in 0..3 {
        let mut cands = base.clone();
        let mut other_cands = base_others.clone();
        let (mut l, mut r) = (0usize, 0usize);
        let found = loop {
            if cands.len() == 1 && other_cands.iter().all(Vec::is_empty) {
                break true;
            }
            if l + r >= MAX_PAD {
                break false;
            }
            let can_left = pos > aml::HEADER_LEN + l && !blocked(pos - l - 1);
            let can_right = pos + len + r < data.len() && !blocked(pos + len + r);
            let go_right = match mode {
                0 => !can_left,
                1 => can_right,
                _ => (r <= l && can_right) || !can_left,
            };
            if go_right && can_right {
                let b = data[pos + len + r];
                cands.retain(|&p| data.get(p + len + r) == Some(&b));
                for (o, list) in others.iter().zip(other_cands.iter_mut()) {
                    list.retain(|&p| o.get(p + len + r) == Some(&b));
                }
                r += 1;
            } else if can_left {
                let b = data[pos - l - 1];
                cands.retain(|&p| p > l && data[p - l - 1] == b);
                for (o, list) in others.iter().zip(other_cands.iter_mut()) {
                    list.retain(|&p| p > l && o.get(p - l - 1) == Some(&b));
                }
                l += 1;
            } else {
                break false;
            }
        };
        if found && best.is_none_or(|(bl, br)| l + r < bl + br) {
            best = Some((l, r));
        }
    }
    best
}

/// ACPI patches for one SSDT; table-specific renames are computed on the
/// tables as modified by the patches before them.
struct Patches<'a> {
    tables: Option<&'a AcpiTables>,
    list: Vec<AcpiPatch>,
}

impl<'a> Patches<'a> {
    fn new(tables: Option<&'a AcpiTables>) -> Self {
        Self {
            tables,
            list: Vec::new(),
        }
    }

    fn global(&mut self, comment: &str, find: &[u8; 4], replace: &[u8; 4]) {
        self.list.push(global_patch(comment, find, replace, true));
    }

    fn patched(&self, idx: usize) -> Option<Vec<u8>> {
        let table = self.tables?.table(idx)?;
        let mut data = table.data.clone();
        for p in &self.list {
            let sig_ok = p
                .table_signature
                .as_deref()
                .is_none_or(|s| s == table.signature);
            let id_ok = p
                .oem_table_id
                .as_deref()
                .is_none_or(|s| s == table.oem_table_id);
            if sig_ok && id_ok {
                if let (Some(f), Some(r)) = (unhex(&p.find), unhex(&p.replace)) {
                    apply_patch(&mut data, &f, &r, p.count);
                }
            }
        }
        Some(data)
    }

    /// Rename the final NameSeg of `obj` (e.g. `_STA` → `XSTA`) with a find
    /// pattern unique in its table. `enabled: false` emits it disabled.
    fn rename(&mut self, comment: &str, obj: &NsObject, to: &[u8; 4], enabled: bool) -> bool {
        let Some(tables) = self.tables else {
            return false;
        };
        let Some(table) = tables.table(obj.table) else {
            return false;
        };
        let Some(data) = self.patched(obj.table) else {
            return false;
        };
        let pos = obj.name_offset;
        if data
            .get(pos..pos + 4)
            .map(|s| s != obj.path.last().map(|s| s.as_slice()).unwrap_or(&[]))
            .unwrap_or(true)
        {
            return false;
        }
        let others: Vec<Vec<u8>> = (0..tables.tables().len())
            .filter(|&i| {
                i != obj.table
                    && tables
                        .table(i)
                        .is_some_and(|t| t.signature == table.signature)
            })
            .filter_map(|i| self.patched(i))
            .collect();
        let avoid = volatile_mask(&data, pos);
        let unique = shortest_unique(&data, pos, 4, &others, Some(&avoid))
            .or_else(|| shortest_unique(&data, pos, 4, &others, None));
        let Some((l, r)) = unique else {
            tracing::warn!(object = %sp(&obj.path), "no unique ACPI patch pattern found");
            return false;
        };
        let find = data[pos - l..pos + 4 + r].to_vec();
        let mut replace = find.clone();
        replace[l..l + 4].copy_from_slice(to);
        self.list.push(AcpiPatch {
            comment: comment.to_string(),
            find: hex(&find),
            replace: hex(&replace),
            table_signature: Some(table.signature.clone()),
            oem_table_id: if table.signature == "DSDT" {
                None
            } else {
                table.exact_oem_table_id().map(str::to_string)
            },
            count: 1,
            enabled,
        });
        true
    }
}

struct Build {
    file_name: String,
    table_id: &'static str,
    revision: u32,
    summary: Vec<String>,
    terms: Vec<Term>,
    patches: Vec<AcpiPatch>,
}

impl Build {
    fn new(file_name: &str, table_id: &'static str, revision: u32, summary: &str) -> Self {
        Self {
            file_name: file_name.to_string(),
            table_id,
            revision,
            summary: summary.lines().map(str::to_string).collect(),
            terms: Vec::new(),
            patches: Vec::new(),
        }
    }

    fn finish(self) -> GeneratedSsdt {
        let mut comment = self.summary.join("\n");
        comment.push_str("\nBuilt from this machine's ACPI tables by OpCore-OneClick.");
        if !self.patches.is_empty() {
            comment.push_str("\n\nRequires these ACPI patches (config.plist ACPI > Patch):");
            for p in &self.patches {
                let scope = match (&p.table_signature, &p.oem_table_id) {
                    (Some(s), Some(id)) => format!(", {s} \"{id}\""),
                    (Some(s), None) => format!(", {s}"),
                    _ => String::new(),
                };
                comment.push_str(&format!(
                    "\n  - {}{}: {} -> {}{scope}",
                    p.comment,
                    if p.enabled { "" } else { " (disabled)" },
                    p.find,
                    p.replace
                ));
            }
        }
        let aml = aml::encode_table(OEM_ID, self.table_id, self.revision, &self.terms);
        let dsl = aml::asl_table(OEM_ID, self.table_id, self.revision, &comment, &self.terms);
        GeneratedSsdt {
            file_name: self.file_name,
            aml,
            dsl,
            patches: self.patches,
        }
    }
}

/// `Device` under `parent` unless `parent.name` exists: unconditional with
/// tables (callers check), `If (!CondRefOf (...))` from facts.
fn guarded(ctx: &Ctx, target: &[Seg], externals: &mut Vec<Term>, body: Vec<Term>) -> Vec<Term> {
    if ctx.tables.is_some() {
        return body;
    }
    push_external(externals, &sp(target), ObjType::Device);
    vec![Term::if_then(
        Term::lnot(Term::cond_ref_of(&sp(target))),
        body,
    )]
}

/// Free name `base` (or `base` with a hex suffix) under `parent`.
fn unique_name(
    ctx: &Ctx,
    parent_path: &[Seg],
    base: &str,
    start: Option<u32>,
    used: &[Seg],
) -> Option<Seg> {
    let mut n = start;
    for _ in 0..0x1000 {
        let name = match n {
            None => aml::name_seg(base),
            Some(v) => {
                let suffix = format!("{v:X}");
                let keep = 4usize.saturating_sub(suffix.len());
                aml::name_seg(&format!("{}{suffix}", &base[..keep.min(base.len())]))
            }
        };
        let taken = ctx
            .tables
            .is_some_and(|t| !t.ns().all(&child(parent_path, &name)).is_empty())
            || used.contains(&name);
        if !taken {
            return Some(name);
        }
        n = Some(n.map_or(0, |v| v + 1));
    }
    None
}

// ── EC + USBX ───────────────────────────────────────────────────────────────

struct Ec<'a> {
    path: Vec<Seg>,
    has_sta: bool,
    sta: Option<&'a NsObject>,
    valid: bool,
}

fn ec_usbx(kind: &SsdtKind, ctx: &Ctx, laptop: bool) -> Result<GeneratedSsdt, AppError> {
    let mut b = Build::new(
        kind.file_name(),
        "SsdtEC",
        0x1000,
        if laptop {
            "SSDT-EC-USBX (laptop): keeps the real embedded controller, adds a\nfake EC only when none is named EC, and the USBX power properties."
        } else {
            "SSDT-EC-USBX (desktop): disables the real embedded controller in macOS,\nadds a fake EC device and the USBX power properties."
        },
    );
    let mut patches = Patches::new(ctx.tables);
    let mut ext = Vec::new();
    let mut body = Vec::new();

    let ecs: Vec<Ec> = match ctx.tables {
        Some(t) => t
            .embedded_controllers()
            .into_iter()
            .map(|(o, valid)| {
                let sta = t.sta(&o.path);
                Ec {
                    path: o.path.clone(),
                    has_sta: sta.is_some(),
                    sta,
                    valid,
                }
            })
            .collect(),
        None => ctx
            .path(&ctx.facts.ec_path)
            .map(|p| Ec {
                path: p,
                has_sta: ctx.facts.ec_has_sta,
                sta: None,
                valid: true,
            })
            .into_iter()
            .collect(),
    };
    let named_ec = ecs.iter().any(|e| e.path.last() == Some(b"EC__"));
    let lpc = ctx.lpc();

    // Where the fake EC goes, and whether a real EC named EC must become EC0.
    let mut fake_parent = lpc.clone().unwrap_or_else(|| vec![SB]);
    let mut rename_ec = false;
    if !laptop && named_ec {
        let ec0_free = ecs
            .iter()
            .filter(|e| e.path.last() == Some(b"EC__"))
            .all(|e| {
                ctx.tables
                    .is_some_and(|t| t.ns().all(&child(&parent(&e.path), b"EC0_")).is_empty())
            });
        if ctx.tables.is_some() && ec0_free {
            rename_ec = true;
        } else {
            fake_parent = vec![SB];
        }
    }
    if rename_ec {
        patches.global(
            "EC to EC0 - must come before any EC _STA to XSTA renames",
            b"EC__",
            b"EC0_",
        );
    }
    if lpc.as_ref() == Some(&fake_parent) {
        let real_ec_there = ecs
            .iter()
            .any(|e| parent(&e.path) == fake_parent && e.path.last() == Some(b"EC__"));
        if real_ec_there && !rename_ec {
            fake_parent = vec![SB];
        }
    }

    if !laptop {
        for ec in ecs.iter().filter(|e| e.valid) {
            let mut path = ec.path.clone();
            if rename_ec && path.last() == Some(b"EC__") {
                if let Some(last) = path.last_mut() {
                    *last = *b"EC0_";
                }
            }
            match (ec.sta, ec.has_sta) {
                (Some(sta), _) => {
                    let comment = format!("{} _STA to XSTA rename", last_name(&path));
                    if patches.rename(&comment, sta, b"XSTA", true) {
                        push_external(&mut ext, &sp(&path), ObjType::Device);
                        push_external(&mut ext, &format!("{}.XSTA", sp(&path)), sta_type(sta));
                        body.push(Term::scope(
                            &sp(&path),
                            vec![sta_method(Term::int(0), xsta_ref(&path))],
                        ));
                    } else {
                        body.push(Term::Comment(format!(
                            "{} keeps its own _STA (no unique rename found).",
                            sp(&path)
                        )));
                    }
                }
                (None, true) => body.push(Term::Comment(format!(
                    "{} has its own _STA; it stays enabled without an _STA to XSTA rename.",
                    sp(&path)
                ))),
                (None, false) => {
                    push_external(&mut ext, &sp(&path), ObjType::Device);
                    body.push(Term::scope(&sp(&path), vec![sta_disable()]));
                }
            }
        }
    } else if let Some(t) = ctx.tables {
        // Laptops keep the real EC; offer a disabled _STA override when its
        // _STA does not report it present (SSDTTime behaviour).
        for ec in ecs.iter().filter(|e| e.valid) {
            let Some(sta) = ec.sta else { continue };
            if !t.sta_needs_forcing(sta) {
                continue;
            }
            let comment = format!(
                "{} _STA to XSTA rename (enable only if the EC is not detected)",
                last_name(&ec.path)
            );
            if patches.rename(&comment, sta, b"XSTA", false) {
                let p = sp(&ec.path);
                push_external(&mut ext, &p, ObjType::Device);
                push_external(&mut ext, &format!("{p}._STA"), sta_type(sta));
                push_external(&mut ext, &format!("{p}.XSTA"), sta_type(sta));
                body.push(Term::if_then(
                    Term::and(
                        Term::cond_ref_of(&format!("{p}.XSTA")),
                        Term::lnot(Term::cond_ref_of(&format!("{p}._STA"))),
                    ),
                    vec![Term::scope(
                        &p,
                        vec![sta_method(Term::int(0x0F), xsta_ref(&ec.path))],
                    )],
                ));
            }
        }
    }

    if !(laptop && named_ec) {
        let target = child(&fake_parent, b"EC__");
        let exists_already =
            ctx.tables.is_some_and(|t| !t.ns().all(&target).is_empty()) && !rename_ec;
        if !exists_already {
            scope_external(&fake_parent, &mut ext);
            let device = Term::device(
                "EC",
                vec![Term::name("_HID", Term::str("ACID0001")), sta_enable()],
            );
            let scope = Term::scope(&sp(&fake_parent), vec![device]);
            body.extend(guarded(ctx, &target, &mut ext, vec![scope]));
        }
    }

    let usbx_exists = ctx
        .tables
        .is_some_and(|t| t.devices_named(b"USBX").into_iter().next().is_some());
    if !usbx_exists {
        let props = USBX_PROPERTIES
            .iter()
            .flat_map(|(k, v)| [Term::str(k), Term::int(*v)])
            .collect::<Vec<_>>();
        body.push(Term::scope(
            "\\_SB",
            vec![Term::device(
                "USBX",
                vec![Term::name("_ADR", Term::int(0)), dsm(props), sta_enable()],
            )],
        ));
    }
    if !body.iter().any(|t| !matches!(t, Term::Comment(_))) {
        return Err(not_needed(
            kind,
            "the embedded controller and USBX devices already exist",
        ));
    }

    b.terms = ext;
    b.terms.extend(body);
    b.patches = patches.list;
    Ok(b.finish())
}

// ── PLUG / PLUG-ALT / CPUR ──────────────────────────────────────────────────

fn processors(kind: &SsdtKind, ctx: &Ctx, plugin: bool) -> Result<GeneratedSsdt, AppError> {
    let (cpus, acpi0007): (CpuList, bool) = match ctx.tables {
        Some(t) => t.cpus(),
        None => {
            let list: CpuList = ctx
                .facts
                .cpu_paths
                .iter()
                .filter_map(|p| parse_path(p))
                .enumerate()
                .map(|(i, p)| (p, Some(i as u64)))
                .collect();
            (list, ctx.facts.cpu_uses_acpi0007)
        }
    };
    if cpus.is_empty() {
        return Err(insufficient(
            kind,
            "no processor objects or ACPI0007 devices are known",
        ));
    }
    if !acpi0007 {
        if !plugin {
            return Err(not_needed(
                kind,
                "the CPUs are already declared as Processor objects",
            ));
        }
        // A second _DSM would make the table fail to load (Apple firmware
        // already sets plugin-type this way).
        if ctx.exists(&child(&cpus[0].0, b"_DSM")) {
            return Err(not_needed(
                kind,
                "the first CPU object already has a _DSM method",
            ));
        }
        return Ok(plug_processor(&cpus[0].0));
    }
    processor_redefinition(kind, ctx, &cpus, plugin)
}

fn plug_processor(cpu: &[Seg]) -> GeneratedSsdt {
    let mut b = Build::new(
        "SSDT-PLUG.aml",
        "CpuPlug",
        0x3000,
        &format!(
            "SSDT-PLUG: plugin-type = 1 on the first CPU object ({}) for XCPM.",
            sp(cpu)
        ),
    );
    let cpu_path = sp(cpu);
    b.terms = vec![
        Term::external(&cpu_path, ObjType::Processor),
        Term::scope(
            &cpu_path,
            vec![Term::method(
                "_DSM",
                4,
                vec![Term::if_else(
                    darwin(),
                    vec![
                        Term::if_then(
                            Term::lnot(Term::Arg(2)),
                            vec![Term::ret(Term::Buffer(vec![0x03]))],
                        ),
                        Term::ret(Term::Package(vec![Term::str("plugin-type"), Term::int(1)])),
                    ],
                    vec![Term::ret(Term::Buffer(vec![0x00]))],
                )],
            )],
        ),
    ];
    b.finish()
}

fn processor_redefinition(
    kind: &SsdtKind,
    ctx: &Ctx,
    cpus: &[(Vec<Seg>, Option<u64>)],
    plugin: bool,
) -> Result<GeneratedSsdt, AppError> {
    let parent_path: Vec<Seg> = match cpus[0].0.first() {
        Some(first) if cpus[0].0.len() > 1 => vec![*first],
        _ => Vec::new(),
    };
    let valid: Vec<(&Vec<Seg>, u8)> = cpus
        .iter()
        .filter_map(|(p, uid)| uid.and_then(|u| u8::try_from(u).ok()).map(|u| (p, u)))
        .collect();
    if valid.is_empty() {
        return Err(insufficient(
            kind,
            "the ACPI0007 devices have no usable _UID",
        ));
    }
    let known: Vec<&Vec<Seg>> = cpus.iter().map(|(p, _)| p).collect();
    let taken = |name: &Seg| {
        let path = child(&parent_path, name);
        known.iter().any(|k| **k == path)
            || ctx.tables.is_some_and(|t| !t.ns().all(&path).is_empty())
    };
    let mut processors = Vec::new();
    for (i, (path, uid)) in valid.iter().enumerate() {
        let suffix = format!("{i:X}");
        let name = CPU_NAME_SCHEMES.iter().find_map(|scheme| {
            let keep = 4usize.saturating_sub(suffix.len());
            let seg = aml::name_seg(&format!("{}{suffix}", &scheme[..keep]));
            (!taken(&seg)).then_some(seg)
        });
        let Some(name) = name else {
            return Err(insufficient(kind, "no free name for the processor objects"));
        };
        let mut pbody = vec![
            Term::Comment(sp(path)),
            Term::name("_HID", Term::str("ACPI0007")),
            Term::name("_UID", Term::int(u64::from(*uid))),
            sta_enable(),
        ];
        if plugin && i == 0 {
            pbody.push(dsm(vec![Term::str("plugin-type"), Term::int(1)]));
        }
        processors.push(Term::Processor {
            name: seg_text(&name),
            id: *uid,
            pblk: 0x0000_0510,
            pblk_len: 0x06,
            body: pbody,
        });
    }
    let parent_text = if parent_path.is_empty() {
        "\\".to_string()
    } else {
        sp(&parent_path)
    };
    let (file, table_id, summary) = if plugin {
        (
            "SSDT-PLUG-ALT.aml",
            "CpuPlugA",
            format!(
                "SSDT-PLUG-ALT: the CPUs are ACPI0007 devices ({} ...), so legacy\nProcessor objects are declared for macOS, with plugin-type = 1 on the first.",
                sp(&cpus[0].0)
            ),
        )
    } else {
        (
            "SSDT-CPUR.aml",
            "CPUR",
            format!(
                "SSDT-CPUR: the CPUs are ACPI0007 devices ({} ...), so legacy\nProcessor objects are declared for macOS.",
                sp(&cpus[0].0)
            ),
        )
    };
    let mut b = Build::new(file, table_id, 0x3000, &summary);
    scope_external(&parent_path, &mut b.terms);
    b.terms.push(Term::scope(&parent_text, processors));
    Ok(b.finish())
}

// ── AWAC / RTC ──────────────────────────────────────────────────────────────

fn fake_rtc(ctx: &Ctx, lpc: &[Seg], ext: &mut Vec<Term>) -> Vec<Term> {
    let name = unique_name(ctx, lpc, "RTC0", None, &[]).unwrap_or(*b"RTC0");
    let device = Term::device(
        &seg_text(&name),
        vec![
            Term::name("_HID", Term::EisaId("PNP0B00".into())),
            Term::name(
                "_CRS",
                Term::Resources(vec![
                    Resource::Io {
                        min: 0x70,
                        max: 0x70,
                        align: 0x01,
                        len: 0x08,
                    },
                    Resource::IrqNoFlags { mask: 1 << 8 },
                ]),
            ),
            sta_enable(),
        ],
    );
    scope_external(lpc, ext);
    let scope = Term::scope(&sp(lpc), vec![device]);
    guarded(ctx, &child(lpc, &name), ext, vec![scope])
}

fn set_stas(ctx: &Ctx, stas: &[Seg], ext: &mut Vec<Term>) -> Term {
    push_external(ext, &sp(stas), ObjType::Int);
    let store = Term::if_then(
        darwin(),
        vec![Term::store(Term::int(1), Term::path(&sp(stas)))],
    );
    // A second \_INI would make the whole table fail to load, so when one
    // exists (or the tables are not known) STAS is set from module-level
    // code, as in Dortania's prebuilt SSDT-AWAC.
    let root_ini_may_exist = ctx.tables.is_none_or(|t| t.exists(&[*b"_INI"]));
    if root_ini_may_exist {
        Term::scope("\\_SB", vec![store])
    } else {
        Term::scope("\\", vec![Term::method("_INI", 0, vec![store])])
    }
}

fn awac(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let mut b = Build::new(
        kind.file_name(),
        "RTCAWAC",
        0,
        "SSDT-AWAC: macOS needs the legacy RTC (PNP0B00) instead of the AWAC\nclock (ACPI000E): switch STAS to RTC mode, or override the device _STA,\nor add a fake RTC0.",
    );
    let mut ext = Vec::new();
    let mut body = Vec::new();
    let mut patches = Patches::new(ctx.tables);
    let lpc = ctx.lpc();

    match ctx.tables {
        Some(t) => {
            let awac = t.devices_with_id(&["ACPI000E"]).into_iter().next();
            let rtc = t.devices_with_id(&["PNP0B00"]).into_iter().next();
            let stas = t.stas_path();
            let uses_stas = |dev: &NsObject| {
                stas.is_some()
                    && t.sta(&dev.path).is_some_and(|s| {
                        matches!(s.kind, NsKind::Method { .. }) && t.mentions(s, b"STAS")
                    })
            };
            let awac_var = awac.is_some_and(uses_stas);
            let rtc_var = rtc.is_some_and(uses_stas);
            let rtc_sta = rtc.and_then(|r| t.sta(&r.path));
            if awac.is_none() {
                match (rtc, rtc_sta) {
                    (Some(_), None) => {
                        return Err(not_needed(
                            kind,
                            "the legacy RTC is present and has no AWAC",
                        ))
                    }
                    (Some(_), Some(s)) if !rtc_var && !t.sta_needs_forcing(s) => {
                        return Err(not_needed(kind, "the legacy RTC is always enabled"))
                    }
                    _ => {}
                }
            }
            if let (true, Some(stas)) = (awac_var || rtc_var, &stas) {
                body.push(set_stas(ctx, stas, &mut ext));
            }
            for (dev, is_awac, has_var) in [(awac, true, awac_var), (rtc, false, rtc_var)] {
                let Some(dev) = dev else { continue };
                if has_var {
                    continue;
                }
                let (macos, original) = if is_awac { (0u64, 0x0Fu64) } else { (0x0F, 0) };
                let p = sp(&dev.path);
                match t.sta(&dev.path) {
                    // An RTC that always reports itself present needs no override.
                    Some(sta) if !is_awac && !t.sta_needs_forcing(sta) => {}
                    Some(sta) => {
                        let comment = format!("{} _STA to XSTA rename", last_name(&dev.path));
                        if !patches.rename(&comment, sta, b"XSTA", true) {
                            return Err(insufficient(
                                kind,
                                "no unique _STA rename for the clock device",
                            ));
                        }
                        push_external(&mut ext, &p, ObjType::Device);
                        push_external(&mut ext, &format!("{p}.XSTA"), sta_type(sta));
                        body.push(Term::scope(
                            &p,
                            vec![
                                Term::name("ZSTA", Term::int(original)),
                                Term::method(
                                    "_STA",
                                    0,
                                    vec![
                                        Term::if_then(darwin(), vec![Term::ret(Term::int(macos))]),
                                        Term::if_then(
                                            Term::cond_ref_of(&format!("{p}.XSTA")),
                                            vec![Term::store(
                                                xsta_ref(&dev.path),
                                                Term::path("ZSTA"),
                                            )],
                                        ),
                                        Term::ret(Term::path("ZSTA")),
                                    ],
                                ),
                            ],
                        ));
                    }
                    None if is_awac => {
                        push_external(&mut ext, &p, ObjType::Device);
                        body.push(Term::scope(&p, vec![sta_disable()]));
                    }
                    None => {}
                }
            }
            if rtc.is_none() {
                let lpc =
                    lpc.ok_or_else(|| insufficient(kind, "no LPC bridge for the fake RTC0"))?;
                body.extend(fake_rtc(ctx, &lpc, &mut ext));
            }
        }
        None => {
            let awac = ctx.path(&ctx.facts.awac_path);
            let rtc = ctx.path(&ctx.facts.rtc_path);
            match (awac, rtc) {
                (Some(_), _) if ctx.facts.awac_has_stas => {
                    body.push(set_stas(ctx, &[*b"STAS"], &mut ext));
                }
                (Some(awac), None) => {
                    let p = sp(&awac);
                    push_external(&mut ext, &p, ObjType::Device);
                    push_external(&mut ext, &format!("{p}._STA"), ObjType::Method);
                    body.push(Term::if_then(
                        Term::lnot(Term::cond_ref_of(&format!("{p}._STA"))),
                        vec![Term::scope(&p, vec![sta_disable()])],
                    ));
                    let lpc =
                        lpc.ok_or_else(|| insufficient(kind, "no LPC bridge for the fake RTC0"))?;
                    body.extend(fake_rtc(ctx, &lpc, &mut ext));
                }
                (Some(_), Some(_)) => {
                    return Err(insufficient(
                        kind,
                        "the AWAC has no STAS switch; the RTC _STA needs the ACPI tables",
                    ))
                }
                (None, None) => {
                    let lpc =
                        lpc.ok_or_else(|| insufficient(kind, "no LPC bridge for the fake RTC0"))?;
                    body.extend(fake_rtc(ctx, &lpc, &mut ext));
                }
                (None, Some(_)) => {
                    return Err(not_needed(
                        kind,
                        "the legacy RTC is present and has no AWAC",
                    ))
                }
            }
        }
    }
    b.terms = ext;
    b.terms.extend(body);
    b.patches = patches.list;
    Ok(b.finish())
}

// ── PMC ─────────────────────────────────────────────────────────────────────

fn pmc(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let lpc = ctx
        .lpc()
        .ok_or_else(|| insufficient(kind, "the LPC bridge path is unknown"))?;
    if let Some(t) = ctx.tables {
        if !t.devices_with_id(&["APP9876"]).is_empty() || t.exists(&child(&lpc, b"PMCR")) {
            return Err(not_needed(kind, "a PMCR device already exists"));
        }
    }
    let mut b = Build::new(
        kind.file_name(),
        "PMCR",
        0x1000,
        "SSDT-PMC: PMCR device (APP9876) that maps the PMC MMIO region\n0xFE000000-0xFE00FFFF so NVRAM works on true 300-series boards.",
    );
    let mut ext = Vec::new();
    scope_external(&lpc, &mut ext);
    let device = Term::device(
        "PMCR",
        vec![
            Term::name("_HID", Term::EisaId("APP9876".into())),
            sta_method(Term::int(0x0B), Term::int(0)),
            Term::name(
                "_CRS",
                Term::Resources(vec![Resource::Memory32Fixed {
                    writable: true,
                    base: 0xFE00_0000,
                    len: 0x0001_0000,
                }]),
            ),
        ],
    );
    let scope = Term::scope(&sp(&lpc), vec![device]);
    let body = guarded(ctx, &child(&lpc, b"PMCR"), &mut ext, vec![scope]);
    b.terms = ext;
    b.terms.extend(body);
    Ok(b.finish())
}

// ── RHUB reset ──────────────────────────────────────────────────────────────

fn rhub(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let mut b = Build::new(
        kind.file_name(),
        "UsbReset",
        0x1000,
        "SSDT-RHUB: disables the USB root hub devices in macOS so the ports are\nbuilt from the controllers themselves; controllers named XHC1/EHC1/EHC2/PXSX\nare replaced by new devices at the same address.",
    );
    let mut ext = Vec::new();
    let mut body = Vec::new();
    let mut patches = Patches::new(ctx.tables);

    match ctx.tables {
        Some(t) => {
            let mut hubs = t.root_hubs();
            hubs.sort_by_key(|h| {
                h.last_seg()
                    .and_then(|s| ROOT_HUB_NAMES.iter().position(|n| *n == s))
            });
            if hubs.is_empty() {
                return Err(not_needed(kind, "no RHUB/HUBN/URTH devices exist"));
            }
            let mut used: Vec<Seg> = Vec::new();
            let mut xhc_num = 2u32;
            let mut ehc_num = 1u32;
            for hub in hubs {
                let controller = parent(&hub.path);
                let Some(cname) = controller.last().copied() else {
                    continue;
                };
                if ILLEGAL_USB_NAMES.contains(&&cname) || used.contains(&cname) {
                    let grand = parent(&controller);
                    let ehci = cname.starts_with(b"EHC");
                    let (base, num) = if ehci {
                        ("EH01", &mut ehc_num)
                    } else {
                        ("XHCI", &mut xhc_num)
                    };
                    let Some(new_name) = unique_name(ctx, &grand, base, Some(*num), &used) else {
                        continue;
                    };
                    *num += 1;
                    if let Some(sta) = t.sta(&controller) {
                        let comment = format!("{} _STA to XSTA rename", last_name(&controller));
                        if !patches.rename(&comment, sta, b"XSTA", true) {
                            continue;
                        }
                    }
                    used.push(new_name);
                    push_external(&mut ext, &sp(&grand), ObjType::Device);
                    push_external(&mut ext, &sp(&controller), ObjType::Device);
                    body.push(Term::scope(&sp(&controller), vec![sta_disable()]));
                    let adr = t.adr(&controller).unwrap_or(0);
                    body.push(Term::scope(
                        &sp(&grand),
                        vec![Term::device(
                            &seg_text(&new_name),
                            vec![Term::name("_ADR", Term::int(adr)), sta_enable()],
                        )],
                    ));
                } else {
                    used.push(cname);
                    if let Some(sta) = t.sta(&hub.path) {
                        let comment = format!("{} _STA to XSTA rename", sp(&hub.path));
                        if !patches.rename(&comment, sta, b"XSTA", true) {
                            continue;
                        }
                    }
                    push_external(&mut ext, &sp(&hub.path), ObjType::Device);
                    body.push(Term::scope(&sp(&hub.path), vec![sta_disable()]));
                }
            }
        }
        None => {
            let hubs: Vec<Vec<Seg>> = ctx
                .facts
                .rhub_paths
                .iter()
                .filter_map(|p| parse_path(p))
                .collect();
            if hubs.is_empty() {
                return Err(insufficient(kind, "no root hub paths are known"));
            }
            for hub in hubs {
                let p = sp(&hub);
                push_external(&mut ext, &p, ObjType::Device);
                push_external(&mut ext, &format!("{p}._STA"), ObjType::Method);
                body.push(Term::if_then(
                    Term::lnot(Term::cond_ref_of(&format!("{p}._STA"))),
                    vec![Term::scope(&p, vec![sta_disable()])],
                ));
            }
        }
    }
    if body.is_empty() {
        return Err(insufficient(kind, "no root hub could be disabled"));
    }
    b.terms = ext;
    b.terms.extend(body);
    b.patches = patches.list;
    Ok(b.finish())
}

// ── XOSI ────────────────────────────────────────────────────────────────────

fn xosi(ctx: &Ctx) -> GeneratedSsdt {
    let mut patches = Patches::new(ctx.tables);
    let mut strings: Vec<&str> = OSI_STRINGS.to_vec();
    if let Some(t) = ctx.tables {
        let found = t.osi_strings();
        if let Some(last) = OSI_STRINGS
            .iter()
            .rposition(|s| found.iter().any(|f| f == s))
        {
            strings.truncate(last + 1);
        }
        for (from, to) in osi_prefixed_renames(t) {
            let name = String::from_utf8_lossy(&from).into_owned();
            let new_name = String::from_utf8_lossy(&to).into_owned();
            let comment =
                format!("{name} to {new_name} rename - must come before _OSI to XOSI rename");
            patches.global(&comment, &from, &to);
        }
    }
    patches.global(
        "_OSI to XOSI rename - requires SSDT-XOSI.aml",
        b"_OSI",
        b"XOSI",
    );
    let last = strings.last().copied().unwrap_or("Windows 2022");
    let mut b = Build::new(
        "SSDT-XOSI.aml",
        "XOSI",
        0x1000,
        &format!(
            "SSDT-XOSI: with the _OSI to XOSI rename, macOS answers the firmware's\nWindows checks as Windows (through \"{last}\"), which enables devices such\nas I2C touchpads that are only exposed to Windows."
        ),
    );
    b.terms = vec![Term::Method {
        name: "XOSI".into(),
        args: 1,
        serialized: false,
        body: vec![
            Term::store(
                Term::Package(strings.iter().map(|s| Term::str(s)).collect()),
                Term::Local(0),
            ),
            Term::if_else(
                darwin(),
                vec![Term::ret(Term::not_equal(
                    Term::Match {
                        package: Box::new(Term::Local(0)),
                        op1: MatchOp::Meq,
                        operand1: Box::new(Term::Arg(0)),
                        op2: MatchOp::Mtr,
                        operand2: Box::new(Term::int(0)),
                        start: Box::new(Term::int(0)),
                    },
                    Term::int(ONES),
                ))],
                vec![Term::ret(Term::call("_OSI", vec![Term::Arg(0)]))],
            ),
        ],
    }];
    b.patches = patches.list;
    b.finish()
}

/// Objects named `OSI?` (e.g. a Dell `OSID` method) put a false `_OSI`
/// into the AML wherever a NameSeg ending in `_` precedes them ("EC0_OSID"),
/// which the `_OSI` to `XOSI` rename would corrupt. Like SSDTTime they are
/// renamed to `XSI?` first (`YSI?`/`ZSI?` when that name is in use): always
/// for `OSID`, otherwise when such a false match exists.
fn osi_prefixed_renames(t: &AcpiTables) -> Vec<([u8; 4], [u8; 4])> {
    let mut names: Vec<Seg> = t
        .ns()
        .objects
        .iter()
        .filter_map(|o| o.last_seg().copied())
        .filter(|s| s.starts_with(b"OSI"))
        .collect();
    names.sort_by_key(|s| (s != b"OSID", *s));
    names.dedup();
    let blocks: Vec<&[u8]> = t
        .tables()
        .iter()
        .filter(|x| x.is_definition_block())
        .map(|x| x.data.as_slice())
        .collect();
    let occurs = |pattern: &[u8]| blocks.iter().any(|d| !find_all(d, pattern).is_empty());
    names
        .into_iter()
        .filter_map(|from| {
            let false_match = occurs(&[b'_', from[0], from[1], from[2], from[3]]);
            if !(from == *b"OSID" || false_match) {
                return None;
            }
            let to = b"XYZ"
                .iter()
                .copied()
                .map(|c| [c, from[1], from[2], from[3]])
                .find(|to| !occurs(to));
            if to.is_none() {
                tracing::warn!(name = %String::from_utf8_lossy(&from), "no free rename target, skipping");
            }
            to.map(|to| (from, to))
        })
        .collect()
}

// ── GPI0 ────────────────────────────────────────────────────────────────────

fn gpi0(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let mut b = Build::new(
        kind.file_name(),
        "GPI0",
        0,
        "SSDT-GPI0: enables the GPIO controller in macOS so VoodooI2C can use\nGPIO interrupts for the I2C touchpad.",
    );
    let mut ext = Vec::new();
    let mut body = Vec::new();
    let mut patches = Patches::new(ctx.tables);
    match ctx.tables {
        Some(t) => {
            let gpio = ctx
                .path(&ctx.facts.gpio_path)
                .ok_or_else(|| not_needed(kind, "there is no GPIO controller device"))?;
            let sta = t
                .sta(&gpio)
                .ok_or_else(|| not_needed(kind, "the GPIO controller has no _STA"))?;
            if !t.sta_needs_forcing(sta) {
                return Err(not_needed(kind, "the GPIO controller is always enabled"));
            }
            let comment = format!("{} _STA to XSTA rename", last_name(&gpio));
            if patches.rename(&comment, sta, b"XSTA", true) {
                let p = sp(&gpio);
                push_external(&mut ext, &p, ObjType::Device);
                push_external(&mut ext, &format!("{p}.XSTA"), sta_type(sta));
                body.push(Term::scope(
                    &p,
                    vec![sta_method(Term::int(0x0F), xsta_ref(&gpio))],
                ));
            } else if let Some(gpen) = t.resolve_seg(b"GPEN", &sta.path) {
                body.push(gpen_enable(&gpen, &mut ext, false));
            } else {
                return Err(insufficient(
                    kind,
                    "no unique _STA rename and no GPEN switch",
                ));
            }
        }
        None => body.push(gpen_enable(&[*b"GPEN"], &mut ext, true)),
    }
    b.terms = ext;
    b.terms.extend(body);
    b.patches = patches.list;
    Ok(b.finish())
}

/// `\GPEN = One` under Darwin (Dortania's SSDT-GPI0).
fn gpen_enable(gpen: &[Seg], ext: &mut Vec<Term>, guard: bool) -> Term {
    let p = sp(gpen);
    push_external(ext, &p, ObjType::FieldUnit);
    let set = Term::if_then(darwin(), vec![Term::store(Term::int(1), Term::path(&p))]);
    if guard {
        Term::if_then(Term::cond_ref_of(&p), vec![set])
    } else {
        set
    }
}

// ── PNLF ────────────────────────────────────────────────────────────────────

fn pnlf(ctx: &Ctx, uid: u32) -> GeneratedSsdt {
    let mut patches = Patches::new(ctx.tables);
    let existing = match ctx.tables {
        Some(t) => t
            .tables()
            .iter()
            .any(|tb| tb.is_definition_block() && !find_all(&tb.data, b"PNLF").is_empty()),
        None => ctx.facts.pnlf_exists,
    };
    if existing {
        patches.global("PNLF to XNLF rename", b"PNLF", b"XNLF");
    }
    if let Some(t) = ctx.tables {
        let defs = |pattern: &[u8]| {
            t.tables()
                .iter()
                .any(|tb| !find_all(&tb.data, pattern).is_empty())
        };
        if defs(&[0x08, b'N', b'B', b'C', b'F', 0x0A, 0x00]) {
            patches.list.push(AcpiPatch {
                comment: "NBCF 0x00 to 0x01 for BrightnessKeys.kext".into(),
                find: "084E4243460A00".into(),
                replace: "084E4243460A01".into(),
                table_signature: None,
                oem_table_id: None,
                count: 0,
                enabled: false,
            });
        }
        if defs(&[0x08, b'N', b'B', b'C', b'F', 0x00]) {
            patches.list.push(AcpiPatch {
                comment: "NBCF Zero to One for BrightnessKeys.kext".into(),
                find: "084E42434600".into(),
                replace: "084E42434601".into(),
                table_signature: None,
                oem_table_id: None,
                count: 0,
                enabled: false,
            });
        }
    }
    let mut b = Build::new(
        "SSDT-PNLF.aml",
        "PNLF",
        0,
        &format!(
            "SSDT-PNLF: backlight device for the internal panel. _UID {uid} selects the\nWhateverGreen backlight profile (14 SNB/IVB, 15 HSW/BDW, 16 SKL/KBL, 19 CFL+)."
        ),
    );
    b.terms = vec![Term::device(
        "PNLF",
        vec![
            Term::name("_HID", Term::EisaId("APP0002".into())),
            Term::name("_CID", Term::str("backlight")),
            Term::name("_UID", Term::int(u64::from(uid))),
            sta_method(Term::int(0x0B), Term::int(0)),
        ],
    )];
    b.patches = patches.list;
    b.finish()
}

// ── UNC ─────────────────────────────────────────────────────────────────────

fn unc(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let (unc0, prbm, has_ini) = match ctx.tables {
        Some(t) => {
            let unc0 = t
                .devices_named(b"UNC0")
                .into_iter()
                .next()
                .map(|o| o.path.clone())
                .ok_or_else(|| not_needed(kind, "there is no UNC0 uncore bridge device"))?;
            let prbm = t
                .resolve_seg(b"PRBM", &unc0)
                .ok_or_else(|| insufficient(kind, "the PRBM processor bit mask is missing"))?;
            let has_ini = t.exists(&child(&unc0, b"_INI"));
            (unc0, prbm, has_ini)
        }
        None => (vec![SB, *b"UNC0"], vec![*b"PRBM"], false),
    };
    let mut b = Build::new(
        kind.file_name(),
        "UNC",
        0,
        "SSDT-UNC: clears PRBM so the uncore PCI bridges of absent CPU sockets\nare hidden; IOPCIFamily panics on them since macOS 11.",
    );
    let set = Term::if_then(
        darwin(),
        vec![Term::store(Term::int(0), Term::path(&sp(&prbm)))],
    );
    b.terms = vec![
        Term::external(&sp(&unc0), ObjType::Device),
        Term::external(&sp(&prbm), ObjType::Int),
    ];
    if has_ini {
        b.terms.push(set);
    } else {
        b.terms.push(Term::scope(
            &sp(&unc0),
            vec![Term::method("_INI", 0, vec![set])],
        ));
    }
    Ok(b.finish())
}

// ── RTC0-RANGE (HEDT) ───────────────────────────────────────────────────────

/// Default ranges of the OpenCore sample (ASUS X299: 0x70-0x73, 0x74-0x77).
const RTC0_RANGE_DEFAULT: [Resource; 3] = [
    Resource::Io {
        min: 0x70,
        max: 0x70,
        align: 0x01,
        len: 0x04,
    },
    Resource::Io {
        min: 0x74,
        max: 0x74,
        align: 0x01,
        len: 0x04,
    },
    Resource::IrqNoFlags { mask: 1 << 8 },
];

/// The RTC `_CRS` with every gap between consecutive IO ranges closed by
/// growing the earlier range (SSDTTime's check). None when the template
/// cannot be decoded or has no IO range; `(resources, false)` when no range
/// needed growing.
fn rtc_closed_ranges(crs: &[u8]) -> Option<(Vec<Resource>, bool)> {
    let mut res = aml::decode_resources(crs)?;
    let ios: Vec<usize> = (0..res.len())
        .filter(|&i| matches!(res[i], Resource::Io { .. }))
        .collect();
    if ios.is_empty() {
        return None;
    }
    let mut changed = false;
    for pair in ios.windows(2) {
        let (
            Resource::Io {
                min: a, len: a_len, ..
            },
            Resource::Io { min: b, .. },
        ) = (res[pair[0]], res[pair[1]])
        else {
            continue;
        };
        if u32::from(b) > u32::from(a) + u32::from(a_len) {
            if let (Ok(grown), Resource::Io { len, .. }) = (u8::try_from(b - a), &mut res[pair[0]])
            {
                *len = grown;
                changed = true;
            }
        }
    }
    Some((res, changed))
}

fn rtc0_range(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let rtc = match ctx.tables {
        Some(t) => t
            .devices_with_id(&["PNP0B00"])
            .into_iter()
            .next()
            .map(|o| o.path.clone()),
        None => ctx.path(&ctx.facts.rtc_path),
    };
    let has_awac = match ctx.tables {
        Some(t) => !t.devices_with_id(&["ACPI000E"]).is_empty(),
        None => ctx.facts.awac_path.is_some(),
    };
    let parent_path = match &rtc {
        Some(r) if r.len() > 1 => parent(r),
        _ => ctx.lpc().ok_or_else(|| {
            insufficient(kind, "neither the RTC nor the LPC bridge path is known")
        })?,
    };
    let mut resources = RTC0_RANGE_DEFAULT.to_vec();
    let mut summary = "SSDT-RTC0-RANGE: new RTC device covering the whole 0x70-0x77 range;\nmany HEDT boards map only part of it and macOS 11+ can halt at boot.".to_string();
    if let (Some(t), Some(r)) = (ctx.tables, &rtc) {
        let crs = t.ns().child(r, b"_CRS").map(|o| &o.kind);
        if let Some(NsKind::Name(Value::Buffer(buf))) = crs {
            match rtc_closed_ranges(buf) {
                Some((fixed, true)) => {
                    resources = fixed;
                    summary = format!(
                        "SSDT-RTC0-RANGE: {} leaves gaps between its IO ranges; this RTC device\nmaps the same ranges with the gaps closed (macOS 11+ can halt at boot otherwise).",
                        sp(r)
                    );
                }
                Some((_, false)) => return Err(not_needed(kind, "the RTC IO ranges have no gaps")),
                None => {}
            }
        }
    }
    let used: Vec<Seg> = rtc.iter().filter_map(|r| r.last().copied()).collect();
    let name = unique_name(ctx, &parent_path, "RTC0", Some(0), &used)
        .ok_or_else(|| insufficient(kind, "no free RTC device name"))?;
    summary.push_str("\nUse it instead of SSDT-AWAC, not together with it.");
    let mut b = Build::new(kind.file_name(), "RtcRange", 0, &summary);
    let mut ext = Vec::new();
    let mut body = Vec::new();
    let mut patches = Patches::new(ctx.tables);
    // The original RTC must not stay enabled next to the new one. Like
    // Dortania's SSDT-RTC0-RANGE-HEDT it gets a disabling _STA when it has
    // none; an existing _STA is left alone only on AWAC boards where it
    // follows STAS (AWAC mode turns the RTC off).
    if let Some(r) = &rtc {
        let p = sp(r);
        match ctx.tables {
            Some(t) => match t.sta(r) {
                Some(sta) => {
                    let follows_stas = has_awac
                        && matches!(sta.kind, NsKind::Method { .. })
                        && t.mentions(sta, b"STAS");
                    if !follows_stas {
                        if !patches.rename(
                            &format!("{} _STA to XSTA rename", last_name(r)),
                            sta,
                            b"XSTA",
                            true,
                        ) {
                            return Err(insufficient(
                                kind,
                                "no unique _STA rename to disable the original RTC",
                            ));
                        }
                        push_external(&mut ext, &p, ObjType::Device);
                        body.push(Term::scope(&p, vec![sta_disable()]));
                    }
                }
                None => {
                    push_external(&mut ext, &p, ObjType::Device);
                    body.push(Term::scope(&p, vec![sta_disable()]));
                }
            },
            None => {
                push_external(&mut ext, &p, ObjType::Device);
                push_external(&mut ext, &format!("{p}._STA"), ObjType::Method);
                body.push(Term::if_then(
                    Term::lnot(Term::cond_ref_of(&format!("{p}._STA"))),
                    vec![Term::scope(&p, vec![sta_disable()])],
                ));
            }
        }
    }
    scope_external(&parent_path, &mut ext);
    let device = Term::device(
        &seg_text(&name),
        vec![
            Term::name("_HID", Term::EisaId("PNP0B00".into())),
            Term::name("_CRS", Term::Resources(resources)),
            sta_enable(),
        ],
    );
    let scope = Term::scope(&sp(&parent_path), vec![device]);
    body.extend(guarded(
        ctx,
        &child(&parent_path, &name),
        &mut ext,
        vec![scope],
    ));
    b.terms = ext;
    b.terms.extend(body);
    b.patches = patches.list;
    Ok(b.finish())
}

// ── IMEI ────────────────────────────────────────────────────────────────────

fn imei(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    if let Some(t) = ctx.tables {
        if !t.root_devices_at(0x0016_0000).is_empty() {
            return Err(not_needed(
                kind,
                "an IMEI/HECI device already exists at 0x00160000",
            ));
        }
    }
    let igpu = ctx.path(&ctx.facts.igpu_path).filter(|p| p.len() > 1);
    let parent_path = igpu
        .map(|p| parent(&p))
        .or_else(|| ctx.path(&ctx.facts.pci_root))
        .ok_or_else(|| insufficient(kind, "the PCI root path is unknown"))?;
    if ctx.exists(&child(&parent_path, b"IMEI")) {
        return Err(not_needed(kind, "an IMEI device already exists"));
    }
    let mut b = Build::new(
        kind.file_name(),
        "IMEI",
        0,
        "SSDT-IMEI: IMEI device at 0x00160000 so the Intel ME can be given a\nmatching device-id (Sandy Bridge CPU on 7-series, Ivy Bridge CPU on 6-series).",
    );
    let p = sp(&parent_path);
    let mut ext = vec![Term::external(&p, ObjType::Device)];
    let device = Term::device(
        "IMEI",
        vec![Term::name("_ADR", Term::int(0x0016_0000)), sta_enable()],
    );
    let scope = Term::scope(&p, vec![device]);
    let body = if ctx.tables.is_some() {
        vec![scope]
    } else {
        let names = [b"IMEI", b"HECI", b"MEI_"];
        let mut cond: Option<Term> = None;
        for n in names {
            let target = sp(&child(&parent_path, n));
            push_external(&mut ext, &target, ObjType::Device);
            let t = Term::lnot(Term::cond_ref_of(&target));
            cond = Some(match cond {
                None => t,
                Some(c) => Term::and(c, t),
            });
        }
        vec![Term::if_then(cond.unwrap_or(Term::int(1)), vec![scope])]
    };
    b.terms = ext;
    b.terms.extend(body);
    Ok(b.finish())
}

// ── SBUS / MCHC ─────────────────────────────────────────────────────────────

fn sbus_mchc(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let bus = ctx
        .path(&ctx.facts.smbus_path)
        .filter(|p| p.len() > 1)
        .ok_or_else(|| insufficient(kind, "the SMBus controller path is unknown"))?;
    let bus_parent = parent(&bus);
    let mchc = child(&bus_parent, b"MCHC");
    let bus0 = child(&bus, b"BUS0");
    if ctx.exists(&mchc) && ctx.exists(&bus0) {
        return Err(not_needed(kind, "MCHC and BUS0 already exist"));
    }
    let mut b = Build::new(
        kind.file_name(),
        "SBUSMCHC",
        0,
        "SSDT-SBUS-MCHC: memory controller (MCHC) and SMBus (BUS0) devices for\nAppleSMBusController / AppleSMBusPCI.",
    );
    let mut ext = vec![
        Term::external(&sp(&bus_parent), ObjType::Device),
        Term::external(&sp(&mchc), ObjType::Device),
        Term::external(&sp(&bus), ObjType::Device),
    ];
    let mut body = Vec::new();
    if !ctx.exists(&mchc) {
        let device = Term::device("MCHC", vec![Term::name("_ADR", Term::int(0)), sta_enable()]);
        body.push(Term::if_then(
            Term::lnot(Term::cond_ref_of(&sp(&mchc))),
            vec![Term::scope(&sp(&bus_parent), vec![device])],
        ));
    }
    if !ctx.exists(&bus0) {
        let device = Term::device(
            &sp(&bus0),
            vec![
                Term::name("_CID", Term::str("smbus")),
                Term::name("_ADR", Term::int(0)),
                sta_enable(),
            ],
        );
        if ctx.tables.is_some() {
            body.push(device);
        } else {
            push_external(&mut ext, &sp(&bus0), ObjType::Device);
            body.push(Term::if_then(
                Term::lnot(Term::cond_ref_of(&sp(&bus0))),
                vec![device],
            ));
        }
    }
    b.terms = ext;
    b.terms.extend(body);
    Ok(b.finish())
}

// ── ALS0 ────────────────────────────────────────────────────────────────────

fn als0(kind: &SsdtKind, ctx: &Ctx) -> Result<GeneratedSsdt, AppError> {
    let mut b = Build::new(
        kind.file_name(),
        "ALS0",
        0,
        "SSDT-ALS0: ambient light sensor device; macOS 10.15+ needs one for the\nbacklight to work (SMCLightSensor reports it).",
    );
    if let Some(t) = ctx.tables {
        if let Some(als) = t.devices_with_id(&["ACPI0008"]).into_iter().next() {
            let Some(sta) = t.sta(&als.path) else {
                return Err(not_needed(kind, "a real ambient light sensor exists"));
            };
            if !t.sta_needs_forcing(sta) {
                return Err(not_needed(kind, "a real ambient light sensor exists"));
            }
            let mut patches = Patches::new(ctx.tables);
            if !patches.rename(
                &format!("{} _STA to XSTA rename", last_name(&als.path)),
                sta,
                b"XSTA",
                true,
            ) {
                return Err(insufficient(
                    kind,
                    "no unique _STA rename for the light sensor",
                ));
            }
            let p = sp(&als.path);
            b.summary = vec![format!(
                "SSDT-ALS0: enables the existing ambient light sensor {p} in macOS."
            )];
            b.terms = vec![
                Term::external(&p, ObjType::Device),
                Term::external(&format!("{p}._STA"), sta_type(sta)),
                Term::external(&format!("{p}.XSTA"), sta_type(sta)),
                Term::if_then(
                    Term::and(
                        Term::cond_ref_of(&format!("{p}.XSTA")),
                        Term::lnot(Term::cond_ref_of(&format!("{p}._STA"))),
                    ),
                    vec![Term::scope(
                        &p,
                        vec![sta_method(Term::int(0x0F), xsta_ref(&als.path))],
                    )],
                ),
            ];
            b.patches = patches.list;
            return Ok(b.finish());
        }
    }
    let target = vec![SB, *b"ALS0"];
    let mut ext = Vec::new();
    let device = Term::device(
        "ALS0",
        vec![
            Term::name("_HID", Term::str("ACPI0008")),
            Term::name("_CID", Term::str("smc-als")),
            Term::name("_ALI", Term::int(0x012C)),
            Term::name(
                "_ALR",
                Term::Package(vec![Term::Package(vec![
                    Term::int(0x64),
                    Term::int(0x012C),
                ])]),
            ),
            sta_enable(),
        ],
    );
    let body = guarded(
        ctx,
        &target,
        &mut ext,
        vec![Term::scope("\\_SB", vec![device])],
    );
    b.terms = ext;
    b.terms.extend(body);
    Ok(b.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_canonical() {
        assert_eq!(
            SsdtKind::EcUsbx { laptop: true }.file_name(),
            "SSDT-EC-USBX.aml"
        );
        assert_eq!(
            SsdtKind::EcUsbx { laptop: false }.file_name(),
            "SSDT-EC-USBX.aml"
        );
        assert_eq!(SsdtKind::Rtc0Range.file_name(), "SSDT-RTC0-RANGE.aml");
        assert_eq!(SsdtKind::SbusMchc.file_name(), "SSDT-SBUS-MCHC.aml");
    }

    #[test]
    fn fallbacks() {
        assert!(matches!(
            fallback_prebuilt(&SsdtKind::EcUsbx { laptop: false }),
            Some(SsdtSource::Dortania { file }) if file == "SSDT-EC-USBX-DESKTOP.aml"
        ));
        assert!(
            matches!(fallback_prebuilt(&SsdtKind::Plug), Some(SsdtSource::OcSample { file }) if file == "SSDT-PLUG.aml")
        );
        assert!(fallback_prebuilt(&SsdtKind::Gpi0).is_none());
        assert!(fallback_prebuilt(&SsdtKind::SbusMchc).is_none());
        let p = fallback_patches(&SsdtKind::Xosi);
        assert_eq!(p.len(), 1);
        assert_eq!(
            (p[0].find.as_str(), p[0].replace.as_str()),
            ("5F4F5349", "584F5349")
        );
        assert!(fallback_patches(&SsdtKind::Plug).is_empty());
        assert!(matches!(
            fallback_prebuilt_acpi0007(&SsdtKind::Plug),
            Some(SsdtSource::OcSample { file }) if file == "SSDT-PLUG-ALT.aml"
        ));
        assert!(matches!(
            fallback_prebuilt_acpi0007(&SsdtKind::Awac),
            Some(SsdtSource::Dortania { file }) if file == "SSDT-AWAC.aml"
        ));
    }

    #[test]
    fn rtc_gaps_are_closed() {
        let crs = [
            0x47, 0x01, 0x70, 0x00, 0x70, 0x00, 0x01, 0x02, 0x47, 0x01, 0x74, 0x00, 0x74, 0x00,
            0x01, 0x04, 0x22, 0x00, 0x01, 0x79, 0x00,
        ];
        let (fixed, changed) = rtc_closed_ranges(&crs).expect("decodes");
        assert!(changed);
        assert_eq!(
            fixed[0],
            Resource::Io {
                min: 0x70,
                max: 0x70,
                align: 1,
                len: 4
            }
        );
        assert_eq!(
            fixed[1],
            Resource::Io {
                min: 0x74,
                max: 0x74,
                align: 1,
                len: 4
            }
        );
        assert_eq!(fixed[2], Resource::IrqNoFlags { mask: 1 << 8 });
        let (_, changed) =
            rtc_closed_ranges(&[0x47, 0x01, 0x70, 0x00, 0x70, 0x00, 0x01, 0x08, 0x79, 0x00])
                .expect("ok");
        assert!(!changed);
        assert!(
            rtc_closed_ranges(&[0x22, 0x00, 0x01, 0x79, 0x00]).is_none(),
            "no IO range"
        );
    }

    #[test]
    fn unique_padding() {
        let mut data = vec![0u8; aml::HEADER_LEN];
        data.extend_from_slice(b"AA_STABB_STACC_STA");
        let pos = aml::HEADER_LEN + 8; // the second _STA, between "BB" and "CC"
        let (l, r) = shortest_unique(&data, pos, 4, &[], None).expect("unique");
        let pattern = &data[pos - l..pos + 4 + r];
        assert_eq!(find_all(&data, pattern), vec![pos - l]);
        assert_eq!(l + r, 1);
        // A pattern present in another table of the same signature is rejected.
        let other = data.clone();
        assert!(shortest_unique(&data, pos, 4, &[other], None).is_none());
    }

    #[test]
    fn padding_avoids_names_other_patches_rewrite() {
        // Both _STA have "AA" on the left; one byte on the right ("E" of
        // "EC__") would make the first unique, but an EC to EC0 rename
        // would then break the pattern, so it grows to the left instead.
        let mut data = vec![0u8; aml::HEADER_LEN];
        data.extend_from_slice(b"1AA_STAEC__2AA_STAQQ__");
        let pos = aml::HEADER_LEN + 3;
        assert_eq!(shortest_unique(&data, pos, 4, &[], None), Some((0, 1)));
        let avoid = volatile_mask(&data, pos);
        assert_eq!(
            shortest_unique(&data, pos, 4, &[], Some(&avoid)),
            Some((3, 0))
        );
        // The other _STA is protected the same way.
        let second = aml::HEADER_LEN + 14;
        assert!(avoid[second] && avoid[second + 3] && !avoid[pos]);
        // When only protected bytes can make it unique, nothing is found.
        let mut tight = vec![0u8; aml::HEADER_LEN];
        tight.extend_from_slice(b"_STAEC___STAQQ__");
        let core = aml::HEADER_LEN;
        let avoid = volatile_mask(&tight, core);
        assert_eq!(shortest_unique(&tight, core, 4, &[], Some(&avoid)), None);
        assert_eq!(shortest_unique(&tight, core, 4, &[], None), Some((0, 1)));
    }

    #[test]
    fn patch_application() {
        let mut d = b"EC__xEC__yEC__".to_vec();
        apply_patch(&mut d, b"EC__", b"EC0_", 2);
        assert_eq!(&d, b"EC0_xEC0_yEC__");
        assert_eq!(unhex("5F4F5349"), Some(b"_OSI".to_vec()));
        assert_eq!(unhex("5F4"), None);
    }
}
