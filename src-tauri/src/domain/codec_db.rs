//! HD Audio codec database and AppleALC layout-id selection. Data comes from
//! AppleALC 1.9.8 `Resources/*/Info.plist` (`data/applealc_codecs.json`) and,
//! for codecs AppleALC does not support, the Linux HDA codec tables
//! (`data/hda_codec_names.json`). Both files are regenerated with
//! `data/gen_codec_data.py`.
//!
//! Default layout ranking (deterministic, never random). Every layout of the
//! codec gets a tier; the best tier wins, ties go to the generic author bonus,
//! then to the lowest non-zero layout id.
//!
//! Laptops:
//! 1. the comment quotes the exact codec subsystem id;
//! 2. the comment names the subsystem vendor (Dell, HP, Lenovo, ASUS, Acer,
//!    MSI, ... incl. their product lines) and is not a desktop board/PC;
//!    "series" layouts first, then layouts by Mirone/InsanelyDeepak/Toleda;
//! 3. generic laptop layouts: Mirone layout 3, other Mirone "laptop patch"
//!    layouts, InsanelyDeepak 13, other InsanelyDeepak layouts;
//! 4. layouts that name no vendor and no desktop;
//! 5. layouts for another vendor's laptop; 6. desktop layouts.
//!
//! Desktops (and all-in-ones):
//! 1. exact subsystem id;
//! 2. generic desktop layouts: Toleda 1 (5/6 jacks), Mirone 7 (5/6 ports),
//!    Mirone 5 (3 ports), Toleda 3 (3 jacks), other Mirone port layouts,
//!    Toleda 2 (repurposed 5.1);
//! 3. the comment names the board vendor and is not a laptop;
//! 4. any other non-laptop layout; 5. laptop layouts.
//!
//! BIOS pin defaults would allow a better match, but they are not available
//! from Windows; the UI offers the full ranked list for `alcid=` testing.

use once_cell::sync::Lazy;
use serde::Deserialize;

const APPLEALC_JSON: &str = include_str!("data/applealc_codecs.json");
const CODEC_NAMES_JSON: &str = include_str!("data/hda_codec_names.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecInfo {
    /// 0xVVVVDDDD
    pub id: u32,
    /// "Realtek ALC897"
    pub name: String,
    /// All layout ids AppleALC ships for this codec.
    pub layouts: Vec<u32>,
}

#[derive(Deserialize)]
struct AppleAlcFile {
    codecs: Vec<RawCodec>,
}

#[derive(Deserialize)]
struct RawCodec {
    id: String,
    vendor: String,
    name: String,
    layouts: Vec<(u32, String)>,
}

#[derive(Deserialize)]
struct CodecNamesFile {
    names: Vec<(String, String)>,
}

struct Layout {
    id: u32,
    comment: String,
    tokens: Tokens,
}

struct Codec {
    id: u32,
    name: String,
    layouts: Vec<Layout>,
}

fn parse_codec_hex(text: &str) -> Option<u32> {
    let t = text.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 8 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(t, 16).ok()
}

fn load_codecs(json: &str) -> Result<Vec<Codec>, String> {
    let file: AppleAlcFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut codecs = Vec::with_capacity(file.codecs.len());
    for raw in file.codecs {
        let id = parse_codec_hex(&raw.id).ok_or_else(|| format!("bad codec id {}", raw.id))?;
        let mut layouts: Vec<Layout> = raw
            .layouts
            .into_iter()
            .map(|(layout, comment)| Layout {
                id: layout,
                tokens: Tokens::new(&comment),
                comment,
            })
            .collect();
        layouts.sort_by_key(|l| l.id);
        layouts.dedup_by_key(|l| l.id);
        codecs.push(Codec {
            id,
            name: format!("{} {}", raw.vendor, raw.name),
            layouts,
        });
    }
    codecs.sort_by_key(|c| c.id);
    Ok(codecs)
}

fn load_names(json: &str) -> Result<Vec<(u32, String)>, String> {
    let file: CodecNamesFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut names = Vec::with_capacity(file.names.len());
    for (id, name) in file.names {
        let id = parse_codec_hex(&id).ok_or_else(|| format!("bad codec id {id}"))?;
        names.push((id, name));
    }
    names.sort_by_key(|(id, _)| *id);
    Ok(names)
}

static CODECS: Lazy<Vec<Codec>> = Lazy::new(|| {
    load_codecs(APPLEALC_JSON).unwrap_or_else(|e| {
        tracing::error!("embedded AppleALC codec table is invalid: {e}");
        Vec::new()
    })
});

static NAMES: Lazy<Vec<(u32, String)>> = Lazy::new(|| {
    load_names(CODEC_NAMES_JSON).unwrap_or_else(|e| {
        tracing::error!("embedded HDA codec name table is invalid: {e}");
        Vec::new()
    })
});

fn find(codec_id: u32) -> Option<&'static Codec> {
    let codecs: &'static Vec<Codec> = &CODECS;
    codecs
        .binary_search_by_key(&codec_id, |c| c.id)
        .ok()
        .map(|i| &codecs[i])
}

/// Combine a 16-bit vendor and device id into 0xVVVVDDDD.
pub fn codec_id(vendor_id: u16, device_id: u16) -> u32 {
    (u32::from(vendor_id) << 16) | u32::from(device_id)
}

/// Build a codec id from scanner strings ("10ec", "0897").
pub fn parse_codec_id(vendor_id: &str, device_id: &str) -> Option<u32> {
    let vendor = parse_codec_hex(vendor_id).filter(|v| *v <= 0xFFFF)?;
    let device = parse_codec_hex(device_id).filter(|d| *d <= 0xFFFF)?;
    Some((vendor << 16) | device)
}

/// The codec subsystem ("10280798", vendor first as in HDAUDIO `SUBSYS_`).
pub fn parse_subsystem(text: &str) -> Option<u32> {
    let compact: String = text
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '_' | ' '))
        .collect();
    let compact = compact
        .strip_prefix("0x")
        .or_else(|| compact.strip_prefix("0X"))
        .unwrap_or(&compact);
    if compact.len() != 8 {
        return None;
    }
    parse_codec_hex(compact)
}

pub fn lookup(codec_id: u32) -> Option<CodecInfo> {
    find(codec_id).map(|c| CodecInfo {
        id: c.id,
        name: c.name.clone(),
        layouts: c.layouts.iter().map(|l| l.id).collect(),
    })
}

/// True when AppleALC ships at least one layout for the codec.
pub fn is_supported(codec_id: u32) -> bool {
    find(codec_id).is_some_and(|c| !c.layouts.is_empty())
}

/// All layouts of a codec with their AppleALC comments, ascending by id.
pub fn layout_comments(codec_id: u32) -> Vec<(u32, String)> {
    find(codec_id)
        .map(|c| {
            c.layouts
                .iter()
                .map(|l| (l.id, l.comment.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Display name of an HDA codec vendor.
pub fn vendor_name(vendor_id: u16) -> Option<&'static str> {
    Some(match vendor_id {
        0x10EC => "Realtek",
        0x14F1 => "Conexant",
        0x111D => "IDT",
        0x8384 => "SigmaTel",
        0x11D4 => "Analog Devices",
        0x1013 => "Cirrus Logic",
        0x1102 => "Creative",
        0x1106 => "VIA",
        0x8086 => "Intel",
        0x1002 | 0x1022 => "AMD",
        0x10DE => "NVIDIA",
        0x13F6 | 0x434D => "C-Media",
        0x1057 => "Motorola",
        0x11C1 => "LSI",
        0x163C => "Smart Link",
        0x1543 => "Silicon Labs",
        0x1854 => "LG",
        0x17E8 => "Chrontel",
        0x19E5 => "Huawei",
        0x1FA8 => "Senary",
        0x1D17 => "Zhaoxin",
        0x6766 => "Glenfly",
        0x4C54 => "Lisuan",
        0x0014 => "Loongson",
        0x1095 => "Silicon Image",
        0x1AF4 => "Red Hat",
        _ => return None,
    })
}

fn with_vendor(vendor: Option<&str>, name: &str) -> String {
    match vendor {
        Some(v) => {
            let first = v
                .split_whitespace()
                .next()
                .unwrap_or(v)
                .to_ascii_lowercase();
            if name.to_ascii_lowercase().starts_with(&first) {
                name.to_string()
            } else {
                format!("{v} {name}")
            }
        }
        None => name.to_string(),
    }
}

/// Friendly name for any codec id, even ones AppleALC does not support.
pub fn codec_name(codec_id: u32) -> String {
    if let Some(codec) = find(codec_id) {
        return codec.name.clone();
    }
    let vendor_id = (codec_id >> 16) as u16;
    let device_id = (codec_id & 0xFFFF) as u16;
    let vendor = vendor_name(vendor_id);
    let names: &Vec<(u32, String)> = &NAMES;
    if let Ok(i) = names.binary_search_by_key(&codec_id, |(id, _)| *id) {
        return with_vendor(vendor, &names[i].1);
    }
    match vendor_id {
        0x10DE => "NVIDIA HDMI/DP".to_string(),
        0x8086 if is_hdmi_codec(vendor_id, device_id) => "Intel HDMI/DP".to_string(),
        0x1002 => "AMD HDMI/DP".to_string(),
        _ => match vendor {
            Some(v) => format!("{v} codec {device_id:04X}"),
            None => format!("Unknown codec 0x{codec_id:08X}"),
        },
    }
}

/// OEM part numbers printed instead of the Realtek name (Linux
/// `rename_pci_tbl`), keyed by codec id and subsystem vendor.
const OEM_ALIASES: &[(u32, u16, &str)] = &[
    (0x10EC_0280, 0x1028, "ALC3220"),
    (0x10EC_0280, 0x103C, "ALC3228"),
    (0x10EC_0282, 0x1028, "ALC3221"),
    (0x10EC_0282, 0x1043, "ALC3229"),
    (0x10EC_0282, 0x103C, "ALC3227"),
    (0x10EC_0283, 0x1028, "ALC3223"),
    (0x10EC_0283, 0x17AA, "ALC3239"),
    (0x10EC_0288, 0x1028, "ALC3263"),
    (0x10EC_0292, 0x1028, "ALC3226"),
    (0x10EC_0292, 0x17AA, "ALC3232"),
    (0x10EC_0293, 0x1028, "ALC3235"),
    (0x10EC_0255, 0x1028, "ALC3234"),
    (0x10EC_0668, 0x1028, "ALC3661"),
    (0x10EC_0668, 0x103C, "ALC3662"),
    (0x10EC_0275, 0x1028, "ALC3260"),
    (0x10EC_0899, 0x1028, "ALC3861"),
    (0x10EC_0298, 0x1028, "ALC3266"),
    (0x10EC_0236, 0x1028, "ALC3204"),
    (0x10EC_0256, 0x1028, "ALC3246"),
    (0x10EC_0225, 0x1028, "ALC3253"),
    (0x10EC_0295, 0x1028, "ALC3254"),
    (0x10EC_0299, 0x1028, "ALC3271"),
    (0x10EC_0233, 0x1043, "ALC3236"),
    (0x10EC_0286, 0x103C, "ALC3242"),
    (0x10EC_0290, 0x103C, "ALC3241"),
    (0x10EC_0670, 0x1025, "ALC669X"),
    (0x10EC_0676, 0x1025, "ALC679X"),
    (0x10EC_0257, 0x12F0, "ALC3328"),
];

/// OEM alias of a codec for a given codec subsystem (0xVVVVDDDD), e.g.
/// ALC256 on a Dell is sold as "ALC3246".
pub fn oem_alias(codec_id: u32, subsystem: Option<u32>) -> Option<&'static str> {
    let vendor = (subsystem? >> 16) as u16;
    OEM_ALIASES
        .iter()
        .find(|(id, v, _)| *id == codec_id && *v == vendor)
        .map(|(_, _, alias)| *alias)
}

/// "Realtek ALC256 (ALC3246)" when the board vendor uses an OEM alias.
pub fn codec_display_name(codec_id: u32, subsystem: Option<u32>) -> String {
    let name = codec_name(codec_id);
    match oem_alias(codec_id, subsystem) {
        Some(alias) => format!("{name} ({alias})"),
        None => name,
    }
}

/// Resolve a codec typed by the user ("ALC897", "Realtek ALC1220", "ALC3246",
/// "CX20751/2") to its id. Only AppleALC codecs and OEM aliases are known.
pub fn find_codec_by_name(name: &str) -> Option<u32> {
    let wanted = name.trim().to_ascii_lowercase();
    if wanted.is_empty() {
        return None;
    }
    if let Some(&(id, _, _)) = OEM_ALIASES
        .iter()
        .find(|(_, _, alias)| alias.eq_ignore_ascii_case(&wanted))
    {
        return Some(id);
    }
    let codecs: &Vec<Codec> = &CODECS;
    codecs
        .iter()
        .find(|c| {
            let full = c.name.to_ascii_lowercase();
            let short = full.split_once(' ').map(|(_, rest)| rest).unwrap_or(&full);
            full == wanted || short == wanted || name_variants(short).any(|v| v == wanted)
        })
        .map(|c| c.id)
}

/// Expand a combined AppleALC name into its members: "cx20751/2" → cx20751,
/// cx20752; "vt2020/2021" → vt2020, vt2021; "92hd81b1x5/92hd87b1" → both.
fn name_variants(short: &str) -> impl Iterator<Item = String> + '_ {
    let mut parts = short.split('/');
    let first = parts.next().unwrap_or_default();
    std::iter::once(first.to_string()).chain(parts.filter_map(move |part| {
        if part.chars().any(|c| c.is_ascii_alphabetic()) {
            Some(part.to_string())
        } else if !part.is_empty() && part.len() < first.len() {
            first
                .get(..first.len() - part.len())
                .map(|prefix| format!("{prefix}{part}"))
        } else {
            None
        }
    }))
}

/// True for HDMI/DP codecs (Intel 8086:28xx, AMD 1002:aaxx, NVIDIA 10de:xxxx).
/// Neither Intel, AMD nor NVIDIA make analog HDA codecs, so every codec id of
/// those vendors is treated as digital; the other HDMI-only codec vendors
/// (VIA/Zhaoxin/Glenfly/Chrontel/Silicon Image/Lisuan/Loongson) are covered too.
pub fn is_hdmi_codec(vendor_id: u16, device_id: u16) -> bool {
    match vendor_id {
        0x8086 | 0x1002 | 0x10DE => true,
        0x1106 => (0x9F80..=0x9F8F).contains(&device_id),
        0x1D17 | 0x6766 | 0x17E8 | 0x4C54 => true,
        0x1095 => matches!(device_id, 0x1390 | 0x1392),
        0x0014 => device_id == 0x7A47,
        _ => false,
    }
}

/// `is_hdmi_codec` on a combined 0xVVVVDDDD id.
pub fn is_hdmi_codec_id(codec_id: u32) -> bool {
    is_hdmi_codec((codec_id >> 16) as u16, (codec_id & 0xFFFF) as u16)
}

// ── Layout ranking ──────────────────────────────────────────────────────────

struct Tokens {
    lower: String,
    words: Vec<String>,
}

impl Tokens {
    fn new(comment: &str) -> Self {
        let lower = comment.to_lowercase();
        let words = lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        Self { lower, words }
    }

    /// Short tokens must be whole words ("hp", "msi"), longer ones may be part
    /// of a word ("xiaomi" in "XiaoMiAir").
    fn mentions(&self, token: &str) -> bool {
        if token.len() <= 3 {
            self.words.iter().any(|w| w == token)
        } else {
            self.lower.contains(token)
        }
    }

    fn mentions_any(&self, tokens: &[&str]) -> bool {
        tokens.iter().any(|t| self.mentions(t))
    }
}

/// Subsystem vendor → words that identify the OEM (brand and product lines)
/// in AppleALC layout comments.
const OEM_TOKENS: &[(u16, &[&str])] = &[
    (
        0x1028,
        &[
            "dell",
            "optiplex",
            "latitude",
            "inspiron",
            "vostro",
            "precision",
            "xps",
            "alienware",
        ],
    ),
    (
        0x103C,
        &[
            "hp",
            "compaq",
            "elitebook",
            "probook",
            "zbook",
            "pavilion",
            "envy",
            "spectre",
            "omen",
            "elitedesk",
            "prodesk",
        ],
    ),
    (
        0x17AA,
        &[
            "lenovo",
            "thinkpad",
            "ideapad",
            "thinkcentre",
            "ideacentre",
            "thinkbook",
            "yoga",
            "legion",
            "xiaoxin",
            "tianyi",
            "qitian",
            "rescuer",
        ],
    ),
    (
        0x1043,
        &[
            "asus", "vivobook", "zenbook", "rog", "tuf", "strix", "proart", "prime",
        ],
    ),
    (
        0x1025,
        &[
            "acer",
            "aspire",
            "predator",
            "nitro",
            "swift",
            "veriton",
            "travelmate",
        ],
    ),
    (0x1462, &["msi", "mortar", "tomahawk"]),
    (0x1458, &["gigabyte", "aorus", "ga", "brix"]),
    (0x1849, &["asrock", "deskmini", "fatal1ty"]),
    (0x144D, &["samsung", "galaxy"]),
    (0x1179, &["toshiba", "satellite", "dynabook", "portege"]),
    (0x1558, &["clevo", "hasee", "mechrevo", "thunderobot"]),
    (0x1D05, &["tongfang", "mechrevo"]),
    (0x1A58, &["razer"]),
    (0x1414, &["microsoft", "surface"]),
    (0x19E5, &["huawei", "matebook", "honor", "magicbook"]),
    (0x1D72, &["xiaomi", "mibook", "redmibook"]),
    (0x10CF, &["fujitsu", "lifebook", "esprimo", "celsius"]),
    (0x104D, &["sony", "vaio"]),
    (0x17C0, &["medion", "akoya"]),
    (0x8086, &["nuc"]),
    (0x106B, &["imac", "macbook", "macmini", "macpro"]),
];

const LAPTOP_TOKENS: &[&str] = &[
    "laptop",
    "notebook",
    "vivobook",
    "zenbook",
    "thinkpad",
    "ideapad",
    "thinkbook",
    "latitude",
    "inspiron",
    "elitebook",
    "probook",
    "zbook",
    "spectre",
    "envy",
    "omen",
    "matebook",
    "magicbook",
    "macbook",
    "surface",
    "yoga",
    "blade",
    "redmibook",
    "mibook",
    "xiaomi",
    "galaxy",
    "chromebook",
    "nitro",
    "zephyrus",
    "legion",
    "clevo",
    "hasee",
    "mechrevo",
    "thunderobot",
    "x360",
    "2in1",
    "tablet",
    "corebook",
    "air",
    "xps",
    "lifebook",
    "satellite",
    "vaio",
];

const DESKTOP_TOKENS: &[&str] = &[
    "desktop",
    "optiplex",
    "thinkcentre",
    "ideacentre",
    "elitedesk",
    "prodesk",
    "nuc",
    "deskmini",
    "brix",
    "minisforum",
    "aio",
    "all in one",
    "imac",
    "macmini",
    "macpro",
    "veriton",
    "esprimo",
    "mff",
    "sff",
    "tiny",
    "corebox",
    "mortar",
    "tomahawk",
    "itx",
    "matx",
    "alpha",
    "huananzhi",
    "onda",
    "ops",
    "motherboard",
];

/// Intel/AMD desktop chipset numbers by prefix letter ("Z390", "B550M", "H81M").
const CHIPSETS: &[(char, &[u16])] = &[
    (
        'z',
        &[
            68, 75, 77, 87, 97, 170, 270, 370, 390, 490, 590, 690, 790, 890,
        ],
    ),
    (
        'b',
        &[
            75, 85, 150, 250, 350, 360, 365, 450, 460, 550, 560, 650, 660, 760, 850, 860,
        ],
    ),
    (
        'h',
        &[
            55, 57, 61, 67, 77, 81, 87, 97, 110, 170, 270, 310, 370, 410, 470, 510, 570, 610, 670,
            770, 810,
        ],
    ),
    ('x', &[58, 79, 99, 299, 370, 399, 470, 570, 670, 870]),
    ('q', &[77, 87, 170, 270, 370, 470, 570, 670]),
    ('a', &[320, 520, 620]),
];

fn is_chipset_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(prefix) = chars.next() else {
        return false;
    };
    let rest = chars.as_str();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !(2..=3).contains(&digits.len()) || rest.len() - digits.len() > 4 {
        return false;
    }
    let Ok(number) = digits.parse::<u16>() else {
        return false;
    };
    CHIPSETS
        .iter()
        .any(|(letter, numbers)| *letter == prefix && numbers.contains(&number))
}

fn desktop_looking(t: &Tokens) -> bool {
    t.mentions_any(DESKTOP_TOKENS) || t.words.iter().any(|w| is_chipset_word(w))
}

fn laptop_looking(t: &Tokens) -> bool {
    t.mentions_any(LAPTOP_TOKENS)
}

fn named_vendors(t: &Tokens) -> Vec<u16> {
    OEM_TOKENS
        .iter()
        .filter(|(_, tokens)| t.mentions_any(tokens))
        .map(|(vendor, _)| *vendor)
        .collect()
}

fn generic_author(t: &Tokens) -> bool {
    t.mentions_any(&["mirone", "insanelydeepak", "toleda"])
}

fn desktop_generic(id: u32, t: &Tokens) -> Option<u8> {
    if t.mentions("laptop") {
        return None;
    }
    let toleda = t.mentions("toleda");
    let mirone = t.mentions("mirone");
    match id {
        1 if toleda => Some(6),
        7 if mirone => Some(5),
        5 if mirone => Some(4),
        3 if toleda => Some(3),
        _ if mirone && t.mentions("ports") => Some(2),
        2 if toleda => Some(1),
        _ => None,
    }
}

fn laptop_generic(id: u32, t: &Tokens) -> Option<u8> {
    let mirone = t.mentions("mirone");
    let deepak = t.mentions("insanelydeepak");
    match id {
        3 if mirone => Some(4),
        _ if mirone && t.mentions("laptop") => Some(3),
        13 if deepak => Some(2),
        _ if deepak => Some(1),
        _ => None,
    }
}

fn rank(layout: &Layout, subsystem: Option<u32>, is_laptop: bool) -> (u8, u8) {
    let t = &layout.tokens;
    let oem = subsystem
        .map(|s| (s >> 16) as u16)
        .filter(|v| *v != 0 && *v != 0xFFFF);
    if let (Some(ssid), Some(_)) = (subsystem, oem) {
        if t.lower.contains(&format!("{ssid:08x}")) {
            return (7, 0);
        }
    }
    let named = named_vendors(t);
    let vendor_match = oem.is_some_and(|v| named.contains(&v));
    let series_bonus = if t.mentions("series") { 2 } else { 0 } + u8::from(generic_author(t));
    if is_laptop {
        let desk = desktop_looking(t);
        if vendor_match && !desk {
            (6, series_bonus)
        } else if let Some(bonus) = laptop_generic(layout.id, t) {
            (5, bonus)
        } else if !desk && named.is_empty() {
            (4, 0)
        } else if !desk {
            (3, 0)
        } else if vendor_match {
            (2, 0)
        } else {
            (1, 0)
        }
    } else {
        let lap = laptop_looking(t);
        if let Some(bonus) = desktop_generic(layout.id, t) {
            (6, bonus)
        } else if vendor_match && !lap {
            (5, series_bonus)
        } else if !lap {
            (3, 0)
        } else if vendor_match {
            (2, 0)
        } else {
            (1, 0)
        }
    }
}

/// Every layout of the codec, best first (see the module docs for the order).
/// `subsystem` is the codec subsystem id 0xVVVVDDDD (vendor in the high half).
pub fn ranked_layouts(codec_id: u32, subsystem: Option<u32>, is_laptop: bool) -> Vec<u32> {
    let Some(codec) = find(codec_id) else {
        return Vec::new();
    };
    let mut scored: Vec<((u8, u8), u32)> = codec
        .layouts
        .iter()
        .map(|l| (rank(l, subsystem, is_laptop), l.id))
        .collect();
    scored.sort_by(|(rank_a, id_a), (rank_b, id_b)| {
        rank_b
            .cmp(rank_a)
            .then_with(|| (*id_b != 0).cmp(&(*id_a != 0)))
            .then_with(|| id_a.cmp(id_b))
    });
    scored.into_iter().map(|(_, id)| id).collect()
}

/// Prefer comments naming the scanned chassis, without treating nearby model
/// numbers (such as T430/T430s or Y530/Y540) as interchangeable.
pub fn ranked_layouts_for_model(
    codec_id: u32,
    subsystem: Option<u32>,
    is_laptop: bool,
    model: Option<&str>,
) -> Vec<u32> {
    let mut layouts = ranked_layouts(codec_id, subsystem, is_laptop);
    let Some(model) = model.filter(|s| !s.trim().is_empty()) else {
        return layouts;
    };
    let tokens = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 4 && w.chars().any(|c| c.is_ascii_digit()))
            .map(str::to_ascii_lowercase)
            .collect()
    };
    let model_tokens = tokens(model);
    if model_tokens.is_empty() {
        return layouts;
    }
    if let Some(codec) = find(codec_id) {
        layouts.sort_by_key(|id| {
            let matched = codec.layouts.iter().find(|l| l.id == *id).is_some_and(|l| {
                let comment_tokens = tokens(&l.comment);
                model_tokens.iter().any(|m| comment_tokens.contains(m))
            });
            let dock_only = codec
                .layouts
                .iter()
                .find(|l| l.id == *id)
                .is_some_and(|l| l.comment.to_ascii_lowercase().contains("with dock"));
            (!matched, dock_only)
        });
    }
    layouts
}

/// Deterministic default layout-id for a codec (never random). `is_laptop`
/// and the codec subsystem id may be used to prefer OEM-specific layouts.
pub fn default_layout(codec_id: u32, subsystem: Option<u32>, is_laptop: bool) -> Option<u32> {
    ranked_layouts(codec_id, subsystem, is_laptop)
        .first()
        .copied()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ALC256: u32 = 0x10EC_0256;
    const ALC897: u32 = 0x10EC_0897;
    const DELL: u32 = 0x1028_0798;
    const HP: u32 = 0x103C_8575;
    const LENOVO: u32 = 0x17AA_2233;
    const ASUS: u32 = 0x1043_8698;
    const MSI: u32 = 0x1462_7C56;
    const GIGABYTE: u32 = 0x1458_A0C3;

    #[test]
    fn embedded_tables_parse() {
        let codecs = load_codecs(APPLEALC_JSON).expect("AppleALC table");
        assert_eq!(codecs.len(), 109);
        let mut ids: Vec<u32> = codecs.iter().map(|c| c.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), codecs.len(), "duplicate codec ids");
        for codec in &codecs {
            assert!(!codec.layouts.is_empty(), "{:08x} has no layouts", codec.id);
            assert!(codec.layouts.windows(2).all(|w| w[0].id < w[1].id));
            let vendor = vendor_name((codec.id >> 16) as u16).expect("known vendor");
            assert!(codec.name.starts_with(vendor), "{} vs {vendor}", codec.name);
            assert!(!is_hdmi_codec_id(codec.id), "{:08x} is analog", codec.id);
        }
        let names = load_names(CODEC_NAMES_JSON).expect("name table");
        assert!(names.len() > 300);
        assert!(names
            .iter()
            .all(|(id, n)| *id >> 16 != 0x10DE && !n.is_empty()));
        assert!(load_codecs("{}").is_err());
        assert!(
            load_codecs(r#"{"codecs":[{"id":"zz","vendor":"x","name":"y","layouts":[]}]}"#)
                .is_err()
        );
        assert!(load_names("[]").is_err());
    }

    #[test]
    fn lookup_matches_applealc_1_9_8() {
        let alc897 = lookup(ALC897).expect("ALC897");
        assert_eq!(alc897.name, "Realtek ALC897");
        assert_eq!(
            alc897.layouts,
            vec![11, 12, 13, 21, 22, 23, 31, 66, 69, 77, 97, 98, 99]
        );
        let alc1220 = lookup(0x10EC_1220).expect("ALC1220");
        assert_eq!(alc1220.layouts.len(), 24);
        assert_eq!(
            lookup(0x10EC_1168).map(|c| c.name),
            Some("Realtek ALCS1220A".into())
        );
        assert_eq!(
            lookup(0x10EC_0899).map(|c| c.name),
            Some("Realtek ALC898".into())
        );
        assert_eq!(
            lookup(0x10EC_0867).map(|c| c.name),
            Some("Realtek ALC891".into())
        );
        assert_eq!(
            lookup(0x10EC_0286).map(|c| c.name),
            Some("Realtek ALC286".into())
        );
        assert_eq!(
            lookup(ALC256).map(|c| c.name),
            Some("Realtek ALC256".into())
        );
        assert_eq!(
            lookup(0x14F1_510F).map(|c| c.name),
            Some("Conexant CX20751/2".into())
        );
        assert_eq!(
            lookup(0x1106_0441).map(|c| c.name),
            Some("VIA VT2020/2021".into())
        );
        assert_eq!(lookup(0x1013_4206).map(|c| c.layouts.len()), Some(26));
        assert_eq!(lookup(0x10EC_0885).map(|c| c.layouts.len()), Some(18));
        assert_eq!(
            lookup(0x1102_0011).map(|c| c.layouts.first().copied()),
            Some(Some(0))
        );
        // Two AppleALC folders share 111D:7605.
        let idt = lookup(0x111D_7605).expect("IDT 92HD81B1X5");
        assert_eq!(idt.name, "IDT 92HD81B1X5/92HD87B1");
        assert_eq!(idt.layouts, vec![3, 11, 12, 20, 21, 28, 76]);
        assert_eq!(lookup(0x10EC_0231), None);
        assert_eq!(lookup(0), None);
        assert!(is_supported(ALC897));
        assert!(!is_supported(0x10EC_0711));
        assert!(layout_comments(0x10EC_0900)[0].1.contains("Toleda"));
        assert!(layout_comments(0xDEAD_BEEF).is_empty());
    }

    #[test]
    fn names_for_supported_and_unsupported_codecs() {
        assert_eq!(codec_name(ALC897), "Realtek ALC897");
        assert_eq!(codec_name(0x10EC_0231), "Realtek ALC231");
        assert_eq!(codec_name(0x10EC_0711), "Realtek ALC711");
        assert_eq!(codec_name(0x10EC_0880), "Realtek ALC880");
        // Plain kernel entries win over revision-specific ones (0861 rev 0x100340 is ALC660).
        assert_eq!(codec_name(0x10EC_0861), "Realtek ALC861");
        assert_eq!(codec_name(0x10EC_0862), "Realtek ALC861-VD");
        assert_eq!(codec_name(0x10EC_0230), "Realtek ALC230");
        assert_eq!(codec_name(0x14F1_1F87), "Conexant SN6140");
        assert_eq!(codec_name(0x111D_76E3), "IDT 92HD98BXX");
        assert_eq!(codec_name(0x1013_8409), "Cirrus Logic CS8409");
        assert_eq!(codec_name(0x1106_0397), "VIA VT1708S");
        assert_eq!(codec_name(0x11D4_1988), "Analog Devices AD1988A");
        assert_eq!(codec_name(0x11D4_1981), "Analog Devices AD1981");
        assert_eq!(codec_name(0x8384_7680), "SigmaTel STAC9221 A1");
        assert_eq!(codec_name(0x8086_280B), "Intel Kabylake HDMI");
        assert_eq!(codec_name(0x8086_28FF), "Intel HDMI/DP");
        assert_eq!(codec_name(0x1002_AA01), "AMD R6xx HDMI");
        assert_eq!(codec_name(0x1002_AA99), "AMD HDMI/DP");
        assert_eq!(codec_name(0x10DE_0099), "NVIDIA HDMI/DP");
        assert_eq!(codec_name(0x17E8_0047), "Chrontel HDMI");
        assert_eq!(codec_name(0x19E5_8326), "Huawei HW8326");
        assert_eq!(codec_name(0x10EC_0999), "Realtek codec 0999");
        assert_eq!(codec_name(0x1234_5678), "Unknown codec 0x12345678");
    }

    #[test]
    fn oem_aliases() {
        assert_eq!(oem_alias(ALC256, Some(DELL)), Some("ALC3246"));
        assert_eq!(oem_alias(ALC256, Some(HP)), None);
        assert_eq!(oem_alias(ALC256, None), None);
        assert_eq!(
            codec_display_name(0x10EC_0292, Some(0x17AA_2214)),
            "Realtek ALC292 (ALC3232)"
        );
        assert_eq!(
            codec_display_name(0x10EC_0233, Some(ASUS)),
            "Realtek ALC233 (ALC3236)"
        );
        assert_eq!(codec_display_name(ALC897, Some(ASUS)), "Realtek ALC897");
        assert_eq!(find_codec_by_name("ALC3246"), Some(ALC256));
        assert_eq!(find_codec_by_name("alc897"), Some(ALC897));
        assert_eq!(find_codec_by_name("Realtek ALC1220"), Some(0x10EC_1220));
        assert_eq!(find_codec_by_name("CX20751"), Some(0x14F1_510F));
        assert_eq!(find_codec_by_name("CX20752"), Some(0x14F1_510F));
        assert_eq!(find_codec_by_name("VT2021"), Some(0x1106_0441));
        assert_eq!(find_codec_by_name("92HD87B1"), Some(0x111D_7605));
        assert_eq!(find_codec_by_name("92HD81B1X5"), Some(0x111D_7605));
        assert_eq!(find_codec_by_name("92HD87B3"), Some(0x111D_76D1));
        assert_eq!(find_codec_by_name("ALC9999"), None);
        assert_eq!(find_codec_by_name(" "), None);
        // Bare number fragments of combined names are not codec names.
        assert_eq!(find_codec_by_name("2"), None);
        assert_eq!(find_codec_by_name("2021"), None);
        assert_eq!(find_codec_by_name("65"), None);
    }

    #[test]
    fn id_helpers() {
        assert_eq!(codec_id(0x10EC, 0x0897), ALC897);
        assert_eq!(parse_codec_id("10ec", "0897"), Some(ALC897));
        assert_eq!(parse_codec_id("0x10EC", "0x0897"), Some(ALC897));
        assert_eq!(parse_codec_id("10ec0", "0897"), None);
        assert_eq!(parse_codec_id("", "0897"), None);
        assert_eq!(parse_subsystem("10280798"), Some(DELL));
        assert_eq!(parse_subsystem("0x1028:0798"), Some(DELL));
        assert_eq!(parse_subsystem("1028"), None);
        assert_eq!(parse_subsystem("zz280798"), None);
    }

    #[test]
    fn hdmi_codecs() {
        assert!(is_hdmi_codec(0x8086, 0x2807));
        assert!(is_hdmi_codec(0x8086, 0x280B));
        assert!(is_hdmi_codec(0x8086, 0x2882));
        assert!(is_hdmi_codec(0x8086, 0x0054));
        assert!(is_hdmi_codec(0x1002, 0xAA01));
        assert!(is_hdmi_codec(0x1002, 0x793C));
        assert!(is_hdmi_codec(0x10DE, 0x0040));
        assert!(is_hdmi_codec(0x1106, 0x9F84));
        assert!(is_hdmi_codec(0x1D17, 0x9F86));
        assert!(is_hdmi_codec_id(0x8086_2818));
        assert!(!is_hdmi_codec(0x10EC, 0x0897));
        assert!(!is_hdmi_codec(0x14F1, 0x510F));
        assert!(!is_hdmi_codec(0x1106, 0x0441));
        assert!(!is_hdmi_codec(0x1022, 0x1457));
    }

    #[test]
    fn chipset_words() {
        for w in [
            "z390", "b460m", "h81m", "x570", "z97x", "q87tn", "h510d4", "b760m", "x79g", "a320m",
        ] {
            assert!(is_chipset_word(w), "{w}");
        }
        for w in [
            "x555uj", "x441ua", "b470", "x1", "t480", "a515", "x230", "z", "h1000000",
        ] {
            assert!(!is_chipset_word(w), "{w}");
        }
    }

    #[test]
    fn desktop_defaults_prefer_generic_layouts() {
        for (codec, expected) in [
            (0x10EC_0887, 1),
            (0x10EC_0888, 1),
            (0x10EC_0889, 1),
            (0x10EC_0892, 1),
            (0x10EC_0899, 1),
            (0x10EC_0900, 1),
            (0x10EC_0B00, 1),
            (0x10EC_1168, 1),
            (0x10EC_1220, 1),
            (0x10EC_0885, 1),
            (0x10EC_0662, 7),
            (0x10EC_0882, 7),
            (0x10EC_0883, 7),
            (0x1106_0441, 7),
            (0x11D4_198B, 7),
        ] {
            assert_eq!(
                default_layout(codec, None, false),
                Some(expected),
                "{codec:08x}"
            );
            // A board vendor named in some comments does not beat the generic set.
            assert_eq!(
                default_layout(codec, Some(GIGABYTE), false),
                Some(expected),
                "{codec:08x}"
            );
            assert_eq!(
                default_layout(codec, Some(MSI), false),
                Some(expected),
                "{codec:08x}"
            );
        }
    }

    #[test]
    fn desktop_defaults_without_generic_layouts_use_board_vendor() {
        assert_eq!(default_layout(ALC897, Some(MSI), false), Some(13));
        assert_eq!(default_layout(ALC897, Some(ASUS), false), Some(66));
        assert_eq!(default_layout(ALC897, Some(GIGABYTE), false), Some(11));
        assert_eq!(default_layout(ALC897, None, false), Some(11));
        // Dell OptiPlex desktops with a "laptop" codec.
        assert_eq!(default_layout(0x10EC_0255, Some(DELL), false), Some(11));
        assert_eq!(default_layout(0x10EC_0867, Some(HP), false), Some(11));
    }

    #[test]
    fn laptop_defaults() {
        // Generic laptop layouts when the vendor is unknown.
        assert_eq!(default_layout(0x10EC_0269, None, true), Some(3));
        assert_eq!(default_layout(0x10EC_0255, None, true), Some(3));
        assert_eq!(default_layout(0x10EC_0236, None, true), Some(3));
        assert_eq!(default_layout(0x10EC_0295, None, true), Some(3));
        assert_eq!(default_layout(0x10EC_0233, None, true), Some(3));
        assert_eq!(default_layout(0x10EC_0298, None, true), Some(3));
        assert_eq!(default_layout(ALC256, None, true), Some(13));
        assert_eq!(default_layout(0x10EC_0892, None, true), Some(4));
        // OEM-specific layouts by codec subsystem vendor.
        assert_eq!(default_layout(ALC256, Some(DELL), true), Some(13));
        assert_eq!(default_layout(0x10EC_0255, Some(DELL), true), Some(20));
        assert_eq!(default_layout(0x10EC_0295, Some(HP), true), Some(1));
        assert_eq!(default_layout(0x10EC_0269, Some(LENOVO), true), Some(18));
        assert_eq!(default_layout(0x10EC_0257, Some(LENOVO), true), Some(11));
        assert_eq!(default_layout(0x10EC_0245, Some(HP), true), Some(13));
        assert_eq!(default_layout(0x10EC_0294, Some(ASUS), true), Some(11));
        // A vendor without matching comments falls back to the generic set.
        assert_eq!(
            default_layout(0x10EC_0269, Some(0x1B0A_1234), true),
            Some(3)
        );
        assert_eq!(
            default_layout(0x10EC_0269, Some(0x10EC_0269), true),
            Some(3)
        );
        // Tongfang/Mechrevo machines get the Mechrevo layout.
        assert_eq!(
            default_layout(0x10EC_0269, Some(0x1D05_1234), true),
            Some(88)
        );
    }

    #[test]
    fn exact_subsystem_in_comment_wins() {
        assert_eq!(
            default_layout(0x10EC_1168, Some(0x1462_CB17), false),
            Some(88)
        );
        assert_eq!(
            default_layout(0x10EC_1168, Some(0x1462_CB18), false),
            Some(1)
        );
    }

    #[test]
    fn rankings_are_complete_and_deterministic() {
        let codecs: &Vec<Codec> = &CODECS;
        for codec in codecs {
            let mut all: Vec<u32> = codec.layouts.iter().map(|l| l.id).collect();
            for subsystem in [
                None,
                Some(DELL),
                Some(HP),
                Some(LENOVO),
                Some(ASUS),
                Some(0),
                Some(0xFFFF_FFFF),
            ] {
                for laptop in [false, true] {
                    let ranked = ranked_layouts(codec.id, subsystem, laptop);
                    let mut sorted = ranked.clone();
                    sorted.sort_unstable();
                    all.sort_unstable();
                    assert_eq!(sorted, all, "{:08x}", codec.id);
                    assert_eq!(ranked, ranked_layouts(codec.id, subsystem, laptop));
                    assert_eq!(
                        default_layout(codec.id, subsystem, laptop),
                        ranked.first().copied()
                    );
                }
            }
        }
        assert_eq!(default_layout(0x10EC_0231, None, true), None);
        assert!(ranked_layouts(0x10EC_0231, None, false).is_empty());
    }

    #[test]
    fn layout_zero_is_never_the_default_when_others_exist() {
        assert_ne!(default_layout(0x1102_0011, None, false), Some(0));
        assert_ne!(default_layout(0x1102_0011, None, true), Some(0));
    }
}
