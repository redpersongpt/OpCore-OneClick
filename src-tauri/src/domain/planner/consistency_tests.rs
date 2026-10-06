//! The compatibility report and the planner must agree on every release:
//! what the report calls supported or offers as an expert option builds, and
//! what it calls unsupported is refused by `plan`.

use super::test_support::*;
use super::*;
use crate::contracts::SupportLevel;
use crate::domain::model::{GpuFamily, ProfileAudio, VmKind};
use crate::domain::{compatibility, kext_catalog};

fn igpu_of(platform: CpuPlatform) -> Option<GpuFamily> {
    use CpuPlatform as P;
    use GpuFamily as G;
    Some(match platform {
        P::Arrandale | P::Lynnfield => G::IntelIronLake,
        P::SandyBridge => G::IntelSandyBridge,
        P::IvyBridge => G::IntelIvyBridge,
        P::Haswell => G::IntelHaswell,
        P::Broadwell => G::IntelBroadwell,
        P::Skylake => G::IntelSkylake,
        P::KabyLake => G::IntelKabyLake,
        P::CoffeeLake => G::IntelCoffeeLake,
        P::CometLake => G::IntelCometLake,
        P::IceLake => G::IntelIceLake,
        P::RocketLake | P::TigerLake | P::AlderLake | P::RaptorLake | P::ArrowLake => G::IntelXe,
        P::IntelAtom => G::IntelLowPower,
        P::AmdZen | P::AmdZen2 | P::AmdZen3 => G::AmdApuVega,
        P::AmdZen4 | P::AmdZen5 => G::AmdApuRdna,
        _ => return None,
    })
}

const DGPUS: &[GpuFamily] = &[
    GpuFamily::AmdPolaris,
    GpuFamily::AmdNavi21,
    GpuFamily::AmdNavi22,
    GpuFamily::AmdGcn2,
    GpuFamily::NvidiaKepler,
    GpuFamily::NvidiaPascal,
    GpuFamily::NvidiaFermi,
    GpuFamily::NvidiaModern,
];

/// Profiles across every platform, form factor and GPU layout, plus VMs,
/// CPUs without AVX2, AMD core counts the patches cannot use and legacy BIOS.
fn profiles() -> Vec<(String, HardwareProfile)> {
    let mut out = Vec::new();
    let mut platforms: Vec<CpuPlatform> = cpu_db::all_platforms().to_vec();
    platforms.extend([CpuPlatform::Unknown, CpuPlatform::AppleSilicon]);
    for &platform in &platforms {
        for form in [FormFactor::Desktop, FormFactor::Laptop] {
            let mut layouts: Vec<Vec<ProfileGpu>> = vec![vec![]];
            let igpu = igpu_of(platform);
            if let Some(f) = igpu {
                layouts.push(vec![gpu(f, true)]);
            }
            for &d in DGPUS {
                layouts.push(vec![gpu(d, false)]);
                if let Some(f) = igpu {
                    layouts.push(vec![gpu(f, true), gpu(d, false)]);
                }
            }
            for gpus in layouts {
                let label = format!(
                    "{platform:?}/{form:?}/{:?}",
                    gpus.iter().map(|g| g.family).collect::<Vec<_>>()
                );
                let mut p = profile(cpu(platform, "Test CPU", "Test", 6), form, gpus);
                p.audio = Some(ProfileAudio {
                    codec_name: "Realtek ALC897".into(),
                    codec_id: Some(0x10ec_0897),
                    ..Default::default()
                });
                out.push((label.clone(), p.clone()));
                if cpu_db::platform_info(platform).has_avx2 {
                    let mut no_avx2 = p.clone();
                    no_avx2.cpu.has_avx2 = Some(false);
                    out.push((format!("{label}/no-avx2"), no_avx2));
                }
                if form == FormFactor::Desktop {
                    let mut legacy = p.clone();
                    legacy.firmware_uefi = Some(false);
                    out.push((format!("{label}/legacy"), legacy));
                }
            }
        }
        // AMD core counts the kernel patches cannot be built for.
        if cpu_db::platform_info(platform).vendor == CpuVendor::Amd {
            for cores in [0, 96] {
                let mut p = profile(
                    cpu(platform, "Test CPU", "Test", cores),
                    FormFactor::Desktop,
                    vec![gpu(GpuFamily::AmdPolaris, false)],
                );
                p.cpu.threads = cores * 2;
                out.push((format!("{platform:?}/{cores} cores"), p));
            }
        }
        for kind in [VmKind::Kvm, VmKind::HyperV] {
            let mut p = vm_profile(platform, kind);
            out.push((format!("{platform:?}/vm-{kind:?}"), p.clone()));
            p.cpu.has_avx2 = Some(false);
            out.push((format!("{platform:?}/vm-{kind:?}/no-avx2"), p.clone()));
            p.gpus.push(gpu(GpuFamily::AmdPolaris, false));
            out.push((format!("{platform:?}/vm-{kind:?}/no-avx2/polaris"), p));
        }
    }
    out
}

/// Compare report and planner for every release of one share of the
/// profiles (split so the shares run in parallel).
fn check_share(share: usize) {
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for (label, p) in profiles().into_iter().skip(share).step_by(SHARES) {
        for target in MacOsVersion::ALL {
            let report = compatibility::assess(&p, Some(target));
            let option = report.versions.iter().find(|o| o.version == target);
            let supported = option.is_some_and(|o| o.supported);
            let expert = !supported && report.level == SupportLevel::Partial;
            let built = plan(&p, &options(target));
            checked += 1;
            if let Ok(built) = &built {
                for k in &built.kexts {
                    let known =
                        kext_catalog::entry(&k.catalog_id).is_some_and(|e| e.provides(&k.bundle));
                    if !known {
                        mismatches.push(format!(
                            "{label} {}: {} {} is not in the catalog",
                            target.id(),
                            k.catalog_id,
                            k.bundle
                        ));
                    }
                }
            }
            match (&built, supported || expert) {
                (Ok(_), false) => mismatches.push(format!(
                    "{label} {}: the report refuses it ({:?}: {}) but plan() builds",
                    target.id(),
                    report.level,
                    report.summary
                )),
                (Err(e), true) => mismatches.push(format!(
                    "{label} {}: the report {} it but plan() fails with {}: {}",
                    target.id(),
                    if supported {
                        "supports"
                    } else {
                        "offers as expert option"
                    },
                    e.code,
                    e.message
                )),
                _ => {}
            }
        }
    }
    assert!(checked > 500, "only {checked} combinations");
    assert!(
        mismatches.is_empty(),
        "{} of {checked} disagree:\n{}",
        mismatches.len(),
        mismatches[..mismatches.len().min(40)].join("\n")
    );
}

const SHARES: usize = 8;

#[test]
fn agree_share_0() {
    check_share(0);
}

#[test]
fn agree_share_1() {
    check_share(1);
}

#[test]
fn agree_share_2() {
    check_share(2);
}

#[test]
fn agree_share_3() {
    check_share(3);
}

#[test]
fn agree_share_4() {
    check_share(4);
}

#[test]
fn agree_share_5() {
    check_share(5);
}

#[test]
fn agree_share_6() {
    check_share(6);
}

#[test]
fn agree_share_7() {
    check_share(7);
}

#[test]
fn every_kind_of_verdict_is_covered() {
    let (mut supported, mut expert, mut refused) = (0, 0, 0);
    for (_, p) in profiles().into_iter().step_by(7) {
        for target in [
            MacOsVersion::HighSierra,
            MacOsVersion::Ventura,
            MacOsVersion::Tahoe,
        ] {
            let report = compatibility::assess(&p, Some(target));
            match report.versions.iter().find(|o| o.version == target) {
                Some(o) if o.supported => supported += 1,
                _ if report.level == SupportLevel::Partial => expert += 1,
                _ => refused += 1,
            }
        }
    }
    assert!(
        supported > 50 && expert > 20 && refused > 50,
        "{supported}/{expert}/{refused}"
    );
}

/// Releases past the native CPU ceiling build with the workaround kexts and
/// a note saying what is lost.
#[test]
fn workaround_targets_get_their_kexts() {
    let has =
        |plan: &BuildPlan, id: &str| plan.kexts.iter().any(|k| k.enabled && k.catalog_id == id);
    let warned = |plan: &BuildPlan, title: &str| {
        plan.notes.iter().any(|n| {
            n.component == "cpu" && n.level == NoteLevel::Warning && n.title.contains(title)
        })
    };

    let ivy = profile(
        cpu(
            CpuPlatform::IvyBridge,
            "Intel(R) Core(TM) i7-3770 CPU @ 3.40GHz",
            "Ivy Bridge",
            4,
        ),
        FormFactor::Desktop,
        vec![gpu(GpuFamily::AmdPolaris, false)],
    );
    let monterey = plan(&ivy, &options(MacOsVersion::Monterey)).expect("native");
    assert!(!has(&monterey, "CryptexFixup") && !has(&monterey, "AppleIntelCPUPowerManagement"));
    let report = compatibility::assess(&ivy, Some(MacOsVersion::Ventura));
    assert_eq!(report.level, SupportLevel::Partial);
    let ventura = plan(&ivy, &options(MacOsVersion::Ventura)).expect("expert option");
    for id in [
        "CryptexFixup",
        "AppleIntelCPUPowerManagement",
        "AppleIntelCPUPowerManagementClient",
    ] {
        assert!(has(&ventura, id), "{id}");
    }
    assert!(warned(&ventura, "AVX2"));
    // Polaris without AVX2 is restored by a root patch, with what it needs.
    assert!(has(&ventura, "AMFIPass"));
    assert_eq!(ventura.csr_active_config, SIP_ROOT_PATCH);
    assert_eq!(ventura.smbios.secure_boot_model, "Disabled");

    let penryn = profile(
        cpu(
            CpuPlatform::Penryn,
            "Intel(R) Core(TM)2 Quad CPU Q9550",
            "Penryn",
            4,
        ),
        FormFactor::Desktop,
        vec![gpu(GpuFamily::AmdPolaris, false)],
    );
    let high_sierra = plan(&penryn, &options(MacOsVersion::HighSierra)).expect("native");
    assert!(!has(&high_sierra, "telemetrap"));
    let mojave = plan(&penryn, &options(MacOsVersion::Mojave)).expect("expert option");
    assert!(has(&mojave, "telemetrap") && warned(&mojave, "SSE4.2"));
    let telemetrap = mojave.kexts.iter().find(|k| k.catalog_id == "telemetrap");
    assert_eq!(
        telemetrap.and_then(|k| k.min_kernel.as_deref()),
        Some("18.0.0")
    );
    assert!(plan(&penryn, &options(MacOsVersion::Ventura)).is_err());

    let mut guest = vm_profile(CpuPlatform::Unknown, VmKind::Kvm);
    guest.cpu.has_avx2 = Some(false);
    assert!(!has(
        &plan(&guest, &options(MacOsVersion::Monterey)).expect("vm"),
        "CryptexFixup"
    ));
    assert!(has(
        &plan(&guest, &options(MacOsVersion::Sonoma)).expect("vm"),
        "CryptexFixup"
    ));
}

/// The notes stage consolidates post-install steps by topic; no plan lists
/// the same step twice.
#[test]
fn post_install_steps_are_not_repeated() {
    let mut repeats = Vec::new();
    for (label, p) in profiles().into_iter().step_by(3) {
        for target in [
            MacOsVersion::Catalina,
            MacOsVersion::Monterey,
            MacOsVersion::Sonoma,
            MacOsVersion::Tahoe,
        ] {
            let Ok(plan) = plan(&p, &options(target)) else {
                continue;
            };
            let mut seen = std::collections::HashSet::new();
            for n in &plan.post_install {
                if !seen.insert(n.title.to_ascii_lowercase()) {
                    repeats.push(format!("{label} {}: {}", target.id(), n.title));
                }
            }
        }
    }
    repeats.sort();
    repeats.dedup();
    assert!(repeats.is_empty(), "{}", repeats.join("\n"));
}
