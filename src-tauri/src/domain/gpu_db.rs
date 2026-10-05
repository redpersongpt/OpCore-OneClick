//! GPU identification and macOS support, keyed by PCI vendor/device id with
//! name parsing as fallback.

use super::model::{GpuFamily, GpuVendor, MacOsVersion, ProfileGpu};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuIdentity {
    pub vendor: GpuVendor,
    pub family: GpuFamily,
    pub is_igpu: bool,
    /// Friendly model name when known from the id table ("Radeon RX 6600 XT").
    pub model_name: Option<String>,
}

/// Identify a GPU from PCI ids (lowercase hex, no prefix) and/or its name.
pub fn identify(vendor_id: Option<&str>, device_id: Option<&str>, name: &str) -> GpuIdentity {
    todo!("identify {vendor_id:?} {device_id:?} {name}")
}

/// Extra requirement to drive this GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuRequirement {
    /// WhateverGreen + standard properties.
    Standard,
    /// NootRX replaces WhateverGreen (Navi 21/22/23 variants Apple never shipped).
    NootRx,
    /// NootedRed replaces WhateverGreen (Vega APUs).
    NootedRed,
    /// Needs a `device-id` spoof (bytes, little endian) to a supported id.
    DeviceIdSpoof([u8; 4]),
}

#[derive(Debug, Clone)]
pub struct GpuSupport {
    pub family: GpuFamily,
    /// Can macOS show a picture through this GPU on some release.
    pub display_capable: bool,
    /// First release with drivers.
    pub min_native: Option<MacOsVersion>,
    /// Last release with native (unpatched) drivers; None = still supported in Tahoe.
    pub max_native: Option<MacOsVersion>,
    /// Newest release reachable with OCLP root patches after install (None = no patch path).
    pub max_with_root_patch: Option<MacOsVersion>,
    pub requirement: GpuRequirement,
    /// Recommended boot-args for this GPU (e.g. "agdpmod=pikera" for Navi).
    pub boot_args: Vec<&'static str>,
    pub notes: Vec<String>,
}

/// Support facts for one concrete GPU (device-id specific where it matters,
/// e.g. RX 6600 vs RX 6500, Navi 21 XTXH, Lexa spoof, UHD 630 desktop vs mobile).
pub fn support(gpu: &ProfileGpu) -> GpuSupport {
    todo!("support {:?}", gpu.family)
}

/// Native (no root patch) support of `gpu` on `version`.
pub fn natively_supported_on(gpu: &ProfileGpu, version: MacOsVersion) -> bool {
    let s = support(gpu);
    s.display_capable
        && s.min_native.is_none_or(|min| min <= version)
        && s.max_native.is_none_or(|max| version <= max)
}

/// All families in UI order with labels (for the manual editor).
pub fn all_families() -> &'static [(GpuFamily, &'static str)] {
    todo!()
}
