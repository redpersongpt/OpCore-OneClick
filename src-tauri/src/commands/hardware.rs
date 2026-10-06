//! Hardware scan and profile commands.
//!
//! Profiles are exported as a versioned JSON envelope
//! (`{"format": "opcore-oneclick-profile", "version": 1, "profile": ...}`) so
//! an EFI for this machine can be built on another computer. The envelope
//! also carries the machine's DSDT/SSDT dumps (base64, no other ACPI tables:
//! MSDM would leak the Windows product key), so path-correct SSDTs can still
//! be generated there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::build::staging::{new_build_id, write_atomic};
use crate::build::{blocking, is_plain_file_name, retention};
use crate::contracts::{Catalog, CatalogOption, ScanResult};
use crate::domain::model::{FormFactor, HardwareProfile, MacOsVersion};
use crate::domain::{cpu_db, gpu_db, kext_catalog, profile as profile_db, smbios_db};
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::services::ocvalidate::check_aml;
use crate::tasks::cancellation::CancellationToken;
use crate::tasks::registry::TaskRegistry;

pub const PROFILE_FORMAT: &str = "opcore-oneclick-profile";
pub const PROFILE_FORMAT_VERSION: u32 = 1;
/// Largest profile file accepted or written.
const MAX_PROFILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ACPI_TABLES: usize = 64;
const MAX_ACPI_TABLE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ACPI_TOTAL_BYTES: usize = 16 * 1024 * 1024;
/// Per-scan and per-import ACPI dump folders inside `paths.acpi`.
const SCAN_PREFIX: &str = "scan-";
const IMPORT_PREFIX: &str = "import-";
const KEEP_SCANS: usize = 2;
const KEEP_IMPORTS: usize = 3;

/// Scan this machine (task kind "hardware-scan"), dump ACPI tables into a
/// fresh folder under `paths.acpi`, and interpret the result into a profile.
#[tauri::command]
pub async fn scan_hardware(
    task_registry: State<'_, Arc<TaskRegistry>>,
    paths: State<'_, AppPaths>,
) -> Result<ScanResult, AppError> {
    let registry: Arc<TaskRegistry> = Arc::clone(&task_registry);
    let (task_id, cancel) = registry.create("hardware-scan").await;
    let result = crate::build::guarded(run_scan(&registry, &task_id, &paths, &cancel)).await;
    match &result {
        Ok(_) => registry.complete(&task_id).await,
        Err(e) if e.code == "TASK_CANCELLED" => {}
        Err(e) => {
            tracing::warn!(code = %e.code, "hardware scan failed: {}", e.message);
            registry.fail(&task_id, &e.message).await;
        }
    }
    result
}

async fn run_scan(
    registry: &Arc<TaskRegistry>,
    task_id: &str,
    paths: &AppPaths,
    cancel: &CancellationToken,
) -> Result<ScanResult, AppError> {
    registry.update_progress(task_id, 0.02, Some("Reading the hardware".into())).await;
    let acpi_dir = paths.acpi.join(format!("{SCAN_PREFIX}{}", new_build_id()));
    std::fs::create_dir_all(&acpi_dir)?;

    // The scanners report no progress of their own; keep the bar moving
    // for a while (the watchdog still catches a scan that hangs for good).
    let heartbeat = {
        let registry = Arc::clone(registry);
        let task_id = task_id.to_string();
        tauri::async_runtime::spawn(async move {
            for tick in 1..=30 {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let progress = 0.05 + 0.75 * (1.0 - 0.9f64.powi(tick));
                registry.update_progress(&task_id, progress, Some("Reading the hardware".into())).await;
            }
        })
    };
    let scanned = crate::platform::scan(&acpi_dir, cancel).await;
    heartbeat.abort();

    let detected = match scanned.and_then(|d| cancel.check().map(|_| d)) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&acpi_dir);
            return Err(e);
        }
    };
    if detected.acpi_tables_dir.is_none() {
        let _ = std::fs::remove_dir_all(&acpi_dir);
    }
    registry.update_progress(task_id, 0.85, Some("Interpreting the scan".into())).await;
    let raw = detected.clone();
    let profile = blocking(move || Ok(profile_db::build_profile(&raw))).await?;
    retention::prune_prefixed(&paths.acpi, SCAN_PREFIX, KEEP_SCANS, Some(&acpi_dir));
    tracing::info!(
        cpu = %profile.cpu.name,
        platform = ?profile.cpu.platform,
        gpus = profile.gpus.len(),
        acpi = detected.acpi_tables_dir.is_some(),
        "hardware scanned"
    );
    Ok(ScanResult { detected, profile })
}

/// Re-interpret a manually edited profile.
#[tauri::command]
pub async fn refresh_profile(profile: HardwareProfile) -> Result<HardwareProfile, AppError> {
    blocking(move || Ok(profile_db::refresh_profile(profile))).await
}

/// Options for the manual profile editor and version picker.
#[tauri::command]
pub async fn get_catalog() -> Result<Catalog, AppError> {
    Ok(catalog())
}

pub fn catalog() -> Catalog {
    let macos_versions = MacOsVersion::newest_first()
        .map(|v| CatalogOption {
            id: v.id().to_string(),
            label: v.display_name(),
            detail: Some(format!("Darwin {}", v.darwin_major())),
        })
        .collect();

    let cpu_platforms = cpu_db::all_platforms()
        .iter()
        .filter_map(|&platform| {
            let info = cpu_db::platform_info(platform);
            let detail = if !info.supported {
                "Not supported by macOS".to_string()
            } else {
                let min = info.min_macos.unwrap_or(MacOsVersion::HighSierra);
                let max = info.max_macos.unwrap_or(MacOsVersion::Tahoe);
                let mut detail = format!("macOS {}", version_range(min, max));
                // The opt-in path past the native ceiling (CryptexFixup, telemetrap).
                if let Some(w) = cpu_db::ceiling_workaround(platform) {
                    let beyond = version_range(w.from, w.max_macos.unwrap_or(MacOsVersion::Tahoe));
                    match w.kext {
                        Some(kext) => detail.push_str(&format!("; {beyond} with {kext}")),
                        None => detail.push_str(&format!("; {beyond} untested")),
                    }
                }
                detail
            };
            let label = if info.label.is_empty() { serde_id(&platform)? } else { info.label.to_string() };
            Some(CatalogOption { id: serde_id(&platform)?, label, detail: Some(detail) })
        })
        .collect();

    let gpu_families = gpu_db::all_families()
        .iter()
        .filter_map(|(family, label)| {
            Some(CatalogOption { id: serde_id(family)?, label: label.to_string(), detail: None })
        })
        .collect();

    let smbios_models = smbios_db::all()
        .iter()
        .map(|m| CatalogOption {
            id: m.model.to_string(),
            label: m.model.to_string(),
            detail: Some(format!("{}; macOS {} to {}", m.description, m.min_release, m.max_release)),
        })
        .collect();

    let form_factors = [
        (FormFactor::Desktop, "Desktop", "Tower or small desktop with its own monitor"),
        (FormFactor::Laptop, "Laptop", "Notebook with an internal panel and a battery"),
        (FormFactor::AllInOne, "All-in-one", "Desktop board with an internal panel"),
        (FormFactor::MiniPc, "Mini PC / NUC", "Mobile CPU without an internal panel or battery"),
    ]
    .iter()
    .filter_map(|(ff, label, detail)| {
        Some(CatalogOption { id: serde_id(ff)?, label: (*label).to_string(), detail: Some((*detail).to_string()) })
    })
    .collect();

    Catalog {
        macos_versions,
        cpu_platforms,
        gpu_families,
        smbios_models,
        form_factors,
        opencore_version: kext_catalog::opencore_release().version.to_string(),
    }
}

/// "Monterey only" or "High Sierra to Monterey".
fn version_range(min: MacOsVersion, max: MacOsVersion) -> String {
    if min == max {
        format!("{} only", min.marketing_name())
    } else {
        format!("{} to {}", min.marketing_name(), max.marketing_name())
    }
}

/// The IPC spelling of an enum value ("coffee_lake").
fn serde_id<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value).ok()?.as_str().map(str::to_string)
}

/// Save a profile as JSON (to build an EFI for this machine on another computer).
#[tauri::command]
pub async fn export_profile(profile: HardwareProfile, path: String) -> Result<(), AppError> {
    blocking(move || {
        let dest = target_path(&path)?;
        let bytes = encode_profile(&profile)?;
        write_atomic(&dest, &bytes)?;
        tracing::info!(path = %dest.display(), bytes = bytes.len(), "profile exported");
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn import_profile(path: String, paths: State<'_, AppPaths>) -> Result<HardwareProfile, AppError> {
    let acpi_root = paths.acpi.clone();
    blocking(move || import_from(Path::new(path.trim()), &acpi_root)).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileEnvelope {
    format: String,
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exported_at: Option<String>,
    profile: HardwareProfile,
    /// DSDT/SSDT dumps: file name → base64.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    acpi_tables: BTreeMap<String, String>,
}

/// A decoded profile file.
#[derive(Debug, Clone)]
pub struct ImportedProfile {
    pub profile: HardwareProfile,
    /// Valid DSDT/SSDT tables from the file: (file name, bytes).
    pub acpi_tables: Vec<(String, Vec<u8>)>,
}

/// Serialise `profile` into an envelope, with its ACPI tables when the dump
/// folder is readable. The local dump path itself is not exported.
pub fn encode_profile(profile: &HardwareProfile) -> Result<Vec<u8>, AppError> {
    validate_profile(profile)?;
    let tables = profile.acpi_tables_dir.as_deref().map(|d| read_tables(Path::new(d))).unwrap_or_default();
    let engine = base64::engine::general_purpose::STANDARD;
    let mut exported = profile.clone();
    exported.acpi_tables_dir = None;
    let envelope = ProfileEnvelope {
        format: PROFILE_FORMAT.into(),
        version: PROFILE_FORMAT_VERSION,
        app_version: Some(crate::APP_VERSION.into()),
        exported_at: Some(chrono::Utc::now().to_rfc3339()),
        profile: exported,
        acpi_tables: tables.into_iter().map(|(name, bytes)| (name, engine.encode(bytes))).collect(),
    };
    let bytes = serde_json::to_vec_pretty(&envelope)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(AppError::new("PROFILE_TOO_LARGE", "The profile is too large to export"));
    }
    Ok(bytes)
}

/// Parse and check a profile file. The profile comes back marked "imported"
/// and without a local ACPI dump path.
pub fn decode_profile(bytes: &[u8]) -> Result<ImportedProfile, AppError> {
    let invalid = |why: String| {
        AppError::new("PROFILE_INVALID", format!("This is not a valid OpCore-OneClick profile: {why}")).recoverable()
    };
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    let format = value.get("format").and_then(serde_json::Value::as_str);
    if format != Some(PROFILE_FORMAT) {
        return Err(AppError::new("PROFILE_FORMAT_UNKNOWN", "This file is not an OpCore-OneClick hardware profile")
            .recoverable()
            .with_suggestion("Export the profile again with \"Export profile\" in OpCore-OneClick."));
    }
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(PROFILE_FORMAT_VERSION) => {}
        Some(v) if v > u64::from(PROFILE_FORMAT_VERSION) => {
            return Err(AppError::new(
                "PROFILE_VERSION_UNSUPPORTED",
                format!("The profile was saved by a newer OpCore-OneClick (format {v})"),
            )
            .recoverable()
            .with_suggestion("Update OpCore-OneClick, then import the profile again."));
        }
        _ => return Err(invalid("missing or unknown format version".into())),
    }
    let envelope: ProfileEnvelope = serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
    let mut profile = envelope.profile;
    validate_profile(&profile)?;
    profile.source = "imported".into();
    profile.acpi_tables_dir = None;
    if !profile.scan_confidence.is_finite() {
        profile.scan_confidence = 0.0;
    }
    profile.scan_confidence = profile.scan_confidence.clamp(0.0, 1.0);

    let engine = base64::engine::general_purpose::STANDARD;
    let mut tables = Vec::new();
    let mut total = 0usize;
    for (name, data) in envelope.acpi_tables.iter().take(MAX_ACPI_TABLES) {
        let Ok(bytes) = engine.decode(data.as_bytes()) else {
            tracing::warn!(table = %name, "ACPI table in the profile is not base64, skipped");
            continue;
        };
        if !table_ok(name, &bytes) || total + bytes.len() > MAX_ACPI_TOTAL_BYTES {
            tracing::warn!(table = %name, "ACPI table in the profile is not usable, skipped");
            continue;
        }
        total += bytes.len();
        tables.push((name.clone(), bytes));
    }
    // SSDTs are generated from the DSDT; without it the rest is useless.
    if !tables.iter().any(|(_, b)| b.starts_with(b"DSDT")) {
        tables.clear();
    }
    Ok(ImportedProfile { profile, acpi_tables: tables })
}

fn import_from(path: &Path, acpi_root: &Path) -> Result<HardwareProfile, AppError> {
    let meta = std::fs::metadata(path)
        .map_err(|_| AppError::new("PROFILE_NOT_FOUND", format!("{} does not exist", path.display())).recoverable())?;
    if !meta.is_file() {
        return Err(AppError::new("PROFILE_NOT_FOUND", format!("{} is not a file", path.display())).recoverable());
    }
    if meta.len() > MAX_PROFILE_BYTES {
        return Err(AppError::new("PROFILE_TOO_LARGE", "The file is too large to be a hardware profile").recoverable());
    }
    let bytes = std::fs::read(path)?;
    let imported = decode_profile(&bytes)?;
    let mut profile = imported.profile;
    if !imported.acpi_tables.is_empty() {
        let dir = acpi_root.join(format!("{IMPORT_PREFIX}{}", new_build_id()));
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            imported.acpi_tables.iter().try_for_each(|(name, bytes)| std::fs::write(dir.join(name), bytes))
        });
        if let Err(e) = written {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(AppError::new("IO_ERROR", format!("Could not store the profile's ACPI tables: {e}")));
        }
        profile.acpi_tables_dir = Some(dir.to_string_lossy().into_owned());
        retention::prune_prefixed(acpi_root, IMPORT_PREFIX, KEEP_IMPORTS, Some(&dir));
    }
    tracing::info!(cpu = %profile.cpu.name, tables = imported.acpi_tables.len(), "profile imported");
    Ok(profile)
}

/// Export destination: an absolute file path (not a folder).
fn target_path(path: &str) -> Result<PathBuf, AppError> {
    let dest = PathBuf::from(path.trim());
    if dest.as_os_str().is_empty() || !dest.is_absolute() {
        return Err(AppError::new("PATH_INVALID", format!("'{path}' is not a full file path")).recoverable());
    }
    if dest.is_dir() {
        return Err(AppError::new("PATH_INVALID", format!("{} is a folder", dest.display())).recoverable());
    }
    Ok(dest)
}

/// DSDT and SSDT dumps in `dir` (other tables are never exported).
fn read_tables(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, path)| is_plain_file_name(name, ".aml") && path.is_file())
        .collect();
    names.sort();
    let mut tables = Vec::new();
    let mut total = 0usize;
    for (name, path) in names {
        if tables.len() >= MAX_ACPI_TABLES {
            break;
        }
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() as usize > MAX_ACPI_TABLE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if table_ok(&name, &bytes) && total + bytes.len() <= MAX_ACPI_TOTAL_BYTES {
            total += bytes.len();
            tables.push((name, bytes));
        }
    }
    // SSDTs are generated from the DSDT; without it the rest is useless.
    if !tables.iter().any(|(_, b)| b.starts_with(b"DSDT")) {
        tables.clear();
    }
    tables
}

fn table_ok(name: &str, bytes: &[u8]) -> bool {
    is_plain_file_name(name, ".aml")
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && bytes.len() <= MAX_ACPI_TABLE_BYTES
        && (bytes.starts_with(b"DSDT") || bytes.starts_with(b"SSDT"))
        && check_aml(bytes).is_ok()
}

/// Sanity limits for a profile that came from a file or the UI.
fn validate_profile(profile: &HardwareProfile) -> Result<(), AppError> {
    let fail = |why: String| {
        AppError::new("PROFILE_INVALID", format!("The hardware profile is not valid: {why}")).recoverable()
    };
    let long = |s: &str| s.chars().count() > 512;
    let mut names: Vec<&str> = vec![
        &profile.cpu.name,
        &profile.cpu.codename,
        &profile.motherboard_vendor,
        &profile.motherboard_model,
        &profile.source,
    ];
    names.extend(profile.chipset.as_deref());
    names.extend(profile.gpus.iter().map(|g| g.name.as_str()));
    names.extend(profile.ethernet.iter().map(|n| n.name.as_str()));
    names.extend(profile.wifi.iter().chain(profile.bluetooth.iter()).map(|n| n.name.as_str()));
    names.extend(profile.storage.iter().map(|s| s.name.as_str()));
    names.extend(profile.audio.iter().map(|a| a.codec_name.as_str()));
    if let Some(s) = names.into_iter().find(|s| long(s)) {
        return Err(fail(format!("a name is too long ({} characters)", s.chars().count())));
    }
    if profile.gpus.len() > 16 || profile.ethernet.len() > 16 || profile.storage.len() > 64 {
        return Err(fail("too many devices".into()));
    }
    if profile.cpu.cores > 1024 || profile.cpu.threads > 4096 || profile.ram_gb > 1 << 20 {
        return Err(fail("CPU or memory size out of range".into()));
    }
    if profile.audio.as_ref().and_then(|a| a.layout_id).is_some_and(|id| id > 0xFFFF) {
        return Err(fail("audio layout-id out of range".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{CpuPlatform, CpuVendor, ProfileAudio, ProfileCpu, ProfileGpu};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-profile-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut t = signature.to_vec();
        t.extend_from_slice(&((36 + body.len()) as u32).to_le_bytes());
        t.extend_from_slice(&[2, 0]);
        t.extend_from_slice(b"OCLICKTESTTABL");
        t.extend_from_slice(&[0; 12]);
        t.extend_from_slice(body);
        t
    }

    fn sample() -> HardwareProfile {
        HardwareProfile {
            cpu: ProfileCpu {
                name: "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz".into(),
                vendor: CpuVendor::Intel,
                platform: CpuPlatform::CoffeeLake,
                cores: 8,
                threads: 8,
                ..ProfileCpu::default()
            },
            gpus: vec![ProfileGpu { name: "Intel UHD Graphics 630".into(), ..ProfileGpu::default() }],
            audio: Some(ProfileAudio {
                codec_name: "Realtek ALC1220".into(),
                layout_id: Some(7),
                ..ProfileAudio::default()
            }),
            motherboard_vendor: "Gigabyte".into(),
            motherboard_model: "Z390 AORUS PRO".into(),
            chipset: Some("Z390".into()),
            ram_gb: 32,
            source: "scan".into(),
            scan_confidence: 0.9,
            ..HardwareProfile::default()
        }
    }

    #[test]
    fn envelope_round_trip_with_acpi_tables() {
        let tmp = TempDir::new();
        let dump = tmp.0.join("scan");
        std::fs::create_dir_all(&dump).unwrap();
        std::fs::write(dump.join("DSDT.aml"), table(b"DSDT", b"dsdt body")).unwrap();
        std::fs::write(dump.join("SSDT-1.aml"), table(b"SSDT", b"ssdt")).unwrap();
        std::fs::write(dump.join("MSDM.aml"), table(b"MSDM", b"PRODUCT-KEY")).unwrap();
        std::fs::write(dump.join("FACP.aml"), table(b"FACP", b"x")).unwrap();
        let mut profile = sample();
        profile.acpi_tables_dir = Some(dump.to_string_lossy().into_owned());

        let bytes = encode_profile(&profile).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("\"format\": \"opcore-oneclick-profile\""));
        assert!(!text.contains(&*dump.to_string_lossy()), "the local dump path is not exported");
        assert!(!text.contains("MSDM.aml"));

        let imported = decode_profile(&bytes).unwrap();
        assert_eq!(imported.profile.source, "imported");
        assert_eq!(imported.profile.cpu.platform, CpuPlatform::CoffeeLake);
        assert_eq!(imported.profile.audio.as_ref().unwrap().layout_id, Some(7));
        let names: Vec<&str> = imported.acpi_tables.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["DSDT.aml", "SSDT-1.aml"]);

        let file = tmp.0.join("profile.json");
        std::fs::write(&file, &bytes).unwrap();
        let acpi_root = tmp.0.join("acpi");
        let restored = import_from(&file, &acpi_root).unwrap();
        let dir = PathBuf::from(restored.acpi_tables_dir.unwrap());
        assert!(dir.starts_with(&acpi_root));
        assert_eq!(std::fs::read(dir.join("DSDT.aml")).unwrap(), table(b"DSDT", b"dsdt body"));
    }

    #[test]
    fn ssdt_dumps_without_a_dsdt_are_not_exported() {
        let tmp = TempDir::new();
        let dump = tmp.0.join("scan");
        std::fs::create_dir_all(&dump).unwrap();
        std::fs::write(dump.join("SSDT-1.aml"), table(b"SSDT", b"ssdt")).unwrap();
        let mut profile = sample();
        profile.acpi_tables_dir = Some(dump.to_string_lossy().into_owned());
        let text = String::from_utf8(encode_profile(&profile).unwrap()).unwrap();
        assert!(!text.contains("\"acpiTables\""), "{text}");
    }

    #[test]
    fn profiles_without_tables_import_without_a_dump_path() {
        let tmp = TempDir::new();
        let mut profile = sample();
        profile.acpi_tables_dir = Some(tmp.0.join("gone").to_string_lossy().into_owned());
        let bytes = encode_profile(&profile).unwrap();
        let file = tmp.0.join("p.json");
        std::fs::write(&file, bytes).unwrap();
        let restored = import_from(&file, &tmp.0.join("acpi")).unwrap();
        assert_eq!(restored.acpi_tables_dir, None);
        assert!(!tmp.0.join("acpi").exists());
    }

    #[test]
    fn foreign_and_future_files_are_refused() {
        assert_eq!(decode_profile(b"not json").unwrap_err().code, "PROFILE_INVALID");
        let bare = serde_json::to_vec(&sample()).unwrap();
        assert_eq!(decode_profile(&bare).unwrap_err().code, "PROFILE_FORMAT_UNKNOWN");
        let future = serde_json::json!({ "format": PROFILE_FORMAT, "version": 2, "profile": {} });
        assert_eq!(
            decode_profile(&serde_json::to_vec(&future).unwrap()).unwrap_err().code,
            "PROFILE_VERSION_UNSUPPORTED"
        );
        let broken = serde_json::json!({ "format": PROFILE_FORMAT, "version": 1, "profile": { "cpu": 5 } });
        assert_eq!(decode_profile(&serde_json::to_vec(&broken).unwrap()).unwrap_err().code, "PROFILE_INVALID");

        let mut huge = sample();
        huge.cpu.name = "x".repeat(1000);
        assert_eq!(encode_profile(&huge).unwrap_err().code, "PROFILE_INVALID");
    }

    #[test]
    fn damaged_tables_are_dropped() {
        let mut bytes: serde_json::Value = serde_json::from_slice(&encode_profile(&sample()).unwrap()).unwrap();
        let engine = base64::engine::general_purpose::STANDARD;
        bytes["acpiTables"] = serde_json::json!({
            "SSDT-1.aml": engine.encode(table(b"SSDT", b"x")),
            "../evil.aml": engine.encode(table(b"DSDT", b"x")),
            "DSDT.aml": "not base64!",
        });
        let imported = decode_profile(&serde_json::to_vec(&bytes).unwrap()).unwrap();
        // No usable DSDT, so the SSDT alone is not kept either.
        assert!(imported.acpi_tables.is_empty());
    }

    #[test]
    fn export_paths_must_be_absolute_files() {
        assert_eq!(target_path("relative.json").unwrap_err().code, "PATH_INVALID");
        assert_eq!(target_path("").unwrap_err().code, "PATH_INVALID");
        let tmp = TempDir::new();
        assert_eq!(target_path(&tmp.0.to_string_lossy()).unwrap_err().code, "PATH_INVALID");
        assert!(target_path(&tmp.0.join("p.json").to_string_lossy()).is_ok());
    }

    #[test]
    fn catalog_lists_everything_with_ipc_ids() {
        let c = catalog();
        assert_eq!(c.opencore_version, "1.0.8");
        assert_eq!(c.macos_versions.len(), MacOsVersion::ALL.len());
        assert_eq!(c.macos_versions[0].id, "26");
        assert_eq!(c.macos_versions[0].label, "macOS Tahoe 26");
        assert_eq!(c.cpu_platforms.len(), cpu_db::all_platforms().len());
        let coffee = c.cpu_platforms.iter().find(|o| o.id == "coffee_lake").unwrap();
        assert!(coffee.detail.as_deref().unwrap().starts_with("macOS"));
        let ivy = c.cpu_platforms.iter().find(|o| o.id == "ivy_bridge").unwrap();
        let ivy = ivy.detail.as_deref().unwrap();
        assert!(ivy.contains("to Monterey") && ivy.contains("Ventura to Tahoe with CryptexFixup.kext"), "{ivy}");
        assert!(c.cpu_platforms.iter().any(|o| o.id == "unknown"));
        assert_eq!(c.gpu_families.len(), gpu_db::all_families().len());
        assert!(c.gpu_families.iter().any(|o| o.id == "amd_navi21"));
        assert_eq!(c.smbios_models.len(), smbios_db::all().len());
        let imac = c.smbios_models.iter().find(|o| o.id == "iMac20,1").unwrap();
        assert!(imac.detail.as_deref().unwrap().contains("10.15.6 to 26"));
        let ids: Vec<&str> = c.form_factors.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["desktop", "laptop", "all_in_one", "mini_pc"]);
    }
}
