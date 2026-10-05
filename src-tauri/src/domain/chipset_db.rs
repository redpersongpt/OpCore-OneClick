//! PCH / chipset identification from the LPC/eSPI bridge device id or from
//! board / product strings.

use super::model::CpuVendor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipsetInfo {
    /// "Z390", "B460", "HM370", "X570", ...
    pub name: String,
    pub vendor: CpuVendor,
    /// Intel PCH series (6, 7, 8, 9, 100, 200, 300, 400, 500, 600, 700, 800)
    /// or AMD socket family (300 for AM4 300-series, 600 for AM5 600-series, ...).
    pub series: u32,
    pub is_mobile: bool,
    pub is_hedt: bool,
}

impl ChipsetInfo {
    /// 300-series Intel boards lack native NVRAM: need SSDT-PMC (except Z370).
    pub fn needs_pmc(&self) -> bool {
        todo!()
    }
    /// Non-native XHCI controllers needing XHCI-unsupported.kext
    /// (H370/B360/H310 and Z390 on High Sierra, X79/X99, some 9-series/X299).
    pub fn needs_xhci_unsupported(&self) -> bool {
        todo!()
    }
}

/// Look up an Intel PCH / AMD FCH by the LPC bridge PCI id ("8086", "a305").
pub fn from_lpc(vendor_id: &str, device_id: &str) -> Option<ChipsetInfo> {
    todo!("from_lpc {vendor_id}:{device_id}")
}

/// Best-effort guess from a motherboard product name ("ROG STRIX Z390-E GAMING").
pub fn from_board_name(name: &str) -> Option<ChipsetInfo> {
    todo!("from_board_name {name}")
}
