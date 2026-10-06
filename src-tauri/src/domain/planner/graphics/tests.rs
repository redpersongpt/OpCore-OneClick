use super::*;
use crate::domain::kext_catalog;
use crate::domain::model::{
    BuildOptions, CpuPlatform, CpuVendor, FormFactor, HardwareProfile, MacOsVersion, ProfileCpu,
    VmKind,
};
use crate::domain::planner::{empty_plan, PlanContext};

use CpuPlatform::*;
use MacOsVersion::*;

const PEG: &str = "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)";
const PEG2: &str = "PciRoot(0x0)/Pci(0x1,0x1)/Pci(0x0,0x0)";
const APU: &str = "PciRoot(0x0)/Pci(0x8,0x1)/Pci(0x0,0x0)";

// ── Fixtures ────────────────────────────────────────────────────────────────

fn gpu(vendor: &str, device: &str, name: &str, path: Option<&str>) -> ProfileGpu {
    let id = gpu_db::identify(Some(vendor), Some(device), name);
    ProfileGpu {
        name: name.to_string(),
        vendor: id.vendor,
        family: id.family,
        vendor_id: Some(vendor.to_string()),
        device_id: Some(device.to_string()),
        is_igpu: id.is_igpu,
        pci_path: path.map(str::to_string),
        ..Default::default()
    }
}

fn intel(device: &str, name: &str) -> ProfileGpu {
    gpu("8086", device, name, Some(IGPU_PATH))
}

fn amd(device: &str, name: &str) -> ProfileGpu {
    gpu("1002", device, name, Some(PEG))
}

fn apu(device: &str) -> ProfileGpu {
    gpu("1002", device, "AMD Radeon(TM) Graphics", Some(APU))
}

fn nvidia(device: &str, name: &str) -> ProfileGpu {
    gpu("10de", device, name, Some(PEG))
}

fn at(mut gpu: ProfileGpu, path: Option<&str>) -> ProfileGpu {
    gpu.pci_path = path.map(str::to_string);
    gpu
}

fn machine(platform: CpuPlatform, form: FormFactor, gpus: Vec<ProfileGpu>) -> HardwareProfile {
    let vendor = match platform {
        AmdK10 | AmdBulldozer | AmdJaguar | AmdZen | AmdZen2 | AmdZen3 | AmdZen4 | AmdZen5 => {
            CpuVendor::Amd
        }
        _ => CpuVendor::Intel,
    };
    HardwareProfile {
        cpu: ProfileCpu {
            name: "Test CPU".into(),
            vendor,
            platform,
            is_mobile: matches!(form, FormFactor::Laptop | FormFactor::MiniPc),
            ..Default::default()
        },
        form_factor: form,
        gpus,
        ..Default::default()
    }
}

fn desktop(platform: CpuPlatform, gpus: Vec<ProfileGpu>) -> HardwareProfile {
    machine(platform, FormFactor::Desktop, gpus)
}

fn laptop(platform: CpuPlatform, gpus: Vec<ProfileGpu>) -> HardwareProfile {
    machine(platform, FormFactor::Laptop, gpus)
}

fn vm(gpus: Vec<ProfileGpu>) -> HardwareProfile {
    let mut p = desktop(CometLake, gpus);
    p.vm = Some(VmKind::Kvm);
    p
}

fn options(target: MacOsVersion) -> BuildOptions {
    BuildOptions {
        target,
        ..Default::default()
    }
}

struct Run {
    display: DisplayPlan,
    plan: BuildPlan,
    root_patch: bool,
    kext: GpuKext,
}

fn run(profile: &HardwareProfile, target: MacOsVersion, model: &str) -> Run {
    run_with(profile, &options(target), model)
}

fn run_with(profile: &HardwareProfile, options: &BuildOptions, model: &str) -> Run {
    let ctx = PlanContext::new(profile, options);
    let display = choose_display(&ctx).expect("a display path");
    let mut plan = empty_plan(options.target);
    // Written by the smbios stage, which runs before graphics::apply.
    plan.smbios.model = model.to_string();
    apply(&ctx, &display, &mut plan);
    Run {
        root_patch: needs_root_patch_graphics(&ctx, &display),
        kext: gpu_kext(&ctx, &display),
        display,
        plan,
    }
}

fn fail(profile: &HardwareProfile, target: MacOsVersion) -> AppError {
    let options = options(target);
    let ctx = PlanContext::new(profile, &options);
    choose_display(&ctx).expect_err("no display path")
}

impl Run {
    fn entry(&self, path: &str) -> Option<&DevicePropertyEntry> {
        self.plan.device_properties.iter().find(|e| e.path == path)
    }

    /// Data (hex) or string value of one property.
    fn prop(&self, path: &str, key: &str) -> Option<String> {
        self.plan
            .device_properties
            .iter()
            .filter(|e| e.path == path)
            .flat_map(|e| &e.properties)
            .find(|p| p.key == key)
            .map(|p| match &p.value {
                PlistScalar::Data(hex) | PlistScalar::Str(hex) => hex.clone(),
                other => format!("{other:?}"),
            })
    }

    fn keys(&self, path: &str) -> Vec<&str> {
        self.plan
            .device_properties
            .iter()
            .filter(|e| e.path == path)
            .flat_map(|e| &e.properties)
            .map(|p| p.key.as_str())
            .collect()
    }

    fn has_arg(&self, arg: &str) -> bool {
        self.plan.boot_args.iter().any(|a| a == arg)
    }

    fn arg(&self, key: &str) -> Option<&str> {
        self.plan
            .boot_args
            .iter()
            .find(|a| a.split('=').next() == Some(key))
            .map(String::as_str)
    }

    fn kexts(&self) -> Vec<&str> {
        self.plan
            .kexts
            .iter()
            .map(|k| k.catalog_id.as_str())
            .collect()
    }

    fn noted(&self, needle: &str) -> bool {
        self.plan
            .notes
            .iter()
            .chain(&self.plan.post_install)
            .any(|n| n.title.contains(needle) || n.detail.contains(needle))
    }
}

fn display(
    primary: Option<usize>,
    igpu: Option<usize>,
    headless: bool,
    disabled: &[usize],
) -> DisplayPlan {
    DisplayPlan {
        primary,
        igpu,
        igpu_headless: headless,
        disabled: disabled.to_vec(),
    }
}

// ── Intel desktops ──────────────────────────────────────────────────────────

#[test]
fn coffee_lake_uhd630_alone_drives_the_displays() {
    let p = desktop(CoffeeLake, vec![intel("3e92", "Intel UHD Graphics 630")]);
    let r = run(&p, Sequoia, "iMac19,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "framebuffer-patch-enable",
            "framebuffer-stolenmem"
        ]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("07009B3E")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-patch-enable").as_deref(),
        Some("01000000")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-stolenmem").as_deref(),
        Some("00003001")
    );
    assert_eq!(r.kexts(), ["WhateverGreen"]);
    assert!(r.plan.boot_args.is_empty());
    assert!(!r.root_patch);
}

#[test]
fn i9_9900k_with_rx580_keeps_the_igpu_headless_without_pikera() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e98", "Intel UHD Graphics 630"),
            amd("67df", "Radeon RX 580"),
        ],
    );
    let r = run(&p, Sequoia, "iMac19,1");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(r.keys(IGPU_PATH), ["AAPL,ig-platform-id"]);
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300913E")
    );
    assert!(r.entry(PEG).is_none());
    // Polaris never takes agdpmod=pikera.
    assert!(r.arg("agdpmod").is_none());
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
    assert!(!r.plan.kexts[1].required);
    assert!(r.noted("runs headless"));
}

#[test]
fn i7_10700f_with_rx6600xt_takes_pikera_only_on_imac_boards() {
    let p = desktop(CometLake, vec![amd("73ff", "Radeon RX 6600 XT")]);
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert!(r.arg("agdpmod").is_none());
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
    assert!(r.plan.device_properties.is_empty());

    let r = run(&p, Sequoia, "iMacPro1,1");
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=pikera"));
}

#[test]
fn navi_on_tahoe_uses_agdpmod_ignore() {
    let p = desktop(CometLake, vec![amd("73ff", "Radeon RX 6600 XT")]);
    let r = run(&p, Tahoe, "MacPro7,1");
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=ignore"));
    assert!(r.plan.kernel_patches.is_empty());

    let p = desktop(
        CometLake,
        vec![
            intel("9bc5", "Intel UHD Graphics 630"),
            amd("731f", "Radeon RX 5700 XT"),
        ],
    );
    let r = run(&p, Tahoe, "iMac20,1");
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=ignore"));
    let patch = &r.plan.kernel_patches[0];
    assert_eq!(
        patch.identifier,
        "com.apple.driver.AppleGraphicsDevicePolicy"
    );
    assert_eq!(
        (patch.find.as_str(), patch.replace.as_str()),
        ("626F6172642D6964", "626F6172642D6978")
    );
    assert_eq!((patch.count, patch.min_kernel.as_str()), (1, "25.0.0"));
    assert!(r.noted("macOS 26"));
}

#[test]
fn comet_lake_with_rx5700xt_on_imac20_1() {
    let p = desktop(
        CometLake,
        vec![
            intel("9bc5", "Intel UHD Graphics 630"),
            amd("731f", "Radeon RX 5700 XT"),
        ],
    );
    let r = run(&p, Sonoma, "iMac20,1");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300C89B")
    );
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=pikera"));
    assert!(r.plan.kernel_patches.is_empty());
}

#[test]
fn alder_lake_with_rx6900xt_hides_the_xe_igpu() {
    let p = desktop(
        AlderLake,
        vec![
            intel("4680", "Intel UHD Graphics 770"),
            amd("73bf", "Radeon RX 6900 XT"),
        ],
    );
    assert_eq!(p.gpus[0].family, GpuFamily::IntelXe);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert!(r.has_arg("-wegnoigpu"));
    assert!(r.arg("agdpmod").is_none());
    assert_eq!(r.prop(IGPU_PATH, "class-code").as_deref(), Some("FFFFFFFF"));
    assert_eq!(r.kext, GpuKext::WhateverGreen);
}

#[test]
fn alder_lake_with_rtx3080_has_no_display_path() {
    let p = desktop(
        AlderLake,
        vec![
            intel("4680", "Intel UHD Graphics 770"),
            nvidia("2206", "NVIDIA GeForce RTX 3080"),
        ],
    );
    assert_eq!(p.gpus[1].family, GpuFamily::NvidiaModern);
    let err = fail(&p, Sonoma);
    assert_eq!(err.code, "NO_DISPLAY_PATH");
    assert!(err.recoverable);
    assert!(err.message.contains("RTX 3080"), "{}", err.message);
    assert!(err.message.contains("no driver"), "{}", err.message);
    assert!(err.suggestion.unwrap_or_default().contains("RX 580"));
}

#[test]
fn haswell_hd4600_is_native_on_monterey_and_root_patched_on_ventura() {
    let p = desktop(Haswell, vec![intel("0412", "Intel HD Graphics 4600")]);
    let r = run(&p, Monterey, "iMac16,2");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "framebuffer-patch-enable",
            "framebuffer-stolenmem",
            "framebuffer-fbmem"
        ]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300220D")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-fbmem").as_deref(),
        Some("00009000")
    );
    assert!(!r.root_patch);
    assert!(r.plan.post_install.is_empty());

    let r = run(&p, Ventura, "iMac18,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert!(r.root_patch);
    assert!(r.noted("OCLP root patches"));
    assert!(r
        .plan
        .post_install
        .iter()
        .any(|n| n.title.contains("Legacy Patcher")));
}

#[test]
fn haswell_desktop_hd4400_takes_the_hd4600_id() {
    let p = desktop(Haswell, vec![intel("041e", "Intel HD Graphics 4400")]);
    let r = run(&p, BigSur, "iMac15,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300220D")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("12040000"));
}

#[test]
fn skylake_desktop_runs_as_kaby_lake_from_ventura() {
    let p = desktop(Skylake, vec![intel("1912", "Intel HD Graphics 530")]);
    let r = run(&p, Ventura, "iMac18,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001259")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("12590000"));
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "device-id",
            "framebuffer-patch-enable",
            "framebuffer-stolenmem"
        ]
    );
    assert!(!r.root_patch);
    assert!(r.noted("run as Kaby Lake"));

    let r = run(&p, Monterey, "iMac17,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001219")
    );
    assert!(r.prop(IGPU_PATH, "device-id").is_none());
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-fbmem").as_deref(),
        Some("00009000")
    );
}

#[test]
fn kaby_lake_desktop_hd630() {
    let p = desktop(KabyLake, vec![intel("5912", "Intel HD Graphics 630")]);
    let r = run(&p, Sonoma, "iMac19,1");
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "framebuffer-patch-enable",
            "framebuffer-stolenmem"
        ]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001259")
    );
}

#[test]
fn sandy_bridge_hd3000_headless_next_to_gtx680() {
    let mut p = desktop(
        SandyBridge,
        vec![
            intel("0112", "Intel HD Graphics 3000"),
            nvidia("1180", "NVIDIA GeForce GTX 680"),
        ],
    );
    p.chipset = Some("Z77".into());
    let r = run(&p, HighSierra, "iMac12,2");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(r.keys(IGPU_PATH), ["AAPL,snb-platform-id", "device-id"]);
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,snb-platform-id").as_deref(),
        Some("00000500")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("02010000"));
    // Sandy Bridge CPU on a 7-series board.
    assert_eq!(
        r.prop("PciRoot(0x0)/Pci(0x16,0x0)", "device-id").as_deref(),
        Some("3A1C0000")
    );
    assert_eq!(r.kexts(), ["WhateverGreen"]);
    assert!(!r.root_patch);

    // No Sandy Bridge graphics driver after High Sierra: the iGPU is hidden.
    let r = run(&p, Mojave, "MacPro6,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert!(r.has_arg("-wegnoigpu"));
    assert_eq!(r.prop(IGPU_PATH, "class-code").as_deref(), Some("FFFFFFFF"));
}

#[test]
fn sandy_bridge_hd3000_desktop_display() {
    let mut p = desktop(SandyBridge, vec![intel("0112", "Intel HD Graphics 3000")]);
    p.chipset = Some("H61".into());
    let r = run(&p, HighSierra, "iMac12,2");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,snb-platform-id").as_deref(),
        Some("10000300")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("26010000"));
    assert!(r.prop(IGPU_PATH, "AAPL,ig-platform-id").is_none());
    assert!(r.entry("PciRoot(0x0)/Pci(0x16,0x0)").is_none());
}

#[test]
fn ivy_bridge_desktop_display_headless_and_dropped() {
    let p = desktop(IvyBridge, vec![intel("0162", "Intel HD Graphics 4000")]);
    let r = run(&p, Catalina, "iMac13,1");
    assert_eq!(r.keys(IGPU_PATH), ["AAPL,ig-platform-id"]);
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0A006601")
    );

    let p = desktop(
        IvyBridge,
        vec![
            intel("0162", "Intel HD Graphics 4000"),
            amd("67df", "Radeon RX 580"),
        ],
    );
    let r = run(&p, BigSur, "iMac15,1");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("07006201")
    );

    // HD 4000 has no driver in Monterey: the RX 580 does everything.
    let r = run(&p, Monterey, "MacPro6,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert!(r.has_arg("-wegnoigpu"));
}

#[test]
fn macpro_smbios_hides_a_headless_igpu() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e98", "Intel UHD Graphics 630"),
            amd("67df", "Radeon RX 580"),
        ],
    );
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert!(r.has_arg("-wegnoigpu"));
    assert_eq!(r.prop(IGPU_PATH, "class-code").as_deref(), Some("FFFFFFFF"));
}

#[test]
fn navi24_is_disabled_next_to_a_supported_igpu() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e92", "Intel UHD Graphics 630"),
            amd("743f", "Radeon RX 6500 XT"),
        ],
    );
    let r = run(&p, Sonoma, "iMac19,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
    assert!(!r.has_arg("-wegnoegpu"));
    assert!(r.noted("RX 6500 XT disabled"));
}

#[test]
fn navi24_without_an_igpu_is_an_error() {
    let p = desktop(AlderLake, vec![amd("743f", "Radeon RX 6500 XT")]);
    let err = fail(&p, Sonoma);
    assert_eq!(err.code, "NO_DISPLAY_PATH");
    assert!(err.message.contains("RX 6500 XT"));
}

#[test]
fn kepler_gt710_native_on_big_sur_root_patched_on_monterey() {
    let p = desktop(AmdZen2, vec![nvidia("128b", "NVIDIA GeForce GT 710")]);
    let r = run(&p, BigSur, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert_eq!(r.kexts(), ["WhateverGreen"]);
    assert!(!r.root_patch);

    let r = run(&p, Monterey, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert!(r.root_patch);
    assert!(r.noted("OCLP root patches"));
}

#[test]
fn native_igpu_wins_over_a_root_patched_kepler() {
    let p = desktop(
        Haswell,
        vec![
            intel("0412", "Intel HD Graphics 4600"),
            nvidia("128b", "NVIDIA GeForce GT 710"),
        ],
    );
    let r = run(&p, Monterey, "iMac16,2");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
    assert!(!r.root_patch);
}

#[test]
fn pascal_on_high_sierra_uses_the_web_driver() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e92", "Intel UHD Graphics 630"),
            nvidia("1b81", "NVIDIA GeForce GTX 1070"),
        ],
    );
    let options = options(HighSierra);
    let ctx = PlanContext::new(&p, &options);
    let r = run(&p, HighSierra, "iMac18,3");
    // Desktop UHD 630 is native only from Mojave.
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert!(uses_nvidia_web_driver(&ctx, &r.display));
    assert!(r.has_arg("nvda_drv_vrl=1"));
    assert!(r.has_arg("-wegnoigpu"));
    let var = r
        .plan
        .nvram_add
        .iter()
        .find(|v| v.key == "nvda_drv")
        .expect("nvda_drv");
    assert_eq!(var.guid, "7C436110-AB2A-4BBB-A880-FE41995C9F82");
    assert_eq!(var.value, PlistScalar::Data("31".into()));
    assert!(r
        .plan
        .post_install
        .iter()
        .any(|n| n.title.contains("Web Driver")));
}

#[test]
fn pascal_on_ventura_is_disabled_and_the_igpu_drives() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e92", "Intel UHD Graphics 630"),
            nvidia("1b81", "NVIDIA GeForce GTX 1070"),
        ],
    );
    let r = run(&p, Ventura, "iMac19,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
    assert!(r.arg("nvda_drv_vrl").is_none());
    assert!(r.plan.nvram_add.is_empty());
}

#[test]
fn unsupported_dgpu_without_path_uses_wegnoegpu_when_no_dgpu_is_left() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e92", "Intel UHD Graphics 630"),
            at(nvidia("2503", "NVIDIA GeForce RTX 3060"), None),
        ],
    );
    let r = run(&p, Sonoma, "iMac19,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert!(r.has_arg("-wegnoegpu"));
    assert!(r.entry(PEG).is_none());
}

#[test]
fn unsupported_dgpu_next_to_a_supported_one_gets_disable_gpu() {
    let p = desktop(
        AmdZen2,
        vec![
            amd("67df", "Radeon RX 580"),
            at(nvidia("2503", "NVIDIA GeForce RTX 3060"), Some(PEG2)),
        ],
    );
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[1]));
    assert_eq!(r.prop(PEG2, "disable-gpu").as_deref(), Some("01000000"));
    assert!(!r.has_arg("-wegnoegpu"));
    assert!(r.entry(PEG).is_none());
}

#[test]
fn unsupported_dgpu_without_path_next_to_a_supported_one_is_reported() {
    let p = desktop(
        AmdZen2,
        vec![
            amd("67df", "Radeon RX 580"),
            at(nvidia("2503", "NVIDIA GeForce RTX 3060"), None),
        ],
    );
    let r = run(&p, Sonoma, "MacPro7,1");
    assert!(!r.has_arg("-wegnoegpu"));
    assert!(r
        .plan
        .notes
        .iter()
        .any(|n| n.level == NoteLevel::Warning && n.title.contains("Cannot disable")));
}

#[test]
fn two_supported_amd_cards_both_stay_enabled() {
    let p = desktop(
        AmdZen3,
        vec![
            amd("67df", "Radeon RX 580"),
            at(amd("73bf", "Radeon RX 6800 XT"), Some(PEG2)),
        ],
    );
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
}

#[test]
fn navi22_next_to_a_whatevergreen_card_is_disabled() {
    let p = desktop(
        AmdZen3,
        vec![
            at(amd("73df", "Radeon RX 6700 XT"), Some(PEG2)),
            amd("67df", "Radeon RX 580"),
        ],
    );
    let r = run(&p, Sonoma, "MacPro7,1");
    // The RX 580 (WhateverGreen) is preferred; NootRX cannot load next to it.
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert_eq!(r.prop(PEG2, "disable-gpu").as_deref(), Some("01000000"));
    assert_eq!(r.kext, GpuKext::WhateverGreen);
    assert!(r.noted("exclude each other"));
}

#[test]
fn user_disabled_dgpu_is_disabled() {
    let mut rx = amd("67df", "Radeon RX 580");
    rx.disabled = true;
    let p = desktop(
        CoffeeLake,
        vec![intel("3e92", "Intel UHD Graphics 630"), rx],
    );
    let r = run(&p, Sonoma, "iMac19,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
    assert!(r.noted("Disabled in the hardware editor"));
}

#[test]
fn unsupported_dgpu_stays_enabled_when_disabling_is_off() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e92", "Intel UHD Graphics 630"),
            nvidia("2503", "NVIDIA GeForce RTX 3060"),
        ],
    );
    let options = BuildOptions {
        disable_unsupported_gpus: false,
        ..options(Sonoma)
    };
    let r = run_with(&p, &options, "iMac19,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert!(r.entry(PEG).is_none());
    assert!(!r.has_arg("-wegnoegpu"));
    assert!(r
        .plan
        .notes
        .iter()
        .any(|n| n.level == NoteLevel::Warning && n.title.contains("left enabled")));
}

#[test]
fn no_gpu_at_all_is_an_error() {
    let err = fail(&desktop(CoffeeLake, vec![]), Sonoma);
    assert_eq!(err.code, "NO_GPU");
    assert!(err.recoverable);
}

#[test]
fn non_avx2_cpu_needs_oclp_for_polaris_on_ventura() {
    let mut p = desktop(
        Haswell,
        vec![
            intel("0402", "Intel HD Graphics"),
            amd("67df", "Radeon RX 580"),
        ],
    );
    p.cpu.has_avx2 = Some(false);
    let r = run(&p, Ventura, "iMac18,2");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert!(r.root_patch);

    // Navi has no public root patch for CPUs without AVX2.
    let mut p = desktop(Haswell, vec![amd("73ff", "Radeon RX 6600 XT")]);
    p.cpu.has_avx2 = Some(false);
    let err = fail(&p, Ventura);
    assert!(err.message.contains("AVX2"), "{}", err.message);
}

#[test]
fn cape_verde_needs_radpg_and_root_patches_after_monterey() {
    let p = desktop(AmdZen, vec![amd("683f", "Radeon HD 7750")]);
    let r = run(&p, Monterey, "MacPro6,1");
    assert!(r.has_arg("radpg=15"));
    assert!(!r.root_patch);
    let r = run(&p, Ventura, "MacPro7,1");
    assert!(r.root_patch);
}

#[test]
fn headless_framebuffers_by_generation() {
    let with_rx580 =
        |platform, id: &str| desktop(platform, vec![intel(id, ""), amd("67df", "Radeon RX 580")]);
    // (platform, iGPU id, target, platform id, device-id)
    let cases = [
        (Haswell, "041e", BigSur, "04001204", Some("12040000")),
        (Broadwell, "1622", Monterey, "07002216", None),
        (Skylake, "1912", Monterey, "01001219", None),
        (Skylake, "1912", Ventura, "03001259", Some("12590000")),
        (KabyLake, "5912", Sonoma, "03001259", None),
        (KabyLake, "5902", Sonoma, "03001259", Some("12590000")),
        (CoffeeLake, "3e98", Sonoma, "0300913E", None),
        (CometLake, "9bc8", Sonoma, "0300C89B", None),
    ];
    for (platform, id, target, platform_id, device_id) in cases {
        let r = run(&with_rx580(platform, id), target, "iMac19,1");
        let what = format!("{platform:?} {id} {target:?}");
        assert_eq!(r.display, display(Some(1), Some(0), true, &[]), "{what}");
        assert_eq!(
            r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
            Some(platform_id),
            "{what}"
        );
        assert_eq!(
            r.prop(IGPU_PATH, "device-id").as_deref(),
            device_id,
            "{what}"
        );
        assert!(r.arg("agdpmod").is_none(), "{what}");
        assert!(!r.has_arg("-wegnoigpu"), "{what}");
    }
}

#[test]
fn kepler_on_big_sur_keeps_a_kaby_lake_igpu_headless() {
    let p = desktop(
        KabyLake,
        vec![
            intel("5912", "Intel HD Graphics 630"),
            nvidia("128b", "NVIDIA GeForce GT 710"),
        ],
    );
    let r = run(&p, BigSur, "iMac18,3");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("03001259")
    );
    assert_eq!(r.kexts(), ["WhateverGreen"]);
    assert!(r.plan.boot_args.is_empty());
}

#[test]
fn polaris_on_tahoe_gets_agdpmod_ignore_without_the_navi_patch() {
    let p = desktop(
        CometLake,
        vec![
            intel("9bc5", "Intel UHD Graphics 630"),
            amd("67df", "Radeon RX 580"),
        ],
    );
    let r = run(&p, Tahoe, "iMac20,1");
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=ignore"));
    assert!(r.plan.kernel_patches.is_empty());
}

#[test]
fn web_driver_card_next_to_an_amd_display_gpu() {
    let p = desktop(
        CoffeeLake,
        vec![
            amd("67df", "Radeon RX 580"),
            at(nvidia("1b81", "NVIDIA GeForce GTX 1070"), Some(PEG2)),
        ],
    );
    let high_sierra = options(HighSierra);
    let ctx = PlanContext::new(&p, &high_sierra);
    let r = run(&p, HighSierra, "iMacPro1,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert!(uses_nvidia_web_driver(&ctx, &r.display));
    assert!(r.has_arg("nvda_drv_vrl=1"));
    assert!(r.plan.nvram_add.iter().any(|v| v.key == "nvda_drv"));

    let mojave = options(Mojave);
    let ctx = PlanContext::new(&p, &mojave);
    let r = run(&p, Mojave, "iMacPro1,1");
    assert_eq!(r.display, display(Some(0), None, false, &[1]));
    assert!(!uses_nvidia_web_driver(&ctx, &r.display));
}

#[test]
fn r9_390_is_spoofed_to_the_r9_290x() {
    let p = desktop(AmdZen, vec![amd("67b1", "Radeon R9 390")]);
    let r = run(&p, Monterey, "MacPro6,1");
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("B0670000"));
    assert!(r.has_arg("-radcodec"));
    assert!(!r.root_patch);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert!(r.root_patch);
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("B0670000"));
}

#[test]
fn nootedred_disables_an_rdna_card_with_its_own_reason() {
    let p = desktop(AmdZen3, vec![apu("1638"), amd("743f", "Radeon RX 6500 XT")]);
    let r = run(&p, Sonoma, "iMac20,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.kexts(), ["NootedRed", "SMCRadeonSensors"]);
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
    assert!(r.noted("refuses to run while a GCN 5 or RDNA"));
}

// ── AMD spoofs, NootRX and NootedRed ───────────────────────────────────────

#[test]
fn lexa_rx550_is_spoofed_to_baffin() {
    let p = desktop(AmdZen2, vec![amd("699f", "Radeon RX 550")]);
    let r = run(&p, Monterey, "MacPro7,1");
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("FF670000"));
    assert_eq!(r.prop(PEG, "model").as_deref(), Some("Radeon RX 550"));
    assert!(r.has_arg("-radcodec"));
}

#[test]
fn spoof_without_pci_path_warns() {
    let p = desktop(AmdZen2, vec![at(amd("699f", "Radeon RX 550"), None)]);
    let r = run(&p, Monterey, "MacPro7,1");
    assert!(r.plan.device_properties.is_empty());
    assert!(r
        .plan
        .notes
        .iter()
        .any(|n| n.level == NoteLevel::Warning && n.title.contains("spoof")));
}

#[test]
fn ryzen_with_rx6700xt_uses_nootrx_only() {
    let p = desktop(AmdZen3, vec![amd("73df", "Radeon RX 6700 XT")]);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert_eq!(r.kexts(), ["NootRX", "SMCRadeonSensors"]);
    assert!(r.plan.device_properties.is_empty());
    assert!(r.plan.boot_args.is_empty());

    // Navi 22 needs macOS 12.
    let err = fail(&p, BigSur);
    assert!(err.message.contains("Monterey"), "{}", err.message);
    assert!(err.suggestion.unwrap_or_default().contains("or newer"));
}

#[test]
fn rx6950xt_takes_the_whatevergreen_spoof() {
    // Dortania GPU Buyers Guide: spoof the RX 6950 XT to the RX 6900 XT.
    let p = desktop(AmdZen3, vec![amd("73a5", "Radeon RX 6950 XT")]);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
    assert_eq!(r.kext, GpuKext::WhateverGreen);
    assert_eq!(r.keys(PEG), ["device-id", "model"]);
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("BF730000"));
    assert_eq!(r.prop(PEG, "model").as_deref(), Some("Radeon RX 6950 XT"));
    assert!(r.has_arg("-radcodec"));
    assert!(r.arg("agdpmod").is_none());

    let p = desktop(
        CometLake,
        vec![
            intel("9bc5", "Intel UHD Graphics 630"),
            amd("73a5", "Radeon RX 6950 XT"),
        ],
    );
    let r = run(&p, Sonoma, "iMac20,1");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("BF730000"));
    assert!(r.has_arg("-radcodec"));
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=pikera"));
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300C89B")
    );
}

#[test]
fn rx6950xt_without_a_device_path_falls_back_to_nootrx() {
    let p = desktop(AmdZen3, vec![at(amd("73a5", "Radeon RX 6950 XT"), None)]);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.kexts(), ["NootRX", "SMCRadeonSensors"]);
    assert!(r.plan.device_properties.is_empty());
    assert!(r.plan.boot_args.is_empty());
}

#[test]
fn rx6650xt_on_tahoe_with_an_imac_board() {
    let p = desktop(
        CometLake,
        vec![
            intel("9bc8", "Intel UHD Graphics 630"),
            amd("73ef", "Radeon RX 6650 XT"),
        ],
    );
    let r = run(&p, Tahoe, "iMac20,1");
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("FF730000"));
    assert_eq!(r.arg("agdpmod"), Some("agdpmod=ignore"));
    assert_eq!(r.plan.kernel_patches.len(), 1);
    assert_eq!(r.kext, GpuKext::WhateverGreen);
}

#[test]
fn alder_lake_with_rx6950xt_hides_the_igpu_through_whatevergreen() {
    let p = desktop(
        AlderLake,
        vec![
            intel("4680", "Intel UHD Graphics 770"),
            amd("73a5", "Radeon RX 6950 XT"),
        ],
    );
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
    assert!(r.has_arg("-wegnoigpu"));
    assert_eq!(r.prop(PEG, "device-id").as_deref(), Some("BF730000"));
}

#[test]
fn rx6950xt_and_rx6700xt_both_run_on_nootrx() {
    let p = desktop(
        AmdZen3,
        vec![
            amd("73a5", "Radeon RX 6950 XT"),
            at(amd("73df", "Radeon RX 6700 XT"), Some(PEG2)),
        ],
    );
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert_eq!(r.kexts(), ["NootRX", "SMCRadeonSensors"]);
    // NootRX drives both by their real ids: no spoof, no WhateverGreen args.
    assert!(r.plan.device_properties.is_empty());
    assert!(r.plan.boot_args.is_empty());

    // A third card that needs WhateverGreen keeps the spoof path; the Navi 22
    // card goes.
    let p = desktop(
        AmdZen3,
        vec![
            amd("73a5", "Radeon RX 6950 XT"),
            at(amd("73df", "Radeon RX 6700 XT"), Some(PEG2)),
            at(amd("6798", "Radeon R9 280X"), Some(APU)),
        ],
    );
    let r = run(&p, Monterey, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[1]));
    assert_eq!(r.kext, GpuKext::WhateverGreen);
    assert_eq!(r.prop(PEG2, "disable-gpu").as_deref(), Some("01000000"));
    assert!(r.noted("WhateverGreen and NootRX exclude each other"));
}

#[test]
fn navi22_keeps_a_coffee_lake_igpu_headless_without_whatevergreen() {
    let p = desktop(
        CoffeeLake,
        vec![
            intel("3e92", "Intel UHD Graphics 630"),
            amd("73df", "Radeon RX 6700 XT"),
        ],
    );
    let r = run(&p, Sonoma, "iMac19,1");
    assert_eq!(r.display, display(Some(1), Some(0), true, &[]));
    assert_eq!(r.kexts(), ["NootRX", "SMCRadeonSensors"]);
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300913E")
    );
    assert!(!r.has_arg("-wegnoigpu"));
}

#[test]
fn navi22_cannot_keep_a_skylake_igpu_on_ventura() {
    let p = desktop(
        Skylake,
        vec![
            intel("1912", "Intel HD Graphics 530"),
            amd("73df", "Radeon RX 6700 XT"),
        ],
    );
    let r = run(&p, Ventura, "iMacPro1,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert_eq!(r.kexts(), ["NootRX", "SMCRadeonSensors"]);
    assert!(!r.has_arg("-wegnoigpu"));
    assert!(r.noted("in the firmware"));
}

#[test]
fn ryzen_5600g_alone_uses_nootedred() {
    let p = desktop(AmdZen3, vec![apu("1638")]);
    assert_eq!(p.gpus[0].family, GpuFamily::AmdApuVega);
    let r = run(&p, Sequoia, "iMac20,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert_eq!(r.kexts(), ["NootedRed", "SMCRadeonSensors"]);
    assert!(r.plan.device_properties.is_empty());
    assert!(r.noted("UMA"));
}

#[test]
fn ryzen_apu_with_rx580_prefers_the_dgpu() {
    let p = desktop(AmdZen3, vec![apu("1638"), amd("67df", "Radeon RX 580")]);
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
    assert_eq!(r.prop(APU, "disable-gpu").as_deref(), Some("01000000"));
    assert!(!r.has_arg("-wegnoegpu"));
}

#[test]
fn ryzen_laptop_vega_disables_the_nvidia_dgpu() {
    let p = laptop(
        AmdZen2,
        vec![apu("1636"), nvidia("1f91", "NVIDIA GeForce GTX 1650")],
    );
    let r = run(&p, Sonoma, "MacBookPro16,2");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.kexts(), ["NootedRed", "SMCRadeonSensors"]);
    assert!(r.has_arg("-wegnoegpu"));
    assert!(r
        .plan
        .post_install
        .iter()
        .any(|n| n.title.contains("discrete GPU")));
}

#[test]
fn nootedred_warns_about_macpro_smbios() {
    let p = desktop(AmdZen2, vec![apu("1636")]);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert!(r
        .plan
        .notes
        .iter()
        .any(|n| n.level == NoteLevel::Warning && n.title.contains("NootedRed")));
}

// ── Intel laptops ───────────────────────────────────────────────────────────

#[test]
fn kaby_lake_r_laptop_with_mx150() {
    let p = laptop(
        KabyLake,
        vec![
            intel("5917", "Intel UHD Graphics 620"),
            nvidia("1d10", "NVIDIA GeForce MX150"),
        ],
    );
    assert_eq!(p.gpus[1].family, GpuFamily::NvidiaPascal);
    let r = run(&p, Ventura, "MacBookPro14,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0000C087")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("16590000"));
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-fbmem").as_deref(),
        Some("00009000")
    );
    assert!(r.has_arg("-wegnoegpu"));
    assert!(r.entry(PEG).is_none());
    assert!(r.noted("Optimus"));
}

#[test]
fn coffee_lake_laptop_uhd630_backlight_by_release() {
    let p = laptop(CoffeeLake, vec![intel("3e9b", "Intel UHD Graphics 630")]);
    let r = run(&p, Sonoma, "MacBookPro15,1");
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "framebuffer-patch-enable",
            "framebuffer-stolenmem",
            "framebuffer-fbmem",
            "enable-backlight-registers-alternative-fix"
        ]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0900A53E")
    );

    let r = run(&p, BigSur, "MacBookPro15,1");
    assert_eq!(
        r.prop(IGPU_PATH, "enable-backlight-registers-fix")
            .as_deref(),
        Some("01000000")
    );
    assert!(r
        .prop(IGPU_PATH, "enable-backlight-registers-alternative-fix")
        .is_none());
}

#[test]
fn comet_lake_u_uhd620_runs_as_0x3e9b() {
    let p = laptop(CometLake, vec![intel("9b41", "Intel UHD Graphics 620")]);
    let r = run(&p, BigSur, "MacBookPro16,2");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00009B3E")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("9B3E0000"));
    assert_eq!(
        r.prop(IGPU_PATH, "enable-backlight-registers-fix")
            .as_deref(),
        Some("01000000")
    );
}

#[test]
fn whiskey_lake_uhd620_runs_as_0x3e9b() {
    let p = laptop(CoffeeLake, vec![intel("3ea0", "Intel UHD Graphics 620")]);
    let r = run(&p, Sequoia, "MacBookPro15,2");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00009B3E")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("9B3E0000"));
}

#[test]
fn ice_lake_laptop() {
    let p = laptop(IceLake, vec![intel("8a52", "Intel Iris Plus Graphics G7")]);
    let r = run(&p, Sequoia, "MacBookAir9,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0000528A")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-stolenmem").as_deref(),
        Some("00003001")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-fbmem").as_deref(),
        Some("00009000")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "enable-backlight-registers-fix")
            .as_deref(),
        Some("01000000")
    );
    assert!(r.has_arg("-igfxcdc"));
    assert!(r.has_arg("-igfxdvmt"));
    assert!(r.noted("-noDC9"));
    assert_eq!(r.kexts(), ["WhateverGreen"]);
}

#[test]
fn haswell_laptop_framebuffers() {
    let p = laptop(Haswell, vec![intel("0a16", "Intel HD Graphics 4400")]);
    let r = run(&p, Monterey, "MacBookPro11,4");
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "device-id",
            "framebuffer-patch-enable",
            "framebuffer-cursormem"
        ]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0600260A")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("12040000"));
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-cursormem").as_deref(),
        Some("00009000")
    );

    let p = laptop(Haswell, vec![intel("0a26", "Intel HD Graphics 5000")]);
    let r = run(&p, Monterey, "MacBookPro11,4");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0500260A")
    );
    assert!(r.prop(IGPU_PATH, "device-id").is_none());
}

#[test]
fn sandy_bridge_laptop_uses_snb_platform_id() {
    let p = laptop(SandyBridge, vec![intel("0126", "Intel HD Graphics 3000")]);
    let r = run(&p, HighSierra, "MacBookPro8,1");
    assert_eq!(r.keys(IGPU_PATH), ["AAPL,snb-platform-id"]);
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,snb-platform-id").as_deref(),
        Some("00000100")
    );
}

#[test]
fn ivy_bridge_laptop_defaults_to_the_low_resolution_framebuffer() {
    let p = laptop(IvyBridge, vec![intel("0166", "Intel HD Graphics 4000")]);
    let r = run(&p, BigSur, "MacBookPro11,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("03006601")
    );
    assert!(r.noted("1600x900"));
}

#[test]
fn broadwell_laptop_hd5600() {
    let p = laptop(Broadwell, vec![intel("1612", "Intel HD Graphics 5600")]);
    let r = run(&p, Monterey, "MacBookPro11,4");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("06002616")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("26160000"));
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-fbmem").as_deref(),
        Some("00009000")
    );
}

#[test]
fn skylake_laptop_hd520_by_release() {
    let p = laptop(Skylake, vec![intel("1916", "Intel HD Graphics 520")]);
    let r = run(&p, Monterey, "MacBookPro13,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001619")
    );
    assert!(r.prop(IGPU_PATH, "device-id").is_none());

    let r = run(&p, Sonoma, "MacBookPro15,2");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001B59")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("16590000"));
}

#[test]
fn tiger_lake_laptop_has_no_display_path() {
    let p = laptop(
        TigerLake,
        vec![
            intel("9a49", "Intel Iris Xe Graphics"),
            nvidia("1f91", "NVIDIA GeForce GTX 1650"),
        ],
    );
    let err = fail(&p, Sonoma);
    assert_eq!(err.code, "NO_DISPLAY_PATH");
    assert!(err.message.contains("Iris Xe"));
    assert!(err.suggestion.unwrap_or_default().contains("laptop"));
}

#[test]
fn laptop_with_mux_amd_dgpu_drives_the_panel_from_it() {
    let mut p = laptop(
        TigerLake,
        vec![
            intel("9a49", "Intel Iris Xe Graphics"),
            amd("7340", "Radeon RX 5500M"),
        ],
    );
    // With the iGPU listed the laptop runs in hybrid mode: no display path.
    let err = fail(&p, Sonoma);
    assert_eq!(err.code, "NO_DISPLAY_PATH");
    assert!(err.suggestion.as_deref().is_some_and(|s| s.contains("MUX")), "{err:?}");
    // MUX switched to discrete-only mode: the iGPU is off.
    p.gpus[0].disabled = true;
    let r = run(&p, Sonoma, "MacBookPro16,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert!(r.has_arg("applbkl=3"));
    assert!(r.has_arg("-wegnoigpu"));
    assert!(r.arg("agdpmod").is_none());
    assert!(r
        .plan
        .notes
        .iter()
        .any(|n| n.level == NoteLevel::Warning && n.title.contains("laptop panel")));
}

#[test]
fn laptop_disables_a_supported_amd_dgpu_too() {
    let p = laptop(
        CoffeeLake,
        vec![
            intel("3e9b", "Intel UHD Graphics 630"),
            amd("67ef", "Radeon Pro 560X"),
        ],
    );
    let r = run(&p, Sonoma, "MacBookPro15,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert!(r.has_arg("-wegnoegpu"));
    assert_eq!(r.kexts(), ["WhateverGreen"]);
}

#[test]
fn laptop_dgpu_left_enabled_on_request_is_flagged() {
    let p = laptop(
        CoffeeLake,
        vec![
            intel("3e9b", "Intel UHD Graphics 630"),
            amd("67ef", "Radeon Pro 560X"),
        ],
    );
    let options = BuildOptions {
        disable_unsupported_gpus: false,
        ..options(Sonoma)
    };
    let r = run_with(&p, &options, "MacBookPro15,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[]));
    assert!(!r.has_arg("-wegnoegpu"));
    assert!(r.plan.notes.iter().any(|n| n.level == NoteLevel::Warning
        && n.title.contains("left enabled")
        && n.detail.contains("internal panel")));
}

#[test]
fn amber_lake_uhd617_keeps_its_id() {
    let p = laptop(KabyLake, vec![intel("87c0", "Intel UHD Graphics 617")]);
    let r = run(&p, Sonoma, "MacBookAir8,1");
    assert_eq!(
        r.keys(IGPU_PATH),
        [
            "AAPL,ig-platform-id",
            "framebuffer-patch-enable",
            "framebuffer-stolenmem",
            "framebuffer-fbmem"
        ]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0000C087")
    );
    assert!(r.noted("enable-backlight-registers-alternative-fix"));
}

#[test]
fn kaby_lake_laptop_hd620_uses_0x591b0000() {
    let p = laptop(KabyLake, vec![intel("5916", "Intel HD Graphics 620")]);
    let r = run(&p, Monterey, "MacBookPro14,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001B59")
    );
    assert!(r.prop(IGPU_PATH, "device-id").is_none());
    // No backlight register fix by default before Coffee Lake.
    assert!(r
        .keys(IGPU_PATH)
        .iter()
        .all(|k| !k.starts_with("enable-backlight")));
}

#[test]
fn arrandale_laptop_uses_single_link_lvds() {
    let p = laptop(Arrandale, vec![intel("0046", "Intel HD Graphics")]);
    let r = run(&p, HighSierra, "MacBookPro6,2");
    assert_eq!(
        r.keys(IGPU_PATH),
        ["framebuffer-patch-enable", "framebuffer-singlelink"]
    );
    assert_eq!(
        r.prop(IGPU_PATH, "framebuffer-singlelink").as_deref(),
        Some("01000000")
    );
}

#[test]
fn skylake_hd510_laptop_takes_the_dortania_recipe() {
    let p = laptop(Skylake, vec![intel("1906", "Intel HD Graphics 510")]);
    let r = run(&p, Monterey, "MacBookPro13,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("00001B19")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("02190000"));
}

#[test]
fn ivy_bridge_on_a_6_series_board_gets_the_imei_id() {
    let mut p = laptop(IvyBridge, vec![intel("0166", "Intel HD Graphics 4000")]);
    p.chipset = Some("HM65".into());
    let r = run(&p, Catalina, "MacBookPro10,2");
    assert_eq!(
        r.prop("PciRoot(0x0)/Pci(0x16,0x0)", "device-id").as_deref(),
        Some("3A1E0000")
    );
    p.chipset = Some("HM76".into());
    let r = run(&p, Catalina, "MacBookPro10,2");
    assert!(r.entry("PciRoot(0x0)/Pci(0x16,0x0)").is_none());
}

// ── Mini PCs, all-in-ones and VMs ──────────────────────────────────────────

#[test]
fn nuc_with_iris_plus_655() {
    let p = machine(
        CoffeeLake,
        FormFactor::MiniPc,
        vec![intel("3ea5", "Intel Iris Plus Graphics 655")],
    );
    let r = run(&p, Sequoia, "Macmini8,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0000A53E")
    );
    assert!(r
        .prop(IGPU_PATH, "enable-backlight-registers-fix")
        .is_none());
    assert!(r
        .prop(IGPU_PATH, "enable-backlight-registers-alternative-fix")
        .is_none());
}

#[test]
fn nuc_framebuffers_by_generation() {
    let nuc = |platform, id: &str| machine(platform, FormFactor::MiniPc, vec![intel(id, "")]);
    let r = run(&nuc(Haswell, "0a16"), Monterey, "Macmini7,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0300220D")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("12040000"));

    let r = run(&nuc(Broadwell, "1616"), Monterey, "iMac16,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("02001616")
    );

    let r = run(&nuc(Skylake, "1926"), Monterey, "iMac17,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("02002619")
    );
    let r = run(&nuc(Skylake, "1926"), Ventura, "iMac18,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("02002659")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("26590000"));

    let r = run(&nuc(CoffeeLake, "3e9b"), Sequoia, "Macmini8,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("07009B3E")
    );
    let r = run(&nuc(CometLake, "9b41"), Sequoia, "Macmini8,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("07009B3E")
    );
    assert_eq!(r.prop(IGPU_PATH, "device-id").as_deref(), Some("9B3E0000"));
}

#[test]
fn all_in_one_uses_the_panel_framebuffer() {
    let mut p = machine(
        CoffeeLake,
        FormFactor::AllInOne,
        vec![intel("3e92", "Intel UHD Graphics 630")],
    );
    p.cpu.is_mobile = false;
    let r = run(&p, Sonoma, "iMac19,1");
    assert_eq!(
        r.prop(IGPU_PATH, "AAPL,ig-platform-id").as_deref(),
        Some("0900A53E")
    );
    assert_eq!(
        r.prop(IGPU_PATH, "enable-backlight-registers-alternative-fix")
            .as_deref(),
        Some("01000000")
    );
}

#[test]
fn vm_with_a_virtual_display() {
    let p = vm(vec![gpu("1234", "1111", "QEMU Standard VGA", None)]);
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[]));
    assert!(r.plan.kexts.is_empty());
    assert!(r.plan.device_properties.is_empty());
    assert!(r.plan.boot_args.is_empty());
    assert_eq!(r.kext, GpuKext::None);
}

#[test]
fn vm_with_a_passed_through_rx580() {
    let p = vm(vec![
        gpu("1b36", "0100", "QXL paravirtual graphic card", None),
        amd("67df", "Radeon RX 580"),
    ]);
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(1), None, false, &[]));
    assert_eq!(r.kexts(), ["WhateverGreen", "SMCRadeonSensors"]);
}

#[test]
fn vm_without_any_display_adapter() {
    let r = run(&vm(vec![]), Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(None, None, false, &[]));
    assert!(r.plan.kexts.is_empty());
}

#[test]
fn vm_disables_an_unsupported_passthrough_gpu() {
    let p = vm(vec![
        gpu("1234", "1111", "QEMU Standard VGA", None),
        nvidia("2503", "NVIDIA GeForce RTX 3060"),
    ]);
    let r = run(&p, Sequoia, "MacPro7,1");
    assert_eq!(r.display, display(Some(0), None, false, &[1]));
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
    assert_eq!(r.kexts(), ["WhateverGreen"]);
}

// ── Invariants ──────────────────────────────────────────────────────────────

#[test]
fn gpu_kexts_exist_in_the_catalog() {
    for kext in [GpuKext::WhateverGreen, GpuKext::NootRx, GpuKext::NootedRed] {
        let (id, bundle) = kext.catalog().expect("catalog entry");
        let entry = kext_catalog::entry(id).expect("known catalog id");
        assert!(entry.provides(bundle), "{id} provides {bundle}");
    }
    assert!(kext_catalog::entry("SMCRadeonSensors")
        .is_some_and(|e| e.provides("SMCRadeonSensors.kext")));
    assert_eq!(GpuKext::None.catalog(), None);
}

#[test]
fn every_scenario_writes_at_most_one_gpu_kext_and_unique_boot_args() {
    let scenarios = [
        (
            desktop(
                CoffeeLake,
                vec![intel("3e92", "UHD 630"), amd("73a5", "RX 6950 XT")],
            ),
            Sonoma,
        ),
        (
            desktop(AmdZen3, vec![apu("1638"), amd("73df", "RX 6700 XT")]),
            Sonoma,
        ),
        (
            desktop(AmdZen3, vec![apu("1638"), nvidia("2503", "RTX 3060")]),
            Sonoma,
        ),
        (
            laptop(
                CometLake,
                vec![intel("9bc4", "UHD 630"), nvidia("1f91", "GTX 1650")],
            ),
            Tahoe,
        ),
        (
            desktop(
                Skylake,
                vec![intel("1912", "HD 530"), amd("731f", "RX 5700 XT")],
            ),
            Tahoe,
        ),
    ];
    for (profile, target) in &scenarios {
        let r = run(profile, *target, "iMac20,1");
        let gpu_kexts = r
            .kexts()
            .into_iter()
            .filter(|k| matches!(*k, "WhateverGreen" | "NootRX" | "NootedRed"))
            .count();
        assert!(gpu_kexts <= 1, "{:?}", r.kexts());
        let mut keys: Vec<&str> = r
            .plan
            .boot_args
            .iter()
            .map(|a| a.split('=').next().unwrap())
            .collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total, "{:?}", r.plan.boot_args);
        for entry in &r.plan.device_properties {
            assert!(entry.path.starts_with("PciRoot("), "{}", entry.path);
            assert!(!entry.properties.is_empty());
        }
    }
}

#[test]
fn apu_with_navi22_prefers_nootrx_and_disables_the_apu() {
    let p = desktop(AmdZen3, vec![apu("1638"), amd("73df", "Radeon RX 6700 XT")]);
    let r = run(&p, Sonoma, "MacPro7,1");
    assert_eq!(r.display, display(Some(1), None, false, &[0]));
    assert_eq!(r.kexts(), ["NootRX", "SMCRadeonSensors"]);
}

#[test]
fn apu_with_unsupported_dgpu_uses_nootedred() {
    let p = desktop(
        AmdZen3,
        vec![apu("1638"), nvidia("2503", "NVIDIA GeForce RTX 3060")],
    );
    let r = run(&p, Sonoma, "iMac20,1");
    assert_eq!(r.display, display(Some(0), Some(0), false, &[1]));
    assert_eq!(r.kexts(), ["NootedRed", "SMCRadeonSensors"]);
    assert_eq!(r.prop(PEG, "disable-gpu").as_deref(), Some("01000000"));
}

// ── Platform sweep ──────────────────────────────────────────────────────────

/// Typical integrated graphics of each platform (desktop, laptop).
fn typical_igpu(platform: CpuPlatform) -> (Option<ProfileGpu>, Option<ProfileGpu>) {
    let i = |id: &str| Some(intel(id, ""));
    let a = |id: &str| Some(apu(id));
    match platform {
        Lynnfield => (i("0042"), None),
        Arrandale => (None, i("0046")),
        SandyBridge => (i("0112"), i("0126")),
        IvyBridge => (i("0162"), i("0166")),
        Haswell => (i("0412"), i("0a16")),
        Broadwell => (i("1622"), i("1616")),
        Skylake => (i("1912"), i("1916")),
        KabyLake => (i("5912"), i("5917")),
        CoffeeLake => (i("3e92"), i("3e9b")),
        CometLake => (i("9bc5"), i("9b41")),
        IceLake => (None, i("8a52")),
        RocketLake => (i("4c8a"), None),
        TigerLake => (None, i("9a49")),
        AlderLake | RaptorLake => (i("4680"), i("46a6")),
        AmdZen => (a("15dd"), a("15d8")),
        AmdZen2 => (a("1636"), a("1636")),
        AmdZen3 => (a("1638"), a("1638")),
        AmdZen4 => (a("164e"), a("15bf")),
        AmdZen5 => (a("13c0"), a("150e")),
        _ => (None, None),
    }
}

#[test]
fn every_supported_platform_form_factor_and_release() {
    let dgpus: Vec<Option<ProfileGpu>> = vec![
        None,
        Some(amd("68b8", "Radeon HD 5770")),
        Some(amd("683f", "Radeon HD 7750")),
        Some(amd("67df", "Radeon RX 580")),
        Some(amd("699f", "Radeon RX 550")),
        Some(amd("687f", "Radeon RX Vega 64")),
        Some(amd("66af", "Radeon VII")),
        Some(amd("731f", "Radeon RX 5700 XT")),
        Some(amd("73ff", "Radeon RX 6600 XT")),
        Some(amd("73df", "Radeon RX 6700 XT")),
        Some(amd("73a5", "Radeon RX 6950 XT")),
        Some(at(amd("73a5", "Radeon RX 6950 XT"), None)),
        Some(amd("743f", "Radeon RX 6500 XT")),
        Some(amd("744c", "Radeon RX 7900 XTX")),
        Some(nvidia("0f02", "GeForce GT 730")),
        Some(nvidia("128b", "GeForce GT 710")),
        Some(nvidia("1380", "GeForce GTX 750 Ti")),
        Some(nvidia("1b81", "GeForce GTX 1070")),
        Some(nvidia("2503", "GeForce RTX 3060")),
        Some(at(nvidia("2503", "GeForce RTX 3060"), None)),
    ];
    let forms = [
        FormFactor::Desktop,
        FormFactor::Laptop,
        FormFactor::AllInOne,
        FormFactor::MiniPc,
    ];
    let mut checked = 0;
    for &platform in crate::domain::cpu_db::all_platforms() {
        let info = crate::domain::cpu_db::platform_info(platform);
        if !info.supported {
            continue;
        }
        let (desktop_igpu, laptop_igpu) = typical_igpu(platform);
        for form in forms {
            let igpu = if form == FormFactor::Laptop {
                laptop_igpu.clone()
            } else {
                desktop_igpu.clone()
            };
            for dgpu in &dgpus {
                let gpus: Vec<ProfileGpu> = igpu.iter().chain(dgpu.iter()).cloned().collect();
                let profile = machine(platform, form, gpus);
                for target in MacOsVersion::ALL {
                    if info.min_macos.is_some_and(|min| target < min)
                        || info.max_macos.is_some_and(|max| target > max)
                    {
                        continue;
                    }
                    for disable_unsupported_gpus in [true, false] {
                        let options = BuildOptions {
                            disable_unsupported_gpus,
                            ..options(target)
                        };
                        check_scenario(&profile, &options);
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 8000, "{checked}");
}

fn check_scenario(profile: &HardwareProfile, options: &BuildOptions) {
    let target = options.target;
    let ctx = PlanContext::new(profile, options);
    let what = format!(
        "{:?} {:?} {:?} disable={} {:?}",
        profile.cpu.platform,
        profile.form_factor,
        target,
        options.disable_unsupported_gpus,
        profile
            .gpus
            .iter()
            .map(|g| (g.device_id.clone(), g.pci_path.is_some()))
            .collect::<Vec<_>>()
    );
    // Laptop dGPUs only count without an enabled iGPU (MUX in discrete mode).
    let native = profile.gpus.iter().enumerate().any(|(i, g)| {
        gpu_db::support(g).display_capable
            && gpu_db::natively_supported_on(g, target)
            && crate::domain::compatibility::can_drive_display(profile, i)
    });
    let display = match choose_display(&ctx) {
        Ok(display) => display,
        Err(err) => {
            assert!(!native || ctx.needs_cryptexfixup, "{what}: {}", err.message);
            let code = if profile.gpus.is_empty() {
                "NO_GPU"
            } else {
                "NO_DISPLAY_PATH"
            };
            assert_eq!(err.code, code, "{what}");
            assert!(err.recoverable, "{what}");
            assert!(err.suggestion.is_some(), "{what}");
            return;
        }
    };
    let primary = display
        .primary
        .expect("bare metal always has a primary GPU");
    assert!(!display.disabled.contains(&primary), "{what}");
    if let Some(igpu) = display.igpu {
        assert!(!display.disabled.contains(&igpu), "{what}");
        assert!(profile.gpus[igpu].is_igpu, "{what}");
    }
    if ctx.is_laptop && profile.gpus[primary].is_igpu && options.disable_unsupported_gpus {
        // Optimus / switchable graphics: every dGPU is disabled.
        let dgpus: Vec<usize> = (0..profile.gpus.len())
            .filter(|&i| !profile.gpus[i].is_igpu)
            .collect();
        assert!(dgpus.iter().all(|i| display.disabled.contains(i)), "{what}");
    }

    let mut plan = empty_plan(target);
    plan.smbios.model = "iMac20,1".into();
    apply(&ctx, &display, &mut plan);
    let kext = gpu_kext(&ctx, &display);

    let gpu_kexts: Vec<&KextSelection> = plan
        .kexts
        .iter()
        .filter(|k| {
            matches!(
                k.catalog_id.as_str(),
                "WhateverGreen" | "NootRX" | "NootedRed"
            )
        })
        .collect();
    assert_eq!(gpu_kexts.len(), 1, "{what}: {:?}", plan.kexts);
    assert!(gpu_kexts[0].required, "{what}");
    assert_eq!(
        kext.catalog().map(|(id, _)| id),
        Some(gpu_kexts[0].catalog_id.as_str()),
        "{what}"
    );
    let mut keys: Vec<&str> = plan
        .boot_args
        .iter()
        .map(|a| a.split('=').next().unwrap_or(a))
        .collect();
    let total = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), total, "{what}: {:?}", plan.boot_args);
    // WhateverGreen-only switches never go into a build without it.
    if kext != GpuKext::WhateverGreen {
        for key in ["-wegnoigpu", "agdpmod", "-igfxcdc", "-igfxdvmt"] {
            assert!(!keys.contains(&key), "{what}: {:?}", plan.boot_args);
        }
    }
    // -wegnoegpu hides every dGPU: never while one drives the displays.
    if !profile.gpus[primary].is_igpu {
        assert!(
            !keys.contains(&"-wegnoegpu"),
            "{what}: {:?}",
            plan.boot_args
        );
    }
    for entry in &plan.device_properties {
        assert!(entry.path.starts_with("PciRoot("), "{what}");
        assert!(!entry.properties.is_empty(), "{what}");
        for prop in &entry.properties {
            if let PlistScalar::Data(hex) = &prop.value {
                assert!(
                    !hex.is_empty()
                        && hex.len() % 2 == 0
                        && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{what}: {hex}"
                );
            }
        }
    }
    // Every disabled dGPU is hidden one way or another, or the user is told.
    for &i in &display.disabled {
        let gpu = &profile.gpus[i];
        if gpu.is_igpu {
            continue;
        }
        let by_property = gpu.pci_path.as_deref().is_some_and(|path| {
            plan.device_properties
                .iter()
                .any(|e| e.path == path && e.properties.iter().any(|p| p.key == "disable-gpu"))
        });
        let by_arg = keys.contains(&"-wegnoegpu");
        let warned = plan
            .notes
            .iter()
            .any(|n| n.level == NoteLevel::Warning && n.title.starts_with("Cannot disable"));
        assert!(by_property || by_arg || warned, "{what}");
    }
    // An Intel iGPU that is kept gets its framebuffer unless it is hidden.
    if let Some(igpu) = display.igpu {
        let gpu = &profile.gpus[igpu];
        if gpu.vendor == GpuVendor::Intel && !keys.contains(&"-wegnoigpu") {
            assert!(
                plan.device_properties.iter().any(|e| e.path == IGPU_PATH),
                "{what}"
            );
        }
    }
    // GPU temperatures only for an AMD display GPU.
    let amd_display = profile.gpus[primary].vendor == GpuVendor::Amd;
    let sensors = plan
        .kexts
        .iter()
        .any(|k| k.catalog_id == "SMCRadeonSensors");
    assert!(!sensors || amd_display, "{what}");
    assert!(!plan.notes.is_empty(), "{what}");
    assert!(plan.notes.iter().all(|n| n.component == "gpu"), "{what}");
}
