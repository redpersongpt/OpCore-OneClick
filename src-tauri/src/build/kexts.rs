//! Kext phase bookkeeping: one download per catalog archive, bundle
//! installation into EFI/OC/Kexts, and the rule for failures: a required kext
//! fails the build, an optional one is left out of the EFI and the plan
//! (status `Skipped`, with a warning), together with optional kexts that can
//! no longer load because of it.

use std::collections::HashSet;
use std::path::Path;

use plist::Value;

use crate::contracts::{ArtifactStatus, KextResult};
use crate::domain::model::KextSelection;
use crate::error::AppError;
use crate::services::artifacts::{self, FetchedKext};

use super::staging::find_ci;
use super::while_doing;

/// The selections served by one catalog archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KextGroup {
    pub catalog_id: String,
    /// Indices into the selection list, in plan order.
    pub members: Vec<usize>,
    /// At least one member is required.
    pub required: bool,
}

/// Group selections by catalog id in first-appearance order, so every archive
/// is downloaded once.
pub fn group_by_catalog(kexts: &[KextSelection]) -> Vec<KextGroup> {
    let mut groups: Vec<KextGroup> = Vec::new();
    for (i, sel) in kexts.iter().enumerate() {
        match groups.iter_mut().find(|g| g.catalog_id == sel.catalog_id) {
            Some(group) => {
                group.members.push(i);
                group.required |= sel.required;
            }
            None => {
                groups.push(KextGroup { catalog_id: sel.catalog_id.clone(), members: vec![i], required: sel.required })
            }
        }
    }
    groups
}

/// State of the kext phase: the plan's selections plus one result each.
#[derive(Debug)]
pub struct KextStage {
    selections: Vec<KextSelection>,
    results: Vec<Option<KextResult>>,
    skipped: Vec<bool>,
    /// A kext or a plugin was left out, so dependencies need a second look.
    pruned: bool,
    warnings: Vec<String>,
}

impl KextStage {
    pub fn new(selections: Vec<KextSelection>) -> Self {
        let n = selections.len();
        Self { selections, results: vec![None; n], skipped: vec![false; n], pruned: false, warnings: Vec::new() }
    }

    pub fn selections(&self) -> &[KextSelection] {
        &self.selections
    }

    pub fn groups(&self) -> Vec<KextGroup> {
        group_by_catalog(&self.selections)
    }

    /// The archive of `group` could not be obtained.
    pub fn archive_failed(&mut self, group: &KextGroup, err: AppError) -> Result<(), AppError> {
        if err.code == "TASK_CANCELLED" {
            return Err(err);
        }
        if group.required {
            return Err(while_doing(err, &format!("Could not download {} (required)", group.catalog_id)));
        }
        for &i in &group.members {
            self.skip(i, &format!("download failed: {}", err.message), None);
        }
        Ok(())
    }

    /// Copy the bundles `group` asks for from the extracted archive.
    pub fn install(&mut self, group: &KextGroup, fetched: &FetchedKext, kexts_dir: &Path) -> Result<(), AppError> {
        for &i in &group.members {
            if self.skipped[i] {
                continue;
            }
            let required = self.selections[i].required;
            let bundle = self.selections[i].bundle.clone();
            if let Err(e) = artifacts::install_bundle(fetched, &bundle, kexts_dir) {
                if required {
                    return Err(while_doing(e, &format!("Installing {bundle} (required)")));
                }
                self.skip(i, &e.message, Some(kexts_dir));
                continue;
            }
            let on_disk = find_ci(kexts_dir, &bundle)
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| bundle.clone());
            let missing = missing_plugins(&kexts_dir.join(&on_disk), &self.selections[i]);
            if !missing.is_empty() {
                if required {
                    return Err(AppError::new(
                        "KEXT_PLUGIN_MISSING",
                        format!("{bundle} from {} has no {}", group.catalog_id, missing.join(", ")),
                    ));
                }
                self.selections[i].plugins.retain(|p| !missing.contains(&p.bundle));
                self.pruned = true;
                self.warnings.push(format!(
                    "{bundle}: plugin {} is not in the downloaded archive and was left out",
                    missing.join(", ")
                ));
            }
            let sel = &self.selections[i];
            self.results[i] = Some(KextResult {
                name: on_disk,
                catalog_id: sel.catalog_id.clone(),
                version: Some(fetched.version.clone()),
                status: fetched.status,
                enabled: sel.enabled,
                reason: sel.reason.clone(),
                error: None,
            });
        }
        Ok(())
    }

    /// After optional kexts or plugins were left out, leave out the optional
    /// kexts (and plugins of optional kexts) whose OSBundleLibraries are no
    /// longer provided by any selected bundle; a required kext in that state
    /// stays, with a warning. Does nothing when nothing was left out: the
    /// plan itself is checked by ocvalidate.
    pub fn drop_unmet_dependencies(&mut self, kexts_dir: &Path) {
        if !self.pruned {
            return;
        }
        let mut reported: HashSet<usize> = HashSet::new();
        loop {
            let provided = self.provided_ids(kexts_dir);
            let mut changed = false;
            for i in 0..self.selections.len() {
                if self.skipped[i] || !self.selections[i].enabled {
                    continue;
                }
                let sel = &self.selections[i];
                let Some(bundle_dir) = find_ci(kexts_dir, &sel.bundle) else { continue };
                let unmet = |dir: &Path| {
                    bundle_info(dir).and_then(|(_, libs)| {
                        libs.into_iter().find(|l| !is_apple(l) && !provided.contains(&l.to_ascii_lowercase()))
                    })
                };
                if let Some(lib) = unmet(&bundle_dir) {
                    if sel.required {
                        if reported.insert(i) {
                            tracing::warn!(bundle = %sel.bundle, lib, "required kext depends on a bundle that is not loaded");
                            self.warnings.push(format!(
                                "{} needs {lib}, which was left out of this build; it will not load",
                                sel.bundle
                            ));
                        }
                        continue;
                    }
                    self.skip(i, &format!("needs {lib}, which is not loaded"), Some(kexts_dir));
                    changed = true;
                    continue;
                }
                if sel.required {
                    continue;
                }
                let plugins_dir = bundle_dir.join("Contents").join("PlugIns");
                let broken: Vec<(String, String)> = sel
                    .plugins
                    .iter()
                    .filter(|p| p.enabled)
                    .filter_map(|p| {
                        let dir = find_ci(&plugins_dir, &p.bundle)?;
                        unmet(&dir).map(|lib| (p.bundle.clone(), lib))
                    })
                    .collect();
                if !broken.is_empty() {
                    let bundle = sel.bundle.clone();
                    self.selections[i].plugins.retain(|p| !broken.iter().any(|(b, _)| *b == p.bundle));
                    for (plugin, lib) in broken {
                        self.warnings.push(format!("{bundle}: plugin {plugin} was left out because it needs {lib}"));
                    }
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Bundle ids of every selected bundle and selected plugin still in the build.
    fn provided_ids(&self, kexts_dir: &Path) -> HashSet<String> {
        let mut ids = HashSet::new();
        for (i, sel) in self.selections.iter().enumerate() {
            if self.skipped[i] {
                continue;
            }
            let Some(dir) = find_ci(kexts_dir, &sel.bundle) else { continue };
            if let Some((id, _)) = bundle_info(&dir) {
                ids.insert(id.to_ascii_lowercase());
            }
            let plugins = dir.join("Contents").join("PlugIns");
            for p in &sel.plugins {
                if let Some((id, _)) = find_ci(&plugins, &p.bundle).and_then(|d| bundle_info(&d)) {
                    ids.insert(id.to_ascii_lowercase());
                }
            }
        }
        ids
    }

    fn skip(&mut self, i: usize, reason: &str, kexts_dir: Option<&Path>) {
        self.skipped[i] = true;
        self.pruned = true;
        let sel = &self.selections[i];
        tracing::warn!(kext = %sel.bundle, catalog = %sel.catalog_id, reason, "optional kext left out");
        self.warnings.push(format!("{} ({}) was left out: {reason}", sel.bundle, sel.catalog_id));
        self.results[i] = Some(KextResult {
            name: sel.bundle.clone(),
            catalog_id: sel.catalog_id.clone(),
            version: self.results[i].as_ref().and_then(|r| r.version.clone()),
            status: ArtifactStatus::Skipped,
            enabled: false,
            reason: sel.reason.clone(),
            error: Some(reason.to_string()),
        });
        // Remove the copied bundle unless another selection still uses it.
        let Some(dir) = kexts_dir else { return };
        let shared = self
            .selections
            .iter()
            .enumerate()
            .any(|(j, other)| j != i && !self.skipped[j] && other.bundle.eq_ignore_ascii_case(&sel.bundle));
        if !shared {
            if let Some(path) = find_ci(dir, &sel.bundle) {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!(dir = %path.display(), error = %e, "could not remove a left-out kext");
                }
            }
        }
    }

    /// Selections that stay in the plan, one result per original selection
    /// (in plan order), and the warnings collected.
    pub fn finish(self) -> (Vec<KextSelection>, Vec<KextResult>, Vec<String>) {
        let mut kept = Vec::new();
        let mut results = Vec::new();
        for ((sel, result), skipped) in self.selections.into_iter().zip(self.results).zip(self.skipped) {
            results.push(result.unwrap_or_else(|| KextResult {
                name: sel.bundle.clone(),
                catalog_id: sel.catalog_id.clone(),
                version: None,
                status: ArtifactStatus::Failed,
                enabled: false,
                reason: sel.reason.clone(),
                error: Some("not processed".into()),
            }));
            if !skipped {
                kept.push(sel);
            }
        }
        (kept, results, self.warnings)
    }
}

/// Enabled plugins of `sel` that are not inside the installed bundle.
fn missing_plugins(bundle_dir: &Path, sel: &KextSelection) -> Vec<String> {
    let plugins_dir = bundle_dir.join("Contents").join("PlugIns");
    sel.plugins
        .iter()
        .filter(|p| p.enabled && find_ci(&plugins_dir, &p.bundle).is_none_or(|d| !d.is_dir()))
        .map(|p| p.bundle.clone())
        .collect()
}

/// (CFBundleIdentifier, OSBundleLibraries keys) of a bundle.
fn bundle_info(dir: &Path) -> Option<(String, Vec<String>)> {
    let value = Value::from_file(dir.join("Contents").join("Info.plist")).ok()?;
    let dict = value.as_dictionary()?;
    let id = dict.get("CFBundleIdentifier")?.as_string()?.trim().to_string();
    let libs = dict
        .get("OSBundleLibraries")
        .and_then(Value::as_dictionary)
        .map(|d| d.keys().map(|k| k.to_string()).collect())
        .unwrap_or_default();
    Some((id, libs))
}

fn is_apple(bundle_id: &str) -> bool {
    bundle_id.to_ascii_lowercase().starts_with("com.apple.")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::staging::test_dir::TempDir;
    use super::*;
    use crate::domain::model::PluginSelection;

    fn sel(catalog: &str, bundle: &str, required: bool, plugins: &[&str]) -> KextSelection {
        KextSelection {
            catalog_id: catalog.into(),
            bundle: bundle.into(),
            plugins: plugins
                .iter()
                .map(|p| PluginSelection { bundle: (*p).into(), enabled: true, min_kernel: None, max_kernel: None })
                .collect(),
            enabled: true,
            min_kernel: None,
            max_kernel: None,
            required,
            reason: format!("{bundle} reason"),
        }
    }

    fn bundle(dir: &Path, name: &str, id: &str, libs: &[&str]) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
        let libs: String = libs.iter().map(|l| format!("<key>{l}</key><string>1.0</string>")).collect();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>{id}</string>\
             <key>OSBundleLibraries</key><dict>{libs}</dict></dict></plist>"
        );
        std::fs::write(path.join("Contents/Info.plist"), plist).unwrap();
        path
    }

    fn fetched(root: &Path, catalog: &str) -> FetchedKext {
        let dir = root.join(catalog).join("bundles");
        std::fs::create_dir_all(&dir).unwrap();
        FetchedKext {
            catalog_id: catalog.into(),
            version: "1.0.0".into(),
            status: ArtifactStatus::Downloaded,
            bundles_dir: dir,
        }
    }

    #[test]
    fn groups_keep_plan_order_and_merge_requirements() {
        let kexts = vec![
            sel("Lilu", "Lilu.kext", true, &[]),
            sel("VirtualSMC", "VirtualSMC.kext", true, &[]),
            sel("WhateverGreen", "WhateverGreen.kext", true, &[]),
            sel("VirtualSMC", "SMCProcessor.kext", false, &[]),
            sel("BrcmPatchRAM", "BlueToolFixup.kext", false, &[]),
            sel("VirtualSMC", "SMCSuperIO.kext", false, &[]),
            sel("BrcmPatchRAM", "BrcmPatchRAM3.kext", false, &[]),
        ];
        let groups = group_by_catalog(&kexts);
        let ids: Vec<&str> = groups.iter().map(|g| g.catalog_id.as_str()).collect();
        assert_eq!(ids, ["Lilu", "VirtualSMC", "WhateverGreen", "BrcmPatchRAM"]);
        assert_eq!(groups[1].members, vec![1, 3, 5]);
        assert!(groups[1].required);
        assert_eq!(groups[3].members, vec![4, 6]);
        assert!(!groups[3].required);
        assert!(group_by_catalog(&[]).is_empty());
    }

    #[test]
    fn installs_bundles_and_reports_versions() {
        let tmp = TempDir::new("kexts");
        let kexts_dir = tmp.path().join("EFI/OC/Kexts");
        let smc = fetched(tmp.path(), "VirtualSMC");
        bundle(&smc.bundles_dir, "VirtualSMC.kext", "as.vit9696.VirtualSMC", &["as.vit9696.Lilu"]);
        bundle(&smc.bundles_dir, "SMCProcessor.kext", "as.vit9696.SMCProcessor", &["as.vit9696.VirtualSMC"]);
        let mut stage = KextStage::new(vec![
            sel("VirtualSMC", "VirtualSMC.kext", true, &[]),
            sel("VirtualSMC", "smcprocessor.kext", false, &[]),
        ]);
        let groups = stage.groups();
        assert_eq!(groups.len(), 1);
        stage.install(&groups[0], &smc, &kexts_dir).unwrap();
        let (kept, results, warnings) = stage.finish();
        assert_eq!(kept.len(), 2);
        assert!(warnings.is_empty());
        assert_eq!(results[0].status, ArtifactStatus::Downloaded);
        assert_eq!(results[0].version.as_deref(), Some("1.0.0"));
        // The on-disk spelling is what Kernel->Add will use.
        assert_eq!(results[1].name, "SMCProcessor.kext");
        assert!(kexts_dir.join("VirtualSMC.kext/Contents/Info.plist").is_file());
    }

    #[test]
    fn failed_optional_archive_is_skipped_and_required_fails() {
        let mut stage = KextStage::new(vec![
            sel("Lilu", "Lilu.kext", true, &[]),
            sel("BrcmPatchRAM", "BrcmFirmwareData.kext", false, &[]),
            sel("BrcmPatchRAM", "BrcmPatchRAM3.kext", false, &[]),
        ]);
        let groups = stage.groups();
        let offline = || AppError::new("NETWORK_ERROR", "could not connect").recoverable();
        stage.archive_failed(&groups[1], offline()).unwrap();
        let err = stage.archive_failed(&groups[0], offline()).unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");
        assert!(err.message.contains("Lilu"), "{}", err.message);
        let cancelled = stage.archive_failed(&groups[1], AppError::new("TASK_CANCELLED", "x")).unwrap_err();
        assert_eq!(cancelled.code, "TASK_CANCELLED");

        let (kept, results, warnings) = stage.finish();
        assert_eq!(kept.iter().map(|k| k.bundle.as_str()).collect::<Vec<_>>(), ["Lilu.kext"]);
        assert_eq!(results.len(), 3);
        assert_eq!(results[1].status, ArtifactStatus::Skipped);
        assert!(!results[1].enabled);
        assert!(results[1].error.as_deref().unwrap().contains("could not connect"));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn missing_bundles_and_plugins() {
        let tmp = TempDir::new("kexts");
        let kexts_dir = tmp.path().join("Kexts");
        let ps2 = fetched(tmp.path(), "VoodooPS2Controller");
        let ctrl =
            bundle(&ps2.bundles_dir, "VoodooPS2Controller.kext", "as.acidanthera.voodoo.driver.PS2Controller", &[]);
        bundle(
            &ctrl.join("Contents/PlugIns"),
            "VoodooPS2Keyboard.kext",
            "as.acidanthera.voodoo.driver.PS2Keyboard",
            &[],
        );

        let mut stage = KextStage::new(vec![
            sel(
                "VoodooPS2Controller",
                "VoodooPS2Controller.kext",
                false,
                &["VoodooPS2Keyboard.kext", "VoodooPS2Gone.kext"],
            ),
            sel("VoodooPS2Controller", "NotThere.kext", false, &[]),
        ]);
        let groups = stage.groups();
        stage.install(&groups[0], &ps2, &kexts_dir).unwrap();
        let (kept, results, warnings) = stage.finish();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].plugins.len(), 1);
        assert_eq!(kept[0].plugins[0].bundle, "VoodooPS2Keyboard.kext");
        assert_eq!(results[1].status, ArtifactStatus::Skipped);
        assert_eq!(warnings.len(), 2, "{warnings:?}");

        // The same gaps fail the build for required kexts.
        let mut strict =
            KextStage::new(vec![sel("VoodooPS2Controller", "VoodooPS2Controller.kext", true, &["VoodooPS2Gone.kext"])]);
        let groups = strict.groups();
        assert_eq!(strict.install(&groups[0], &ps2, &kexts_dir).unwrap_err().code, "KEXT_PLUGIN_MISSING");
        let mut strict = KextStage::new(vec![sel("VoodooPS2Controller", "NotThere.kext", true, &[])]);
        let groups = strict.groups();
        assert_eq!(strict.install(&groups[0], &ps2, &kexts_dir).unwrap_err().code, "KEXT_BUNDLE_MISSING");
    }

    #[test]
    fn dependents_of_skipped_kexts_are_left_out() {
        let tmp = TempDir::new("kexts");
        let kexts_dir = tmp.path().join("Kexts");
        let lilu = fetched(tmp.path(), "Lilu");
        bundle(&lilu.bundles_dir, "Lilu.kext", "as.vit9696.Lilu", &["com.apple.kpi.libkern"]);
        let brcm = fetched(tmp.path(), "BrcmPatchRAM");
        bundle(&brcm.bundles_dir, "BlueToolFixup.kext", "as.acidanthera.BlueToolFixup", &["as.vit9696.Lilu"]);
        let intel = fetched(tmp.path(), "IntelBluetoothFirmware");
        bundle(&intel.bundles_dir, "IntelBTPatcher.kext", "com.zxystd.IntelBTPatcher", &["com.example.Missing"]);
        let input = fetched(tmp.path(), "VoodooRMI");
        let rmi = bundle(&input.bundles_dir, "VoodooRMI.kext", "me.kishorprins.VoodooRMI", &["as.vit9696.Lilu"]);
        bundle(&rmi.join("Contents/PlugIns"), "RMII2C.kext", "me.kishorprins.RMII2C", &["com.example.Missing"]);

        let mut stage = KextStage::new(vec![
            sel("Lilu", "Lilu.kext", true, &[]),
            sel("Missing", "Missing.kext", false, &[]),
            sel("BrcmPatchRAM", "BlueToolFixup.kext", false, &[]),
            sel("IntelBluetoothFirmware", "IntelBTPatcher.kext", false, &[]),
            sel("VoodooRMI", "VoodooRMI.kext", false, &["RMII2C.kext"]),
        ]);
        let groups = stage.groups();
        stage.install(&groups[0], &lilu, &kexts_dir).unwrap();
        stage.archive_failed(&groups[1], AppError::new("HTTP_NOT_FOUND", "404")).unwrap();
        stage.install(&groups[2], &brcm, &kexts_dir).unwrap();
        stage.install(&groups[3], &intel, &kexts_dir).unwrap();
        stage.install(&groups[4], &input, &kexts_dir).unwrap();
        stage.drop_unmet_dependencies(&kexts_dir);

        let (kept, results, _) = stage.finish();
        let names: Vec<&str> = kept.iter().map(|k| k.bundle.as_str()).collect();
        assert_eq!(names, ["Lilu.kext", "BlueToolFixup.kext", "VoodooRMI.kext"]);
        assert!(kept[2].plugins.is_empty(), "the plugin with a missing dependency is dropped");
        assert_eq!(results[3].status, ArtifactStatus::Skipped);
        assert!(!kexts_dir.join("IntelBTPatcher.kext").exists());
        assert!(kexts_dir.join("BlueToolFixup.kext").exists());
    }

    #[test]
    fn a_missing_plugin_also_triggers_the_dependency_check() {
        let tmp = TempDir::new("kexts");
        let kexts_dir = tmp.path().join("Kexts");
        let i2c = fetched(tmp.path(), "VoodooI2C");
        let main =
            bundle(&i2c.bundles_dir, "VoodooI2C.kext", "com.alexandred.VoodooI2C", &["org.coolstar.VoodooGPIO"]);
        std::fs::create_dir_all(main.join("Contents/PlugIns")).unwrap();
        bundle(&i2c.bundles_dir, "VoodooI2CHID.kext", "com.alexandred.VoodooI2CHID", &["com.alexandred.VoodooI2C"]);

        let mut stage = KextStage::new(vec![
            sel("VoodooI2C", "VoodooI2C.kext", false, &["VoodooGPIO.kext"]),
            sel("VoodooI2C", "VoodooI2CHID.kext", false, &[]),
        ]);
        let groups = stage.groups();
        stage.install(&groups[0], &i2c, &kexts_dir).unwrap();
        stage.drop_unmet_dependencies(&kexts_dir);
        let (kept, results, warnings) = stage.finish();
        // VoodooI2C cannot load without VoodooGPIO, and VoodooI2CHID not without VoodooI2C.
        assert!(kept.is_empty(), "{kept:?}");
        assert!(results.iter().all(|r| r.status == ArtifactStatus::Skipped));
        assert!(!kexts_dir.join("VoodooI2C.kext").exists());
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn required_kexts_with_a_missing_dependency_stay_with_a_warning() {
        let tmp = TempDir::new("kexts");
        let kexts_dir = tmp.path().join("Kexts");
        let alc = fetched(tmp.path(), "AppleALC");
        bundle(&alc.bundles_dir, "AppleALC.kext", "as.vit9696.AppleALC", &["as.vit9696.Lilu"]);
        let mut stage =
            KextStage::new(vec![sel("Lilu", "Lilu.kext", false, &[]), sel("AppleALC", "AppleALC.kext", true, &[])]);
        let groups = stage.groups();
        stage.archive_failed(&groups[0], AppError::new("NETWORK_ERROR", "offline")).unwrap();
        stage.install(&groups[1], &alc, &kexts_dir).unwrap();
        stage.drop_unmet_dependencies(&kexts_dir);
        let (kept, _, warnings) = stage.finish();
        assert_eq!(kept.iter().map(|k| k.bundle.as_str()).collect::<Vec<_>>(), ["AppleALC.kext"]);
        assert_eq!(warnings.iter().filter(|w| w.contains("as.vit9696.Lilu")).count(), 1, "{warnings:?}");
    }

    #[test]
    fn dependency_check_is_inert_without_failures() {
        let tmp = TempDir::new("kexts");
        let kexts_dir = tmp.path().join("Kexts");
        let intel = fetched(tmp.path(), "IntelBluetoothFirmware");
        bundle(&intel.bundles_dir, "IntelBTPatcher.kext", "com.zxystd.IntelBTPatcher", &["as.vit9696.Lilu"]);
        let mut stage = KextStage::new(vec![sel("IntelBluetoothFirmware", "IntelBTPatcher.kext", false, &[])]);
        let groups = stage.groups();
        stage.install(&groups[0], &intel, &kexts_dir).unwrap();
        stage.drop_unmet_dependencies(&kexts_dir);
        let (kept, _, _) = stage.finish();
        assert_eq!(kept.len(), 1);
    }
}
