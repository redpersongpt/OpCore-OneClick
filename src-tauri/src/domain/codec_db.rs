//! HD Audio codec database and AppleALC layout-id selection. Data comes from
//! AppleALC 1.9.8 `Resources/*/Info.plist` + `HDAConfigDefault`
//! (`data/applealc_codecs.json`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecInfo {
    /// 0xVVVVDDDD
    pub id: u32,
    /// "Realtek ALC897"
    pub name: String,
    /// All layout ids AppleALC ships for this codec.
    pub layouts: Vec<u32>,
}

pub fn lookup(codec_id: u32) -> Option<CodecInfo> {
    todo!("lookup {codec_id:#x}")
}

/// Friendly name for any codec id, even ones AppleALC does not support.
pub fn codec_name(codec_id: u32) -> String {
    todo!("codec_name {codec_id:#x}")
}

/// Deterministic default layout-id for a codec (never random). `is_laptop`
/// and the codec subsystem id may be used to prefer OEM-specific layouts.
pub fn default_layout(codec_id: u32, subsystem: Option<u32>, is_laptop: bool) -> Option<u32> {
    todo!("default_layout {codec_id:#x} {subsystem:?} {is_laptop}")
}

/// True for HDMI/DP codecs (Intel 8086:28xx, AMD 1002:aaxx, NVIDIA 10de:xxxx).
pub fn is_hdmi_codec(vendor_id: u16, device_id: u16) -> bool {
    todo!("is_hdmi_codec {vendor_id:#x}:{device_id:#x}")
}
