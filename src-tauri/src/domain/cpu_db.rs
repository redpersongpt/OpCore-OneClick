//! CPU identification: CPUID family/model/stepping first, brand-string parsing
//! as fallback. Covers every Intel generation from Penryn to Arrow/Lunar Lake,
//! Xeon E3/E5/E7/W/Scalable, Atom-class, and AMD K10 through Zen 5.

use super::model::{CpuPlatform, CpuVendor, MacOsVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuIdentity {
    pub vendor: CpuVendor,
    pub platform: CpuPlatform,
    /// Display codename ("Coffee Lake-S", "Comet Lake-U", "Vermeer").
    pub codename: String,
    pub is_mobile: bool,
    pub is_hybrid: bool,
    /// Expected feature support for this platform when CPUID flags are unknown.
    pub has_avx2: bool,
}

/// Identify a CPU. `vendor` is the CPUID vendor string or a friendly name
/// ("GenuineIntel", "Intel", "AuthenticAMD", "AMD"). `family`/`model` are
/// CPUID *display* values (extended family/model already folded in).
pub fn identify(
    name: &str,
    vendor: &str,
    family: Option<u32>,
    model: Option<u32>,
    stepping: Option<u32>,
) -> CpuIdentity {
    todo!("identify {name} {vendor} {family:?} {model:?} {stepping:?}")
}

#[derive(Debug, Clone, Copy)]
pub struct PlatformInfo {
    pub platform: CpuPlatform,
    pub vendor: CpuVendor,
    pub label: &'static str,
    /// Platform can run macOS through OpenCore at all.
    pub supported: bool,
    /// HEDT / workstation platform (X58/X79/X99/X299, Threadripper).
    pub hedt: bool,
    pub has_avx2: bool,
    /// Newest macOS the CPU itself can run (None = Tahoe / no CPU-imposed cap).
    /// CPU caps are independent of the GPU; e.g. pre-AVX2 → Monterey unless CryptexFixup.
    pub max_macos: Option<MacOsVersion>,
    /// Oldest macOS that recognises the platform (e.g. Comet Lake → Catalina 10.15.4).
    pub min_macos: Option<MacOsVersion>,
    pub notes: &'static [&'static str],
}

pub fn platform_info(platform: CpuPlatform) -> PlatformInfo {
    todo!("platform_info {platform:?}")
}

/// All platforms in UI order (for the manual editor).
pub fn all_platforms() -> &'static [CpuPlatform] {
    todo!()
}
