//! End-to-end EFI builds through `build::run` with the real pinned downloads:
//! OpenCore, OcBinaryData, kexts, Dortania SSDTs and (for AMD) AMD_Vanilla,
//! then OpenCore's own ocvalidate. The plans are written by hand so the test
//! does not depend on the planner. Covered: Coffee Lake desktop (graphical
//! picker), Kaby Lake laptop (text picker, PS/2 plugins, emulated NVRAM) and
//! a Ryzen desktop (AMD_Vanilla).
//!
//! Needs network access:
//! `cargo test --test pipeline_efi_network -- --ignored --nocapture`
//! (`PIPELINE_KEEP=1` keeps the output in the temp directory).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use app_lib::build::progress::{BuildProgress, Phase};
use app_lib::build::{self, BuildEnv};
use app_lib::contracts::ArtifactStatus;
use app_lib::domain::model::{
    hex_upper, BinaryPatch, BuildOptions, BuildPlan, CpuPlatform, CpuVendor, DeviceBus, DeviceProperty,
    DevicePropertyEntry, DriverPlan, FormFactor, HardwareProfile, KextSelection, MacOsVersion, NoteLevel, PickerStyle,
    PlistScalar, PluginSelection, ProfileAudio, ProfileCpu, ProfileGpu, ProfileNic, SettingMap, SmbiosPlan, SsdtPlan,
    SsdtSource,
};
use app_lib::domain::smbios_gen::{mlb_checksum_valid, serial_format_valid};
use app_lib::services::http::Downloader;
use app_lib::tasks::cancellation::CancellationToken;
use plist::Value;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("oneclick-pipeline-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if std::env::var_os("PIPELINE_KEEP").is_some() {
            eprintln!("kept {}", self.0.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn s(v: &str) -> PlistScalar {
    PlistScalar::Str(v.into())
}

fn settings(items: &[(&str, PlistScalar)]) -> SettingMap {
    items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn kext(catalog: &str, bundle: &str, required: bool, plugins: &[&str]) -> KextSelection {
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
        reason: format!("{bundle} for this machine"),
    }
}

fn dortania(file: &str, required: bool) -> SsdtPlan {
    SsdtPlan {
        file_name: file.into(),
        source: SsdtSource::Dortania { file: file.into() },
        required,
        reason: String::new(),
    }
}

fn driver(path: &str, source: &str) -> DriverPlan {
    DriverPlan { path: path.into(), load_early: false, enabled: true, comment: String::new(), source: source.into() }
}

/// A tiny but well-formed SSDT (header only), standing in for generated AML.
fn generated_table() -> SsdtPlan {
    let mut t = b"SSDT".to_vec();
    t.extend_from_slice(&36u32.to_le_bytes());
    t.extend_from_slice(&[2, 0]);
    t.extend_from_slice(b"OCLICK");
    t.extend_from_slice(b"PIPETEST");
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(b"INTL");
    t.extend_from_slice(&0x2023_0628u32.to_le_bytes());
    let sum = t.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    t[9] = 0u8.wrapping_sub(sum);
    SsdtPlan {
        file_name: "SSDT-TEST.aml".into(),
        source: SsdtSource::Generated { aml_hex: hex_upper(&t), dsl: String::new() },
        required: true,
        reason: "generated".into(),
    }
}

fn base_plan(model: &str, target: MacOsVersion) -> BuildPlan {
    let mut plan = app_lib::domain::planner::empty_plan(target);
    plan.smbios = SmbiosPlan {
        model: model.into(),
        reason: "test".into(),
        secure_boot_model: "Disabled".into(),
        board_id_skip: false,
        alternatives: vec![],
    };
    plan.kernel_quirks =
        settings(&[("PanicNoKextDump", PlistScalar::Bool(true)), ("PowerTimeoutKernelPanic", PlistScalar::Bool(true))]);
    plan.misc_boot = settings(&[("PickerMode", s("External")), ("HideAuxiliary", PlistScalar::Bool(true))]);
    plan.misc_debug = settings(&[
        ("AppleDebug", PlistScalar::Bool(true)),
        ("ApplePanic", PlistScalar::Bool(true)),
        ("DisableWatchDog", PlistScalar::Bool(true)),
        ("Target", PlistScalar::Int(67)),
    ]);
    plan.misc_security = settings(&[
        ("AllowSetDefault", PlistScalar::Bool(true)),
        ("BlacklistAppleUpdate", PlistScalar::Bool(true)),
        ("SecureBootModel", s("Disabled")),
    ]);
    plan.tools = vec!["OpenShell.efi".into()];
    plan.boot_args = vec!["-v".into(), "keepsyms=1".into(), "debug=0x100".into()];
    plan.nvram_settings = settings(&[("WriteFlash", PlistScalar::Bool(true))]);
    plan.drivers = vec![
        driver("HfsPlus.efi", "ocbinarydata"),
        driver("OpenRuntime.efi", "opencore"),
        driver("OpenCanopy.efi", "opencore"),
        driver("ResetNvramEntry.efi", "opencore"),
    ];
    plan
}

/// i7-9700K on a Z390 board, UHD 630 driving the display, I219-V Ethernet.
fn coffee_lake_profile() -> HardwareProfile {
    HardwareProfile {
        cpu: ProfileCpu {
            name: "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz".into(),
            vendor: CpuVendor::Intel,
            platform: CpuPlatform::CoffeeLake,
            codename: "Coffee Lake-S".into(),
            family: Some(6),
            model: Some(0x9E),
            stepping: Some(13),
            cores: 8,
            threads: 8,
            is_mobile: false,
            has_avx2: Some(true),
            has_sse4_2: Some(true),
            is_hybrid: false,
        },
        form_factor: FormFactor::Desktop,
        gpus: vec![ProfileGpu {
            name: "Intel UHD Graphics 630".into(),
            vendor_id: Some("8086".into()),
            device_id: Some("3e98".into()),
            is_igpu: true,
            pci_path: Some("PciRoot(0x0)/Pci(0x2,0x0)".into()),
            ..ProfileGpu::default()
        }],
        audio: Some(ProfileAudio {
            codec_name: "Realtek ALC1220".into(),
            codec_id: Some(0x10ec_1220),
            controller_pci_path: Some("PciRoot(0x0)/Pci(0x1f,0x3)".into()),
            ..ProfileAudio::default()
        }),
        ethernet: vec![ProfileNic {
            name: "Intel Ethernet Connection (7) I219-V".into(),
            bus: DeviceBus::Pci,
            vendor_id: Some("8086".into()),
            device_id: Some("15bc".into()),
            pci_path: Some("PciRoot(0x0)/Pci(0x1f,0x6)".into()),
            mac_address: Some("18:C0:4D:12:34:56".into()),
            ..ProfileNic::default()
        }],
        motherboard_vendor: "Gigabyte Technology Co., Ltd.".into(),
        motherboard_model: "Z390 AORUS PRO".into(),
        chipset: Some("Z390".into()),
        ram_gb: 32,
        firmware_uefi: Some(true),
        source: "manual".into(),
        scan_confidence: 1.0,
        ..HardwareProfile::default()
    }
}

/// What the planner produces for that machine on macOS 15 (Dortania
/// "Coffee Lake" desktop guide), plus an optional kext and an optional SSDT
/// that cannot be provided, to exercise the skip path.
fn coffee_lake_plan() -> BuildPlan {
    let mut plan = base_plan("iMac19,1", MacOsVersion::Sequoia);
    plan.ssdts = vec![
        dortania("SSDT-PLUG-DRTNIA.aml", true),
        dortania("SSDT-EC-USBX-DESKTOP.aml", true),
        dortania("SSDT-AWAC.aml", true),
        dortania("SSDT-PMC.aml", true),
        SsdtPlan {
            file_name: "SSDT-ALS0.aml".into(),
            source: SsdtSource::OcSample { file: "SSDT-ALS0.aml".into() },
            required: false,
            reason: "sample".into(),
        },
        generated_table(),
        dortania("SSDT-GPIO.aml", false),
    ];
    plan.acpi_patches = vec![app_lib::domain::model::AcpiPatch {
        comment: "GPI0 fix - requires SSDT-GPIO.aml".into(),
        find: "5F535441".into(),
        replace: "58535441".into(),
        table_signature: None,
        oem_table_id: None,
        count: 0,
        enabled: true,
    }];
    plan.booter_quirks = settings(&[
        ("DevirtualiseMmio", PlistScalar::Bool(true)),
        ("EnableWriteUnprotector", PlistScalar::Bool(false)),
        ("ProtectUefiServices", PlistScalar::Bool(true)),
        ("RebuildAppleMemoryMap", PlistScalar::Bool(true)),
        ("SyncRuntimePermissions", PlistScalar::Bool(true)),
    ]);
    plan.kernel_quirks.insert("AppleXcpmCfgLock".into(), PlistScalar::Bool(true));
    plan.kernel_quirks.insert("DisableIoMapper".into(), PlistScalar::Bool(true));
    plan.device_properties = vec![
        DevicePropertyEntry {
            path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
            properties: vec![
                DeviceProperty { key: "AAPL,ig-platform-id".into(), value: PlistScalar::Data("07009B3E".into()) },
                DeviceProperty { key: "framebuffer-patch-enable".into(), value: PlistScalar::Data("01000000".into()) },
                DeviceProperty { key: "framebuffer-stolenmem".into(), value: PlistScalar::Data("00003001".into()) },
            ],
            reason: String::new(),
        },
        DevicePropertyEntry {
            path: "PciRoot(0x0)/Pci(0x1f,0x3)".into(),
            properties: vec![DeviceProperty { key: "layout-id".into(), value: PlistScalar::Data("07000000".into()) }],
            reason: String::new(),
        },
    ];
    plan.kexts = vec![
        kext("Lilu", "Lilu.kext", true, &[]),
        kext("VirtualSMC", "VirtualSMC.kext", true, &[]),
        kext("VirtualSMC", "SMCProcessor.kext", false, &[]),
        kext("VirtualSMC", "SMCSuperIO.kext", false, &[]),
        kext("WhateverGreen", "WhateverGreen.kext", true, &[]),
        kext("AppleALC", "AppleALC.kext", true, &[]),
        kext("IntelMausi", "IntelMausi.kext", false, &[]),
        kext("RestrictEvents", "RestrictEvents.kext", false, &[]),
        kext("NVMeFix", "NVMeFix.kext", false, &[]),
        kext("NotInTheCatalog", "Imaginary.kext", false, &[]),
    ];
    plan
}

fn options(target: MacOsVersion) -> BuildOptions {
    BuildOptions { target, picker: PickerStyle::Graphical, ..BuildOptions::default() }
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

fn kernel_add_paths(config: &Path) -> Vec<String> {
    let root = Value::from_file(config).unwrap();
    root.as_dictionary()
        .and_then(|d| d.get("Kernel"))
        .and_then(Value::as_dictionary)
        .and_then(|k| k.get("Add"))
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(|e| e.as_dictionary()?.get("BundlePath")?.as_string().map(str::to_string))
        .collect()
}

fn print_result(result: &app_lib::contracts::BuildResult) {
    println!("build {} at {}", result.build_id, result.efi_path);
    for k in &result.kexts {
        println!("  kext {:<28} {:?} {:?} {}", k.name, k.status, k.version, k.error.as_deref().unwrap_or(""));
    }
    for s in &result.ssdts {
        println!("  ssdt {:<28} {:?} ({})", s.file_name, s.status, s.source);
    }
    for w in &result.warnings {
        println!("  warning: {w}");
    }
    println!("{}", result.validation.ocvalidate_output.clone().unwrap_or_default());
    for i in &result.validation.issues {
        println!("  {:?} [{}] {}", i.level, i.source, i.message);
    }
}

#[tokio::test]
#[ignore]
async fn coffee_lake_desktop_builds_a_valid_efi() {
    let tmp = TempDir::new();
    let builds = tmp.0.join("builds");
    let work = tmp.0.join("work");
    let downloader = Downloader::new(tmp.0.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    let seen: Mutex<Vec<BuildProgress>> = Mutex::new(Vec::new());
    let sink = |p: BuildProgress| seen.lock().unwrap().push(p);
    let env = BuildEnv {
        builds_dir: &builds,
        work_dir: &work,
        downloader: &downloader,
        cancel: &cancel,
        progress: &sink,
        keep_builds: 1,
    };
    let profile = coffee_lake_profile();
    let result = build::run(&env, &profile, &options(MacOsVersion::Sequoia), coffee_lake_plan()).await.unwrap();
    print_result(&result);

    // ocvalidate from the same OpenCore release found nothing to complain about.
    assert!(result.validation.ocvalidate_ran, "ocvalidate should run on this host");
    assert!(result.validation.valid, "{:#?}", result.validation.issues);
    assert!(result.validation.issues.iter().all(|i| i.level != NoteLevel::Blocking));
    assert_eq!(result.opencore_version, "1.0.8");

    // Layout: only what the plan asked for.
    let efi = Path::new(&result.efi_path).join("EFI");
    assert!(efi.join("BOOT/BOOTx64.efi").is_file());
    assert!(efi.join("OC/OpenCore.efi").is_file());
    assert_eq!(
        listing(&efi.join("OC/Drivers")),
        ["HfsPlus.efi", "OpenCanopy.efi", "OpenRuntime.efi", "ResetNvramEntry.efi"]
    );
    assert_eq!(listing(&efi.join("OC/Tools")), ["OpenShell.efi"]);
    assert!(!listing(&efi.join("OC/Resources/Image")).is_empty(), "OpenCanopy resources");
    assert_eq!(
        listing(&efi.join("OC/ACPI")),
        [
            "SSDT-ALS0.aml",
            "SSDT-AWAC.aml",
            "SSDT-EC-USBX-DESKTOP.aml",
            "SSDT-PLUG-DRTNIA.aml",
            "SSDT-PMC.aml",
            "SSDT-TEST.aml"
        ]
    );
    assert!(Path::new(&result.efi_path).join("build.json").is_file());

    // The impossible optional pieces were left out, and so was the patch tied to the SSDT.
    let imaginary = result.kexts.iter().find(|k| k.catalog_id == "NotInTheCatalog").unwrap();
    assert_eq!(imaginary.status, ArtifactStatus::Skipped);
    assert!(result.plan.kexts.iter().all(|k| k.catalog_id != "NotInTheCatalog"));
    let gpio = result.ssdts.iter().find(|s| s.file_name == "SSDT-GPIO.aml").unwrap();
    assert_eq!(gpio.status, ArtifactStatus::Skipped);
    assert!(!result.plan.acpi_patches[0].enabled);
    assert_eq!(result.ssdts.iter().find(|s| s.file_name == "SSDT-TEST.aml").unwrap().status, ArtifactStatus::Generated);
    assert_eq!(result.ssdts.iter().find(|s| s.file_name == "SSDT-ALS0.aml").unwrap().status, ArtifactStatus::Bundled);
    assert!(result
        .kexts
        .iter()
        .filter(|k| k.catalog_id != "NotInTheCatalog")
        .all(|k| matches!(k.status, ArtifactStatus::Downloaded | ArtifactStatus::Cached)));

    // Kernel->Add is ordered by dependencies.
    let order = kernel_add_paths(Path::new(&result.config_plist_path));
    assert_eq!(order[0], "Lilu.kext");
    assert_eq!(order[1], "VirtualSMC.kext");
    assert!(order.contains(&"IntelMausi.kext".to_string()));

    // A real identity for the planned model, ROM from the I219's MAC.
    assert_eq!(result.identity.model, "iMac19,1");
    assert!(serial_format_valid(&result.identity.serial), "{}", result.identity.serial);
    assert!(mlb_checksum_valid(&result.identity.mlb), "{}", result.identity.mlb);
    assert_eq!(result.identity.rom, "18C04D123456");

    // Progress went through every phase that applies, in order, up to 100 %.
    let seen = seen.lock().unwrap().clone();
    let phases: Vec<Phase> = seen.iter().map(|p| p.phase).fold(Vec::new(), |mut v, p| {
        if v.last() != Some(&p) {
            v.push(p);
        }
        v
    });
    assert_eq!(
        phases,
        [
            Phase::OpenCore,
            Phase::Resources,
            Phase::Assemble,
            Phase::Kexts,
            Phase::Acpi,
            Phase::Config,
            Phase::Save,
            Phase::Validate
        ]
    );
    assert!(seen.windows(2).all(|w| w[0].fraction <= w[1].fraction + 1e-9));

    // validate_efi finds the matching ocvalidate, also after the scratch
    // space was wiped (from the download cache, without network).
    let build_dir = PathBuf::from(&result.efi_path);
    let target = build::validate::resolve_target(&build_dir.join("EFI/OC/config.plist")).unwrap();
    assert_eq!(target.build_dir.as_deref(), Some(build_dir.as_path()));
    assert!(build::validate::locate_ocvalidate(Some(&build_dir), &downloader, &work).await.is_some());
    std::fs::remove_dir_all(&work).unwrap();
    let ocvalidate = build::validate::locate_ocvalidate(Some(&build_dir), &downloader, &work).await;
    assert!(ocvalidate.is_some());
    let again_valid = app_lib::services::ocvalidate::validate_efi(&build_dir.join("EFI"), ocvalidate.as_deref()).await;
    assert!(again_valid.valid && again_valid.ocvalidate_ran);

    // Export never overwrites an existing EFI.
    let usb = tmp.0.join("usb");
    std::fs::create_dir_all(&usb).unwrap();
    let first = build::export::export_efi(&build_dir, &usb).unwrap();
    let second = build::export::export_efi(&build_dir, &usb).unwrap();
    assert_eq!(first, usb.canonicalize().unwrap().join("EFI"));
    assert_eq!(second.file_name().unwrap(), "EFI-2");
    assert!(second.join("OC/config.plist").is_file());

    // A rebuild reuses the identity, and only the newest build is kept.
    let mut again = options(MacOsVersion::Sequoia);
    again.identity = Some(result.identity.clone());
    let second = build::run(&env, &profile, &again, coffee_lake_plan()).await.unwrap();
    assert_eq!(second.identity, result.identity);
    assert!(second.kexts.iter().filter(|k| k.catalog_id == "Lilu").all(|k| k.status == ArtifactStatus::Cached));
    assert!(!Path::new(&result.efi_path).exists(), "the older build was pruned");
    assert_eq!(listing(&builds).len(), 1);
}

/// Ryzen 7 5800X on B550 with an RX 6800 XT: exercises the AMD_Vanilla step.
#[tokio::test]
#[ignore]
async fn ryzen_desktop_gets_amd_vanilla_patches() {
    let tmp = TempDir::new();
    let builds = tmp.0.join("builds");
    let work = tmp.0.join("work");
    let downloader = Downloader::new(tmp.0.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    let env = BuildEnv {
        builds_dir: &builds,
        work_dir: &work,
        downloader: &downloader,
        cancel: &cancel,
        progress: &build::progress::NoProgress,
        keep_builds: 5,
    };
    let profile = HardwareProfile {
        cpu: ProfileCpu {
            name: "AMD Ryzen 7 5800X 8-Core Processor".into(),
            vendor: CpuVendor::Amd,
            platform: CpuPlatform::AmdZen3,
            cores: 8,
            threads: 16,
            ..ProfileCpu::default()
        },
        chipset: Some("B550".into()),
        motherboard_model: "ROG STRIX B550-F GAMING".into(),
        ..HardwareProfile::default()
    };
    let mut plan = base_plan("MacPro7,1", MacOsVersion::Sonoma);
    plan.ssdts = vec![dortania("SSDT-EC-USBX-DESKTOP.aml", true), dortania("SSDT-CPUR.aml", true)];
    plan.booter_quirks = settings(&[
        ("EnableWriteUnprotector", PlistScalar::Bool(false)),
        ("RebuildAppleMemoryMap", PlistScalar::Bool(true)),
        ("SyncRuntimePermissions", PlistScalar::Bool(true)),
        ("SetupVirtualMap", PlistScalar::Bool(false)),
    ]);
    plan.kernel_quirks.insert("ProvideCurrentCpuInfo".into(), PlistScalar::Bool(true));
    plan.kernel_emulate = settings(&[("DummyPowerManagement", PlistScalar::Bool(true))]);
    plan.amd_core_count = Some(8);
    plan.kernel_patches = vec![];
    plan.boot_args.push("agdpmod=pikera".into());
    plan.kexts = vec![
        kext("Lilu", "Lilu.kext", true, &[]),
        kext("VirtualSMC", "VirtualSMC.kext", true, &[]),
        kext("WhateverGreen", "WhateverGreen.kext", true, &[]),
        kext("AppleALC", "AppleALC.kext", true, &[]),
        kext("AppleMCEReporterDisabler", "AppleMCEReporterDisabler.kext", true, &[]),
        kext("RealtekRTL8111-2.4.2", "RealtekRTL8111.kext", false, &[]),
    ];
    let result = build::run(&env, &profile, &options(MacOsVersion::Sonoma), plan).await.unwrap();
    print_result(&result);

    assert!(result.validation.ocvalidate_ran);
    assert!(result.validation.valid, "{:#?}", result.validation.issues);
    let patches: &[BinaryPatch] = &result.plan.kernel_patches;
    assert!(patches.len() >= 20, "AMD_Vanilla patches appended ({})", patches.len());
    let core_count: Vec<&BinaryPatch> =
        patches.iter().filter(|p| p.comment.contains("cpuid_cores_per_package")).collect();
    assert!(!core_count.is_empty());
    assert!(core_count.iter().all(|p| p.replace[2..4].eq_ignore_ascii_case("08")), "core count written");
    let pat_enabled: Vec<&str> = patches
        .iter()
        .filter(|p| p.comment.contains("_mtrr_update_action") && p.enabled)
        .map(|p| p.comment.as_str())
        .collect();
    assert!(!pat_enabled.is_empty() && pat_enabled.iter().all(|c| !c.to_ascii_lowercase().starts_with("shaneee")));
}

fn plist_at<'a>(root: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(root, |v, key| v.as_dictionary().and_then(|d| d.get(key)).unwrap_or_else(|| panic!("{key}")))
}

/// i5-7200U laptop: HD 620, PS/2 keyboard and trackpad, text picker, and
/// emulated NVRAM listed after OpenRuntime to exercise the LoadEarly rules.
#[tokio::test]
#[ignore]
async fn kaby_lake_laptop_with_text_picker_and_ps2_plugins() {
    let tmp = TempDir::new();
    let builds = tmp.0.join("builds");
    let downloader = Downloader::new(tmp.0.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    let seen: Mutex<Vec<BuildProgress>> = Mutex::new(Vec::new());
    let sink = |p: BuildProgress| seen.lock().unwrap().push(p);
    let env = BuildEnv {
        builds_dir: &builds,
        work_dir: &tmp.0.join("work"),
        downloader: &downloader,
        cancel: &cancel,
        progress: &sink,
        keep_builds: 5,
    };
    let profile = HardwareProfile {
        cpu: ProfileCpu {
            name: "Intel(R) Core(TM) i5-7200U CPU @ 2.50GHz".into(),
            vendor: CpuVendor::Intel,
            platform: CpuPlatform::KabyLake,
            cores: 2,
            threads: 4,
            is_mobile: true,
            ..ProfileCpu::default()
        },
        form_factor: FormFactor::Laptop,
        wifi: Some(ProfileNic {
            name: "Intel Dual Band Wireless-AC 8265".into(),
            bus: DeviceBus::Pci,
            mac_address: Some("A0:C5:89:12:34:56".into()),
            ..ProfileNic::default()
        }),
        ..HardwareProfile::default()
    };

    // Dortania "Kaby Lake (laptop)": MacBookPro14,1, HD 620 0x591B0000.
    let mut plan = base_plan("MacBookPro14,1", MacOsVersion::Ventura);
    plan.smbios.secure_boot_model = "Default".into();
    plan.misc_security.insert("SecureBootModel".into(), s("Default"));
    plan.misc_boot.insert("PickerMode".into(), s("Builtin"));
    plan.drivers = vec![
        driver("OpenHfsPlus.efi", "opencore"),
        driver("OpenRuntime.efi", "opencore"),
        driver("OpenVariableRuntimeDxe.efi", "opencore"),
        driver("ResetNvramEntry.efi", "opencore"),
        driver("OpenCanopy.efi", "opencore"),
    ];
    plan.ssdts = vec![
        dortania("SSDT-PLUG-DRTNIA.aml", true),
        dortania("SSDT-EC-USBX-LAPTOP.aml", true),
        dortania("SSDT-PNLF.aml", true),
        dortania("SSDT-XOSI.aml", true),
    ];
    plan.acpi_patches = vec![app_lib::domain::model::AcpiPatch {
        comment: "_OSI to XOSI rename - requires SSDT-XOSI.aml".into(),
        find: "5F4F5349".into(),
        replace: "584F5349".into(),
        table_signature: None,
        oem_table_id: None,
        count: 0,
        enabled: true,
    }];
    plan.kernel_quirks.insert("AppleXcpmCfgLock".into(), PlistScalar::Bool(true));
    plan.kernel_quirks.insert("DisableIoMapper".into(), PlistScalar::Bool(true));
    plan.device_properties = vec![DevicePropertyEntry {
        path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
        properties: vec![
            DeviceProperty { key: "AAPL,ig-platform-id".into(), value: PlistScalar::Data("00001B59".into()) },
            DeviceProperty { key: "framebuffer-patch-enable".into(), value: PlistScalar::Data("01000000".into()) },
            DeviceProperty { key: "framebuffer-stolenmem".into(), value: PlistScalar::Data("00003001".into()) },
            DeviceProperty { key: "framebuffer-fbmem".into(), value: PlistScalar::Data("00009000".into()) },
        ],
        reason: String::new(),
    }];
    plan.kexts = vec![
        kext("Lilu", "Lilu.kext", true, &[]),
        kext("VirtualSMC", "VirtualSMC.kext", true, &[]),
        kext("VirtualSMC", "SMCProcessor.kext", false, &[]),
        kext("VirtualSMC", "SMCBatteryManager.kext", false, &[]),
        kext("WhateverGreen", "WhateverGreen.kext", true, &[]),
        kext("AppleALC", "AppleALC.kext", false, &[]),
        kext("ECEnabler", "ECEnabler.kext", false, &[]),
        kext("BrightnessKeys", "BrightnessKeys.kext", false, &[]),
        kext(
            "VoodooPS2Controller",
            "VoodooPS2Controller.kext",
            true,
            &["VoodooInput.kext", "VoodooPS2Keyboard.kext", "VoodooPS2Mouse.kext", "VoodooPS2Trackpad.kext"],
        ),
    ];
    let opts = BuildOptions { target: MacOsVersion::Ventura, picker: PickerStyle::Text, ..BuildOptions::default() };
    let result = build::run(&env, &profile, &opts, plan).await.unwrap();
    print_result(&result);

    assert!(result.validation.ocvalidate_ran);
    assert!(result.validation.valid, "{:#?}", result.validation.issues);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);

    // The text picker needs neither OpenCanopy nor OcBinaryData.
    let efi = Path::new(&result.efi_path).join("EFI");
    assert_eq!(
        listing(&efi.join("OC/Drivers")),
        ["OpenHfsPlus.efi", "OpenRuntime.efi", "OpenVariableRuntimeDxe.efi", "ResetNvramEntry.efi"]
    );
    assert!(listing(&efi.join("OC/Resources")).is_empty());
    let phases: Vec<Phase> = seen.lock().unwrap().iter().map(|p| p.phase).collect();
    assert!(!phases.contains(&Phase::Resources));

    // Emulated NVRAM: OpenVariableRuntimeDxe first, both runtime drivers early.
    let config = Value::from_file(&result.config_plist_path).unwrap();
    let drivers: Vec<(String, bool)> = plist_at(&config, &["UEFI", "Drivers"])
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let d = d.as_dictionary().unwrap();
            (
                d.get("Path").and_then(Value::as_string).unwrap().to_string(),
                d.get("LoadEarly").and_then(Value::as_boolean).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        drivers,
        [
            ("OpenHfsPlus.efi".to_string(), false),
            ("OpenVariableRuntimeDxe.efi".to_string(), true),
            ("OpenRuntime.efi".to_string(), true),
            ("ResetNvramEntry.efi".to_string(), false),
        ]
    );
    assert_eq!(plist_at(&config, &["Misc", "Boot", "PickerMode"]).as_string(), Some("Builtin"));

    // Every PS/2 plugin is in Kernel->Add, after its parent.
    let order = kernel_add_paths(Path::new(&result.config_plist_path));
    let parent = order.iter().position(|p| p == "VoodooPS2Controller.kext").unwrap();
    for plugin in ["VoodooInput.kext", "VoodooPS2Keyboard.kext", "VoodooPS2Mouse.kext", "VoodooPS2Trackpad.kext"] {
        let path = format!("VoodooPS2Controller.kext/Contents/PlugIns/{plugin}");
        let at = order.iter().position(|p| *p == path).unwrap_or_else(|| panic!("{path} missing: {order:?}"));
        assert!(at > parent, "{path}");
    }
    assert!(order.iter().position(|p| p == "Lilu.kext") < order.iter().position(|p| p == "ECEnabler.kext"));

    // The XOSI rename stays on because its SSDT is there.
    assert!(result.plan.acpi_patches[0].enabled);
    // No Ethernet: the Wi-Fi card's MAC is the ROM.
    assert_eq!(result.identity.rom, "A0C589123456");
}
