//! Kext source registry with GitHub repos and URLs.
//! Ported from electron/kextSourcePolicy.ts

#[derive(Debug, Clone)]
pub struct KextRegistryEntry {
    pub repo: String,
    pub asset_filter: Option<String>,
    pub direct_url: Option<String>,
    pub static_version: Option<String>,
    pub embedded_fallback: bool,
}

#[derive(Debug, Clone)]
pub struct KextReleaseProbe {
    pub version: Option<String>,
    pub asset_url: Option<String>,
    pub asset_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KextSourceRoute {
    Bundled,
    Github,
    Direct,
    Embedded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct KextSourceResolution {
    pub route: KextSourceRoute,
    pub available: bool,
    pub version: Option<String>,
    pub asset_url: Option<String>,
    pub message: String,
}

pub struct ResolveOptions {
    pub direct_url_reachable: Option<bool>,
    pub direct_url_error: Option<String>,
}

pub fn kext_registry_entry(kext_name: &str) -> Option<KextRegistryEntry> {
    let (repo, asset_filter, direct_url, static_version) = match kext_name {
        "Lilu.kext" => ("acidanthera/Lilu", Some("RELEASE"), None, None),
        "VirtualSMC.kext" => ("acidanthera/VirtualSMC", Some("RELEASE"), None, None),
        "SMCBatteryManager.kext" => ("acidanthera/VirtualSMC", Some("RELEASE"), None, None),
        "WhateverGreen.kext" => ("acidanthera/WhateverGreen", Some("RELEASE"), None, None),
        "AppleALC.kext" => ("acidanthera/AppleALC", Some("RELEASE"), None, None),
        "NootedRed.kext" => (
            "ChefKissInc/NootedRed",
            None,
            Some("https://nightly.link/ChefKissInc/NootedRed/workflows/main/master/Artifacts.zip"),
            Some("nightly"),
        ),
        "NootRX.kext" => (
            "ChefKissInc/NootRX",
            None,
            Some("https://nightly.link/ChefKissInc/NootRX/workflows/main/master/Artifacts.zip"),
            Some("nightly"),
        ),
        "RTCMemoryFixup.kext" => ("acidanthera/RTCMemoryFixup", Some("RELEASE"), None, None),
        "VoodooPS2Controller.kext" => ("acidanthera/VoodooPS2", Some("RELEASE"), None, None),
        "VoodooI2C.kext" => ("VoodooI2C/VoodooI2C", Some("VoodooI2C"), None, None),
        "VoodooI2CHID.kext" => ("VoodooI2C/VoodooI2C", Some("VoodooI2C"), None, None),
        "AMDRyzenCPUPowerManagement.kext" => (
            "trulyspinach/SMCAMDProcessor",
            None,
            Some("https://github.com/trulyspinach/SMCAMDProcessor/releases/latest/download/AMDRyzenCPUPowerManagement.kext.zip"),
            Some("latest"),
        ),
        "SMCAMDProcessor.kext" => (
            "trulyspinach/SMCAMDProcessor",
            None,
            Some("https://github.com/trulyspinach/SMCAMDProcessor/releases/latest/download/SMCAMDProcessor.kext.zip"),
            Some("latest"),
        ),
        "AppleMCEReporterDisabler.kext" => (
            "acidanthera/bugtracker",
            None,
            Some("https://github.com/acidanthera/bugtracker/files/3703498/AppleMCEReporterDisabler.kext.zip"),
            Some("bugtracker"),
        ),
        "RestrictEvents.kext" => ("acidanthera/RestrictEvents", Some("RELEASE"), None, None),
        "NVMeFix.kext" => ("acidanthera/NVMeFix", Some("RELEASE"), None, None),
        "CPUTopologyRebuild.kext" => ("b00t0x/CpuTopologyRebuild", Some("RELEASE"), None, None),
        "ECEnabler.kext" => ("averycblack/ECEnabler", Some("RELEASE"), None, None),
        "SMCProcessor.kext" => ("acidanthera/VirtualSMC", Some("RELEASE"), None, None),
        "SMCSuperIO.kext" => ("acidanthera/VirtualSMC", Some("RELEASE"), None, None),
        "IntelMausi.kext" => ("acidanthera/IntelMausi", Some("RELEASE"), None, None),
        "RealtekRTL8111.kext" => ("Mieze/RTL8111_driver_for_OS_X", Some("RealtekRTL8111"), None, None),
        "itlwm.kext" => ("OpenIntelWireless/itlwm", Some("itlwm"), None, None),
        "AirportItlwm.kext" => ("OpenIntelWireless/itlwm", Some("AirportItlwm"), None, None),
        "AirportBrcmFixup.kext" => ("acidanthera/AirportBrcmFixup", Some("RELEASE"), None, None),
        "USBInjectAll.kext" => ("Sniki/OS-X-USB-Inject-All", Some("RELEASE"), None, None),
        "IntelBluetoothFirmware.kext" => ("OpenIntelWireless/IntelBluetoothFirmware", Some("IntelBluetoothFirmware"), None, None),
        "BlueToolFixup.kext" => ("acidanthera/BrcmPatchRAM", Some("RELEASE"), None, None),
        "BrcmPatchRAM3.kext" => ("acidanthera/BrcmPatchRAM", Some("RELEASE"), None, None),
        "BrcmFirmwareData.kext" => ("acidanthera/BrcmPatchRAM", Some("RELEASE"), None, None),
        "CPUFriend.kext" => ("acidanthera/CPUFriend", Some("RELEASE"), None, None),
        "XHCI-unsupported.kext" => ("Sniki/OS-X-USB-Inject-All", Some("RELEASE"), None, None),
        "LucyRTL8125Ethernet.kext" => ("Mieze/LucyRTL8125Ethernet", Some("LucyRTL8125Ethernet"), None, None),
        _ => return None,
    };

    Some(KextRegistryEntry {
        repo: repo.to_string(),
        asset_filter: asset_filter.map(str::to_string),
        direct_url: direct_url.map(str::to_string),
        static_version: static_version.map(str::to_string),
        embedded_fallback: false,
    })
}

pub fn is_optional_kext(kext_name: &str) -> bool {
    matches!(
        kext_name,
        "itlwm.kext"
            | "AirportItlwm.kext"
            | "IntelBluetoothFirmware.kext"
            | "BlueToolFixup.kext"
            | "BrcmPatchRAM3.kext"
            | "BrcmFirmwareData.kext"
            | "AirportBrcmFixup.kext"
            | "VoodooI2C.kext"
            | "VoodooI2CHID.kext"
            | "CPUFriend.kext"
            | "XHCI-unsupported.kext"
            | "LucyRTL8125Ethernet.kext"
            | "NVMeFix.kext"
    )
}

fn normalize_message(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => fallback.to_string(),
    }
}

pub fn resolve_kext_source_plan(
    kext_name: &str,
    entry: Option<&KextRegistryEntry>,
    probe: Option<&KextReleaseProbe>,
    options: Option<&ResolveOptions>,
) -> KextSourceResolution {
    let entry = match entry {
        None => return KextSourceResolution {
            route: KextSourceRoute::Bundled, available: true,
            version: Some("bundled".into()), asset_url: None,
            message: format!("{} is bundled with the app.", kext_name),
        },
        Some(e) => e,
    };

    if let Some(ref direct_url) = entry.direct_url {
        let reachable = options.and_then(|o| o.direct_url_reachable).unwrap_or(true);
        if reachable {
            return KextSourceResolution {
                route: KextSourceRoute::Direct, available: true,
                version: entry.static_version.clone().or(Some("direct".into())),
                asset_url: Some(direct_url.clone()),
                message: format!("{} can be downloaded directly without the GitHub API.", kext_name),
            };
        }
        if entry.embedded_fallback {
            return KextSourceResolution {
                route: KextSourceRoute::Embedded, available: true,
                version: Some("embedded".into()), asset_url: None,
                message: format!("{} direct download is unavailable, but a bundled fallback is ready.", kext_name),
            };
        }
        let err = options.and_then(|o| o.direct_url_error.as_deref());
        return KextSourceResolution {
            route: KextSourceRoute::Failed, available: false,
            version: None, asset_url: None,
            message: normalize_message(err, &format!("{} direct download is unavailable.", kext_name)),
        };
    }

    if let Some(p) = probe {
        if p.asset_url.is_some() {
            return KextSourceResolution {
                route: KextSourceRoute::Github, available: true,
                version: p.version.clone().or(Some("unknown".into())),
                asset_url: p.asset_url.clone(),
                message: format!("{} latest release asset was resolved from GitHub.", kext_name),
            };
        }
    }

    if entry.embedded_fallback {
        let msg = match probe.and_then(|p| p.error.as_deref()) {
            Some(_) => format!("{} GitHub lookup failed, but a bundled fallback is ready.", kext_name),
            None => format!("{} release asset was not found, but a bundled fallback is ready.", kext_name),
        };
        return KextSourceResolution {
            route: KextSourceRoute::Embedded, available: true,
            version: Some("embedded".into()), asset_url: None, message: msg,
        };
    }

    let err = probe.and_then(|p| p.error.as_deref());
    KextSourceResolution {
        route: KextSourceRoute::Failed, available: false,
        version: probe.and_then(|p| p.version.clone()),
        asset_url: None,
        message: normalize_message(err, &format!("No usable release asset was found for {}.", kext_name)),
    }
}
