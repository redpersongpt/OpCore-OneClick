//! PCH / chipset identification from the LPC/eSPI bridge device id or from
//! board / product strings.
//!
//! Intel device ids come from pci.ids (2026-10-01) cross-checked against
//! Hardware-Sniffer's chipset table; AMD chipsets are recognised by board name
//! or by the Promontory upstream/USB function ids, since the AMD FCH LPC bridge
//! (1022:790e) is identical on every board.

use super::model::{CpuVendor, MacOsVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipsetInfo {
    /// "Z390", "B460", "HM370", "X570", ...
    pub name: String,
    pub vendor: CpuVendor,
    /// Intel PCH series (6, 7, 8, 9, 100, 200, 300, 400, 500, 600, 700, 800)
    /// or AMD socket family (300 for AM4 300-series, 600 for AM5 600-series, ...).
    /// X58/ICH10 era Intel = 5/4/3, AMD 9-series (AM3+) = 9, chipsets without a
    /// numbered series (Atom SoCs, AMD FM1/FM2) = 0.
    pub series: u32,
    pub is_mobile: bool,
    pub is_hedt: bool,
}

impl ChipsetInfo {
    /// 300-series Intel desktop boards lack native NVRAM: need SSDT-PMC (except
    /// Z370). Dortania lists B360, B365, H310, H370 and Z390; Q370/Q360/C246/C242
    /// share the same Cannon Point-H firmware layout. Laptops are left out: the
    /// Dortania laptop guide adds SSDT-PMC only for 9th-gen Coffee Lake-H on an
    /// HM370/QM370 PCH (not for 8th gen), which depends on the CPU generation
    /// and is decided by the caller.
    pub fn needs_pmc(&self) -> bool {
        self.vendor == CpuVendor::Intel
            && self.series == 300
            && !self.is_mobile
            && self.name != "Z370"
    }

    /// Non-native XHCI controllers needing XHCI-unsupported.kext on at least one
    /// supported release (Dortania ktext.md): H370/B360/H310 (and the other
    /// Cannon Point-H desktop parts), Z390 on High Sierra, X79 and X99. The
    /// 9-series and X299 controllers are native and need nothing.
    pub fn needs_xhci_unsupported(&self) -> bool {
        self.vendor == CpuVendor::Intel
            && (XHCI_UNSUPPORTED_ALWAYS.contains(&self.name.as_str()) || self.name == "Z390")
    }

    /// Release- and board-vendor-aware variant of [`Self::needs_xhci_unsupported`].
    /// Z390's controller is native from Mojave on; ASRock Intel boards older than
    /// the 400 series need the kext as well (Dortania ktext.md).
    pub fn needs_xhci_unsupported_on(
        &self,
        target: MacOsVersion,
        board_vendor: Option<&str>,
    ) -> bool {
        if self.vendor != CpuVendor::Intel {
            return false;
        }
        if XHCI_UNSUPPORTED_ALWAYS.contains(&self.name.as_str()) {
            return true;
        }
        if self.name == "Z390" && target < MacOsVersion::Mojave {
            return true;
        }
        let asrock = board_vendor.is_some_and(|v| v.to_ascii_lowercase().contains("asrock"));
        asrock && !self.is_mobile && self.series >= 6 && self.series < 400
    }

    /// AMD AM4 desktop chipset (A320..X570).
    pub fn is_am4(&self) -> bool {
        self.vendor == CpuVendor::Amd && !self.is_hedt && matches!(self.series, 300..=500)
    }

    /// AMD AM5 desktop chipset (A620..X870E): needs the AM5 Booter/MMIO recipe.
    pub fn is_am5(&self) -> bool {
        self.vendor == CpuVendor::Amd && !self.is_hedt && matches!(self.series, 600 | 800)
    }
}

const XHCI_UNSUPPORTED_ALWAYS: &[&str] = &[
    "H370", "B360", "H310", "H310D", "Q370", "Q360", "C246", "C242", "X79", "C602", "C604", "C606",
    "C608", "X99", "C612",
];

#[derive(Debug, Clone, Copy)]
struct Def {
    name: &'static str,
    vendor: CpuVendor,
    series: u32,
    mobile: bool,
    hedt: bool,
}

impl Def {
    fn info(&self) -> ChipsetInfo {
        ChipsetInfo {
            name: self.name.to_string(),
            vendor: self.vendor,
            series: self.series,
            is_mobile: self.mobile,
            is_hedt: self.hedt,
        }
    }

    /// Real chipset model names can appear in board names; family placeholders
    /// such as "6 Series" or "Sunrise Point-LP" cannot.
    fn board_matchable(&self) -> bool {
        self.name.chars().all(|c| c.is_ascii_alphanumeric())
    }
}

const fn i(name: &'static str, series: u32) -> Def {
    Def {
        name,
        vendor: CpuVendor::Intel,
        series,
        mobile: false,
        hedt: false,
    }
}
const fn im(name: &'static str, series: u32) -> Def {
    Def {
        name,
        vendor: CpuVendor::Intel,
        series,
        mobile: true,
        hedt: false,
    }
}
const fn ih(name: &'static str, series: u32) -> Def {
    Def {
        name,
        vendor: CpuVendor::Intel,
        series,
        mobile: false,
        hedt: true,
    }
}
const fn a(name: &'static str, series: u32) -> Def {
    Def {
        name,
        vendor: CpuVendor::Amd,
        series,
        mobile: false,
        hedt: false,
    }
}
const fn ah(name: &'static str, series: u32) -> Def {
    Def {
        name,
        vendor: CpuVendor::Amd,
        series,
        mobile: false,
        hedt: true,
    }
}

static CHIPSETS: &[Def] = &[
    // Core 2 / Nehalem era
    i("ICH9", 3),
    im("ICH9M", 3),
    i("ICH10", 4),
    ih("X58", 5),
    // 5 series (Ibex Peak)
    i("5 Series", 5),
    im("Mobile 5 Series", 5),
    i("P55", 5),
    i("H55", 5),
    i("H57", 5),
    i("Q57", 5),
    i("3420", 5),
    i("3450", 5),
    im("PM55", 5),
    im("HM55", 5),
    im("HM57", 5),
    im("QM57", 5),
    im("QS57", 5),
    // 6 series (Cougar Point) and X79/C600 (Patsburg)
    i("6 Series", 6),
    im("Mobile 6 Series", 6),
    i("Z68", 6),
    i("P67", 6),
    i("H67", 6),
    i("H61", 6),
    i("B65", 6),
    i("Q65", 6),
    i("Q67", 6),
    i("C202", 6),
    i("C204", 6),
    i("C206", 6),
    im("HM65", 6),
    im("HM67", 6),
    im("UM67", 6),
    im("QM67", 6),
    im("QS67", 6),
    ih("X79", 6),
    ih("C602", 6),
    ih("C604", 6),
    ih("C606", 6),
    ih("C608", 6),
    // 7 series (Panther Point)
    i("7 Series", 7),
    i("Z77", 7),
    i("Z75", 7),
    i("H77", 7),
    i("B75", 7),
    i("Q75", 7),
    i("Q77", 7),
    i("C216", 7),
    im("HM70", 7),
    im("HM75", 7),
    im("HM76", 7),
    im("HM77", 7),
    im("UM77", 7),
    im("QM77", 7),
    im("QS77", 7),
    im("NM70", 7),
    // 8 series (Lynx Point)
    i("8 Series", 8),
    i("Z87", 8),
    i("Z85", 8),
    i("H87", 8),
    i("H81", 8),
    i("B85", 8),
    i("Q85", 8),
    i("Q87", 8),
    i("C222", 8),
    i("C224", 8),
    i("C226", 8),
    im("HM86", 8),
    im("HM87", 8),
    im("QM87", 8),
    im("Lynx Point-LP", 8),
    // 9 series (Wildcat Point) and X99 (Wellsburg)
    i("9 Series", 9),
    i("Z97", 9),
    i("H97", 9),
    im("HM97", 9),
    im("QM97", 9),
    im("Wildcat Point-LP", 9),
    ih("X99", 9),
    ih("C612", 9),
    // 100 series (Sunrise Point)
    i("100 Series", 100),
    i("Z170", 100),
    i("H170", 100),
    i("H110", 100),
    i("B150", 100),
    i("Q150", 100),
    i("Q170", 100),
    i("C232", 100),
    i("C236", 100),
    im("HM170", 100),
    im("QM170", 100),
    im("HM175", 100),
    im("QM175", 100),
    im("CM236", 100),
    im("CM238", 100),
    im("QMS180", 100),
    im("QMU185", 100),
    im("Sunrise Point-LP", 100),
    // 200 series (Union Point), X299/C422 and Lewisburg
    i("Z270", 200),
    i("H270", 200),
    i("B250", 200),
    i("Q250", 200),
    i("Q270", 200),
    ih("X299", 200),
    ih("C422", 200),
    ih("C621", 200),
    ih("C622", 200),
    ih("C624", 200),
    ih("C625", 200),
    ih("C626", 200),
    ih("C627", 200),
    ih("C628", 200),
    ih("C629", 200),
    ih("C620 Series", 200),
    // 300 series (Cannon Point; Z370, B365 and H310C reuse 200-series silicon)
    i("300 Series", 300),
    i("Z370", 300),
    i("Z390", 300),
    i("H370", 300),
    i("B360", 300),
    i("B365", 300),
    i("H310", 300),
    i("H310D", 300),
    i("Q370", 300),
    i("Q360", 300),
    i("C246", 300),
    i("C242", 300),
    im("HM370", 300),
    im("QM370", 300),
    im("CM246", 300),
    im("Cannon Point-LP", 300),
    // 400 series (Comet Lake PCH) and Ice Lake PCH-LP
    i("Z490", 400),
    i("H470", 400),
    i("B460", 400),
    i("H410", 400),
    i("Q470", 400),
    i("W480", 400),
    i("H420E", 400),
    im("HM470", 400),
    im("QM480", 400),
    im("WM490", 400),
    im("400 Series PCH-LP", 400),
    im("Ice Lake PCH-LP", 400),
    // 500 series (Rocket Lake / Tiger Lake)
    i("500 Series", 500),
    i("Z590", 500),
    i("H570", 500),
    i("B560", 500),
    i("H510", 500),
    i("Q570", 500),
    i("W580", 500),
    i("C252", 500),
    i("C256", 500),
    i("R580E", 500),
    im("RM590E", 500),
    im("HM570", 500),
    im("QM580", 500),
    im("WM590", 500),
    im("500 Series PCH-LP", 500),
    // 600 series (Alder Lake)
    i("Z690", 600),
    i("H670", 600),
    i("B660", 600),
    i("H610", 600),
    i("Q670", 600),
    i("W680", 600),
    i("R680E", 600),
    i("Q670E", 600),
    i("H610E", 600),
    im("HM670", 600),
    im("WM690", 600),
    im("Alder Lake PCH-P", 600),
    // 700 series (Raptor Lake) and W790
    i("Z790", 700),
    i("H770", 700),
    i("B760", 700),
    i("C262", 700),
    i("C266", 700),
    ih("W790", 700),
    im("HM770", 700),
    im("WM790", 700),
    im("Raptor Lake PCH-P", 700),
    // 800 series (Arrow Lake) and Core Ultra SoCs
    i("Z890", 800),
    i("B860", 800),
    i("H810", 800),
    i("Q870", 800),
    i("W880", 800),
    im("HM870", 800),
    im("WM880", 800),
    im("Meteor Lake SoC", 800),
    im("Arrow Lake-H SoC", 800),
    im("Arrow Lake-HX SoC", 800),
    im("Lunar Lake SoC", 800),
    im("Panther Lake SoC", 800),
    // Atom-class SoCs
    im("NM10", 0),
    im("Apollo Lake SoC", 0),
    im("Gemini Lake SoC", 0),
    im("Elkhart Lake SoC", 0),
    im("Jasper Lake SoC", 0),
    im("Alder Lake-N SoC", 0),
    // AMD FM1/FM2/FM2+ and AM3+
    a("A55", 0),
    a("A58", 0),
    a("A68H", 0),
    a("A75", 0),
    a("A78", 0),
    a("A85X", 0),
    a("A88X", 0),
    a("970", 9),
    a("990X", 9),
    a("990FX", 9),
    // AM4 (A300/X300: chipset-less ASRock DeskMini boards)
    a("AMD 300 Series", 300),
    a("A300", 300),
    a("X300", 300),
    a("A320", 300),
    a("B350", 300),
    a("X370", 300),
    a("AMD 400 Series", 400),
    a("B450", 400),
    a("X470", 400),
    a("AMD 500 Series", 500),
    a("A520", 500),
    a("B550", 500),
    a("X570", 500),
    // AM5
    a("AMD 600 Series", 600),
    a("A620", 600),
    a("B650", 600),
    a("B650E", 600),
    a("X670", 600),
    a("X670E", 600),
    a("AMD 800 Series", 800),
    a("B840", 800),
    a("B850", 800),
    a("X870", 800),
    a("X870E", 800),
    // Threadripper
    ah("X399", 300),
    ah("TRX40", 500),
    ah("WRX80", 500),
    ah("TRX50", 600),
    ah("WRX90", 600),
];

/// Intel LPC/eSPI bridge device id → chipset name.
static INTEL_LPC: &[(u16, &str)] = &[
    // ICH9 / ICH10
    (0x2912, "ICH9"),
    (0x2914, "ICH9"),
    (0x2916, "ICH9"),
    (0x2918, "ICH9"),
    (0x2917, "ICH9M"),
    (0x2919, "ICH9M"),
    (0x3a14, "ICH10"),
    (0x3a16, "ICH10"),
    (0x3a18, "ICH10"),
    (0x3a1a, "ICH10"),
    // 5 series
    (0x3b01, "Mobile 5 Series"),
    (0x3b05, "Mobile 5 Series"),
    (0x3b02, "P55"),
    (0x3b03, "PM55"),
    (0x3b06, "H55"),
    (0x3b07, "QM57"),
    (0x3b08, "H57"),
    (0x3b09, "HM55"),
    (0x3b0a, "Q57"),
    (0x3b0b, "HM57"),
    (0x3b0f, "QS57"),
    (0x3b14, "3420"),
    (0x3b16, "3450"),
    // 6 series
    (0x1c41, "Mobile 6 Series"),
    (0x1c43, "Mobile 6 Series"),
    (0x1c44, "Z68"),
    (0x1c46, "P67"),
    (0x1c47, "UM67"),
    (0x1c49, "HM65"),
    (0x1c4a, "H67"),
    (0x1c4b, "HM67"),
    (0x1c4c, "Q65"),
    (0x1c4d, "QS67"),
    (0x1c4e, "Q67"),
    (0x1c4f, "QM67"),
    (0x1c50, "B65"),
    (0x1c52, "C202"),
    (0x1c54, "C204"),
    (0x1c56, "C206"),
    (0x1c58, "B65"),
    (0x1c59, "HM67"),
    (0x1c5a, "Q67"),
    (0x1c5c, "H61"),
    (0x1d40, "X79"),
    (0x1d41, "X79"),
    // 7 series
    (0x1e44, "Z77"),
    (0x1e46, "Z75"),
    (0x1e47, "Q77"),
    (0x1e48, "Q75"),
    (0x1e49, "B75"),
    (0x1e4a, "H77"),
    (0x1e53, "C216"),
    (0x1e55, "QM77"),
    (0x1e56, "QS77"),
    (0x1e57, "HM77"),
    (0x1e58, "UM77"),
    (0x1e59, "HM76"),
    (0x1e5b, "UM77"),
    (0x1e5d, "HM75"),
    (0x1e5e, "HM70"),
    (0x1e5f, "NM70"),
    // 8 series
    (0x8c44, "Z87"),
    (0x8c46, "Z85"),
    (0x8c49, "HM86"),
    (0x8c4a, "H87"),
    (0x8c4b, "HM87"),
    (0x8c4c, "Q85"),
    (0x8c4e, "Q87"),
    (0x8c4f, "QM87"),
    (0x8c50, "B85"),
    (0x8c52, "C222"),
    (0x8c54, "C224"),
    (0x8c56, "C226"),
    (0x8c5c, "H81"),
    // 9 series
    (0x8cc3, "HM97"),
    (0x8cc4, "Z97"),
    (0x8cc5, "QM97"),
    (0x8cc6, "H97"),
    // 100 series
    (0xa143, "H110"),
    (0xa144, "H170"),
    (0xa145, "Z170"),
    (0xa146, "Q170"),
    (0xa147, "Q150"),
    (0xa148, "B150"),
    (0xa149, "C236"),
    (0xa14a, "C232"),
    (0xa14d, "QM170"),
    (0xa14e, "HM170"),
    (0xa150, "CM236"),
    (0xa152, "HM175"),
    (0xa153, "QM175"),
    (0xa151, "QMS180"),
    (0xa154, "CM238"),
    (0xa155, "QMU185"),
    // Lewisburg
    (0xa1c1, "C621"),
    (0xa1c2, "C622"),
    (0xa1c3, "C624"),
    (0xa1c4, "C625"),
    (0xa1c5, "C626"),
    (0xa1c6, "C627"),
    (0xa1c7, "C628"),
    (0xa1ca, "C629"),
    (0xa1c8, "C620 Series"),
    (0xa1cb, "C620 Series"),
    (0xa242, "C624"),
    (0xa243, "C627"),
    (0xa244, "C621"),
    (0xa245, "C627"),
    (0xa246, "C628"),
    // 200 series, X299/C422 and 300-series parts on 200-series silicon
    (0xa2c4, "H270"),
    (0xa2c5, "Z270"),
    (0xa2c6, "Q270"),
    (0xa2c7, "Q250"),
    (0xa2c8, "B250"),
    (0xa2c9, "Z370"),
    (0xa2ca, "H310"),
    (0xa2cc, "B365"),
    (0xa2d2, "X299"),
    (0xa2d3, "C422"),
    // 300 series
    (0xa303, "H310"),
    (0xa304, "H370"),
    (0xa305, "Z390"),
    (0xa306, "Q370"),
    (0xa307, "Q360"),
    (0xa308, "B360"),
    (0xa309, "C246"),
    (0xa30a, "C242"),
    (0xa30c, "QM370"),
    (0xa30d, "HM370"),
    (0xa30e, "CM246"),
    (0xa313, "300 Series"),
    (0x9d84, "Cannon Point-LP"),
    // 400 series
    (0x0284, "400 Series PCH-LP"),
    (0x0285, "400 Series PCH-LP"),
    (0x0684, "H470"),
    (0x0685, "Z490"),
    (0x0687, "Q470"),
    (0x068c, "QM480"),
    (0x068d, "HM470"),
    (0x068e, "WM490"),
    (0x0697, "W480"),
    (0x069a, "H420E"),
    (0xa3c8, "B460"),
    (0xa3da, "H410"),
    (0x3482, "Ice Lake PCH-LP"),
    (0x3882, "Ice Lake PCH-LP"),
    // 500 series
    (0x4381, "500 Series"),
    (0x4382, "500 Series"),
    (0x4383, "500 Series"),
    (0x4384, "Q570"),
    (0x4385, "Z590"),
    (0x4386, "H570"),
    (0x4387, "B560"),
    (0x4388, "H510"),
    (0x4389, "WM590"),
    (0x438a, "QM580"),
    (0x438b, "HM570"),
    (0x438c, "C252"),
    (0x438d, "C256"),
    (0x438e, "H310D"),
    (0x438f, "W580"),
    (0x4390, "RM590E"),
    (0x4391, "R580E"),
    (0xa082, "500 Series PCH-LP"),
    // 600 series
    (0x7a83, "Q670"),
    (0x7a84, "Z690"),
    (0x7a85, "H670"),
    (0x7a86, "B660"),
    (0x7a87, "H610"),
    (0x7a88, "W680"),
    (0x7a8c, "HM670"),
    (0x7a8d, "WM690"),
    (0x7a90, "R680E"),
    (0x7a91, "Q670E"),
    (0x7a92, "H610E"),
    (0x5181, "Alder Lake PCH-P"),
    (0x5182, "Alder Lake PCH-P"),
    (0x5187, "Alder Lake PCH-P"),
    // 700 series
    (0x7a04, "Z790"),
    (0x7a05, "H770"),
    (0x7a06, "B760"),
    (0x7a0c, "HM770"),
    (0x7a0d, "WM790"),
    (0x7a13, "C266"),
    (0x7a14, "C262"),
    (0x7a8a, "W790"),
    (0x519d, "Raptor Lake PCH-P"),
    (0x519e, "Raptor Lake PCH-P"),
    // 800 series and Core Ultra SoCs
    (0x7f03, "Q870"),
    (0x7f04, "Z890"),
    (0x7f06, "B860"),
    (0x7f07, "H810"),
    (0x7f08, "W880"),
    (0x7f0c, "HM870"),
    (0x7f0d, "WM880"),
    (0x7e01, "Meteor Lake SoC"),
    (0x7e02, "Meteor Lake SoC"),
    (0x7702, "Arrow Lake-H SoC"),
    (0x7730, "Arrow Lake-H SoC"),
    (0x7202, "Arrow Lake-H SoC"),
    (0xae10, "Arrow Lake-HX SoC"),
    (0xa806, "Lunar Lake SoC"),
    (0xa807, "Lunar Lake SoC"),
    (0x7203, "Lunar Lake SoC"),
    (0xe300, "Panther Lake SoC"),
    (0xe302, "Panther Lake SoC"),
    (0xe31f, "Panther Lake SoC"),
    (0xe400, "Panther Lake SoC"),
    (0xe402, "Panther Lake SoC"),
    (0xe41f, "Panther Lake SoC"),
    // Atom-class
    (0x27bc, "NM10"),
    (0x5ae8, "Apollo Lake SoC"),
    (0x31e8, "Gemini Lake SoC"),
    (0x3197, "Gemini Lake SoC"),
    (0x4b00, "Elkhart Lake SoC"),
    (0x4d87, "Jasper Lake SoC"),
    (0x5481, "Alder Lake-N SoC"),
];

/// Intel family ranges for LPC ids pci.ids only names generically.
static INTEL_LPC_RANGES: &[(u16, u16, &str)] = &[
    (0x3400, 0x3407, "X58"),
    (0x3b00, 0x3b1f, "5 Series"),
    (0x1c40, 0x1c5f, "6 Series"),
    (0x1e40, 0x1e5f, "7 Series"),
    (0x8c40, 0x8c5f, "8 Series"),
    (0x9c40, 0x9c47, "Lynx Point-LP"),
    (0x8cc0, 0x8cc7, "9 Series"),
    (0x9cc0, 0x9cc9, "Wildcat Point-LP"),
    (0x8d40, 0x8d4f, "X99"),
    (0xa140, 0xa15f, "100 Series"),
    (0x9d40, 0x9d5f, "Sunrise Point-LP"),
];

/// AMD chipset functions (Promontory upstream port / SATA / USB) → chipset.
static AMD_FUNCTIONS: &[(u16, &str)] = &[
    (0x43b0, "AMD 300 Series"),
    (0x43b4, "AMD 300 Series"),
    (0x43b5, "AMD 300 Series"),
    (0x43b7, "AMD 300 Series"),
    (0x43b8, "A320"),
    (0x43b9, "AMD 300 Series"),
    (0x43bb, "AMD 300 Series"),
    (0x43bc, "A320"),
    (0x43b1, "X399"),
    (0x43b6, "X399"),
    (0x43ba, "X399"),
    (0x43c6, "AMD 400 Series"),
    (0x43c7, "AMD 400 Series"),
    (0x43c8, "AMD 400 Series"),
    (0x43d5, "AMD 400 Series"),
    (0x43e9, "AMD 500 Series"),
    (0x43ea, "AMD 500 Series"),
    (0x43eb, "AMD 500 Series"),
    (0x43ee, "AMD 500 Series"),
    (0x43ec, "A520"),
    (0x57ad, "X570"),
    (0x43f4, "AMD 600 Series"),
    (0x43f5, "AMD 600 Series"),
    (0x43f6, "AMD 600 Series"),
    (0x43f7, "AMD 600 Series"),
    (0x43fa, "A620"),
    (0x43fc, "AMD 800 Series"),
    (0x43fd, "AMD 800 Series"),
];

fn def_by_name(name: &str) -> Option<&'static Def> {
    CHIPSETS.iter().find(|d| d.name.eq_ignore_ascii_case(name))
}

/// Look up a chipset by its marketing or family name ("Z390", "b550").
pub fn from_name(name: &str) -> Option<ChipsetInfo> {
    def_by_name(name.trim()).map(Def::info)
}

fn parse_hex_id(id: &str) -> Option<u16> {
    let t = id.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 4 {
        return None;
    }
    u16::from_str_radix(t, 16).ok()
}

/// Look up an Intel PCH / AMD FCH by the LPC bridge PCI id ("8086", "a305").
/// For AMD the generic FCH LPC bridge carries no chipset information; the
/// Promontory upstream port / USB / SATA function ids are accepted instead and
/// name only the series (B350 and X370 share silicon), so the board name
/// should refine an AMD result. X58 boards expose an ICH10 LPC bridge and are
/// reported as ICH10.
pub fn from_lpc(vendor_id: &str, device_id: &str) -> Option<ChipsetInfo> {
    let vendor = parse_hex_id(vendor_id)?;
    let device = parse_hex_id(device_id)?;
    let name = match vendor {
        0x8086 => INTEL_LPC
            .iter()
            .find(|(id, _)| *id == device)
            .map(|(_, n)| *n)
            .or_else(|| {
                INTEL_LPC_RANGES
                    .iter()
                    .find(|(lo, hi, _)| (*lo..=*hi).contains(&device))
                    .map(|(_, _, n)| *n)
            })?,
        0x1022 => AMD_FUNCTIONS
            .iter()
            .find(|(id, _)| *id == device)
            .map(|(_, n)| *n)?,
        _ => return None,
    };
    def_by_name(name).map(Def::info)
}

/// Best-effort guess from a motherboard product name ("ROG STRIX Z390-E GAMING").
/// Some laptop model names look like desktop boards (ASUS "X570ZD" is a Ryzen
/// laptop), so callers should not trust a desktop chipset match on a laptop.
pub fn from_board_name(name: &str) -> Option<ChipsetInfo> {
    let upper = name.to_ascii_uppercase();
    upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 3)
        .find_map(match_board_token)
        .map(Def::info)
}

/// Chipset from the LPC/eSPI id and the board name together. A specific LPC
/// match wins; a family-level one ("AMD 500 Series", "6 Series") is replaced
/// by a board-name match of the same family, and so is an id shared by a
/// different chipset on the same silicon (ICH10 on X58, X570 on TRX40/WRX80,
/// A620 on B840).
pub fn resolve(lpc: Option<(&str, &str)>, board_name: Option<&str>) -> Option<ChipsetInfo> {
    let from_id = lpc.and_then(|(vendor, device)| from_lpc(vendor, device));
    let from_board = board_name.and_then(from_board_name);
    match (from_id, from_board) {
        (Some(id), Some(board)) if board_refines(&id, &board) => Some(board),
        (Some(id), _) => Some(id),
        (None, board) => board,
    }
}

fn board_refines(id: &ChipsetInfo, board: &ChipsetInfo) -> bool {
    // Boards whose chipset exposes another chipset's ids: X58 uses an ICH10
    // southbridge, TRX40/WRX80 the X570 Promontory silicon and B840 the A620 one.
    let same_silicon = matches!(
        (id.name.as_str(), board.name.as_str()),
        ("ICH10", "X58") | ("X570", "TRX40" | "WRX80") | ("A620", "B840")
    );
    if same_silicon {
        return true;
    }
    let generic = def_by_name(&id.name).is_some_and(|d| !d.board_matchable());
    // AMD 800-series boards reuse the 600-series Promontory 21 silicon.
    let am5 = |c: &ChipsetInfo| c.vendor == CpuVendor::Amd && matches!(c.series, 600 | 800);
    let same_family = id.vendor == board.vendor
        && id.is_mobile == board.is_mobile
        && (id.series == board.series || (am5(id) && am5(board)));
    generic && same_family
}

/// Match one board-name token: an exact chipset name, optionally behind an old
/// ASUS prefix ("P8Z77", "P6X58D"), Gigabyte's AMD "A" prefix ("AB350M") or an
/// FM2 socket prefix ("F2A88XM", "FM2A88X"), followed by at most three
/// form-factor letters ("B460M", "H310CM", "Z68XP").
/// All-digit names ("970", "3420") only match with a letter suffix ("970A") so
/// that OEM model numbers such as "Inspiron 3420" are not taken for chipsets.
fn match_board_token(token: &str) -> Option<&'static Def> {
    let mut candidates: Vec<(&str, bool)> = vec![(token, false)];
    for prefix in ["P6", "P7", "P8", "P9"] {
        if let Some(rest) = token.strip_prefix(prefix) {
            candidates.push((rest, false));
        }
    }
    for prefix in ["A", "F2", "FM2"] {
        if let Some(rest) = token.strip_prefix(prefix) {
            candidates.push((rest, true));
        }
    }
    for (candidate, amd_only) in candidates {
        let best = CHIPSETS
            .iter()
            .filter(|d| d.board_matchable() && (!amd_only || d.vendor == CpuVendor::Amd))
            .filter(|d| {
                let digits_only = d.name.chars().all(|c| c.is_ascii_digit());
                candidate.strip_prefix(d.name).is_some_and(|rest| {
                    rest.len() <= 3
                        && rest.chars().all(|c| c.is_ascii_alphabetic())
                        && (!digits_only || !rest.is_empty())
                })
            })
            .max_by_key(|d| d.name.len());
        if best.is_some() {
            return best;
        }
    }
    None
}

/// AMD_Vanilla's PCI enumeration fix is limited to these AM5 boards with
/// onboard Thunderbolt/USB4 and Wi-Fi enabled.
pub fn needs_amd_hotplug_fix(
    profile: &crate::domain::model::HardwareProfile,
    chipset: Option<&ChipsetInfo>,
) -> bool {
    const BOARDS: &[&str] = &[
        "CROSSHAIR X670E HERO",
        "CROSSHAIR X670E GENE",
        "CROSSHAIR X670E EXTREME",
        "PROART X670E-CREATOR",
    ];
    let board = profile
        .motherboard_model
        .to_ascii_uppercase()
        .replace("ROG ", "");
    chipset.is_some_and(ChipsetInfo::is_am5)
        && profile.wifi.is_some()
        && BOARDS.iter().any(|b| board.contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lpc(dev: &str) -> ChipsetInfo {
        from_lpc("8086", dev).unwrap_or_else(|| panic!("no chipset for 8086:{dev}"))
    }

    fn board(name: &str) -> ChipsetInfo {
        from_board_name(name).unwrap_or_else(|| panic!("no chipset for board {name}"))
    }

    #[test]
    fn intel_lpc_lookup_by_generation() {
        let cases: &[(&str, &str, u32, bool)] = &[
            ("3b06", "H55", 5, false),
            ("3b09", "HM55", 5, true),
            ("1c44", "Z68", 6, false),
            ("1c5c", "H61", 6, false),
            ("1c49", "HM65", 6, true),
            ("1e44", "Z77", 7, false),
            ("1e57", "HM77", 7, true),
            ("8c44", "Z87", 8, false),
            ("8c4f", "QM87", 8, true),
            ("8cc4", "Z97", 9, false),
            ("a145", "Z170", 100, false),
            ("a14e", "HM170", 100, true),
            ("a2c5", "Z270", 200, false),
            ("a2c9", "Z370", 300, false),
            ("a305", "Z390", 300, false),
            ("a308", "B360", 300, false),
            ("a2cc", "B365", 300, false),
            ("a30d", "HM370", 300, true),
            ("0685", "Z490", 400, false),
            ("a3c8", "B460", 400, false),
            ("068d", "HM470", 400, true),
            ("4385", "Z590", 500, false),
            ("4387", "B560", 500, false),
            ("7a84", "Z690", 600, false),
            ("7a86", "B660", 600, false),
            ("7a04", "Z790", 700, false),
            ("7a06", "B760", 700, false),
            ("7f04", "Z890", 800, false),
            ("7f06", "B860", 800, false),
            ("7f0c", "HM870", 800, true),
        ];
        for (dev, name, series, mobile) in cases {
            let c = lpc(dev);
            assert_eq!(c.name, *name, "8086:{dev}");
            assert_eq!(c.series, *series, "8086:{dev}");
            assert_eq!(c.is_mobile, *mobile, "8086:{dev}");
            assert_eq!(c.vendor, CpuVendor::Intel);
        }
    }

    #[test]
    fn hedt_lpc_ids() {
        for (dev, name) in [
            ("1d41", "X79"),
            ("8d47", "X99"),
            ("a2d2", "X299"),
            ("a2d3", "C422"),
            ("a1c1", "C621"),
            ("3405", "X58"),
        ] {
            let c = lpc(dev);
            assert_eq!(c.name, name);
            assert!(c.is_hedt, "{name} should be HEDT");
        }
        assert!(!lpc("a305").is_hedt);
    }

    #[test]
    fn generic_lpc_ranges_and_laptop_pchs() {
        assert_eq!(lpc("1c42").name, "6 Series");
        assert_eq!(lpc("9c43").name, "Lynx Point-LP");
        assert!(lpc("9c43").is_mobile);
        assert_eq!(lpc("9d4e").name, "Sunrise Point-LP");
        assert_eq!(lpc("9d84").name, "Cannon Point-LP");
        assert_eq!(lpc("0284").series, 400);
        assert_eq!(lpc("3482").name, "Ice Lake PCH-LP");
        assert_eq!(lpc("a082").series, 500);
        assert_eq!(lpc("5181").series, 600);
        assert_eq!(lpc("519d").series, 700);
        assert_eq!(lpc("4d87").name, "Jasper Lake SoC");
    }

    #[test]
    fn lpc_id_formats() {
        assert_eq!(
            from_lpc("0x8086", "0xA305").map(|c| c.name),
            Some("Z390".into())
        );
        assert_eq!(
            from_lpc(" 8086 ", "A305").map(|c| c.name),
            Some("Z390".into())
        );
        assert!(from_lpc("8086", "zzzz").is_none());
        assert!(from_lpc("8086", "").is_none());
        assert!(from_lpc("8086", "12345").is_none());
        assert!(from_lpc("10de", "a305").is_none());
        assert!(from_lpc("8086", "ffff").is_none());
    }

    #[test]
    fn amd_function_ids() {
        assert!(
            from_lpc("1022", "790e").is_none(),
            "generic FCH LPC bridge carries no chipset"
        );
        assert_eq!(
            from_lpc("1022", "57ad").map(|c| c.name),
            Some("X570".into())
        );
        assert_eq!(
            from_lpc("1022", "43ec").map(|c| c.name),
            Some("A520".into())
        );
        assert_eq!(from_lpc("1022", "43fa").map(|c| c.series), Some(600));
        let x399 = from_lpc("1022", "43ba").unwrap();
        assert!(x399.is_hedt);
        assert_eq!(x399.vendor, CpuVendor::Amd);
    }

    #[test]
    fn board_names() {
        let cases: &[(&str, &str)] = &[
            ("ROG STRIX Z390-E GAMING", "Z390"),
            ("B550 AORUS ELITE", "B550"),
            ("PRIME X299-A II", "X299"),
            ("MAG B660M MORTAR", "B660"),
            ("MAG B660M MORTAR WIFI DDR4", "B660"),
            ("TUF GAMING X570-PLUS (WI-FI)", "X570"),
            ("PRIME B460M-A", "B460"),
            ("H310CM-HDV", "H310"),
            ("B365M DS3H", "B365"),
            ("Z370P D3", "Z370"),
            ("P8Z77-V LX", "Z77"),
            ("P8H61-M LE", "H61"),
            ("P8P67 PRO", "P67"),
            ("P6X58D-E", "X58"),
            ("GA-X58A-UD3R", "X58"),
            ("GA-Z77X-UD3H", "Z77"),
            ("Z68XP-UD3", "Z68"),
            ("GA-B85M-D3H", "B85"),
            ("X99-A II", "X99"),
            ("X79-DELUXE", "X79"),
            ("Z590I AORUS ULTRA", "Z590"),
            ("ROG STRIX Z690-I GAMING WIFI", "Z690"),
            ("AB350M-DS3H", "B350"),
            ("AX370-Gaming 5", "X370"),
            ("A320M-K", "A320"),
            ("B450M DS3H", "B450"),
            ("X470 AORUS ULTRA GAMING", "X470"),
            ("MEG X570S ACE MAX", "X570"),
            ("PRO B650M-P", "B650"),
            ("ROG STRIX B650E-F GAMING WIFI", "B650E"),
            ("X670E AORUS MASTER", "X670E"),
            ("X870E AORUS MASTER", "X870E"),
            ("PRO B840-P WIFI", "B840"),
            ("TRX40 AORUS XTREME", "TRX40"),
            ("Pro WS WRX80E-SAGE SE WIFI", "WRX80"),
            ("TRX50 AERO D", "TRX50"),
            ("Pro WS WRX90E-SAGE SE", "WRX90"),
            ("MEG X399 CREATION", "X399"),
            ("990FXA-UD3", "990FX"),
            ("970A-DS3P", "970"),
            ("A88XM-PLUS", "A88X"),
            ("F2A88XM-D3H", "A88X"),
            ("FM2A88X Extreme6+", "A88X"),
            ("Z890 AORUS MASTER", "Z890"),
            ("B860M AORUS ELITE WIFI6E", "B860"),
            ("PRIME H610M-E D4", "H610"),
            ("Z790 AORUS ELITE AX", "Z790"),
            ("W480-CREATOR", "W480"),
        ];
        for (name, want) in cases {
            assert_eq!(board(name).name, *want, "board {name}");
        }
    }

    #[test]
    fn board_names_without_chipset() {
        for name in [
            "ROG MAXIMUS XI HERO",
            "0X3D66",
            "",
            "System Product Name",
            "ROG CROSSHAIR VIII HERO",
            "Default string",
            "X570AORUSMASTERX",
            "Inspiron 3420",
            "970 Series",
        ] {
            assert!(from_board_name(name).is_none(), "{name}");
        }
    }

    #[test]
    fn board_vendor_and_series() {
        let c = board("B550 AORUS ELITE");
        assert_eq!(c.vendor, CpuVendor::Amd);
        assert_eq!(c.series, 500);
        let c = board("ROG STRIX Z390-E GAMING");
        assert_eq!(c.vendor, CpuVendor::Intel);
        assert_eq!(c.series, 300);
        assert!(!c.is_mobile);
        assert!(board("TRX50 AERO D").is_hedt);
        assert!(board("PRIME X299-A II").is_hedt);
    }

    #[test]
    fn pmc_rules() {
        for name in ["Z390", "B360", "B365", "H310", "H370", "Q370", "C246"] {
            assert!(from_name(name).unwrap().needs_pmc(), "{name}");
        }
        for name in [
            "Z370", "HM370", "QM370", "Z490", "B460", "Z270", "Z690", "B550", "X299",
        ] {
            assert!(!from_name(name).unwrap().needs_pmc(), "{name}");
        }
        assert!(!lpc("9d84").needs_pmc());
    }

    #[test]
    fn xhci_unsupported_rules() {
        for name in ["H370", "B360", "H310", "Z390", "X79", "X99"] {
            assert!(from_name(name).unwrap().needs_xhci_unsupported(), "{name}");
        }
        for name in [
            "Z370", "B365", "Z490", "B460", "Z690", "X299", "B550", "X570", "HM370",
        ] {
            assert!(!from_name(name).unwrap().needs_xhci_unsupported(), "{name}");
        }
        let z390 = from_name("Z390").unwrap();
        assert!(z390.needs_xhci_unsupported_on(MacOsVersion::HighSierra, None));
        assert!(!z390.needs_xhci_unsupported_on(MacOsVersion::Mojave, None));
        assert!(from_name("H370")
            .unwrap()
            .needs_xhci_unsupported_on(MacOsVersion::Sequoia, None));
        let z270 = from_name("Z270").unwrap();
        assert!(z270.needs_xhci_unsupported_on(MacOsVersion::Ventura, Some("ASRock")));
        assert!(
            !z270.needs_xhci_unsupported_on(MacOsVersion::Ventura, Some("ASUSTeK COMPUTER INC."))
        );
        let z490 = from_name("Z490").unwrap();
        assert!(!z490.needs_xhci_unsupported_on(MacOsVersion::Ventura, Some("ASRock")));
        assert!(!from_name("X570")
            .unwrap()
            .needs_xhci_unsupported_on(MacOsVersion::Ventura, Some("ASRock")));
    }

    #[test]
    fn extra_lpc_ids() {
        assert_eq!(lpc("438e").name, "H310D");
        assert!(lpc("438e").needs_pmc());
        assert_eq!(lpc("27bc").name, "NM10");
        assert!(lpc("27bc").is_mobile);
        assert_eq!(lpc("3197").name, "Gemini Lake SoC");
        assert_eq!(lpc("4390").series, 500);
        assert_eq!(lpc("7a92").name, "H610E");
        assert_eq!(lpc("4382").name, "500 Series");
        assert!(lpc("a1cb").is_hedt);
        assert_eq!(lpc("519e").series, 700);
        assert_eq!(lpc("a155").name, "QMU185");
        assert_eq!(lpc("a2ca").name, "H310");
        assert!(lpc("a2ca").needs_pmc());
    }

    #[test]
    fn amd_socket_helpers() {
        for name in [
            "A320", "B350", "X370", "B450", "X470", "A520", "B550", "X570",
        ] {
            let c = from_name(name).unwrap();
            assert!(c.is_am4(), "{name}");
            assert!(!c.is_am5(), "{name}");
        }
        for name in [
            "A620", "B650", "B650E", "X670E", "B840", "B850", "X870", "X870E",
        ] {
            let c = from_name(name).unwrap();
            assert!(c.is_am5(), "{name}");
            assert!(!c.is_am4(), "{name}");
        }
        for name in [
            "X399", "TRX40", "WRX80", "TRX50", "WRX90", "Z390", "990FX", "A88X",
        ] {
            let c = from_name(name).unwrap();
            assert!(!c.is_am4() && !c.is_am5(), "{name}");
        }
    }

    #[test]
    fn resolve_combines_lpc_and_board() {
        let c = resolve(Some(("1022", "43e9")), Some("B550 AORUS ELITE")).unwrap();
        assert_eq!(c.name, "B550");
        let c = resolve(Some(("1022", "43e9")), None).unwrap();
        assert_eq!(c.name, "AMD 500 Series");
        // A board name from another family never overrides the id.
        let c = resolve(Some(("1022", "43e9")), Some("X670E AORUS MASTER")).unwrap();
        assert_eq!(c.name, "AMD 500 Series");
        let c = resolve(Some(("1022", "43f4")), Some("X870E AORUS MASTER")).unwrap();
        assert_eq!(c.name, "X870E");
        let c = resolve(Some(("8086", "a305")), Some("PRIME B360M-A")).unwrap();
        assert_eq!(c.name, "Z390", "a specific LPC match wins");
        let c = resolve(Some(("8086", "1c42")), Some("P8Z68-V PRO")).unwrap();
        assert_eq!(c.name, "Z68");
        let c = resolve(Some(("8086", "3a16")), Some("GA-X58A-UD3R")).unwrap();
        assert_eq!(c.name, "X58");
        assert!(c.is_hedt);
        let c = resolve(Some(("1022", "790e")), Some("ROG STRIX X570-E GAMING")).unwrap();
        assert_eq!(c.name, "X570");
        assert!(resolve(Some(("1022", "790e")), Some("Default string")).is_none());
        assert!(resolve(None, None).is_none());
    }

    #[test]
    fn workstation_and_mini_pc_chipsets() {
        for name in ["C602", "C606", "C612"] {
            let c = from_name(name).unwrap();
            assert!(c.is_hedt, "{name}");
            assert!(c.needs_xhci_unsupported(), "{name}");
            assert!(!c.needs_pmc(), "{name}");
        }
        assert_eq!(board("P9X79 WS").name, "X79");
        for name in ["A300M-STX", "X300M-STX"] {
            let c = board(name);
            assert!(c.is_am4(), "{name}");
            assert_eq!(c.vendor, CpuVendor::Amd);
        }
        assert!(from_board_name("X3000").is_none());
    }

    #[test]
    fn mobile_generic_lpc_ids() {
        for dev in ["3b01", "3b05", "1c41", "1c43"] {
            assert!(lpc(dev).is_mobile, "8086:{dev}");
        }
        assert!(!lpc("1c42").is_mobile);
        // A desktop board name never turns a mobile PCH into a desktop one.
        let c = resolve(Some(("8086", "1c43")), Some("P8Z68-V PRO")).unwrap();
        assert_eq!(c.name, "Mobile 6 Series");
    }

    #[test]
    fn resolve_same_silicon_chipsets() {
        let c = resolve(Some(("1022", "57ad")), Some("TRX40 AORUS XTREME")).unwrap();
        assert_eq!(c.name, "TRX40");
        assert!(c.is_hedt);
        let c = resolve(Some(("1022", "57ad")), Some("Pro WS WRX80E-SAGE SE WIFI")).unwrap();
        assert_eq!(c.name, "WRX80");
        let c = resolve(Some(("1022", "57ad")), Some("X670E AORUS MASTER")).unwrap();
        assert_eq!(c.name, "X570", "another family keeps the id");
        let c = resolve(Some(("1022", "43fa")), Some("PRO B840-P WIFI")).unwrap();
        assert_eq!(c.name, "B840");
        assert!(c.is_am5());
        let c = resolve(Some(("1022", "43fa")), Some("PRO B650-P WIFI")).unwrap();
        assert_eq!(c.name, "A620");
    }

    #[test]
    fn table_integrity() {
        for (id, name) in INTEL_LPC {
            assert!(
                def_by_name(name).is_some(),
                "LPC {id:04x} names unknown chipset {name}"
            );
        }
        for (_, _, name) in INTEL_LPC_RANGES {
            assert!(
                def_by_name(name).is_some(),
                "range names unknown chipset {name}"
            );
        }
        for (id, name) in AMD_FUNCTIONS {
            let d = def_by_name(name)
                .unwrap_or_else(|| panic!("AMD {id:04x} names unknown chipset {name}"));
            assert_eq!(d.vendor, CpuVendor::Amd);
        }
        let mut names: Vec<&str> = CHIPSETS.iter().map(|d| d.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CHIPSETS.len(), "duplicate chipset names");
        let mut ids: Vec<u16> = INTEL_LPC.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), INTEL_LPC.len(), "duplicate LPC ids");
    }
}
