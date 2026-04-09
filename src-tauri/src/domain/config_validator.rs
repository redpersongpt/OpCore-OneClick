//! Config.plist and EFI validation.
//! Ported from electron/configValidator.ts

use std::collections::HashSet;

/// Required top-level OpenCore config.plist sections.
pub const REQUIRED_SECTIONS: &[&str] = &[
    "ACPI", "Booter", "DeviceProperties", "Kernel", "Misc", "NVRAM", "PlatformInfo", "UEFI",
];

/// Lilu plugin kexts that require Lilu.kext as a dependency.
pub fn lilu_plugin_kexts() -> HashSet<&'static str> {
    [
        "WhateverGreen.kext", "AppleALC.kext", "RestrictEvents.kext",
        "CPUTopologyRebuild.kext", "NootRX.kext", "NootedRed.kext", "NVMeFix.kext",
    ].into_iter().collect()
}

/// VirtualSMC plugin kexts that require VirtualSMC.kext.
pub fn virtualsmc_plugin_kexts() -> HashSet<&'static str> {
    [
        "SMCProcessor.kext", "SMCSuperIO.kext", "SMCBatteryManager.kext",
        "SMCLightSensor.kext", "SMCDellSensors.kext",
    ].into_iter().collect()
}

/// VoodooI2C plugin kexts.
pub fn voodooi2c_plugin_kexts() -> HashSet<&'static str> {
    ["VoodooI2CHID.kext"].into_iter().collect()
}

#[derive(Debug, Clone)]
pub struct PlistValidationIssue {
    pub code: String,
    pub severity: String, // "blocked" | "warning"
    pub message: String,
    pub detail: Option<String>,
    pub component: String,
    pub expected_path: String,
    pub actual_condition: String,
}

#[derive(Debug, Clone)]
pub struct PlistValidationResult {
    pub overall: String, // "pass" | "warning" | "blocked"
    pub issues: Vec<PlistValidationIssue>,
    pub checked_at: String,
}

/// Validate a config.plist XML string for structural correctness.
pub fn validate_config_plist_content(plist_content: &str) -> PlistValidationResult {
    let mut issues = Vec::new();

    // Check basic XML structure
    let has_plist_open = plist_content.contains("<plist");
    let has_plist_close = plist_content.contains("</plist>");
    let has_dict = plist_content.contains("<dict>");

    if !has_plist_open || !has_plist_close || !has_dict {
        issues.push(PlistValidationIssue {
            code: "PLIST_INVALID".into(),
            severity: "blocked".into(),
            message: "config.plist is not a valid XML plist".into(),
            detail: Some("Missing <plist>, </plist>, or <dict> structure".into()),
            component: "config.plist".into(),
            expected_path: "EFI/OC/config.plist".into(),
            actual_condition: "Missing required plist XML structure".into(),
        });
    } else {
        // Check required sections
        let missing: Vec<&str> = REQUIRED_SECTIONS.iter()
            .filter(|&&section| {
                let key_tag = format!("<key>{}</key>", section);
                !plist_content.contains(&key_tag)
            })
            .copied()
            .collect();

        if !missing.is_empty() {
            issues.push(PlistValidationIssue {
                code: "PLIST_SECTIONS_MISSING".into(),
                severity: "blocked".into(),
                message: format!("config.plist is missing required OpenCore sections: {}", missing.join(", ")),
                detail: Some(format!("Required: {}", REQUIRED_SECTIONS.join(", "))),
                component: "config.plist".into(),
                expected_path: "EFI/OC/config.plist".into(),
                actual_condition: format!("Missing top-level config sections: {}", missing.join(", ")),
            });
        }
    }

    // Check kext dependencies in plist content
    let kexts_in_config = extract_plist_array_entries(plist_content, &["Kernel", "Add"], "BundlePath");
    let top_level_kexts: HashSet<String> = kexts_in_config.iter()
        .map(|bp| bp.split('/').next().unwrap_or(bp).to_string())
        .collect();

    if !top_level_kexts.is_empty() {
        // Lilu dependency
        if !top_level_kexts.contains("Lilu.kext") {
            let lilu_plugins = lilu_plugin_kexts();
            let has_lilu_plugin = top_level_kexts.iter().any(|k| lilu_plugins.contains(k.as_str()));
            if has_lilu_plugin {
                issues.push(PlistValidationIssue {
                    code: "KEXT_LILU_DEPENDENCY".into(),
                    severity: "blocked".into(),
                    message: "Lilu plugin selected without Lilu.kext".into(),
                    detail: None,
                    component: "Lilu.kext".into(),
                    expected_path: "EFI/OC/Kexts/Lilu.kext".into(),
                    actual_condition: "Lilu plugins present but Lilu.kext missing".into(),
                });
            }
        }

        // VirtualSMC dependency
        if !top_level_kexts.contains("VirtualSMC.kext") {
            let vsmc_plugins = virtualsmc_plugin_kexts();
            let has_vsmc_plugin = top_level_kexts.iter().any(|k| vsmc_plugins.contains(k.as_str()));
            if has_vsmc_plugin {
                issues.push(PlistValidationIssue {
                    code: "KEXT_VIRTUALSMC_DEPENDENCY".into(),
                    severity: "blocked".into(),
                    message: "VirtualSMC plugin selected without VirtualSMC.kext".into(),
                    detail: None,
                    component: "VirtualSMC.kext".into(),
                    expected_path: "EFI/OC/Kexts/VirtualSMC.kext".into(),
                    actual_condition: "VirtualSMC plugins present but VirtualSMC.kext missing".into(),
                });
            }
        }

        // VoodooI2C dependency
        let vi2c_plugins = voodooi2c_plugin_kexts();
        let has_vi2c_plugin = kexts_in_config.iter().any(|bp| {
            let leaf = bp.split('/').last().unwrap_or(bp);
            vi2c_plugins.contains(leaf)
        });
        if has_vi2c_plugin && !top_level_kexts.contains("VoodooI2C.kext") {
            issues.push(PlistValidationIssue {
                code: "KEXT_VOODOOI2C_DEPENDENCY".into(),
                severity: "blocked".into(),
                message: "VoodooI2C plugin selected without VoodooI2C.kext".into(),
                detail: None,
                component: "VoodooI2C.kext".into(),
                expected_path: "EFI/OC/Kexts/VoodooI2C.kext".into(),
                actual_condition: "VoodooI2CHID configured but VoodooI2C.kext missing".into(),
            });
        }
    }

    // Check AMD patches on non-AMD
    let _has_amd_patches = extract_plist_array_entries(plist_content, &["Kernel", "Patch"], "Comment")
        .iter()
        .any(|c| {
            let lower = c.to_lowercase();
            lower.contains("amd") || lower.contains("ryzen") || lower.contains("threadripper")
                || lower.contains("genuineintel") && lower.contains("bypass")
        });

    // Check placeholder serials
    let serial = extract_simple_plist_value(plist_content, "SystemSerialNumber");
    let uuid = extract_simple_plist_value(plist_content, "SystemUUID");
    let mlb = extract_simple_plist_value(plist_content, "MLB");

    let is_placeholder = |v: &Option<String>| -> bool {
        match v {
            None => false,
            Some(v) => {
                v.starts_with("W00000") || v.starts_with("M00000")
                    || v == "00000000-0000-0000-0000-000000000000"
                    || v.starts_with("OC") || v.starts_with("OCB")
            }
        }
    };

    if is_placeholder(&serial) || is_placeholder(&mlb) || is_placeholder(&uuid) {
        issues.push(PlistValidationIssue {
            code: "PLATFORMINFO_PLACEHOLDER_SERIALS".into(),
            severity: "warning".into(),
            message: "PlatformInfo contains placeholder serial numbers".into(),
            detail: Some("Generate valid serials with GenSMBIOS before booting.".into()),
            component: "PlatformInfo".into(),
            expected_path: "EFI/OC/config.plist".into(),
            actual_condition: "Placeholder serial values detected".into(),
        });
    }

    let overall = if issues.iter().any(|i| i.severity == "blocked") { "blocked" }
        else if issues.iter().any(|i| i.severity == "warning") { "warning" }
        else { "pass" };

    PlistValidationResult {
        overall: overall.into(),
        issues,
        checked_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Extract array entries from a plist section using simple string matching.
fn extract_plist_array_entries(plist: &str, section_path: &[&str], entry_key: &str) -> Vec<String> {
    let mut cursor = plist;
    for key in section_path {
        let key_tag = format!("<key>{}</key>", key);
        match cursor.find(&key_tag) {
            Some(idx) => cursor = &cursor[idx + key_tag.len()..],
            None => return vec![],
        }
    }
    let array_start = match cursor.find("<array>") {
        Some(i) => i,
        None => return vec![],
    };
    let array_end = match cursor[array_start..].find("</array>") {
        Some(i) => array_start + i,
        None => return vec![],
    };
    let array_content = &cursor[array_start..array_end];

    let mut results = Vec::new();
    let key_tag = format!("<key>{}</key>", entry_key);
    let mut search_from = 0;
    loop {
        match array_content[search_from..].find(&key_tag) {
            None => break,
            Some(idx) => {
                let after_key = search_from + idx + key_tag.len();
                if let Some(str_start) = array_content[after_key..].find("<string>") {
                    if let Some(str_end) = array_content[after_key..].find("</string>") {
                        let val_start = after_key + str_start + "<string>".len();
                        let val_end = after_key + str_end;
                        if val_start <= val_end && val_end <= array_content.len() {
                            results.push(array_content[val_start..val_end].to_string());
                        }
                    }
                }
                search_from = after_key;
            }
        }
    }
    results
}

fn extract_simple_plist_value(plist: &str, key: &str) -> Option<String> {
    let pattern = format!("<key>{}</key>", key);
    let idx = plist.find(&pattern)?;
    let after = &plist[idx + pattern.len()..];
    let str_start = after.find("<string>")?;
    let str_end = after.find("</string>")?;
    if str_start < str_end {
        Some(after[str_start + "<string>".len()..str_end].trim().to_string())
    } else {
        None
    }
}
