//! End-to-end check of the artifact pipeline with the real pinned downloads:
//! OpenCore, OcBinaryData, a Dortania SSDT and a laptop-style kext set are
//! fetched and installed, Kernel->Add comes from `kernel_add`, and the EFI is
//! checked by the layout validator and OpenCore's own ocvalidate.
//!
//! Needs network access: `cargo test --test artifacts_efi_network -- --ignored --nocapture`

use std::path::{Path, PathBuf};

use app_lib::domain::kernel_add::{build_kernel_add, KernelAddEntry};
use app_lib::domain::kext_catalog;
use app_lib::domain::model::{KextSelection, NoteLevel, PluginSelection};
use app_lib::services::artifacts::{self, FetchedKext};
use app_lib::services::http::Downloader;
use app_lib::services::ocvalidate::validate_efi;
use app_lib::tasks::cancellation::CancellationToken;
use plist::{Dictionary, Value};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("oneclick-efi-net-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sel(catalog: &str, bundle: &str, plugins: &[&str]) -> KextSelection {
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
        required: true,
        reason: String::new(),
    }
}

fn add_dict(e: &KernelAddEntry) -> Value {
    let mut d = Dictionary::new();
    d.insert("Arch".into(), e.arch.clone().into());
    d.insert("BundlePath".into(), e.bundle_path.clone().into());
    d.insert("Comment".into(), e.comment.clone().into());
    d.insert("Enabled".into(), e.enabled.into());
    d.insert("ExecutablePath".into(), e.executable_path.clone().into());
    d.insert("MaxKernel".into(), e.max_kernel.clone().into());
    d.insert("MinKernel".into(), e.min_kernel.clone().into());
    d.insert("PlistPath".into(), e.plist_path.clone().into());
    Value::Dictionary(d)
}

fn copy_file(from: &Path, to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(from, to).unwrap_or_else(|e| panic!("{} -> {}: {e}", from.display(), to.display()));
}

#[tokio::test]
#[ignore]
async fn pinned_artifacts_build_a_valid_efi() {
    let tmp = TempDir::new();
    let dl = Downloader::new(tmp.0.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    let work = tmp.0.join("work");

    let oc = artifacts::fetch_opencore(&dl, &work, false, false, &cancel).await.unwrap();
    let bin = artifacts::fetch_ocbinarydata(&dl, &work, &cancel).await.unwrap();
    let efi = tmp.0.join("build").join("EFI");
    copy_file(&oc.x64_efi().join("BOOT/BOOTx64.efi"), &efi.join("BOOT/BOOTx64.efi"));
    copy_file(&oc.x64_efi().join("OC/OpenCore.efi"), &efi.join("OC/OpenCore.efi"));
    copy_file(&oc.x64_efi().join("OC/Drivers/OpenRuntime.efi"), &efi.join("OC/Drivers/OpenRuntime.efi"));
    copy_file(&bin.join("Drivers/HfsPlus.efi"), &efi.join("OC/Drivers/HfsPlus.efi"));
    let ssdt = artifacts::fetch_dortania_ssdt(&dl, "SSDT-EC-USBX-LAPTOP.aml", &cancel).await.unwrap();
    std::fs::create_dir_all(efi.join("OC/ACPI")).unwrap();
    std::fs::write(efi.join("OC/ACPI/SSDT-EC-USBX-LAPTOP.aml"), ssdt).unwrap();

    // Deliberately out of dependency order.
    let selections = vec![
        sel("VoodooI2C", "VoodooI2CHID.kext", &[]),
        sel("BrcmPatchRAM", "BrcmPatchRAM3.kext", &[]),
        sel("BrcmPatchRAM", "BlueToolFixup.kext", &[]),
        sel("BrcmPatchRAM", "BrcmFirmwareData.kext", &[]),
        sel("AppleALC", "AppleALC.kext", &[]),
        sel("VirtualSMC", "SMCBatteryManager.kext", &[]),
        sel("VirtualSMC", "SMCProcessor.kext", &[]),
        sel(
            "VoodooPS2Controller",
            "VoodooPS2Controller.kext",
            &["VoodooInput.kext", "VoodooPS2Keyboard.kext", "VoodooPS2Trackpad.kext", "VoodooPS2Mouse.kext"],
        ),
        sel("VoodooI2C", "VoodooI2C.kext", &["VoodooGPIO.kext", "VoodooI2CServices.kext", "VoodooInput.kext"]),
        sel("VoodooRMI", "VoodooRMI.kext", &["VoodooInput.kext", "RMII2C.kext"]),
        sel("WhateverGreen", "WhateverGreen.kext", &[]),
        sel("AirportBrcmFixup", "AirportBrcmFixup.kext", &["AirPortBrcmNIC_Injector.kext"]),
        sel("USBToolBox", "UTBDefault.kext", &[]),
        sel("USBToolBox", "USBToolBox.kext", &[]),
        sel("VirtualSMC", "VirtualSMC.kext", &[]),
        sel("Lilu", "Lilu.kext", &[]),
    ];
    let kexts_dir = efi.join("OC/Kexts");
    let mut fetched: Vec<FetchedKext> = Vec::new();
    for s in &selections {
        if !fetched.iter().any(|f| f.catalog_id == s.catalog_id) {
            let entry = kext_catalog::entry(&s.catalog_id).unwrap();
            fetched.push(artifacts::fetch_kext(&dl, entry, &work, false, &cancel).await.unwrap());
        }
        let f = fetched.iter().find(|f| f.catalog_id == s.catalog_id).unwrap();
        artifacts::install_bundle(f, &s.bundle, &kexts_dir).unwrap();
    }

    let entries = build_kernel_add(&selections, &kexts_dir).unwrap();
    let order: Vec<&str> = entries.iter().map(|e| e.bundle_path.as_str()).collect();
    println!("{order:#?}");
    assert_eq!(order[0], "Lilu.kext");
    assert_eq!(order[1], "VirtualSMC.kext");
    let enabled_inputs: Vec<&str> = entries
        .iter()
        .filter(|e| e.enabled && e.bundle_id == "me.kishorprins.VoodooInput")
        .map(|e| e.bundle_path.as_str())
        .collect();
    assert_eq!(enabled_inputs, vec!["VoodooRMI.kext/Contents/PlugIns/VoodooInput.kext"]);
    let utb = entries.iter().find(|e| e.bundle_path == "UTBDefault.kext").unwrap();
    assert_eq!(utb.executable_path, "");

    let mut config = Value::from_file(oc.sample_plist()).unwrap();
    let root = config.as_dictionary_mut().unwrap();
    let kernel = root.get_mut("Kernel").and_then(Value::as_dictionary_mut).unwrap();
    kernel.insert("Add".into(), Value::Array(entries.iter().map(add_dict).collect()));
    let acpi = root.get_mut("ACPI").and_then(Value::as_dictionary_mut).unwrap();
    let mut ssdt_entry = Dictionary::new();
    ssdt_entry.insert("Comment".into(), "EC and USBX".into());
    ssdt_entry.insert("Enabled".into(), true.into());
    ssdt_entry.insert("Path".into(), "SSDT-EC-USBX-LAPTOP.aml".into());
    acpi.insert("Add".into(), Value::Array(vec![Value::Dictionary(ssdt_entry)]));
    config.to_file_xml(efi.join("OC/config.plist")).unwrap();

    let ocvalidate = oc.ocvalidate().expect("ocvalidate for this host");
    let result = validate_efi(&efi, Some(&ocvalidate)).await;
    println!("{}", result.ocvalidate_output.clone().unwrap_or_default());
    for i in &result.issues {
        println!("{:?} [{}] {}", i.level, i.source, i.message);
    }
    assert!(result.ocvalidate_ran);
    assert!(result.valid, "{:#?}", result.issues);
    assert!(result.issues.iter().all(|i| i.level != NoteLevel::Blocking));
}
