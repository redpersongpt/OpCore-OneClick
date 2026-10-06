//! End-to-end check against OpenCore's own tools: downloads OpenCore 1.0.8
//! (or uses `OPENCORE_RELEASE_ZIP`), lets its macserial decode serials and
//! MLBs from the built-in generator, generates identities with macserial,
//! writes configs for a few typical machines and runs the matching
//! ocvalidate on each. Needs network access:
//! `cargo test --test config_identity_amd_ocvalidate -- --ignored --nocapture`
//! (`OC_KEEP_CONFIGS=1` keeps the generated configs in the temp directory).

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use app_lib::domain::amd_patches::{
    amd_vanilla_patches, pin_for_target, select_pat_patch, PatPatch, AMD_VANILLA, AMD_VANILLA_TAHOE,
};
use app_lib::domain::config_writer::{write_config, ConfigInputs, APPLE_BOOT_VARIABLE_GUID};
use app_lib::domain::kernel_add::KernelAddEntry;
use app_lib::domain::kext_catalog::Pin;
use app_lib::domain::model::{
    AcpiDelete, AcpiPatch, BinaryPatch, BuildPlan, DeviceProperty, DevicePropertyEntry, DriverPlan,
    KernelBlock, MacOsVersion, NvramVariable, PlatformIdentity, PlistScalar, SettingMap,
    SmbiosPlan, SsdtPlan, SsdtSource,
};
use app_lib::domain::smbios_gen::{generate_identity, mlb_checksum_valid, native_serial_and_mlb};

const OPENCORE_URL: &str =
    "https://github.com/acidanthera/OpenCorePkg/releases/download/1.0.8/OpenCore-1.0.8-RELEASE.zip";
const OPENCORE_SHA256: &str = "2011e8b7216ecb2645d97ea710df965947edd406846e92050b9bc4190be6d27b";
const SAMPLE: &[u8] = include_bytes!("fixtures/Sample-1.0.8.plist");

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn download(url: &str) -> Vec<u8> {
    reqwest::get(url)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec()
}

fn unzip_file(zip: &[u8], name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("{name} not in the OpenCore zip"));
    let mut out = Vec::new();
    file.read_to_end(&mut out).unwrap();
    out
}

fn write_executable(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn host_suffix() -> &'static str {
    if cfg!(windows) {
        ".exe"
    } else if cfg!(target_os = "linux") {
        ".linux"
    } else {
        ""
    }
}

fn s(v: &str) -> PlistScalar {
    PlistScalar::Str(v.to_string())
}

fn settings(items: &[(&str, PlistScalar)]) -> SettingMap {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn ssdt(name: &str) -> SsdtPlan {
    SsdtPlan {
        file_name: name.to_string(),
        source: SsdtSource::Dortania {
            file: name.to_string(),
        },
        required: true,
        reason: String::new(),
    }
}

fn driver(path: &str) -> DriverPlan {
    DriverPlan {
        path: path.to_string(),
        load_early: false,
        enabled: true,
        comment: String::new(),
        source: "opencore".to_string(),
    }
}

fn kext(bundle: &str, executable: &str) -> KernelAddEntry {
    let name = bundle.rsplit('/').next().unwrap_or(bundle);
    KernelAddEntry {
        arch: "x86_64".to_string(),
        bundle_path: bundle.to_string(),
        comment: name.to_string(),
        enabled: true,
        executable_path: executable.to_string(),
        max_kernel: String::new(),
        min_kernel: String::new(),
        plist_path: "Contents/Info.plist".to_string(),
        bundle_id: String::new(),
    }
}

fn base_plan(model: &str, target: MacOsVersion) -> BuildPlan {
    BuildPlan {
        target,
        smbios: SmbiosPlan {
            model: model.to_string(),
            reason: String::new(),
            secure_boot_model: "Default".to_string(),
            board_id_skip: false,
            alternatives: Vec::new(),
        },
        ssdts: Vec::new(),
        acpi_patches: Vec::new(),
        acpi_deletes: Vec::new(),
        acpi_quirks: SettingMap::new(),
        booter_quirks: SettingMap::new(),
        booter_patches: Vec::new(),
        mmio_whitelist: Vec::new(),
        device_properties: Vec::new(),
        kexts: Vec::new(),
        kernel_patches: Vec::new(),
        amd_core_count: None,
        kernel_blocks: Vec::new(),
        kernel_quirks: settings(&[
            ("PanicNoKextDump", PlistScalar::Bool(true)),
            ("PowerTimeoutKernelPanic", PlistScalar::Bool(true)),
        ]),
        kernel_emulate: SettingMap::new(),
        misc_boot: settings(&[
            ("PickerMode", s("External")),
            ("HideAuxiliary", PlistScalar::Bool(true)),
        ]),
        misc_debug: settings(&[
            ("AppleDebug", PlistScalar::Bool(true)),
            ("ApplePanic", PlistScalar::Bool(true)),
            ("DisableWatchDog", PlistScalar::Bool(true)),
            ("Target", PlistScalar::Int(67)),
        ]),
        misc_security: settings(&[
            ("AllowSetDefault", PlistScalar::Bool(true)),
            ("BlacklistAppleUpdate", PlistScalar::Bool(true)),
        ]),
        tools: vec!["OpenShell.efi".to_string()],
        boot_args: vec!["-v".into(), "keepsyms=1".into(), "debug=0x100".into()],
        csr_active_config: 0,
        nvram_add: Vec::new(),
        nvram_delete: Vec::new(),
        nvram_settings: settings(&[("WriteFlash", PlistScalar::Bool(true))]),
        platform_info: SettingMap::new(),
        drivers: vec![
            driver("HfsPlus.efi"),
            driver("OpenRuntime.efi"),
            driver("OpenCanopy.efi"),
            driver("ResetNvramEntry.efi"),
        ],
        uefi_quirks: SettingMap::new(),
        uefi_apfs: SettingMap::new(),
        uefi_output: SettingMap::new(),
        uefi_input: SettingMap::new(),
        bios_settings: Vec::new(),
        notes: Vec::new(),
        post_install: Vec::new(),
    }
}

struct Case {
    name: &'static str,
    plan: BuildPlan,
    kexts: Vec<KernelAddEntry>,
}

/// Coffee Lake desktop, UHD 630 + Intel NIC, macOS 15.
fn intel_desktop() -> Case {
    let mut plan = base_plan("iMac19,1", MacOsVersion::Sequoia);
    plan.smbios.secure_boot_model = "Disabled".into();
    plan.ssdts = vec![
        ssdt("SSDT-PLUG-DRTNIA.aml"),
        ssdt("SSDT-EC-USBX-DESKTOP.aml"),
        ssdt("SSDT-AWAC.aml"),
        ssdt("SSDT-PMC.aml"),
    ];
    plan.booter_quirks = settings(&[
        ("DevirtualiseMmio", PlistScalar::Bool(true)),
        ("EnableWriteUnprotector", PlistScalar::Bool(false)),
        ("ProtectUefiServices", PlistScalar::Bool(true)),
        ("RebuildAppleMemoryMap", PlistScalar::Bool(true)),
        ("SyncRuntimePermissions", PlistScalar::Bool(true)),
    ]);
    plan.kernel_quirks
        .insert("AppleXcpmCfgLock".into(), PlistScalar::Bool(true));
    plan.kernel_quirks
        .insert("DisableIoMapper".into(), PlistScalar::Bool(true));
    plan.device_properties = vec![
        DevicePropertyEntry {
            path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
            properties: vec![
                DeviceProperty {
                    key: "AAPL,ig-platform-id".into(),
                    value: PlistScalar::Data("07009B3E".into()),
                },
                DeviceProperty {
                    key: "framebuffer-patch-enable".into(),
                    value: PlistScalar::Data("01000000".into()),
                },
                DeviceProperty {
                    key: "framebuffer-stolenmem".into(),
                    value: PlistScalar::Data("00003001".into()),
                },
            ],
            reason: String::new(),
        },
        DevicePropertyEntry {
            path: "PciRoot(0x0)/Pci(0x1f,0x3)".into(),
            properties: vec![DeviceProperty {
                key: "layout-id".into(),
                value: PlistScalar::Data("01000000".into()),
            }],
            reason: String::new(),
        },
    ];
    plan.boot_args.push("alcid=1".into());
    plan.misc_security
        .insert("SecureBootModel".into(), s("Disabled"));
    plan.nvram_add = vec![NvramVariable {
        guid: "4D1FDA02-38C7-4A6A-9CC6-4BCCA8B30102".into(),
        key: "revpatch".into(),
        value: s("sbvmm"),
    }];
    Case {
        name: "intel-desktop",
        plan,
        kexts: vec![
            kext("Lilu.kext", "Contents/MacOS/Lilu"),
            kext("VirtualSMC.kext", "Contents/MacOS/VirtualSMC"),
            kext("SMCProcessor.kext", "Contents/MacOS/SMCProcessor"),
            kext("SMCSuperIO.kext", "Contents/MacOS/SMCSuperIO"),
            kext("WhateverGreen.kext", "Contents/MacOS/WhateverGreen"),
            kext("AppleALC.kext", "Contents/MacOS/AppleALC"),
            kext("RestrictEvents.kext", "Contents/MacOS/RestrictEvents"),
            kext("IntelMausi.kext", "Contents/MacOS/IntelMausi"),
            kext("USBMap.kext", ""),
        ],
    }
}

/// Kaby Lake-R laptop with a PS/2 keyboard and trackpad, macOS 13.
fn intel_laptop() -> Case {
    let mut plan = base_plan("MacBookPro14,1", MacOsVersion::Ventura);
    plan.ssdts = vec![
        ssdt("SSDT-PLUG-DRTNIA.aml"),
        ssdt("SSDT-EC-USBX-LAPTOP.aml"),
        ssdt("SSDT-PNLF.aml"),
        ssdt("SSDT-XOSI.aml"),
    ];
    plan.acpi_patches = vec![AcpiPatch {
        comment: "Change _OSI to XOSI".into(),
        find: "5F4F5349".into(),
        replace: "584F5349".into(),
        table_signature: None,
        oem_table_id: None,
        count: 0,
        enabled: true,
        ..Default::default()
    }];
    plan.booter_quirks = settings(&[
        ("EnableWriteUnprotector", PlistScalar::Bool(false)),
        ("RebuildAppleMemoryMap", PlistScalar::Bool(true)),
        ("SyncRuntimePermissions", PlistScalar::Bool(true)),
    ]);
    plan.kernel_quirks
        .insert("AppleXcpmCfgLock".into(), PlistScalar::Bool(true));
    plan.kernel_quirks
        .insert("DisableIoMapper".into(), PlistScalar::Bool(true));
    plan.device_properties = vec![DevicePropertyEntry {
        path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
        properties: vec![
            DeviceProperty {
                key: "AAPL,ig-platform-id".into(),
                value: PlistScalar::Data("0000C087".into()),
            },
            DeviceProperty {
                key: "device-id".into(),
                value: PlistScalar::Data("16590000".into()),
            },
            DeviceProperty {
                key: "framebuffer-patch-enable".into(),
                value: PlistScalar::Data("01000000".into()),
            },
            DeviceProperty {
                key: "framebuffer-stolenmem".into(),
                value: PlistScalar::Data("00003001".into()),
            },
            DeviceProperty {
                key: "framebuffer-fbmem".into(),
                value: PlistScalar::Data("00009000".into()),
            },
        ],
        reason: String::new(),
    }];
    plan.drivers.insert(1, driver("Ps2KeyboardDxe.efi"));
    plan.uefi_input = settings(&[("KeySupport", PlistScalar::Bool(true))]);
    let ps2 = "VoodooPS2Controller.kext";
    Case {
        name: "intel-laptop",
        plan,
        kexts: vec![
            kext("Lilu.kext", "Contents/MacOS/Lilu"),
            kext("VirtualSMC.kext", "Contents/MacOS/VirtualSMC"),
            kext("SMCBatteryManager.kext", "Contents/MacOS/SMCBatteryManager"),
            kext("SMCProcessor.kext", "Contents/MacOS/SMCProcessor"),
            kext("WhateverGreen.kext", "Contents/MacOS/WhateverGreen"),
            kext("AppleALC.kext", "Contents/MacOS/AppleALC"),
            kext("ECEnabler.kext", "Contents/MacOS/ECEnabler"),
            kext(ps2, "Contents/MacOS/VoodooPS2Controller"),
            kext(
                &format!("{ps2}/Contents/PlugIns/VoodooInput.kext"),
                "Contents/MacOS/VoodooInput",
            ),
            kext(
                &format!("{ps2}/Contents/PlugIns/VoodooPS2Keyboard.kext"),
                "Contents/MacOS/VoodooPS2Keyboard",
            ),
            kext(
                &format!("{ps2}/Contents/PlugIns/VoodooPS2Trackpad.kext"),
                "Contents/MacOS/VoodooPS2Trackpad",
            ),
            kext("BrightnessKeys.kext", "Contents/MacOS/BrightnessKeys"),
        ],
    }
}

/// Ryzen desktop with an RX 6600, MacPro7,1, AMD_Vanilla patches.
fn amd_desktop(name: &'static str, target: MacOsVersion, amd_patches: Vec<BinaryPatch>) -> Case {
    let mut plan = base_plan("MacPro7,1", target);
    plan.smbios.secure_boot_model = "Disabled".into();
    plan.ssdts = vec![ssdt("SSDT-EC-USBX-DESKTOP.aml"), ssdt("SSDT-CPUR.aml")];
    plan.booter_quirks = settings(&[
        ("EnableWriteUnprotector", PlistScalar::Bool(false)),
        ("RebuildAppleMemoryMap", PlistScalar::Bool(true)),
        ("SyncRuntimePermissions", PlistScalar::Bool(true)),
        ("ResizeAppleGpuBars", PlistScalar::Int(-1)),
    ]);
    plan.booter_patches = vec![BinaryPatch {
        comment: "Skip Board ID check".into(),
        arch: "x86_64".into(),
        identifier: "Apple".into(),
        base: String::new(),
        find:
            "0050006C006100740066006F0072006D0053007500700070006F00720074002E0070006C006900730074"
                .into(),
        mask: String::new(),
        replace:
            "002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E"
                .into(),
        replace_mask: String::new(),
        count: 0,
        limit: 0,
        skip: 0,
        min_kernel: String::new(),
        max_kernel: String::new(),
        enabled: false,
    }];
    plan.amd_core_count = Some(8);
    plan.kernel_patches = amd_patches;
    plan.kernel_blocks = vec![KernelBlock {
        // Typographic characters must not reach ocvalidate.
        comment: "Allow IOSkywalk Downgrade \u{2014} macOS \u{2265} 14".into(),
        identifier: "com.apple.iokit.IOSkywalkFamily".into(),
        strategy: "Exclude".into(),
        min_kernel: "23.0.0".into(),
        max_kernel: String::new(),
        enabled: false,
    }];
    plan.kernel_quirks
        .insert("ProvideCurrentCpuInfo".into(), PlistScalar::Bool(true));
    plan.kernel_emulate = settings(&[("DummyPowerManagement", PlistScalar::Bool(true))]);
    plan.misc_security
        .insert("SecureBootModel".into(), s("Disabled"));
    plan.boot_args
        .extend(["agdpmod=pikera".to_string(), "revpatch=sbvmm".to_string()]);
    plan.nvram_add = vec![
        NvramVariable {
            guid: APPLE_BOOT_VARIABLE_GUID.into(),
            key: "bluetoothExternalDongleFailed".into(),
            value: PlistScalar::Data("00".into()),
        },
        NvramVariable {
            guid: APPLE_BOOT_VARIABLE_GUID.into(),
            key: "bluetoothInternalControllerInfo".into(),
            value: PlistScalar::Data("0000000000000000000000000000".into()),
        },
    ];
    plan.device_properties = vec![DevicePropertyEntry {
        path: "PciRoot(0x0)/Pci(0x3,0x1)/Pci(0x0,0x0)/Pci(0x0,0x0)/Pci(0x0,0x0)".into(),
        properties: vec![DeviceProperty {
            key: "unfairgva".into(),
            value: PlistScalar::Int(1),
        }],
        reason: String::new(),
    }];
    Case {
        name,
        plan,
        kexts: vec![
            kext("Lilu.kext", "Contents/MacOS/Lilu"),
            kext("VirtualSMC.kext", "Contents/MacOS/VirtualSMC"),
            kext("WhateverGreen.kext", "Contents/MacOS/WhateverGreen"),
            kext("AppleALC.kext", "Contents/MacOS/AppleALC"),
            kext("RestrictEvents.kext", "Contents/MacOS/RestrictEvents"),
            kext("AppleMCEReporterDisabler.kext", ""),
            kext("RealtekRTL8111.kext", "Contents/MacOS/RealtekRTL8111"),
            kext("BlueToolFixup.kext", "Contents/MacOS/BlueToolFixup"),
        ],
    }
}

/// Ivy Bridge desktop on Catalina: CpuPm/Cpu0Ist deleted, legacy APFS
/// discovery, text picker (OpenCanopy.efi present but not loaded), emulated
/// NVRAM driver.
fn ivy_desktop() -> Case {
    let mut plan = base_plan("iMac13,2", MacOsVersion::Catalina);
    plan.smbios.secure_boot_model = "Disabled".into();
    plan.ssdts = vec![ssdt("SSDT-EC-USBX-DESKTOP.aml"), ssdt("SSDT-IMEI.aml")];
    plan.acpi_deletes = vec![
        AcpiDelete {
            comment: "Delete CpuPm".into(),
            table_signature: "SSDT".into(),
            oem_table_id: "CpuPm".into(),
            all: true,
        },
        AcpiDelete {
            comment: "Delete Cpu0Ist".into(),
            table_signature: "SSDT".into(),
            oem_table_id: "Cpu0Ist".into(),
            all: true,
        },
    ];
    plan.kernel_quirks
        .insert("AppleCpuPmCfgLock".into(), PlistScalar::Bool(true));
    plan.kernel_quirks
        .insert("AppleXcpmCfgLock".into(), PlistScalar::Bool(true));
    plan.kernel_quirks
        .insert("XhciPortLimit".into(), PlistScalar::Bool(true));
    plan.misc_boot = settings(&[
        ("PickerMode", s("Builtin")),
        ("Timeout", PlistScalar::Int(10)),
    ]);
    plan.misc_security
        .insert("SecureBootModel".into(), s("Disabled"));
    plan.uefi_apfs = settings(&[
        ("MinDate", PlistScalar::Int(-1)),
        ("MinVersion", PlistScalar::Int(-1)),
    ]);
    plan.drivers = vec![
        DriverPlan {
            load_early: true,
            ..driver("OpenVariableRuntimeDxe.efi")
        },
        DriverPlan {
            load_early: true,
            ..driver("OpenRuntime.efi")
        },
        driver("HfsPlus.efi"),
        driver("OpenCanopy.efi"),
        driver("ResetNvramEntry.efi"),
    ];
    plan.nvram_settings = settings(&[
        ("LegacyOverwrite", PlistScalar::Bool(true)),
        ("WriteFlash", PlistScalar::Bool(true)),
    ]);
    plan.platform_info = settings(&[("Generic/ProcessorType", PlistScalar::Int(0))]);
    Case {
        name: "ivy-desktop",
        plan,
        kexts: vec![
            kext("Lilu.kext", "Contents/MacOS/Lilu"),
            kext("VirtualSMC.kext", "Contents/MacOS/VirtualSMC"),
            kext("WhateverGreen.kext", "Contents/MacOS/WhateverGreen"),
            kext("AppleALC.kext", "Contents/MacOS/AppleALC"),
            kext("RealtekRTL8111.kext", "Contents/MacOS/RealtekRTL8111"),
        ],
    }
}

fn files_of(names: impl IntoIterator<Item = String>) -> Vec<String> {
    names.into_iter().collect()
}

/// Every SMBIOS a Hackintosh build may realistically use.
const NATIVE_MODELS: &[&str] = &[
    "iMac13,1",
    "iMac13,2",
    "iMac13,3",
    "iMac14,1",
    "iMac14,2",
    "iMac14,3",
    "iMac14,4",
    "iMac15,1",
    "iMac16,1",
    "iMac16,2",
    "iMac17,1",
    "iMac18,1",
    "iMac18,2",
    "iMac18,3",
    "iMac19,1",
    "iMac19,2",
    "iMac20,1",
    "iMac20,2",
    "iMacPro1,1",
    "MacPro5,1",
    "MacPro6,1",
    "MacPro7,1",
    "MacBookPro11,1",
    "MacBookPro11,2",
    "MacBookPro11,3",
    "MacBookPro11,4",
    "MacBookPro11,5",
    "MacBookPro12,1",
    "MacBookPro13,1",
    "MacBookPro13,2",
    "MacBookPro13,3",
    "MacBookPro14,1",
    "MacBookPro14,2",
    "MacBookPro14,3",
    "MacBookPro15,1",
    "MacBookPro15,2",
    "MacBookPro15,3",
    "MacBookPro15,4",
    "MacBookPro16,1",
    "MacBookPro16,2",
    "MacBookPro16,3",
    "MacBookPro16,4",
    "MacBookAir6,1",
    "MacBookAir6,2",
    "MacBookAir7,1",
    "MacBookAir7,2",
    "MacBookAir8,1",
    "MacBookAir8,2",
    "MacBookAir9,1",
    "Macmini6,1",
    "Macmini6,2",
    "Macmini7,1",
    "Macmini8,1",
    "MacBook8,1",
    "MacBook9,1",
    "MacBook10,1",
];

fn macserial_output(macserial: &Path, args: &[&str]) -> String {
    let output = Command::new(macserial).args(args).output().unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// macserial's own decoder must accept what the built-in generator produces:
/// known model, production year valid for it, correct MLB length and checksum.
fn check_native_identities(macserial: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for model in NATIVE_MODELS {
        for _ in 0..5 {
            let (serial, mlb) = native_serial_and_mlb(model).unwrap();
            let info = macserial_output(macserial, &["-i", &serial]);
            let model_line = info
                .lines()
                .find(|l| l.trim_start().starts_with("Model:"))
                .unwrap_or_default();
            if !info.contains("Valid: Possibly") || !model_line.ends_with(&format!(" - {model}")) {
                problems.push(format!("{model} {serial}:\n{info}"));
            }
            let length = if mlb.len() == 13 { "legacy" } else { "modern" };
            let verify = macserial_output(macserial, &["--verify", &mlb]);
            if !verify.contains("Valid MLB checksum.")
                || !verify.contains(&format!("Valid MLB length: {length}"))
            {
                problems.push(format!("{model} {mlb}:\n{verify}"));
            }
        }
    }
    problems
}

async fn amd_set(pin: Pin, pat: PatPatch) -> Vec<BinaryPatch> {
    let plist = download(pin.url).await;
    assert_eq!(Some(sha256_hex(&plist).as_str()), pin.sha256, "{}", pin.url);
    let mut patches = amd_vanilla_patches(&plist, 8).unwrap();
    select_pat_patch(&mut patches, pat);
    patches
}

#[tokio::test]
#[ignore = "downloads OpenCore 1.0.8 and AMD_Vanilla"]
async fn opencore_tools_accept_generated_output() {
    let zip = match std::env::var_os("OPENCORE_RELEASE_ZIP") {
        Some(path) => std::fs::read(path).unwrap(),
        None => download(OPENCORE_URL).await,
    };
    assert_eq!(sha256_hex(&zip), OPENCORE_SHA256, "OpenCore zip hash");
    assert_eq!(
        unzip_file(&zip, "Docs/Sample.plist"),
        SAMPLE,
        "fixture is the 1.0.8 Sample.plist"
    );

    let work: PathBuf = std::env::temp_dir().join(format!("oc-validate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).unwrap();
    let suffix = host_suffix();
    let ocvalidate = work.join(format!("ocvalidate{suffix}"));
    write_executable(
        &ocvalidate,
        &unzip_file(&zip, &format!("Utilities/ocvalidate/ocvalidate{suffix}")),
    );
    let macserial = work.join(format!("macserial{suffix}"));
    write_executable(
        &macserial,
        &unzip_file(&zip, &format!("Utilities/macserial/macserial{suffix}")),
    );

    let problems = check_native_identities(&macserial);
    assert!(
        problems.is_empty(),
        "macserial rejects native identities:\n{}",
        problems.join("\n")
    );
    println!(
        "== macserial accepts {} native serial/MLB pairs",
        NATIVE_MODELS.len() * 5
    );

    assert_eq!(pin_for_target(MacOsVersion::Sequoia).url, AMD_VANILLA.url);
    assert_eq!(
        pin_for_target(MacOsVersion::Tahoe).url,
        AMD_VANILLA_TAHOE.url
    );
    let amd_sequoia = amd_set(pin_for_target(MacOsVersion::Sequoia), PatPatch::Algrey).await;
    let amd_tahoe = amd_set(pin_for_target(MacOsVersion::Tahoe), PatPatch::Shaneee).await;

    let cases = [
        intel_desktop(),
        intel_laptop(),
        amd_desktop("amd-desktop-sequoia", MacOsVersion::Sequoia, amd_sequoia),
        amd_desktop("amd-desktop-tahoe", MacOsVersion::Tahoe, amd_tahoe),
        ivy_desktop(),
    ];
    let mut failures = Vec::new();
    for case in &cases {
        let identity: PlatformIdentity = generate_identity(
            &case.plan.smbios.model,
            Some(&macserial),
            Some("A4:83:E7:11:22:33"),
        )
        .unwrap();
        assert!(
            mlb_checksum_valid(&identity.mlb),
            "{} {:?}",
            case.name,
            identity
        );

        let ssdt_files = files_of(case.plan.ssdts.iter().map(|s| s.file_name.clone()));
        let driver_files = files_of(case.plan.drivers.iter().map(|d| d.path.clone()));
        let tool_files = files_of(case.plan.tools.iter().cloned());
        let xml = write_config(
            SAMPLE,
            &ConfigInputs {
                plan: &case.plan,
                kernel_add: &case.kexts,
                identity: &identity,
                ssdt_files: &ssdt_files,
                driver_files: &driver_files,
                tool_files: &tool_files,
            },
        )
        .unwrap_or_else(|e| panic!("{}: {e}", case.name));

        let config = work.join(format!("{}.plist", case.name));
        std::fs::write(&config, &xml).unwrap();
        let output = Command::new(&ocvalidate).arg(&config).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!(
            "== {} ({}, {} {})\n{}",
            case.name,
            case.plan.smbios.model,
            identity.serial,
            identity.mlb,
            stdout.trim()
        );
        if !stdout.contains("No issues found") {
            failures.push(case.name);
        }
    }
    // Negative control: ocvalidate must flag a config with illegal values.
    {
        let mut case = intel_desktop();
        case.plan
            .misc_security
            .insert("Vault".into(), s("Insecure"));
        case.plan
            .misc_boot
            .insert("HibernateMode".into(), s("Sometimes"));
        let identity = generate_identity(&case.plan.smbios.model, None, None).unwrap();
        let ssdt_files = files_of(case.plan.ssdts.iter().map(|s| s.file_name.clone()));
        let driver_files = files_of(case.plan.drivers.iter().map(|d| d.path.clone()));
        let tool_files = files_of(case.plan.tools.iter().cloned());
        let xml = write_config(
            SAMPLE,
            &ConfigInputs {
                plan: &case.plan,
                kernel_add: &case.kexts,
                identity: &identity,
                ssdt_files: &ssdt_files,
                driver_files: &driver_files,
                tool_files: &tool_files,
            },
        )
        .unwrap();
        let config = work.join("broken.plist");
        std::fs::write(&config, &xml).unwrap();
        let output = Command::new(&ocvalidate).arg(&config).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("== broken (negative control)\n{}", stdout.trim());
        assert!(
            !stdout.contains("No issues found")
                && stdout.contains("Vault is borked")
                && stdout.contains("HibernateMode is borked"),
            "ocvalidate missed the broken values"
        );
    }

    // Set OC_KEEP_CONFIGS=1 to inspect the generated files.
    if failures.is_empty() && std::env::var_os("OC_KEEP_CONFIGS").is_none() {
        let _ = std::fs::remove_dir_all(&work);
    }
    assert!(
        failures.is_empty(),
        "ocvalidate reported issues for {failures:?} (configs kept in {})",
        work.display()
    );
}
