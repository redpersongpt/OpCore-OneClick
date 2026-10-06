//! End-to-end check of the whole planner: for a set of realistic machines and
//! every release the compatibility report supports or offers as an expert
//! option, `planner::plan` must succeed and the config.plist written from the
//! plan (OpenCore 1.0.8 Sample.plist, Kernel->Add built from stand-in kext
//! bundles by `kernel_add`) must pass OpenCore's ocvalidate.
//!
//! Needs the ocvalidate binary of OpenCore 1.0.8:
//! `OCVALIDATE=/path/to/ocvalidate cargo test --test int_plan_ocvalidate -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::process::Command;

use app_lib::contracts::SupportLevel;
use app_lib::domain::config_writer::{write_config, ConfigInputs};
use app_lib::domain::model::{
    BuildOptions, BuildPlan, DeviceBus, FormFactor, HardwareProfile, InputBus, KextSelection,
    MacOsVersion, PlatformIdentity, ProfileAudio, ProfileCpu, ProfileGpu, ProfileInput, ProfileNic,
    ProfileStorage, StorageKind, TouchpadVendor, VmKind,
};
use app_lib::domain::{compatibility, cpu_db, gpu_db, kernel_add, planner};

const SAMPLE: &[u8] = include_bytes!("fixtures/Sample-1.0.8.plist");

struct Machine {
    name: &'static str,
    cpu: (&'static str, &'static str, u32, u32),
    form: FormFactor,
    gpus: Vec<(&'static str, &'static str, &'static str, bool)>,
    chipset: Option<&'static str>,
    vendor: &'static str,
    board: &'static str,
    codec: Option<u32>,
    ethernet: Vec<(&'static str, &'static str)>,
    wifi: Option<(&'static str, &'static str)>,
    bluetooth: Option<(&'static str, &'static str)>,
    touchpad: Option<(InputBus, TouchpadVendor, &'static str)>,
    sata: Option<(&'static str, &'static str)>,
    vm: Option<VmKind>,
    uefi: bool,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            name: "",
            cpu: ("", "GenuineIntel", 4, 8),
            form: FormFactor::Desktop,
            gpus: vec![],
            chipset: None,
            vendor: "",
            board: "",
            codec: None,
            ethernet: vec![],
            wifi: None,
            bluetooth: None,
            touchpad: None,
            sata: None,
            vm: None,
            uefi: true,
        }
    }
}

fn pci(vendor: &str, device: &str, path: &str) -> ProfileNic {
    ProfileNic {
        name: String::new(),
        bus: DeviceBus::Pci,
        vendor_id: Some(vendor.into()),
        device_id: Some(device.into()),
        subsystem_id: None,
        pci_path: Some(path.into()),
        mac_address: Some("00:1B:21:3A:4C:5D".into()),
    }
}

fn usb(vendor: &str, device: &str) -> ProfileNic {
    ProfileNic {
        bus: DeviceBus::Usb,
        pci_path: None,
        mac_address: None,
        ..pci(vendor, device, "")
    }
}

impl Machine {
    fn profile(&self) -> HardwareProfile {
        let (brand, vendor, cores, threads) = self.cpu;
        let id = cpu_db::identify(brand, vendor, None, None, None);
        let laptop = self.form == FormFactor::Laptop;
        let gpus = self
            .gpus
            .iter()
            .enumerate()
            .map(|(i, (vendor, device, name, igpu))| {
                let g = gpu_db::identify(Some(vendor), Some(device), name);
                ProfileGpu {
                    name: name.to_string(),
                    vendor: g.vendor,
                    family: g.family,
                    vendor_id: Some(vendor.to_string()),
                    device_id: Some(device.to_string()),
                    is_igpu: *igpu,
                    pci_path: Some(if *igpu {
                        "PciRoot(0x0)/Pci(0x2,0x0)".to_string()
                    } else {
                        format!("PciRoot(0x0)/Pci(0x1,0x{i})/Pci(0x0,0x0)")
                    }),
                    ..Default::default()
                }
            })
            .collect();
        HardwareProfile {
            cpu: ProfileCpu {
                name: brand.into(),
                vendor: id.vendor,
                platform: id.platform,
                codename: id.codename.clone(),
                cores,
                threads,
                is_mobile: id.is_mobile || laptop,
                is_hybrid: id.is_hybrid,
                ..Default::default()
            },
            form_factor: self.form,
            vm: self.vm,
            gpus,
            audio: self.codec.map(|codec_id| ProfileAudio {
                codec_name: format!("Codec {codec_id:08x}"),
                codec_id: Some(codec_id),
                controller_vendor_id: Some("8086".into()),
                controller_device_id: Some("a348".into()),
                controller_pci_path: Some("PciRoot(0x0)/Pci(0x1f,0x3)".into()),
                layout_id: None,
            }),
            ethernet: self
                .ethernet
                .iter()
                .enumerate()
                .map(|(i, (v, d))| pci(v, d, &format!("PciRoot(0x0)/Pci(0x1c,0x{i})/Pci(0x0,0x0)")))
                .collect(),
            wifi: self
                .wifi
                .map(|(v, d)| pci(v, d, "PciRoot(0x0)/Pci(0x1c,0x6)/Pci(0x0,0x0)")),
            bluetooth: self.bluetooth.map(|(v, d)| usb(v, d)),
            input: ProfileInput {
                keyboard_bus: if laptop { InputBus::Ps2 } else { InputBus::Usb },
                touchpad_bus: self.touchpad.map(|t| t.0),
                touchpad_vendor: self.touchpad.map(|t| t.1),
                touchpad_hid: self.touchpad.map(|t| t.2.to_string()),
                has_touchscreen: false,
            },
            storage: {
                let mut s = vec![ProfileStorage {
                    name: "Samsung SSD 970 EVO Plus".into(),
                    kind: StorageKind::Nvme,
                    vendor_id: Some("144d".into()),
                    device_id: Some("a808".into()),
                    size_bytes: Some(1_000_204_886_016),
                }];
                if let Some((v, d)) = self.sata {
                    s.push(ProfileStorage {
                        name: "SATA controller".into(),
                        kind: StorageKind::Sata,
                        vendor_id: Some(v.into()),
                        device_id: Some(d.into()),
                        size_bytes: None,
                    });
                }
                s
            },
            motherboard_vendor: self.vendor.into(),
            motherboard_model: self.board.into(),
            chipset: self.chipset.map(str::to_string),
            ram_gb: 32,
            has_battery: laptop,
            firmware_uefi: Some(self.uefi),
            source: "scan".into(),
            scan_confidence: 1.0,
            ..Default::default()
        }
    }
}

fn machines() -> Vec<Machine> {
    use FormFactor::*;
    vec![
        Machine {
            name: "coffee-lake-z390-rx580",
            cpu: (
                "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz",
                "GenuineIntel",
                8,
                8,
            ),
            gpus: vec![
                ("8086", "3e98", "Intel UHD Graphics 630", true),
                ("1002", "67df", "Radeon RX 580", false),
            ],
            chipset: Some("Z390"),
            vendor: "Gigabyte Technology Co., Ltd.",
            board: "Z390 AORUS PRO",
            codec: Some(0x10EC_1220),
            ethernet: vec![("8086", "15bc")],
            wifi: Some(("14e4", "43a0")),
            bluetooth: Some(("05ac", "828d")),
            ..Default::default()
        },
        Machine {
            name: "comet-lake-z490-igpu",
            cpu: (
                "Intel(R) Core(TM) i9-10900K CPU @ 3.70GHz",
                "GenuineIntel",
                10,
                20,
            ),
            gpus: vec![("8086", "9bc5", "Intel UHD Graphics 630", true)],
            chipset: Some("Z490"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "ROG STRIX Z490-E GAMING",
            codec: Some(0x10EC_1200),
            ethernet: vec![("8086", "15f3")],
            wifi: Some(("8086", "06f0")),
            bluetooth: Some(("8087", "0026")),
            ..Default::default()
        },
        Machine {
            name: "kaby-lake-dell-laptop",
            cpu: (
                "Intel(R) Core(TM) i7-7500U CPU @ 2.70GHz",
                "GenuineIntel",
                2,
                4,
            ),
            form: Laptop,
            gpus: vec![
                ("8086", "5916", "Intel HD Graphics 620", true),
                ("10de", "134d", "GeForce 940MX", false),
            ],
            chipset: Some("Sunrise Point-LP"),
            vendor: "Dell Inc.",
            board: "Inspiron 5567",
            codec: Some(0x10EC_0256),
            wifi: Some(("8086", "24fd")),
            bluetooth: Some(("8087", "0a2b")),
            touchpad: Some((InputBus::I2c, TouchpadVendor::Synaptics, "DLL07BE")),
            ..Default::default()
        },
        Machine {
            name: "haswell-h87-kepler",
            cpu: (
                "Intel(R) Core(TM) i5-4570 CPU @ 3.20GHz",
                "GenuineIntel",
                4,
                4,
            ),
            gpus: vec![
                ("8086", "0412", "Intel HD Graphics 4600", true),
                ("10de", "1187", "GeForce GTX 760", false),
            ],
            chipset: Some("H87"),
            vendor: "MSI",
            board: "H87-G43",
            codec: Some(0x10EC_0892),
            ethernet: vec![("10ec", "8168")],
            ..Default::default()
        },
        Machine {
            name: "ivy-bridge-z77-rx580",
            cpu: (
                "Intel(R) Core(TM) i7-3770K CPU @ 3.50GHz",
                "GenuineIntel",
                4,
                8,
            ),
            gpus: vec![
                ("8086", "0162", "Intel HD Graphics 4000", true),
                ("1002", "67df", "Radeon RX 580", false),
            ],
            chipset: Some("Z77"),
            vendor: "ASRock",
            board: "Z77 Extreme4",
            codec: Some(0x10EC_0892),
            ethernet: vec![("8086", "1503")],
            ..Default::default()
        },
        Machine {
            name: "sandy-bridge-z77-imei",
            cpu: (
                "Intel(R) Core(TM) i5-2500K CPU @ 3.30GHz",
                "GenuineIntel",
                4,
                4,
            ),
            gpus: vec![("8086", "0112", "Intel HD Graphics 3000", true)],
            chipset: Some("Z77"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "P8Z77-V LX",
            codec: Some(0x10EC_0887),
            ethernet: vec![("10ec", "8168")],
            sata: Some(("8086", "1e02")),
            ..Default::default()
        },
        Machine {
            name: "penryn-legacy-bios",
            cpu: (
                "Intel(R) Core(TM)2 Quad CPU Q9550 @ 2.83GHz",
                "GenuineIntel",
                4,
                4,
            ),
            gpus: vec![("1002", "67df", "Radeon RX 580", false)],
            chipset: Some("P45"),
            vendor: "Gigabyte Technology Co., Ltd.",
            board: "EP45-UD3P",
            codec: Some(0x10EC_0889),
            ethernet: vec![("10ec", "8168")],
            uefi: false,
            ..Default::default()
        },
        Machine {
            name: "zen2-x570-aquantia-navi23",
            cpu: ("AMD Ryzen 5 3600 6-Core Processor", "AuthenticAMD", 6, 12),
            gpus: vec![("1002", "73ff", "Radeon RX 6600 XT", false)],
            chipset: Some("X570"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "ProArt X570-CREATOR WIFI",
            codec: Some(0x10EC_1220),
            ethernet: vec![("1d6a", "07b1"), ("8086", "15f3")],
            wifi: Some(("8086", "2723")),
            bluetooth: Some(("8087", "0029")),
            ..Default::default()
        },
        Machine {
            name: "zen3-laptop-nootedred",
            cpu: (
                "AMD Ryzen 7 5800H with Radeon Graphics",
                "AuthenticAMD",
                8,
                16,
            ),
            form: Laptop,
            gpus: vec![("1002", "1638", "AMD Radeon Graphics", true)],
            chipset: Some("FCH"),
            vendor: "LENOVO",
            board: "LNVNB161216",
            codec: Some(0x10EC_0257),
            wifi: Some(("8086", "2723")),
            bluetooth: Some(("8087", "0029")),
            touchpad: Some((InputBus::I2c, TouchpadVendor::Elan, "ELAN0001")),
            ..Default::default()
        },
        Machine {
            name: "fx-8350-990fx",
            cpu: ("AMD FX(tm)-8350 Eight-Core Processor", "AuthenticAMD", 4, 8),
            gpus: vec![("1002", "67df", "Radeon RX 580", false)],
            chipset: Some("990FX"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "SABERTOOTH 990FX R2.0",
            codec: Some(0x10EC_0892),
            ethernet: vec![("10ec", "8168")],
            ..Default::default()
        },
        Machine {
            name: "zen4-am5-navi21",
            cpu: (
                "AMD Ryzen 9 7950X 16-Core Processor",
                "AuthenticAMD",
                16,
                32,
            ),
            gpus: vec![
                ("1002", "164e", "AMD Radeon Graphics", true),
                ("1002", "73bf", "Radeon RX 6800 XT", false),
            ],
            chipset: Some("X670E"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "ROG CROSSHAIR X670E HERO",
            codec: Some(0x10EC_4082),
            ethernet: vec![("8086", "125c")],
            ..Default::default()
        },
        Machine {
            name: "threadripper-trx40-vega",
            cpu: (
                "AMD Ryzen Threadripper 3970X 32-Core Processor",
                "AuthenticAMD",
                32,
                64,
            ),
            gpus: vec![("1002", "687f", "Radeon RX Vega 64", false)],
            chipset: Some("TRX40"),
            vendor: "Gigabyte Technology Co., Ltd.",
            board: "TRX40 AORUS MASTER",
            codec: Some(0x10EC_1220),
            ethernet: vec![("8086", "1539")],
            ..Default::default()
        },
        Machine {
            name: "skylake-x-x299-asus",
            cpu: (
                "Intel(R) Core(TM) i9-7900X CPU @ 3.30GHz",
                "GenuineIntel",
                10,
                20,
            ),
            gpus: vec![("1002", "687f", "Radeon RX Vega 56", false)],
            chipset: Some("X299"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "PRIME X299-A",
            codec: Some(0x10EC_1220),
            ethernet: vec![("8086", "15b8")],
            ..Default::default()
        },
        Machine {
            name: "haswell-e-x99",
            cpu: (
                "Intel(R) Core(TM) i7-5820K CPU @ 3.30GHz",
                "GenuineIntel",
                6,
                12,
            ),
            gpus: vec![("1002", "67df", "Radeon RX 580", false)],
            chipset: Some("X99"),
            vendor: "MSI",
            board: "X99A SLI PLUS",
            codec: Some(0x10EC_1150),
            ethernet: vec![("8086", "15a1")],
            ..Default::default()
        },
        Machine {
            name: "alder-lake-z690-navi21",
            cpu: (
                "12th Gen Intel(R) Core(TM) i7-12700K",
                "GenuineIntel",
                12,
                20,
            ),
            gpus: vec![
                ("8086", "4680", "Intel UHD Graphics 770", true),
                ("1002", "73bf", "Radeon RX 6800 XT", false),
            ],
            chipset: Some("Z690"),
            vendor: "Micro-Star International Co., Ltd.",
            board: "PRO Z690-A WIFI",
            codec: Some(0x10EC_0897),
            ethernet: vec![("8086", "15f3")],
            wifi: Some(("8086", "2725")),
            bluetooth: Some(("8087", "0032")),
            ..Default::default()
        },
        Machine {
            name: "arrow-lake-z890-navi23",
            cpu: ("Intel(R) Core(TM) Ultra 7 265K", "GenuineIntel", 20, 20),
            gpus: vec![("1002", "73ff", "Radeon RX 6600", false)],
            chipset: Some("Z890"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "PRIME Z890-P",
            codec: Some(0x10EC_0897),
            ethernet: vec![("10ec", "8125")],
            ..Default::default()
        },
        Machine {
            name: "ice-lake-laptop",
            cpu: (
                "Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz",
                "GenuineIntel",
                4,
                8,
            ),
            form: Laptop,
            gpus: vec![("8086", "8a52", "Intel Iris Plus Graphics", true)],
            chipset: Some("Ice Lake-LP"),
            vendor: "HP",
            board: "HP Spectre x360 Convertible 13-aw0xxx",
            codec: Some(0x10EC_0285),
            wifi: Some(("8086", "34f0")),
            bluetooth: Some(("8087", "0026")),
            touchpad: Some((InputBus::I2c, TouchpadVendor::Synaptics, "SYNA3290")),
            ..Default::default()
        },
        Machine {
            name: "broadwell-nuc",
            cpu: (
                "Intel(R) Core(TM) i5-5250U CPU @ 1.60GHz",
                "GenuineIntel",
                2,
                4,
            ),
            form: MiniPc,
            gpus: vec![("8086", "1626", "Intel HD Graphics 6000", true)],
            vendor: "Intel Corporation",
            board: "NUC5i5RYB",
            codec: Some(0x10EC_0283),
            ethernet: vec![("8086", "15a2")],
            wifi: Some(("8086", "095a")),
            ..Default::default()
        },
        Machine {
            name: "x58-kepler",
            cpu: (
                "Intel(R) Xeon(R) CPU X5675 @ 3.07GHz",
                "GenuineIntel",
                6,
                12,
            ),
            gpus: vec![("10de", "1180", "GeForce GTX 680", false)],
            chipset: Some("X58"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "P6T DELUXE V2",
            codec: Some(0x1106_0441),
            ethernet: vec![("11ab", "4363")],
            ..Default::default()
        },
        Machine {
            name: "comet-lake-laptop-broadcom-legacy",
            cpu: (
                "Intel(R) Core(TM) i7-10750H CPU @ 2.60GHz",
                "GenuineIntel",
                6,
                12,
            ),
            form: Laptop,
            gpus: vec![
                ("8086", "9bc4", "Intel UHD Graphics", true),
                ("10de", "1f95", "GeForce GTX 1650 Ti", false),
            ],
            chipset: Some("HM470"),
            vendor: "ASUSTeK COMPUTER INC.",
            board: "ROG Strix G512LI",
            codec: Some(0x10EC_0294),
            ethernet: vec![("10ec", "8168")],
            wifi: Some(("14e4", "43b1")),
            touchpad: Some((InputBus::I2c, TouchpadVendor::Elan, "ELAN1200")),
            ..Default::default()
        },
        Machine {
            name: "kvm-haswell",
            cpu: (
                "Intel Core Processor (Haswell, no TSX)",
                "GenuineIntel",
                4,
                8,
            ),
            gpus: vec![("1234", "1111", "QEMU Standard VGA", false)],
            vendor: "QEMU",
            board: "Standard PC (Q35 + ICH9, 2009)",
            ethernet: vec![("1af4", "1000")],
            vm: Some(VmKind::Kvm),
            ..Default::default()
        },
        Machine {
            name: "hyper-v",
            cpu: (
                "Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz",
                "GenuineIntel",
                6,
                12,
            ),
            gpus: vec![("1414", "5353", "Microsoft Hyper-V Video", false)],
            vendor: "Microsoft Corporation",
            board: "Virtual Machine",
            vm: Some(VmKind::HyperV),
            ..Default::default()
        },
        Machine {
            name: "vmware-zen3",
            cpu: ("AMD Ryzen 7 5800X 8-Core Processor", "AuthenticAMD", 8, 16),
            gpus: vec![("15ad", "0405", "VMware SVGA II Adapter", false)],
            vendor: "VMware, Inc.",
            board: "440BX Desktop Reference Platform",
            ethernet: vec![("15ad", "07b0")],
            vm: Some(VmKind::Vmware),
            ..Default::default()
        },
    ]
}

/// Kexts ocvalidate expects to be codeless (Utilities/ocvalidate/KextInfo.c),
/// plus other codeless injectors the planner selects.
const CODELESS: &[&str] = &[
    "AirPortBrcm4360_Injector.kext",
    "AirPortBrcmNIC_Injector.kext",
    "CPUFriendDataProvider.kext",
    "BrcmBluetoothInjector.kext",
    "BrcmBluetoothInjectorLegacy.kext",
    "Legacy_USB3.kext",
    "Legacy_InternalHub-EHCx.kext",
    "WebCamera.kext",
    "SATA-unsupported.kext",
    "AppleMCEReporterDisabler.kext",
];

/// Bundle id and OSBundleLibraries of a stand-in, following the dependency
/// table ocvalidate checks the Kernel->Add order against.
fn identity_of(name: &str) -> (String, Vec<&'static str>) {
    const LILU: &str = "as.vit9696.Lilu";
    const VSMC: &str = "as.vit9696.VirtualSMC";
    let stem = name.trim_end_matches(".kext");
    match stem {
        "Lilu" => (LILU.into(), vec![]),
        "VirtualSMC" => (VSMC.into(), vec![LILU]),
        _ if stem.starts_with("SMC") => (format!("org.test.{stem}"), vec![LILU, VSMC]),
        "CPUFriendDataProvider" => (format!("org.test.{stem}"), vec!["org.test.CPUFriend"]),
        "WhateverGreen"
        | "AppleALC"
        | "AppleALCU"
        | "AirportBrcmFixup"
        | "BrightnessKeys"
        | "CpuTscSync"
        | "CPUFriend"
        | "CryptexFixup"
        | "DebugEnhancer"
        | "HibernationFixup"
        | "NVMeFix"
        | "RestrictEvents"
        | "RTCMemoryFixup"
        | "FeatureUnlock"
        | "MacHyperVSupport"
        | "MacHyperVSupportMonterey"
        | "BlueToolFixup"
        | "NootedRed"
        | "NootRX"
        | "AMFIPass"
        | "ECEnabler"
        | "CpuTopologyRebuild"
        | "AirportItlwm" => (format!("org.test.{stem}"), vec![LILU]),
        _ => (format!("org.test.{stem}"), vec![]),
    }
}

/// Stand-in bundles for every selected kext and plugin, so `kernel_add`
/// builds Kernel->Add from real Info.plists the way the build does.
fn stage_kexts(dir: &Path, kexts: &[KextSelection]) {
    let write = |bundle_dir: &Path, name: &str| {
        let stem = name.trim_end_matches(".kext");
        let contents = bundle_dir.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        let executable = if CODELESS.contains(&name) {
            String::new()
        } else {
            std::fs::create_dir_all(contents.join("MacOS")).unwrap();
            std::fs::write(contents.join("MacOS").join(stem), b"\xCF\xFA\xED\xFE").unwrap();
            format!("<key>CFBundleExecutable</key><string>{stem}</string>")
        };
        let (id, libs) = identity_of(name);
        let libs: String = libs
            .iter()
            .map(|l| format!("<key>{l}</key><string>1.0.0</string>"))
            .collect();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>{id}</string>{executable}\
             <key>CFBundleVersion</key><string>1.0.0</string>\
             <key>OSBundleLibraries</key><dict>{libs}</dict></dict></plist>\n"
        );
        std::fs::write(contents.join("Info.plist"), plist).unwrap();
    };
    for k in kexts {
        let bundle_dir = dir.join(&k.bundle);
        if !bundle_dir.exists() {
            write(&bundle_dir, &k.bundle);
        }
        for p in &k.plugins {
            let plugin_dir = bundle_dir.join("Contents").join("PlugIns").join(&p.bundle);
            if !plugin_dir.exists() {
                write(&plugin_dir, &p.bundle);
            }
        }
    }
}

fn write_plan_config(plan: &BuildPlan, work: &Path) -> Vec<u8> {
    let kexts_dir = work.join("Kexts");
    let _ = std::fs::remove_dir_all(&kexts_dir);
    std::fs::create_dir_all(&kexts_dir).unwrap();
    stage_kexts(&kexts_dir, &plan.kexts);
    let kernel_add = kernel_add::build_kernel_add(&plan.kexts, &kexts_dir).unwrap();
    let identity = PlatformIdentity {
        model: plan.smbios.model.clone(),
        serial: "C02XG0FDH7JY".into(),
        mlb: "C02839303QXH69FJA".into(),
        system_uuid: "DBB364D6-44B2-4A02-B922-AB4396F16DA8".into(),
        rom: "112233445566".into(),
    };
    let ssdt_files: Vec<String> = plan.ssdts.iter().map(|s| s.file_name.clone()).collect();
    let driver_files: Vec<String> = plan.drivers.iter().map(|d| d.path.clone()).collect();
    write_config(
        SAMPLE,
        &ConfigInputs {
            plan,
            kernel_add: &kernel_add,
            identity: &identity,
            ssdt_files: &ssdt_files,
            driver_files: &driver_files,
            tool_files: &plan.tools,
        },
    )
    .unwrap()
}

#[test]
#[ignore]
fn every_offered_release_plans_and_passes_ocvalidate() {
    let Some(ocvalidate) = std::env::var_os("OCVALIDATE").map(PathBuf::from) else {
        eprintln!("OCVALIDATE is not set; skipping");
        return;
    };
    let work = std::env::temp_dir().join(format!("int-plan-ocvalidate-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for machine in machines() {
        let profile = machine.profile();
        let mut offered = 0;
        for target in MacOsVersion::ALL {
            let report = compatibility::assess(&profile, Some(target));
            let supported = report
                .versions
                .iter()
                .any(|o| o.version == target && o.supported);
            if !supported && report.level != SupportLevel::Partial {
                continue;
            }
            offered += 1;
            let label = format!("{} {}", machine.name, target.id());
            let options = BuildOptions {
                target,
                ..Default::default()
            };
            let plan = match planner::plan(&profile, &options) {
                Ok(plan) => plan,
                Err(e) => {
                    failures.push(format!("{label}: plan failed: {} {}", e.code, e.message));
                    continue;
                }
            };
            let config = write_plan_config(&plan, &work);
            let path = work.join(format!("{}-{}.plist", machine.name, target.id()));
            std::fs::write(&path, &config).unwrap();
            let out = Command::new(&ocvalidate).arg(&path).output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            checked += 1;
            if !stdout.contains("No issues found") {
                failures.push(format!("{label} ({}):\n{stdout}", path.display()));
            }
        }
        assert!(offered > 0, "{}: no release offered", machine.name);
        println!("{}: {offered} releases", machine.name);
    }
    println!("ocvalidate checked {checked} configs");
    if std::env::var_os("KEEP_CONFIGS").is_some() || !failures.is_empty() {
        println!("configs kept in {}", work.display());
    } else {
        let _ = std::fs::remove_dir_all(&work);
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
