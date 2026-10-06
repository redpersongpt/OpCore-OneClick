//! Golden matrix: every hardware profile in `fixtures/profiles` runs through
//! the real interpretation (`profile::refresh_profile`), compatibility report,
//! planner and config writer for every macOS release, without network access.
//!
//! - Consistency: a release the report calls supported or an expert option
//!   must plan; an unsupported one must be refused with an error code and a
//!   message. On a machine where some release builds, every refusal must be
//!   recoverable (the user can pick another release); a machine that cannot
//!   run macOS at all may refuse for good (Apple silicon, Atom).
//! - The recommended release per fixture (`EXPECTED`) and the machines that
//!   must not build at all (`MUST_NOT_BUILD`).
//! - Invariants on every plan (see `golden/invariants.rs`) and a config.plist
//!   written from OpenCore 1.0.8's Sample.plist.
//! - A text snapshot per fixture in `snapshots/` (`GOLDEN_UPDATE=1` rewrites).
//! - `golden_ocvalidate` (ignored, needs network once) runs OpenCore's own
//!   ocvalidate on the config of every buildable release:
//!   `cargo test --test golden -- --ignored golden_ocvalidate`.
//!   `OPENCORE_RELEASE_ZIP=<OpenCore-1.0.8-RELEASE.zip>` seeds the download
//!   cache from a local copy; `OCVALIDATE=<binary>` overrides the validator.
//!
//! Failures that belong to other parts of the code base are listed in
//! `KNOWN_FAILURES`; an entry that stops failing fails the run, so the list
//! only shrinks.

#[path = "golden/config.rs"]
mod config;
#[path = "golden/invariants.rs"]
mod invariants;
#[path = "golden/ocvalidate.rs"]
mod ocvalidate;
#[path = "golden/snapshot.rs"]
mod snapshot;
#[path = "golden/support.rs"]
mod support;

use std::collections::HashSet;

use app_lib::build::amd;
use app_lib::domain::model::{
    BuildPlan, CpuVendor, MacOsVersion, NoteLevel, PickerStyle, PlanNote, PlistScalar,
};
use app_lib::domain::planner::graphics;
use app_lib::domain::smbios_gen;
use app_lib::domain::{amd_patches, chipset_db, codec_db, cpu_db, gpu_db, smbios_db};

use support::{settle, Failures, Fixture, FixtureRun, Known, Verdict};

/// Recommended release per fixture (the newest one the report calls fully
/// supported); None = no release is fully supported.
const EXPECTED: &[(&str, Option<&str>)] = &[
    ("aio_i58500_m920z", Some("15")),
    ("alder_i512600k_z690_rx6600", Some("15")),
    ("apple_m2pro_macmini", None),
    ("arrandale_i5520m_t410", Some("10.13")),
    ("arrow_ultra9_285k_rx6800xt", Some("15")),
    ("athlon_3000g_a320", Some("15")),
    ("broadwell_i55300u_t450", Some("12")),
    ("broadwell_nuc5i5ryk", Some("12")),
    ("celeron_n4000_14cf", None),
    ("coffee_i59400f_rx6600xt", Some("15")),
    ("coffee_i78750h_gtx1060_y530", Some("13")),
    ("coffee_i99900k_z390_rx580", Some("13")),
    ("comet_i710510u_x1c8", Some("15")),
    ("comet_i710700k_z490_rx5700xt", Some("15")),
    ("fx8350_990fx_rx570", Some("12")),
    ("haswell_i74700mq_e6540", Some("12")),
    ("haswell_i74790k_hd4600", Some("12")),
    ("haswell_i74790k_rx580", Some("15")),
    ("icelake_i71065g7_xps7390", Some("15")),
    ("ivy_i53320m_t430", Some("11")),
    ("ivy_i73770_gtx770", Some("11")),
    ("kaby_i77700k_rx570", Some("15")),
    ("kabyr_i58250u_s510ua", Some("15")),
    ("lynnfield_i7870_gtx650", Some("11")),
    ("meteor_ultra7_155h_ux3405", None),
    ("nuc8i7beh_iris655", Some("15")),
    ("penryn_q9550_hd4670", Some("10.13")),
    ("pentium_g4560_h110", Some("12")),
    ("raptor_i713700kf_rtx4070", None),
    ("raptor_i913900k_z790_rx6900xt", Some("15")),
    ("rocket_i711700k_rx6800", Some("15")),
    ("ryzen_4700u_ideapad5", Some("15")),
    ("ryzen_5600g_b450_nootedred", Some("15")),
    ("ryzen_5600x_x570_rx580", Some("15")),
    ("ryzen_5800x_b550_rx6700xt", Some("15")),
    ("ryzen_6800u_zenbook", None),
    ("ryzen_6900hs_g14_rx6700s", None),
    ("ryzen_7950x_x670e_rx6950xt", Some("26")),
    ("sandy_i52500k_hd3000", Some("10.13")),
    ("sandy_i52520m_t420", Some("10.13")),
    ("skylake_i56300u_t460", Some("15")),
    ("skylake_i56600k_hd530", Some("15")),
    ("threadripper_3970x_trx40_rx6900xt", Some("15")),
    ("tiger_i71165g7_xps9310", None),
    ("vm_hyperv_i79700k", Some("26")),
    ("vm_kvm_ryzen5950x", Some("26")),
    ("vm_vmware_i78700k", Some("26")),
    ("x299_i910900x_rx6800xt", Some("15")),
    ("x58_xeon_x5670_rx580", Some("12")),
    ("x79_i74930k_r9280x", Some("12")),
    ("x99_i75820k_vega64", Some("15")),
    ("xeon_e31230v3_gtx750ti", Some("10.13")),
    ("xeon_w2145_radeon_vii", Some("15")),
];

/// Machines that must not get any plan: no CPU or GPU path on any release.
const MUST_NOT_BUILD: &[&str] = &[
    "apple_m2pro_macmini",
    "celeron_n4000_14cf",
    "meteor_ultra7_155h_ux3405",
    "raptor_i713700kf_rtx4070",
    "ryzen_6800u_zenbook",
    "tiger_i71165g7_xps9310",
];

/// Failures owned by other parts of the code base (planner stages, knowledge
/// base, compatibility report). Each entry says what has to change. The first
/// matching entry takes a failure, so broader entries go after narrower ones
/// (`fixture_set_covers_the_matrix` rejects an entry that can never match).
const KNOWN_FAILURES: &[Known] = &[
];

const MATRIX_CHECKS: &[&str] = &[
    "consistency",
    "error-code",
    "report-agreement",
    "expectation",
    "gpu-kext",
    "core-kexts",
    "voodooinput",
    "smbios-support",
    "tahoe-smbios",
    "board-id-skip",
    "secure-boot",
    "root-patch",
    "amd-core-count",
    "cryptexfixup",
    "cpuid-spoof",
    "kext-catalog",
    "kext-duplicate",
    "kext-range",
    "audio-layout",
    "igpu-properties",
    "ssdt-source",
    "efi-files",
    "boot-args",
    "apfs-min",
    "xhci-port-limit",
    "usbx",
    "wifi",
    "device-properties",
    "acpi-patch",
    "note-text",
    "schema",
    "config-write",
];

fn runs() -> Vec<FixtureRun> {
    support::fixtures().iter().map(support::run).collect()
}

#[test]
fn fixture_set_covers_the_matrix() {
    let fixtures = support::fixtures();
    assert!(fixtures.len() >= 45, "only {} fixtures", fixtures.len());
    let names: HashSet<&str> = fixtures.iter().map(|f| f.name.as_str()).collect();
    let expected: HashSet<&str> = EXPECTED.iter().map(|(n, _)| *n).collect();
    let missing: Vec<_> = names.difference(&expected).collect();
    let stale: Vec<_> = expected.difference(&names).collect();
    assert!(
        missing.is_empty(),
        "fixtures without an EXPECTED entry: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "EXPECTED entries without a fixture: {stale:?}"
    );
    for name in MUST_NOT_BUILD {
        assert!(
            names.contains(name),
            "MUST_NOT_BUILD names an unknown fixture {name}"
        );
        assert!(
            EXPECTED.iter().any(|(n, r)| n == name && r.is_none()),
            "{name} must not build but has a recommended release"
        );
    }
    for check in invariants::CHECKS {
        assert!(
            MATRIX_CHECKS.contains(check),
            "MATRIX_CHECKS misses the invariant {check}"
        );
    }
    for (i, later) in KNOWN_FAILURES.iter().enumerate() {
        if let Some(k) = KNOWN_FAILURES[..i].iter().find(|k| k.shadows(later)) {
            panic!(
                "KNOWN_FAILURES entry {} @ {} [{}] never matches: {} @ {} [{}] comes first and takes \
                 all of its failures",
                later.fixture, later.target, later.check, k.fixture, k.target, k.check
            );
        }
    }
    for k in KNOWN_FAILURES {
        assert!(
            k.fixture == "*" || names.contains(k.fixture),
            "KNOWN_FAILURES names an unknown fixture {}",
            k.fixture
        );
        assert!(
            k.target == "*" || MacOsVersion::ALL.iter().any(|v| v.id() == k.target),
            "KNOWN_FAILURES entry {} has a bad target {}",
            k.fixture,
            k.target
        );
        assert!(
            MATRIX_CHECKS.contains(&k.check) || ["realism", "ocvalidate"].contains(&k.check),
            "KNOWN_FAILURES entry {} has an unknown check {}",
            k.fixture,
            k.check
        );
    }
}

/// The fixtures look like what the scanner produces: the CPU, GPU, codec and
/// chipset fields are what the knowledge base derives from the ids, and
/// `refresh_profile` leaves them unchanged.
#[test]
fn fixtures_match_the_knowledge_base() {
    let mut out = Failures::default();
    for Fixture { name, profile } in support::fixtures() {
        let mut fail = |detail: String| out.push(&name, None, "realism", detail);
        let c = &profile.cpu;
        let vendor = match c.vendor {
            CpuVendor::Intel => "GenuineIntel",
            CpuVendor::Amd => "AuthenticAMD",
            CpuVendor::Apple => "Apple",
            CpuVendor::Unknown => "",
        };
        let id = cpu_db::identify(&c.name, vendor, c.family, c.model, c.stepping);
        if (
            id.platform,
            id.codename.as_str(),
            id.is_mobile,
            id.is_hybrid,
        ) != (c.platform, c.codename.as_str(), c.is_mobile, c.is_hybrid)
        {
            fail(format!(
                "CPU identifies as {:?} {} mobile={} hybrid={}",
                id.platform, id.codename, id.is_mobile, id.is_hybrid
            ));
        }
        if c.has_avx2 != Some(id.has_avx2) {
            fail(format!(
                "CPU AVX2 {:?}, knowledge base {}",
                c.has_avx2, id.has_avx2
            ));
        }
        for g in &profile.gpus {
            let gi = gpu_db::identify(g.vendor_id.as_deref(), g.device_id.as_deref(), &g.name);
            if (gi.vendor, gi.family, gi.is_igpu) != (g.vendor, g.family, g.is_igpu) {
                fail(format!(
                    "GPU {} identifies as {:?} {:?} igpu={}",
                    g.name, gi.vendor, gi.family, gi.is_igpu
                ));
            }
        }
        if let Some(a) = &profile.audio {
            match a.codec_id {
                Some(codec) if codec_db::lookup(codec).is_some() => {}
                other => fail(format!("codec {other:?} is unknown")),
            }
        }
        if let Some(chipset) = profile.chipset.as_deref() {
            if chipset_db::from_name(chipset).is_none() {
                fail(format!("chipset {chipset} is unknown"));
            }
        }
        let before = serde_json::to_value(&profile).unwrap();
        let after =
            serde_json::to_value(app_lib::domain::profile::refresh_profile(profile.clone()))
                .unwrap();
        if before != after {
            fail("refresh_profile changes the scanned profile".into());
        }
    }
    settle(out, KNOWN_FAILURES, &["realism"]);
}

#[test]
fn tahoe_models_match_smbios_db() {
    let mut db: Vec<&str> = smbios_db::models_supporting(MacOsVersion::Tahoe)
        .iter()
        .map(|m| m.model)
        .collect();
    db.sort_unstable();
    let mut listed = invariants::TAHOE_MODELS.to_vec();
    listed.sort_unstable();
    assert_eq!(db, listed);
}

/// One deliberate defect for the invariant self-test: `edit` breaks the plan
/// of `fixture` for `target`, and `check` must report it.
struct Damage {
    check: &'static str,
    fixture: &'static str,
    target: MacOsVersion,
    edit: fn(&mut BuildPlan),
}

const COFFEE: &str = "coffee_i99900k_z390_rx580";

fn secure_boot(plan: &mut BuildPlan, model: &str) {
    plan.misc_security
        .insert("SecureBootModel".into(), PlistScalar::Str(model.into()));
    plan.smbios.secure_boot_model = model.into();
}

const DAMAGE: &[Damage] = &[
    Damage {
        check: "core-kexts",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.kexts.retain(|k| k.bundle != "Lilu.kext"),
    },
    Damage {
        check: "gpu-kext",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.kexts.retain(|k| k.catalog_id != "WhateverGreen"),
    },
    Damage {
        check: "voodooinput",
        fixture: "kabyr_i58250u_s510ua",
        target: MacOsVersion::Sequoia,
        edit: |p| {
            for k in &mut p.kexts {
                for plugin in &mut k.plugins {
                    if plugin.bundle == "VoodooInput.kext" {
                        plugin.enabled = true;
                    }
                }
            }
        },
    },
    Damage {
        check: "smbios-support",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.smbios.model = "iMac18,3".into(),
    },
    Damage {
        check: "tahoe-smbios",
        fixture: COFFEE,
        target: MacOsVersion::Tahoe,
        edit: |p| p.smbios.model = "iMac19,1".into(),
    },
    Damage {
        check: "board-id-skip",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.smbios.board_id_skip = true;
            p.booter_patches.iter_mut().for_each(|b| b.enabled = false);
        },
    },
    Damage {
        check: "secure-boot",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.misc_security
                .insert("SecureBootModel".into(), PlistScalar::Str("Default".into()));
        },
    },
    Damage {
        check: "secure-boot",
        fixture: COFFEE,
        target: MacOsVersion::Mojave,
        edit: |p| secure_boot(p, "Default"),
    },
    Damage {
        check: "root-patch",
        fixture: "haswell_i74790k_hd4600",
        target: MacOsVersion::Ventura,
        edit: |p| p.csr_active_config = 0,
    },
    Damage {
        check: "amd-core-count",
        fixture: "ryzen_5600x_x570_rx580",
        target: MacOsVersion::Sequoia,
        edit: |p| p.amd_core_count = Some(8),
    },
    Damage {
        check: "amd-core-count",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.amd_core_count = Some(8),
    },
    Damage {
        check: "cryptexfixup",
        fixture: "ivy_i73770_gtx770",
        target: MacOsVersion::Ventura,
        edit: |p| p.kexts.retain(|k| k.catalog_id != "CryptexFixup"),
    },
    Damage {
        check: "cpuid-spoof",
        fixture: "alder_i512600k_z690_rx6600",
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.kernel_emulate.remove("Cpuid1Data");
        },
    },
    Damage {
        check: "kext-catalog",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.kexts[0].catalog_id = "NoSuchKext".into(),
    },
    Damage {
        check: "kext-duplicate",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            let alc = p.kexts.iter().find(|k| k.catalog_id == "AppleALC").cloned();
            p.kexts.extend(alc);
        },
    },
    Damage {
        check: "kext-range",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.kexts[0].min_kernel = Some("21.0".into()),
    },
    Damage {
        check: "audio-layout",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.device_properties
                .retain(|e| e.properties.iter().all(|d| d.key != "layout-id"));
            p.boot_args.retain(|a| !a.starts_with("alcid="));
        },
    },
    Damage {
        check: "igpu-properties",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.device_properties
                .retain(|e| e.path != graphics::IGPU_PATH)
        },
    },
    Damage {
        check: "ssdt-source",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            let first = p.ssdts[0].clone();
            p.ssdts.push(first);
        },
    },
    Damage {
        check: "efi-files",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.drivers.retain(|d| d.path != "OpenRuntime.efi"),
    },
    Damage {
        check: "boot-args",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.boot_args.push("revpatch=sbvmm".into());
            p.kexts.retain(|k| k.catalog_id != "RestrictEvents");
        },
    },
    Damage {
        check: "apfs-min",
        fixture: COFFEE,
        target: MacOsVersion::Mojave,
        edit: |p| {
            p.uefi_apfs.insert("MinDate".into(), PlistScalar::Int(0));
        },
    },
    Damage {
        check: "xhci-port-limit",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            p.kernel_quirks
                .insert("XhciPortLimit".into(), PlistScalar::Bool(true));
        },
    },
    Damage {
        check: "usbx",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| p.ssdts.retain(|s| !s.file_name.contains("USBX")),
    },
    Damage {
        check: "wifi",
        fixture: "broadwell_i55300u_t450",
        target: MacOsVersion::Monterey,
        edit: |p| secure_boot(p, "Disabled"),
    },
    Damage {
        check: "device-properties",
        fixture: COFFEE,
        target: MacOsVersion::Sequoia,
        edit: |p| {
            let entry = &mut p.device_properties[0];
            let first = entry.properties[0].clone();
            entry.properties.push(first);
        },
    },
    Damage {
        check: "acpi-patch",
        fixture: "ivy_i53320m_t430",
        target: MacOsVersion::BigSur,
        edit: |p| p.ssdts.retain(|s| !s.file_name.contains("XOSI")),
    },
    Damage {
        check: "note-text",
        fixture: COFFEE,
        target: MacOsVersion::Ventura,
        edit: |p| {
            p.notes.push(PlanNote {
                level: NoteLevel::Info,
                component: "usb".into(),
                title: "Repeated".into(),
                detail:
                    "This sentence is long enough to count. This sentence is long enough to count."
                        .into(),
            });
        },
    },
];

/// The invariants are not vacuous: each damaged copy of a good plan trips
/// the check that guards it, which the untouched plan passes.
#[test]
fn invariants_catch_broken_plans() {
    let mut fixtures: Vec<&str> = DAMAGE.iter().map(|d| d.fixture).collect();
    fixtures.sort_unstable();
    fixtures.dedup();
    let runs: Vec<FixtureRun> = support::fixtures()
        .iter()
        .filter(|f| fixtures.contains(&f.name.as_str()))
        .map(support::run)
        .collect();
    assert_eq!(runs.len(), fixtures.len(), "self-test fixture missing");
    let violations = |fixture: &str, target: MacOsVersion, edit: fn(&mut BuildPlan)| {
        let run = runs.iter().find(|r| r.name == fixture).unwrap();
        let t = run.target(target);
        let mut plan = t
            .plan
            .as_ref()
            .unwrap_or_else(|e| panic!("{fixture} @ {} does not build: {e}", target.id()))
            .clone();
        edit(&mut plan);
        let mut out = Failures::default();
        invariants::check(run, t, &plan, &mut out);
        out.0.into_iter().map(|f| f.check).collect::<Vec<_>>()
    };
    for d in DAMAGE {
        let clean = violations(d.fixture, d.target, |_| {});
        assert!(
            !clean.contains(&d.check),
            "{} @ {} fails {} before the damage",
            d.fixture,
            d.target.id(),
            d.check
        );
        let found = violations(d.fixture, d.target, d.edit);
        assert!(
            found.contains(&d.check),
            "{} @ {} not detected on {}: {found:?}",
            d.check,
            d.target.id(),
            d.fixture
        );
    }
    let covered: HashSet<&str> = DAMAGE.iter().map(|d| d.check).collect();
    let untested: Vec<&str> = invariants::CHECKS
        .iter()
        .copied()
        .filter(|c| !covered.contains(c))
        .collect();
    assert!(
        untested.is_empty(),
        "invariants without a self-test: {untested:?}"
    );

    // Overrides must exist in Sample.plist with the same type.
    let run = runs.iter().find(|r| r.name == COFFEE).unwrap();
    let mut plan = run.target(MacOsVersion::Sequoia).plan.clone().unwrap();
    let sample = config::sample_root();
    assert!(config::schema_errors(&sample, &plan).is_empty());
    plan.kernel_quirks
        .insert("NoSuchQuirk".into(), PlistScalar::Bool(true));
    plan.kernel_quirks
        .insert("XhciPortLimit".into(), PlistScalar::Int(1));
    assert_eq!(config::schema_errors(&sample, &plan).len(), 2);
}

fn check_consistency(run: &FixtureRun, out: &mut Failures) {
    let name = run.name.as_str();
    let buildable = run.buildable();
    let expected_rec = EXPECTED
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, r)| *r);
    let rec = run.overview.recommended;
    if rec.map(MacOsVersion::id) != expected_rec {
        out.push(
            name,
            None,
            "expectation",
            format!(
                "recommended {}, expected {}",
                rec.map_or("none", MacOsVersion::id),
                expected_rec.unwrap_or("none")
            ),
        );
    }
    if MUST_NOT_BUILD.contains(&name) && buildable {
        let ok: Vec<&str> = run
            .targets
            .iter()
            .filter(|t| t.plan.is_ok())
            .map(|t| t.target.id())
            .collect();
        out.push(
            name,
            None,
            "expectation",
            format!("must not build, but plans for {ok:?}"),
        );
    }
    if let Some(rec) = rec {
        if run.target(rec).verdict != Verdict::Supported {
            out.push(
                name,
                Some(rec),
                "report-agreement",
                "the recommended release is not supported",
            );
        }
    }
    for t in &run.targets {
        let overview = run.overview.versions.iter().find(|o| o.version == t.target);
        let selected = t.report.versions.iter().find(|o| o.version == t.target);
        if overview.map(|o| (o.supported, o.needs_root_patch))
            != selected.map(|o| (o.supported, o.needs_root_patch))
        {
            out.push(
                name,
                Some(t.target),
                "report-agreement",
                "assess(None) and assess(Some) disagree on this release",
            );
        }
        match (&t.plan, t.verdict.buildable()) {
            (Ok(_), true) | (Err(_), false) => {}
            (Ok(_), false) => out.push(
                name,
                Some(t.target),
                "consistency",
                format!(
                    "report says {}, but the planner builds it",
                    t.verdict.label()
                ),
            ),
            (Err(e), true) => out.push(
                name,
                Some(t.target),
                "consistency",
                format!(
                    "report says {}, but the planner refuses: {e}",
                    t.verdict.label()
                ),
            ),
        }
        if let Err(e) = &t.plan {
            if e.code.trim().is_empty() || e.message.trim().is_empty() {
                out.push(
                    name,
                    Some(t.target),
                    "error-code",
                    "error without code or message",
                );
            } else if buildable && !e.recoverable {
                out.push(
                    name,
                    Some(t.target),
                    "error-code",
                    format!(
                        "{} is not recoverable although other releases build",
                        e.code
                    ),
                );
            }
        }
    }
}

fn check_plans(run: &FixtureRun, sample: &plist::Dictionary, out: &mut Failures) {
    for t in &run.targets {
        let Ok(plan) = &t.plan else { continue };
        invariants::check(run, t, plan, out);
        for e in config::schema_errors(sample, plan) {
            out.push(&run.name, Some(t.target), "schema", e);
        }
        let kernel_add = config::kernel_add(plan);
        let identity = config::identity(plan);
        let picker = support::options(t.target).picker;
        if let Err(e) = config::write(config::SAMPLE, plan, &kernel_add, &identity, picker) {
            out.push(&run.name, Some(t.target), "config-write", e.to_string());
        }
    }
}

#[test]
fn golden_matrix() {
    let sample = config::sample_root();
    let mut out = Failures::default();
    for run in runs() {
        check_consistency(&run, &mut out);
        check_plans(&run, &sample, &mut out);
    }
    settle(out, KNOWN_FAILURES, MATRIX_CHECKS);
}

#[test]
fn golden_snapshots() {
    let mut errors = Vec::new();
    let mut written = HashSet::new();
    for run in runs() {
        let text = snapshot::render(&run);
        written.insert(format!("{}.txt", run.name));
        if let Err(e) = snapshot::compare(&run.name, &text) {
            errors.push(e);
        }
    }
    let dir = support::tests_dir().join("snapshots");
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".txt") && !written.contains(&name) {
            if snapshot::updating() {
                std::fs::remove_file(entry.path()).unwrap();
            } else {
                errors.push(format!("{name} belongs to no fixture"));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// OpenCore's ocvalidate on the config of every buildable release, AMD
/// kernel patches and a macserial identity included. Missing files are not
/// part of this check: ocvalidate only reads config.plist.
#[tokio::test]
#[ignore]
async fn golden_ocvalidate() {
    let dl = ocvalidate::downloader();
    let package = ocvalidate::opencore(&dl).await;
    let sample = std::fs::read(package.sample_plist()).unwrap();
    assert_eq!(
        sample,
        config::SAMPLE,
        "fixtures/Sample-1.0.8.plist differs from the release"
    );
    let bin = std::env::var_os("OCVALIDATE")
        .map(Into::into)
        .or_else(|| package.ocvalidate())
        .expect("no ocvalidate for this host");
    let macserial = package.macserial();
    let dir = ocvalidate::cache_root().join("configs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut out = Failures::default();
    let mut checked = 0;
    for run in runs() {
        for t in &run.targets {
            let Ok(plan) = &t.plan else { continue };
            let mut plan = plan.clone();
            if let Some(cores) = plan.amd_core_count {
                let bytes = ocvalidate::fetch(&dl, amd_patches::pin_for_target(t.target)).await;
                let (patches, _) =
                    amd::prepare(&bytes, cores, &run.profile).expect("AMD_Vanilla patches");
                plan.kernel_patches.extend(patches);
            }
            let identity =
                smbios_gen::generate_identity(&plan.smbios.model, macserial.as_deref(), None)
                    .unwrap_or_else(|_| config::identity(&plan));
            let kernel_add = config::kernel_add(&plan);
            let picker: PickerStyle = support::options(t.target).picker;
            let written = match config::write(&sample, &plan, &kernel_add, &identity, picker) {
                Ok(bytes) => bytes,
                Err(e) => {
                    out.push(
                        &run.name,
                        Some(t.target),
                        "ocvalidate",
                        format!("config not written: {e}"),
                    );
                    continue;
                }
            };
            let path = dir.join(format!("{}-{}.plist", run.name, t.target.id()));
            std::fs::write(&path, written).unwrap();
            let (ok, text) = ocvalidate::run(&bin, &path);
            if !ok {
                let issues: Vec<&str> = text
                    .lines()
                    .filter(|l| {
                        !l.trim().is_empty()
                            && !l.starts_with("Serialized")
                            && !l.contains("Completed validating")
                    })
                    .collect();
                out.push(&run.name, Some(t.target), "ocvalidate", issues.join(" | "));
            }
            checked += 1;
        }
    }
    eprintln!("ocvalidate checked {checked} configs in {}", dir.display());
    settle(out, KNOWN_FAILURES, &["ocvalidate"]);
}
