//! DSDT/SSDT analysis without iasl: walk AML scopes (Scope/Device/Processor/
//! Method/ThermalZone/PowerResource with PkgLength) to recover the namespace,
//! then collect the facts SSDT generation needs.
//!
//! Lookups follow SSDTTime: the LPC bridge is the parent of the first
//! `PNP0C09` EC, else a device named LPCB/LPC0/LPC/SBRG/PX40, else the device
//! at `_ADR 0x001F0000` (Intel) / `0x00140003` (AMD) without a `_HID`; the
//! SMBus controller is looked up at 0x001F0004, 0x001F0003 (when the HDA is
//! not there) and 0x00140000 (AMD).
//!
//! The main PCI root is the host bridge holding the LPC bridge (HEDT boards
//! also declare their uncore bridges as `PNP0A03`). The iGPU is the device at
//! `0x00020000` under it that has display methods or a usual iGPU name and no
//! `_PRT`: on HEDT that address is a PCIe root port, on AMD a dummy bridge.

use std::path::Path;

use crate::domain::model::AcpiFacts;
use crate::error::AppError;

use super::aml::{self, HEADER_LEN};
use super::namespace::{Namespace, NsKind, NsObject};
use super::parse::{find_all, string_literals, Decoder, NamePath, Seg, Value};

const MAX_TABLE_FILES: usize = 256;
const MAX_TABLE_SIZE: u64 = 16 * 1024 * 1024;
/// Upper bound for all tables read from one directory together.
const MAX_TOTAL_SIZE: u64 = 64 * 1024 * 1024;

pub(crate) const PCI_ROOT_IDS: [&str; 2] = ["PNP0A08", "PNP0A03"];
pub(crate) const LPC_NAMES: [&[u8; 4]; 5] = [b"LPCB", b"LPC0", b"LPC_", b"SBRG", b"PX40"];
pub(crate) const ROOT_HUB_NAMES: [&[u8; 4]; 3] = [b"RHUB", b"HUBN", b"URTH"];
/// GPIO controller ids (Intel PCH/SoC generations as matched by the Linux
/// pinctrl drivers, and AMD FCH).
const GPIO_IDS: [&str; 29] = [
    "INT33C7", "INT3437", "INT344B", "INT3450", "INT3451", "INT3452", "INT3453", "INT3455",
    "INT345D", "INT34BB", "INT34C3", "INT34C4", "INT34C5", "INT34C6", "INT34C8", "INT34D1",
    "INTC1020", "INTC1055", "INTC1056", "INTC1057", "INTC105E", "INTC1082", "INTC1083", "INTC1084",
    "INTC1085", "AMDI0030", "AMD0030", "AMDI0031", "AMDI0033",
];
/// Names firmware gives the integrated GPU at 0x00020000.
const IGPU_NAMES: [&[u8; 4]; 10] = [
    b"GFX0", b"IGPU", b"VID_", b"VID0", b"VID1", b"GFX_", b"VGA_", b"IGFX", b"IGD_", b"IGD0",
];
/// Controller name prefixes of USB 1.1/2.0 host controllers.
const LEGACY_USB_PREFIXES: [&[u8]; 4] = [b"EHC", b"EH0", b"UHC", b"OHC"];

/// CPU objects: (path, processor id or `_UID`).
pub(crate) type CpuList = Vec<(Vec<Seg>, Option<u64>)>;

/// One ACPI table image (header included).
#[derive(Debug, Clone)]
pub struct AcpiTable {
    pub signature: String,
    pub revision: u8,
    pub oem_id: String,
    /// OEM table id with trailing NUL padding removed (spaces are kept).
    pub oem_table_id: String,
    pub oem_revision: u32,
    pub data: Vec<u8>,
}

fn header_text(bytes: &[u8]) -> String {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    bytes[..end]
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

impl AcpiTable {
    /// Validate the header of a raw table image. Bytes past the declared
    /// length are dropped.
    pub fn parse(mut bytes: Vec<u8>) -> Result<Self, AppError> {
        if bytes.len() < HEADER_LEN {
            return Err(AppError::new(
                "ACPI_TABLE_INVALID",
                "ACPI table is shorter than its header",
            ));
        }
        let declared = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if declared < HEADER_LEN || declared > bytes.len() {
            return Err(AppError::new(
                "ACPI_TABLE_INVALID",
                format!(
                    "ACPI table length {declared} does not match the {} bytes read",
                    bytes.len()
                ),
            ));
        }
        if !bytes[..4]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            return Err(AppError::new(
                "ACPI_TABLE_INVALID",
                "ACPI table signature is not ASCII",
            ));
        }
        bytes.truncate(declared);
        Ok(Self {
            signature: header_text(&bytes[0..4]),
            revision: bytes[8],
            oem_id: header_text(&bytes[10..16]),
            oem_table_id: header_text(&bytes[16..24]),
            oem_revision: u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            data: bytes,
        })
    }

    pub fn length(&self) -> u32 {
        self.data.len() as u32
    }

    pub fn checksum_ok(&self) -> bool {
        aml::byte_sum(&self.data) == 0
    }

    /// DSDT or SSDT (tables that carry AML).
    pub fn is_definition_block(&self) -> bool {
        matches!(self.signature.as_str(), "DSDT" | "SSDT")
    }

    /// The OEM table id when it can be matched exactly by an ACPI patch
    /// (printable ASCII, NUL padded).
    pub fn exact_oem_table_id(&self) -> Option<&str> {
        let raw = &self.data[16..24];
        let end = raw.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        (end > 0 && raw[..end].iter().all(|b| (0x20..0x7F).contains(b)))
            .then_some(self.oem_table_id.as_str())
    }
}

/// A DSDT plus SSDTs with their recovered namespace.
#[derive(Debug)]
pub struct AcpiTables {
    tables: Vec<AcpiTable>,
    ns: Namespace,
    /// Device objects (index into the namespace), one per path, in
    /// definition order, with their `_HID`/`_CID` ids.
    devices: Vec<(usize, Vec<String>)>,
    /// PCI host bridges, main one first.
    pci_roots: Vec<Vec<Seg>>,
    facts: AcpiFacts,
}

/// "\\_SB_.PCI0" style segments → "\\_SB.PCI0".
pub(crate) fn display_path(segs: &[Seg]) -> String {
    let joined: Vec<String> = segs
        .iter()
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    aml::asl_path(&format!("\\{}", joined.join(".")))
}

/// Absolute ACPI path text ("\\_SB.PCI0.LPCB", "_SB_.PCI0") → padded segments.
pub(crate) fn parse_path(text: &str) -> Option<Vec<Seg>> {
    let t = text.trim();
    let body = t.strip_prefix('\\').unwrap_or(t);
    if body.is_empty() || !aml::is_valid_path(body) || body.starts_with('^') {
        return None;
    }
    Some(body.split('.').map(aml::name_seg).collect())
}

/// Read at most `limit + 1` bytes, so an oversized file is detected
/// without loading all of it.
fn read_limited(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    std::fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)?;
    Ok(out)
}

fn natural_key(name: &str) -> Vec<(u8, u64, String)> {
    // Split into digit / non-digit runs so "SSDT10" sorts after "SSDT9".
    let mut out = Vec::new();
    let mut digits = String::new();
    let mut text = String::new();
    for c in name.to_ascii_uppercase().chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                out.push((0, 0, std::mem::take(&mut text)));
            }
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push((1, digits.parse().unwrap_or(u64::MAX), String::new()));
                digits.clear();
            }
            text.push(c);
        }
    }
    if !text.is_empty() {
        out.push((0, 0, text));
    }
    if !digits.is_empty() {
        out.push((1, digits.parse().unwrap_or(u64::MAX), String::new()));
    }
    out
}

impl AcpiTables {
    /// Index the tables (DSDT first, then the others in the given order).
    /// Extra DSDTs and byte-identical copies of a table are dropped.
    pub fn new(tables: Vec<AcpiTable>) -> Self {
        let (mut ordered, rest): (Vec<AcpiTable>, Vec<AcpiTable>) =
            tables.into_iter().partition(|t| t.signature == "DSDT");
        ordered.truncate(1);
        for t in rest {
            if t.signature != "DSDT" && !ordered.iter().any(|o| o.data == t.data) {
                ordered.push(t);
            }
        }
        for t in ordered.iter().filter(|t| !t.checksum_ok()) {
            tracing::warn!(signature = %t.signature, oem_table_id = %t.oem_table_id, "ACPI table checksum mismatch");
        }
        let ns = Namespace::build(
            ordered
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_definition_block())
                .map(|(i, t)| (i, t.data.as_slice())),
        );
        let mut out = Self {
            tables: ordered,
            ns,
            devices: Vec::new(),
            pci_roots: Vec::new(),
            facts: AcpiFacts::default(),
        };
        out.devices = out.index_devices();
        out.pci_roots = out.rank_pci_roots();
        out.facts = out.collect_facts();
        out
    }

    fn index_devices(&self) -> Vec<(usize, Vec<String>)> {
        let mut seen = std::collections::HashSet::new();
        self.ns
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.kind == NsKind::Device && seen.insert(o.path.as_slice()))
            .map(|(i, o)| (i, self.ids(&o.path)))
            .collect()
    }

    /// PCI host bridges: the one holding the LPC bridge first, then `\_SB`
    /// children, `PNP0A08` before `PNP0A03`, structured finds before
    /// heuristic ones.
    fn rank_pci_roots(&self) -> Vec<Vec<Seg>> {
        let lpc = self.lpc_bridge();
        let mut roots: Vec<(&NsObject, bool)> = self
            .devices
            .iter()
            .filter(|(_, ids)| ids.iter().any(|i| PCI_ROOT_IDS.contains(&i.as_str())))
            .filter_map(|(i, ids)| {
                Some((
                    self.ns.objects.get(*i)?,
                    ids.first().is_some_and(|i| i == "PNP0A08"),
                ))
            })
            .collect();
        roots.sort_by_key(|(o, pcie)| {
            let holds_lpc = lpc
                .as_ref()
                .is_some_and(|l| l.len() > o.path.len() && l.starts_with(&o.path));
            (
                !holds_lpc,
                o.path.len() != 2 || o.path[0] != *b"_SB_",
                !pcie,
                o.heuristic,
            )
        });
        roots.into_iter().map(|(o, _)| o.path.clone()).collect()
    }

    /// Parse raw table images (DSDT and SSDTs; other tables are kept but not walked).
    pub fn from_images(images: Vec<Vec<u8>>) -> Result<Self, AppError> {
        let tables = images
            .into_iter()
            .map(AcpiTable::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(tables))
    }

    /// Load every DSDT/SSDT image found in `dir` (`DSDT.aml`, `SSDT*.aml`,
    /// `*.dat`, extension-less sysfs names). Requires a DSDT.
    pub fn load_dir(dir: &Path) -> Result<Self, AppError> {
        let entries = std::fs::read_dir(dir).map_err(|e| {
            AppError::new(
                "ACPI_DIR_UNREADABLE",
                format!("Cannot read ACPI tables in {}: {e}", dir.display()),
            )
        })?;
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || meta.len() < HEADER_LEN as u64 || meta.len() > MAX_TABLE_SIZE {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(
                ext.as_deref(),
                None | Some("aml") | Some("dat") | Some("bin")
            ) {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            files.push((name, path));
            if files.len() >= MAX_TABLE_FILES {
                break;
            }
        }
        files.sort_by_key(|(name, _)| natural_key(name));
        let mut tables = Vec::new();
        let mut total = 0u64;
        for (name, path) in files {
            let bytes = match read_limited(&path, MAX_TABLE_SIZE) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(file = %name, error = %e, "skipping unreadable ACPI table");
                    continue;
                }
            };
            if !(bytes.starts_with(b"DSDT") || bytes.starts_with(b"SSDT")) {
                continue;
            }
            let size = bytes.len() as u64;
            if size > MAX_TABLE_SIZE || total + size > MAX_TOTAL_SIZE {
                tracing::warn!(file = %name, "skipping ACPI table: size limit reached");
                continue;
            }
            total += size;
            match AcpiTable::parse(bytes) {
                Ok(t) => tables.push(t),
                Err(e) => tracing::warn!(file = %name, error = %e, "skipping invalid ACPI table"),
            }
        }
        if !tables.iter().any(|t| t.signature == "DSDT") {
            return Err(AppError::new(
                "ACPI_DSDT_MISSING",
                format!("No DSDT found in {}", dir.display()),
            )
            .with_suggestion("Dump the ACPI tables again from the target machine."));
        }
        Ok(Self::new(tables))
    }

    pub fn tables(&self) -> &[AcpiTable] {
        &self.tables
    }

    pub fn dsdt(&self) -> Option<&AcpiTable> {
        self.tables.iter().find(|t| t.signature == "DSDT")
    }

    pub(crate) fn ns(&self) -> &Namespace {
        &self.ns
    }

    pub(crate) fn table(&self, idx: usize) -> Option<&AcpiTable> {
        self.tables.get(idx)
    }

    // ── Queries ─────────────────────────────────────────────────────────────

    /// Real Device objects in definition order, one per path.
    pub(crate) fn devices(&self) -> Vec<&NsObject> {
        self.devices
            .iter()
            .filter_map(|(i, _)| self.ns.objects.get(*i))
            .collect()
    }

    pub(crate) fn devices_named(&self, seg: &Seg) -> Vec<&NsObject> {
        self.devices()
            .into_iter()
            .filter(|o| o.last_seg() == Some(seg))
            .collect()
    }

    fn value_ids(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Int(v) if *v <= u64::from(u32::MAX) => out.push(aml::eisa_id_string(*v as u32)),
            Value::Str(s) => out.push(s.trim().to_ascii_uppercase()),
            Value::Package(items) => items.iter().for_each(|v| Self::value_ids(v, out)),
            _ => {}
        }
    }

    /// `_HID` and `_CID` ids of a device (EISA ids expanded, `_HID` methods
    /// contribute the id-like strings they return).
    pub(crate) fn ids(&self, dev: &[Seg]) -> Vec<String> {
        let mut out = Vec::new();
        for seg in [b"_HID", b"_CID"] {
            let Some(obj) = self.ns.child(dev, seg) else {
                continue;
            };
            match &obj.kind {
                NsKind::Name(v) => Self::value_ids(v, &mut out),
                NsKind::Method { .. } => out.extend(
                    string_literals(self.body(obj))
                        .into_iter()
                        .filter(|s| {
                            (4..=9).contains(&s.len())
                                && s.chars().all(|c| c.is_ascii_alphanumeric())
                        })
                        .map(|s| s.to_ascii_uppercase()),
                ),
                _ => {}
            }
        }
        out
    }

    pub(crate) fn devices_with_id(&self, ids: &[&str]) -> Vec<&NsObject> {
        self.devices
            .iter()
            .filter(|(_, own)| {
                own.iter()
                    .any(|i| ids.iter().any(|w| w.eq_ignore_ascii_case(i)))
            })
            .filter_map(|(i, _)| self.ns.objects.get(*i))
            .collect()
    }

    pub(crate) fn int_name(&self, dev: &[Seg], seg: &Seg) -> Option<u64> {
        match &self.ns.child(dev, seg)?.kind {
            NsKind::Name(Value::Int(v)) => Some(*v),
            _ => None,
        }
    }

    pub(crate) fn adr(&self, dev: &[Seg]) -> Option<u64> {
        self.int_name(dev, b"_ADR")
    }

    pub(crate) fn uid(&self, dev: &[Seg]) -> Option<u64> {
        self.int_name(dev, b"_UID")
    }

    /// The `_STA` of a device, as a Method or a Name.
    pub(crate) fn sta(&self, dev: &[Seg]) -> Option<&NsObject> {
        self.ns
            .child(dev, b"_STA")
            .filter(|o| matches!(o.kind, NsKind::Method { .. } | NsKind::Name(_)))
    }

    /// Body bytes of a Method/Device (empty for other objects).
    pub(crate) fn body(&self, obj: &NsObject) -> &[u8] {
        match (obj.body, self.tables.get(obj.table)) {
            (Some((s, e)), Some(t)) if s <= e && e <= t.data.len() => &t.data[s..e],
            _ => &[],
        }
    }

    /// Whether a method body references a NameSeg.
    pub(crate) fn mentions(&self, obj: &NsObject, seg: &[u8; 4]) -> bool {
        !find_all(self.body(obj), seg).is_empty()
    }

    /// Constant operands of the `Return`s in a method (None when the body
    /// cannot be decoded).
    pub(crate) fn method_returns(&self, obj: &NsObject) -> Option<Vec<Option<u64>>> {
        let (start, end) = obj.body?;
        let table = self.tables.get(obj.table)?;
        let mut out = Vec::new();
        Decoder::new(&table.data, &self.ns, &obj.path)
            .returns(start, end, &mut out)
            .ok()?;
        Some(out)
    }

    /// True when a device `_STA` does not unconditionally report 0x0F (the
    /// SSDTTime test for forcing a device on).
    pub(crate) fn sta_needs_forcing(&self, sta: &NsObject) -> bool {
        match &sta.kind {
            NsKind::Name(Value::Int(v)) => *v != 0x0F,
            NsKind::Name(_) => true,
            NsKind::Method { .. } => match self.method_returns(sta) {
                Some(rets) => rets.len() != 1 || rets[0] != Some(0x0F),
                None => true,
            },
            _ => false,
        }
    }

    /// Absolute path a lone NameSeg resolves to from `scope`.
    pub(crate) fn resolve_seg(&self, seg: &Seg, scope: &[Seg]) -> Option<Vec<Seg>> {
        let np = NamePath {
            root: false,
            parents: 0,
            segs: vec![*seg],
        };
        self.ns.resolve(&np, scope).filter(|p| self.ns.exists(p))
    }

    pub(crate) fn exists(&self, path: &[Seg]) -> bool {
        self.ns.exists(path)
    }

    fn pci_root(&self) -> Option<&Vec<Seg>> {
        self.pci_roots.first()
    }

    fn is_under_pci_root(&self, dev: &[Seg]) -> bool {
        dev.split_last()
            .is_some_and(|(_, parent)| self.pci_roots.iter().any(|r| r == parent))
    }

    /// The integrated GPU: at `0x00020000` under a PCI root (the main one
    /// first), with display methods or a usual iGPU name, and not a bridge.
    pub(crate) fn igpu(&self) -> Option<&NsObject> {
        let main = self.pci_root();
        let mut list: Vec<&NsObject> = self
            .root_devices_at(0x0002_0000)
            .into_iter()
            .filter(|o| {
                let has = |seg: &[u8; 4]| self.ns.child(&o.path, seg).is_some();
                let named = o.last_seg().is_some_and(|s| IGPU_NAMES.contains(&s));
                !has(b"_PRT") && (has(b"_DOD") || has(b"_DOS") || named)
            })
            .collect();
        list.sort_by_key(|o| o.path.split_last().map(|(_, p)| p) != main.map(Vec::as_slice));
        list.into_iter().next()
    }

    /// Devices at `_ADR adr` directly under a PCI root.
    pub(crate) fn root_devices_at(&self, adr: u64) -> Vec<&NsObject> {
        self.devices()
            .into_iter()
            .filter(|o| self.adr(&o.path) == Some(adr) && self.is_under_pci_root(&o.path))
            .collect()
    }

    /// Embedded controllers (`PNP0C09`): (device, has _HID+_CRS+_GPE).
    pub(crate) fn embedded_controllers(&self) -> Vec<(&NsObject, bool)> {
        self.devices_with_id(&["PNP0C09"])
            .into_iter()
            .map(|o| {
                let valid = [b"_HID", b"_CRS", b"_GPE"]
                    .iter()
                    .all(|s| self.ns.child(&o.path, s).is_some());
                (o, valid)
            })
            .collect()
    }

    pub(crate) fn lpc_bridge(&self) -> Option<Vec<Seg>> {
        for table in 0..self.tables.len() {
            let in_table = |o: &&NsObject| o.table == table;
            if let Some(ec) = self
                .devices_with_id(&["PNP0C09"])
                .into_iter()
                .find(in_table)
            {
                if let Some((_, parent)) = ec.path.split_last() {
                    if !parent.is_empty() {
                        return Some(parent.to_vec());
                    }
                }
            }
            for name in LPC_NAMES {
                if let Some(dev) = self.devices_named(name).into_iter().find(in_table) {
                    return Some(dev.path.clone());
                }
            }
            let by_adr = self.devices().into_iter().filter(in_table).find(|o| {
                matches!(self.adr(&o.path), Some(0x001F_0000 | 0x0014_0003))
                    && self.ns.child(&o.path, b"_HID").is_none()
                    && o.path.len() > 1
            });
            if let Some(dev) = by_adr {
                return Some(dev.path.clone());
            }
        }
        None
    }

    /// Processor objects sorted by processor id, else ACPI0007 devices sorted by
    /// `_UID`. Returns (path, uid or processor id, uses ACPI0007).
    pub(crate) fn cpus(&self) -> (CpuList, bool) {
        let mut seen = std::collections::HashSet::new();
        let mut procs: CpuList = self
            .ns
            .objects
            .iter()
            .filter_map(|o| match o.kind {
                NsKind::Processor { id } if seen.insert(o.path.clone()) => {
                    Some((o.path.clone(), Some(u64::from(id))))
                }
                _ => None,
            })
            .collect();
        if !procs.is_empty() {
            procs.sort_by_key(|(_, id)| *id);
            return (procs, false);
        }
        let mut devs: CpuList = self
            .devices_with_id(&["ACPI0007"])
            .into_iter()
            .map(|o| (o.path.clone(), self.uid(&o.path)))
            .collect();
        devs.sort_by_key(|(_, uid)| (uid.is_none(), *uid));
        let uses = !devs.is_empty();
        (devs, uses)
    }

    /// SMBus controller (SSDTTime order): (path, `_ADR`).
    pub(crate) fn smbus(&self) -> Option<(Vec<Seg>, u64)> {
        let at = |adr: u64, exclude: &[&str]| -> Option<Vec<Seg>> {
            self.devices().into_iter().find_map(|o| {
                if self.adr(&o.path) != Some(adr) || o.path.len() < 2 {
                    return None;
                }
                let name = String::from_utf8_lossy(o.last_seg()?).to_ascii_uppercase();
                (!exclude.iter().any(|x| name.contains(x))).then(|| o.path.clone())
            })
        };
        let dev_1f4 = at(0x001F_0004, &["XHC"]);
        let dev_1f3 = at(0x001F_0003, &["AZAL", "HDEF", "HDAS"]);
        let dev_1b = at(0x001B_0000, &["XHC"]);
        let dev_14 = at(0x0014_0000, &["XHC"]);
        match (dev_1f4, dev_1f3, dev_1b, dev_14) {
            (Some(p), Some(_), _, _) => Some((p, 0x001F_0004)),
            (None, Some(p), Some(_), _) => Some((p, 0x001F_0003)),
            (Some(p), None, _, _) => Some((p, 0x001F_0004)),
            (None, Some(p), None, _) => Some((p, 0x001F_0003)),
            (None, None, _, Some(p)) => Some((p, 0x0014_0000)),
            _ => None,
        }
    }

    /// Root hubs (RHUB/HUBN/URTH) under USB controllers.
    pub(crate) fn root_hubs(&self) -> Vec<&NsObject> {
        self.devices()
            .into_iter()
            .filter(|o| {
                o.last_seg().is_some_and(|s| ROOT_HUB_NAMES.contains(&s))
                    && o.path
                        .split_last()
                        .is_some_and(|(_, parent)| !parent.is_empty() && self.ns.exists(parent))
            })
            .collect()
    }

    /// USB 3 controllers: parents of `RHUB` devices that are not named like
    /// EHCI/UHCI/OHCI controllers, and `XHC*` devices at 0x00140000.
    fn xhci_controllers(&self) -> Vec<Vec<Seg>> {
        let mut out: Vec<Vec<Seg>> = Vec::new();
        for hub in self.root_hubs() {
            if hub.last_seg() != Some(b"RHUB") {
                continue;
            }
            if let Some((_, parent)) = hub.path.split_last() {
                let legacy = parent
                    .last()
                    .is_some_and(|s| LEGACY_USB_PREFIXES.iter().any(|p| s.starts_with(p)));
                if !legacy && !out.iter().any(|p| p == parent) {
                    out.push(parent.to_vec());
                }
            }
        }
        for dev in self.root_devices_at(0x0014_0000) {
            let named_xhc = dev.last_seg().is_some_and(|s| s.starts_with(b"XHC"));
            if named_xhc && !out.contains(&dev.path) {
                out.push(dev.path.clone());
            }
        }
        out
    }

    fn gpio(&self) -> Option<Vec<Seg>> {
        if let Some(dev) = self.devices_named(b"GPI0").into_iter().next() {
            return Some(dev.path.clone());
        }
        self.devices_with_id(&GPIO_IDS)
            .into_iter()
            .next()
            .map(|o| o.path.clone())
    }

    /// Windows `_OSI` strings ("Windows 2015", "Windows 2001 SP1") found as
    /// string literals in any table, whether passed to `_OSI` directly or
    /// through a firmware helper method; in order of first appearance.
    pub(crate) fn osi_strings(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in self.tables.iter().filter(|t| t.is_definition_block()) {
            for pos in find_all(&t.data, b"\x0DWindows 20") {
                let rest = &t.data[pos + 1..];
                let Some(nul) = rest.iter().take(32).position(|&b| b == 0) else {
                    continue;
                };
                let Ok(s) = std::str::from_utf8(&rest[..nul]) else {
                    continue;
                };
                let tail = s.as_bytes().get("Windows ".len()..).unwrap_or_default();
                let well_formed = tail.len() >= 4
                    && tail[..4].iter().all(u8::is_ascii_digit)
                    && tail[4..]
                        .iter()
                        .all(|b| b.is_ascii_alphanumeric() || *b == b'.' || *b == b' ');
                if well_formed && !out.iter().any(|o| o == s) {
                    out.push(s.to_string());
                }
            }
        }
        out
    }

    fn pnlf_exists(&self) -> bool {
        !self.devices_named(b"PNLF").is_empty() || !self.devices_with_id(&["APP0002"]).is_empty()
    }

    /// STAS (AWAC/RTC switch variable) referenced by the AWAC or RTC `_STA`.
    pub(crate) fn stas_path(&self) -> Option<Vec<Seg>> {
        let awac = self.devices_with_id(&["ACPI000E"]).into_iter().next();
        let rtc = self.devices_with_id(&["PNP0B00"]).into_iter().next();
        for dev in [awac, rtc].into_iter().flatten() {
            if let Some(sta) = self.sta(&dev.path) {
                if matches!(sta.kind, NsKind::Method { .. }) && self.mentions(sta, b"STAS") {
                    if let Some(p) = self.resolve_seg(b"STAS", &sta.path) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }

    /// The facts SSDT generation needs.
    pub fn facts(&self) -> AcpiFacts {
        self.facts.clone()
    }

    pub(crate) fn facts_ref(&self) -> &AcpiFacts {
        &self.facts
    }

    fn collect_facts(&self) -> AcpiFacts {
        let display = |p: &[Seg]| display_path(p);
        let ecs = self.embedded_controllers();
        let ec = ecs
            .iter()
            .find(|(_, valid)| *valid)
            .or_else(|| ecs.first())
            .map(|(o, _)| *o);
        let (cpus, acpi0007) = self.cpus();
        let awac = self.devices_with_id(&["ACPI000E"]).into_iter().next();
        let igpu = self.igpu();
        let dsdt = self.dsdt();
        AcpiFacts {
            pci_root: self.pci_root().map(|p| display(p)),
            lpc_bridge: self.lpc_bridge().map(|p| display(&p)),
            ec_path: ec.map(|o| display(&o.path)),
            ec_has_sta: ec.is_some_and(|o| self.sta(&o.path).is_some()),
            cpu_paths: cpus.iter().map(|(p, _)| display(p)).collect(),
            cpu_uses_acpi0007: acpi0007,
            awac_path: awac.map(|o| display(&o.path)),
            awac_has_stas: awac.is_some() && self.stas_path().is_some(),
            rtc_path: self
                .devices_with_id(&["PNP0B00"])
                .first()
                .map(|o| display(&o.path)),
            hpet_path: self
                .devices_with_id(&["PNP0103"])
                .first()
                .map(|o| display(&o.path)),
            igpu_path: igpu.map(|o| display(&o.path)),
            xhci_paths: self.xhci_controllers().iter().map(|p| display(p)).collect(),
            rhub_paths: self.root_hubs().iter().map(|o| display(&o.path)).collect(),
            gpio_path: self.gpio().map(|p| display(&p)),
            smbus_path: self.smbus().map(|(p, _)| display(&p)),
            pnlf_exists: self.pnlf_exists(),
            has_osi_windows: !self.osi_strings().is_empty(),
            dsdt_oem_table_id: dsdt.map(|t| t.oem_table_id.clone()),
            dsdt_length: dsdt.map(AcpiTable::length),
        }
    }
}

/// Load the tables in `dir` and index them (needed for exact renames).
pub fn load_tables(dir: &Path) -> Result<AcpiTables, AppError> {
    AcpiTables::load_dir(dir)
}

/// Parse `DSDT.aml` (+ `SSDT*.aml` for processor objects defined there) in
/// `dir` and return the extracted facts.
pub fn parse_tables(dir: &Path) -> Result<AcpiFacts, AppError> {
    Ok(AcpiTables::load_dir(dir)?.facts())
}

/// Parse a single DSDT image (header + AML).
pub fn parse_dsdt(bytes: &[u8]) -> Result<AcpiFacts, AppError> {
    let table = AcpiTable::parse(bytes.to_vec())?;
    if table.signature != "DSDT" {
        return Err(AppError::new(
            "ACPI_TABLE_INVALID",
            format!("Expected a DSDT, got {}", table.signature),
        ));
    }
    Ok(AcpiTables::new(vec![table]).facts())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stop_after_the_limit() {
        let dir = std::env::temp_dir().join(format!("oneclick-acpi-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("SSDT9.aml");
        std::fs::write(&file, vec![0x5Au8; 100]).expect("write");
        assert_eq!(read_limited(&file, 10).expect("read").len(), 11);
        assert_eq!(read_limited(&file, 1000).expect("read").len(), 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_files_sort_naturally() {
        let mut names = vec!["SSDT10.aml", "SSDT2.aml", "DSDT.aml", "SSDT1.aml"];
        names.sort_by_key(|n| natural_key(n));
        assert_eq!(
            names,
            vec!["DSDT.aml", "SSDT1.aml", "SSDT2.aml", "SSDT10.aml"]
        );
    }
}
