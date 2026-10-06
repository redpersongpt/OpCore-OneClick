//! Structural rules every buildable plan must follow, whatever the machine.

use std::collections::HashSet;

use app_lib::domain::cpu_db;
use app_lib::domain::kext_catalog;
use app_lib::domain::model::{
    BuildPlan, CpuPlatform, CpuVendor, DriverPlan, GpuFamily, GpuVendor, HardwareProfile,
    KextSelection, MacOsVersion, PlistScalar, SsdtSource,
};
use app_lib::domain::planner::{self, graphics, PlanContext};
use app_lib::domain::smbios_db;

use super::config;
use super::support::{options, Failures, FixtureRun, TargetRun};

/// Models Apple supports on macOS 26 Tahoe (DESIGN key facts).
pub const TAHOE_MODELS: &[&str] = &[
    "MacPro7,1",
    "iMac20,1",
    "iMac20,2",
    "MacBookPro16,1",
    "MacBookPro16,2",
    "MacBookPro16,4",
];

/// Every check `check` reports under.
pub const CHECKS: &[&str] = &[
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
    "kext-duplicate",
    "kext-catalog",
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
];

const GPU_KEXTS: &[&str] = &["WhateverGreen", "NootRX", "NootedRed"];

/// Intel platforms that only boot with a CPUID spoof (Comet Lake identity).
const SPOOFED_PLATFORMS: &[CpuPlatform] = &[
    CpuPlatform::RocketLake,
    CpuPlatform::AlderLake,
    CpuPlatform::RaptorLake,
    CpuPlatform::ArrowLake,
];

/// Skylake-era and newer models: macOS expects a USBX device for USB power
/// with them (Dortania smbios-support), so the plan must ship an EC-USBX SSDT.
const USBX_MODELS: &[&str] = &[
    "iMac17,",
    "iMac18,",
    "iMac19,",
    "iMac20,",
    "iMacPro1,",
    "MacPro7,",
    "Macmini8,",
    "MacBook9,",
    "MacBook10,",
    "MacBookAir8,",
    "MacBookAir9,",
    "MacBookPro13,",
    "MacBookPro14,",
    "MacBookPro15,",
    "MacBookPro16,",
];

fn enabled(plan: &BuildPlan) -> impl Iterator<Item = &KextSelection> {
    plan.kexts.iter().filter(|k| k.enabled)
}

fn has_kext(plan: &BuildPlan, catalog_id: &str, bundle: &str) -> bool {
    enabled(plan).any(|k| k.catalog_id == catalog_id && k.bundle.eq_ignore_ascii_case(bundle))
}

/// `SecureBootModel` as written to config.plist (Misc/Security overrides the
/// SMBIOS stage's choice).
pub fn secure_boot_model(plan: &BuildPlan) -> String {
    match plan.misc_security.get("SecureBootModel") {
        Some(PlistScalar::Str(s)) => s.clone(),
        _ => plan.smbios.secure_boot_model.clone(),
    }
}

/// "21.0.0" → (21, 0, 0); None for anything else.
fn kernel(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

fn range_ok(min: Option<&str>, max: Option<&str>) -> Result<(), String> {
    for v in [min, max].into_iter().flatten() {
        if !v.is_empty() && kernel(v).is_none() {
            return Err(format!("kernel version '{v}' is not N.N.N"));
        }
    }
    Ok(())
}

/// The kernel range overlaps some point release of `target`.
fn loads_on(min: Option<&str>, max: Option<&str>, target: MacOsVersion) -> bool {
    let first = (target.darwin_major(), 0, 0);
    let last = (target.darwin_major(), 99, 99);
    let low = min.and_then(kernel).is_none_or(|m| m <= last);
    let high = max.and_then(kernel).is_none_or(|m| first <= m);
    low && high
}

fn physical_cores(profile: &HardwareProfile) -> u32 {
    if profile.cpu.platform == CpuPlatform::AmdBulldozer {
        profile.cpu.cores.max(profile.cpu.threads)
    } else {
        profile.cpu.cores
    }
}

pub fn check(run: &FixtureRun, t: &TargetRun, plan: &BuildPlan, out: &mut Failures) {
    let name = run.name.as_str();
    let target = t.target;
    let mut fail = |check: &'static str, detail: String| {
        assert!(CHECKS.contains(&check), "unlisted invariant {check}");
        out.push(name, Some(target), check, detail)
    };
    let profile = &run.profile;
    let opts = options(target);
    let mut ctx = PlanContext::new(profile, &opts);
    let display = graphics::choose_display(&ctx).ok();
    ctx.display = display.clone();

    // One GPU kext, the one the graphics stage picks for this display plan.
    let gpu_kexts: Vec<&str> = enabled(plan)
        .filter(|k| GPU_KEXTS.contains(&k.catalog_id.as_str()))
        .map(|k| k.catalog_id.as_str())
        .collect();
    // A guest with only an emulated display has no GPU path for a GPU kext.
    let vm_without_gpu = ctx.is_vm
        && display
            .as_ref()
            .and_then(|d| d.primary)
            .and_then(|i| profile.gpus.get(i))
            .is_none_or(|g| g.family == GpuFamily::VirtualDisplay);
    let expected = display
        .as_ref()
        .and_then(|d| graphics::gpu_kext(&ctx, d).catalog())
        .map(|(id, _)| id);
    if gpu_kexts.len() > 1 || (gpu_kexts.is_empty() && !vm_without_gpu) {
        fail(
            "gpu-kext",
            format!("GPU kexts {gpu_kexts:?}, expected exactly one"),
        );
    } else if let (Some(found), Some(expected)) = (gpu_kexts.first(), expected) {
        if *found != expected {
            fail(
                "gpu-kext",
                format!("{found} selected, graphics stage expects {expected}"),
            );
        }
    }

    for (id, bundle) in [("Lilu", "Lilu.kext"), ("VirtualSMC", "VirtualSMC.kext")] {
        if !has_kext(plan, id, bundle) {
            fail("core-kexts", format!("{bundle} is missing"));
        }
    }

    let voodoo_input = plan
        .kexts
        .iter()
        .filter(|k| k.enabled)
        .map(|k| {
            let own = usize::from(k.bundle.eq_ignore_ascii_case("VoodooInput.kext"));
            own + k
                .plugins
                .iter()
                .filter(|p| p.enabled && p.bundle.eq_ignore_ascii_case("VoodooInput.kext"))
                .count()
        })
        .sum::<usize>();
    if voodoo_input > 1 {
        fail(
            "voodooinput",
            format!("{voodoo_input} enabled VoodooInput copies"),
        );
    }

    // SMBIOS.
    let model = plan.smbios.model.as_str();
    let skip = plan.smbios.board_id_skip;
    if smbios_db::find(model).is_none() {
        fail("smbios-support", format!("{model} is not in smbios_db"));
    } else if !smbios_db::supports(model, target) && !skip {
        fail(
            "smbios-support",
            format!(
                "{model} does not support {} and board_id_skip is off",
                target.id()
            ),
        );
    }
    if target == MacOsVersion::Tahoe && !skip && !TAHOE_MODELS.contains(&model) {
        fail(
            "tahoe-smbios",
            format!("{model} is not a Tahoe model and board_id_skip is off"),
        );
    }
    if skip && plan.booter_patches.iter().all(|p| !p.enabled) {
        fail(
            "board-id-skip",
            "board_id_skip is set but no Booter patch is enabled".into(),
        );
    }
    let sbm = secure_boot_model(plan);
    if target >= MacOsVersion::Sonoma && sbm != "Disabled" {
        fail(
            "secure-boot",
            format!("SecureBootModel {sbm} on {}", target.id()),
        );
    }
    if skip && sbm != "Disabled" {
        fail(
            "secure-boot",
            format!("SecureBootModel {sbm} with the board-id skip"),
        );
    }
    if sbm != plan.smbios.secure_boot_model && sbm != "Disabled" {
        fail(
            "secure-boot",
            format!(
                "Misc/Security says {sbm}, the SMBIOS plan says {}",
                plan.smbios.secure_boot_model
            ),
        );
    }
    let root_patch = display
        .as_ref()
        .map(|d| planner::root_patching_planned(&ctx, d, plan))
        .unwrap_or_default();
    if (root_patch.any() && plan.csr_active_config == 0)
        || (root_patch.secure_boot_disabled() && sbm != "Disabled")
    {
        fail(
            "root-patch",
            format!(
                "root patches planned with SecureBootModel {sbm} and csr-active-config {:#010x}",
                plan.csr_active_config
            ),
        );
    }

    // CPU. A guest that sees the AMD host CPU needs the same kernel patches.
    let amd = profile.cpu.vendor == CpuVendor::Amd;
    if amd {
        let cores = physical_cores(profile);
        if plan.amd_core_count != Some(cores) {
            fail(
                "amd-core-count",
                format!("amd_core_count {:?}, expected {cores}", plan.amd_core_count),
            );
        }
    } else if plan.amd_core_count.is_some() {
        fail(
            "amd-core-count",
            format!("amd_core_count {:?} on a non-AMD CPU", plan.amd_core_count),
        );
    }
    let identity = planner::cpu_identity(&profile.cpu);
    if !ctx.is_vm
        && cpu_db::needs_cryptexfixup(&identity, target)
        && !has_kext(plan, "CryptexFixup", "CryptexFixup.kext")
    {
        fail(
            "cryptexfixup",
            "the CPU has no AVX2 but CryptexFixup is missing".into(),
        );
    }
    if !ctx.is_vm && SPOOFED_PLATFORMS.contains(&profile.cpu.platform) {
        let spoofed = matches!(
            plan.kernel_emulate.get("Cpuid1Data"),
            Some(PlistScalar::Data(hex)) if hex.chars().any(|c| c != '0')
        );
        if !spoofed {
            fail(
                "cpuid-spoof",
                "no Cpuid1Data spoof for a CPU macOS does not know".into(),
            );
        }
    }

    // Kexts.
    let mut repeated = HashSet::new();
    for k in enabled(plan) {
        let copies = enabled(plan)
            .filter(|o| o.bundle.eq_ignore_ascii_case(&k.bundle))
            .filter(|o| loads_on(o.min_kernel.as_deref(), o.max_kernel.as_deref(), target))
            .count();
        if copies > 1 && repeated.insert(k.bundle.to_ascii_lowercase()) {
            fail(
                "kext-duplicate",
                format!("{} is enabled {copies} times for {}", k.bundle, target.id()),
            );
        }
    }
    for k in &plan.kexts {
        match kext_catalog::entry(&k.catalog_id) {
            None => fail(
                "kext-catalog",
                format!("{} is not in the catalog", k.catalog_id),
            ),
            Some(entry) if !entry.provides(&k.bundle) => fail(
                "kext-catalog",
                format!("{} does not provide {}", k.catalog_id, k.bundle),
            ),
            Some(_) => {}
        }
        if let Err(e) = range_ok(k.min_kernel.as_deref(), k.max_kernel.as_deref()) {
            fail("kext-range", format!("{}: {e}", k.bundle));
        }
        if k.enabled
            && k.required
            && !loads_on(k.min_kernel.as_deref(), k.max_kernel.as_deref(), target)
        {
            fail(
                "kext-range",
                format!(
                    "required {} does not load on {} ({:?}..{:?})",
                    k.bundle,
                    target.id(),
                    k.min_kernel,
                    k.max_kernel
                ),
            );
        }
        for p in &k.plugins {
            if let Err(e) = range_ok(p.min_kernel.as_deref(), p.max_kernel.as_deref()) {
                fail("kext-range", format!("{}/{}: {e}", k.bundle, p.bundle));
            }
        }
    }
    if enabled(plan).any(|k| k.catalog_id == "AppleALC") && profile.audio.is_some() {
        let alcid = plan.boot_args.iter().any(|a| a.starts_with("alcid="));
        let property = plan
            .device_properties
            .iter()
            .any(|e| e.properties.iter().any(|p| p.key == "layout-id"));
        if !alcid && !property {
            fail(
                "audio-layout",
                "AppleALC without alcid= or a layout-id property".into(),
            );
        }
    }

    // An Intel iGPU kept enabled for macOS needs its framebuffer properties
    // (Iron Lake has a single framebuffer and no platform id).
    if let Some(d) = display.as_ref() {
        let igpu = d.igpu.and_then(|i| profile.gpus.get(i));
        if igpu
            .is_some_and(|g| g.vendor == GpuVendor::Intel && g.family != GpuFamily::IntelIronLake)
        {
            let props = plan
                .device_properties
                .iter()
                .find(|e| e.path == graphics::IGPU_PATH)
                .map(|e| {
                    e.properties
                        .iter()
                        .map(|p| p.key.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !props
                .iter()
                .any(|k| *k == "AAPL,ig-platform-id" || *k == "AAPL,snb-platform-id")
            {
                fail(
                    "igpu-properties",
                    format!("Intel iGPU kept without a platform id ({props:?})"),
                );
            }
        }
    }

    // EFI files.
    let mut names = HashSet::new();
    for s in &plan.ssdts {
        if !s.file_name.ends_with(".aml") || !names.insert(s.file_name.to_ascii_lowercase()) {
            fail(
                "ssdt-source",
                format!("bad or repeated SSDT file name {}", s.file_name),
            );
        }
        match &s.source {
            SsdtSource::OcSample { file } if !config::OC_ACPI_SAMPLES.contains(&file.as_str()) => {
                fail(
                    "ssdt-source",
                    format!("{} is not an OpenCore 1.0.8 sample", file),
                )
            }
            SsdtSource::Dortania { file } if kext_catalog::dortania_ssdt(file).is_none() => fail(
                "ssdt-source",
                format!("{file} has no pinned Dortania download"),
            ),
            SsdtSource::Generated { aml_hex, .. } => {
                if let Err(e) = aml_header(aml_hex) {
                    fail("ssdt-source", format!("{}: {e}", s.file_name));
                }
            }
            _ => {}
        }
    }
    for d in &plan.drivers {
        if !driver_known(d) {
            fail(
                "efi-files",
                format!(
                    "driver {} ({}) is not in its source package",
                    d.path, d.source
                ),
            );
        }
    }
    if !plan
        .drivers
        .iter()
        .any(|d| d.enabled && d.path == "OpenRuntime.efi")
    {
        fail("efi-files", "OpenRuntime.efi is not loaded".into());
    }
    for tool in &plan.tools {
        if !config::OC_TOOLS.contains(&tool.as_str()) {
            fail("efi-files", format!("tool {tool} is not in OpenCore 1.0.8"));
        }
    }

    let mut keys = HashSet::new();
    for arg in plan.boot_args.iter().flat_map(|a| a.split_whitespace()) {
        let key = arg.split('=').next().unwrap_or(arg);
        if !keys.insert(key.to_string()) {
            fail("boot-args", format!("boot-arg {key} appears twice"));
        }
        let restrict_events = ["revpatch", "revcpu", "revcpuname", "revblock"].contains(&key);
        if restrict_events && !enabled(plan).any(|k| k.catalog_id == "RestrictEvents") {
            fail("boot-args", format!("{arg} without RestrictEvents"));
        }
    }

    release_rules(plan, target, ctx.is_vm, &sbm, &mut fail);
    plan_hygiene(plan, &mut fail);
}

/// Settings a release needs whatever the hardware (OpenCore docs, Dortania).
fn release_rules(
    plan: &BuildPlan,
    target: MacOsVersion,
    is_vm: bool,
    sbm: &str,
    fail: &mut impl FnMut(&'static str, String),
) {
    let model = plan.smbios.model.as_str();
    // Before Big Sur, Apple Secure Boot needs a T2 model at or after its
    // first release; x86legacy (every other model) starts with macOS 11.
    if target < MacOsVersion::BigSur && sbm != "Disabled" {
        let t2 = smbios_db::find(model)
            .filter(|m| m.secure_boot_model.is_some())
            .is_some_and(|m| m.min_os.is_none_or(|min| min <= target));
        if !t2 {
            fail(
                "secure-boot",
                format!(
                    "SecureBootModel {sbm} on {} with {model}, which is no T2 model there",
                    target.id()
                ),
            );
        }
    }
    // The default MinDate/MinVersion (0) only load the APFS driver of Big Sur
    // and newer.
    if target < MacOsVersion::BigSur {
        let lowered =
            |key: &str| matches!(plan.uefi_apfs.get(key), Some(PlistScalar::Int(v)) if *v != 0);
        if !lowered("MinDate") || !lowered("MinVersion") {
            fail(
                "apfs-min",
                "UEFI/APFS MinDate/MinVersion keep the Big Sur default".into(),
            );
        }
    }
    // XhciPortLimit breaks USB from macOS 11.3 until OpenCore 1.0.7's Tahoe
    // rework; those releases need a USB map instead.
    let xhci = matches!(
        plan.kernel_quirks.get("XhciPortLimit"),
        Some(PlistScalar::Bool(true))
    );
    if xhci && (MacOsVersion::BigSur..=MacOsVersion::Sequoia).contains(&target) {
        fail(
            "xhci-port-limit",
            format!("XhciPortLimit is on for {}", target.id()),
        );
    }
    if !is_vm
        && USBX_MODELS.iter().any(|p| model.starts_with(p))
        && !plan
            .ssdts
            .iter()
            .any(|s| s.file_name.to_ascii_uppercase().contains("USBX"))
    {
        fail("usbx", format!("{model} without a USBX device"));
    }
    // AirportItlwm loads through Apple Secure Boot, except on the legacy
    // wireless stack (root-patched, SecureBootModel Disabled).
    let legacy_stack =
        enabled(plan).any(|k| k.bundle.eq_ignore_ascii_case("IO80211FamilyLegacy.kext"));
    if sbm == "Disabled"
        && !legacy_stack
        && enabled(plan).any(|k| k.bundle.eq_ignore_ascii_case("AirportItlwm.kext"))
    {
        fail("wifi", "AirportItlwm with SecureBootModel Disabled".into());
    }
}

/// Shape of what config.plist and the notes receive.
fn plan_hygiene(plan: &BuildPlan, fail: &mut impl FnMut(&'static str, String)) {
    let mut paths = HashSet::new();
    for e in &plan.device_properties {
        if !paths.insert(e.path.as_str()) {
            fail("device-properties", format!("{} appears twice", e.path));
        }
        if e.properties.is_empty() {
            fail("device-properties", format!("{} has no properties", e.path));
        }
        let mut keys = HashSet::new();
        for p in &e.properties {
            if !keys.insert(p.key.as_str()) {
                fail(
                    "device-properties",
                    format!("{} sets {} twice", e.path, p.key),
                );
            }
        }
    }
    // Renames written for a generated table ("... - requires SSDT-XOSI.aml").
    for p in plan.acpi_patches.iter().filter(|p| p.enabled) {
        if let Some((_, table)) = p.comment.split_once("requires ") {
            let table = table.trim();
            if table.ends_with(".aml") && !plan.ssdts.iter().any(|s| s.file_name == table) {
                fail("acpi-patch", format!("'{}' without {table}", p.comment));
            }
        }
    }
    for n in plan.notes.iter().chain(&plan.post_install) {
        let mut seen = HashSet::new();
        let sentences = n
            .detail
            .split(". ")
            .map(|s| s.trim().trim_end_matches('.'))
            .filter(|s| s.len() > 30);
        for s in sentences {
            if !seen.insert(s) {
                fail("note-text", format!("'{}' repeats \"{s}\"", n.title));
                break;
            }
        }
    }
}

fn driver_known(d: &DriverPlan) -> bool {
    match d.source.as_str() {
        "opencore" => config::OC_DRIVERS.contains(&d.path.as_str()),
        "ocbinarydata" => config::OCBINARYDATA_DRIVERS.contains(&d.path.as_str()),
        _ => false,
    }
}

/// The AML must start with an "SSDT" header whose length matches the bytes.
fn aml_header(hex: &str) -> Result<(), String> {
    if !hex.len().is_multiple_of(2) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("AML is not hex".into());
    }
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    if bytes.len() < 36 || &bytes[..4] != b"SSDT" {
        return Err("AML has no SSDT header".into());
    }
    let length = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if length != bytes.len() {
        return Err(format!("header length {length}, {} bytes", bytes.len()));
    }
    let sum = bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    if sum != 0 {
        return Err("bad table checksum".into());
    }
    Ok(())
}
