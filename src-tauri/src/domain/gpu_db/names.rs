//! Name-based GPU identification, used for manual entry and for adapters that
//! come without PCI ids. Handles Windows WMI names ("Intel(R) UHD Graphics
//! 630"), lspci names ("GK208B [GeForce GT 730]") and short user input
//! ("RX 580", "GTX 1060").

use once_cell::sync::Lazy;
use regex::Regex;

use crate::domain::model::GpuFamily::{self, *};
use crate::domain::model::GpuVendor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NameGuess {
    pub vendor: GpuVendor,
    pub family: GpuFamily,
    pub igpu: bool,
}

/// Lowercase, drop trademark marks, turn punctuation into spaces and collapse
/// whitespace: "AMD Radeon(TM) RX 6600 XT" → "amd radeon rx 6600 xt".
pub(super) fn normalize(name: &str) -> String {
    let lower = name
        .to_lowercase()
        .replace("(r)", " ")
        .replace("(tm)", " ")
        .replace("(c)", " ");
    let mapped: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn re(pattern: &str) -> Regex {
    // Patterns are compile-time constants covered by the unit tests.
    Regex::new(pattern).unwrap_or_else(|e| panic!("invalid GPU name pattern {pattern}: {e}"))
}

fn cap_u32(caps: &regex::Captures<'_>, idx: usize) -> Option<u32> {
    caps.get(idx).and_then(|m| m.as_str().parse().ok())
}

fn cap_str<'a>(caps: &regex::Captures<'a>, idx: usize) -> &'a str {
    caps.get(idx).map_or("", |m| m.as_str())
}

/// Identify vendor and family from a GPU name. `family` is `Unknown` when the
/// name is ambiguous (e.g. "AMD Radeon(TM) Graphics", "Intel(R) UHD Graphics").
pub(super) fn guess(name: &str) -> NameGuess {
    let n = normalize(name);
    let vendor = vendor_of(&n);
    guess_for_vendor(&n, vendor)
}

/// Like `guess`, but with the vendor already known (from the PCI vendor id).
pub(super) fn guess_for_vendor(n: &str, vendor: GpuVendor) -> NameGuess {
    let (family, igpu) = match vendor {
        GpuVendor::Virtual => (Some(VirtualDisplay), false),
        GpuVendor::Intel => {
            let family = intel_family(n);
            let igpu = family != Some(IntelArc);
            (family, igpu)
        }
        GpuVendor::Amd => {
            let family = amd_family(n);
            let igpu = match family {
                Some(f) => matches!(f, AmdApuVega | AmdApuRdna | AmdApuLegacy),
                // A bare "Radeon Graphics" is an APU.
                None => RE_AMD_BARE_GRAPHICS.is_match(n),
            };
            (family, igpu)
        }
        GpuVendor::Nvidia => (nvidia_family(n), false),
        GpuVendor::Unknown => (None, false),
    };
    NameGuess {
        vendor,
        family: family.unwrap_or(GpuFamily::Unknown),
        igpu,
    }
}

// ── Vendor ──────────────────────────────────────────────────────────────────

static RE_VIRTUAL: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(vmware|virtualbox|vbox ?svga|vbox ?vga|qxl|virtio|bochs|qemu|hyper v|parallels|microsoft basic (display|render)|remote display|citrix|parsec|spacedesk|displaylink|llvmpipe|softpipe|software rasterizer|virtual (display|monitor|graphics)|indirect display|std ?vga|cirrus logic|xen)\b",
    )
});
static RE_NVIDIA: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(nvidia|geforce|quadro|nvs|titan|rtx|gtx|tesla [a-z]\d|grid [km]\d|(rtx|gtx|gts|gt|mx) ?\d{3,4}[a-z]*|(g[fkmpv]|tu|ga|ad|gb|gh)\d{3}[a-z]*|gt2\d\d[a-z]*)\b",
    )
});
static RE_AMD: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(amd|ati|radeon|firepro|firegl|instinct|advanced micro devices|navi ?\d\d|ellesmere|baffin|lexa|polaris ?\d\d|tahiti|pitcairn|hawaii|bonaire|tonga|fiji|renoir|cezanne|picasso|raven|rx ?\d{3,4}[a-z]*|r[579] ?m?\d{3}[a-z]*|vega|fury)\b",
    )
});
static RE_INTEL: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(intel|iris|uhd graphics|hd graphics|uhd ?p?\d{3}|hd ?p?\d{3,4}|gma|graphics media accelerator|arc)\b",
    )
});

/// Four-digit "HD nnnn" models Intel used; any other "HD nnnn" is a Radeon.
const INTEL_HD_NUMBERS: [u32; 15] = [
    2000, 2500, 3000, 4000, 4200, 4400, 4600, 4700, 5000, 5300, 5500, 5600, 6000, 6200, 6300,
];

static RE_BARE_HD4: Lazy<Regex> = Lazy::new(|| re(r"^hd ?(\d{4})[a-z]?\b"));

fn is_bare_radeon_hd(n: &str) -> bool {
    RE_BARE_HD4
        .captures(n)
        .and_then(|caps| cap_u32(&caps, 1))
        .is_some_and(|number| !INTEL_HD_NUMBERS.contains(&number))
}

fn vendor_of(n: &str) -> GpuVendor {
    if RE_VIRTUAL.is_match(n) {
        GpuVendor::Virtual
    } else if RE_NVIDIA.is_match(n) {
        GpuVendor::Nvidia
    } else if RE_AMD.is_match(n) || is_bare_radeon_hd(n) {
        GpuVendor::Amd
    } else if RE_INTEL.is_match(n) {
        GpuVendor::Intel
    } else {
        GpuVendor::Unknown
    }
}

// ── Intel ───────────────────────────────────────────────────────────────────

static RE_INTEL_GMA: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(gma|graphics media accelerator|express chipset|4 series|g3[13]|g4[15]|q4[35]|b43|9[14]5g|9[46]5gm?|x3100)\b",
    )
});
static RE_INTEL_ARC_DGPU: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(arc (pro )?[ab] ?\d{2,3}[a-z]?|iris xe max|dg1|dg2|alchemist|battlemage|data center gpu)\b",
    )
});
static RE_INTEL_LOW_POWER: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(alder ?lake n|twin ?lake|jasper ?lake|elkhart ?lake|gemini ?lake|apollo ?lake|bay ?trail|cherry ?trail|braswell|lakefield|atom)\b",
    )
});
static RE_INTEL_XE_CODENAME: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(tiger ?lake|rocket ?lake|alder ?lake|raptor ?lake|meteor ?lake|arrow ?lake|lunar ?lake|panther ?lake|wildcat ?lake|nova ?lake)\b",
    )
});
static RE_INTEL_CODENAME: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(ice ?lake|comet ?lake|coffee ?lake|whiskey ?lake|kaby ?lake|amber ?lake|sky ?lake|broadwell|haswell|crystal ?well|4th gen(eration)? core|ivy ?bridge|3rd gen(eration)? core|sandy ?bridge|2nd gen(eration)? core|iron ?lake|clarkdale|arrandale)\b",
    )
});
static RE_INTEL_IRIS_PRO: Lazy<Regex> = Lazy::new(|| re(r"\biris pro (graphics )?p?(\d{3,4})\b"));
static RE_INTEL_IRIS_PLUS: Lazy<Regex> =
    Lazy::new(|| re(r"\biris plus( graphics)?( (\d{3}|g\d))?\b"));
static RE_INTEL_IRIS: Lazy<Regex> = Lazy::new(|| re(r"\biris (graphics )?p?(\d{3,4})\b"));
static RE_INTEL_UHD: Lazy<Regex> = Lazy::new(|| re(r"\buhd ?(graphics )?p?(\d{3})\b"));
static RE_INTEL_HD: Lazy<Regex> = Lazy::new(|| re(r"\bhd ?(graphics )?p?(\d{3,4})\b"));

fn intel_family(n: &str) -> Option<GpuFamily> {
    if n.contains("graphics media accelerator hd") {
        return Some(IntelIronLake);
    }
    if RE_INTEL_GMA.is_match(n) {
        return Some(IntelGma);
    }
    if RE_INTEL_ARC_DGPU.is_match(n) {
        return Some(IntelArc);
    }
    if RE_INTEL_LOW_POWER.is_match(n) {
        return Some(IntelLowPower);
    }
    if RE_INTEL_XE_CODENAME.is_match(n) {
        return Some(IntelXe);
    }
    if let Some(caps) = RE_INTEL_CODENAME.captures(n) {
        let code = cap_str(&caps, 1).replace(' ', "");
        let family = match code.as_str() {
            "icelake" => IntelIceLake,
            "cometlake" => IntelCometLake,
            "coffeelake" | "whiskeylake" => IntelCoffeeLake,
            "kabylake" | "amberlake" => IntelKabyLake,
            "skylake" => IntelSkylake,
            "broadwell" => IntelBroadwell,
            "haswell" | "crystalwell" => IntelHaswell,
            "ivybridge" => IntelIvyBridge,
            "sandybridge" => IntelSandyBridge,
            "ironlake" | "clarkdale" | "arrandale" => IntelIronLake,
            c if c.starts_with("4thgen") => IntelHaswell,
            c if c.starts_with("3rdgen") => IntelIvyBridge,
            c if c.starts_with("2ndgen") => IntelSandyBridge,
            _ => return None,
        };
        return Some(family);
    }
    if n.contains("iris xe") || n.contains("arc graphics") || RE_ARC_IGPU.is_match(n) {
        return Some(IntelXe);
    }
    if let Some(caps) = RE_INTEL_IRIS_PRO.captures(n) {
        return match cap_u32(&caps, 2)? {
            5200 => Some(IntelHaswell),
            6200 | 6300 => Some(IntelBroadwell),
            580 => Some(IntelSkylake),
            _ => None,
        };
    }
    if let Some(caps) = RE_INTEL_IRIS_PLUS.captures(n) {
        return match cap_str(&caps, 3) {
            "640" | "650" => Some(IntelKabyLake),
            "645" | "655" => Some(IntelCoffeeLake),
            // "Iris Plus Graphics" with no number or a G-level is Ice Lake.
            "" | "g4" | "g7" => Some(IntelIceLake),
            _ => None,
        };
    }
    if let Some(caps) = RE_INTEL_IRIS.captures(n) {
        return match cap_u32(&caps, 2)? {
            5100 => Some(IntelHaswell),
            6100 => Some(IntelBroadwell),
            540 | 550 | 555 => Some(IntelSkylake),
            _ => None,
        };
    }
    if n.contains("uhd graphics g1") || n.contains("uhd g1") {
        return Some(IntelIceLake);
    }
    if let Some(caps) = RE_INTEL_UHD.captures(n) {
        return match cap_u32(&caps, 2)? {
            600 | 605 => Some(IntelLowPower),
            // UHD 610/630 exist on Coffee Lake and Comet Lake, UHD 620 on Kaby
            // Lake-R, Whiskey Lake and Comet Lake; the device id decides.
            610 | 630 => Some(IntelCoffeeLake),
            615 | 617 | 620 => Some(IntelKabyLake),
            700..=799 => Some(IntelXe),
            _ => None,
        };
    }
    if let Some(caps) = RE_INTEL_HD.captures(n) {
        return match cap_u32(&caps, 2)? {
            2000 | 3000 => Some(IntelSandyBridge),
            2500 | 4000 => Some(IntelIvyBridge),
            4200 | 4400 | 4600 | 4700 | 5000 => Some(IntelHaswell),
            5300 | 5500 | 5600 | 5700 | 6000 | 6300 => Some(IntelBroadwell),
            510 | 515 | 520 | 530 | 535 => Some(IntelSkylake),
            610 | 615 | 620 | 630 | 635 => Some(IntelKabyLake),
            400 | 405 | 500 | 505 => Some(IntelLowPower),
            _ => None,
        };
    }
    if n == "intel graphics" || (n.starts_with("intel graphics ") && !n.contains("family")) {
        // Meteor Lake and newer report a bare "Intel(R) Graphics".
        return Some(IntelXe);
    }
    if n.contains("core processor integrated graphics") {
        // lspci name for Clarkdale/Arrandale; later generations carry an ordinal.
        return Some(IntelIronLake);
    }
    None
}

static RE_ARC_IGPU: Lazy<Regex> = Lazy::new(|| re(r"\barc (pro )?1[34]0[tv]\b"));

// ── AMD ─────────────────────────────────────────────────────────────────────

static RE_AMD_BARE_GRAPHICS: Lazy<Regex> = Lazy::new(|| {
    re(r"^(amd |ati |advanced micro devices inc amd ati )?radeon graphics( processor)?$")
});
static RE_AMD_NAVI_CHIP: Lazy<Regex> = Lazy::new(|| re(r"\bnavi ?(\d{2})\b"));
static RE_AMD_CHIP: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(ellesmere|baffin|lexa|polaris ?\d{2}|tonga|fiji|amethyst|antigua|topaz|iceland|meso|hawaii|grenada|vesuvius|bonaire|tobago|saturn|emerald|strato|tahiti|malta|pitcairn|curacao|trinidad|cape verde|venus|chelsea|heathrow|tropo|oland|hainan|mars|opal|neptune|wimbledon|cayman|antilles|barts|turks|caicos|cypress|hemlock|juniper|redwood|cedar|broadway|madison|robson|whistler|seymour|thames|blackcomb|granville|lexington|pinewood|onega|rv[67]\d\d|r[67]00|raven|picasso|renoir|lucienne|cezanne|barcelo|dali|pollock|rembrandt|phoenix|hawk ?point|raphael|granite ridge|dragon range|mendocino|strix (point|halo)|krackan|van ?gogh|kaveri|godavari|carrizo|bristol|stoney|kabini|mullins|temash|beema|trinity|richland|sumo|llano|wrestler|wani|rs[78]80)\b",
    )
});
static RE_AMD_RX4: Lazy<Regex> = Lazy::new(|| re(r"\brx ?(\d{4})( ?[a-z]+)?"));
static RE_AMD_RX3: Lazy<Regex> = Lazy::new(|| re(r"\brx ?(\d{3})[a-z]*\b"));
static RE_AMD_PRO: Lazy<Regex> = Lazy::new(|| re(r"\bpro (wx|w|v)? ?(\d{3,4})( ?[a-z]+)?"));
static RE_AMD_R_SERIES: Lazy<Regex> = Lazy::new(|| re(r"\br([579]) ?(m)?(\d{3})[a-z]*\b"));
static RE_AMD_R_GRAPHICS: Lazy<Regex> = Lazy::new(|| re(r"\br[1-7]e? graphics\b"));
static RE_AMD_HD: Lazy<Regex> = Lazy::new(|| re(r"\bhd ?(\d{4})([a-z]?)\b"));
static RE_AMD_FIREPRO: Lazy<Regex> = Lazy::new(|| re(r"\bfirepro ([a-z]?)(\d{3,4})([a-z]?)\b"));
static RE_AMD_APU_RDNA: Lazy<Regex> = Lazy::new(|| re(r"\bradeon (\d{3}m|80[4-6]0s)\b"));
static RE_AMD_LEXA_PLAIN: Lazy<Regex> = Lazy::new(|| re(r"\bradeon (540|550|630)x?\b"));

fn amd_family(n: &str) -> Option<GpuFamily> {
    if n.contains("instinct") {
        return if n.contains("mi25") {
            Some(AmdVega10)
        } else if n.contains("mi50") || n.contains("mi60") {
            Some(AmdVega20)
        } else {
            None
        };
    }
    // Kaby Lake-G "Radeon RX Vega M" is Polaris 22, not a Vega APU.
    if n.contains("vega m ")
        || n.ends_with("vega m")
        || n.contains("polaris 22")
        || n.contains("polaris22")
    {
        return Some(AmdPolaris);
    }
    if n.contains("6750 gre") {
        return Some(AmdNavi22);
    }
    if let Some(caps) = RE_AMD_NAVI_CHIP.captures(n) {
        return match cap_u32(&caps, 1)? {
            10 => Some(AmdNavi10),
            12 => Some(AmdNavi12),
            14 => Some(AmdNavi14),
            21 => Some(AmdNavi21),
            22 => Some(AmdNavi22),
            23 => Some(AmdNavi23),
            24 => Some(AmdNavi24),
            31..=33 | 44 | 48 => Some(AmdRdna3Plus),
            _ => None,
        };
    }
    if n.contains("radeon vii") || n.contains("pro vii") || n.contains("vega ii") {
        return Some(AmdVega20);
    }
    if let Some(family) = amd_vega(n) {
        return Some(family);
    }
    // Retail RX numbers before codenames: board names such as "ASUS Phoenix
    // RX 550" would otherwise hit the codename table.
    let rx4 = RE_AMD_RX4
        .captures(n)
        .and_then(|caps| amd_rx4(cap_u32(&caps, 1)?, cap_str(&caps, 2).trim()));
    if rx4.is_some() {
        return rx4;
    }
    let rx3 = RE_AMD_RX3
        .captures(n)
        .and_then(|caps| match cap_u32(&caps, 1)? {
            460 | 470 | 480 | 560 | 570 | 580 | 590 => Some(AmdPolaris),
            540 | 550 | 640 => Some(AmdLexa),
            _ => None,
        });
    if rx3.is_some() {
        return rx3;
    }
    if let Some(caps) = RE_AMD_CHIP.captures(n) {
        return amd_chip_family(cap_str(&caps, 1));
    }
    if let Some(caps) = RE_AMD_PRO.captures(n) {
        if let Some(family) = amd_pro(
            cap_str(&caps, 1),
            cap_u32(&caps, 2)?,
            cap_str(&caps, 3).trim(),
        ) {
            return Some(family);
        }
    }
    if RE_AMD_LEXA_PLAIN.is_match(n) {
        return Some(AmdLexa);
    }
    if n.contains("fury") || n.contains("r9 nano") {
        return Some(AmdGcn3);
    }
    if let Some(caps) = RE_AMD_R_SERIES.captures(n) {
        let mobile = !cap_str(&caps, 2).is_empty();
        return amd_r_series(cap_str(&caps, 1), mobile, cap_u32(&caps, 3)?);
    }
    if RE_AMD_R_GRAPHICS.is_match(n) {
        return Some(AmdApuLegacy);
    }
    if let Some(caps) = RE_AMD_FIREPRO.captures(n) {
        return amd_firepro(cap_str(&caps, 1), cap_u32(&caps, 2)?, cap_str(&caps, 3));
    }
    if let Some(caps) = RE_AMD_HD.captures(n) {
        return amd_hd(cap_u32(&caps, 1)?, cap_str(&caps, 2));
    }
    if RE_AMD_APU_RDNA.is_match(n) {
        return Some(AmdApuRdna);
    }
    None
}

static RE_VEGA_DISCRETE: Lazy<Regex> = Lazy::new(|| {
    re(
        r"\b(vega (56|64|48)|vega 64x|vega frontier|vega fe|wx (8100|8200|9100)|vega ?10 (xl|xt|xtx|gl|lea)|vega ?12|pro vega (16|20)|pro ssg)\b",
    )
});

fn amd_vega(n: &str) -> Option<GpuFamily> {
    if !n.contains("vega") {
        return None;
    }
    // "Radeon RX Vega" with no number is the Vega 56/64 retail card.
    if RE_VEGA_DISCRETE.is_match(n) || n.ends_with("rx vega") {
        return Some(AmdVega10);
    }
    if n.contains("vega 20") || n.contains("vega20") {
        return Some(AmdVega20);
    }
    // "Radeon Vega 8 Graphics", "Radeon RX Vega 10 Graphics" (2700U),
    // "Radeon Vega Mobile Gfx", lspci "[Radeon Vega Series / ...]".
    Some(AmdApuVega)
}

fn amd_chip_family(chip: &str) -> Option<GpuFamily> {
    let c = chip.replace(' ', "");
    Some(match c.as_str() {
        "ellesmere" | "baffin" | "polaris10" | "polaris11" | "polaris20" | "polaris21"
        | "polaris30" => AmdPolaris,
        "lexa" | "polaris12" => AmdLexa,
        "tonga" | "fiji" | "amethyst" | "antigua" | "topaz" | "iceland" | "meso" => AmdGcn3,
        "hawaii" | "grenada" | "vesuvius" | "bonaire" | "tobago" | "saturn" | "emerald"
        | "strato" => AmdGcn2,
        "tahiti" | "malta" | "pitcairn" | "curacao" | "trinidad" | "capeverde" | "venus"
        | "chelsea" | "heathrow" | "tropo" | "oland" | "hainan" | "mars" | "opal" | "neptune"
        | "wimbledon" => AmdGcn1,
        "raven" | "picasso" | "renoir" | "lucienne" | "cezanne" | "barcelo" | "dali"
        | "pollock" => AmdApuVega,
        "rembrandt" | "phoenix" | "hawkpoint" | "raphael" | "graniteridge" | "dragonrange"
        | "mendocino" | "strixpoint" | "strixhalo" | "krackan" | "vangogh" => AmdApuRdna,
        "kaveri" | "godavari" | "carrizo" | "bristol" | "stoney" | "kabini" | "mullins"
        | "temash" | "beema" | "trinity" | "richland" | "sumo" | "llano" | "wrestler" | "wani"
        | "rs780" | "rs880" => AmdApuLegacy,
        c if c.starts_with("polaris") => AmdPolaris,
        _ => AmdTeraScale,
    })
}

/// "Radeon Pro W6800", "Radeon Pro WX 7100", "Radeon Pro 5500M", "Radeon Pro V620".
fn amd_pro(series: &str, number: u32, suffix: &str) -> Option<GpuFamily> {
    let mobile = suffix.starts_with('m');
    Some(match (series, number) {
        ("w", 7000..=7999) => AmdRdna3Plus,
        ("w", 6800..=6999) => AmdNavi21,
        ("w", 6600..=6799) => AmdNavi23,
        ("w", 6300..=6599) => AmdNavi24,
        ("w", 5700..=5799) => AmdNavi10,
        ("w", 5300..=5599) => AmdNavi14,
        ("wx", 8100 | 8200 | 9100) => AmdVega10,
        ("wx", 4100..=7199) => AmdPolaris,
        ("wx", 2100..=3299) => AmdLexa,
        ("v", 7300 | 5300) => AmdPolaris,
        ("v", 320 | 340) => AmdVega10,
        ("v", 520 | 540) => AmdNavi12,
        ("v", 620) => AmdNavi21,
        ("v", 700..=799) => AmdRdna3Plus,
        ("", 5600) if mobile => AmdNavi12,
        ("", 5700) => AmdNavi10,
        ("", 5300 | 5500) => AmdNavi14,
        ("", 450..=599) => AmdPolaris,
        _ => return None,
    })
}

/// "RX 6800 XT", "RX 6800M", "RX 6700S", "RX 7900 XTX", "RX 9070".
fn amd_rx4(number: u32, suffix: &str) -> Option<GpuFamily> {
    let first = suffix.split(' ').next().unwrap_or("");
    let mobile = first.starts_with('m');
    let s_part = first == "s";
    Some(match number {
        9000..=9999 | 7000..=7999 => AmdRdna3Plus,
        6900..=6999 => AmdNavi21,
        6850 => AmdNavi22,
        6800 if mobile => AmdNavi22,
        6800 if s_part => AmdNavi23,
        6800 => AmdNavi21,
        6700 if s_part => AmdNavi23,
        6700..=6799 => AmdNavi22,
        6600..=6699 => AmdNavi23,
        6300..=6599 => AmdNavi24,
        5600..=5799 => AmdNavi10,
        5300..=5599 => AmdNavi14,
        _ => return None,
    })
}

/// "R9 290X", "R7 370", "R9 M370X", "R5 230".
fn amd_r_series(class: &str, mobile: bool, number: u32) -> Option<GpuFamily> {
    Some(match (class, mobile, number) {
        ("9", false, 285 | 380) => AmdGcn3,
        ("9", false, 290 | 295 | 390) => AmdGcn2,
        ("9", false, 260 | 360) => AmdGcn2,
        ("9", false, _) => AmdGcn1,
        ("9", true, 295 | 390 | 395) => AmdGcn3,
        ("9", true, 270 | 280 | 380) => AmdGcn2,
        ("9", true, _) => AmdGcn1,
        ("7", false, 260 | 360) => AmdGcn2,
        ("7", _, _) => AmdGcn1,
        ("5", false, 230 | 235 | 310) => AmdTeraScale,
        ("5", _, _) => AmdGcn1,
        _ => return None,
    })
}

/// "FirePro W9100", "FirePro D700", "FirePro V4800".
fn amd_firepro(series: &str, number: u32, suffix: &str) -> Option<GpuFamily> {
    Some(match (series, number) {
        ("w", 8100 | 9100) => AmdGcn2,
        ("w", 5100 | 4300) => AmdGcn2,
        ("w", 6150 | 4170 | 5170) if suffix == "m" => AmdGcn2,
        ("m", 6100) => AmdGcn2,
        ("w", 7100) | ("s", 7150) | ("w", 5130) => AmdGcn3,
        ("w" | "s" | "d" | "r" | "m", _) => AmdGcn1,
        ("v", _) | ("", _) => AmdTeraScale,
        _ => return None,
    })
}

/// "HD 7970", "HD 7790", "HD 6870", "HD 8570D".
fn amd_hd(number: u32, suffix: &str) -> Option<GpuFamily> {
    if suffix == "d" || suffix == "g" {
        return Some(AmdApuLegacy);
    }
    Some(match number {
        7790 | 8770 | 8930 => AmdGcn2,
        7700..=7999 => AmdGcn1,
        8730..=8999 | 8570 | 8670 | 8690 => AmdGcn1,
        8180 | 8210 | 8240 | 8250 | 8280 | 8330 | 8400 => AmdApuLegacy,
        // Brazos (E-/C-series APUs)
        6250 | 6290 | 6310 | 6320 | 7290 | 7310 | 7340 => AmdApuLegacy,
        3000 | 3100 | 3200 | 3300 | 4200 | 4225 | 4250 | 4270 | 4290 => AmdApuLegacy,
        2000..=8999 => AmdTeraScale,
        _ => return None,
    })
}

// ── NVIDIA ──────────────────────────────────────────────────────────────────

static RE_NV_CHIP: Lazy<Regex> =
    Lazy::new(|| re(r"\b(gf|gk|gm|gp|gv|tu|ga|ad|gb|gh)(\d{3})[a-z]*\b"));
static RE_NV_TESLA_CHIP: Lazy<Regex> =
    Lazy::new(|| re(r"\b(gt2\d\d|g8\d|g9\d|mcp7\d|mcp89)[a-z]*\b"));
static RE_NV_PRE_TESLA_CHIP: Lazy<Regex> =
    Lazy::new(|| re(r"\b(nv\d{1,2}|g7\d|c51|c61|c67|c68|c73)\b"));
static RE_NV_MX: Lazy<Regex> = Lazy::new(|| re(r"\bmx ?(\d{3})\b"));
static RE_NV_GTX: Lazy<Regex> = Lazy::new(|| re(r"\bgtx ?(\d{3,4})( ?ti)?( ?(mx|m))?\b"));
static RE_NV_GTS: Lazy<Regex> = Lazy::new(|| re(r"\bgts ?(\d{3})"));
static RE_NV_GT: Lazy<Regex> = Lazy::new(|| re(r"\bgt ?(\d{3,4})( ?(m|mx))?\b"));
static RE_NV_NVS: Lazy<Regex> = Lazy::new(|| re(r"\bnvs ?(\d{3,4})"));
static RE_NV_QUADRO: Lazy<Regex> =
    Lazy::new(|| re(r"\bquadro (fx |cx |plex )?([a-z]{0,2})(\d{2,4})"));
static RE_NV_DATACENTER: Lazy<Regex> = Lazy::new(|| re(r"\b(tesla|grid) ([a-z])(\d{1,4})"));
static RE_NV_PLAIN: Lazy<Regex> = Lazy::new(|| re(r"\bgeforce (\d{3,4})( ?(m|mx))?\b"));

fn nvidia_family(n: &str) -> Option<GpuFamily> {
    if let Some(caps) = RE_NV_CHIP.captures(n) {
        return Some(match cap_str(&caps, 1) {
            "gf" => NvidiaFermi,
            "gk" => NvidiaKepler,
            "gm" => NvidiaMaxwell,
            "gp" => NvidiaPascal,
            _ => NvidiaModern,
        });
    }
    if RE_NV_TESLA_CHIP.is_match(n) {
        return Some(NvidiaTesla);
    }
    if RE_NV_PRE_TESLA_CHIP.is_match(n) {
        return None;
    }
    if n.contains("rtx") {
        return Some(NvidiaModern);
    }
    if n.contains("titan") {
        return Some(if n.contains("titan v") || n.contains("titan rtx") {
            NvidiaModern
        } else if n.contains("titan xp") || (n.contains("titan x") && n.contains("pascal")) {
            NvidiaPascal
        } else if n.contains("titan x") {
            NvidiaMaxwell
        } else {
            NvidiaKepler
        });
    }
    if let Some(caps) = RE_NV_DATACENTER.captures(n) {
        let number = cap_u32(&caps, 3)?;
        return Some(match (cap_str(&caps, 2), number) {
            ("k", _) => NvidiaKepler,
            ("m", 2000..=2999) | ("c", 2000..=2999) => NvidiaFermi,
            ("m", _) => NvidiaMaxwell,
            ("c", _) | ("s", _) => NvidiaTesla,
            ("p", _) => NvidiaPascal,
            _ => NvidiaModern,
        });
    }
    if let Some(caps) = RE_NV_MX.captures(n) {
        return Some(match cap_u32(&caps, 1)? {
            110 | 130 => NvidiaMaxwell,
            150 | 230 | 250 | 330 | 350 => NvidiaPascal,
            _ => NvidiaModern,
        });
    }
    if let Some(caps) = RE_NV_GTX.captures(n) {
        return nvidia_gtx(cap_u32(&caps, 1)?, cap_str(&caps, 4));
    }
    if let Some(caps) = RE_NV_GTS.captures(n) {
        return Some(if cap_u32(&caps, 1)? >= 400 {
            NvidiaFermi
        } else {
            NvidiaTesla
        });
    }
    if let Some(caps) = RE_NV_GT.captures(n) {
        let mobile = !cap_str(&caps, 2).is_empty();
        return nvidia_gt(cap_u32(&caps, 1)?, mobile);
    }
    if let Some(caps) = RE_NV_NVS.captures(n) {
        return Some(match cap_u32(&caps, 1)? {
            810 => NvidiaMaxwell,
            510 => NvidiaKepler,
            310 | 315 | 4200 | 5200 | 5400 => NvidiaFermi,
            _ => NvidiaTesla,
        });
    }
    if n.contains("quadro") {
        return nvidia_quadro(n);
    }
    if let Some(caps) = RE_NV_PLAIN.captures(n) {
        let mobile = !cap_str(&caps, 2).is_empty();
        return nvidia_plain(cap_u32(&caps, 1)?, mobile);
    }
    None
}

/// `suffix` is "m" or "mx" for notebook parts.
fn nvidia_gtx(number: u32, suffix: &str) -> Option<GpuFamily> {
    let mobile = !suffix.is_empty();
    Some(match number {
        1600..=1699 | 2000..=2999 => NvidiaModern,
        1000..=1099 => NvidiaPascal,
        900..=999 => NvidiaMaxwell,
        870 | 880 => NvidiaKepler,
        800..=899 => NvidiaMaxwell,
        745 | 750 if !mobile => NvidiaMaxwell,
        700..=799 => NvidiaKepler,
        // GTX 670M/675M are Fermi; the 670MX/675MX refresh is Kepler.
        670 | 675 if suffix == "m" => NvidiaFermi,
        600..=699 => NvidiaKepler,
        400..=599 => NvidiaFermi,
        100..=299 => NvidiaTesla,
        _ => return None,
    })
}

fn nvidia_gt(number: u32, mobile: bool) -> Option<GpuFamily> {
    Some(match (number, mobile) {
        (1010 | 1030, _) => NvidiaPascal,
        (610 | 620 | 625 | 630 | 635 | 710 | 720 | 820, true) => NvidiaFermi,
        (640 | 645 | 650 | 730 | 735 | 740 | 745 | 750 | 755, true) => NvidiaKepler,
        (800..=899, true) => NvidiaMaxwell,
        // GT 710/720/740 are all Kepler; GT 730 is mostly GK208 (a GF108
        // variant exists), GT 630 mostly GF108, GT 640 mostly GK107.
        (710 | 720 | 730 | 740 | 635 | 640, false) => NvidiaKepler,
        (705 | 610 | 620 | 630 | 645, false) => NvidiaFermi,
        (400..=599, _) => NvidiaFermi,
        (100..=399, _) => NvidiaTesla,
        _ => return None,
    })
}

fn nvidia_quadro(n: &str) -> Option<GpuFamily> {
    if n.contains("quadro rtx") || n.contains("quadro gv") {
        return Some(NvidiaModern);
    }
    if n.contains("quadro gp") {
        return Some(NvidiaPascal);
    }
    let caps = RE_NV_QUADRO.captures(n)?;
    let line = cap_str(&caps, 1).trim();
    let letters = cap_str(&caps, 2);
    let number = cap_u32(&caps, 3)?;
    Some(match (line, letters) {
        ("fx", _) => NvidiaTesla,
        ("cx", _) => NvidiaFermi,
        (_, "t") => NvidiaModern,
        (_, "p") => NvidiaPascal,
        (_, "m") => NvidiaMaxwell,
        // Quadro K620/K1200/K2200 are Maxwell GM107, the rest of K is Kepler.
        (_, "k") if matches!(number, 620 | 1200 | 2200) => NvidiaMaxwell,
        (_, "k") => NvidiaKepler,
        (_, "") => NvidiaFermi,
        _ => return None,
    })
}

fn nvidia_plain(number: u32, mobile: bool) -> Option<GpuFamily> {
    Some(match (number, mobile) {
        (8000..=9999, _) => NvidiaTesla,
        (6000..=7999, _) => return None,
        (900..=999, true) => NvidiaMaxwell,
        (820, true) => NvidiaFermi,
        (800..=899, true) => NvidiaMaxwell,
        (600..=799, true) => NvidiaFermi,
        (100..=499, _) => NvidiaTesla,
        _ => return None,
    })
}

// ── Model hints used by the support rules (input is `normalize`d) ───────────

static RE_COMPUTE: Lazy<Regex> = Lazy::new(|| {
    re(r"\b(tesla [a-z]\d|grid [a-z]\d|cmp \d|instinct|radeon pro v\d|firepro s\d|data center gpu)")
});
static RE_OLAND: Lazy<Regex> =
    Lazy::new(|| re(r"\b(r[57] (240|330|340|430|435)|hd (8570|8670))\b"));
static RE_CAPE_VERDE: Lazy<Regex> =
    Lazy::new(|| re(r"\b(hd (7730|7750|7770|8740|8760)|r7 250[xe]?)\b"));

/// Compute / data-centre boards without display outputs.
pub(super) fn is_compute_card(n: &str) -> bool {
    RE_COMPUTE.is_match(n)
}

/// Retail names that are always Oland (no macOS driver).
pub(super) fn is_oland(n: &str) -> bool {
    RE_OLAND.is_match(n)
}

/// Retail names that are always Cape Verde (needs `radpg=15`).
pub(super) fn is_cape_verde(n: &str) -> bool {
    RE_CAPE_VERDE.is_match(n)
}

/// Every pattern of this module, so a test can compile them all.
#[cfg(test)]
pub(super) fn all_patterns() -> Vec<&'static Regex> {
    vec![
        &RE_VIRTUAL,
        &RE_NVIDIA,
        &RE_AMD,
        &RE_INTEL,
        &RE_BARE_HD4,
        &RE_INTEL_GMA,
        &RE_INTEL_ARC_DGPU,
        &RE_INTEL_LOW_POWER,
        &RE_INTEL_XE_CODENAME,
        &RE_INTEL_CODENAME,
        &RE_INTEL_IRIS_PRO,
        &RE_INTEL_IRIS_PLUS,
        &RE_INTEL_IRIS,
        &RE_INTEL_UHD,
        &RE_INTEL_HD,
        &RE_ARC_IGPU,
        &RE_AMD_BARE_GRAPHICS,
        &RE_AMD_NAVI_CHIP,
        &RE_AMD_CHIP,
        &RE_AMD_RX4,
        &RE_AMD_RX3,
        &RE_AMD_PRO,
        &RE_AMD_R_SERIES,
        &RE_AMD_R_GRAPHICS,
        &RE_AMD_HD,
        &RE_AMD_FIREPRO,
        &RE_AMD_APU_RDNA,
        &RE_AMD_LEXA_PLAIN,
        &RE_VEGA_DISCRETE,
        &RE_NV_CHIP,
        &RE_NV_TESLA_CHIP,
        &RE_NV_PRE_TESLA_CHIP,
        &RE_NV_MX,
        &RE_NV_GTX,
        &RE_NV_GTS,
        &RE_NV_GT,
        &RE_NV_NVS,
        &RE_NV_QUADRO,
        &RE_NV_DATACENTER,
        &RE_NV_PLAIN,
        &RE_COMPUTE,
        &RE_OLAND,
        &RE_CAPE_VERDE,
    ]
}
