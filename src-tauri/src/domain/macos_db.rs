//! Per-macOS-release facts: hardware floors and Apple recovery parameters.

use super::model::MacOsVersion;

/// Parameters for `osrecovery.apple.com/InstallationPayload/RecoveryImage`,
/// matching OpenCorePkg `Utilities/macrecovery/recovery_urls.txt` (1.0.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub board_id: &'static str,
    pub mlb: &'static str,
    /// "default" or "latest" (`os=` field of the request body).
    pub os_type: &'static str,
}

/// Recovery board-id / MLB / os type for each release.
///
/// High Sierra `Mac-7BA5B2D9E42DDD94` + `00000000000J80300`, Mojave
/// `Mac-7BA5B2DFE22DDD8C` + `00000000000KXPG00`, Catalina `Mac-00BE6ED71E35EB86`,
/// Big Sur `Mac-2BD1B31983FE1663`, Monterey `Mac-E43C1C25D4880AD6`, Ventura
/// `Mac-B4831CEBD52A0C4C`, Sonoma `Mac-827FAC58A8FDFA22`, Sequoia
/// `Mac-7BA5B2D9E42DDD94`, Tahoe `Mac-CFF7D910A743CAAF` with `os=latest`
/// (zero MLB `00000000000000000` from Catalina on).
pub fn recovery_request(version: MacOsVersion) -> RecoveryRequest {
    todo!("recovery_request for {version:?}")
}

/// True when this release needs AVX2 (macOS 13+). Pre-Haswell CPUs need
/// CryptexFixup for these releases.
pub fn requires_avx2(version: MacOsVersion) -> bool {
    version >= MacOsVersion::Ventura
}

/// Human-readable, release-specific caveats shown in the version picker
/// (e.g. Tahoe: AppleHDA removed, AirportItlwm unavailable; Sonoma: Broadcom
/// BCM4360 family needs root patch; Ventura: Skylake iGPU spoof, AVX2).
pub fn release_caveats(version: MacOsVersion) -> Vec<&'static str> {
    todo!("release_caveats for {version:?}")
}
