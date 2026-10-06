//! Kernel->Add entry generation from the kexts actually placed in EFI/OC/Kexts:
//! reads each bundle's Info.plist (CFBundleIdentifier, CFBundleExecutable,
//! OSBundleLibraries), computes ExecutablePath ("" for codeless kexts), adds
//! one entry per selected plugin, keeps exactly one enabled VoodooInput (and
//! one enabled copy of any other bundle id per kernel range), and orders
//! entries so every dependency loads first (stable topological sort; Lilu
//! first, VirtualSMC second, selection order as the tie-break).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::Path;

use serde_json::json;

use crate::domain::model::KextSelection;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelAddEntry {
    pub arch: String,
    pub bundle_path: String,
    pub comment: String,
    pub enabled: bool,
    pub executable_path: String,
    pub max_kernel: String,
    pub min_kernel: String,
    pub plist_path: String,
    /// CFBundleIdentifier (for diagnostics / dependency checks).
    pub bundle_id: String,
}

const LILU_ID: &str = "as.vit9696.Lilu";
const VIRTUALSMC_ID: &str = "as.vit9696.VirtualSMC";
const VOODOO_INPUT_ID: &str = "me.kishorprins.VoodooInput";
const PLIST_PATH: &str = "Contents/Info.plist";

/// Ordering constraints that are not expressed in OSBundleLibraries:
/// (bundle id, must load after bundle id). AppleALC goes after NootedRed,
/// the order OpCore-Simplify enforces with an artificial dependency.
const LOAD_AFTER: &[(&str, &str)] =
    &[("as.vit9696.AppleALC", "org.ChefKiss.NootedRed"), ("as.vit9696.AppleALCU", "org.ChefKiss.NootedRed")];

/// What a bundle's Info.plist says about it.
#[derive(Debug, Clone)]
struct BundleInfo {
    id: String,
    executable_path: String,
    libraries: Vec<String>,
}

/// Kernel->Add entries for `selections`, whose bundles must already be in
/// `kexts_dir` (EFI/OC/Kexts). Fails on missing bundles or plugins, unreadable
/// Info.plists and dependency cycles. A disabled bundle or plugin that is not
/// on disk is left out instead, since it would never load anyway.
pub fn build_kernel_add(selections: &[KextSelection], kexts_dir: &Path) -> Result<Vec<KernelAddEntry>, AppError> {
    let mut entries: Vec<(KernelAddEntry, Vec<String>)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |entry: KernelAddEntry, libs: Vec<String>| {
        if seen.insert(entry.bundle_path.to_ascii_lowercase()) {
            entries.push((entry, libs));
        } else {
            tracing::warn!(bundle = %entry.bundle_path, "kext selected twice, keeping the first entry");
        }
    };

    for sel in selections {
        check_bundle_name(&sel.bundle)?;
        // BundlePath must match the disk; FAT32 forgives case, ext4 does not.
        let bundle = on_disk_name(kexts_dir, &sel.bundle).unwrap_or_else(|| sel.bundle.clone());
        let bundle_dir = kexts_dir.join(&bundle);
        if !bundle_dir.is_dir() {
            if !sel.enabled {
                tracing::warn!(bundle = %sel.bundle, "disabled kext is not in EFI/OC/Kexts, leaving it out");
                continue;
            }
            return Err(AppError::new("KEXT_BUNDLE_MISSING", format!("{} is not in EFI/OC/Kexts", sel.bundle))
                .with_context(json!({ "catalogId": sel.catalog_id, "bundle": sel.bundle })));
        }
        let info = read_bundle(&bundle_dir, &bundle)?;
        push(
            entry(&bundle, &bundle, &info, sel.enabled, sel.min_kernel.as_deref(), sel.max_kernel.as_deref()),
            info.libraries,
        );
        let plugins_dir = bundle_dir.join("Contents").join("PlugIns");
        for plugin in &sel.plugins {
            check_bundle_name(&plugin.bundle)?;
            let name = on_disk_name(&plugins_dir, &plugin.bundle).unwrap_or_else(|| plugin.bundle.clone());
            let rel = format!("{bundle}/Contents/PlugIns/{name}");
            let dir = plugins_dir.join(&name);
            let enabled = sel.enabled && plugin.enabled;
            if !dir.is_dir() {
                if !enabled {
                    tracing::warn!(plugin = %rel, "disabled plugin does not exist, leaving it out");
                    continue;
                }
                return Err(AppError::new(
                    "KEXT_PLUGIN_MISSING",
                    format!("{} has no plugin {}", sel.bundle, plugin.bundle),
                )
                .with_context(json!({ "catalogId": sel.catalog_id, "bundle": sel.bundle, "plugin": plugin.bundle })));
            }
            let info = read_bundle(&dir, &rel)?;
            // A plugin cannot load without its parent, so its kernel range is
            // the parent's, narrowed further by the planner's own bounds.
            let min = tighter_bound(plugin.min_kernel.as_deref(), sel.min_kernel.as_deref(), true);
            let max = tighter_bound(plugin.max_kernel.as_deref(), sel.max_kernel.as_deref(), false);
            push(entry(&rel, &name, &info, enabled, min, max), info.libraries);
        }
    }

    keep_one_voodoo_input(&mut entries);
    disable_duplicate_ids(&mut entries);
    order(entries)
}

fn entry(
    bundle_path: &str,
    name: &str,
    info: &BundleInfo,
    enabled: bool,
    min: Option<&str>,
    max: Option<&str>,
) -> KernelAddEntry {
    KernelAddEntry {
        arch: "Any".to_string(),
        bundle_path: bundle_path.to_string(),
        comment: name.to_string(),
        enabled,
        executable_path: info.executable_path.clone(),
        max_kernel: max.unwrap_or_default().trim().to_string(),
        min_kernel: min.unwrap_or_default().trim().to_string(),
        plist_path: PLIST_PATH.to_string(),
        bundle_id: info.id.clone(),
    }
}

/// Spelling of `name` inside `dir` on disk: the exact name when present,
/// otherwise the one entry that matches it ignoring ASCII case.
fn on_disk_name(dir: &Path, name: &str) -> Option<String> {
    let names: Vec<String> =
        std::fs::read_dir(dir).ok()?.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).collect();
    if names.iter().any(|n| n == name) {
        return Some(name.to_string());
    }
    let mut matches = names.into_iter().filter(|n| n.eq_ignore_ascii_case(name));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// Bundle and plugin names must be single `.kext` path components.
fn check_bundle_name(name: &str) -> Result<(), AppError> {
    let b = name.as_bytes();
    let ok = b.len() > 5
        && b[b.len() - 5..].eq_ignore_ascii_case(b".kext")
        && !name.contains(['/', '\\', ':', '\0'])
        && !name.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(AppError::new("INVALID_BUNDLE_NAME", format!("'{name}' is not a .kext bundle name")))
    }
}

/// Read `Contents/Info.plist` (XML or binary). ExecutablePath is set only
/// when `Contents/MacOS/<CFBundleExecutable>` exists and is not empty, which
/// makes codeless injectors (and stripped plugins) come out as "".
fn read_bundle(dir: &Path, label: &str) -> Result<BundleInfo, AppError> {
    let plist_path = dir.join("Contents").join("Info.plist");
    let invalid = |why: String| {
        AppError::new("KEXT_INVALID_BUNDLE", format!("{label}: {why}")).with_context(json!({ "bundle": label }))
    };
    if !plist_path.is_file() {
        return Err(invalid("Contents/Info.plist is missing".into()));
    }
    let value =
        plist::Value::from_file(&plist_path).map_err(|e| invalid(format!("Info.plist cannot be parsed ({e})")))?;
    let dict = value.as_dictionary().ok_or_else(|| invalid("Info.plist is not a dictionary".into()))?;
    let id = dict
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("Info.plist has no CFBundleIdentifier".into()))?
        .to_string();

    let executable_path = match dict.get("CFBundleExecutable").and_then(plist::Value::as_string).map(str::trim) {
        Some(exe) if !exe.is_empty() && !exe.contains(['/', '\\']) && exe != ".." => {
            let bin = dir.join("Contents").join("MacOS").join(exe);
            match std::fs::metadata(&bin) {
                Ok(m) if m.is_file() && m.len() > 0 => format!("Contents/MacOS/{exe}"),
                _ => {
                    tracing::warn!(
                        bundle = label,
                        executable = exe,
                        "declared executable is missing; treating the kext as codeless"
                    );
                    String::new()
                }
            }
        }
        _ => String::new(),
    };

    let libraries = dict
        .get("OSBundleLibraries")
        .and_then(plist::Value::as_dictionary)
        .map(|libs| libs.keys().map(|k| k.to_string()).collect())
        .unwrap_or_default();
    Ok(BundleInfo { id, executable_path, libraries })
}

/// VoodooInput ships inside VoodooPS2Controller, VoodooI2C and VoodooRMI;
/// only one copy may load. Preference: VoodooRMI's (newest), VoodooI2C's,
/// the standalone bundle, VoodooPS2's.
fn keep_one_voodoo_input(entries: &mut [(KernelAddEntry, Vec<String>)]) {
    let rank = |path: &str| {
        let p = path.to_ascii_lowercase();
        if p.starts_with("voodoormi.kext/") {
            0
        } else if p.starts_with("voodooi2c.kext/") {
            1
        } else if p == "voodooinput.kext" {
            2
        } else if p.starts_with("voodoops2controller.kext/") {
            3
        } else {
            4
        }
    };
    let enabled: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, (e, _))| e.enabled && e.bundle_id.eq_ignore_ascii_case(VOODOO_INPUT_ID))
        .map(|(i, _)| i)
        .collect();
    let Some(&keep) = enabled.iter().min_by_key(|&&i| (rank(&entries[i].0.bundle_path), i)) else { return };
    for &i in &enabled {
        if i != keep {
            tracing::info!(disabled = %entries[i].0.bundle_path, kept = %entries[keep].0.bundle_path, "only one VoodooInput may load");
            entries[i].0.enabled = false;
        }
    }
}

/// A bundle id may only load once per kernel. An enabled entry whose id and
/// kernel range clash with an earlier enabled entry is disabled: the first
/// one selected wins (disjoint ranges, e.g. MacHyperVSupport and
/// MacHyperVSupportMonterey, are fine).
fn disable_duplicate_ids(entries: &mut [(KernelAddEntry, Vec<String>)]) {
    for i in 0..entries.len() {
        if !entries[i].0.enabled {
            continue;
        }
        let clash = (0..i).find(|&j| {
            let (a, b) = (&entries[j].0, &entries[i].0);
            a.enabled && a.bundle_id.eq_ignore_ascii_case(&b.bundle_id) && ranges_overlap(a, b)
        });
        if let Some(j) = clash {
            tracing::warn!(
                disabled = %entries[i].0.bundle_path,
                kept = %entries[j].0.bundle_path,
                bundle_id = %entries[i].0.bundle_id,
                "bundle id selected twice for the same kernels"
            );
            entries[i].0.enabled = false;
        }
    }
}

/// OpenCore's Darwin version encoding (`A*10000 + B*100 + C`); empty or
/// unparsable bounds are open.
fn kernel_bound(s: &str) -> Option<u32> {
    let mut it = s.trim().split('.').map(|p| p.trim().parse::<u32>().ok());
    let major = it.next().flatten()?;
    let minor = it.next().flatten().unwrap_or(0).min(99);
    let patch = it.next().flatten().unwrap_or(0).min(99);
    Some(major.saturating_mul(10_000).saturating_add(minor * 100 + patch))
}

/// The stricter of a plugin's and its parent's bound: the later MinKernel
/// (`later` true) or the earlier MaxKernel. Empty bounds are open; a bound
/// that does not parse is kept as given (ocvalidate reports it).
fn tighter_bound<'a>(own: Option<&'a str>, parent: Option<&'a str>, later: bool) -> Option<&'a str> {
    let own = own.map(str::trim).filter(|s| !s.is_empty());
    let parent = parent.map(str::trim).filter(|s| !s.is_empty());
    match (own, parent) {
        (Some(o), Some(p)) => match (kernel_bound(o), kernel_bound(p)) {
            (Some(vo), Some(vp)) if (vp > vo) == later => Some(p),
            _ => Some(o),
        },
        (o, p) => o.or(p),
    }
}

fn ranges_overlap(a: &KernelAddEntry, b: &KernelAddEntry) -> bool {
    let range = |e: &KernelAddEntry| {
        (kernel_bound(&e.min_kernel).unwrap_or(0), kernel_bound(&e.max_kernel).unwrap_or(u32::MAX))
    };
    let ((amin, amax), (bmin, bmax)) = (range(a), range(b));
    amin <= bmax && bmin <= amax
}

/// Stable topological sort over OSBundleLibraries (bundle ids compared
/// case-insensitively). Among entries whose dependencies are satisfied,
/// Lilu goes first, then VirtualSMC, then the earliest in selection order.
fn order(entries: Vec<(KernelAddEntry, Vec<String>)>) -> Result<Vec<KernelAddEntry>, AppError> {
    let n = entries.len();
    let mut providers: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (e, _)) in entries.iter().enumerate() {
        providers.entry(e.bundle_id.to_ascii_lowercase()).or_default().push(i);
    }
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut pending = vec![0usize; n];
    for (i, (e, libs)) in entries.iter().enumerate() {
        let extra = LOAD_AFTER
            .iter()
            .filter(|(id, _)| e.bundle_id.eq_ignore_ascii_case(id))
            .map(|(_, after)| after.to_string());
        let mut deps: HashSet<usize> = HashSet::new();
        for lib in libs.iter().cloned().chain(extra) {
            if let Some(ps) = providers.get(&lib.to_ascii_lowercase()) {
                deps.extend(ps.iter().copied().filter(|&p| p != i));
            }
        }
        pending[i] = deps.len();
        for p in deps {
            dependents[p].push(i);
        }
    }
    let priority = |e: &KernelAddEntry| {
        if e.bundle_id.eq_ignore_ascii_case(LILU_ID) {
            0u8
        } else if e.bundle_id.eq_ignore_ascii_case(VIRTUALSMC_ID) {
            1
        } else {
            2
        }
    };
    let mut ready: BinaryHeap<Reverse<(u8, usize)>> =
        (0..n).filter(|&i| pending[i] == 0).map(|i| Reverse((priority(&entries[i].0), i))).collect();
    let mut sorted = Vec::with_capacity(n);
    while let Some(Reverse((_, i))) = ready.pop() {
        sorted.push(i);
        for &d in &dependents[i] {
            pending[d] -= 1;
            if pending[d] == 0 {
                ready.push(Reverse((priority(&entries[d].0), d)));
            }
        }
    }
    if sorted.len() < n {
        let placed: HashSet<usize> = sorted.iter().copied().collect();
        let cycle: Vec<String> =
            (0..n).filter(|i| !placed.contains(i)).map(|i| entries[i].0.bundle_path.clone()).collect();
        return Err(AppError::new(
            "KEXT_DEPENDENCY_CYCLE",
            format!("Kext dependencies form a cycle: {}", cycle.join(", ")),
        )
        .with_context(json!({ "bundles": cycle })));
    }
    let mut slots: Vec<Option<KernelAddEntry>> = entries.into_iter().map(|(e, _)| Some(e)).collect();
    Ok(sorted.into_iter().filter_map(|i| slots[i].take()).collect())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::model::PluginSelection;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-kadd-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Create `<root>/<rel>` as a kext with the given id, executable and deps.
    fn bundle(root: &Path, rel: &str, id: &str, exe: Option<&str>, libs: &[&str], binary_plist: bool) {
        let dir = root.join(rel);
        std::fs::create_dir_all(dir.join("Contents")).unwrap();
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), id.into());
        if let Some(e) = exe {
            d.insert("CFBundleExecutable".into(), e.into());
            std::fs::create_dir_all(dir.join("Contents/MacOS")).unwrap();
            std::fs::write(dir.join("Contents/MacOS").join(e), b"\xcf\xfa\xed\xfe").unwrap();
        }
        let mut l = plist::Dictionary::new();
        for lib in libs {
            l.insert((*lib).into(), "1.0.0".into());
        }
        d.insert("OSBundleLibraries".into(), plist::Value::Dictionary(l));
        let path = dir.join("Contents/Info.plist");
        let v = plist::Value::Dictionary(d);
        if binary_plist {
            v.to_file_binary(&path).unwrap();
        } else {
            v.to_file_xml(&path).unwrap();
        }
    }

    fn simple(root: &Path, name: &str, id: &str, libs: &[&str]) {
        let exe = name.trim_end_matches(".kext");
        bundle(root, name, id, Some(exe), libs, false);
    }

    fn sel(catalog: &str, bundle: &str, plugins: &[(&str, bool)]) -> KextSelection {
        KextSelection {
            catalog_id: catalog.into(),
            bundle: bundle.into(),
            plugins: plugins
                .iter()
                .map(|(b, en)| PluginSelection {
                    bundle: (*b).into(),
                    enabled: *en,
                    min_kernel: None,
                    max_kernel: None,
                })
                .collect(),
            enabled: true,
            min_kernel: None,
            max_kernel: None,
            required: true,
            reason: String::new(),
        }
    }

    fn paths(entries: &[KernelAddEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.bundle_path.as_str()).collect()
    }

    fn pos(entries: &[KernelAddEntry], path: &str) -> usize {
        entries.iter().position(|e| e.bundle_path == path).unwrap_or_else(|| panic!("{path} missing"))
    }

    fn lilu_stack(k: &Path) {
        simple(k, "Lilu.kext", "as.vit9696.Lilu", &["com.apple.kpi.bsd", "com.apple.kernel.6.0"]);
        simple(k, "VirtualSMC.kext", "as.vit9696.VirtualSMC", &["as.vit9696.Lilu", "com.apple.iokit.IOACPIFamily"]);
        simple(k, "SMCProcessor.kext", "as.vit9696.SMCProcessor", &["as.vit9696.Lilu", "as.vit9696.VirtualSMC"]);
        simple(k, "SMCSuperIO.kext", "ru.joedm.SMCSuperIO", &["AS.VIT9696.LILU", "as.vit9696.virtualsmc"]);
        simple(k, "WhateverGreen.kext", "as.vit9696.WhateverGreen", &["as.vit9696.Lilu"]);
    }

    #[test]
    fn lilu_first_virtualsmc_second_dependencies_before_dependents() {
        let tmp = TempDir::new();
        lilu_stack(&tmp.0);
        let selections = vec![
            sel("WhateverGreen", "WhateverGreen.kext", &[]),
            sel("VirtualSMC", "SMCSuperIO.kext", &[]),
            sel("VirtualSMC", "SMCProcessor.kext", &[]),
            sel("VirtualSMC", "VirtualSMC.kext", &[]),
            sel("Lilu", "Lilu.kext", &[]),
        ];
        let out = build_kernel_add(&selections, &tmp.0).unwrap();
        assert_eq!(
            paths(&out),
            vec!["Lilu.kext", "VirtualSMC.kext", "WhateverGreen.kext", "SMCSuperIO.kext", "SMCProcessor.kext"]
        );
        let lilu = &out[0];
        assert_eq!(lilu.executable_path, "Contents/MacOS/Lilu");
        assert_eq!(lilu.plist_path, "Contents/Info.plist");
        assert_eq!(lilu.arch, "Any");
        assert_eq!(lilu.bundle_id, "as.vit9696.Lilu");
        assert!(lilu.enabled);
    }

    #[test]
    fn already_valid_order_is_kept() {
        let tmp = TempDir::new();
        lilu_stack(&tmp.0);
        let selections = vec![
            sel("Lilu", "Lilu.kext", &[]),
            sel("VirtualSMC", "VirtualSMC.kext", &[]),
            sel("VirtualSMC", "SMCProcessor.kext", &[]),
            sel("WhateverGreen", "WhateverGreen.kext", &[]),
            sel("VirtualSMC", "SMCSuperIO.kext", &[]),
        ];
        let out = build_kernel_add(&selections, &tmp.0).unwrap();
        assert_eq!(
            paths(&out),
            vec!["Lilu.kext", "VirtualSMC.kext", "SMCProcessor.kext", "WhateverGreen.kext", "SMCSuperIO.kext"]
        );
    }

    fn input_stack(k: &Path) {
        let ps2 = "VoodooPS2Controller.kext";
        simple(k, ps2, "as.acidanthera.voodoo.driver.PS2Controller", &["com.apple.iokit.IOACPIFamily"]);
        for (p, id) in [
            ("VoodooPS2Keyboard", "as.acidanthera.voodoo.driver.PS2Keyboard"),
            ("VoodooPS2Trackpad", "as.acidanthera.voodoo.driver.PS2Trackpad"),
            ("VoodooPS2Mouse", "as.acidanthera.voodoo.driver.PS2Mouse"),
        ] {
            bundle(
                k,
                &format!("{ps2}/Contents/PlugIns/{p}.kext"),
                id,
                Some(p),
                &["as.acidanthera.voodoo.driver.PS2Controller"],
                false,
            );
        }
        bundle(
            k,
            &format!("{ps2}/Contents/PlugIns/VoodooInput.kext"),
            VOODOO_INPUT_ID,
            Some("VoodooInput"),
            &[],
            false,
        );

        let i2c = "VoodooI2C.kext";
        simple(k, i2c, "com.alexandred.VoodooI2C", &["com.alexandred.VoodooI2CServices", "org.coolstar.VoodooGPIO"]);
        bundle(
            k,
            &format!("{i2c}/Contents/PlugIns/VoodooGPIO.kext"),
            "org.coolstar.VoodooGPIO",
            Some("VoodooGPIO"),
            &[],
            false,
        );
        bundle(
            k,
            &format!("{i2c}/Contents/PlugIns/VoodooI2CServices.kext"),
            "com.alexandred.VoodooI2CServices",
            Some("VoodooI2CServices"),
            &[],
            true,
        );
        bundle(
            k,
            &format!("{i2c}/Contents/PlugIns/VoodooInput.kext"),
            VOODOO_INPUT_ID,
            Some("VoodooInput"),
            &[],
            false,
        );
        simple(k, "VoodooI2CHID.kext", "com.alexandred.VoodooI2CHID", &["com.alexandred.VoodooI2C"]);

        let rmi = "VoodooRMI.kext";
        simple(k, rmi, "com.1Revenger1.VoodooRMI", &[]);
        bundle(
            k,
            &format!("{rmi}/Contents/PlugIns/VoodooInput.kext"),
            VOODOO_INPUT_ID,
            Some("VoodooInput"),
            &[],
            false,
        );
        bundle(
            k,
            &format!("{rmi}/Contents/PlugIns/RMII2C.kext"),
            "com.1Revenger1.RMII2C",
            Some("RMII2C"),
            &["com.1Revenger1.VoodooRMI", "com.alexandred.VoodooI2C"],
            false,
        );
        bundle(
            k,
            &format!("{rmi}/Contents/PlugIns/RMISMBus.kext"),
            "com.1Revenger1.RMISMBus",
            Some("RMISMBus"),
            &["com.1Revenger1.VoodooRMI", "de.leo-labs.VoodooSMBus"],
            false,
        );
        simple(k, "VoodooSMBus.kext", "de.leo-labs.VoodooSMBus", &[]);
    }

    #[test]
    fn input_plugins_get_entries_and_one_voodoo_input() {
        let tmp = TempDir::new();
        input_stack(&tmp.0);
        let selections = vec![
            sel(
                "VoodooPS2Controller",
                "VoodooPS2Controller.kext",
                &[
                    ("VoodooInput.kext", true),
                    ("VoodooPS2Keyboard.kext", true),
                    ("VoodooPS2Trackpad.kext", true),
                    ("VoodooPS2Mouse.kext", true),
                ],
            ),
            sel(
                "VoodooI2C",
                "VoodooI2C.kext",
                &[("VoodooGPIO.kext", true), ("VoodooI2CServices.kext", true), ("VoodooInput.kext", true)],
            ),
            sel("VoodooI2C", "VoodooI2CHID.kext", &[]),
            sel(
                "VoodooRMI",
                "VoodooRMI.kext",
                &[("VoodooInput.kext", true), ("RMII2C.kext", true), ("RMISMBus.kext", false)],
            ),
        ];
        let out = build_kernel_add(&selections, &tmp.0).unwrap();
        assert_eq!(out.len(), 14);

        let vi: Vec<&KernelAddEntry> = out.iter().filter(|e| e.bundle_id == VOODOO_INPUT_ID).collect();
        assert_eq!(vi.len(), 3);
        let enabled: Vec<&str> = vi.iter().filter(|e| e.enabled).map(|e| e.bundle_path.as_str()).collect();
        assert_eq!(enabled, vec!["VoodooRMI.kext/Contents/PlugIns/VoodooInput.kext"]);

        let i2c = pos(&out, "VoodooI2C.kext");
        assert!(pos(&out, "VoodooI2C.kext/Contents/PlugIns/VoodooGPIO.kext") < i2c);
        assert!(pos(&out, "VoodooI2C.kext/Contents/PlugIns/VoodooI2CServices.kext") < i2c);
        assert!(pos(&out, "VoodooI2CHID.kext") > i2c);
        let rmii2c = pos(&out, "VoodooRMI.kext/Contents/PlugIns/RMII2C.kext");
        assert!(rmii2c > i2c && rmii2c > pos(&out, "VoodooRMI.kext"));
        let kb = pos(&out, "VoodooPS2Controller.kext/Contents/PlugIns/VoodooPS2Keyboard.kext");
        assert!(kb > pos(&out, "VoodooPS2Controller.kext"));
        assert_eq!(out[0].bundle_path, "VoodooPS2Controller.kext");

        // RMISMBus was deselected: listed, but disabled. Its dependency
        // VoodooSMBus was not selected, so it simply comes after VoodooRMI.
        let smbus = &out[pos(&out, "VoodooRMI.kext/Contents/PlugIns/RMISMBus.kext")];
        assert!(!smbus.enabled);
        assert_eq!(smbus.comment, "RMISMBus.kext");
        // Binary Info.plist is read too.
        assert_eq!(
            out[pos(&out, "VoodooI2C.kext/Contents/PlugIns/VoodooI2CServices.kext")].executable_path,
            "Contents/MacOS/VoodooI2CServices"
        );
    }

    #[test]
    fn i2c_voodoo_input_beats_ps2() {
        let tmp = TempDir::new();
        input_stack(&tmp.0);
        let selections = vec![
            sel(
                "VoodooPS2Controller",
                "VoodooPS2Controller.kext",
                &[("VoodooInput.kext", true), ("VoodooPS2Keyboard.kext", true)],
            ),
            sel(
                "VoodooI2C",
                "VoodooI2C.kext",
                &[("VoodooGPIO.kext", true), ("VoodooI2CServices.kext", true), ("VoodooInput.kext", true)],
            ),
        ];
        let out = build_kernel_add(&selections, &tmp.0).unwrap();
        let enabled: Vec<&str> = out
            .iter()
            .filter(|e| e.enabled && e.bundle_id == VOODOO_INPUT_ID)
            .map(|e| e.bundle_path.as_str())
            .collect();
        assert_eq!(enabled, vec!["VoodooI2C.kext/Contents/PlugIns/VoodooInput.kext"]);

        // PS2 alone keeps its own copy.
        let out = build_kernel_add(&selections[..1], &tmp.0).unwrap();
        assert!(out.iter().any(|e| e.enabled && e.bundle_id == VOODOO_INPUT_ID));
    }

    #[test]
    fn codeless_and_missing_executables_get_empty_path() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "USBToolBox.kext", "com.dhinakg.USBToolBox.kext", &[]);
        bundle(k, "UTBDefault.kext", "com.dhinakg.USBToolBox.injector", None, &["com.dhinakg.USBToolBox.kext"], false);
        // Declares an executable that is not shipped.
        bundle(k, "Broken.kext", "org.example.Broken", None, &[], false);
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), "org.example.Broken".into());
        d.insert("CFBundleExecutable".into(), "Broken".into());
        plist::Value::Dictionary(d).to_file_xml(k.join("Broken.kext/Contents/Info.plist")).unwrap();
        // Real executable named differently from the bundle.
        bundle(k, "AAAMouSSE.kext", "org.example.MouSSE", Some("MouSSE"), &[], false);

        let selections = vec![
            sel("UTBDefault", "UTBDefault.kext", &[]),
            sel("USBToolBox", "USBToolBox.kext", &[]),
            sel("Broken", "Broken.kext", &[]),
            sel("MouSSE", "AAAMouSSE.kext", &[]),
        ];
        let out = build_kernel_add(&selections, k).unwrap();
        assert_eq!(paths(&out)[..2], ["USBToolBox.kext", "UTBDefault.kext"]);
        assert_eq!(out[pos(&out, "UTBDefault.kext")].executable_path, "");
        assert_eq!(out[pos(&out, "Broken.kext")].executable_path, "");
        assert_eq!(out[pos(&out, "AAAMouSSE.kext")].executable_path, "Contents/MacOS/MouSSE");
    }

    #[test]
    fn kernel_ranges_come_from_selection_and_plugins_inherit() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "Lilu.kext", "as.vit9696.Lilu", &[]);
        simple(k, "AirportBrcmFixup.kext", "as.lvs1974.AirportBrcmFixup", &["as.vit9696.Lilu"]);
        bundle(
            k,
            "AirportBrcmFixup.kext/Contents/PlugIns/AirPortBrcm4360_Injector.kext",
            "as.lvs1974.AirportBrcm4360Injector",
            None,
            &[],
            false,
        );
        bundle(
            k,
            "AirportBrcmFixup.kext/Contents/PlugIns/AirPortBrcmNIC_Injector.kext",
            "as.lvs1974.AirportBrcmNICInjector",
            None,
            &[],
            false,
        );
        let mut fixup = sel("AirportBrcmFixup", "AirportBrcmFixup.kext", &[]);
        fixup.min_kernel = Some("17.0.0".into());
        fixup.plugins = vec![
            PluginSelection {
                bundle: "AirPortBrcm4360_Injector.kext".into(),
                enabled: false,
                min_kernel: None,
                max_kernel: Some("19.99.99".into()),
            },
            PluginSelection {
                bundle: "AirPortBrcmNIC_Injector.kext".into(),
                enabled: true,
                min_kernel: Some("20.0.0".into()),
                max_kernel: None,
            },
        ];
        let out = build_kernel_add(&[sel("Lilu", "Lilu.kext", &[]), fixup], k).unwrap();
        let b4360 = &out[pos(&out, "AirportBrcmFixup.kext/Contents/PlugIns/AirPortBrcm4360_Injector.kext")];
        assert_eq!(
            (b4360.min_kernel.as_str(), b4360.max_kernel.as_str(), b4360.enabled),
            ("17.0.0", "19.99.99", false)
        );
        let nic = &out[pos(&out, "AirportBrcmFixup.kext/Contents/PlugIns/AirPortBrcmNIC_Injector.kext")];
        assert_eq!((nic.min_kernel.as_str(), nic.max_kernel.as_str(), nic.enabled), ("20.0.0", "", true));
        assert_eq!(nic.executable_path, "");

        // A disabled parent disables its plugins.
        let mut off = sel("AirportBrcmFixup", "AirportBrcmFixup.kext", &[("AirPortBrcmNIC_Injector.kext", true)]);
        off.enabled = false;
        let out = build_kernel_add(&[off], k).unwrap();
        assert!(out.iter().all(|e| !e.enabled));
    }

    #[test]
    fn plugin_ranges_never_exceed_the_parent() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "IO80211FamilyLegacy.kext", "com.apple.iokit.IO80211FamilyLegacy", &[]);
        bundle(
            k,
            "IO80211FamilyLegacy.kext/Contents/PlugIns/AirPortBrcmNIC.kext",
            "com.apple.driver.AirPort.BrcmNIC",
            Some("AirPortBrcmNIC"),
            &["com.apple.iokit.IO80211FamilyLegacy"],
            false,
        );
        let mut legacy = sel("IO80211FamilyLegacy", "IO80211FamilyLegacy.kext", &[]);
        legacy.min_kernel = Some("23.0.0".into());
        legacy.max_kernel = Some("25.99.99".into());
        legacy.plugins = vec![PluginSelection {
            bundle: "AirPortBrcmNIC.kext".into(),
            enabled: true,
            min_kernel: Some("20.0.0".into()),
            max_kernel: Some("24.99.99".into()),
        }];
        let out = build_kernel_add(&[legacy], k).unwrap();
        let nic = &out[pos(&out, "IO80211FamilyLegacy.kext/Contents/PlugIns/AirPortBrcmNIC.kext")];
        // Min comes from the parent (later), Max from the plugin (earlier).
        assert_eq!((nic.min_kernel.as_str(), nic.max_kernel.as_str()), ("23.0.0", "24.99.99"));
        assert!(pos(&out, "IO80211FamilyLegacy.kext") < pos(&out, &nic.bundle_path));

        assert_eq!(tighter_bound(Some("21.0.0"), Some("20.0.0"), true), Some("21.0.0"));
        assert_eq!(tighter_bound(Some("21.0.0"), Some("20.0.0"), false), Some("20.0.0"));
        assert_eq!(tighter_bound(None, Some(" 19.99.99 "), false), Some("19.99.99"));
        assert_eq!(tighter_bound(Some(""), None, true), None);
        assert_eq!(tighter_bound(Some("bogus"), Some("20.0.0"), true), Some("bogus"));
    }

    #[test]
    fn disabled_entries_missing_on_disk_are_left_out() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "Lilu.kext", "as.vit9696.Lilu", &[]);
        simple(k, "AirportBrcmFixup.kext", "as.lvs1974.AirportBrcmFixup", &["as.vit9696.Lilu"]);
        let fixup = sel(
            "AirportBrcmFixup",
            "AirportBrcmFixup.kext",
            &[("AirPortBrcm4360_Injector.kext", false), ("AirPortBrcmNIC_Injector.kext", false)],
        );
        let mut ghost = sel("Ghost", "Ghost.kext", &[]);
        ghost.enabled = false;
        let out = build_kernel_add(&[sel("Lilu", "Lilu.kext", &[]), fixup.clone(), ghost], k).unwrap();
        assert_eq!(paths(&out), vec!["Lilu.kext", "AirportBrcmFixup.kext"]);

        // An enabled plugin that is missing is still an error.
        let mut wanted = fixup;
        wanted.plugins[1].enabled = true;
        assert_eq!(build_kernel_add(&[wanted], k).unwrap_err().code, "KEXT_PLUGIN_MISSING");
    }

    #[test]
    fn errors_for_missing_bundles_plugins_and_cycles() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "Lilu.kext", "as.vit9696.Lilu", &[]);
        let err = build_kernel_add(&[sel("X", "Missing.kext", &[])], k).unwrap_err();
        assert_eq!(err.code, "KEXT_BUNDLE_MISSING");
        let err = build_kernel_add(&[sel("Lilu", "Lilu.kext", &[("Nope.kext", true)])], k).unwrap_err();
        assert_eq!(err.code, "KEXT_PLUGIN_MISSING");
        let err = build_kernel_add(&[sel("Lilu", "../Lilu.kext", &[])], k).unwrap_err();
        assert_eq!(err.code, "INVALID_BUNDLE_NAME");

        simple(k, "A.kext", "org.example.A", &["org.example.B"]);
        simple(k, "B.kext", "org.example.B", &["org.example.A"]);
        let err =
            build_kernel_add(&[sel("Lilu", "Lilu.kext", &[]), sel("A", "A.kext", &[]), sel("B", "B.kext", &[])], k)
                .unwrap_err();
        assert_eq!(err.code, "KEXT_DEPENDENCY_CYCLE");
        assert!(err.message.contains("A.kext") && err.message.contains("B.kext"));

        std::fs::create_dir_all(k.join("NoPlist.kext/Contents")).unwrap();
        assert_eq!(build_kernel_add(&[sel("N", "NoPlist.kext", &[])], k).unwrap_err().code, "KEXT_INVALID_BUNDLE");
    }

    #[test]
    fn bundle_paths_use_the_spelling_on_disk() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "CpuTopologyRebuild.kext", "org.b00t0x.CpuTopologyRebuild", &[]);
        bundle(k, "VoodooPS2Controller.kext", "as.acidanthera.voodoo.driver.PS2Controller", None, &[], false);
        bundle(
            k,
            "VoodooPS2Controller.kext/Contents/PlugIns/VoodooPS2Keyboard.kext",
            "as.acidanthera.voodoo.driver.PS2Keyboard",
            Some("VoodooPS2Keyboard"),
            &["as.acidanthera.voodoo.driver.PS2Controller"],
            false,
        );
        let out = build_kernel_add(
            &[
                sel("CpuTopologyRebuild", "CPUTopologyRebuild.kext", &[]),
                sel("VoodooPS2Controller", "voodoops2controller.kext", &[("VOODOOPS2KEYBOARD.kext", true)]),
            ],
            k,
        )
        .unwrap();
        assert_eq!(
            paths(&out),
            vec![
                "CpuTopologyRebuild.kext",
                "VoodooPS2Controller.kext",
                "VoodooPS2Controller.kext/Contents/PlugIns/VoodooPS2Keyboard.kext"
            ]
        );
        assert_eq!(out[0].comment, "CpuTopologyRebuild.kext");
        assert_eq!(out[0].executable_path, "Contents/MacOS/CpuTopologyRebuild");
    }

    #[test]
    fn duplicate_selection_is_listed_once_and_self_dependency_is_ignored() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "Self.kext", "org.example.Self", &["org.example.Self"]);
        let out = build_kernel_add(&[sel("S", "Self.kext", &[]), sel("S", "Self.kext", &[])], k).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn duplicate_bundle_ids_keep_the_first_unless_ranges_are_disjoint() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "BrcmFirmwareData.kext", "as.acidanthera.BrcmFirmwareStore", &[]);
        simple(k, "BrcmFirmwareRepo.kext", "as.acidanthera.BrcmFirmwareStore", &[]);
        simple(k, "MacHyperVSupport.kext", "fish.goldfish64.MacHyperVSupport", &[]);
        simple(k, "MacHyperVSupportMonterey.kext", "fish.goldfish64.MacHyperVSupport", &[]);
        let mut legacy = sel("MacHyperVSupport", "MacHyperVSupport.kext", &[]);
        legacy.max_kernel = Some("20.99.99".into());
        let mut modern = sel("MacHyperVSupport", "MacHyperVSupportMonterey.kext", &[]);
        modern.min_kernel = Some("21.0.0".into());
        let out = build_kernel_add(
            &[
                sel("BrcmPatchRAM", "BrcmFirmwareData.kext", &[]),
                sel("BrcmPatchRAM", "BrcmFirmwareRepo.kext", &[]),
                legacy,
                modern,
            ],
            k,
        )
        .unwrap();
        let enabled = |p: &str| out[pos(&out, p)].enabled;
        assert!(enabled("BrcmFirmwareData.kext"));
        assert!(!enabled("BrcmFirmwareRepo.kext"));
        assert!(enabled("MacHyperVSupport.kext"));
        assert!(enabled("MacHyperVSupportMonterey.kext"));
        assert_eq!(kernel_bound("20.99.99"), Some(209_999));
        assert_eq!(kernel_bound("25"), Some(250_000));
        assert_eq!(kernel_bound(""), None);
    }

    #[test]
    fn applealc_loads_after_nootedred() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "Lilu.kext", "as.vit9696.Lilu", &[]);
        simple(k, "AppleALC.kext", "as.vit9696.AppleALC", &["as.vit9696.Lilu"]);
        simple(k, "NootedRed.kext", "org.ChefKiss.NootedRed", &["as.vit9696.Lilu"]);
        simple(k, "WhateverGreen.kext", "as.vit9696.WhateverGreen", &["as.vit9696.Lilu"]);
        let alc_first = [sel("AppleALC", "AppleALC.kext", &[]), sel("Lilu", "Lilu.kext", &[])];
        let with_nr = [&alc_first[..], &[sel("NootedRed", "NootedRed.kext", &[])]].concat();
        let out = build_kernel_add(&with_nr, k).unwrap();
        assert_eq!(paths(&out), vec!["Lilu.kext", "NootedRed.kext", "AppleALC.kext"]);
        // Without NootedRed nothing changes.
        let with_weg = [&alc_first[..], &[sel("WhateverGreen", "WhateverGreen.kext", &[])]].concat();
        let out = build_kernel_add(&with_weg, k).unwrap();
        assert_eq!(paths(&out), vec!["Lilu.kext", "AppleALC.kext", "WhateverGreen.kext"]);
    }

    #[test]
    fn real_world_bluetooth_order() {
        let tmp = TempDir::new();
        let k = &tmp.0;
        simple(k, "Lilu.kext", "as.vit9696.Lilu", &[]);
        simple(
            k,
            "BrcmPatchRAM3.kext",
            "as.acidanthera.BrcmPatchRAM3",
            &["as.acidanthera.BrcmFirmwareStore", "com.apple.iokit.IOUSBHostFamily"],
        );
        simple(k, "BrcmFirmwareData.kext", "as.acidanthera.BrcmFirmwareStore", &[]);
        simple(k, "BlueToolFixup.kext", "as.acidanthera.BlueToolFixup", &["as.vit9696.Lilu"]);
        let out = build_kernel_add(
            &[
                sel("BrcmPatchRAM", "BlueToolFixup.kext", &[]),
                sel("BrcmPatchRAM", "BrcmPatchRAM3.kext", &[]),
                sel("BrcmPatchRAM", "BrcmFirmwareData.kext", &[]),
                sel("Lilu", "Lilu.kext", &[]),
            ],
            k,
        )
        .unwrap();
        assert_eq!(paths(&out), vec!["Lilu.kext", "BlueToolFixup.kext", "BrcmFirmwareData.kext", "BrcmPatchRAM3.kext"]);
    }
}
