//! Compact text snapshot of one fixture: the compatibility verdict and plan
//! summary for every release, then the full plan for the recommended one.
//! `GOLDEN_UPDATE=1` rewrites the files instead of comparing.

use std::fmt::Write as _;
use std::path::PathBuf;

use app_lib::domain::model::{
    BinaryPatch, BuildPlan, KextSelection, MacOsVersion, NoteLevel, PlanNote, PlistScalar,
    SettingMap, SsdtSource,
};

use super::invariants::secure_boot_model;
use super::support::{tests_dir, FixtureRun};

pub fn path(name: &str) -> PathBuf {
    tests_dir().join("snapshots").join(format!("{name}.txt"))
}

pub fn updating() -> bool {
    std::env::var("GOLDEN_UPDATE").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Compare `text` with the stored snapshot (or store it when updating).
pub fn compare(name: &str, text: &str) -> Result<(), String> {
    let path = path(name);
    if updating() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        return Ok(());
    }
    let Ok(stored) = std::fs::read_to_string(&path) else {
        return Err(format!(
            "{} is missing; run with GOLDEN_UPDATE=1",
            path.display()
        ));
    };
    let stored = stored.replace("\r\n", "\n");
    if stored == text {
        return Ok(());
    }
    let mut diff = String::new();
    let old: Vec<&str> = stored.lines().collect();
    let new: Vec<&str> = text.lines().collect();
    let mut shown = 0;
    for i in 0..old.len().max(new.len()) {
        let (a, b) = (old.get(i).copied(), new.get(i).copied());
        if a != b {
            let _ = writeln!(
                diff,
                "  line {}:\n    - {}\n    + {}",
                i + 1,
                a.unwrap_or("<end>"),
                b.unwrap_or("<end>")
            );
            shown += 1;
            if shown == 8 {
                diff.push_str("  ...\n");
                break;
            }
        }
    }
    Err(format!(
        "{} differs (GOLDEN_UPDATE=1 rewrites it):\n{diff}",
        path.display()
    ))
}

fn scalar(v: &PlistScalar) -> String {
    match v {
        PlistScalar::Bool(b) => b.to_string(),
        PlistScalar::Int(i) => i.to_string(),
        PlistScalar::Str(s) => format!("\"{s}\""),
        PlistScalar::Data(d) => format!("<{d}>"),
    }
}

fn settings(out: &mut String, label: &str, map: &SettingMap) {
    if map.is_empty() {
        return;
    }
    let on: Vec<&str> = map
        .iter()
        .filter(|(_, v)| **v == PlistScalar::Bool(true))
        .map(|(k, _)| k.as_str())
        .collect();
    let off: Vec<&str> = map
        .iter()
        .filter(|(_, v)| **v == PlistScalar::Bool(false))
        .map(|(k, _)| k.as_str())
        .collect();
    let values: Vec<String> = map
        .iter()
        .filter(|(_, v)| !matches!(v, PlistScalar::Bool(_)))
        .map(|(k, v)| format!("{k}={}", scalar(v)))
        .collect();
    let _ = writeln!(out, "{label}:");
    if !on.is_empty() {
        let _ = writeln!(out, "  on: {}", on.join(" "));
    }
    if !off.is_empty() {
        let _ = writeln!(out, "  off: {}", off.join(" "));
    }
    if !values.is_empty() {
        let _ = writeln!(out, "  set: {}", values.join(" "));
    }
}

fn range(min: Option<&str>, max: Option<&str>) -> String {
    let (min, max) = (min.unwrap_or(""), max.unwrap_or(""));
    if min.is_empty() && max.is_empty() {
        String::new()
    } else {
        format!(" [{}..{}]", min, max)
    }
}

fn mark(enabled: bool) -> char {
    if enabled {
        '+'
    } else {
        '-'
    }
}

fn kext(out: &mut String, k: &KextSelection) {
    let required = if k.required { " required" } else { "" };
    let _ = writeln!(
        out,
        "  {} {} ({}){}{}",
        mark(k.enabled),
        k.bundle,
        k.catalog_id,
        range(k.min_kernel.as_deref(), k.max_kernel.as_deref()),
        required
    );
    for p in &k.plugins {
        let _ = writeln!(
            out,
            "      {} {}{}",
            mark(p.enabled),
            p.bundle,
            range(p.min_kernel.as_deref(), p.max_kernel.as_deref())
        );
    }
}

fn patch(out: &mut String, p: &BinaryPatch) {
    let _ = writeln!(
        out,
        "  {} {} ({}){}",
        mark(p.enabled),
        p.comment,
        p.identifier,
        range(Some(&p.min_kernel), Some(&p.max_kernel))
    );
}

fn level(n: &PlanNote) -> &'static str {
    match n.level {
        NoteLevel::Info => "info",
        NoteLevel::Warning => "warning",
        NoteLevel::Blocking => "blocking",
    }
}

fn gpu_kext(plan: &BuildPlan) -> &str {
    plan.kexts
        .iter()
        .filter(|k| k.enabled)
        .map(|k| k.catalog_id.as_str())
        .find(|id| matches!(*id, "WhateverGreen" | "NootRX" | "NootedRed"))
        .unwrap_or("none")
}

fn header(out: &mut String, run: &FixtureRun) {
    let p = &run.profile;
    let c = &p.cpu;
    let _ = writeln!(out, "fixture: {}", run.name);
    let _ = writeln!(
        out,
        "cpu: {} | {:?} ({}) | {}C/{}T | avx2 {}",
        c.name.split_whitespace().collect::<Vec<_>>().join(" "),
        c.platform,
        c.codename,
        c.cores,
        c.threads,
        c.has_avx2.map_or("?".to_string(), |v| v.to_string())
    );
    let _ = writeln!(
        out,
        "machine: {:?}{} | {} {} | chipset {} | uefi {}",
        p.form_factor,
        p.vm.map(|v| format!(" vm {v:?}")).unwrap_or_default(),
        p.motherboard_vendor,
        p.motherboard_model,
        p.chipset.as_deref().unwrap_or("-"),
        p.firmware_uefi.map_or("?".to_string(), |v| v.to_string())
    );
    for g in &p.gpus {
        let _ = writeln!(
            out,
            "gpu: {} [{}:{} {:?}{}]",
            g.name,
            g.vendor_id.as_deref().unwrap_or("-"),
            g.device_id.as_deref().unwrap_or("-"),
            g.family,
            if g.is_igpu { " igpu" } else { "" }
        );
    }
    if let Some(a) = &p.audio {
        let _ = writeln!(
            out,
            "audio: {} [{:08x}]",
            a.codec_name,
            a.codec_id.unwrap_or(0)
        );
    }
    for n in p.ethernet.iter().chain(&p.wifi).chain(&p.bluetooth) {
        let _ = writeln!(
            out,
            "net: {} [{}:{} {:?}]",
            n.name,
            n.vendor_id.as_deref().unwrap_or("-"),
            n.device_id.as_deref().unwrap_or("-"),
            n.bus
        );
    }
    let i = &p.input;
    if let Some(bus) = i.touchpad_bus {
        let _ = writeln!(
            out,
            "touchpad: {:?} {:?} {}",
            bus,
            i.touchpad_vendor,
            i.touchpad_hid.as_deref().unwrap_or("-")
        );
    }
}

fn versions(out: &mut String, run: &FixtureRun) {
    let o = &run.overview;
    let _ = writeln!(
        out,
        "assessment: {:?}, recommended {}",
        o.level,
        o.recommended.map_or("none", MacOsVersion::id)
    );
    let _ = writeln!(out, "summary: {}", o.summary);
    let _ = writeln!(out, "releases:");
    for t in &run.targets {
        let root = if t
            .report
            .versions
            .iter()
            .any(|v| v.version == t.target && v.needs_root_patch)
        {
            " root-patch"
        } else {
            ""
        };
        let result = match &t.plan {
            Ok(plan) => format!(
                "ok {}{} sbm={} csr={:08x} gpu={} kexts={} ssdts={}",
                plan.smbios.model,
                if plan.smbios.board_id_skip {
                    " (skip)"
                } else {
                    ""
                },
                secure_boot_model(plan),
                plan.csr_active_config,
                gpu_kext(plan),
                plan.kexts.iter().filter(|k| k.enabled).count(),
                plan.ssdts.len()
            ),
            Err(e) => format!(
                "err {}{}",
                e.code,
                if e.recoverable { " (recoverable)" } else { "" }
            ),
        };
        let _ = writeln!(
            out,
            "  {:<5} {:<11}{} | {}",
            t.target.id(),
            t.verdict.label(),
            root,
            result
        );
    }
}

fn plan_details(out: &mut String, plan: &BuildPlan) {
    let s = &plan.smbios;
    let _ = writeln!(
        out,
        "smbios: {}{} | alternatives: {}",
        s.model,
        if s.board_id_skip {
            " (board-id skip)"
        } else {
            ""
        },
        if s.alternatives.is_empty() {
            "-".to_string()
        } else {
            s.alternatives.join(" ")
        }
    );
    let sbm = secure_boot_model(plan);
    if sbm == s.secure_boot_model {
        let _ = writeln!(out, "secure-boot-model: {sbm}");
    } else {
        let _ = writeln!(
            out,
            "secure-boot-model: {sbm} (smbios stage: {})",
            s.secure_boot_model
        );
    }
    let _ = writeln!(out, "csr-active-config: {:08x}", plan.csr_active_config);
    let _ = writeln!(out, "boot-args: {}", plan.boot_args.join(" "));
    if let Some(cores) = plan.amd_core_count {
        let _ = writeln!(out, "amd-core-count: {cores}");
    }
    let _ = writeln!(out, "kexts:");
    for k in &plan.kexts {
        kext(out, k);
    }
    if !plan.ssdts.is_empty() {
        let _ = writeln!(out, "ssdts:");
        for t in &plan.ssdts {
            let source = match &t.source {
                SsdtSource::Generated { aml_hex, .. } => {
                    format!("machine ({} bytes)", aml_hex.len() / 2)
                }
                SsdtSource::OcSample { file } => format!("opencore {file}"),
                SsdtSource::Dortania { file } => format!("dortania {file}"),
            };
            let _ = writeln!(
                out,
                "  {} <- {}{}",
                t.file_name,
                source,
                if t.required { "" } else { " (optional)" }
            );
        }
    }
    if !plan.acpi_patches.is_empty() {
        let _ = writeln!(out, "acpi-patches:");
        for p in &plan.acpi_patches {
            let _ = writeln!(
                out,
                "  {} {} [{} -> {}{}]",
                mark(p.enabled),
                p.comment,
                p.find,
                p.replace,
                p.table_signature
                    .as_deref()
                    .map(|t| format!(" in {t}"))
                    .unwrap_or_default()
            );
        }
    }
    for d in &plan.acpi_deletes {
        let _ = writeln!(
            out,
            "acpi-delete: {} {} {}",
            d.table_signature,
            d.oem_table_id.trim(),
            d.comment
        );
    }
    if !plan.device_properties.is_empty() {
        let _ = writeln!(out, "device-properties:");
        for e in &plan.device_properties {
            let _ = writeln!(out, "  {}", e.path);
            for p in &e.properties {
                let _ = writeln!(out, "    {} = {}", p.key, scalar(&p.value));
            }
        }
    }
    settings(out, "acpi-quirks", &plan.acpi_quirks);
    settings(out, "booter-quirks", &plan.booter_quirks);
    for m in &plan.mmio_whitelist {
        let _ = writeln!(
            out,
            "mmio: {} {:#x} {}",
            mark(m.enabled),
            m.address,
            m.comment
        );
    }
    if !plan.booter_patches.is_empty() {
        let _ = writeln!(out, "booter-patches:");
        for p in &plan.booter_patches {
            patch(out, p);
        }
    }
    settings(out, "kernel-quirks", &plan.kernel_quirks);
    settings(out, "kernel-emulate", &plan.kernel_emulate);
    if !plan.kernel_patches.is_empty() {
        let _ = writeln!(out, "kernel-patches:");
        for p in &plan.kernel_patches {
            patch(out, p);
        }
    }
    for b in &plan.kernel_blocks {
        let _ = writeln!(
            out,
            "kernel-block: {} {} {}{}",
            mark(b.enabled),
            b.identifier,
            b.strategy,
            range(Some(&b.min_kernel), Some(&b.max_kernel))
        );
    }
    settings(out, "misc-boot", &plan.misc_boot);
    settings(out, "misc-security", &plan.misc_security);
    let _ = writeln!(out, "tools: {}", plan.tools.join(" "));
    for v in &plan.nvram_add {
        let _ = writeln!(
            out,
            "nvram-add: {}:{} = {}",
            v.guid,
            v.key,
            scalar(&v.value)
        );
    }
    for v in &plan.nvram_delete {
        let _ = writeln!(out, "nvram-delete: {}:{}", v.guid, v.key);
    }
    settings(out, "nvram", &plan.nvram_settings);
    settings(out, "platform-info", &plan.platform_info);
    let drivers: Vec<String> = plan
        .drivers
        .iter()
        .map(|d| {
            format!(
                "{}{}{}",
                if d.enabled { "" } else { "-" },
                d.path,
                if d.load_early { "(early)" } else { "" }
            )
        })
        .collect();
    let _ = writeln!(out, "drivers: {}", drivers.join(" "));
    settings(out, "uefi-quirks", &plan.uefi_quirks);
    settings(out, "uefi-apfs", &plan.uefi_apfs);
    settings(out, "uefi-output", &plan.uefi_output);
    settings(out, "uefi-input", &plan.uefi_input);
    for b in &plan.bios_settings {
        let _ = writeln!(
            out,
            "bios: {} = {}{}",
            b.name,
            b.value,
            if b.required { " (required)" } else { "" }
        );
    }
    if !plan.notes.is_empty() {
        let _ = writeln!(out, "notes:");
        for n in &plan.notes {
            let _ = writeln!(out, "  [{}] {}: {}", level(n), n.component, n.title);
        }
    }
    if !plan.post_install.is_empty() {
        let _ = writeln!(out, "post-install:");
        for n in &plan.post_install {
            let _ = writeln!(out, "  [{}] {}: {}", level(n), n.component, n.title);
        }
    }
}

pub fn render(run: &FixtureRun) -> String {
    let mut out = String::new();
    header(&mut out, run);
    versions(&mut out, run);
    let notes: Vec<&PlanNote> = run
        .overview
        .notes
        .iter()
        .filter(|n| n.level != NoteLevel::Info)
        .collect();
    if !notes.is_empty() {
        let _ = writeln!(out, "report-notes:");
        for n in notes {
            let _ = writeln!(out, "  [{}] {}: {}", level(n), n.component, n.title);
        }
    }
    match run.overview.recommended {
        Some(rec) => {
            let _ = writeln!(out, "\n== plan for {} (recommended) ==", rec.display_name());
            match &run.target(rec).plan {
                Ok(plan) => plan_details(&mut out, plan),
                Err(e) => {
                    let _ = writeln!(out, "error: {e}");
                }
            }
        }
        None => {
            let _ = writeln!(out, "\n== no recommended release ==");
            for t in &run.targets {
                if let Err(e) = &t.plan {
                    let _ = writeln!(out, "{}: {e}", t.target.id());
                }
            }
        }
    }
    out
}
