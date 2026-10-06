//! Per-macOS-release facts: hardware floors and Apple recovery parameters.

use super::model::MacOsVersion;

/// MLB sent for releases whose board serves its newest image to any serial.
pub const MLB_ZERO: &str = "00000000000000000";

/// Parameters for `osrecovery.apple.com/InstallationPayload/RecoveryImage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub board_id: &'static str,
    pub mlb: &'static str,
    /// "default" or "latest" (`os=` field of the request body).
    pub os_type: &'static str,
}

const fn req(board_id: &'static str, mlb: &'static str, os_type: &'static str) -> RecoveryRequest {
    RecoveryRequest { board_id, mlb, os_type }
}

/// Board-ids whose newest supported release is the target, so Apple serves
/// that release. Verified against osrecovery.apple.com on 2026-10-05
/// (OpenCorePkg 1.0.8 `recovery_urls.txt`, Dortania). `os=latest` picks the
/// newer point release where it differs; Big Sur's `latest` image is older
/// (11.5.2) than `default` (11.6). Tahoe needs `latest`: with `default` the
/// same boards return Sequoia. Later entries are fallbacks.
const HIGH_SIERRA: &[RecoveryRequest] = &[
    req("Mac-7BA5B2D9E42DDD94", "00000000000J80300", "default"),
    req("Mac-BE088AF8C5EB4FA2", "00000000000J80300", "default"),
];
const MOJAVE: &[RecoveryRequest] = &[req("Mac-7BA5B2DFE22DDD8C", "00000000000KXPG00", "default")];
const CATALINA: &[RecoveryRequest] = &[
    req("Mac-00BE6ED71E35EB86", MLB_ZERO, "default"),
    req("Mac-CFF7D910A743CAAF", "00000000000PHCD00", "default"),
];
const BIG_SUR: &[RecoveryRequest] = &[
    req("Mac-2BD1B31983FE1663", MLB_ZERO, "default"),
    req("Mac-42FD25EABCABB274", MLB_ZERO, "default"),
];
const MONTEREY: &[RecoveryRequest] = &[
    req("Mac-E43C1C25D4880AD6", MLB_ZERO, "latest"),
    req("Mac-FFE5EF870D7BA81A", MLB_ZERO, "latest"),
];
const VENTURA: &[RecoveryRequest] = &[
    req("Mac-B4831CEBD52A0C4C", MLB_ZERO, "latest"),
    req("Mac-4B682C642B45593E", MLB_ZERO, "latest"),
];
const SONOMA: &[RecoveryRequest] = &[
    req("Mac-827FAC58A8FDFA22", MLB_ZERO, "latest"),
    req("Mac-226CB3C6A851A671", MLB_ZERO, "latest"),
];
const SEQUOIA: &[RecoveryRequest] = &[
    req("Mac-7BA5B2D9E42DDD94", MLB_ZERO, "latest"),
    req("Mac-937A206F2EE63C01", MLB_ZERO, "latest"),
];
const TAHOE: &[RecoveryRequest] = &[
    req("Mac-CFF7D910A743CAAF", MLB_ZERO, "latest"),
    req("Mac-27AD2F918AE68F61", MLB_ZERO, "latest"),
    req("Mac-AF89B6D9451A490B", MLB_ZERO, "latest"),
];

/// All requests for `version`, preferred first.
pub fn recovery_requests(version: MacOsVersion) -> &'static [RecoveryRequest] {
    match version {
        MacOsVersion::HighSierra => HIGH_SIERRA,
        MacOsVersion::Mojave => MOJAVE,
        MacOsVersion::Catalina => CATALINA,
        MacOsVersion::BigSur => BIG_SUR,
        MacOsVersion::Monterey => MONTEREY,
        MacOsVersion::Ventura => VENTURA,
        MacOsVersion::Sonoma => SONOMA,
        MacOsVersion::Sequoia => SEQUOIA,
        MacOsVersion::Tahoe => TAHOE,
    }
}

/// Recovery board-id / MLB / os type for each release (the preferred entry
/// of [`recovery_requests`]).
pub fn recovery_request(version: MacOsVersion) -> RecoveryRequest {
    recovery_requests(version)[0]
}

/// Recovery images Apple served on 2026-10-05, by the product id in the
/// download path (`…/<product>/<token>/RecoveryImage/BaseSystem.dmg`).
const KNOWN_RECOVERY_PRODUCTS: &[(&str, MacOsVersion)] = &[
    ("091-63921", MacOsVersion::HighSierra),
    ("2Z694-11046", MacOsVersion::HighSierra),
    ("041-94410", MacOsVersion::Mojave),
    ("001-43312", MacOsVersion::Catalina),
    ("001-51031", MacOsVersion::Catalina),
    ("071-71279", MacOsVersion::BigSur),
    ("071-78714", MacOsVersion::BigSur),
    ("012-51692", MacOsVersion::Monterey),
    ("012-40515", MacOsVersion::Monterey),
    ("042-01871", MacOsVersion::Ventura),
    ("042-23155", MacOsVersion::Ventura),
    ("062-53943", MacOsVersion::Sonoma),
    ("062-58679", MacOsVersion::Sonoma),
    ("082-33203", MacOsVersion::Sequoia),
    ("093-10615", MacOsVersion::Sequoia),
    ("140-93589", MacOsVersion::Tahoe),
];

/// The release a known recovery product belongs to (None for products
/// published after this table was made).
pub fn recovery_product_version(product: &str) -> Option<MacOsVersion> {
    KNOWN_RECOVERY_PRODUCTS.iter().find(|(p, _)| p.eq_ignore_ascii_case(product.trim())).map(|(_, v)| *v)
}

/// True when this release needs AVX2 (macOS 13+). Pre-Haswell CPUs need
/// CryptexFixup for these releases.
pub fn requires_avx2(version: MacOsVersion) -> bool {
    version >= MacOsVersion::Ventura
}

/// Human-readable, release-specific caveats shown in the version picker.
pub fn release_caveats(version: MacOsVersion) -> Vec<&'static str> {
    match version {
        MacOsVersion::HighSierra => vec![
            "Last release with NVIDIA Web Drivers: Maxwell and Pascal GeForce cards are accelerated only here (10.13.6).",
            "If the installer reports a damaged or expired copy, set an older date in the recovery Terminal: date 0901000019.",
        ],
        MacOsVersion::Mojave => vec![
            "No NVIDIA Web Drivers: Maxwell and Pascal GeForce cards run without acceleration.",
            "Last release that runs 32-bit apps.",
            "If the installer reports a damaged or expired copy, set an older date in the recovery Terminal: date 0901000019.",
        ],
        MacOsVersion::Catalina => vec![
            "32-bit apps no longer run.",
            "The system volume is read-only; system files live on a separate sealed volume.",
        ],
        MacOsVersion::BigSur => vec![
            "XhciPortLimit no longer works from 11.3: map the USB ports (at most 15 per controller) before installing.",
            "Last release with native NVIDIA Kepler and Intel HD 4000 (Ivy Bridge) graphics.",
        ],
        MacOsVersion::Monterey => vec![
            "NVIDIA Kepler and Ivy Bridge iGPUs lost native support (OpenCore Legacy Patcher root patch needed).",
            "Last release with Haswell, Broadwell and Skylake iGPU drivers.",
        ],
        MacOsVersion::Ventura => vec![
            "Requires AVX2 (Haswell or newer); older CPUs need CryptexFixup and lose delta updates.",
            "Skylake iGPUs must be spoofed as Kaby Lake; Haswell and Broadwell iGPUs need a root patch.",
            "Intel I225-V Ethernet uses a DriverKit driver that needs VT-d, or a replacement kext.",
        ],
        MacOsVersion::Sonoma => vec![
            "Broadcom BCM4360-family Wi-Fi (Fenvi T919, BCM94360CD) needs an OpenCore Legacy Patcher root patch.",
            "Requires AVX2 (Haswell or newer).",
        ],
        MacOsVersion::Sequoia => vec![
            "No official AirportItlwm build: Intel Wi-Fi works through itlwm with HeliPort.",
            "Broadcom BCM4360-family Wi-Fi still needs a root patch.",
            "Requires AVX2 (Haswell or newer).",
        ],
        MacOsVersion::Tahoe => vec![
            "Last macOS for Intel Macs.",
            "AppleHDA was removed: analog audio needs VoodooHDA or a post-install AppleHDA patch; HDMI/DP audio from AMD GPUs still works.",
            "No official AirportItlwm: Intel Wi-Fi works through itlwm with HeliPort.",
            "The pinned IntelBluetoothFirmware 2.5.1 fork supports Tahoe without -ibtcompatbeta (only upstream 2.4.0 needs the flag).",
            "IOUSBFamily was removed: many USB Wi-Fi dongles and older USB Bluetooth adapters stop working.",
            "FileVault volumes cannot be unlocked by Tahoe's APFS driver under OpenCore; leave FileVault off.",
            "Only MacPro7,1, iMac20,1, iMac20,2 and MacBookPro16,1/16,2/16,4 SMBIOS are supported (not MacBookPro16,3); USB maps need the new Tahoe port keys.",
            "Requires AVX2 (Haswell or newer).",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_ids_match_the_verified_table() {
        let expect = [
            (MacOsVersion::HighSierra, "Mac-7BA5B2D9E42DDD94", "00000000000J80300", "default"),
            (MacOsVersion::Mojave, "Mac-7BA5B2DFE22DDD8C", "00000000000KXPG00", "default"),
            (MacOsVersion::Catalina, "Mac-00BE6ED71E35EB86", MLB_ZERO, "default"),
            (MacOsVersion::BigSur, "Mac-2BD1B31983FE1663", MLB_ZERO, "default"),
            (MacOsVersion::Monterey, "Mac-E43C1C25D4880AD6", MLB_ZERO, "latest"),
            (MacOsVersion::Ventura, "Mac-B4831CEBD52A0C4C", MLB_ZERO, "latest"),
            (MacOsVersion::Sonoma, "Mac-827FAC58A8FDFA22", MLB_ZERO, "latest"),
            (MacOsVersion::Sequoia, "Mac-7BA5B2D9E42DDD94", MLB_ZERO, "latest"),
            (MacOsVersion::Tahoe, "Mac-CFF7D910A743CAAF", MLB_ZERO, "latest"),
        ];
        for (version, board, mlb, os) in expect {
            assert_eq!(recovery_request(version), RecoveryRequest { board_id: board, mlb, os_type: os }, "{version:?}");
        }
    }

    #[test]
    fn every_request_is_well_formed() {
        for version in MacOsVersion::ALL {
            let requests = recovery_requests(version);
            assert!(!requests.is_empty());
            for r in requests {
                assert!(r.board_id.starts_with("Mac-") && r.board_id.len() == 20, "{r:?}");
                assert_eq!(r.mlb.len(), 17, "{r:?}");
                assert!(r.os_type == "default" || r.os_type == "latest");
            }
        }
        // Tahoe is only served with os=latest.
        assert!(recovery_requests(MacOsVersion::Tahoe).iter().all(|r| r.os_type == "latest"));
    }

    #[test]
    fn known_products_map_to_releases() {
        assert_eq!(recovery_product_version("140-93589"), Some(MacOsVersion::Tahoe));
        assert_eq!(recovery_product_version("082-33203"), Some(MacOsVersion::Sequoia));
        assert_eq!(recovery_product_version("999-99999"), None);
    }

    #[test]
    fn caveats_exist_for_every_release() {
        for version in MacOsVersion::ALL {
            assert!(!release_caveats(version).is_empty(), "{version:?}");
        }
        let tahoe = release_caveats(MacOsVersion::Tahoe);
        assert!(tahoe.iter().any(|c| c.contains("AppleHDA")));
        // MacBookPro16,3 stops at Sequoia.
        assert!(tahoe.iter().any(|c| c.contains("MacBookPro16,1/16,2/16,4") && c.contains("not MacBookPro16,3")));
        assert!(requires_avx2(MacOsVersion::Ventura));
        assert!(!requires_avx2(MacOsVersion::Monterey));
    }
}
