//! AML namespace walker. Recovers the object tree of DSDT/SSDT images by
//! following Scope/Device/Processor/ThermalZone/PowerResource and
//! If/Else/While bodies through their PkgLength, recording every named
//! definition with its absolute path and byte offsets. Method bodies are
//! not entered during the walk; they are analysed on demand afterwards.
//!
//! Method invocations are stepped over with the callee's argument count.
//! A call to a method defined further down cannot be sized on the first
//! walk, so [`Namespace::build`] walks the tables a second time with the
//! argument counts learnt from the first one when that happened.
//!
//! When a term list still cannot be decoded (vendor bugs), the rest of that
//! list is scanned heuristically for Device/Scope/Processor/Method/Name
//! patterns whose PkgLength fits inside the enclosing object, so one bad
//! statement never hides the rest of the table.

use std::cell::Cell;
use std::collections::HashMap;

use super::parse::{
    data_object, is_lead_char, name_path, pkg_bounds, pkg_length, valid_seg, Arity, Decoder,
    Malformed, NamePath, PResult, Seg, Value, MAX_DEPTH, OSI,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NsKind {
    Device,
    Processor { id: u8 },
    Method { args: u8, serialized: bool },
    Name(Value),
    ThermalZone,
    PowerResource,
    FieldUnit,
    OpRegion,
    Mutex,
    Event,
    Alias,
    BufferField,
    DataRegion,
    External { obj_type: u8, args: u8 },
}

#[derive(Debug, Clone)]
pub(crate) struct NsObject {
    pub path: Vec<Seg>,
    pub kind: NsKind,
    /// Index of the table that defines it.
    pub table: usize,
    /// Offset of the last NameSeg of the definition (rename target).
    pub name_offset: usize,
    /// Body range for Method/Device/Processor/ThermalZone/PowerResource.
    pub body: Option<(usize, usize)>,
    /// Found by the fallback scan rather than by structured decoding.
    pub heuristic: bool,
}

impl NsObject {
    pub fn is_external(&self) -> bool {
        matches!(self.kind, NsKind::External { .. })
    }

    pub fn last_seg(&self) -> Option<&Seg> {
        self.path.last()
    }
}

#[derive(Debug, Default)]
pub(crate) struct Namespace {
    pub objects: Vec<NsObject>,
    by_path: HashMap<Vec<Seg>, Vec<usize>>,
    by_parent: HashMap<Vec<Seg>, Vec<usize>>,
    /// Method argument counts from a previous walk (forward references).
    hints: HashMap<Vec<Seg>, u8>,
    /// A name was invoked before any definition of it was seen.
    unresolved: Cell<bool>,
}

impl Namespace {
    /// Walk every (table index, image) pair in order, twice when a method
    /// was called before its definition.
    pub fn build<'a>(tables: impl Iterator<Item = (usize, &'a [u8])> + Clone) -> Namespace {
        let mut first = Namespace::default();
        for (i, data) in tables.clone() {
            first.walk_table(i, data);
        }
        if !first.unresolved.get() {
            return first;
        }
        let hints = first.method_arities();
        let mut ns = Namespace {
            hints,
            ..Namespace::default()
        };
        for (i, data) in tables {
            ns.walk_table(i, data);
        }
        ns
    }

    fn method_arities(&self) -> HashMap<Vec<Seg>, u8> {
        let mut out = HashMap::new();
        for o in &self.objects {
            let args = match o.kind {
                NsKind::Method { args, .. } => args,
                NsKind::External { obj_type: 8, args } => args,
                _ => continue,
            };
            out.entry(o.path.clone()).or_insert(args);
        }
        out
    }

    /// Argument count of a method known only from the previous walk.
    fn hinted_args(&self, name: &NamePath, scope: &[Seg]) -> Option<u8> {
        if self.hints.is_empty() {
            return None;
        }
        if !name.is_simple() {
            return self.hints.get(&name.resolve(scope)?).copied();
        }
        (0..=scope.len()).rev().find_map(|i| {
            let mut p = scope[..i].to_vec();
            p.push(name.segs[0]);
            self.hints.get(&p).copied()
        })
    }

    fn add(&mut self, obj: NsObject) {
        let idx = self.objects.len();
        self.by_path.entry(obj.path.clone()).or_default().push(idx);
        if let Some((_, parent)) = obj.path.split_last() {
            self.by_parent.entry(parent.to_vec()).or_default().push(idx);
        }
        self.objects.push(obj);
    }

    /// Every definition of `path`, real definitions first.
    pub fn all(&self, path: &[Seg]) -> Vec<&NsObject> {
        let mut out: Vec<&NsObject> = self
            .by_path
            .get(path)
            .map(|ids| ids.iter().filter_map(|&i| self.objects.get(i)).collect())
            .unwrap_or_default();
        out.sort_by_key(|o| o.is_external());
        out
    }

    /// The first real (non-External) definition of `path`.
    pub fn get(&self, path: &[Seg]) -> Option<&NsObject> {
        self.all(path).into_iter().find(|o| !o.is_external())
    }

    pub fn exists(&self, path: &[Seg]) -> bool {
        self.get(path).is_some()
    }

    /// Real objects directly below `path`, in definition order, one per name.
    #[cfg(test)]
    pub fn children(&self, path: &[Seg]) -> Vec<&NsObject> {
        let mut seen: Vec<&Seg> = Vec::new();
        let mut out = Vec::new();
        for &i in self.by_parent.get(path).map(Vec::as_slice).unwrap_or(&[]) {
            let Some(o) = self.objects.get(i) else {
                continue;
            };
            if o.is_external() {
                continue;
            }
            if let Some(seg) = o.last_seg() {
                if !seen.contains(&seg) {
                    seen.push(seg);
                    out.push(o);
                }
            }
        }
        out
    }

    pub fn child(&self, path: &[Seg], seg: &Seg) -> Option<&NsObject> {
        let mut p = path.to_vec();
        p.push(*seg);
        self.get(&p)
    }

    /// Absolute path an AML reference resolves to (search rules for lone
    /// NameSegs), if the target is known.
    pub fn resolve(&self, name: &NamePath, scope: &[Seg]) -> Option<Vec<Seg>> {
        if !name.is_simple() {
            let p = name.resolve(scope)?;
            return self.by_path.contains_key(&p).then_some(p);
        }
        let seg = name.segs[0];
        for i in (0..=scope.len()).rev() {
            let mut p = scope[..i].to_vec();
            p.push(seg);
            if self.by_path.contains_key(&p) {
                return Some(p);
            }
        }
        None
    }

    /// Walk one table image (header included) and record its objects.
    pub fn walk_table(&mut self, table: usize, data: &[u8]) {
        if data.len() <= super::aml::HEADER_LEN {
            return;
        }
        let mut walker = Walker {
            ns: self,
            table,
            data,
            heuristic: 0,
        };
        walker.term_list(super::aml::HEADER_LEN, data.len(), &[], 0);
    }
}

impl Arity for Namespace {
    fn method_args(&self, name: &NamePath, scope: &[Seg]) -> Option<u8> {
        if let Some(path) = self.resolve(name, scope) {
            for o in self.all(&path) {
                match o.kind {
                    NsKind::Method { args, .. } => return Some(args),
                    NsKind::External { obj_type: 8, args } => return Some(args),
                    _ => {}
                }
            }
            return Some(0);
        }
        if let Some(args) = self.hinted_args(name, scope) {
            return Some(args);
        }
        // The OS provides \_OSI.
        if name.last() == Some(&OSI) && (name.is_simple() || (name.root && name.segs.len() == 1)) {
            return Some(1);
        }
        self.unresolved.set(true);
        None
    }
}

struct Walker<'a> {
    ns: &'a mut Namespace,
    table: usize,
    data: &'a [u8],
    heuristic: usize,
}

impl Walker<'_> {
    fn record(
        &mut self,
        path: Vec<Seg>,
        kind: NsKind,
        name_end: usize,
        body: Option<(usize, usize)>,
    ) {
        if path.is_empty() {
            return;
        }
        self.ns.add(NsObject {
            path,
            kind,
            table: self.table,
            name_offset: name_end.saturating_sub(4),
            body,
            heuristic: self.heuristic > 0,
        });
    }

    /// Step over a term with the generic decoder.
    fn skip(&self, pos: usize, end: usize, scope: &[Seg]) -> PResult<usize> {
        let ns: &Namespace = self.ns;
        Decoder::new(self.data, ns, scope).term(pos, end)
    }

    fn term_list(&mut self, start: usize, end: usize, scope: &[Seg], depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut pos = start;
        while pos < end {
            match self.term_obj(pos, end, scope, depth) {
                Ok(next) if next > pos => pos = next,
                _ => {
                    tracing::debug!(
                        table = self.table,
                        offset = pos,
                        "AML term not decodable, scanning heuristically"
                    );
                    self.heuristic_scan(pos, end, scope, depth);
                    return;
                }
            }
        }
    }

    /// Name of a definition: (absolute path, offset after the NameString).
    fn def_name(&self, pos: usize, end: usize, scope: &[Seg]) -> PResult<(Vec<Seg>, usize)> {
        let (np, n) = name_path(self.data, pos)?;
        let after = pos + n;
        if after > end {
            return Err(Malformed);
        }
        let path = np.resolve(scope).ok_or(Malformed)?;
        Ok((path, after))
    }

    fn term_obj(&mut self, pos: usize, end: usize, scope: &[Seg], depth: usize) -> PResult<usize> {
        let d = self.data;
        match d[pos] {
            0x10 => {
                let (body, e) = pkg_bounds(d, pos + 1, end)?;
                let (path, after) = self.def_name(body, e, scope)?;
                self.term_list(after, e, &path, depth + 1);
                Ok(e)
            }
            0x14 => {
                let (body, e) = pkg_bounds(d, pos + 1, end)?;
                let (path, after) = self.def_name(body, e, scope)?;
                let flags = *d.get(after).filter(|_| after < e).ok_or(Malformed)?;
                let kind = NsKind::Method {
                    args: flags & 0x07,
                    serialized: flags & 0x08 != 0,
                };
                self.record(path, kind, after, Some((after + 1, e)));
                Ok(e)
            }
            0x08 => {
                let (path, after) = self.def_name(pos + 1, end, scope)?;
                let (value, next) = match data_object(d, after, end, 0) {
                    Ok((v, n)) => (v, after + n),
                    Err(_) => (Value::Other, self.skip(after, end, scope)?),
                };
                self.record(path, NsKind::Name(value), after, None);
                Ok(next)
            }
            0x06 => {
                let (_, n) = name_path(d, pos + 1)?;
                let (path, after) = self.def_name(pos + 1 + n, end, scope)?;
                self.record(path, NsKind::Alias, after, None);
                Ok(after)
            }
            0x15 => {
                let (path, after) = self.def_name(pos + 1, end, scope)?;
                let bytes = d
                    .get(after..after + 2)
                    .filter(|_| after + 2 <= end)
                    .ok_or(Malformed)?;
                let kind = NsKind::External {
                    obj_type: bytes[0],
                    args: bytes[1],
                };
                self.record(path, kind, after, None);
                Ok(after + 2)
            }
            0x8A..=0x8D | 0x8F => {
                let p = self.skip(pos + 1, end, scope)?;
                let p = self.skip(p, end, scope)?;
                let (path, after) = self.def_name(p, end, scope)?;
                self.record(path, NsKind::BufferField, after, None);
                Ok(after)
            }
            0xA0 | 0xA2 => {
                let (body, e) = pkg_bounds(d, pos + 1, end)?;
                let after = self.skip(body, e, scope)?;
                self.term_list(after, e, scope, depth + 1);
                Ok(e)
            }
            0xA1 => {
                let (body, e) = pkg_bounds(d, pos + 1, end)?;
                self.term_list(body, e, scope, depth + 1);
                Ok(e)
            }
            0x5B => self.ext_obj(pos, end, scope, depth),
            _ => self.skip(pos, end, scope),
        }
    }

    fn ext_obj(&mut self, pos: usize, end: usize, scope: &[Seg], depth: usize) -> PResult<usize> {
        let d = self.data;
        let ext = *d.get(pos + 1).ok_or(Malformed)?;
        match ext {
            // Device, Processor, PowerResource, ThermalZone
            0x82..=0x85 => {
                let (body, e) = pkg_bounds(d, pos + 2, end)?;
                let (path, after) = self.def_name(body, e, scope)?;
                let (kind, extra) = match ext {
                    0x82 => (NsKind::Device, 0),
                    0x83 => (
                        NsKind::Processor {
                            id: *d.get(after).ok_or(Malformed)?,
                        },
                        6,
                    ),
                    0x84 => (NsKind::PowerResource, 3),
                    _ => (NsKind::ThermalZone, 0),
                };
                let inner = after + extra;
                if inner > e {
                    return Err(Malformed);
                }
                self.record(path.clone(), kind, after, Some((inner, e)));
                self.term_list(inner, e, &path, depth + 1);
                Ok(e)
            }
            // Field, IndexField, BankField
            0x81 | 0x86 | 0x87 => {
                let (body, e) = pkg_bounds(d, pos + 2, end)?;
                let (_, n) = name_path(d, body)?;
                let mut p = body + n;
                if ext == 0x86 {
                    let (_, n) = name_path(d, p)?;
                    p += n;
                } else if ext == 0x87 {
                    let (_, n) = name_path(d, p)?;
                    p = self.skip(p + n, e, scope)?;
                }
                // FieldFlags byte, then the unit list.
                self.field_list(p + 1, e, scope)?;
                Ok(e)
            }
            0x80 => {
                let (path, after) = self.def_name(pos + 2, end, scope)?;
                if after >= end {
                    return Err(Malformed);
                }
                let p = self.skip(after + 1, end, scope)?;
                let p = self.skip(p, end, scope)?;
                self.record(path, NsKind::OpRegion, after, None);
                Ok(p)
            }
            0x01 | 0x02 => {
                let (path, after) = self.def_name(pos + 2, end, scope)?;
                let next = if ext == 0x01 { after + 1 } else { after };
                if next > end {
                    return Err(Malformed);
                }
                let kind = if ext == 0x01 {
                    NsKind::Mutex
                } else {
                    NsKind::Event
                };
                self.record(path, kind, after, None);
                Ok(next)
            }
            0x13 => {
                let mut p = pos + 2;
                for _ in 0..3 {
                    p = self.skip(p, end, scope)?;
                }
                let (path, after) = self.def_name(p, end, scope)?;
                self.record(path, NsKind::BufferField, after, None);
                Ok(after)
            }
            0x88 => {
                let (path, after) = self.def_name(pos + 2, end, scope)?;
                let mut p = after;
                for _ in 0..3 {
                    p = self.skip(p, end, scope)?;
                }
                self.record(path, NsKind::DataRegion, after, None);
                Ok(p)
            }
            _ => self.skip(pos, end, scope),
        }
    }

    fn field_list(&mut self, mut p: usize, end: usize, scope: &[Seg]) -> PResult<()> {
        let d = self.data;
        if p > end {
            return Err(Malformed);
        }
        while p < end {
            match d[p] {
                0x00 => {
                    let (_, n) = pkg_length(d, p + 1)?;
                    p += 1 + n;
                }
                0x01 => p += 3,
                0x03 => p += 4,
                0x02 => {
                    if d.get(p + 1) == Some(&0x11) {
                        let (_, e) = pkg_bounds(d, p + 2, end)?;
                        p = e;
                    } else {
                        let (_, n) = name_path(d, p + 1)?;
                        p += 1 + n;
                    }
                }
                c if is_lead_char(c) => {
                    let seg = d.get(p..p + 4).ok_or(Malformed)?;
                    if !valid_seg(seg) {
                        return Err(Malformed);
                    }
                    let (_, n) = pkg_length(d, p + 4)?;
                    let mut path = scope.to_vec();
                    path.push([seg[0], seg[1], seg[2], seg[3]]);
                    self.record(path, NsKind::FieldUnit, p + 4, None);
                    p += 4 + n;
                }
                _ => return Err(Malformed),
            }
        }
        if p > end {
            return Err(Malformed);
        }
        Ok(())
    }

    /// Names whose `Name` definitions are worth recovering heuristically.
    fn interesting_name(seg: &[u8]) -> bool {
        matches!(seg, b"_HID" | b"_CID" | b"_ADR" | b"_UID" | b"_STA")
    }

    /// Plausible definition start at `i`: its package must end inside `end`.
    fn candidate(&self, i: usize, end: usize, scope: &[Seg]) -> bool {
        let d = self.data;
        let pkg_name = |pkg_pos: usize| -> bool {
            match pkg_bounds(d, pkg_pos, end) {
                Ok((body, e)) => match name_path(d, body) {
                    Ok((np, n)) => {
                        body + n <= e && !np.segs.is_empty() && np.resolve(scope).is_some()
                    }
                    Err(_) => false,
                },
                Err(_) => false,
            }
        };
        match d[i] {
            0x5B => matches!(d.get(i + 1), Some(0x82..=0x85)) && pkg_name(i + 2),
            0x10 | 0x14 => pkg_name(i + 1),
            0x08 => match name_path(d, i + 1) {
                Ok((np, n)) if np.is_simple() && Self::interesting_name(&np.segs[0]) => {
                    data_object(d, i + 1 + n, end, 0).is_ok()
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn heuristic_scan(&mut self, start: usize, end: usize, scope: &[Seg], depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        self.heuristic += 1;
        let mut i = start;
        while i < end {
            if self.candidate(i, end, scope) {
                if let Ok(next) = self.term_obj(i, end, scope, depth + 1) {
                    if next > i {
                        i = next;
                        continue;
                    }
                }
            }
            i += 1;
        }
        self.heuristic -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::aml::{encode_body, table, FieldUnit, ObjType, Term};
    use super::*;

    fn walk(terms: &[Term]) -> Namespace {
        let image = table(b"DSDT", 2, "TEST", "TESTDSDT", 1, &encode_body(terms));
        let mut ns = Namespace::default();
        ns.walk_table(0, &image);
        ns
    }

    fn p(s: &str) -> Vec<Seg> {
        s.trim_start_matches('\\')
            .split('.')
            .map(super::super::aml::name_seg)
            .collect()
    }

    #[test]
    fn walks_nested_scopes_and_definitions() {
        let ns = walk(&[
            Term::external("\\_SB.PCI0.XXXX", ObjType::Int),
            Term::scope(
                "\\_SB",
                vec![Term::device(
                    "PCI0",
                    vec![
                        Term::name("_HID", Term::EisaId("PNP0A08".into())),
                        Term::device(
                            "LPCB",
                            vec![
                                Term::name("_ADR", Term::int(0x001F_0000)),
                                Term::device(
                                    "EC0",
                                    vec![Term::method("_STA", 0, vec![Term::ret(Term::int(0x0F))])],
                                ),
                            ],
                        ),
                    ],
                )],
            ),
            Term::OpRegion {
                name: "GNVS".into(),
                space: 0,
                offset: Box::new(Term::int(0x1000)),
                len: Box::new(Term::int(0x10)),
            },
            Term::Field {
                region: "GNVS".into(),
                flags: 0,
                units: vec![
                    FieldUnit {
                        name: Some("OSYS".into()),
                        bits: 16,
                    },
                    FieldUnit {
                        name: None,
                        bits: 8,
                    },
                    FieldUnit {
                        name: Some("GPEN".into()),
                        bits: 8,
                    },
                ],
            },
            Term::if_else(
                Term::cond_ref_of("\\_SB.PCI0"),
                vec![Term::scope(
                    "\\_SB.PCI0",
                    vec![Term::device(
                        "GFX0",
                        vec![Term::name("_ADR", Term::int(0x0002_0000))],
                    )],
                )],
                vec![Term::name("ELSE", Term::int(1))],
            ),
        ]);
        assert!(matches!(
            ns.get(&p("\\_SB.PCI0")).map(|o| &o.kind),
            Some(NsKind::Device)
        ));
        assert!(matches!(
            ns.get(&p("\\_SB.PCI0.LPCB._ADR")).map(|o| &o.kind),
            Some(NsKind::Name(Value::Int(0x001F_0000)))
        ));
        assert!(matches!(
            ns.get(&p("\\_SB.PCI0.LPCB.EC0._STA")).map(|o| &o.kind),
            Some(NsKind::Method { args: 0, .. })
        ));
        assert!(ns.exists(&p("\\GPEN")));
        assert!(ns.exists(&p("\\OSYS")));
        assert!(ns.exists(&p("\\_SB.PCI0.GFX0._ADR")));
        assert!(ns.exists(&p("\\ELSE")));
        assert!(
            !ns.exists(&p("\\_SB.PCI0.XXXX")),
            "externals are not real definitions"
        );
        assert_eq!(ns.all(&p("\\_SB.PCI0.XXXX")).len(), 1);
        let children: Vec<String> = ns
            .children(&p("\\_SB.PCI0"))
            .iter()
            .map(|o| {
                String::from_utf8_lossy(o.last_seg().map(|s| s.as_slice()).unwrap_or(b""))
                    .into_owned()
            })
            .collect();
        assert_eq!(children, vec!["_HID", "LPCB", "GFX0"]);
        assert!(ns.objects.iter().all(|o| !o.heuristic));
    }

    #[test]
    fn resolves_with_search_rules() {
        let ns = walk(&[
            Term::name("STAS", Term::int(1)),
            Term::scope(
                "\\_SB",
                vec![Term::device("AWAC", vec![Term::method("_STA", 0, vec![])])],
            ),
        ]);
        let np = NamePath {
            root: false,
            parents: 0,
            segs: vec![*b"STAS"],
        };
        assert_eq!(ns.resolve(&np, &p("\\_SB.AWAC._STA")), Some(p("\\STAS")));
        assert_eq!(
            ns.method_args(
                &NamePath {
                    root: false,
                    parents: 0,
                    segs: vec![*b"_OSI"]
                },
                &[]
            ),
            Some(1)
        );
    }

    #[test]
    fn recovers_from_undecodable_statement() {
        // A module-level call to an unknown two-argument method desynchronises
        // the decoder; the devices after it must still be found.
        let mut body = encode_body(&[Term::scope("\\_SB", vec![Term::device("PCI0", vec![])])]);
        let mut sb_scope = Vec::new();
        // Scope (\_SB) { Device (DEV1) { Name (_HID, "ACPI000E") } <0x5B 0x99 garbage> Device (DEV2) {...} }
        let dev1 = {
            let mut v = Vec::new();
            Term::device("DEV1", vec![Term::name("_HID", Term::str("ACPI000E"))]).encode(&mut v);
            v
        };
        let dev2 = {
            let mut v = Vec::new();
            Term::device("DEV2", vec![Term::name("_ADR", Term::int(0x0014_0000))]).encode(&mut v);
            v
        };
        let mut inner = super::super::aml::name_string("\\_SB");
        inner.extend_from_slice(&dev1);
        inner.extend_from_slice(&[0x5B, 0x99, 0x42, 0x13]);
        inner.extend_from_slice(&dev2);
        sb_scope.push(0x10);
        sb_scope.extend(super::super::aml::pkg_length(inner.len()));
        sb_scope.extend(inner);
        body.extend(sb_scope);
        let image = table(b"DSDT", 2, "TEST", "TESTDSDT", 1, &body);
        let mut ns = Namespace::default();
        ns.walk_table(0, &image);
        assert!(ns.exists(&p("\\_SB.DEV1._HID")));
        let dev2 = ns
            .get(&p("\\_SB.DEV2"))
            .expect("found by the fallback scan");
        assert!(dev2.heuristic);
        assert!(ns.exists(&p("\\_SB.DEV2._ADR")));
        assert!(ns.exists(&p("\\_SB.PCI0")));
    }

    #[test]
    fn forward_method_calls_are_sized_on_the_second_walk() {
        // Store (FWDM (One, 0x02), XXXX) before Method (FWDM, 2) is defined:
        // the first walk cannot size the call and falls back to scanning.
        let terms = vec![
            Term::name("XXXX", Term::int(0)),
            Term::store(
                Term::call("FWDM", vec![Term::int(1), Term::int(2)]),
                Term::path("XXXX"),
            ),
            Term::method("FWDM", 2, vec![Term::ret(Term::Arg(0))]),
            Term::scope(
                "\\_SB",
                vec![Term::device("DEV1", vec![Term::name("_ADR", Term::int(0))])],
            ),
        ];
        let image = table(b"DSDT", 2, "TEST", "TESTDSDT", 1, &encode_body(&terms));
        let mut single = Namespace::default();
        single.walk_table(0, &image);
        assert!(single.unresolved.get());
        assert!(
            single.objects.iter().any(|o| o.heuristic),
            "the single walk needs the fallback scan"
        );

        let ns = Namespace::build(std::iter::once((0usize, image.as_slice())));
        assert!(ns.objects.iter().all(|o| !o.heuristic));
        assert!(ns.exists(&p("\\_SB.DEV1._ADR")));
        assert!(matches!(
            ns.get(&p("\\FWDM")).map(|o| &o.kind),
            Some(NsKind::Method { args: 2, .. })
        ));
    }

    #[test]
    fn truncated_tables_do_not_panic() {
        let image = table(
            b"DSDT",
            2,
            "TEST",
            "TESTDSDT",
            1,
            &encode_body(&[Term::scope(
                "\\_SB",
                vec![Term::device("PCI0", vec![Term::name("_ADR", Term::int(0))])],
            )]),
        );
        for cut in super::super::aml::HEADER_LEN..image.len() {
            let mut ns = Namespace::default();
            ns.walk_table(0, &image[..cut]);
        }
        // Random noise
        let mut noise = image.clone();
        for (i, b) in noise.iter_mut().enumerate().skip(36) {
            *b = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        let mut ns = Namespace::default();
        ns.walk_table(0, &noise);
    }
}
