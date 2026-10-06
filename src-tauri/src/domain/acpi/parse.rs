//! Low-level AML decoding: PkgLength, NameString, data objects and a length
//! decoder covering every ACPI 6.5 opcode, so statements and expressions can
//! be stepped over without executing anything. Method invocations need the
//! callee's argument count, which the caller supplies through [`Arity`].

pub(crate) type Seg = [u8; 4];

/// The bytes at a position do not form a valid AML construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Malformed;

pub(crate) type PResult<T> = Result<T, Malformed>;

/// Nesting limit for recursive decoding (real tables stay far below it).
pub(crate) const MAX_DEPTH: usize = 96;

/// The predefined `\_OSI` method is not declared in any table.
pub(crate) const OSI: Seg = *b"_OSI";

pub(crate) fn is_lead_char(c: u8) -> bool {
    c.is_ascii_uppercase() || c == b'_'
}

pub(crate) fn is_name_char(c: u8) -> bool {
    is_lead_char(c) || c.is_ascii_digit()
}

/// First byte of a NameString.
pub(crate) fn is_name_start(c: u8) -> bool {
    is_lead_char(c) || matches!(c, b'\\' | b'^' | 0x2E | 0x2F)
}

pub(crate) fn valid_seg(seg: &[u8]) -> bool {
    seg.len() == 4 && is_lead_char(seg[0]) && seg[1..].iter().all(|&c| is_name_char(c))
}

/// Decode a PkgLength at `pos`: (encoded value, bytes used).
pub(crate) fn pkg_length(data: &[u8], pos: usize) -> PResult<(usize, usize)> {
    let b0 = *data.get(pos).ok_or(Malformed)?;
    let extra = usize::from(b0 >> 6);
    if extra == 0 {
        return Ok((usize::from(b0 & 0x3F), 1));
    }
    if b0 & 0x30 != 0 {
        return Err(Malformed);
    }
    let mut value = usize::from(b0 & 0x0F);
    for i in 0..extra {
        let b = *data.get(pos + 1 + i).ok_or(Malformed)?;
        value |= usize::from(b) << (4 + 8 * i);
    }
    Ok((value, extra + 1))
}

/// Bounds of a package whose PkgLength starts at `pkg_pos`:
/// (first byte after the PkgLength, end of the package).
pub(crate) fn pkg_bounds(data: &[u8], pkg_pos: usize, limit: usize) -> PResult<(usize, usize)> {
    let (len, used) = pkg_length(data, pkg_pos)?;
    let end = pkg_pos.checked_add(len).ok_or(Malformed)?;
    if len < used || end > limit || end > data.len() {
        return Err(Malformed);
    }
    Ok((pkg_pos + used, end))
}

/// A decoded NameString.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct NamePath {
    pub root: bool,
    pub parents: usize,
    pub segs: Vec<Seg>,
}

impl NamePath {
    /// Absolute path of a name defined (or referenced without search) in `scope`.
    pub fn resolve(&self, scope: &[Seg]) -> Option<Vec<Seg>> {
        let mut out = if self.root {
            Vec::new()
        } else {
            let keep = scope.len().checked_sub(self.parents)?;
            scope[..keep].to_vec()
        };
        out.extend_from_slice(&self.segs);
        Some(out)
    }

    /// A lone NameSeg, which is looked up with the namespace search rules.
    pub fn is_simple(&self) -> bool {
        !self.root && self.parents == 0 && self.segs.len() == 1
    }

    pub fn last(&self) -> Option<&Seg> {
        self.segs.last()
    }
}

/// Decode a NameString at `pos`: (path, bytes used).
pub(crate) fn name_path(data: &[u8], pos: usize) -> PResult<(NamePath, usize)> {
    let mut np = NamePath::default();
    let mut i = pos;
    match data.get(i) {
        Some(b'\\') => {
            np.root = true;
            i += 1;
        }
        Some(b'^') => {
            while data.get(i) == Some(&b'^') {
                np.parents += 1;
                i += 1;
            }
        }
        _ => {}
    }
    let count = match *data.get(i).ok_or(Malformed)? {
        0x00 => {
            i += 1;
            0
        }
        0x2E => {
            i += 1;
            2
        }
        0x2F => {
            let n = usize::from(*data.get(i + 1).ok_or(Malformed)?);
            if n == 0 {
                return Err(Malformed);
            }
            i += 2;
            n
        }
        c if is_lead_char(c) => 1,
        _ => return Err(Malformed),
    };
    for _ in 0..count {
        let seg = data.get(i..i + 4).ok_or(Malformed)?;
        if !valid_seg(seg) {
            return Err(Malformed);
        }
        np.segs.push([seg[0], seg[1], seg[2], seg[3]]);
        i += 4;
    }
    Ok((np, i - pos))
}

/// Integer constant at `pos`: (value, bytes used).
pub(crate) fn const_int(data: &[u8], pos: usize) -> Option<(u64, usize)> {
    let read = |n: usize| -> Option<u64> {
        let bytes = data.get(pos + 1..pos + 1 + n)?;
        Some(
            bytes
                .iter()
                .rev()
                .fold(0u64, |acc, b| (acc << 8) | u64::from(*b)),
        )
    };
    match *data.get(pos)? {
        0x00 => Some((0, 1)),
        0x01 => Some((1, 1)),
        0xFF => Some((u64::MAX, 1)),
        0x0A => Some((read(1)?, 2)),
        0x0B => Some((read(2)?, 3)),
        0x0C => Some((read(4)?, 5)),
        0x0E => Some((read(8)?, 9)),
        _ => None,
    }
}

/// A decoded data object (the value of a `Name`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Int(u64),
    Str(String),
    Buffer(Vec<u8>),
    Package(Vec<Value>),
    Ref(NamePath),
    Other,
}

/// Decode a DataRefObject at `pos` (bounded by `end`): (value, bytes used).
pub(crate) fn data_object(
    data: &[u8],
    pos: usize,
    end: usize,
    depth: usize,
) -> PResult<(Value, usize)> {
    if pos >= end || depth > MAX_DEPTH {
        return Err(Malformed);
    }
    if let Some((v, n)) = const_int(data, pos) {
        if pos + n > end {
            return Err(Malformed);
        }
        return Ok((Value::Int(v), n));
    }
    match data[pos] {
        0x0D => {
            let rest = &data[pos + 1..end];
            let nul = rest.iter().position(|&b| b == 0).ok_or(Malformed)?;
            let text = String::from_utf8_lossy(&rest[..nul]).into_owned();
            Ok((Value::Str(text), nul + 2))
        }
        0x11 => {
            let (body, e) = pkg_bounds(data, pos + 1, end)?;
            let bytes = match const_int(data, body) {
                Some((_, n)) if body + n <= e => data[body + n..e].to_vec(),
                _ => Vec::new(),
            };
            Ok((Value::Buffer(bytes), e - pos))
        }
        0x12 => {
            let (body, e) = pkg_bounds(data, pos + 1, end)?;
            if body >= e {
                return Err(Malformed);
            }
            let mut items = Vec::new();
            let mut p = body + 1;
            while p < e {
                if is_name_start(data[p]) {
                    let (np, n) = name_path(data, p)?;
                    items.push(Value::Ref(np));
                    p += n;
                } else {
                    let (v, n) = data_object(data, p, e, depth + 1)?;
                    items.push(v);
                    p += n;
                }
            }
            Ok((Value::Package(items), e - pos))
        }
        0x13 => {
            let (_, e) = pkg_bounds(data, pos + 1, end)?;
            Ok((Value::Other, e - pos))
        }
        0x5B if data.get(pos + 1) == Some(&0x30) => Ok((Value::Other, 2)),
        c if is_name_start(c) => {
            let (np, n) = name_path(data, pos)?;
            Ok((Value::Ref(np), n))
        }
        _ => Err(Malformed),
    }
}

/// Argument counts of methods referenced by NameString, from the namespace
/// known so far.
pub(crate) trait Arity {
    fn method_args(&self, name: &NamePath, scope: &[Seg]) -> Option<u8>;
}

/// Steps over AML terms without interpreting them.
pub(crate) struct Decoder<'a> {
    data: &'a [u8],
    arity: &'a dyn Arity,
    scope: &'a [Seg],
    depth: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8], arity: &'a dyn Arity, scope: &'a [Seg]) -> Self {
        Self {
            data,
            arity,
            scope,
            depth: 0,
        }
    }

    fn fixed(&self, pos: usize, n: usize, end: usize) -> PResult<usize> {
        let next = pos.checked_add(n).ok_or(Malformed)?;
        if next > end {
            return Err(Malformed);
        }
        Ok(next)
    }

    fn name(&self, pos: usize, end: usize) -> PResult<usize> {
        if pos >= end {
            return Err(Malformed);
        }
        let (_, n) = name_path(self.data, pos)?;
        self.fixed(pos, n, end)
    }

    fn skip_pkg(&self, pkg_pos: usize, end: usize) -> PResult<usize> {
        let (_, e) = pkg_bounds(self.data, pkg_pos, end)?;
        Ok(e)
    }

    /// Step over one term of any kind (definition, statement or expression).
    pub fn term(&mut self, pos: usize, end: usize) -> PResult<usize> {
        if pos >= end || pos >= self.data.len() {
            return Err(Malformed);
        }
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(Malformed);
        }
        let result = self.term_inner(pos, end);
        self.depth -= 1;
        result
    }

    fn terms(&mut self, mut pos: usize, end: usize, count: usize) -> PResult<usize> {
        for _ in 0..count {
            pos = self.term(pos, end)?;
        }
        Ok(pos)
    }

    fn term_inner(&mut self, pos: usize, end: usize) -> PResult<usize> {
        let d = self.data;
        let p = pos + 1;
        match d[pos] {
            0x00 | 0x01 | 0xFF => Ok(p),
            0x0A => self.fixed(p, 1, end),
            0x0B => self.fixed(p, 2, end),
            0x0C => self.fixed(p, 4, end),
            0x0E => self.fixed(p, 8, end),
            0x0D => {
                let rest = d.get(p..end).ok_or(Malformed)?;
                let nul = rest.iter().position(|&b| b == 0).ok_or(Malformed)?;
                Ok(p + nul + 1)
            }
            // Scope, Buffer, Package, VarPackage, Method, If, Else, While
            0x10..=0x14 | 0xA0..=0xA2 => self.skip_pkg(p, end),
            0x06 => {
                let p = self.name(p, end)?;
                self.name(p, end)
            }
            0x08 => {
                let p = self.name(p, end)?;
                self.term(p, end)
            }
            0x15 => {
                let p = self.name(p, end)?;
                self.fixed(p, 2, end)
            }
            0x60..=0x6E => Ok(p),
            0x70 => {
                let p = self.term(p, end)?;
                self.super_name(p, end)
            }
            // RefOf, Increment, Decrement, SizeOf, ObjectType
            0x71 | 0x75 | 0x76 | 0x87 | 0x8E => self.super_name(p, end),
            // Operand, Operand, Target
            0x72..=0x74 | 0x77 | 0x79..=0x7F | 0x84 | 0x85 | 0x88 => {
                let p = self.terms(p, end, 2)?;
                self.target(p, end)
            }
            0x78 => {
                let p = self.terms(p, end, 2)?;
                let p = self.target(p, end)?;
                self.target(p, end)
            }
            // Operand, Target
            0x80..=0x82 | 0x96..=0x99 => {
                let p = self.term(p, end)?;
                self.target(p, end)
            }
            0x83 | 0x92 | 0xA4 => self.term(p, end),
            0x86 => {
                let p = self.super_name(p, end)?;
                self.term(p, end)
            }
            0x89 => {
                let p = self.term(p, end)?;
                let p = self.fixed(p, 1, end)?;
                let p = self.term(p, end)?;
                let p = self.fixed(p, 1, end)?;
                self.terms(p, end, 2)
            }
            // CreateDWord/Word/Byte/Bit/QWordField
            0x8A..=0x8D | 0x8F => {
                let p = self.terms(p, end, 2)?;
                self.name(p, end)
            }
            0x90 | 0x91 | 0x93..=0x95 => self.terms(p, end, 2),
            0x9C => {
                let p = self.terms(p, end, 2)?;
                self.target(p, end)
            }
            0x9D => {
                let p = self.term(p, end)?;
                self.super_name(p, end)
            }
            0x9E => {
                let p = self.terms(p, end, 3)?;
                self.target(p, end)
            }
            0x9F | 0xA3 | 0xA5 | 0xCC => Ok(p),
            0x5B => self.ext_op(p, end),
            c if is_name_start(c) => {
                let (np, n) = name_path(d, pos)?;
                let args = self.arity.method_args(&np, self.scope).unwrap_or(0);
                self.terms(self.fixed(pos, n, end)?, end, usize::from(args))
            }
            _ => Err(Malformed),
        }
    }

    fn ext_op(&mut self, pos: usize, end: usize) -> PResult<usize> {
        let ext = *self.data.get(pos).ok_or(Malformed)?;
        let p = self.fixed(pos, 1, end)?;
        match ext {
            0x01 => {
                let p = self.name(p, end)?;
                self.fixed(p, 1, end)
            }
            0x02 => self.name(p, end),
            0x12 => {
                let p = self.super_name(p, end)?;
                self.target(p, end)
            }
            0x13 => {
                let p = self.terms(p, end, 3)?;
                self.name(p, end)
            }
            0x1F => self.terms(p, end, 6),
            0x20 => {
                let p = self.name(p, end)?;
                self.target(p, end)
            }
            0x21 | 0x22 => self.term(p, end),
            0x23 => {
                let p = self.super_name(p, end)?;
                self.fixed(p, 2, end)
            }
            0x24 | 0x26 | 0x27 | 0x2A => self.super_name(p, end),
            0x25 => {
                let p = self.super_name(p, end)?;
                self.term(p, end)
            }
            0x28 | 0x29 => {
                let p = self.term(p, end)?;
                self.target(p, end)
            }
            0x30 | 0x31 | 0x33 => Ok(p),
            0x32 => {
                let p = self.fixed(p, 5, end)?;
                self.term(p, end)
            }
            0x80 => {
                let p = self.name(p, end)?;
                let p = self.fixed(p, 1, end)?;
                self.terms(p, end, 2)
            }
            0x81..=0x87 => self.skip_pkg(p, end),
            0x88 => {
                let p = self.name(p, end)?;
                self.terms(p, end, 3)
            }
            _ => Err(Malformed),
        }
    }

    fn super_name(&mut self, pos: usize, end: usize) -> PResult<usize> {
        let b = *self.data.get(pos).ok_or(Malformed)?;
        if pos >= end {
            return Err(Malformed);
        }
        match b {
            0x60..=0x6E => Ok(pos + 1),
            0x5B if self.data.get(pos + 1) == Some(&0x31) => self.fixed(pos, 2, end),
            0x71 | 0x83 | 0x88 => self.term(pos, end),
            c if is_name_start(c) => self.name(pos, end),
            _ => Err(Malformed),
        }
    }

    fn target(&mut self, pos: usize, end: usize) -> PResult<usize> {
        if self.data.get(pos) == Some(&0x00) && pos < end {
            return Ok(pos + 1);
        }
        self.super_name(pos, end)
    }

    /// Collect the operands of every `Return` in a method body, following
    /// If/Else/While bodies: `Some(value)` for integer constants.
    pub fn returns(
        &mut self,
        mut pos: usize,
        end: usize,
        out: &mut Vec<Option<u64>>,
    ) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(Malformed);
        }
        let d = self.data;
        let mut result = Ok(());
        while pos < end {
            let step = match d[pos] {
                op @ 0xA0..=0xA2 => pkg_bounds(d, pos + 1, end).and_then(|(body, e)| {
                    let body = if op == 0xA1 {
                        body
                    } else {
                        self.term(body, e)?
                    };
                    self.returns(body, e, out)?;
                    Ok(e)
                }),
                0xA4 => {
                    out.push(const_int(d, pos + 1).map(|(v, _)| v));
                    self.term(pos, end)
                }
                _ => self.term(pos, end),
            };
            match step {
                Ok(next) => pos = next,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.depth -= 1;
        result
    }
}

/// String literals (StringPrefix ... NUL) of 1-64 printable characters found
/// in `data`, by byte scan. Used for `_HID` methods that return one of
/// several ids.
pub(crate) fn string_literals(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x0D {
            let rest = &data[i + 1..];
            if let Some(nul) = rest.iter().take(65).position(|&b| b == 0) {
                if nul > 0 && rest[..nul].iter().all(|b| (0x20..0x7F).contains(b)) {
                    out.push(String::from_utf8_lossy(&rest[..nul]).into_owned());
                    i += nul + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Every position where `needle` occurs in `hay` (overlapping).
pub(crate) fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return Vec::new();
    }
    hay.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoMethods;
    impl Arity for NoMethods {
        fn method_args(&self, name: &NamePath, _scope: &[Seg]) -> Option<u8> {
            (name.last() == Some(&OSI)).then_some(1)
        }
    }

    #[test]
    fn pkg_length_decoding() {
        assert_eq!(pkg_length(&[0x3F], 0), Ok((0x3F, 1)));
        assert_eq!(pkg_length(&[0x48, 0x06], 0), Ok((0x68, 2)));
        assert_eq!(pkg_length(&[0x81, 0x00, 0x01], 0), Ok((0x1001, 3)));
        assert_eq!(pkg_length(&[0xC1, 0x00, 0x00, 0x01], 0), Ok((0x10_0001, 4)));
        assert_eq!(pkg_length(&[0x70, 0x00], 0), Err(Malformed)); // reserved bits set
        assert_eq!(pkg_length(&[0x48], 0), Err(Malformed)); // truncated
    }

    #[test]
    fn name_path_decoding() {
        let (np, n) = name_path(b"\\/\x03_SB_PCI0LPCB", 0).expect("valid");
        assert!(np.root);
        assert_eq!(np.segs, vec![*b"_SB_", *b"PCI0", *b"LPCB"]);
        assert_eq!(n, 15);
        let (np, n) = name_path(b"^^EC0_", 0).expect("valid");
        assert_eq!((np.parents, np.segs.len(), n), (2, 1, 6));
        let (np, n) = name_path(b"\\\x00", 0).expect("valid");
        assert!(np.root && np.segs.is_empty() && n == 2);
        assert!(name_path(b"1BAD", 0).is_err());
        assert!(name_path(b"AB\x01C", 0).is_err());
        assert_eq!(
            np.resolve(&[*b"_SB_"]),
            Some(vec![]),
            "root prefix ignores the current scope"
        );
        let rel = NamePath {
            root: false,
            parents: 1,
            segs: vec![*b"LPCB"],
        };
        assert_eq!(
            rel.resolve(&[*b"_SB_", *b"PCI0", *b"GFX0"]),
            Some(vec![*b"_SB_", *b"PCI0", *b"LPCB"])
        );
        assert_eq!(rel.resolve(&[]), None);
    }

    #[test]
    fn decoder_steps_over_statements() {
        // If (_OSI ("Darwin")) { Return (0x0F) } Else { Return (Zero) } Store (One, STAS)
        let mut code = vec![0xA0, 0x10, b'_', b'O', b'S', b'I', 0x0D];
        code.extend_from_slice(b"Darwin\0");
        code.extend_from_slice(&[0xA4, 0x0A, 0x0F, 0xA1, 0x03, 0xA4, 0x00, 0x70, 0x01]);
        code.extend_from_slice(b"STAS");
        let mut dec = Decoder::new(&code, &NoMethods, &[]);
        assert_eq!(dec.term(0, code.len()), Ok(17));
        assert_eq!(dec.term(17, code.len()), Ok(21));
        assert_eq!(dec.term(21, code.len()), Ok(code.len()));
        let mut returns = Vec::new();
        Decoder::new(&code, &NoMethods, &[])
            .returns(0, code.len(), &mut returns)
            .expect("parses");
        assert_eq!(returns, vec![Some(0x0F), Some(0)]);
    }

    #[test]
    fn decoder_rejects_garbage() {
        let code = [0x5B, 0x99, 0x00];
        assert!(Decoder::new(&code, &NoMethods, &[])
            .term(0, code.len())
            .is_err());
        let code = [0x70, 0x0A];
        assert!(Decoder::new(&code, &NoMethods, &[])
            .term(0, code.len())
            .is_err());
    }

    #[test]
    fn data_objects() {
        let pkg = [0x12, 0x09, 0x03, 0x0D, b'a', 0x00, 0x0B, 0xEC, 0x13, 0x01];
        let (v, n) = data_object(&pkg, 0, pkg.len(), 0).expect("package");
        assert_eq!(n, pkg.len());
        assert_eq!(
            v,
            Value::Package(vec![
                Value::Str("a".into()),
                Value::Int(0x13EC),
                Value::Int(1)
            ])
        );
        let eisa = [0x0C, 0x41, 0xD0, 0x0B, 0x00];
        assert_eq!(
            data_object(&eisa, 0, eisa.len(), 0),
            Ok((Value::Int(0x000B_D041), 5))
        );
    }

    #[test]
    fn strings_by_scan() {
        let mut body = vec![0xA4, 0x0D];
        body.extend_from_slice(b"INT344B\0");
        body.extend_from_slice(&[0x0D, 0x00, 0x0D]);
        assert_eq!(string_literals(&body), vec!["INT344B".to_string()]);
    }
}
