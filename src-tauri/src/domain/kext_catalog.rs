//! Pinned, tested download manifest for OpenCore, OcBinaryData and every
//! kext the planner can select. Pinned entries download from fixed release
//! URLs (no GitHub API, no rate limit) and are verified by SHA-256. When
//! "use latest releases" is on, `latest_*` describes how to find a newer
//! asset through the GitHub API, falling back to the pin on any failure.
//!
//! Every hash below was computed from the downloaded file (or, for the
//! ChefKiss releases, taken from the `digest` GitHub publishes for the
//! release asset). Files that are not GitHub release assets (OCLP payloads,
//! Dortania, Legacy-Kexts and OpCore-Simplify mirrors) are pinned to a
//! commit, never to a branch, so the URL keeps serving the same bytes.
//!
//! Catalog ids are stable: the planner refers to them through
//! `KextSelection::catalog_id`. Variants of one project use a suffix
//! (`AirportItlwm-Ventura`, `RealtekRTL8111-2.4.2`).

use crate::domain::model::MacOsVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// Plain zip; the `.kext` bundles may sit at the root or in a
    /// subdirectory (`Kexts/`, `<Name>-V1.0/Release/`).
    Zip,
    /// GitHub Actions artifact zip containing an inner `*-RELEASE.zip`.
    NestedZip,
    /// The download is a single zip of one `.kext` folder.
    KextZip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub version: &'static str,
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the downloaded file; None only for moving
    /// nightly artifacts (NootRX), which are then validated structurally.
    pub sha256: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct KextCatalogEntry {
    /// Stable id used by `KextSelection::catalog_id` ("Lilu", "VirtualSMC").
    pub id: &'static str,
    /// GitHub "owner/repo" for latest-release resolution (empty if none).
    pub repo: &'static str,
    pub pin: Pin,
    pub archive: ArchiveKind,
    /// Regex matched against release asset names when resolving "latest"
    /// (must select RELEASE, never DEBUG/RESEARCH_RELEASE). Empty when the
    /// entry is pinned only.
    pub latest_asset_regex: &'static str,
    /// Top-level bundles the archive provides ("VirtualSMC.kext", "SMCProcessor.kext", ...).
    pub bundles: &'static [&'static str],
    pub description: &'static str,
}

impl KextCatalogEntry {
    /// True when "use latest releases" can look this entry up on GitHub.
    pub fn supports_latest(&self) -> bool {
        !self.repo.is_empty() && !self.latest_asset_regex.is_empty()
    }

    pub fn provides(&self, bundle: &str) -> bool {
        self.bundles.iter().any(|b| b.eq_ignore_ascii_case(bundle))
    }
}

macro_rules! gh_release {
    ($repo:literal, $tag:literal, $asset:literal) => {
        concat!("https://github.com/", $repo, "/releases/download/", $tag, "/", $asset)
    };
}

/// OpenCore-Legacy-Patcher payloads, pinned to commit e5af65f (2026-10-05).
macro_rules! oclp_payload {
    ($path:literal) => {
        concat!(
            "https://raw.githubusercontent.com/dortania/OpenCore-Legacy-Patcher/",
            "e5af65f6b2d4a33be84bd29ecaed23f2a1f94913/payloads/Kexts/",
            $path
        )
    };
}

/// Dortania OpenCore-Install-Guide `extra-files`, pinned to commit 6680f2d (2026-03-15).
macro_rules! dortania_extra {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/dortania/OpenCore-Install-Guide/",
            "6680f2df86de1283083b0208e8a8042fa1952b2c/extra-files/",
            $file
        )
    };
}

/// OpCore-Simplify's mirror of binaries that have no upstream release asset,
/// pinned to commit 819ac0d (2025-11-16).
macro_rules! ocs_mirror {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/lzhoang2801/lzhoang2801.github.io/",
            "819ac0d4b49dba10eaee5edbed0b6b53087133f8/public/extra-files/",
            $file
        )
    };
}

/// khronokernel/Legacy-Kexts (the copy Dortania's kext list links), pinned to
/// commit 4dfc274 (2020-10-14).
macro_rules! legacy_kexts {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/khronokernel/Legacy-Kexts/",
            "4dfc274111abdc94e94498d1e76d9354f3700fc9/",
            $file
        )
    };
}

const OPENCORE_VERSION: &str = "1.0.8";

const OPENCORE_RELEASE: Pin = Pin {
    version: OPENCORE_VERSION,
    url: gh_release!("acidanthera/OpenCorePkg", "1.0.8", "OpenCore-1.0.8-RELEASE.zip"),
    sha256: Some("2011e8b7216ecb2645d97ea710df965947edd406846e92050b9bc4190be6d27b"),
};

const OPENCORE_DEBUG: Pin = Pin {
    version: OPENCORE_VERSION,
    url: gh_release!("acidanthera/OpenCorePkg", "1.0.8", "OpenCore-1.0.8-DEBUG.zip"),
    sha256: Some("3f5f42b85703cf2c08e8dcd9245a52fb4326c9f4ef335c75b49f516cbbe131c6"),
};

/// GitHub-generated archive of acidanthera/OcBinaryData at commit 32500d5
/// ("Image: Add OpticalDrive.icns", 2026-09-11, the icon set OpenCore 1.0.8
/// expects). The archive's root folder is `OcBinaryData-<commit>/`.
const OCBINARYDATA: Pin = Pin {
    version: "32500d5e3313f11cad00351ba1c6f9a4aa6cb0f7",
    url: "https://github.com/acidanthera/OcBinaryData/archive/32500d5e3313f11cad00351ba1c6f9a4aa6cb0f7.zip",
    sha256: Some("1ced15b42352a3a6c1b76ca95951f337fa79899acfb4ed80f329191716623741"),
};

/// "OpenCorePkg" repository used for latest-release lookups.
pub const OPENCORE_REPO: &str = "acidanthera/OpenCorePkg";

static CATALOG: &[KextCatalogEntry] = &[
    // ── Core, sensors, graphics, audio ──────────────────────────────────────
    KextCatalogEntry {
        id: "Lilu",
        repo: "acidanthera/Lilu",
        pin: Pin {
            version: "1.7.2",
            url: gh_release!("acidanthera/Lilu", "1.7.2", "Lilu-1.7.2-RELEASE.zip"),
            sha256: Some("53967d7dcfaab01023a33df2e969a89522f13d6654a6a56ac4711b62dabf3ab8"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^Lilu-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["Lilu.kext"],
        description: "Kernel patching engine required by every Lilu plugin; always loads first.",
    },
    KextCatalogEntry {
        id: "VirtualSMC",
        repo: "acidanthera/VirtualSMC",
        pin: Pin {
            version: "1.3.8",
            url: gh_release!("acidanthera/VirtualSMC", "1.3.8", "VirtualSMC-1.3.8-RELEASE.zip"),
            sha256: Some("2e29a8aaf91b4eb0bcdf5913ed17e70f5cf76531122acd4132eff7813a54de8b"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^VirtualSMC-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &[
            "VirtualSMC.kext",
            "SMCProcessor.kext",
            "SMCSuperIO.kext",
            "SMCBatteryManager.kext",
            "SMCLightSensor.kext",
            "SMCDellSensors.kext",
        ],
        description: "SMC emulator plus sensor plugins (Intel CPU, Super I/O fans, battery, ambient light, Dell SMM).",
    },
    KextCatalogEntry {
        id: "WhateverGreen",
        repo: "acidanthera/WhateverGreen",
        pin: Pin {
            version: "1.7.1",
            url: gh_release!("acidanthera/WhateverGreen", "1.7.1", "WhateverGreen-1.7.1-RELEASE.zip"),
            sha256: Some("1bfdd2d40290ecce237606db121cbc01d7041179f03394bd59660a2ed52e5c95"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^WhateverGreen-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["WhateverGreen.kext"],
        description: "Graphics patches for Intel iGPU, AMD and NVIDIA GPUs.",
    },
    KextCatalogEntry {
        id: "AppleALC",
        repo: "acidanthera/AppleALC",
        pin: Pin {
            version: "1.9.8",
            url: gh_release!("acidanthera/AppleALC", "1.9.8", "AppleALC-1.9.8-RELEASE.zip"),
            sha256: Some("d4d36cd2da7863cf7cbfcb0892d45c08457f4f4ddd065a073dd010e46bd63bcb"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^AppleALC-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["AppleALC.kext", "AppleALCU.kext"],
        description: "HDA codec enabler for AppleHDA (AppleALCU: digital audio only).",
    },
    KextCatalogEntry {
        id: "NootedRed",
        repo: "ChefKissInc/NootedRed",
        pin: Pin {
            version: "0.8.10",
            url: gh_release!("ChefKissInc/NootedRed", "v0.8.10", "NootedRed-0.8.10-RELEASE.zip"),
            sha256: Some("28767001f2bbe12d648313e0e3b0c5adb1ac2c2d1f844e4212ee2f88d486da66"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^NootedRed-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["NootedRed.kext"],
        description: "AMD Vega iGPUs of Ryzen 1000-5000 and 7x30 APUs (Raven, Picasso, Renoir, Lucienne, Cezanne, Barcelo). Downloaded from the official release at build time.",
    },
    KextCatalogEntry {
        id: "NootRX",
        repo: "",
        pin: Pin {
            version: "1.0.0",
            url: "https://nightly.link/ChefKissInc/NootRX/workflows/main/master/Artifacts.zip",
            sha256: None,
        },
        archive: ArchiveKind::NestedZip,
        latest_asset_regex: "",
        bundles: &["NootRX.kext"],
        description: "AMD RDNA2 (Navi 21/22/23) dGPU support. Only published as a CI artifact, so it is checked structurally instead of by hash.",
    },
    KextCatalogEntry {
        id: "SMCRadeonSensors",
        repo: "ChefKissInc/SMCRadeonSensors",
        pin: Pin {
            version: "2.4.0",
            url: gh_release!("ChefKissInc/SMCRadeonSensors", "2.4.0", "SMCRadeonSensors-2.4.0-RELEASE.zip"),
            sha256: Some("9a6fd44185b2f51f0c91981c28dd4372a53fef64164dd1e4790c42cf39a97da0"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^SMCRadeonSensors-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["SMCRadeonSensors.kext"],
        description: "AMD GPU temperature sensors through VirtualSMC (macOS 10.14+).",
    },
    // ── Lilu utilities, CPU, misc ───────────────────────────────────────────
    KextCatalogEntry {
        id: "RestrictEvents",
        repo: "acidanthera/RestrictEvents",
        pin: Pin {
            version: "1.1.6",
            url: gh_release!("acidanthera/RestrictEvents", "1.1.6", "RestrictEvents-1.1.6-RELEASE.zip"),
            sha256: Some("98170dfae195ddd28b5d95e3f040125a13ca783bcb9bd1e5b8c588e217b14ee6"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^RestrictEvents-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["RestrictEvents.kext"],
        description: "Blocks unwanted processes, fixes memory/PCI tabs and CPU name, enables OTA updates on 14.4+ (revpatch=sbvmm).",
    },
    KextCatalogEntry {
        id: "CryptexFixup",
        repo: "acidanthera/CryptexFixup",
        pin: Pin {
            version: "1.0.5",
            url: gh_release!("acidanthera/CryptexFixup", "1.0.5", "CryptexFixup-1.0.5-RELEASE.zip"),
            sha256: Some("25041d94a0fe9a0261caf0ba89b36dfcb21682bf3c697a34bcaddc839576ab30"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^CryptexFixup-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["CryptexFixup.kext"],
        description: "Installs the non-AVX2 Rosetta cryptex on macOS 13+ for CPUs without AVX2.",
    },
    KextCatalogEntry {
        id: "telemetrap",
        repo: "",
        pin: Pin {
            version: "1.0.0",
            url: oclp_payload!("SSE/telemetrap-v1.0.0.zip"),
            sha256: Some("609c068866aeb1953c67e29c65ca7d2925dbda593e5fe791e18c5c2e3fa1028a"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["telemetrap.kext"],
        description: "Keeps the SSE4.2-only telemetry plugin from loading, so SSE4.1 CPUs (Penryn) boot macOS 10.14+ (OCLP payload).",
    },
    KextCatalogEntry {
        id: "AppleIntelCPUPowerManagement",
        repo: "",
        pin: Pin {
            version: "1.0.0",
            url: oclp_payload!("Misc/AppleIntelCPUPowerManagement-v1.0.0.zip"),
            sha256: Some("8adeb0f3002387bb18d78c50100fa79f4939277b654e43d8f1178f0831e4f332"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["AppleIntelCPUPowerManagement.kext"],
        description: "Apple's pre-XCPM CPU power management, removed in macOS 13; re-injected for Sandy/Ivy Bridge and older (OCLP payload).",
    },
    KextCatalogEntry {
        id: "AppleIntelCPUPowerManagementClient",
        repo: "",
        pin: Pin {
            version: "1.0.0",
            url: oclp_payload!("Misc/AppleIntelCPUPowerManagementClient-v1.0.0.zip"),
            sha256: Some("8d90f67dc7a94b61e80a0a3448248fa4246eb7271ecd049debfb3d90b6fdfd01"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["AppleIntelCPUPowerManagementClient.kext"],
        description: "Helper of AppleIntelCPUPowerManagement for macOS 13+ (OCLP payload).",
    },
    KextCatalogEntry {
        id: "FeatureUnlock",
        repo: "acidanthera/FeatureUnlock",
        pin: Pin {
            version: "1.1.8",
            url: gh_release!("acidanthera/FeatureUnlock", "1.1.8", "FeatureUnlock-1.1.8-RELEASE.zip"),
            sha256: Some("b1b85c31fe48fc899ac838b013c9b64a842f6f33265200b5ace3ecec5caa045c"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^FeatureUnlock-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["FeatureUnlock.kext"],
        description: "Unlocks Sidecar, AirPlay to Mac, Night Shift and Continuity Camera on unsupported models.",
    },
    KextCatalogEntry {
        id: "NVMeFix",
        repo: "acidanthera/NVMeFix",
        pin: Pin {
            version: "1.1.3",
            url: gh_release!("acidanthera/NVMeFix", "1.1.3", "NVMeFix-1.1.3-RELEASE.zip"),
            sha256: Some("e1d5657ab7ac31f69771708f7b80bf218ab9aa0b8e4c4fe6ff943983037e3dfb"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^NVMeFix-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["NVMeFix.kext"],
        description: "Power management and compatibility fixes for non-Apple NVMe drives (10.14+).",
    },
    KextCatalogEntry {
        id: "HibernationFixup",
        repo: "acidanthera/HibernationFixup",
        pin: Pin {
            version: "1.5.4",
            url: gh_release!("acidanthera/HibernationFixup", "1.5.4", "HibernationFixup-1.5.4-RELEASE.zip"),
            sha256: Some("89f50c5118e664d981d36b8f2b61e02c91ee6f33a7dfb089be890a4231e3b8ae"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^HibernationFixup-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["HibernationFixup.kext"],
        description: "Hibernation fixes for machines without native NVRAM hibernation support.",
    },
    KextCatalogEntry {
        id: "RTCMemoryFixup",
        repo: "acidanthera/RTCMemoryFixup",
        pin: Pin {
            version: "1.0.7",
            url: gh_release!("acidanthera/RTCMemoryFixup", "1.0.7", "RTCMemoryFixup-1.0.7-RELEASE.zip"),
            sha256: Some("defcf370970ec8cdce59c607d9ec95b10f730012c3fce05076eae9dfb387e94a"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^RTCMemoryFixup-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["RTCMemoryFixup.kext"],
        description: "Emulates CMOS regions that firmware breaks when macOS writes them (rtcfx_exclude=).",
    },
    KextCatalogEntry {
        id: "CPUFriend",
        repo: "acidanthera/CPUFriend",
        pin: Pin {
            version: "1.3.0",
            url: gh_release!("acidanthera/CPUFriend", "1.3.0", "CPUFriend-1.3.0-RELEASE.zip"),
            sha256: Some("37645d960f0b3c958cfd0a8a041160532267ec535c4979897123df89c7dbdcde"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^CPUFriend-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["CPUFriend.kext"],
        description: "Injects a custom X86PlatformPlugin frequency-vector table (needs a CPUFriendDataProvider).",
    },
    KextCatalogEntry {
        id: "CpuTopologyRebuild",
        repo: "b00t0x/CpuTopologyRebuild",
        pin: Pin {
            version: "2.0.2",
            url: gh_release!("b00t0x/CpuTopologyRebuild", "2.0.2", "CpuTopologyRebuild-2.0.2-RELEASE.zip"),
            sha256: Some("c25d07464d2c9adf120dbf13a50321ff1bdb273290a5e82235eef4740db7b911"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^CpuTopologyRebuild-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["CpuTopologyRebuild.kext"],
        description: "Rebuilds the P-core/E-core topology of Alder Lake and newer hybrid CPUs.",
    },
    KextCatalogEntry {
        id: "AMDRyzenCPUPowerManagement",
        repo: "trulyspinach/SMCAMDProcessor",
        pin: Pin {
            version: "0.7.2f1",
            url: gh_release!("trulyspinach/SMCAMDProcessor", "0.7.2f1", "AMDRyzenCPUPowerManagement.kext.zip"),
            sha256: Some("844cd318a3f137d46a10e204e1367dbc9593716120289f5261dc2d48110869b4"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AMDRyzenCPUPowerManagement\.kext\.zip$",
        bundles: &["AMDRyzenCPUPowerManagement.kext"],
        description: "AMD Zen CPU power management and frequency reporting.",
    },
    KextCatalogEntry {
        id: "SMCAMDProcessor",
        repo: "trulyspinach/SMCAMDProcessor",
        pin: Pin {
            version: "0.7.2f1",
            url: gh_release!("trulyspinach/SMCAMDProcessor", "0.7.2f1", "SMCAMDProcessor.kext.zip"),
            sha256: Some("2b767df8e145c60bc581322749651026f4e85008b4e3deafe4e620e6eadbfcb5"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^SMCAMDProcessor\.kext\.zip$",
        bundles: &["SMCAMDProcessor.kext"],
        description: "AMD Zen CPU temperature sensors through VirtualSMC (requires AMDRyzenCPUPowerManagement).",
    },
    KextCatalogEntry {
        id: "AppleMCEReporterDisabler",
        repo: "",
        pin: Pin {
            version: "1.2",
            url: "https://github.com/acidanthera/bugtracker/files/3703498/AppleMCEReporterDisabler.kext.zip",
            sha256: Some("470417c4958dd6ecb982182140a6b76cf85f847c15f1b0f6f59617d5abcc5f76"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["AppleMCEReporterDisabler.kext"],
        description: "Codeless kext that keeps AppleIntelMCEReporter from loading (AMD 12.3+, dual-socket Intel) with MacPro6,1/7,1 or iMacPro1,1.",
    },
    KextCatalogEntry {
        id: "CpuTscSync",
        repo: "acidanthera/CpuTscSync",
        pin: Pin {
            version: "1.1.2",
            url: gh_release!("acidanthera/CpuTscSync", "1.1.2", "CpuTscSync-1.1.2-RELEASE.zip"),
            sha256: Some("bc289f780c52015ae788827b3db8e6c2ab9f512992b3a157a4a5c15df1eb4ed3"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^CpuTscSync-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["CpuTscSync.kext"],
        description: "TSC synchronisation for Intel CPUs (HEDT/server boards with unsynced TSC). Not for AMD.",
    },
    KextCatalogEntry {
        id: "ForgedInvariant",
        repo: "ChefKissInc/ForgedInvariant",
        pin: Pin {
            version: "1.5.0",
            url: gh_release!("ChefKissInc/ForgedInvariant", "v1.5.0", "ForgedInvariant-1.5.0-RELEASE.zip"),
            sha256: Some("e684d3bf4c8a10e2b5e1d42ac5c9760e26ed2add8e40e319f3951ea8d41b04c7"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^ForgedInvariant-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["ForgedInvariant.kext"],
        description: "TSC synchronisation for Intel and AMD CPUs. Downloaded from the official release at build time.",
    },
    KextCatalogEntry {
        id: "AmdTscSync",
        repo: "naveenkrdy/AmdTscSync",
        pin: Pin {
            version: "2.0.0",
            url: gh_release!("naveenkrdy/AmdTscSync", "2.0.0", "AmdTscSync-2.0.0-RELEASE.zip"),
            sha256: Some("aed2dffed57b2ea21e5e323a423652e00868d4831558113fd8551f536e3f3133"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^AmdTscSync-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["AmdTscSync.kext"],
        description: "TSC synchronisation for AMD CPUs (no Lilu dependency).",
    },
    KextCatalogEntry {
        id: "VoodooTSCSync",
        repo: "CloverHackyColor/VoodooTSCSync",
        pin: Pin {
            version: "1.1",
            url: gh_release!("CloverHackyColor/VoodooTSCSync", "2.0", "VoodooTSCSync.kext.zip"),
            sha256: Some("4e34a7247f96958b4260b0a6aadb8477ec14c83c78a824af9c424f613df9d455"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^VoodooTSCSync\.kext\.zip$",
        bundles: &["VoodooTSCSync.kext"],
        description: "Legacy TSC synchronisation without Lilu; the IOCPUNumber match in its Info.plist must be adjusted to the CPU.",
    },
    KextCatalogEntry {
        id: "TSCAdjustReset",
        repo: "",
        pin: Pin {
            version: "1.1",
            url: dortania_extra!("TSCAdjustReset.kext.zip"),
            sha256: Some("119b7a050d16f9c69442b66bafc7a1a87cab6caa73ad78e36d740082cc7f8b4e"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["TSCAdjustReset.kext"],
        description: "TSC reset for Skylake-X/Cascade Lake-X (Dortania build); the IOCPUNumber match in its Info.plist must be adjusted to the CPU.",
    },
    KextCatalogEntry {
        id: "AMFIPass",
        repo: "",
        pin: Pin {
            version: "1.4.1",
            url: oclp_payload!("Acidanthera/AMFIPass-v1.4.1-RELEASE.zip"),
            sha256: Some("07b266145906db41f4b13a7938fbb173ea28888cc1fa65f84417f8820adc961e"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["AMFIPass.kext"],
        description: "Keeps AMFI enabled on root-patched systems (OCLP payload; needed with legacy Wi-Fi stacks).",
    },
    KextCatalogEntry {
        id: "iBridged",
        repo: "Carnations-Botanica/iBridged",
        pin: Pin {
            version: "1.0.1",
            url: gh_release!("Carnations-Botanica/iBridged", "1.0.1", "iBridged-1.0.1-RELEASE.zip"),
            sha256: Some("9ad38681c1c59cfb8362bda82082dc257f2c8c3e1a37504021992174fd763e47"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^iBridged-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["iBridged.kext"],
        description: "Emulates the T2 bridge so OTA updates work on 14.4+ while keeping SecureBootModel.",
    },
    KextCatalogEntry {
        id: "ECEnabler",
        repo: "averycblack/ECEnabler",
        pin: Pin {
            version: "1.0.6",
            url: gh_release!("averycblack/ECEnabler", "1.0.6", "ECEnabler-1.0.6-RELEASE.zip"),
            sha256: Some("e21149519856308c6c4c06e0917aef9e3423ff3e23b6ea77989099f4f2cf0f0a"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^ECEnabler-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["ECEnabler.kext"],
        description: "Allows reading EC fields wider than 8 bits (laptop battery status without DSDT patches).",
    },
    KextCatalogEntry {
        id: "BrightnessKeys",
        repo: "acidanthera/BrightnessKeys",
        pin: Pin {
            version: "1.0.3",
            url: gh_release!("acidanthera/BrightnessKeys", "1.0.3", "BrightnessKeys-1.0.3-RELEASE.zip"),
            sha256: Some("c9a80f6275e39c18886087a70ba0fe615ce45f83b7afa254b74335f7a92b8544"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^BrightnessKeys-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["BrightnessKeys.kext"],
        description: "Fn brightness keys on laptops without DSDT patches.",
    },
    // ── Ethernet ────────────────────────────────────────────────────────────
    KextCatalogEntry {
        id: "IntelMausi",
        repo: "acidanthera/IntelMausi",
        pin: Pin {
            version: "1.0.8",
            url: gh_release!("acidanthera/IntelMausi", "1.0.8", "IntelMausi-1.0.8-RELEASE.zip"),
            sha256: Some("cc02ea7e972ead536a51e5f6725e79f121026dbf36a766e97044ef2483fd29bc"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^IntelMausi-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["IntelMausi.kext", "IntelSnowMausi.kext"],
        description: "Intel 82578-I219 Ethernet (acidanthera build).",
    },
    KextCatalogEntry {
        id: "IntelMausiEthernet",
        repo: "Mieze/IntelMausiEthernet",
        pin: Pin {
            version: "3.0.0",
            url: gh_release!("Mieze/IntelMausiEthernet", "v3.0.0", "IntelMausiEthernet-V3.0.0.zip"),
            sha256: Some("f3ef361ad3e6697f975eb69002f9843144772e5b72361c799e2d9ef0cd4c1737"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^IntelMausiEthernet-V[0-9][0-9.]*\.zip$",
        bundles: &["IntelMausiEthernet.kext"],
        description: "Intel I219 Ethernet (Mieze build) with AppleVTD support, for 500/600-series boards.",
    },
    KextCatalogEntry {
        id: "AppleIGB",
        repo: "",
        pin: Pin {
            version: "5.11.4",
            url: gh_release!("donatengit/AppleIGB", "v5.11-mb", "AppleIGB.DEBUG.kext.zip"),
            sha256: Some("8aa7a158e618c0047a45a6db5fb5e0ea7e1f2b7d1f7757e43df4a987cbf460f6"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["AppleIGB.kext"],
        description: "Intel I211/I350 Ethernet on macOS 12+. Upstream only publishes this prerelease build with debug logging (deprecated by its maintainer); it is used instead of third-party rebuilds.",
    },
    KextCatalogEntry {
        id: "SmallTreeIntel82576",
        repo: "khronokernel/SmallTree-I211-AT-patch",
        pin: Pin {
            version: "1.3.0",
            url: gh_release!("khronokernel/SmallTree-I211-AT-patch", "1.3.0", "SmallTreeIntel82576.kext.zip"),
            sha256: Some("5fcced4fcd1e9f1b95012aca7cfd117f4eae13b525b215e740c851fda262fa6d"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^SmallTreeIntel82576\.kext\.zip$",
        bundles: &["SmallTreeIntel82576.kext"],
        description: "Intel I211 Ethernet for macOS 10.15-11 (does not load on 12+).",
    },
    KextCatalogEntry {
        id: "SmallTreeIntel82576-1.2.5",
        repo: "",
        pin: Pin {
            version: "1.2.5",
            url: gh_release!("khronokernel/SmallTree-I211-AT-patch", "1.2.5", "SmallTree-I211-AT-patch.kext.zip"),
            sha256: Some("5ce752a230fb131286a20e3103ef0b0b9e50e05e0ed3fbea0172b67ef0cb84e0"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["SmallTreeIntel82576.kext"],
        description: "Intel I211 Ethernet for macOS 10.13-10.14 (the release notes limit this build to those two).",
    },
    KextCatalogEntry {
        id: "AppleIGC",
        repo: "SongXiaoXi/AppleIGC",
        pin: Pin {
            version: "1.9",
            url: gh_release!("SongXiaoXi/AppleIGC", "v1.9", "AppleIGC.kext.zip"),
            sha256: Some("3398c5cf609b26be9485445c4f10fe2ef2c43554db1c25f406d8bc955c399466"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AppleIGC\.kext\.zip$",
        bundles: &["AppleIGC.kext"],
        description: "Intel I225/I226 2.5GbE without VT-d.",
    },
    KextCatalogEntry {
        id: "AppleIntelI210Ethernet",
        repo: "",
        pin: Pin {
            version: "2.3.1",
            url: dortania_extra!("AppleIntelI210Ethernet.kext.zip"),
            sha256: Some("0bece103ca39c08239de090e0f969602ff260dc586661389be8aab70a1b0bee8"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["AppleIntelI210Ethernet.kext"],
        description: "Apple's I210 driver (Dortania copy) for I225-V with a device-id spoof on macOS 13+ (needs e1000=0).",
    },
    KextCatalogEntry {
        id: "IntelLucy",
        repo: "Mieze/IntelLucy",
        pin: Pin {
            version: "1.1.6",
            url: gh_release!("Mieze/IntelLucy", "v.1.1.6", "IntelLucy-V1.1.6.zip"),
            sha256: Some("c3ea7fdecc04be2f1f33124526a16b0247b783d4a15013b2d157b0c02cef5de2"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^IntelLucy-V[0-9][0-9.]*\.zip$",
        bundles: &["IntelLucy.kext"],
        description: "Intel X520/X540/X550/82598 10GbE (not for I225/I226).",
    },
    KextCatalogEntry {
        id: "AtherosE2200Ethernet",
        repo: "Mieze/AtherosE2200Ethernet",
        pin: Pin {
            version: "2.4.0",
            url: gh_release!("Mieze/AtherosE2200Ethernet", "v2.4.0", "AtherosE2200Ethernet-V2.4.0.zip"),
            sha256: Some("4702edfcda5cc1b9c7e2ec1278130bb02c5c5a42c77f39b525fa5ac1ca900bfb"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^AtherosE2200Ethernet-V[0-9][0-9.]*\.zip$",
        bundles: &["AtherosE2200Ethernet.kext"],
        description: "Qualcomm Atheros AR816x/AR817x and Killer E220x/E2400/E2500 Ethernet.",
    },
    KextCatalogEntry {
        id: "RealtekRTL8111",
        repo: "Mieze/RTL8111_driver_for_OS_X",
        pin: Pin {
            version: "3.0.0",
            url: gh_release!("Mieze/RTL8111_driver_for_OS_X", "v3.0.0", "RealtekRTL8111-V3.0.0.zip"),
            sha256: Some("a0f2e64ac3c76e2d416ff88f35a197ce229e74ea78e968631a736a43b4d8231c"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^RealtekRTL8111-V[0-9][0-9.]*\.zip$",
        bundles: &["RealtekRTL8111.kext"],
        description: "Realtek RTL8111/8168 Gigabit Ethernet with AppleVTD support (Intel platforms).",
    },
    KextCatalogEntry {
        id: "RealtekRTL8111-2.4.2",
        repo: "",
        pin: Pin {
            version: "2.4.2",
            url: gh_release!("Mieze/RTL8111_driver_for_OS_X", "2.4.2", "RealtekRTL8111-V2.4.2.zip"),
            sha256: Some("fa2a6a8435b4f7211fb503e30a2966017cf050c09e839a13e79c4a8a9cd2a8ba"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["RealtekRTL8111.kext"],
        description: "Realtek RTL8111/8168 without AppleVTD; the version the author recommends for AMD systems.",
    },
    KextCatalogEntry {
        id: "RealtekRTL8111-2.2.2",
        repo: "",
        pin: Pin {
            version: "2.2.2",
            url: gh_release!("Mieze/RTL8111_driver_for_OS_X", "v2.2.2", "RealtekRTL8111-V2.2.2.zip"),
            sha256: Some("23542dc0b6e0fae1f2b599de2ee90f2c082f55ad6b93531b05049ad8e231ecb1"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["RealtekRTL8111.kext"],
        description: "Realtek RTL8111/8168 for macOS 10.13 (2.3.0 and newer need 10.14; the archive's Release build is used).",
    },
    KextCatalogEntry {
        id: "LucyRTL8125Ethernet",
        repo: "Mieze/LucyRTL8125Ethernet",
        pin: Pin {
            version: "1.2.3",
            url: gh_release!("Mieze/LucyRTL8125Ethernet", "v.1.2.3", "LucyRTL8125Ethernet-1.2.300.zip"),
            sha256: Some("89ef03362baadba546418ade9614edc0469ae2483ea5380c0dcfb04941201521"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^LucyRTL8125Ethernet-[0-9][0-9.]*\.zip$",
        bundles: &["LucyRTL8125Ethernet.kext"],
        description: "Realtek RTL8125 2.5GbE (fallback for RTL812xLucy).",
    },
    KextCatalogEntry {
        id: "RTL812xLucy",
        repo: "Mieze/RTL812xLucy",
        pin: Pin {
            version: "1.1.1",
            url: gh_release!("Mieze/RTL812xLucy", "v1.1.1", "RTL812xLucy-V1.1.1.zip"),
            sha256: Some("7a50f9b8c776405ea537bf1c42bc7e062446fba8442aa1cfa7f2a60a966b295e"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^RTL812xLucy-V[0-9][0-9.]*\.zip$",
        bundles: &["RTL812xLucy.kext"],
        description: "Realtek RTL8125A/B/BP/CP/D and RTL8126A 2.5/5GbE with AppleVTD support.",
    },
    KextCatalogEntry {
        id: "RealtekRTL8100",
        repo: "",
        pin: Pin {
            version: "2.0.1",
            url: ocs_mirror!("RealtekRTL8100-v2.0.1.zip"),
            sha256: Some("7fa6624e771ae08fd60db52338b2374226d983e43433c7ec4d8e48375f96aeb8"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["RealtekRTL8100.kext"],
        description: "Realtek RTL8100/8101 Fast Ethernet. The upstream repository is archived without release assets, so a pinned mirror is used.",
    },
    // ── Wi-Fi ───────────────────────────────────────────────────────────────
    KextCatalogEntry {
        id: "itlwm",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "itlwm_v2.3.0_stable.kext.zip"),
            sha256: Some("31ed2e5b4bca92645bfab0a397a1f50212c72a04ea15591c061fe74233e4f82b"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^itlwm_v[0-9][0-9.]*_stable\.kext\.zip$",
        bundles: &["itlwm.kext"],
        description: "Intel Wi-Fi as an Ethernet-like interface for every macOS release (needs the HeliPort app).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-HighSierra",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_HighSierra.kext.zip"),
            sha256: Some("2d30b4ae53a99716f532af81f5a5d54ad5259e9f11933607bb8e6379289cafcd"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_HighSierra\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 10.13 (Darwin 17).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-Mojave",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_Mojave.kext.zip"),
            sha256: Some("7abdcfce6052665173635bc895d51eebebc21093c32c7d0099725c7c60497634"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_Mojave\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 10.14 (Darwin 18).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-Catalina",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_Catalina.kext.zip"),
            sha256: Some("68ce60fa7c1f625ba3d07e8ad1b36798e6fffe076f7d0cd021db39b09367d7fb"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_Catalina\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 10.15 (Darwin 19).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-BigSur",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_BigSur.kext.zip"),
            sha256: Some("0d89fe4f515023a81cea16bbc3540d2ceb2d0dc312cc6f1050038e47fea31fdc"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_BigSur\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 11 (Darwin 20).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-Monterey",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_Monterey.kext.zip"),
            sha256: Some("da4a7468bb92fbe873513e0cd0d037d367f65051339d73b726d4c40b6167f923"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_Monterey\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 12 (Darwin 21).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-Ventura",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_Ventura.kext.zip"),
            sha256: Some("7ea19307ef9ae08991d911e6f1de59e682444e21dbcd6342b5e0a037f19c860e"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_Ventura\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 13 (Darwin 22); also the build used on 15/26 with the legacy wireless stack.",
    },
    KextCatalogEntry {
        id: "AirportItlwm-Sonoma14.0",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_Sonoma14.0.kext.zip"),
            sha256: Some("d795fc5844e9126f4327c4be5ceae9297ebf8a0977377a28c4a24346643609bb"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_Sonoma14\.0\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 14.0-14.3 (Darwin 23.0-23.3).",
    },
    KextCatalogEntry {
        id: "AirportItlwm-Sonoma14.4",
        repo: "OpenIntelWireless/itlwm",
        pin: Pin {
            version: "2.3.0",
            url: gh_release!("OpenIntelWireless/itlwm", "v2.3.0", "AirportItlwm_v2.3.0_stable_Sonoma14.4.kext.zip"),
            sha256: Some("7ef12a8037eb9bc795828c348f4fa0728b8ade57aa0984b037816ef91c2d5119"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^AirportItlwm_v[0-9][0-9.]*_stable_Sonoma14\.4\.kext\.zip$",
        bundles: &["AirportItlwm.kext"],
        description: "Native Intel Wi-Fi for macOS 14.4+ (Darwin 23.4+).",
    },
    KextCatalogEntry {
        id: "AirportBrcmFixup",
        repo: "acidanthera/AirportBrcmFixup",
        pin: Pin {
            version: "2.2.1",
            url: gh_release!("acidanthera/AirportBrcmFixup", "2.2.1", "AirportBrcmFixup-2.2.1-RELEASE.zip"),
            sha256: Some("2432c8a62445ad2cdadcc59b73928f01f8bf42bcd3f4386e49bc63a1c91db06f"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^AirportBrcmFixup-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["AirportBrcmFixup.kext"],
        description: "Broadcom Wi-Fi fixes; plugins AirPortBrcm4360_Injector.kext (<=10.15) and AirPortBrcmNIC_Injector.kext (11+).",
    },
    KextCatalogEntry {
        id: "IOSkywalkFamily",
        repo: "",
        pin: Pin {
            version: "1.2.0",
            url: oclp_payload!("Wifi/IOSkywalkFamily-v1.2.0.zip"),
            sha256: Some("1e12b7ef42f55b39ea54ada97b46331220668b2c48a28656e9875c5145fe2479"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["IOSkywalkFamily.kext"],
        description: "Older IOSkywalkFamily for the legacy wireless stack on 14+ (OCLP payload; requires blocking the system copy).",
    },
    KextCatalogEntry {
        id: "IO80211FamilyLegacy",
        repo: "",
        pin: Pin {
            version: "1.0.0",
            url: oclp_payload!("Wifi/IO80211FamilyLegacy-v1.0.0.zip"),
            sha256: Some("e681dcc76a2cd2cea4b0ad5f27a3c816055fde3cdccd890dd10a3e2c84e96d93"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["IO80211FamilyLegacy.kext"],
        description: "Legacy IO80211 family for 14+ (OCLP payload); plugin AirPortBrcmNIC.kext for Broadcom on 14-15.",
    },
    KextCatalogEntry {
        id: "AirPortBrcmNIC-Tahoe",
        repo: "",
        pin: Pin {
            version: "1.0.0",
            url: oclp_payload!("Wifi/AirPortBrcmNIC-Tahoe-v1.0.0.zip"),
            sha256: Some("04edd7127ce096f5de9f8112af6d06744f27e5bc659e3d6516408755cb9af032"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["AirPortBrcmNIC-Tahoe.kext"],
        description: "Broadcom Wi-Fi driver for macOS 26 on the legacy wireless stack (OCLP payload).",
    },
    KextCatalogEntry {
        id: "Feixiao",
        repo: "thegwchr/Feixiao",
        pin: Pin {
            version: "1.0.1",
            url: gh_release!(
                "thegwchr/Feixiao",
                "v1.0.1",
                "Feixiao_v1.0.1_FBFEB3FE-3540-398D-9A72-D54F7BAB548C.zip"
            ),
            sha256: Some("3c18571aa0038487ba713574db0afccaf310763e0be46e9f9aafa00e2f36d982"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^Feixiao_v[0-9][0-9.]*_[0-9A-Fa-f-]+\.zip$",
        bundles: &["rtw88.kext"],
        description: "Experimental Realtek RTL8822BE/8822CE/8821CE PCIe Wi-Fi (Ethernet-like interface, 11+).",
    },
    // ── Bluetooth ───────────────────────────────────────────────────────────
    KextCatalogEntry {
        id: "BrcmPatchRAM",
        repo: "acidanthera/BrcmPatchRAM",
        pin: Pin {
            version: "2.7.2",
            url: gh_release!("acidanthera/BrcmPatchRAM", "2.7.2", "BrcmPatchRAM-2.7.2-RELEASE.zip"),
            sha256: Some("e1c1c55347526d031a8ae2fdd1f52efa3019161e497fb38e1cfa809752f8af21"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^BrcmPatchRAM-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &[
            "BlueToolFixup.kext",
            "BrcmBluetoothInjector.kext",
            "BrcmBluetoothInjectorLegacy.kext",
            "BrcmFirmwareData.kext",
            "BrcmFirmwareRepo.kext",
            "BrcmNonPatchRAM.kext",
            "BrcmNonPatchRAM2.kext",
            "BrcmPatchRAM.kext",
            "BrcmPatchRAM2.kext",
            "BrcmPatchRAM3.kext",
        ],
        description: "Broadcom Bluetooth firmware upload (BrcmPatchRAM3 10.15+, BrcmPatchRAM2 10.11-10.14) and BlueToolFixup (12+).",
    },
    KextCatalogEntry {
        id: "IntelBluetoothFirmware",
        repo: "lshbluesky/IntelBluetoothFirmware",
        pin: Pin {
            version: "2.5.1",
            url: gh_release!("lshbluesky/IntelBluetoothFirmware", "v2.5.1", "IntelBluetooth-2.5.1-RELEASE.zip"),
            sha256: Some("0e58a31993657ee2227fd548dff6d0cdf38bc9e4cb1495816e148c8d6ab43e5c"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^IntelBluetooth-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["IntelBluetoothFirmware.kext", "IntelBTPatcher.kext", "IntelBluetoothInjector.kext"],
        description: "Intel Bluetooth firmware upload; maintained fork with macOS 26 and BE200 support.",
    },
    KextCatalogEntry {
        id: "IntelBluetoothFirmware-2.4.0",
        repo: "",
        pin: Pin {
            version: "2.4.0",
            url: gh_release!("OpenIntelWireless/IntelBluetoothFirmware", "v2.4.0", "IntelBluetooth-v2.4.0.zip"),
            sha256: Some("78418daa11f620012fc53ba68cbd643c2642a45f2f96c594fd01aaf2bc68fe2b"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: "",
        bundles: &["IntelBluetoothFirmware.kext", "IntelBTPatcher.kext", "IntelBluetoothInjector.kext"],
        description: "Upstream Intel Bluetooth release (IntelBTPatcher needs -ibtcompatbeta on macOS 26).",
    },
    KextCatalogEntry {
        id: "RealtekBluetoothFirmware",
        repo: "thegwchr/RealtekBluetoothFirmware",
        pin: Pin {
            version: "1.0.2",
            url: gh_release!(
                "thegwchr/RealtekBluetoothFirmware",
                "v1.0.2",
                "RealtekBluetoothFirmware_v1.0.2_5080405E-068B-3F14-92D7-0F48AB51C52B.zip"
            ),
            sha256: Some("d9fc23d9c8d942720d93e687cfff863aee1198a821ca4f089c8d2eae9cab0c61"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^RealtekBluetoothFirmware_v[0-9][0-9.]*_[0-9A-Fa-f-]+\.zip$",
        bundles: &["RealtekBluetoothFirmware.kext"],
        description: "Experimental Realtek USB Bluetooth firmware loader.",
    },
    // ── Input ───────────────────────────────────────────────────────────────
    KextCatalogEntry {
        id: "VoodooPS2Controller",
        repo: "acidanthera/VoodooPS2",
        pin: Pin {
            version: "2.3.7",
            url: gh_release!("acidanthera/VoodooPS2", "2.3.7", "VoodooPS2Controller-2.3.7-RELEASE.zip"),
            sha256: Some("d5483298736ba4b3b82c7203d9a46968c668fb7af732b8b22c2096deb0acf83f"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^VoodooPS2Controller-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["VoodooPS2Controller.kext"],
        description: "PS/2 keyboard, mouse and trackpad; plugins VoodooPS2Keyboard/Trackpad/Mouse.kext and VoodooInput.kext.",
    },
    KextCatalogEntry {
        id: "VoodooI2C",
        repo: "VoodooI2C/VoodooI2C",
        pin: Pin {
            version: "2.9.1",
            url: gh_release!("VoodooI2C/VoodooI2C", "v2.9.1", "VoodooI2C-2.9.1-RELEASE.zip"),
            sha256: Some("368e5eb60794583c340764813bd7e8244e94d3d3c06ea5b67034addd539487f4"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^VoodooI2C-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &[
            "VoodooI2C.kext",
            "VoodooI2CHID.kext",
            "VoodooI2CELAN.kext",
            "VoodooI2CSynaptics.kext",
            "VoodooI2CFTE.kext",
            "VoodooI2CAtmelMXT.kext",
        ],
        description: "I2C touchpads and touchscreens; plugins VoodooGPIO, VoodooI2CServices, VoodooInput; satellites are top-level bundles.",
    },
    KextCatalogEntry {
        id: "VoodooRMI",
        repo: "VoodooSMBus/VoodooRMI",
        pin: Pin {
            version: "1.4.3",
            url: gh_release!("VoodooSMBus/VoodooRMI", "1.4.3", "VoodooRMI-1.4.3-RELEASE.zip"),
            sha256: Some("8c55d92b5cb23423e332d60f4d3d1e33efaf697c8afaa8441a7f305af3e367f9"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^VoodooRMI-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["VoodooRMI.kext", "VoodooSMBus.kext"],
        description: "Synaptics RMI4 touchpads over SMBus or I2C; plugins RMISMBus, RMII2C, VoodooInput.",
    },
    KextCatalogEntry {
        id: "VoodooSMBus",
        repo: "VoodooSMBus/VoodooSMBus",
        pin: Pin {
            version: "2.2",
            url: gh_release!("VoodooSMBus/VoodooSMBus", "v2.2", "VoodooSMBus-v2.2.zip"),
            sha256: Some("a65d68b112ef89e262cb713af1f821d7eaa5978d3ef1a47f9d4d2338cbc6aa8e"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^VoodooSMBus-v[0-9][0-9.]*\.zip$",
        bundles: &["VoodooSMBus.kext"],
        description: "i801 SMBus driver for ELAN SMBus touchpads (not the build bundled with VoodooRMI).",
    },
    KextCatalogEntry {
        id: "AlpsHID",
        repo: "blankmac/AlpsHID",
        pin: Pin {
            version: "1.2",
            url: gh_release!("blankmac/AlpsHID", "v1.2", "AlpsHID1.2_release.zip"),
            sha256: Some("79c3181532b69b7db4fc27f6a4a22732da3d99ab0a0d10080f81d0bb014521e2"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^AlpsHID[0-9][0-9.]*_release\.zip$",
        bundles: &["AlpsHID.kext", "VoodooI2CHID.kext"],
        description: "Alps USB/I2C touchpads on VoodooI2C; ships its own VoodooI2CHID (never load two).",
    },
    KextCatalogEntry {
        id: "VoodooInput",
        repo: "acidanthera/VoodooInput",
        pin: Pin {
            version: "1.1.6",
            url: gh_release!("acidanthera/VoodooInput", "1.1.6", "VoodooInput-1.1.6-RELEASE.zip"),
            sha256: Some("c84a0ecf146dffc98cf661230e5085f045e333eed34456b7b55262d2e4de8b3b"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^VoodooInput-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["VoodooInput.kext"],
        description: "Standalone Magic Trackpad 2 emulation layer (normally the copy inside the touchpad driver is used).",
    },
    // ── USB ─────────────────────────────────────────────────────────────────
    KextCatalogEntry {
        id: "USBToolBox",
        repo: "USBToolBox/kext",
        pin: Pin {
            version: "1.2.0",
            url: gh_release!("USBToolBox/kext", "1.2.0", "USBToolBox-1.2.0-RELEASE.zip"),
            sha256: Some("c315a3a5acfd496dd97d0d19b4fbd1d487103d2fd541c5651583d4c9cebcfe07"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^USBToolBox-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["USBToolBox.kext", "UTBDefault.kext"],
        description: "USB port mapping driver; UTBDefault.kext enables all ports until a UTBMap.kext is made.",
    },
    KextCatalogEntry {
        id: "XHCI-unsupported",
        repo: "daliansky/OS-X-USB-Inject-All",
        pin: Pin {
            version: "0.9.2",
            url: gh_release!("daliansky/OS-X-USB-Inject-All", "v0.8.1", "XHCI-unsupported.zip"),
            sha256: Some("e4492975e24c60ec4ea107a207a2ec7199a1efca4b843b45967eb5157a0638e7"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: r"^XHCI-unsupported(\.kext)?\.zip$",
        bundles: &["XHCI-unsupported.kext"],
        description: "Codeless injector for non-native Intel XHCI controllers (X79/X99, H310/B360/H370, Z390 before 10.14).",
    },
    KextCatalogEntry {
        id: "GenericUSBXHCI",
        repo: "",
        pin: Pin {
            version: "1.3.0b2",
            url: gh_release!("RattletraPM/GUX-RyzenXHCIFix", "v1.3.0b1-ryzenxhcifix", "GenericUSBXHCI.kext.zip"),
            sha256: Some("2a97459605c13af889ad9e35d23ca520a0debd24e4fb1155b029753548d85b3d"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["GenericUSBXHCI.kext"],
        description: "Generic XHCI driver for Ryzen APU USB 3 controllers (11+). Only published as a prerelease.",
    },
    KextCatalogEntry {
        id: "XLNCUSBFix",
        repo: "",
        pin: Pin {
            version: "1.2",
            url: dortania_extra!("XLNCUSBFix.kext.zip"),
            sha256: Some("9c41be1ac76486347a88cb432d2666ff63b38a3568ba7c5bb163e11659c88125"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["XLNCUSBFix.kext"],
        description: "Codeless USB fix for AMD FX systems.",
    },
    // ── Storage, VMs ────────────────────────────────────────────────────────
    KextCatalogEntry {
        id: "CtlnaAHCIPort",
        repo: "",
        pin: Pin {
            version: "341.0.2",
            url: dortania_extra!("CtlnaAHCIPort.kext.zip"),
            sha256: Some("d55d6372f459488d56a39bef178421f2fda84a169034227faec31b36f61d9a39"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["CtlnaAHCIPort.kext"],
        description: "AHCI driver for SATA controllers macOS 11+ no longer supports (Dortania copy).",
    },
    KextCatalogEntry {
        id: "SATA-unsupported",
        repo: "",
        pin: Pin {
            version: "0.9.2",
            url: legacy_kexts!("Injectors/Zip/SATA-unsupported.kext.zip"),
            sha256: Some("319bba55113888fe58b57cc6bcb1fda506929a370996d7e067fc5578e9c61b84"),
        },
        archive: ArchiveKind::KextZip,
        latest_asset_regex: "",
        bundles: &["SATA-unsupported.kext"],
        description: "Codeless AHCI injector for Intel SATA controllers macOS 10.15 and older do not match (RST mode, some 100-series and mobile PCHs).",
    },
    KextCatalogEntry {
        id: "EmeraldSDHC",
        repo: "acidanthera/EmeraldSDHC",
        pin: Pin {
            version: "0.1.2",
            url: gh_release!("acidanthera/EmeraldSDHC", "0.1.2", "EmeraldSDHC-0.1.2-RELEASE.zip"),
            sha256: Some("64cd5946e65258e92095aef6abd426ef064faef2d0560e0bf2a8fd3b72653973"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^EmeraldSDHC-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["EmeraldSDHC.kext"],
        description: "eMMC storage driver (work in progress).",
    },
    KextCatalogEntry {
        id: "MacHyperVSupport",
        repo: "acidanthera/MacHyperVSupport",
        pin: Pin {
            version: "1.0.0",
            url: gh_release!("acidanthera/MacHyperVSupport", "1.0.0", "MacHyperVSupport-1.0.0-RELEASE.zip"),
            sha256: Some("490ebb1f8822b99d7efc175c0ce0e7a035a844093319927216570106f45bc0cf"),
        },
        archive: ArchiveKind::Zip,
        latest_asset_regex: r"^MacHyperVSupport-[0-9][0-9.]*-RELEASE\.zip$",
        bundles: &["MacHyperVSupport.kext", "MacHyperVSupportMonterey.kext", "MacHyperVFramebuffer.kext"],
        description: "Hyper-V guest support (MacHyperVSupportMonterey for 12+, same bundle id: load one).",
    },
];

pub fn all() -> &'static [KextCatalogEntry] {
    CATALOG
}

pub fn entry(id: &str) -> Option<&'static KextCatalogEntry> {
    all().iter().find(|e| e.id == id)
}

/// OpenCorePkg release pin (RELEASE and DEBUG zips share the version).
pub fn opencore_release() -> Pin {
    OPENCORE_RELEASE
}

pub fn opencore_debug() -> Pin {
    OPENCORE_DEBUG
}

/// OcBinaryData archive pinned to a commit (Resources for OpenCanopy, HfsPlus.efi).
pub fn ocbinarydata() -> Pin {
    OCBINARYDATA
}

/// Catalog id of the official AirportItlwm build for a macOS release, or
/// None when there is none (15 Sequoia and 26 Tahoe: use itlwm, or the
/// Ventura build with the legacy wireless stack). Sonoma maps to the 14.4
/// build because recovery always installs the latest 14.x.
pub fn airport_itlwm_id(target: MacOsVersion) -> Option<&'static str> {
    match target {
        MacOsVersion::HighSierra => Some("AirportItlwm-HighSierra"),
        MacOsVersion::Mojave => Some("AirportItlwm-Mojave"),
        MacOsVersion::Catalina => Some("AirportItlwm-Catalina"),
        MacOsVersion::BigSur => Some("AirportItlwm-BigSur"),
        MacOsVersion::Monterey => Some("AirportItlwm-Monterey"),
        MacOsVersion::Ventura => Some("AirportItlwm-Ventura"),
        MacOsVersion::Sonoma => Some("AirportItlwm-Sonoma14.4"),
        MacOsVersion::Sequoia | MacOsVersion::Tahoe => None,
    }
}

/// Recommended MinKernel/MaxKernel for an AirportItlwm build (each build only
/// matches the IO80211 stack of its own release). The Ventura build used with
/// the legacy wireless stack on 15/26 needs the target's range instead.
pub fn airport_itlwm_kernel_range(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "AirportItlwm-HighSierra" => ("17.0.0", "17.99.99"),
        "AirportItlwm-Mojave" => ("18.0.0", "18.99.99"),
        "AirportItlwm-Catalina" => ("19.0.0", "19.99.99"),
        "AirportItlwm-BigSur" => ("20.0.0", "20.99.99"),
        "AirportItlwm-Monterey" => ("21.0.0", "21.99.99"),
        "AirportItlwm-Ventura" => ("22.0.0", "22.99.99"),
        "AirportItlwm-Sonoma14.0" => ("23.0.0", "23.3.99"),
        "AirportItlwm-Sonoma14.4" => ("23.4.0", "23.99.99"),
        _ => return None,
    })
}

/// Dortania Getting-Started-With-ACPI prebuilt SSDTs, pinned to commit
/// 60029d4 (2024-08-11; `extra-files/compiled` last changed 2021-10-14).
macro_rules! dortania_acpi {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/dortania/Getting-Started-With-ACPI/",
            "60029d4fff6d56e210e5f682f6dd6508c4bb3343/extra-files/compiled/",
            $file
        )
    };
}

const DORTANIA_ACPI_COMMIT: &str = "60029d4fff6d56e210e5f682f6dd6508c4bb3343";

/// (file name, URL, SHA-256) of every prebuilt SSDT in that directory.
static DORTANIA_SSDTS: &[(&str, &str, &str)] = &[
    (
        "SSDT-AWAC.aml",
        dortania_acpi!("SSDT-AWAC.aml"),
        "13e6e56fc0ec9f97a3b69169d5b2a91684221610d248f0d4cc0bac27418d3eb3",
    ),
    (
        "SSDT-CPUR.aml",
        dortania_acpi!("SSDT-CPUR.aml"),
        "edc447852d26c95e5888a8517c78cc8b9dfc5160de9297db971fcae2acb6ff8d",
    ),
    (
        "SSDT-EC-DESKTOP.aml",
        dortania_acpi!("SSDT-EC-DESKTOP.aml"),
        "1db322abbcbc98f47499a572db6534f7bac1b6009c5fa71b235d6ea29079a7fa",
    ),
    (
        "SSDT-EC-LAPTOP.aml",
        dortania_acpi!("SSDT-EC-LAPTOP.aml"),
        "6a0ac070f0db9ff617a8d7396cf2ce3de4e4a5891d14f64791ddd5d7f1514678",
    ),
    (
        "SSDT-EC-USBX-DESKTOP.aml",
        dortania_acpi!("SSDT-EC-USBX-DESKTOP.aml"),
        "1cf941a6767fc95d2aa5c265e82fc0c94f05885a67ba6f9a34fe643b2f12dd2e",
    ),
    (
        "SSDT-EC-USBX-LAPTOP.aml",
        dortania_acpi!("SSDT-EC-USBX-LAPTOP.aml"),
        "dbe6692a8b3747fb18f26b1f12e9fe3e96d0d0964bea71edf0f9d5cb37fba29e",
    ),
    (
        "SSDT-IMEI-S.aml",
        dortania_acpi!("SSDT-IMEI-S.aml"),
        "43343b08adfff1de14f13925a5e9d1acba23e428d06c8a3ead7fdba584676735",
    ),
    (
        "SSDT-IMEI.aml",
        dortania_acpi!("SSDT-IMEI.aml"),
        "c585e0278ef68c867156f316ab94867a3880a7f69266ad6593a1db88151e30b0",
    ),
    (
        "SSDT-PLUG-DRTNIA.aml",
        dortania_acpi!("SSDT-PLUG-DRTNIA.aml"),
        "514aa0614ec36a532ddb2676d599607dedd3f6d49bd250ae1df450402377ba56",
    ),
    (
        "SSDT-PMC.aml",
        dortania_acpi!("SSDT-PMC.aml"),
        "8dcee5b817b8d35a9f5989674bebe2a34af9c2a0d8560932a31be866d2f9ed66",
    ),
    (
        "SSDT-PNLF.aml",
        dortania_acpi!("SSDT-PNLF.aml"),
        "a8aada737d48f7d069285be458d4da0797723add5b56a13c902faa73ae369ef6",
    ),
    (
        "SSDT-RHUB.aml",
        dortania_acpi!("SSDT-RHUB.aml"),
        "05d38acfa46db1f051249d7f90caa8d91c4ff940459b44d40f758b1b6ead6703",
    ),
    (
        "SSDT-RTC0-RANGE-HEDT.aml",
        dortania_acpi!("SSDT-RTC0-RANGE-HEDT.aml"),
        "5a4de4b89b72e1c5ac5fc10b741c32b756c2f120608b82ecc3eeb174c845a648",
    ),
    (
        "SSDT-UNC.aml",
        dortania_acpi!("SSDT-UNC.aml"),
        "fc19ad99bd586f7984e1df3d899a7b59c0de986f1faed0520dc20e9de44d8180",
    ),
    (
        "SSDT-XOSI.aml",
        dortania_acpi!("SSDT-XOSI.aml"),
        "fe798a64615d7195c38cf7f38530dc9fab97e2e88136c46f08d10873d9c35ca8",
    ),
];

/// Pinned download of a prebuilt Dortania SSDT ("SSDT-EC-USBX-DESKTOP.aml",
/// case-insensitive), or None when Dortania does not ship that file.
pub fn dortania_ssdt(file_name: &str) -> Option<Pin> {
    DORTANIA_SSDTS.iter().find(|(name, _, _)| name.eq_ignore_ascii_case(file_name)).map(|&(_, url, sha256)| Pin {
        version: &DORTANIA_ACPI_COMMIT[..7],
        url,
        sha256: Some(sha256),
    })
}

/// Raw URL of a prebuilt Dortania SSDT ("SSDT-EC-USBX-DESKTOP.aml") at the
/// pinned commit. Prefer [`dortania_ssdt`], which also carries the hash.
pub fn dortania_ssdt_url(file_name: &str) -> String {
    match dortania_ssdt(file_name) {
        Some(pin) => pin.url.to_string(),
        None => format!(
            "https://raw.githubusercontent.com/dortania/Getting-Started-With-ACPI/{DORTANIA_ACPI_COMMIT}/extra-files/compiled/{file_name}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use regex::Regex;

    use super::*;

    fn is_sha256(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = HashSet::new();
        for e in all() {
            assert!(seen.insert(e.id), "duplicate catalog id {}", e.id);
        }
    }

    #[test]
    fn pins_are_well_formed() {
        for e in all() {
            assert!(e.pin.url.starts_with("https://"), "{}: url must be https", e.id);
            assert!(!e.pin.version.is_empty(), "{}: empty version", e.id);
            match e.pin.sha256 {
                Some(sha) => assert!(is_sha256(sha), "{}: bad sha256 {sha}", e.id),
                None => {
                    assert_eq!(e.archive, ArchiveKind::NestedZip, "{}: only nightly artifacts may skip the hash", e.id)
                }
            }
            assert!(!e.bundles.is_empty(), "{}: no bundles", e.id);
            for b in e.bundles {
                assert!(b.ends_with(".kext") && !b.contains('/') && !b.contains('\\'), "{}: bad bundle {b}", e.id);
            }
            // Moving branches would change the bytes behind a pinned hash.
            if e.pin.sha256.is_some() {
                for moving in ["/master/", "/main/", "/refs/heads/"] {
                    assert!(!e.pin.url.contains(moving), "{}: url points at a branch", e.id);
                }
            }
        }
        for pin in [opencore_release(), opencore_debug(), ocbinarydata()] {
            assert!(pin.sha256.is_some_and(is_sha256));
        }
    }

    #[test]
    fn latest_regexes_select_release_assets_only() {
        for e in all() {
            if e.latest_asset_regex.is_empty() {
                assert!(!e.supports_latest());
                continue;
            }
            assert!(!e.repo.is_empty(), "{}: regex without repo", e.id);
            let re = Regex::new(e.latest_asset_regex).unwrap_or_else(|err| panic!("{}: {err}", e.id));
            let pinned_asset = e.pin.url.rsplit('/').next().unwrap_or_default();
            assert!(re.is_match(pinned_asset), "{}: regex does not match its own pin {pinned_asset}", e.id);
            let debug = pinned_asset.replace("RELEASE", "DEBUG").replace("release", "debug");
            if debug != pinned_asset {
                assert!(!re.is_match(&debug), "{}: regex matches {debug}", e.id);
            }
        }
        let nr = Regex::new(entry("NootedRed").map(|e| e.latest_asset_regex).unwrap_or_default()).unwrap();
        assert!(nr.is_match("NootedRed-0.8.11-RELEASE.zip"));
        assert!(!nr.is_match("NootedRed-0.8.10-RESEARCH_RELEASE.zip"));
        let itlwm = Regex::new(entry("itlwm").map(|e| e.latest_asset_regex).unwrap_or_default()).unwrap();
        assert!(!itlwm.is_match("AirportItlwm_v2.3.0_stable_BigSur.kext.zip"));
        let ibt =
            Regex::new(entry("IntelBluetoothFirmware").map(|e| e.latest_asset_regex).unwrap_or_default()).unwrap();
        assert!(!ibt.is_match("IntelBluetooth-2.5.1-DEBUG.zip"));
    }

    #[test]
    fn opencore_pins_match_version() {
        assert_eq!(opencore_release().version, "1.0.8");
        assert!(opencore_release().url.ends_with("OpenCore-1.0.8-RELEASE.zip"));
        assert!(opencore_debug().url.ends_with("OpenCore-1.0.8-DEBUG.zip"));
        assert_ne!(opencore_release().sha256, opencore_debug().sha256);
        assert!(ocbinarydata().url.contains(ocbinarydata().version));
    }

    #[test]
    fn airport_itlwm_ids_exist() {
        for v in MacOsVersion::ALL {
            if let Some(id) = airport_itlwm_id(v) {
                let e = entry(id).unwrap_or_else(|| panic!("missing {id}"));
                assert!(e.provides("AirportItlwm.kext"));
            }
        }
        assert!(airport_itlwm_id(MacOsVersion::Tahoe).is_none());
        for e in all().iter().filter(|e| e.id.starts_with("AirportItlwm-")) {
            let (min, max) = airport_itlwm_kernel_range(e.id).unwrap_or_else(|| panic!("no range for {}", e.id));
            assert!(min < max, "{}", e.id);
        }
        assert_eq!(airport_itlwm_kernel_range("AirportItlwm-Sonoma14.4"), Some(("23.4.0", "23.99.99")));
        assert!(airport_itlwm_kernel_range("itlwm").is_none());
    }

    #[test]
    fn dortania_ssdts_are_pinned() {
        let pin = dortania_ssdt("ssdt-ec-usbx-desktop.aml").unwrap();
        assert!(pin.url.ends_with("/extra-files/compiled/SSDT-EC-USBX-DESKTOP.aml"));
        assert!(pin.url.contains(DORTANIA_ACPI_COMMIT));
        assert_eq!(pin.version, "60029d4");
        assert!(dortania_ssdt("SSDT-GPIO.aml").is_none());
        let mut names = HashSet::new();
        for (name, url, sha) in DORTANIA_SSDTS {
            assert!(names.insert(name.to_ascii_lowercase()), "duplicate {name}");
            assert!(url.ends_with(name) && !url.contains("/master/"), "{url}");
            assert!(is_sha256(sha), "{name}");
        }
        assert_eq!(dortania_ssdt_url("SSDT-PLUG-DRTNIA.aml"), dortania_ssdt("SSDT-PLUG-DRTNIA.aml").unwrap().url);
        assert!(dortania_ssdt_url("SSDT-NEW.aml").contains(DORTANIA_ACPI_COMMIT));
    }

    #[test]
    fn known_bundles_are_provided() {
        let expect = [
            ("VirtualSMC", "SMCProcessor.kext"),
            ("VoodooI2C", "VoodooI2CHID.kext"),
            ("BrcmPatchRAM", "BlueToolFixup.kext"),
            ("IntelBluetoothFirmware", "IntelBTPatcher.kext"),
            ("USBToolBox", "UTBDefault.kext"),
            ("VoodooRMI", "VoodooSMBus.kext"),
            ("AppleALC", "AppleALCU.kext"),
            ("CpuTopologyRebuild", "CpuTopologyRebuild.kext"),
        ];
        for (id, bundle) in expect {
            assert!(entry(id).is_some_and(|e| e.provides(bundle)), "{id} should provide {bundle}");
        }
        assert!(entry("RealtekRTL8111-2.4.2").is_some_and(|e| e.pin.version == "2.4.2"));
        assert!(entry("RealtekRTL8111-2.2.2").is_some_and(|e| e.provides("RealtekRTL8111.kext")));
        assert!(entry("SmallTreeIntel82576-1.2.5").is_some_and(|e| e.provides("SmallTreeIntel82576.kext")));
        for id in ["telemetrap", "AppleIntelCPUPowerManagement", "AppleIntelCPUPowerManagementClient"] {
            assert!(entry(id).is_some_and(|e| e.pin.url.contains("/OpenCore-Legacy-Patcher/")), "{id}");
        }
        assert!(entry("SATA-unsupported").is_some_and(|e| e.provides("SATA-unsupported.kext")));
        assert!(entry("NootRX").is_some_and(|e| e.pin.sha256.is_none() && e.archive == ArchiveKind::NestedZip));
    }
}
