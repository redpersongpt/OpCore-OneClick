//! Hardware scan and profile commands.

use std::sync::Arc;

use tauri::State;

use crate::contracts::{Catalog, ScanResult};
use crate::domain::model::HardwareProfile;
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::tasks::registry::TaskRegistry;

/// Scan this machine (task kind "hardware-scan"), dump ACPI tables into
/// `paths.acpi`, and interpret the result into a profile.
#[tauri::command]
pub async fn scan_hardware(
    task_registry: State<'_, Arc<TaskRegistry>>,
    paths: State<'_, AppPaths>,
) -> Result<ScanResult, AppError> {
    let _ = (&task_registry, &paths);
    todo!("scan_hardware")
}

/// Re-interpret a manually edited profile.
#[tauri::command]
pub async fn refresh_profile(profile: HardwareProfile) -> Result<HardwareProfile, AppError> {
    let _ = profile;
    todo!("refresh_profile")
}

/// Options for the manual profile editor and version picker.
#[tauri::command]
pub async fn get_catalog() -> Result<Catalog, AppError> {
    todo!("get_catalog")
}

/// Save a profile as JSON (to build an EFI for this machine on another computer).
#[tauri::command]
pub async fn export_profile(profile: HardwareProfile, path: String) -> Result<(), AppError> {
    let _ = (profile, path);
    todo!("export_profile")
}

#[tauri::command]
pub async fn import_profile(path: String) -> Result<HardwareProfile, AppError> {
    let _ = path;
    todo!("import_profile")
}
