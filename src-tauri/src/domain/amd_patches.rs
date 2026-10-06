//! AMD_Vanilla kernel patches (AMD-OSX/AMD_Vanilla). The upstream repository
//! carries no licence, so `patches.plist` is downloaded at build time from a
//! pinned commit (`AMD_VANILLA`, `AMD_VANILLA_TAHOE`) instead of being vendored.

use std::io::Cursor;

use plist::{Dictionary, Value};

use crate::domain::kext_catalog::Pin;
use crate::error::AppError;

use super::model::{hex_upper, BinaryPatch, MacOsVersion};

/// Pinned `patches.plist` (raw.githubusercontent.com URL at a fixed commit + SHA-256).
/// AMD-OSX/AMD_Vanilla master @ eaf52ef2 (2025-10-17); the file itself was
/// last changed in c735eded (2025-06-10, Tahoe kernel range). 25 patches.
///
/// Upstream master lacks two macOS 26 fixes: the 26.0+ `probeBusGated`
/// 10-bit-tag patch (only on the `beta` branch) and the Darwin 25.4+
/// `thread_invoke`/`thread_dispatch` non-monotonic time fix (PR #215, not
/// merged). See `AMD_VANILLA_TAHOE`.
pub const AMD_VANILLA: Pin = Pin {
    version: "eaf52ef2",
    url: "https://raw.githubusercontent.com/AMD-OSX/AMD_Vanilla/eaf52ef292abf4ebec899df6d48626569ba50cc6/patches.plist",
    sha256: Some("4bc820109b3d020c3c547fa23c49e0098e4f4a2c625ed6184dd54e390e84e1ab"),
};

/// laobamac/AMD_Vanilla master @ a174cca8 (2026-03-30): upstream master plus
/// the two macOS 26 fixes above, with the 12.0-15.x 10-bit-tag patch and the
/// old thread_invoke patch capped so every kernel range gets exactly one
/// variant. 27 patches, same format; the set OpCore-Simplify ships for Tahoe.
pub const AMD_VANILLA_TAHOE: Pin = Pin {
    version: "laobamac-a174cca8",
    url: "https://raw.githubusercontent.com/laobamac/AMD_Vanilla/a174cca80efe2377fde3b902666c70863ea66454/patches.plist",
    sha256: Some("dc147342ba068f8221783b43ca5d233587167e4907ae523d83146e5451bf3298"),
};

/// The patch set to download for `target`: the laobamac set for macOS 26
/// (upstream master has no 26.0+ 10-bit-tag patch and no 26.4+ fix for the
/// non-monotonic time panic), upstream master for every older release, where
/// both sets apply the same patches.
pub fn pin_for_target(target: MacOsVersion) -> Pin {
    if target >= MacOsVersion::Tahoe {
        AMD_VANILLA_TAHOE
    } else {
        AMD_VANILLA
    }
}

/// Comment marker of the core-count patches whose Replace byte 1 carries the
/// physical core count (`mov eax/edx, imm32`).
const CORE_COUNT_MARKER: &str = "cpuid_cores_per_package";
// Comment markers of the PAT patches and of the AM5 hot-plug port patch.
const PAT_MARKER: &str = "_mtrr_update_action";
const HOTPLUG_MARKER: &str = "IOPCIIsHotplugPort";

/// `_mtrr_update_action` PAT patch variant to enable. AMD_Vanilla carries an
/// Algrey and a Shaneee patch for each kernel range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatPatch {
    /// PAT 0x00070106 (WB/WC/UC-/UC): works with every GPU, upstream default.
    #[default]
    Algrey,
    /// PAT 0x06060606 (all write-back): faster AMD GPU compute, but can break
    /// NVIDIA GPUs and HDMI/DP audio.
    Shaneee,
    /// No PAT patch at all (TRX40 Threadripper, per the AMD_Vanilla README).
    Disabled,
}

/// Enable exactly the chosen PAT variant in every kernel range. Returns how
/// many PAT patches were found.
pub fn select_pat_patch(patches: &mut [BinaryPatch], choice: PatPatch) -> usize {
    let mut found = 0;
    for patch in patches
        .iter_mut()
        .filter(|p| p.comment.contains(PAT_MARKER))
    {
        found += 1;
        let shaneee = patch
            .comment
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("shaneee");
        patch.enabled = match choice {
            PatPatch::Algrey => !shaneee,
            PatPatch::Shaneee => shaneee,
            PatPatch::Disabled => false,
        };
    }
    found
}

/// Turn the `IOPCIIsHotplugPort` patch on or off (off upstream; needed on AM5
/// boards whose PCI devices disappear while on-board Thunderbolt/USB4 and
/// Wi-Fi are both enabled). Returns false when the set has no such patch.
pub fn set_hotplug_port_fix(patches: &mut [BinaryPatch], enabled: bool) -> bool {
    let mut found = false;
    for patch in patches
        .iter_mut()
        .filter(|p| p.comment.contains(HOTPLUG_MARKER))
    {
        patch.enabled = enabled;
        found = true;
    }
    found
}

/// Parse AMD_Vanilla `patches.plist` (its `Kernel/Patch` array) into
/// `BinaryPatch`es, keeping upstream order and every field exactly, and set the
/// core-count byte of the `algrey - Force cpuid_cores_per_package` patches'
/// Replace data to `core_count` (physical cores per package, 1..=255).
/// Rejects `core_count == 0`.
pub fn amd_vanilla_patches(
    patches_plist: &[u8],
    core_count: u32,
) -> Result<Vec<BinaryPatch>, AppError> {
    let core_byte = u8::try_from(core_count).ok().filter(|&c| c != 0).ok_or_else(|| {
        AppError::new(
            "AMD_CORE_COUNT_INVALID",
            format!("Physical core count {core_count} is not usable for the AMD core-count patch (1-255)"),
        )
        .with_suggestion("Enter the number of physical CPU cores (not threads) in the hardware profile.")
    })?;

    let root = Value::from_reader(Cursor::new(patches_plist))
        .map_err(|e| invalid(format!("patches.plist is not a property list: {e}")))?;
    let patches = root
        .as_dictionary()
        .and_then(|d| d.get("Kernel"))
        .and_then(Value::as_dictionary)
        .and_then(|d| d.get("Patch"))
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("patches.plist has no Kernel/Patch array".to_string()))?;

    let mut out = Vec::with_capacity(patches.len());
    let mut core_patches = 0usize;
    for (index, value) in patches.iter().enumerate() {
        let entry = value
            .as_dictionary()
            .ok_or_else(|| invalid(format!("Kernel/Patch[{index}] is not a dictionary")))?;
        let mut patch = parse_patch(entry, index)?;
        if patch.comment.contains(CORE_COUNT_MARKER) {
            set_core_count(&mut patch, core_byte, index)?;
            core_patches += 1;
        }
        out.push(patch);
    }

    if core_patches == 0 {
        return Err(invalid(
            "patches.plist contains no cpuid_cores_per_package patch".to_string(),
        ));
    }
    tracing::debug!(
        patches = out.len(),
        core_patches,
        core_count,
        "parsed AMD_Vanilla patches"
    );
    Ok(out)
}

fn invalid(message: String) -> AppError {
    AppError::new("AMD_PATCHES_INVALID", message)
}

fn parse_patch(entry: &Dictionary, index: usize) -> Result<BinaryPatch, AppError> {
    let at = format!("Kernel/Patch[{index}]");
    let ctx = |key: &str| format!("{at}/{key}");

    let string = |key: &str, default: &str| -> Result<String, AppError> {
        match entry.get(key) {
            None => Ok(default.to_string()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(invalid(format!("{} must be a string", ctx(key)))),
        }
    };
    let data = |key: &str| -> Result<Vec<u8>, AppError> {
        match entry.get(key) {
            None => Ok(Vec::new()),
            Some(Value::Data(d)) => Ok(d.clone()),
            Some(_) => Err(invalid(format!("{} must be data", ctx(key)))),
        }
    };
    let integer = |key: &str| -> Result<u32, AppError> {
        match entry.get(key) {
            None => Ok(0),
            Some(Value::Integer(i)) => i
                .as_unsigned()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| invalid(format!("{} is out of range", ctx(key)))),
            Some(_) => Err(invalid(format!("{} must be an integer", ctx(key)))),
        }
    };
    let enabled = match entry.get("Enabled") {
        None => false,
        Some(Value::Boolean(b)) => *b,
        Some(_) => return Err(invalid(format!("{} must be a boolean", ctx("Enabled")))),
    };

    let comment = string("Comment", "")?;
    let identifier = string("Identifier", "")?;
    if identifier.is_empty() {
        return Err(invalid(format!("{} is empty", ctx("Identifier"))));
    }
    // Upstream has one Base with a trailing space; OpenCore trims it too.
    let base = string("Base", "")?.trim().to_string();
    let find = data("Find")?;
    let mask = data("Mask")?;
    let replace = data("Replace")?;
    let replace_mask = data("ReplaceMask")?;

    if replace.is_empty() {
        return Err(invalid(format!("{} is empty", ctx("Replace"))));
    }
    if find.is_empty() && base.is_empty() {
        return Err(invalid(format!("{at} needs Find or Base")));
    }
    if !find.is_empty() && find.len() != replace.len() {
        return Err(invalid(format!("{at}: Find and Replace differ in size")));
    }
    if !mask.is_empty() && mask.len() != find.len() {
        return Err(invalid(format!("{at}: Mask and Find differ in size")));
    }
    if !replace_mask.is_empty() && replace_mask.len() != replace.len() {
        return Err(invalid(format!(
            "{at}: ReplaceMask and Replace differ in size"
        )));
    }

    Ok(BinaryPatch {
        comment,
        arch: string("Arch", "Any")?,
        identifier,
        base,
        find: hex_upper(&find),
        mask: hex_upper(&mask),
        replace: hex_upper(&replace),
        replace_mask: hex_upper(&replace_mask),
        count: integer("Count")?,
        limit: integer("Limit")?,
        skip: integer("Skip")?,
        min_kernel: string("MinKernel", "")?,
        max_kernel: string("MaxKernel", "")?,
        enabled,
    })
}

/// Byte 1 of Replace is the imm32 low byte of `mov eax/edx, imm32`.
fn set_core_count(patch: &mut BinaryPatch, core_count: u8, index: usize) -> Result<(), AppError> {
    let opcode = patch.replace.get(0..2).unwrap_or_default();
    if patch.replace.len() < 4 || !matches!(opcode, "B8" | "BA") {
        return Err(invalid(format!(
            "Kernel/Patch[{index}] core-count Replace {} is not a mov imm32",
            patch.replace
        )));
    }
    patch
        .replace
        .replace_range(2..4, &format!("{core_count:02X}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNTHETIC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Kernel</key>
	<dict>
		<key>Patch</key>
		<array>
			<dict>
				<key>Arch</key>
				<string>x86_64</string>
				<key>Base</key>
				<string>_cpuid_set_info</string>
				<key>Comment</key>
				<string>algrey | Force cpuid_cores_per_package to constant (user-specified) | 13.3+</string>
				<key>Count</key>
				<integer>1</integer>
				<key>Enabled</key>
				<true/>
				<key>Find</key>
				<data>wegaAAA=</data>
				<key>Identifier</key>
				<string>kernel</string>
				<key>Limit</key>
				<integer>0</integer>
				<key>Mask</key>
				<data>//3/AAA=</data>
				<key>MaxKernel</key>
				<string>25.99.99</string>
				<key>MinKernel</key>
				<string>22.4.0</string>
				<key>Replace</key>
				<data>ugAAAAA=</data>
				<key>ReplaceMask</key>
				<data>//////8=</data>
				<key>Skip</key>
				<integer>0</integer>
			</dict>
			<dict>
				<key>Arch</key>
				<string>x86_64</string>
				<key>Base</key>
				<string>_cpuid_set_info </string>
				<key>Comment</key>
				<string>algrey | _cpuid_set_cpufamily | Force CPUFAMILY_INTEL_PENRYN | 11.3+</string>
				<key>Count</key>
				<integer>1</integer>
				<key>Enabled</key>
				<true/>
				<key>Find</key>
				<data>gD0AAAAABnU=</data>
				<key>Identifier</key>
				<string>kernel</string>
				<key>Limit</key>
				<integer>0</integer>
				<key>Mask</key>
				<data>//8AAAAA//8=</data>
				<key>MaxKernel</key>
				<string>25.99.99</string>
				<key>MinKernel</key>
				<string>20.4.0</string>
				<key>Replace</key>
				<data>urxP6ngx2+s=</data>
				<key>ReplaceMask</key>
				<data></data>
				<key>Skip</key>
				<integer>0</integer>
			</dict>
			<dict>
				<key>Arch</key>
				<string>x86_64</string>
				<key>Base</key>
				<string>__ZN17IOPCIConfigurator18IOPCIIsHotplugPortEP16IOPCIConfigEntry</string>
				<key>Comment</key>
				<string>CaseySJ | IOPCIIsHotplugPort | Fix PCI bus enumeration on AM5 | 13.0+</string>
				<key>Count</key>
				<integer>1</integer>
				<key>Enabled</key>
				<false/>
				<key>Find</key>
				<data>hAB1Sw==</data>
				<key>Identifier</key>
				<string>com.apple.iokit.IOPCIFamily</string>
				<key>Limit</key>
				<integer>0</integer>
				<key>Mask</key>
				<data>/wD//w==</data>
				<key>MaxKernel</key>
				<string>25.99.99</string>
				<key>MinKernel</key>
				<string>22.0.0</string>
				<key>Replace</key>
				<data>AADrAA==</data>
				<key>ReplaceMask</key>
				<data>AAD/AA==</data>
				<key>Skip</key>
				<integer>0</integer>
			</dict>
		</array>
	</dict>
</dict>
</plist>
"#;

    #[test]
    fn parses_synthetic_patches_in_order() {
        let patches = amd_vanilla_patches(SYNTHETIC.as_bytes(), 6).unwrap();
        assert_eq!(patches.len(), 3);

        let core = &patches[0];
        assert_eq!(core.find, "C1E81A0000");
        assert_eq!(core.mask, "FFFDFF0000");
        assert_eq!(core.replace, "BA06000000");
        assert_eq!(core.replace_mask, "FFFFFFFFFF");
        assert_eq!(core.base, "_cpuid_set_info");
        assert_eq!(core.identifier, "kernel");
        assert_eq!(core.arch, "x86_64");
        assert_eq!((core.count, core.limit, core.skip), (1, 0, 0));
        assert_eq!(
            (core.min_kernel.as_str(), core.max_kernel.as_str()),
            ("22.4.0", "25.99.99")
        );
        assert!(core.enabled);

        let family = &patches[1];
        assert_eq!(family.base, "_cpuid_set_info", "trailing space trimmed");
        assert_eq!(family.replace, "BABC4FEA7831DBEB", "not a core-count patch");
        assert_eq!(family.replace_mask, "");

        let hotplug = &patches[2];
        assert!(!hotplug.enabled);
        assert_eq!(hotplug.identifier, "com.apple.iokit.IOPCIFamily");
        assert_eq!(hotplug.replace_mask, "0000FF00");
    }

    #[test]
    fn core_count_is_written_as_one_byte() {
        let p = amd_vanilla_patches(SYNTHETIC.as_bytes(), 16).unwrap();
        assert_eq!(p[0].replace, "BA10000000");
        let p = amd_vanilla_patches(SYNTHETIC.as_bytes(), 255).unwrap();
        assert_eq!(p[0].replace, "BAFF000000");
        let p = amd_vanilla_patches(SYNTHETIC.as_bytes(), 1).unwrap();
        assert_eq!(p[0].replace, "BA01000000");
    }

    #[test]
    fn rejects_invalid_core_counts() {
        for cores in [0, 256, 1024] {
            let err = amd_vanilla_patches(SYNTHETIC.as_bytes(), cores).unwrap_err();
            assert_eq!(err.code, "AMD_CORE_COUNT_INVALID", "{cores}");
        }
    }

    #[test]
    fn rejects_malformed_files() {
        assert_eq!(
            amd_vanilla_patches(b"not a plist", 8).unwrap_err().code,
            "AMD_PATCHES_INVALID"
        );
        let no_patch = SYNTHETIC.replace("<key>Patch</key>", "<key>Block</key>");
        assert_eq!(
            amd_vanilla_patches(no_patch.as_bytes(), 8)
                .unwrap_err()
                .code,
            "AMD_PATCHES_INVALID"
        );
        let no_core = SYNTHETIC.replace("cpuid_cores_per_package", "something_else");
        assert_eq!(
            amd_vanilla_patches(no_core.as_bytes(), 8).unwrap_err().code,
            "AMD_PATCHES_INVALID"
        );
        // Find and Replace of different length.
        let uneven = SYNTHETIC.replace("<data>urxP6ngx2+s=</data>", "<data>urxP6ngx</data>");
        assert_eq!(
            amd_vanilla_patches(uneven.as_bytes(), 8).unwrap_err().code,
            "AMD_PATCHES_INVALID"
        );
        // Wrong value type.
        let bad_type = SYNTHETIC.replacen("<integer>1</integer>", "<string>1</string>", 1);
        assert_eq!(
            amd_vanilla_patches(bad_type.as_bytes(), 8)
                .unwrap_err()
                .code,
            "AMD_PATCHES_INVALID"
        );
    }

    #[test]
    fn pins_point_at_fixed_commits() {
        for (pin, repo) in [
            (AMD_VANILLA, "AMD-OSX/AMD_Vanilla"),
            (AMD_VANILLA_TAHOE, "laobamac/AMD_Vanilla"),
        ] {
            let prefix = format!("https://raw.githubusercontent.com/{repo}/");
            assert!(pin.url.starts_with(&prefix), "{}", pin.url);
            assert!(pin.url.ends_with("/patches.plist"));
            let commit = pin.url.split('/').nth(5).unwrap();
            assert_eq!(commit.len(), 40);
            assert!(commit.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(pin.version.ends_with(&commit[..8]), "{}", pin.version);
            let sha = pin.sha256.unwrap();
            assert_eq!(sha.len(), 64);
            assert!(sha
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
    }

    #[test]
    fn tahoe_uses_the_fork_set() {
        assert_eq!(
            pin_for_target(MacOsVersion::Tahoe).url,
            AMD_VANILLA_TAHOE.url
        );
        for target in MacOsVersion::ALL
            .into_iter()
            .filter(|t| *t < MacOsVersion::Tahoe)
        {
            assert_eq!(pin_for_target(target).url, AMD_VANILLA.url, "{target:?}");
        }
    }

    fn pat_set() -> Vec<BinaryPatch> {
        let patch = |comment: &str, enabled: bool| BinaryPatch {
            comment: comment.to_string(),
            arch: "x86_64".into(),
            identifier: "kernel".into(),
            base: String::new(),
            find: "00".into(),
            mask: String::new(),
            replace: "00".into(),
            replace_mask: String::new(),
            count: 0,
            limit: 0,
            skip: 0,
            min_kernel: String::new(),
            max_kernel: String::new(),
            enabled,
        };
        vec![
            patch("algrey | Remove version check and panic | 10.13+", true),
            patch(
                "CaseySJ | IOPCIIsHotplugPort | Fix PCI bus enumeration on AM5 | 13.0+",
                false,
            ),
            patch("algrey | _mtrr_update_action | fix PAT | 10.13+", true),
            patch("Shaneee | _mtrr_update_action | Fix PAT | 10.13+", false),
            patch(
                "Algrey / Zormeister | _mtrr_update_action | Fix PAT | 15.0+",
                true,
            ),
            patch(
                "Shaneee / Zormeister | _mtrr_update_action | Fix PAT | 15.0+",
                false,
            ),
        ]
    }

    fn enabled(patches: &[BinaryPatch]) -> Vec<bool> {
        patches.iter().map(|p| p.enabled).collect()
    }

    #[test]
    fn pat_variant_selection() {
        let mut p = pat_set();
        assert_eq!(select_pat_patch(&mut p, PatPatch::Shaneee), 4);
        assert_eq!(enabled(&p), [true, false, false, true, false, true]);
        select_pat_patch(&mut p, PatPatch::Algrey);
        assert_eq!(enabled(&p), [true, false, true, false, true, false]);
        select_pat_patch(&mut p, PatPatch::Disabled);
        assert_eq!(enabled(&p), [true, false, false, false, false, false]);
        assert_eq!(select_pat_patch(&mut p[..2], PatPatch::Algrey), 0);
    }

    #[test]
    fn hotplug_port_fix_toggle() {
        let mut p = pat_set();
        assert!(set_hotplug_port_fix(&mut p, true));
        assert!(p[1].enabled);
        assert!(set_hotplug_port_fix(&mut p, false));
        assert!(!p[1].enabled);
        assert!(!set_hotplug_port_fix(&mut p[2..], true));
    }

    async fn fetch_pinned(pin: Pin) -> Vec<u8> {
        use sha2::{Digest, Sha256};

        let bytes = reqwest::get(pin.url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(Some(digest.as_str()), pin.sha256, "{}", pin.url);
        bytes.to_vec()
    }

    /// Invariants every AMD_Vanilla set must hold; returns the patches.
    fn check_upstream_set(bytes: &[u8], expected: usize) -> Vec<BinaryPatch> {
        let patches = amd_vanilla_patches(bytes, 8).unwrap();
        assert_eq!(patches.len(), expected);
        let core: Vec<_> = patches
            .iter()
            .filter(|p| p.comment.contains(CORE_COUNT_MARKER))
            .collect();
        let replaces: Vec<_> = core.iter().map(|p| p.replace.as_str()).collect();
        assert_eq!(
            replaces,
            ["B80800000000", "BA0800000000", "BA0800000090", "BA08000000"]
        );
        let ranges: Vec<_> = core
            .iter()
            .map(|p| (p.min_kernel.as_str(), p.max_kernel.as_str()))
            .collect();
        assert_eq!(
            ranges,
            [
                ("17.0.0", "18.99.99"),
                ("19.0.0", "20.99.99"),
                ("21.0.0", "22.3.99"),
                ("22.4.0", "25.99.99")
            ]
        );
        for p in &patches {
            assert_eq!(p.arch, "x86_64");
            assert_eq!(p.base, p.base.trim());
            assert!(
                p.find.is_empty() || p.find.len() == p.replace.len(),
                "{}",
                p.comment
            );
            assert_eq!((p.limit, p.skip), (0, 0), "{}", p.comment);
        }
        let leaf7 = patches
            .iter()
            .find(|p| p.comment.contains("allow leaf7 | 15.0+"))
            .unwrap();
        assert_eq!(
            (leaf7.find.as_str(), leaf7.replace.as_str()),
            ("00050F82", "00000F82")
        );
        let hotplug = patches
            .iter()
            .find(|p| p.comment.contains(HOTPLUG_MARKER))
            .unwrap();
        assert!(!hotplug.enabled);
        let pat: Vec<_> = patches
            .iter()
            .filter(|p| p.comment.contains(PAT_MARKER))
            .collect();
        assert_eq!(pat.len(), 4);
        assert_eq!(pat.iter().filter(|p| p.enabled).count(), 2);
        assert!(pat
            .iter()
            .filter(|p| p.comment.starts_with("Shaneee"))
            .all(|p| !p.enabled));
        patches
    }

    /// Downloads both pinned files and checks hash and content against the
    /// upstream tables (research-amd.md section 3).
    #[tokio::test]
    #[ignore = "needs network access"]
    async fn pinned_upstream_files_match() {
        let master = check_upstream_set(&fetch_pinned(AMD_VANILLA).await, 25);
        assert!(!master.iter().any(|p| p.comment.contains("26.0+")));

        let tahoe = check_upstream_set(&fetch_pinned(AMD_VANILLA_TAHOE).await, 27);
        let tags: Vec<_> = tahoe
            .iter()
            .filter(|p| p.comment.contains("Disable 10 bit tags"))
            .map(|p| {
                (
                    p.find.as_str(),
                    p.min_kernel.as_str(),
                    p.max_kernel.as_str(),
                )
            })
            .collect();
        assert_eq!(
            tags,
            [
                ("E0117200", "21.0.0", "24.99.99"),
                ("E0117340", "25.0.0", "25.99.99")
            ]
        );
        let monotonic: Vec<_> = tahoe
            .iter()
            .filter(|p| p.comment.contains("thread_invoke, thread_dispatch"))
            .map(|p| {
                (
                    p.find.as_str(),
                    p.min_kernel.as_str(),
                    p.max_kernel.as_str(),
                )
            })
            .collect();
        assert_eq!(
            monotonic,
            [
                ("480000800400000F0000000000", "21.0.0", "25.3.99"),
                ("480000900400000F0000000000", "25.4.0", "25.99.99")
            ]
        );

        // Before macOS 26 both sets apply exactly the same patches.
        for darwin in 17..=24 {
            assert_eq!(
                applied(&master, darwin),
                applied(&tahoe, darwin),
                "Darwin {darwin}"
            );
        }
        assert_ne!(applied(&master, 25), applied(&tahoe, 25));
    }

    /// Enabled patches whose kernel range covers `darwin`.0.0 and .99.99,
    /// without their comments.
    fn applied(patches: &[BinaryPatch], darwin: u32) -> Vec<String> {
        let covers = |p: &BinaryPatch, minor: u32| {
            let version = darwin * 10_000 + minor * 100 + minor;
            let parse = |v: &str| {
                let mut n = v.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
                n.next().unwrap_or(0) * 10_000 + n.next().unwrap_or(0) * 100 + n.next().unwrap_or(0)
            };
            (p.min_kernel.is_empty() || parse(&p.min_kernel) <= version)
                && (p.max_kernel.is_empty() || parse(&p.max_kernel) >= version)
        };
        patches
            .iter()
            .filter(|p| p.enabled && (covers(p, 0) || covers(p, 99)))
            .map(|p| {
                format!(
                    "{} {} {} {} {} {} {}",
                    p.identifier, p.base, p.find, p.mask, p.replace, p.replace_mask, p.count
                )
            })
            .collect()
    }
}
