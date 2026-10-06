//! AML (ACPI Machine Language) encoder for the SSDTs this app builds.
//!
//! A table is described once as a small term tree ([`Term`]); the same tree
//! is encoded to AML ([`encode_table`]) and printed as equivalent ASL
//! ([`asl_table`]), so the bytes and the readable source cannot drift apart.
//!
//! The encoding matches what iasl 20200925 emits for the printed ASL:
//! - integers use the shortest form: `Zero`/`One`/`Ones`, then Byte, Word,
//!   DWord or QWord prefixes; `EisaId` is always a DWord;
//! - PkgLength uses the fewest bytes (1-4) and counts its own bytes;
//! - NameStrings keep the root (`\`) / parent (`^`) prefixes as written and
//!   use DualNamePrefix / MultiNamePrefix, NameSegs padded with `_`;
//! - all `External` declarations are emitted first, grouped inside an
//!   `If (Zero)` block, followed by the rest of the definition block.
//!
//! Supported terms: External, Scope, Device, Processor, Method, Name,
//! OperationRegion, Field, If/Else, Return, Store, integer/string/buffer/
//! package/EisaId/ResourceTemplate data, Arg0-6, Local0-7, method calls
//! (`_OSI ("Darwin")`), LNot/LAnd/LOr/LEqual/LNotEqual/LGreater, CondRefOf
//! and Match.

use std::fmt::Write as _;

/// Size of a standard ACPI table header.
pub const HEADER_LEN: usize = 36;
/// Creator id written into generated table headers.
pub const CREATOR_ID: [u8; 4] = *b"INTL";
/// Creator revision: the iasl release whose output this encoder reproduces.
pub const CREATOR_REVISION: u32 = 0x2020_0925;
/// The `Ones` constant of a revision 2 (64-bit integer) table.
pub const ONES: u64 = u64::MAX;

/// An AML byte stream builder.
#[derive(Debug, Default, Clone)]
pub struct AmlBuilder {
    pub bytes: Vec<u8>,
}

impl AmlBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append the encoding of one term.
    pub fn push(&mut self, term: &Term) -> &mut Self {
        term.encode(&mut self.bytes);
        self
    }

    /// Append raw bytes.
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

// ── Primitive encodings ─────────────────────────────────────────────────────

/// Encode a PkgLength for a payload of `len` bytes (the encoding includes itself).
pub fn pkg_length(len: usize) -> Vec<u8> {
    // Same thresholds as iasl: the encoded value counts the length bytes.
    let (count, total) = if len < 0x3F {
        (1, len + 1)
    } else if len + 2 <= 0xFFF {
        (2, len + 2)
    } else if len + 3 <= 0xF_FFFF {
        (3, len + 3)
    } else {
        (4, (len + 4).min(0x0FFF_FFFF))
    };
    if count == 1 {
        return vec![total as u8];
    }
    let mut out = Vec::with_capacity(count);
    out.push((((count - 1) as u8) << 6) | (total & 0x0F) as u8);
    let mut rest = total >> 4;
    for _ in 1..count {
        out.push((rest & 0xFF) as u8);
        rest >>= 8;
    }
    out
}

fn is_lead_char(c: u8) -> bool {
    c.is_ascii_uppercase() || c == b'_'
}

fn is_name_char(c: u8) -> bool {
    is_lead_char(c) || c.is_ascii_digit()
}

/// Pad (or truncate) one NameSeg to four characters: "EC" → "EC__".
/// Lowercase letters are upper-cased and invalid characters become `_`.
pub fn name_seg(seg: &str) -> [u8; 4] {
    let mut out = *b"____";
    for (i, c) in seg.bytes().take(4).enumerate() {
        let c = c.to_ascii_uppercase();
        out[i] = if is_name_char(c) { c } else { b'_' };
    }
    if out[0].is_ascii_digit() {
        out[0] = b'_';
    }
    out
}

/// True when `path` is a syntactically valid ASL NamePath: optional `\` or
/// `^` prefixes followed by dot-separated NameSegs of 1-4 characters.
pub fn is_valid_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let mut i = 0;
    if bytes.first() == Some(&b'\\') {
        i = 1;
    } else {
        while bytes.get(i) == Some(&b'^') {
            i += 1;
        }
    }
    let rest = &path[i..];
    if rest.is_empty() {
        return path == "\\";
    }
    rest.split('.').all(|seg| {
        let s = seg.as_bytes();
        !s.is_empty() && s.len() <= 4 && is_lead_char(s[0]) && s.iter().all(|&c| is_name_char(c))
    })
}

/// Encode an ACPI NameString ("\\_SB.PCI0.LPCB", "EC", "^PCI0") with
/// DualNamePrefix/MultiNamePrefix and NameSeg padding with '_'.
pub fn name_string(path: &str) -> Vec<u8> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(path.len() + 4);
    let mut i = 0;
    if bytes.first() == Some(&b'\\') {
        out.push(b'\\');
        i = 1;
    } else {
        while bytes.get(i) == Some(&b'^') {
            out.push(b'^');
            i += 1;
        }
    }
    let rest = path.get(i..).unwrap_or("");
    let segs: Vec<[u8; 4]> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split('.').take(255).map(name_seg).collect()
    };
    match segs.len() {
        0 => out.push(0x00),
        1 => out.extend_from_slice(&segs[0]),
        2 => {
            out.push(0x2E);
            out.extend_from_slice(&segs[0]);
            out.extend_from_slice(&segs[1]);
        }
        n => {
            out.push(0x2F);
            out.push(n as u8);
            for seg in &segs {
                out.extend_from_slice(seg);
            }
        }
    }
    out
}

/// Shortest integer encoding, as iasl optimises integer constants.
pub fn integer(value: u64) -> Vec<u8> {
    match value {
        0 => vec![0x00],
        1 => vec![0x01],
        ONES => vec![0xFF],
        v if v <= 0xFF => vec![0x0A, v as u8],
        v if v <= 0xFFFF => {
            let mut out = vec![0x0B];
            out.extend_from_slice(&(v as u16).to_le_bytes());
            out
        }
        v if v <= 0xFFFF_FFFF => {
            let mut out = vec![0x0C];
            out.extend_from_slice(&(v as u32).to_le_bytes());
            out
        }
        v => {
            let mut out = vec![0x0E];
            out.extend_from_slice(&v.to_le_bytes());
            out
        }
    }
}

/// AML String: StringPrefix, ASCII bytes, NUL. Characters outside printable
/// ASCII are replaced with '?'.
pub fn string(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 2);
    out.push(0x0D);
    out.extend(
        s.bytes()
            .map(|b| if (0x20..0x7F).contains(&b) { b } else { b'?' }),
    );
    out.push(0x00);
    out
}

/// Compress an EISA id ("PNP0A08") into its 32-bit AML value.
pub fn eisa_id(id: &str) -> Option<u32> {
    let b = id.as_bytes();
    if b.len() != 7 || !b[..3].iter().all(|c| (b'@'..=b'_').contains(c)) {
        return None;
    }
    let hex = u16::from_str_radix(&id[3..], 16).ok()?;
    if !id[3..].bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let c1 = u32::from(b[0] - 0x40);
    let c2 = u32::from(b[1] - 0x40);
    let c3 = u32::from(b[2] - 0x40);
    let b0 = (c1 << 2) | (c2 >> 3);
    let b1 = ((c2 & 7) << 5) | c3;
    let b2 = u32::from(hex >> 8);
    let b3 = u32::from(hex & 0xFF);
    Some(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
}

/// Expand a compressed EISA id value back to text ("PNP0A08").
pub fn eisa_id_string(value: u32) -> String {
    let b = value.to_le_bytes();
    let c1 = ((b[0] >> 2) & 0x1F) + 0x40;
    let c2 = (((b[0] & 0x03) << 3) | (b[1] >> 5)) + 0x40;
    let c3 = (b[1] & 0x1F) + 0x40;
    format!(
        "{}{}{}{:02X}{:02X}",
        c1 as char, c2 as char, c3 as char, b[2], b[3]
    )
}

fn header_field<const N: usize>(text: &str) -> [u8; N] {
    let mut out = [0u8; N];
    for (dst, src) in out.iter_mut().zip(text.bytes()) {
        *dst = if (0x20..0x7F).contains(&src) {
            src
        } else {
            b' '
        };
    }
    out
}

/// Sum of all bytes modulo 256 (zero for a valid table).
pub fn byte_sum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

/// Build a complete ACPI table: 36-byte header + body, with length and checksum.
pub fn table(
    signature: &[u8; 4],
    revision: u8,
    oem_id: &str,
    oem_table_id: &str,
    oem_revision: u32,
    body: &[u8],
) -> Vec<u8> {
    let len = HEADER_LEN + body.len();
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(signature);
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.push(revision);
    out.push(0); // checksum, fixed below
    out.extend_from_slice(&header_field::<6>(oem_id));
    out.extend_from_slice(&header_field::<8>(oem_table_id));
    out.extend_from_slice(&oem_revision.to_le_bytes());
    out.extend_from_slice(&CREATOR_ID);
    out.extend_from_slice(&CREATOR_REVISION.to_le_bytes());
    out.extend_from_slice(body);
    out[9] = 0u8.wrapping_sub(byte_sum(&out));
    out
}

/// Build a complete SSDT table: 36-byte header (signature "SSDT", revision 2,
/// OEM id, OEM table id, OEM revision, creator "INTL" 0x20200925) + body,
/// with correct length and checksum.
pub fn definition_block(
    oem_id: &str,
    oem_table_id: &str,
    oem_revision: u32,
    body: &[u8],
) -> Vec<u8> {
    table(b"SSDT", 2, oem_id, oem_table_id, oem_revision, body)
}

// ── Term tree ───────────────────────────────────────────────────────────────

/// Object types for `External` declarations (ACPI ObjectType values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjType {
    Unknown,
    Int,
    Str,
    Buffer,
    Package,
    FieldUnit,
    Device,
    Event,
    Method,
    Mutex,
    OpRegion,
    PowerRes,
    Processor,
    ThermalZone,
    BufferField,
}

impl ObjType {
    pub fn code(self) -> u8 {
        match self {
            ObjType::Unknown => 0,
            ObjType::Int => 1,
            ObjType::Str => 2,
            ObjType::Buffer => 3,
            ObjType::Package => 4,
            ObjType::FieldUnit => 5,
            ObjType::Device => 6,
            ObjType::Event => 7,
            ObjType::Method => 8,
            ObjType::Mutex => 9,
            ObjType::OpRegion => 10,
            ObjType::PowerRes => 11,
            ObjType::Processor => 12,
            ObjType::ThermalZone => 13,
            ObjType::BufferField => 14,
        }
    }

    pub fn asl(self) -> &'static str {
        match self {
            ObjType::Unknown => "UnknownObj",
            ObjType::Int => "IntObj",
            ObjType::Str => "StrObj",
            ObjType::Buffer => "BuffObj",
            ObjType::Package => "PkgObj",
            ObjType::FieldUnit => "FieldUnitObj",
            ObjType::Device => "DeviceObj",
            ObjType::Event => "EventObj",
            ObjType::Method => "MethodObj",
            ObjType::Mutex => "MutexObj",
            ObjType::OpRegion => "OpRegionObj",
            ObjType::PowerRes => "PowerResObj",
            ObjType::Processor => "ProcessorObj",
            ObjType::ThermalZone => "ThermalZoneObj",
            ObjType::BufferField => "BuffFieldObj",
        }
    }
}

/// Match operators (`MTR`, `MEQ`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOp {
    Mtr,
    Meq,
    Mle,
    Mlt,
    Mge,
    Mgt,
}

impl MatchOp {
    fn code(self) -> u8 {
        match self {
            MatchOp::Mtr => 0,
            MatchOp::Meq => 1,
            MatchOp::Mle => 2,
            MatchOp::Mlt => 3,
            MatchOp::Mge => 4,
            MatchOp::Mgt => 5,
        }
    }

    fn asl(self) -> &'static str {
        match self {
            MatchOp::Mtr => "MTR",
            MatchOp::Meq => "MEQ",
            MatchOp::Mle => "MLE",
            MatchOp::Mlt => "MLT",
            MatchOp::Mge => "MGE",
            MatchOp::Mgt => "MGT",
        }
    }
}

/// Small resource descriptors used by the generated `_CRS` buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// `IO (Decode16, min, max, align, len)`.
    Io {
        min: u16,
        max: u16,
        align: u8,
        len: u8,
    },
    /// `IRQNoFlags () {n,...}` as a 16-bit IRQ mask.
    IrqNoFlags { mask: u16 },
    /// `IRQ (Edge|Level, ActiveHigh|ActiveLow, sharing, ) {n,...}`; `flags`
    /// as encoded (bit 0 edge, bit 3 active low, bits 4-5 shared/wake).
    Irq { mask: u16, flags: u8 },
    /// `Memory32Fixed (ReadWrite|ReadOnly, base, len)`.
    Memory32Fixed { writable: bool, base: u32, len: u32 },
}

/// Decode a resource template made only of the descriptors [`Resource`]
/// can express (up to the End tag). None for anything else.
pub fn decode_resources(buf: &[u8]) -> Option<Vec<Resource>> {
    let u16_at = |d: &[u8], i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
    let u32_at = |d: &[u8], i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        match *buf.get(i)? {
            0x47 => {
                let d = buf.get(i + 1..i + 8)?;
                if d[0] != 0x01 {
                    return None;
                }
                out.push(Resource::Io {
                    min: u16_at(d, 1),
                    max: u16_at(d, 3),
                    align: d[5],
                    len: d[6],
                });
                i += 8;
            }
            0x22 => {
                let d = buf.get(i + 1..i + 3)?;
                out.push(Resource::IrqNoFlags { mask: u16_at(d, 0) });
                i += 3;
            }
            0x23 => {
                let d = buf.get(i + 1..i + 4)?;
                if d[2] & 0xC6 != 0 {
                    return None;
                }
                out.push(Resource::Irq {
                    mask: u16_at(d, 0),
                    flags: d[2],
                });
                i += 4;
            }
            0x86 => {
                let d = buf.get(i + 1..i + 12)?;
                if d[0..2] != [0x09, 0x00] || d[2] & 0xFE != 0 {
                    return None;
                }
                out.push(Resource::Memory32Fixed {
                    writable: d[2] == 1,
                    base: u32_at(d, 3),
                    len: u32_at(d, 7),
                });
                i += 12;
            }
            0x79 => return Some(out),
            _ => return None,
        }
    }
}

fn irq_list(mask: u16) -> String {
    let irqs: Vec<String> = (0..16)
        .filter(|b| mask & (1 << b) != 0)
        .map(|b| b.to_string())
        .collect();
    irqs.join(",")
}

impl Resource {
    fn encode(&self, out: &mut Vec<u8>) {
        match *self {
            Resource::Io {
                min,
                max,
                align,
                len,
            } => {
                out.extend_from_slice(&[0x47, 0x01]);
                out.extend_from_slice(&min.to_le_bytes());
                out.extend_from_slice(&max.to_le_bytes());
                out.push(align);
                out.push(len);
            }
            Resource::IrqNoFlags { mask } => {
                out.push(0x22);
                out.extend_from_slice(&mask.to_le_bytes());
            }
            Resource::Irq { mask, flags } => {
                out.push(0x23);
                out.extend_from_slice(&mask.to_le_bytes());
                out.push(flags);
            }
            Resource::Memory32Fixed {
                writable,
                base,
                len,
            } => {
                out.extend_from_slice(&[0x86, 0x09, 0x00, u8::from(writable)]);
                out.extend_from_slice(&base.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
        }
    }

    fn asl(&self, ind: &str) -> String {
        match *self {
            Resource::Io { min, max, align, len } => format!(
                "{ind}IO (Decode16,\n{ind}    0x{min:04X},             // Range Minimum\n{ind}    0x{max:04X},             // Range Maximum\n{ind}    0x{align:02X},               // Alignment\n{ind}    0x{len:02X},               // Length\n{ind}    )\n"
            ),
            Resource::IrqNoFlags { mask } => format!("{ind}IRQNoFlags ()\n{ind}    {{{}}}\n", irq_list(mask)),
            Resource::Irq { mask, flags } => {
                let trigger = if flags & 0x01 != 0 { "Edge" } else { "Level" };
                let polarity = if flags & 0x08 != 0 { "ActiveLow" } else { "ActiveHigh" };
                let sharing = match (flags >> 4) & 0x03 {
                    0 => "Exclusive",
                    1 => "Shared",
                    2 => "ExclusiveAndWake",
                    _ => "SharedAndWake",
                };
                format!("{ind}IRQ ({trigger}, {polarity}, {sharing}, )\n{ind}    {{{}}}\n", irq_list(mask))
            }
            Resource::Memory32Fixed { writable, base, len } => format!(
                "{ind}Memory32Fixed ({},\n{ind}    0x{base:08X},         // Address Base\n{ind}    0x{len:08X},         // Address Length\n{ind}    )\n",
                if writable { "ReadWrite" } else { "ReadOnly" }
            ),
        }
    }
}

/// One entry of a `Field` list: a named unit or reserved bits (`name: None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldUnit {
    pub name: Option<String>,
    pub bits: u32,
}

/// A node of the table description. Statement-like variants appear in term
/// lists, the others as operands.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    /// ASL-only comment line(s); encodes to nothing.
    Comment(String),
    External {
        path: String,
        kind: ObjType,
        args: u8,
    },
    Scope {
        path: String,
        body: Vec<Term>,
    },
    Device {
        name: String,
        body: Vec<Term>,
    },
    Processor {
        name: String,
        id: u8,
        pblk: u32,
        pblk_len: u8,
        body: Vec<Term>,
    },
    Method {
        name: String,
        args: u8,
        serialized: bool,
        body: Vec<Term>,
    },
    Name {
        name: String,
        value: Box<Term>,
    },
    /// `OperationRegion (name, space, offset, length)`; space 0 = SystemMemory,
    /// 1 = SystemIO, 2 = PCI_Config.
    OpRegion {
        name: String,
        space: u8,
        offset: Box<Term>,
        len: Box<Term>,
    },
    /// `Field (region, flags) { units }`; flags as encoded in AML.
    Field {
        region: String,
        flags: u8,
        units: Vec<FieldUnit>,
    },
    If {
        predicate: Box<Term>,
        then: Vec<Term>,
        otherwise: Option<Vec<Term>>,
    },
    Return(Box<Term>),
    Store {
        value: Box<Term>,
        target: Box<Term>,
    },
    Int(u64),
    Str(String),
    Buffer(Vec<u8>),
    Package(Vec<Term>),
    EisaId(String),
    Resources(Vec<Resource>),
    /// Reference to a named object.
    Path(String),
    /// Method invocation.
    Call {
        path: String,
        args: Vec<Term>,
    },
    Arg(u8),
    Local(u8),
    LNot(Box<Term>),
    LAnd(Box<Term>, Box<Term>),
    LOr(Box<Term>, Box<Term>),
    LEqual(Box<Term>, Box<Term>),
    LNotEqual(Box<Term>, Box<Term>),
    LGreater(Box<Term>, Box<Term>),
    CondRefOf(Box<Term>),
    Match {
        package: Box<Term>,
        op1: MatchOp,
        operand1: Box<Term>,
        op2: MatchOp,
        operand2: Box<Term>,
        start: Box<Term>,
    },
}

// Convenience constructors keep the SSDT templates readable.
impl Term {
    pub fn int(v: u64) -> Term {
        Term::Int(v)
    }
    pub fn str(s: &str) -> Term {
        Term::Str(s.to_string())
    }
    pub fn path(p: &str) -> Term {
        Term::Path(p.to_string())
    }
    pub fn call(p: &str, args: Vec<Term>) -> Term {
        Term::Call {
            path: p.to_string(),
            args,
        }
    }
    pub fn name(n: &str, value: Term) -> Term {
        Term::Name {
            name: n.to_string(),
            value: Box::new(value),
        }
    }
    pub fn external(p: &str, kind: ObjType) -> Term {
        Term::External {
            path: p.to_string(),
            kind,
            args: 0,
        }
    }
    pub fn scope(p: &str, body: Vec<Term>) -> Term {
        Term::Scope {
            path: p.to_string(),
            body,
        }
    }
    pub fn device(n: &str, body: Vec<Term>) -> Term {
        Term::Device {
            name: n.to_string(),
            body,
        }
    }
    pub fn method(n: &str, args: u8, body: Vec<Term>) -> Term {
        Term::Method {
            name: n.to_string(),
            args,
            serialized: false,
            body,
        }
    }
    pub fn if_then(predicate: Term, then: Vec<Term>) -> Term {
        Term::If {
            predicate: Box::new(predicate),
            then,
            otherwise: None,
        }
    }
    pub fn if_else(predicate: Term, then: Vec<Term>, otherwise: Vec<Term>) -> Term {
        Term::If {
            predicate: Box::new(predicate),
            then,
            otherwise: Some(otherwise),
        }
    }
    pub fn ret(v: Term) -> Term {
        Term::Return(Box::new(v))
    }
    pub fn store(value: Term, target: Term) -> Term {
        Term::Store {
            value: Box::new(value),
            target: Box::new(target),
        }
    }
    pub fn lnot(a: Term) -> Term {
        Term::LNot(Box::new(a))
    }
    pub fn and(a: Term, b: Term) -> Term {
        Term::LAnd(Box::new(a), Box::new(b))
    }
    pub fn equal(a: Term, b: Term) -> Term {
        Term::LEqual(Box::new(a), Box::new(b))
    }
    pub fn not_equal(a: Term, b: Term) -> Term {
        Term::LNotEqual(Box::new(a), Box::new(b))
    }
    pub fn cond_ref_of(p: &str) -> Term {
        Term::CondRefOf(Box::new(Term::path(p)))
    }
}

fn encode_list(terms: &[Term]) -> Vec<u8> {
    let mut out = Vec::new();
    for t in terms {
        t.encode(&mut out);
    }
    out
}

/// opcode + PkgLength + payload.
fn encode_pkg(out: &mut Vec<u8>, opcode: &[u8], payload: &[u8]) {
    out.extend_from_slice(opcode);
    out.extend(pkg_length(payload.len()));
    out.extend_from_slice(payload);
}

impl Term {
    /// Append the AML encoding of this term to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Term::Comment(_) => {}
            Term::External { path, kind, args } => {
                out.push(0x15);
                if path.starts_with('\\') || path.starts_with('^') {
                    out.extend(name_string(path));
                } else {
                    out.extend(name_string(&format!("\\{path}")));
                }
                out.push(kind.code());
                out.push(*args);
            }
            Term::Scope { path, body } => {
                let mut payload = name_string(path);
                payload.extend(encode_list(body));
                encode_pkg(out, &[0x10], &payload);
            }
            Term::Device { name, body } => {
                let mut payload = name_string(name);
                payload.extend(encode_list(body));
                encode_pkg(out, &[0x5B, 0x82], &payload);
            }
            Term::Processor {
                name,
                id,
                pblk,
                pblk_len,
                body,
            } => {
                let mut payload = name_string(name);
                payload.push(*id);
                payload.extend_from_slice(&pblk.to_le_bytes());
                payload.push(*pblk_len);
                payload.extend(encode_list(body));
                encode_pkg(out, &[0x5B, 0x83], &payload);
            }
            Term::Method {
                name,
                args,
                serialized,
                body,
            } => {
                let mut payload = name_string(name);
                payload.push((args & 0x07) | if *serialized { 0x08 } else { 0 });
                payload.extend(encode_list(body));
                encode_pkg(out, &[0x14], &payload);
            }
            Term::Name { name, value } => {
                out.push(0x08);
                out.extend(name_string(name));
                value.encode(out);
            }
            Term::OpRegion {
                name,
                space,
                offset,
                len,
            } => {
                out.extend_from_slice(&[0x5B, 0x80]);
                out.extend(name_string(name));
                out.push(*space);
                offset.encode(out);
                len.encode(out);
            }
            Term::Field {
                region,
                flags,
                units,
            } => {
                let mut payload = name_string(region);
                payload.push(*flags);
                for unit in units {
                    match &unit.name {
                        Some(n) => payload.extend_from_slice(&name_seg(n)),
                        None => payload.push(0x00),
                    }
                    payload.extend(pkg_length_raw(unit.bits as usize));
                }
                encode_pkg(out, &[0x5B, 0x81], &payload);
            }
            Term::If {
                predicate,
                then,
                otherwise,
            } => {
                let mut payload = Vec::new();
                predicate.encode(&mut payload);
                payload.extend(encode_list(then));
                encode_pkg(out, &[0xA0], &payload);
                if let Some(e) = otherwise {
                    encode_pkg(out, &[0xA1], &encode_list(e));
                }
            }
            Term::Return(v) => {
                out.push(0xA4);
                v.encode(out);
            }
            Term::Store { value, target } => {
                out.push(0x70);
                value.encode(out);
                target.encode(out);
            }
            Term::Int(v) => out.extend(integer(*v)),
            Term::Str(s) => out.extend(string(s)),
            Term::Buffer(bytes) => {
                let mut payload = integer(bytes.len() as u64);
                payload.extend_from_slice(bytes);
                encode_pkg(out, &[0x11], &payload);
            }
            Term::Package(items) => {
                let mut payload = vec![items.len().min(255) as u8];
                payload.extend(encode_list(items));
                encode_pkg(out, &[0x12], &payload);
            }
            Term::EisaId(id) => {
                out.push(0x0C);
                out.extend_from_slice(&eisa_id(id).unwrap_or(0).to_le_bytes());
            }
            Term::Resources(list) => {
                let mut data = Vec::new();
                for r in list {
                    r.encode(&mut data);
                }
                data.extend_from_slice(&[0x79, 0x00]);
                Term::Buffer(data).encode(out);
            }
            Term::Path(p) => out.extend(name_string(p)),
            Term::Call { path, args } => {
                out.extend(name_string(path));
                for a in args {
                    a.encode(out);
                }
            }
            Term::Arg(n) => out.push(0x68 + (n & 0x07).min(6)),
            Term::Local(n) => out.push(0x60 + (n & 0x07)),
            Term::LNot(a) => {
                out.push(0x92);
                a.encode(out);
            }
            Term::LAnd(a, b) => binary(out, &[0x90], a, b),
            Term::LOr(a, b) => binary(out, &[0x91], a, b),
            Term::LEqual(a, b) => binary(out, &[0x93], a, b),
            Term::LNotEqual(a, b) => binary(out, &[0x92, 0x93], a, b),
            Term::LGreater(a, b) => binary(out, &[0x94], a, b),
            Term::CondRefOf(target) => {
                out.extend_from_slice(&[0x5B, 0x12]);
                target.encode(out);
                out.push(0x00);
            }
            Term::Match {
                package,
                op1,
                operand1,
                op2,
                operand2,
                start,
            } => {
                out.push(0x89);
                package.encode(out);
                out.push(op1.code());
                operand1.encode(out);
                out.push(op2.code());
                operand2.encode(out);
                start.encode(out);
            }
        }
    }
}

fn binary(out: &mut Vec<u8>, op: &[u8], a: &Term, b: &Term) {
    out.extend_from_slice(op);
    a.encode(out);
    b.encode(out);
}

/// PkgLength-style encoding of a raw value (field unit bit lengths do not
/// count their own bytes).
fn pkg_length_raw(value: usize) -> Vec<u8> {
    if value <= 0x3F {
        return vec![value as u8];
    }
    let count = if value <= 0xFFF {
        2
    } else if value <= 0xF_FFFF {
        3
    } else {
        4
    };
    let mut out = vec![(((count - 1) as u8) << 6) | (value & 0x0F) as u8];
    let mut rest = value >> 4;
    for _ in 1..count {
        out.push((rest & 0xFF) as u8);
        rest >>= 8;
    }
    out
}

/// Absolute NameSeg path of `path` written in `scope`.
fn absolute(path: &str, scope: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let bytes = path.as_bytes();
    let (mut out, rest) = if bytes.first() == Some(&b'\\') {
        (Vec::new(), &path[1..])
    } else {
        let ups = bytes.iter().take_while(|&&b| b == b'^').count();
        (
            scope[..scope.len().saturating_sub(ups)].to_vec(),
            &path[ups..],
        )
    };
    if !rest.is_empty() {
        out.extend(rest.split('.').map(name_seg));
    }
    out
}

/// Paths referenced as operands (method invocations or reads), except
/// `CondRefOf` targets.
fn collect_references(term: &Term, scope: &[[u8; 4]], out: &mut Vec<Vec<[u8; 4]>>) {
    fn each<'a>(
        terms: impl IntoIterator<Item = &'a Term>,
        scope: &[[u8; 4]],
        out: &mut Vec<Vec<[u8; 4]>>,
    ) {
        for t in terms {
            collect_references(t, scope, out);
        }
    }
    match term {
        Term::Scope { path, body } => each(body, &absolute(path, scope), out),
        Term::Device { name, body }
        | Term::Processor { name, body, .. }
        | Term::Method { name, body, .. } => each(body, &absolute(name, scope), out),
        Term::Name { value, .. } => each([value.as_ref()], scope, out),
        Term::OpRegion { offset, len, .. } => each([offset.as_ref(), len.as_ref()], scope, out),
        Term::If {
            predicate,
            then,
            otherwise,
        } => {
            each([predicate.as_ref()], scope, out);
            each(then, scope, out);
            if let Some(e) = otherwise {
                each(e, scope, out);
            }
        }
        Term::Return(v) | Term::LNot(v) => each([v.as_ref()], scope, out),
        Term::Store { value, target } => each([value.as_ref(), target.as_ref()], scope, out),
        Term::Package(items) => each(items, scope, out),
        Term::Path(p) => out.push(absolute(p, scope)),
        Term::Call { path, args } => {
            out.push(absolute(path, scope));
            each(args, scope, out);
        }
        Term::LAnd(a, b)
        | Term::LOr(a, b)
        | Term::LEqual(a, b)
        | Term::LNotEqual(a, b)
        | Term::LGreater(a, b) => each([a.as_ref(), b.as_ref()], scope, out),
        Term::Match {
            package,
            operand1,
            operand2,
            start,
            ..
        } => each(
            [
                package.as_ref(),
                operand1.as_ref(),
                operand2.as_ref(),
                start.as_ref(),
            ],
            scope,
            out,
        ),
        _ => {}
    }
}

/// Encode a definition block body: `External`s first (grouped in an
/// `If (Zero)` block as iasl does), then every other top-level term.
/// Like iasl, `MethodObj` externals are only emitted when the table invokes
/// the method.
pub fn encode_body(terms: &[Term]) -> Vec<u8> {
    let mut referenced = Vec::new();
    for t in terms {
        collect_references(t, &[], &mut referenced);
    }
    let externals: Vec<Term> = terms
        .iter()
        .filter(|t| match t {
            Term::External {
                path,
                kind: ObjType::Method,
                ..
            } => referenced.contains(&absolute(path, &[])),
            Term::External { .. } => true,
            _ => false,
        })
        .cloned()
        .collect();
    let mut out = Vec::new();
    if !externals.is_empty() {
        Term::if_then(Term::Int(0), externals).encode(&mut out);
    }
    for t in terms.iter().filter(|t| !matches!(t, Term::External { .. })) {
        t.encode(&mut out);
    }
    out
}

/// Encode a complete SSDT from a term list.
pub fn encode_table(
    oem_id: &str,
    oem_table_id: &str,
    oem_revision: u32,
    terms: &[Term],
) -> Vec<u8> {
    definition_block(oem_id, oem_table_id, oem_revision, &encode_body(terms))
}

// ── ASL text ────────────────────────────────────────────────────────────────

const INDENT: &str = "    ";

fn indent(level: usize) -> String {
    INDENT.repeat(level)
}

/// ASL spelling of a path: NameSeg padding removed ("\\_SB_.EC__" → "\\_SB.EC").
pub fn asl_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut prefix_len = 0;
    while prefix_len < bytes.len() && (bytes[prefix_len] == b'\\' || bytes[prefix_len] == b'^') {
        prefix_len += 1;
    }
    let (prefix, rest) = path.split_at(prefix_len);
    if rest.is_empty() {
        return prefix.to_string();
    }
    let segs: Vec<String> = rest
        .split('.')
        .map(|s| {
            let seg = String::from_utf8_lossy(&name_seg(s)).into_owned();
            let trimmed = seg.trim_end_matches('_');
            if trimmed.is_empty() {
                "_".to_string()
            } else {
                trimmed.to_string()
            }
        })
        .collect();
    format!("{prefix}{}", segs.join("."))
}

/// ASL spelling of an integer constant, as the iasl disassembler prints it.
pub fn asl_int(v: u64) -> String {
    match v {
        0 => "Zero".into(),
        1 => "One".into(),
        ONES => "Ones".into(),
        v if v <= 0xFF => format!("0x{v:02X}"),
        v if v <= 0xFFFF => format!("0x{v:04X}"),
        v if v <= 0xFFFF_FFFF => format!("0x{v:08X}"),
        v => format!("0x{v:016X}"),
    }
}

fn asl_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (' '..='~').contains(&c) => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push('"');
    out
}

fn region_space(space: u8) -> String {
    match space {
        0 => "SystemMemory".into(),
        1 => "SystemIO".into(),
        2 => "PCI_Config".into(),
        3 => "EmbeddedControl".into(),
        4 => "SMBus".into(),
        5 => "SystemCMOS".into(),
        6 => "PciBarTarget".into(),
        v => format!("0x{v:02X}"),
    }
}

fn field_flags(flags: u8) -> String {
    let access = match flags & 0x0F {
        0 => "AnyAcc",
        1 => "ByteAcc",
        2 => "WordAcc",
        3 => "DWordAcc",
        4 => "QWordAcc",
        _ => "BufferAcc",
    };
    let lock = if flags & 0x10 != 0 { "Lock" } else { "NoLock" };
    let update = match (flags >> 5) & 0x03 {
        0 => "Preserve",
        1 => "WriteAsOnes",
        _ => "WriteAsZeros",
    };
    format!("{access}, {lock}, {update}")
}

fn binary_asl(a: &Term, op: &str, b: &Term, level: usize) -> String {
    format!("({} {op} {})", expr(a, level), expr(b, level))
}

/// Operand text. Multi-line operands (Package, Buffer, ResourceTemplate)
/// put their braces at `level`.
fn expr(t: &Term, level: usize) -> String {
    let ind = indent(level);
    match t {
        Term::Int(v) => asl_int(*v),
        Term::Str(s) => asl_string(s),
        Term::EisaId(id) => format!("EisaId ({})", asl_string(id)),
        Term::Path(p) => asl_path(p),
        Term::Call { path, args } => {
            let args: Vec<String> = args.iter().map(|a| expr(a, level)).collect();
            format!("{} ({})", asl_path(path), args.join(", "))
        }
        Term::Arg(n) => format!("Arg{n}"),
        Term::Local(n) => format!("Local{n}"),
        Term::LNot(a) => format!("!{}", expr(a, level)),
        Term::LAnd(a, b) => binary_asl(a, "&&", b, level),
        Term::LOr(a, b) => binary_asl(a, "||", b, level),
        Term::LEqual(a, b) => binary_asl(a, "==", b, level),
        Term::LNotEqual(a, b) => binary_asl(a, "!=", b, level),
        Term::LGreater(a, b) => binary_asl(a, ">", b, level),
        Term::CondRefOf(target) => format!("CondRefOf ({})", expr(target, level)),
        Term::Match {
            package,
            op1,
            operand1,
            op2,
            operand2,
            start,
        } => format!(
            "Match ({}, {}, {}, {}, {}, {})",
            expr(package, level),
            op1.asl(),
            expr(operand1, level),
            op2.asl(),
            expr(operand2, level),
            expr(start, level)
        ),
        Term::Buffer(bytes) => {
            let mut s = format!("Buffer ({})\n{ind}{{\n", asl_int(bytes.len() as u64));
            for chunk in bytes.chunks(8) {
                let line: Vec<String> = chunk.iter().map(|b| format!("0x{b:02X}")).collect();
                let _ = writeln!(s, "{ind}{INDENT}{}", line.join(", "));
            }
            let _ = write!(s, "{ind}}}");
            s
        }
        Term::Package(items) => {
            let mut s = format!("Package (0x{:02X})\n{ind}{{\n", items.len().min(255));
            let inner = indent(level + 1);
            for (i, item) in items.iter().enumerate() {
                let sep = if i + 1 < items.len() { "," } else { "" };
                let _ = writeln!(s, "{inner}{}{sep}", expr(item, level + 1));
            }
            let _ = write!(s, "{ind}}}");
            s
        }
        Term::Resources(list) => {
            let mut s = format!("ResourceTemplate ()\n{ind}{{\n");
            for r in list {
                s.push_str(&r.asl(&indent(level + 1)));
            }
            let _ = write!(s, "{ind}}}");
            s
        }
        // Statements never appear as operands in the generated tables.
        other => {
            let mut s = String::new();
            write_stmt(other, &mut s, level);
            s.trim().to_string()
        }
    }
}

fn write_block(out: &mut String, header: &str, body: &[Term], level: usize) {
    let ind = indent(level);
    let _ = writeln!(out, "{ind}{header}");
    let _ = writeln!(out, "{ind}{{");
    for t in body {
        write_stmt(t, out, level + 1);
    }
    let _ = writeln!(out, "{ind}}}");
}

fn write_stmt(t: &Term, out: &mut String, level: usize) {
    let ind = indent(level);
    match t {
        Term::Comment(text) => {
            for line in text.lines() {
                if line.is_empty() {
                    let _ = writeln!(out, "{ind}//");
                } else {
                    let _ = writeln!(out, "{ind}// {line}");
                }
            }
        }
        Term::External { path, kind, args } => {
            let mut line = format!("{ind}External ({}, {})", asl_path(path), kind.asl());
            if *args > 0 {
                let _ = write!(line, "    // {args} Arguments");
            }
            let _ = writeln!(out, "{line}");
        }
        Term::Scope { path, body } => {
            write_block(out, &format!("Scope ({})", asl_path(path)), body, level)
        }
        Term::Device { name, body } => {
            write_block(out, &format!("Device ({})", asl_path(name)), body, level)
        }
        Term::Processor {
            name,
            id,
            pblk,
            pblk_len,
            body,
        } => write_block(
            out,
            &format!(
                "Processor ({}, 0x{id:02X}, 0x{pblk:08X}, 0x{pblk_len:02X})",
                asl_path(name)
            ),
            body,
            level,
        ),
        Term::Method {
            name,
            args,
            serialized,
            body,
        } => write_block(
            out,
            &format!(
                "Method ({}, {args}, {})",
                asl_path(name),
                if *serialized {
                    "Serialized"
                } else {
                    "NotSerialized"
                }
            ),
            body,
            level,
        ),
        Term::Name { name, value } => {
            let _ = writeln!(
                out,
                "{ind}Name ({}, {})",
                asl_path(name),
                expr(value, level)
            );
        }
        Term::OpRegion {
            name,
            space,
            offset,
            len,
        } => {
            let _ = writeln!(
                out,
                "{ind}OperationRegion ({}, {}, {}, {})",
                asl_path(name),
                region_space(*space),
                expr(offset, level),
                expr(len, level)
            );
        }
        Term::Field {
            region,
            flags,
            units,
        } => {
            let _ = writeln!(
                out,
                "{ind}Field ({}, {})",
                asl_path(region),
                field_flags(*flags)
            );
            let _ = writeln!(out, "{ind}{{");
            for (i, unit) in units.iter().enumerate() {
                let sep = if i + 1 < units.len() { "," } else { "" };
                let name = unit.name.as_deref().map(asl_path).unwrap_or_default();
                let _ = writeln!(out, "{ind}{INDENT}{name},   {}{sep}", unit.bits);
            }
            let _ = writeln!(out, "{ind}}}");
        }
        Term::If {
            predicate,
            then,
            otherwise,
        } => {
            write_block(
                out,
                &format!("If ({})", expr(predicate, level)),
                then,
                level,
            );
            if let Some(e) = otherwise {
                write_block(out, "Else", e, level);
            }
        }
        Term::Return(v) => {
            let _ = writeln!(out, "{ind}Return ({})", expr(v, level));
        }
        Term::Store { value, target } => {
            let _ = writeln!(out, "{ind}{} = {}", expr(target, level), expr(value, level));
        }
        other => {
            let _ = writeln!(out, "{ind}{}", expr(other, level));
        }
    }
}

/// ASL source equivalent to [`encode_table`] for the same arguments.
/// `comment` lines are emitted in a block comment before the definition block.
pub fn asl_table(
    oem_id: &str,
    oem_table_id: &str,
    oem_revision: u32,
    comment: &str,
    terms: &[Term],
) -> String {
    let mut out = String::new();
    if !comment.is_empty() {
        out.push_str("/*\n");
        for line in comment.lines() {
            if line.is_empty() {
                out.push_str(" *\n");
            } else {
                // Table ids quoted in the text must not close the comment.
                let _ = writeln!(out, " * {}", line.replace("*/", "* /"));
            }
        }
        out.push_str(" */\n");
    }
    let _ = writeln!(
        out,
        "DefinitionBlock (\"\", \"SSDT\", 2, {}, {}, 0x{oem_revision:08X})",
        asl_string(oem_id),
        asl_string(oem_table_id)
    );
    out.push_str("{\n");
    let (externals, rest): (Vec<&Term>, Vec<&Term>) = terms
        .iter()
        .partition(|t| matches!(t, Term::External { .. }));
    for t in &externals {
        write_stmt(t, &mut out, 1);
    }
    if !externals.is_empty() && !rest.is_empty() {
        out.push('\n');
    }
    for (i, t) in rest.iter().enumerate() {
        if i > 0 && !matches!(rest[i - 1], Term::Comment(_)) {
            out.push('\n');
        }
        write_stmt(t, &mut out, 1);
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkg_length_boundaries() {
        assert_eq!(pkg_length(0), vec![0x01]);
        assert_eq!(pkg_length(0x3D), vec![0x3E]);
        assert_eq!(pkg_length(0x3E), vec![0x3F]);
        assert_eq!(pkg_length(0x3F), vec![0x41, 0x04]);
        // Values seen in iasl output: 0x0468 total → 0x48 0x06.
        assert_eq!(pkg_length(0x66), vec![0x48, 0x06]);
        assert_eq!(pkg_length(0xFFD), vec![0x4F, 0xFF]);
        assert_eq!(pkg_length(0xFFE), vec![0x81, 0x00, 0x01]);
        assert_eq!(pkg_length(0xFFFFC), vec![0x8F, 0xFF, 0xFF]);
        assert_eq!(pkg_length(0xFFFFD), vec![0xC1, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn name_strings() {
        assert_eq!(name_string("EC"), b"EC__".to_vec());
        assert_eq!(name_string("\\_SB"), b"\\_SB_".to_vec());
        assert_eq!(name_string("\\"), vec![b'\\', 0x00]);
        assert_eq!(name_string("^PCI0"), b"^PCI0".to_vec());
        assert_eq!(
            name_string("^^XHC.RHUB"),
            [b"^^".as_slice(), &[0x2E], b"XHC_RHUB"].concat()
        );
        assert_eq!(
            name_string("\\_SB.PCI0"),
            [b"\\".as_slice(), &[0x2E], b"_SB_PCI0"].concat()
        );
        assert_eq!(
            name_string("\\_SB.PCI0.LPCB"),
            [b"\\".as_slice(), &[0x2F, 0x03], b"_SB_PCI0LPCB"].concat()
        );
        assert_eq!(name_string("_sb.pci0"), [&[0x2E][..], b"_SB_PCI0"].concat());
    }

    #[test]
    fn path_validation() {
        assert!(is_valid_path("\\_SB.PCI0.LPCB"));
        assert!(is_valid_path("\\"));
        assert!(is_valid_path("^^EC0"));
        assert!(is_valid_path("_SB_.PC00"));
        assert!(!is_valid_path(""));
        assert!(!is_valid_path("\\_SB..PCI0"));
        assert!(!is_valid_path("\\_SB.PCI00"));
        assert!(!is_valid_path("\\_SB.0PCI"));
        assert!(!is_valid_path("\\_SB.PC\"0"));
        assert!(!is_valid_path("\\^_SB"));
    }

    #[test]
    fn integers_like_iasl() {
        assert_eq!(integer(0), vec![0x00]);
        assert_eq!(integer(1), vec![0x01]);
        assert_eq!(integer(ONES), vec![0xFF]);
        assert_eq!(integer(0x0F), vec![0x0A, 0x0F]);
        assert_eq!(integer(0x1234), vec![0x0B, 0x34, 0x12]);
        assert_eq!(integer(0x1234_5678), vec![0x0C, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(
            integer(0x12_3456_789A),
            vec![0x0E, 0x9A, 0x78, 0x56, 0x34, 0x12, 0, 0, 0]
        );
    }

    #[test]
    fn eisa_ids_round_trip() {
        assert_eq!(eisa_id("PNP0B00"), Some(0x000B_D041));
        assert_eq!(eisa_id("PNP0A08"), Some(0x080A_D041));
        assert_eq!(eisa_id_string(0x080A_D041), "PNP0A08");
        let app = eisa_id("APP9876").expect("valid id");
        assert_eq!(eisa_id_string(app), "APP9876");
        assert_eq!(eisa_id("PNP0A0"), None);
        assert_eq!(eisa_id("pnp0a08"), None);
        assert_eq!(eisa_id("PNP0AXY"), None);
    }

    #[test]
    fn table_header_and_checksum() {
        let t = definition_block("OCLICK", "TEST", 0x1000, &[0x5B, 0x82]);
        assert_eq!(&t[0..4], b"SSDT");
        assert_eq!(
            u32::from_le_bytes([t[4], t[5], t[6], t[7]]) as usize,
            t.len()
        );
        assert_eq!(t[8], 2);
        assert_eq!(&t[10..16], b"OCLICK");
        assert_eq!(&t[16..24], b"TEST\0\0\0\0");
        assert_eq!(&t[28..32], b"INTL");
        assert_eq!(&t[32..36], &[0x25, 0x09, 0x20, 0x20]);
        assert_eq!(byte_sum(&t), 0);
    }

    #[test]
    fn externals_grouped_like_iasl() {
        // Bytes produced by iasl 20200925 for:
        //   External (_SB_.PCI0, DeviceObj)
        //   External (STAS, IntObj)
        //   Scope (\) { }
        let terms = vec![
            Term::external("\\_SB.PCI0", ObjType::Device),
            Term::external("STAS", ObjType::Int),
            Term::scope("\\", vec![]),
        ];
        let body = encode_body(&terms);
        let expected: Vec<u8> = [
            &[0xA0, 0x17, 0x00][..],
            &[0x15, b'\\', 0x2E],
            b"_SB_PCI0",
            &[0x06, 0x00, 0x15, b'\\'],
            b"STAS",
            &[0x01, 0x00],
            &[0x10, 0x03, b'\\', 0x00],
        ]
        .concat();
        assert_eq!(body, expected);
    }

    #[test]
    fn processor_and_dsm_like_iasl() {
        // iasl output for Processor (C000, 0x00, 0x00000510, 0x06) { Name (_UID, Zero) }
        let t = Term::Processor {
            name: "C000".into(),
            id: 0,
            pblk: 0x510,
            pblk_len: 6,
            body: vec![Term::name("_UID", Term::int(0))],
        };
        let mut out = Vec::new();
        t.encode(&mut out);
        assert_eq!(
            out,
            [
                &[0x5B, 0x83, 0x11][..],
                b"C000",
                &[0x00, 0x10, 0x05, 0x00, 0x00, 0x06, 0x08],
                b"_UID",
                &[0x00]
            ]
            .concat()
        );
    }

    #[test]
    fn resource_template_encoding() {
        let t = Term::Resources(vec![
            Resource::Io {
                min: 0x70,
                max: 0x70,
                align: 1,
                len: 8,
            },
            Resource::IrqNoFlags { mask: 1 << 8 },
            Resource::Memory32Fixed {
                writable: true,
                base: 0xFE00_0000,
                len: 0x1_0000,
            },
        ]);
        let mut out = Vec::new();
        t.encode(&mut out);
        let expected = vec![
            0x11, 0x1C, 0x0A, 0x19, 0x47, 0x01, 0x70, 0x00, 0x70, 0x00, 0x01, 0x08, 0x22, 0x00,
            0x01, 0x86, 0x09, 0x00, 0x01, 0x00, 0x00, 0x00, 0xFE, 0x00, 0x00, 0x01, 0x00, 0x79,
            0x00,
        ];
        assert_eq!(out, expected);
    }

    #[test]
    fn irq_descriptors_like_iasl() {
        // iasl output for IRQ (Edge, ActiveHigh, Exclusive, ) {8},
        // IRQ (Level, ActiveLow, Shared, ) {0,8}, IRQNoFlags () {}.
        let list = vec![
            Resource::Irq {
                mask: 1 << 8,
                flags: 0x01,
            },
            Resource::Irq {
                mask: 0x0101,
                flags: 0x18,
            },
            Resource::IrqNoFlags { mask: 0 },
        ];
        let mut out = Vec::new();
        Term::Resources(list.clone()).encode(&mut out);
        assert_eq!(
            out,
            vec![
                0x11, 0x10, 0x0A, 0x0D, 0x23, 0x00, 0x01, 0x01, 0x23, 0x01, 0x01, 0x18, 0x22, 0x00,
                0x00, 0x79, 0x00
            ]
        );
        assert_eq!(decode_resources(&out[4..]), Some(list));
        let text = Resource::Irq {
            mask: 0x0101,
            flags: 0x18,
        }
        .asl("");
        assert_eq!(text, "IRQ (Level, ActiveLow, Shared, )\n    {0,8}\n");
    }

    #[test]
    fn resource_decoding_rejects_unknown_descriptors() {
        let io = [0x47, 0x01, 0x70, 0x00, 0x70, 0x00, 0x01, 0x02, 0x79, 0x00];
        assert_eq!(
            decode_resources(&io),
            Some(vec![Resource::Io {
                min: 0x70,
                max: 0x70,
                align: 1,
                len: 2
            }])
        );
        // Decode10 IO, a FixedIO descriptor, truncation and a missing End tag.
        assert_eq!(
            decode_resources(&[0x47, 0x00, 0x70, 0x00, 0x70, 0x00, 0x01, 0x02, 0x79, 0x00]),
            None
        );
        assert_eq!(
            decode_resources(&[0x4B, 0x70, 0x00, 0x02, 0x79, 0x00]),
            None
        );
        assert_eq!(decode_resources(&io[..5]), None);
        assert_eq!(decode_resources(&io[..8]), None);
    }

    #[test]
    fn match_and_not_equal_like_iasl() {
        // Return ((Ones != Match (Local0, MEQ, Arg0, MTR, Zero, Zero)))
        let t = Term::ret(Term::not_equal(
            Term::int(ONES),
            Term::Match {
                package: Box::new(Term::Local(0)),
                op1: MatchOp::Meq,
                operand1: Box::new(Term::Arg(0)),
                op2: MatchOp::Mtr,
                operand2: Box::new(Term::int(0)),
                start: Box::new(Term::int(0)),
            },
        ));
        let mut out = Vec::new();
        t.encode(&mut out);
        assert_eq!(
            out,
            vec![0xA4, 0x92, 0x93, 0xFF, 0x89, 0x60, 0x01, 0x68, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn asl_text() {
        let terms = vec![
            Term::external("\\_SB.PCI0.LPCB", ObjType::Device),
            Term::scope(
                "\\_SB.PCI0.LPCB",
                vec![Term::device(
                    "EC",
                    vec![
                        Term::name("_HID", Term::str("ACID0001")),
                        Term::method(
                            "_STA",
                            0,
                            vec![Term::if_else(
                                Term::call("_OSI", vec![Term::str("Darwin")]),
                                vec![Term::ret(Term::int(0x0F))],
                                vec![Term::ret(Term::int(0))],
                            )],
                        ),
                    ],
                )],
            ),
        ];
        let asl = asl_table("OCLICK", "SsdtEC", 0x1000, "Fake EC", &terms);
        assert!(
            asl.contains("DefinitionBlock (\"\", \"SSDT\", 2, \"OCLICK\", \"SsdtEC\", 0x00001000)")
        );
        assert!(asl.contains("    External (\\_SB.PCI0.LPCB, DeviceObj)"));
        assert!(asl.contains("        Device (EC)"));
        assert!(asl.contains("If (_OSI (\"Darwin\"))"));
        assert!(asl.contains("Return (0x0F)"));
        assert!(asl.contains("Return (Zero)"));
        assert!(asl.starts_with("/*\n * Fake EC\n */\n"));

        let asl = asl_table("OCLICK", "T", 0, "SSDT \"A*/B\"", &[]);
        assert!(asl.starts_with("/*\n * SSDT \"A* /B\"\n */\n"));
        assert_eq!(asl.matches("*/").count(), 1);
    }

    #[test]
    fn asl_paths_drop_padding() {
        assert_eq!(asl_path("\\_SB_.PCI0.LPC_.EC__"), "\\_SB.PCI0.LPC.EC");
        assert_eq!(asl_path("\\"), "\\");
        assert_eq!(asl_path("^^RHUB"), "^^RHUB");
    }
}
