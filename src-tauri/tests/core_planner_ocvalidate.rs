//! Runs OpenCore's own ocvalidate on configs written from the SMBIOS, quirks
//! and settings planner stages, for every supported CPU platform, form
//! factor, display layout, firmware type and release, chipset-specific Booter
//! sets and virtual machines. Needs the ocvalidate binary of OpenCore 1.0.8:
//! `OCVALIDATE=/path/to/ocvalidate cargo test --test core_planner_ocvalidate -- --ignored --nocapture`

use std::path::PathBuf;
use std::process::Command;

use app_lib::domain::config_writer::{write_config, ConfigInputs};
use app_lib::domain::cpu_db;
use app_lib::domain::model::{
    BuildOptions, CpuPlatform, FormFactor, GpuFamily, HardwareProfile, MacOsVersion, PickerStyle,
    PlatformIdentity, ProfileCpu, ProfileGpu, VmKind,
};
use app_lib::domain::planner::{
    empty_plan, quirks, settings, smbios, validate, DisplayPlan, PlanContext,
};

const SAMPLE: &[u8] = include_bytes!("fixtures/Sample-1.0.8.plist");

fn igpu_family(platform: CpuPlatform) -> Option<GpuFamily> {
    use CpuPlatform as P;
    use GpuFamily as G;
    Some(match platform {
        P::SandyBridge => G::IntelSandyBridge,
        P::IvyBridge => G::IntelIvyBridge,
        P::Haswell => G::IntelHaswell,
        P::Broadwell => G::IntelBroadwell,
        P::Skylake => G::IntelSkylake,
        P::KabyLake => G::IntelKabyLake,
        P::CoffeeLake => G::IntelCoffeeLake,
        P::CometLake => G::IntelCometLake,
        P::IceLake => G::IntelIceLake,
        P::AmdZen | P::AmdZen2 | P::AmdZen3 => G::AmdApuVega,
        _ => return None,
    })
}

fn gpu(family: GpuFamily, is_igpu: bool) -> ProfileGpu {
    ProfileGpu {
        name: format!("{family:?}"),
        family,
        is_igpu,
        ..Default::default()
    }
}

fn profile(
    platform: CpuPlatform,
    form: FormFactor,
    gpus: Vec<ProfileGpu>,
    vendor: &str,
) -> HardwareProfile {
    HardwareProfile {
        cpu: ProfileCpu {
            name: "Test CPU".into(),
            vendor: cpu_db::platform_info(platform).vendor,
            platform,
            codename: "Test".into(),
            cores: 6,
            threads: 12,
            is_mobile: form != FormFactor::Desktop,
            ..Default::default()
        },
        form_factor: form,
        gpus,
        motherboard_vendor: vendor.into(),
        ram_gb: 32,
        firmware_uefi: Some(true),
        ..Default::default()
    }
}

/// Writes the config for one profile and target and runs ocvalidate on it.
struct Checker {
    ocvalidate: PathBuf,
    dir: PathBuf,
    failures: Vec<String>,
    checked: usize,
}

impl Checker {
    fn check(&mut self, p: &HardwareProfile, display: &DisplayPlan, o: &BuildOptions, label: &str) {
        if validate(p, o).is_err() {
            return;
        }
        let mut ctx = PlanContext::new(p, o);
        ctx.display = Some(display.clone());
        let mut plan = empty_plan(o.target);
        smbios::apply(&ctx, display, &mut plan).unwrap();
        quirks::apply(&ctx, &mut plan);
        settings::apply(&ctx, &mut plan);
        let identity = PlatformIdentity {
            model: plan.smbios.model.clone(),
            serial: "C02XG0FDH7JY".into(),
            mlb: "C02839303QXH69FJA".into(),
            system_uuid: "dbb364d6-44b2-4a02-b922-ab4396f16da8".into(),
            rom: "112233445566".into(),
        };
        let drivers: Vec<String> = plan.drivers.iter().map(|d| d.path.clone()).collect();
        let config = write_config(
            SAMPLE,
            &ConfigInputs {
                plan: &plan,
                kernel_add: &[],
                identity: &identity,
                ssdt_files: &[],
                driver_files: &drivers,
                tool_files: &plan.tools,
            },
        )
        .unwrap();
        let name = format!("{label}-{}.plist", o.target.id()).replace([' ', '/'], "_");
        let path = self.dir.join(name);
        std::fs::write(&path, &config).unwrap();
        let out = Command::new(&self.ocvalidate).arg(&path).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if !stdout.contains("No issues found") {
            self.failures.push(format!("{}:\n{stdout}", path.display()));
        }
        self.checked += 1;
    }
}

fn display(primary: Option<usize>, igpu: Option<usize>) -> DisplayPlan {
    DisplayPlan {
        primary,
        igpu,
        igpu_headless: false,
        disabled: vec![],
    }
}

#[test]
#[ignore]
fn ocvalidate_accepts_core_stage_configs() {
    let Some(ocvalidate) = std::env::var_os("OCVALIDATE").map(PathBuf::from) else {
        eprintln!("OCVALIDATE is not set; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("core-planner-ocvalidate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut checker = Checker {
        ocvalidate,
        dir: dir.clone(),
        failures: Vec::new(),
        checked: 0,
    };
    for &platform in cpu_db::all_platforms() {
        if !cpu_db::platform_info(platform).supported {
            continue;
        }
        for (form, vendor) in [
            (FormFactor::Desktop, "ASUSTeK COMPUTER INC."),
            (FormFactor::Laptop, "Dell Inc."),
            (FormFactor::Laptop, "HP"),
            (FormFactor::MiniPc, "Intel Corporation"),
            (FormFactor::AllInOne, "LENOVO"),
        ] {
            let mut layouts = vec![(
                vec![gpu(GpuFamily::AmdPolaris, false)],
                display(Some(0), None),
            )];
            if let Some(f) = igpu_family(platform) {
                layouts.push((vec![gpu(f, true)], display(Some(0), Some(0))));
            }
            for (gpus, layout) in layouts {
                let mut p = profile(platform, form, gpus, vendor);
                for legacy in [false, true] {
                    p.firmware_uefi = Some(!legacy);
                    for target in MacOsVersion::ALL {
                        let mut o = BuildOptions {
                            target,
                            ..Default::default()
                        };
                        if legacy {
                            o.picker = PickerStyle::Text;
                        }
                        let label = format!("{platform:?}-{form:?}-{vendor}-{legacy}");
                        checker.check(&p, &layout, &o, &label);
                    }
                }
            }
        }
    }
    // Chipset-specific Booter sets (AM5 MMIO whitelist, TRX40/TRX50, X299 on ASUS).
    for (platform, codename, chipset, vendor) in [
        (
            CpuPlatform::AmdZen4,
            "Raphael",
            "B650",
            "ASUSTeK COMPUTER INC.",
        ),
        (
            CpuPlatform::AmdZen2,
            "Castle Peak",
            "X570",
            "Gigabyte Technology Co., Ltd.",
        ),
        (
            CpuPlatform::AmdZen4,
            "Storm Peak",
            "TRX50",
            "ASUSTeK COMPUTER INC.",
        ),
        (
            CpuPlatform::AmdZen3,
            "Vermeer",
            "B450",
            "Micro-Star International Co., Ltd.",
        ),
        (
            CpuPlatform::SkylakeX,
            "Skylake-X",
            "X299",
            "ASUSTeK COMPUTER INC.",
        ),
        (
            CpuPlatform::CoffeeLake,
            "Coffee Lake-S",
            "Z390",
            "Gigabyte Technology Co., Ltd.",
        ),
    ] {
        let mut p = profile(
            platform,
            FormFactor::Desktop,
            vec![gpu(GpuFamily::AmdNavi21, false)],
            vendor,
        );
        p.cpu.codename = codename.into();
        p.chipset = Some(chipset.into());
        for target in MacOsVersion::ALL {
            let o = BuildOptions {
                target,
                ..Default::default()
            };
            checker.check(
                &p,
                &display(Some(0), None),
                &o,
                &format!("{codename}-{chipset}"),
            );
        }
    }
    // Virtual machines, with and without a known guest CPU model.
    for kind in [VmKind::Kvm, VmKind::Vmware, VmKind::HyperV, VmKind::Other] {
        for platform in [
            CpuPlatform::Unknown,
            CpuPlatform::Haswell,
            CpuPlatform::AmdZen3,
        ] {
            let mut p = profile(
                platform,
                FormFactor::Desktop,
                vec![gpu(GpuFamily::VirtualDisplay, false)],
                "QEMU",
            );
            p.vm = Some(kind);
            for target in MacOsVersion::ALL {
                let o = BuildOptions {
                    target,
                    ..Default::default()
                };
                checker.check(
                    &p,
                    &display(None, None),
                    &o,
                    &format!("vm-{kind:?}-{platform:?}"),
                );
            }
        }
    }
    println!("ocvalidate checked {} configs", checker.checked);
    assert!(
        checker.failures.is_empty(),
        "{} failures:\n{}",
        checker.failures.len(),
        checker.failures.join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
