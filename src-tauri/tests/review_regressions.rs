use app_lib::domain::model::{
    BuildOptions, BuildPlan, GpuFamily, HardwareProfile, MacOsVersion, PlistScalar,
};
use app_lib::domain::{codec_db, compatibility, planner};

fn fixture(name: &str) -> HardwareProfile {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profiles")
        .join(format!("{name}.json"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn build(profile: &HardwareProfile, target: MacOsVersion) -> BuildPlan {
    planner::plan(
        profile,
        &BuildOptions {
            target,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn legacy_cpu_kernel_boundaries() {
    let p = fixture("x58_xeon_x5670_rx580");
    let monterey = build(&p, MacOsVersion::Monterey);
    let zlib = monterey
        .kexts
        .iter()
        .find(|k| k.catalog_id == "NoAVXFSCompressionTypeZlib")
        .unwrap();
    assert_eq!(zlib.min_kernel.as_deref(), Some("21.5.0"));
    assert_eq!(zlib.max_kernel.as_deref(), Some("21.99.99"));
    let aspp = monterey
        .kexts
        .iter()
        .find(|k| k.catalog_id == "ASPP-Override")
        .unwrap();
    assert_eq!(aspp.min_kernel.as_deref(), Some("21.4.0"));
    let surplus: Vec<_> = monterey
        .kernel_patches
        .iter()
        .filter(|p| p.comment.starts_with("SurPlus"))
        .collect();
    assert_eq!(surplus.len(), 2);
    assert!(surplus
        .iter()
        .all(|p| p.min_kernel == "20.4.0" && p.max_kernel == "21.1.0" && p.count == 1));
    let ventura = build(&p, MacOsVersion::Ventura);
    let zlib = ventura
        .kexts
        .iter()
        .find(|k| k.catalog_id == "NoAVXFSCompressionTypeZlib-AVXpel")
        .unwrap();
    assert_eq!(zlib.min_kernel.as_deref(), Some("22.0.0"));
    assert!(zlib.max_kernel.is_none());
    assert!(ventura
        .notes
        .iter()
        .any(|n| n.detail.contains("USB 2.0 hub")));
    let native = build(&fixture("haswell_i74790k_rx580"), MacOsVersion::Monterey);
    assert!(!native
        .kexts
        .iter()
        .any(|k| k.catalog_id.starts_with("NoAVX") || k.catalog_id == "ASPP-Override"));
    assert!(!native
        .kernel_patches
        .iter()
        .any(|p| p.comment.starts_with("SurPlus")));
}

#[test]
fn scanned_instruction_flags_override_platform_guesses() {
    let mut p = fixture("pentium_g4560_h110");
    p.cpu.has_avx = Some(false);
    assert!(build(&p, MacOsVersion::Monterey)
        .kexts
        .iter()
        .any(|k| k.catalog_id == "NoAVXFSCompressionTypeZlib"));
    p.cpu.has_avx = Some(true);
    assert!(!build(&p, MacOsVersion::Monterey)
        .kexts
        .iter()
        .any(|k| k.catalog_id.starts_with("NoAVX")));
    p.cpu.has_rdrand = Some(false);
    assert_eq!(
        build(&p, MacOsVersion::BigSur)
            .kernel_patches
            .iter()
            .filter(|p| p.comment.starts_with("SurPlus"))
            .count(),
        2
    );
    p.cpu.has_rdrand = Some(true);
    assert!(!build(&p, MacOsVersion::BigSur)
        .kernel_patches
        .iter()
        .any(|p| p.comment.starts_with("SurPlus")));
}

#[test]
fn penryn_metal_graphics_get_sse_emulation() {
    let mut p = fixture("penryn_q9550_hd4670");
    p.gpus = fixture("haswell_i74790k_rx580")
        .gpus
        .into_iter()
        .filter(|g| g.family == GpuFamily::AmdPolaris)
        .collect();
    assert!(build(&p, MacOsVersion::Mojave)
        .kexts
        .iter()
        .any(|k| k.catalog_id == "AAAMouSSE"));
    assert!(!build(&p, MacOsVersion::HighSierra)
        .kexts
        .iter()
        .any(|k| k.catalog_id == "AAAMouSSE"));
}

#[test]
fn risky_helpers_are_opt_in_and_disabled_graphics_are_hidden() {
    let p = build(
        &fixture("alder_i512600k_z690_rx6600"),
        MacOsVersion::Monterey,
    );
    assert!(
        !p.kexts
            .iter()
            .find(|k| k.catalog_id == "CpuTopologyRebuild")
            .unwrap()
            .enabled
    );
    assert!(p.device_properties.iter().any(|e| e
        .properties
        .iter()
        .any(|p| p.key == "class-code" && p.value == PlistScalar::Data("FFFFFFFF".into()))));
    let p = build(&fixture("ryzen_4700u_ideapad5"), MacOsVersion::Sequoia);
    assert!(
        !p.kexts
            .iter()
            .find(|k| k.catalog_id == "GenericUSBXHCI")
            .unwrap()
            .enabled
    );
    assert!(!p.drivers.iter().any(|d| d.path == "ResetNvramEntry.efi"));
}

#[test]
fn touchscreen_coexists_with_rmi_and_low_uma_is_reported() {
    let p = build(&fixture("icelake_i71065g7_xps7390"), MacOsVersion::Ventura);
    assert!(p
        .kexts
        .iter()
        .any(|k| k.bundle == "VoodooI2CHID.kext" && k.enabled));
    let mut apu = fixture("ryzen_4700u_ideapad5");
    apu.gpus
        .iter_mut()
        .find(|g| g.family == GpuFamily::AmdApuVega)
        .unwrap()
        .vram_mb = Some(256);
    assert!(build(&apu, MacOsVersion::Sequoia)
        .notes
        .iter()
        .any(|n| n.title == "Insufficient UMA memory"));
    assert!(
        serde_json::to_string(&compatibility::assess(&apu, Some(MacOsVersion::Sequoia)))
            .unwrap()
            .contains("512 MiB")
    );
}

#[test]
fn chassis_audio_matching_does_not_confuse_similar_models() {
    for (codec, vendor, model, expected) in [
        (0x14f1506e, 0x17aa0000, "ThinkPad T420", 14),
        (0x10ec0269, 0x17aa0000, "ThinkPad T430", 23),
        (0x10ec0292, 0x10280000, "Latitude E6540", 55),
    ] {
        assert_eq!(
            codec_db::ranked_layouts_for_model(codec, Some(vendor), true, Some(model)).first(),
            Some(&expected),
            "{model}"
        );
    }
    assert_ne!(
        codec_db::ranked_layouts_for_model(0x10ec0257, Some(0x17aa0000), true, Some("Legion Y530"))
            .first(),
        Some(&18)
    );
}
