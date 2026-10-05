//! Compatibility, planning, EFI build/validation/export commands.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::contracts::{BuildResult, CompatibilityReport, ValidationResult};
use crate::domain::model::{BiosSetting, BuildOptions, BuildPlan, HardwareProfile, MacOsVersion};
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::tasks::registry::TaskRegistry;

#[tauri::command]
pub async fn check_compatibility(
    profile: HardwareProfile,
    target: Option<MacOsVersion>,
) -> Result<CompatibilityReport, AppError> {
    let _ = (profile, target);
    todo!("check_compatibility")
}

/// Preview the plan (SMBIOS, kexts, SSDTs, quirks, boot-args) without downloading anything.
#[tauri::command]
pub async fn plan_build(profile: HardwareProfile, options: BuildOptions) -> Result<BuildPlan, AppError> {
    let _ = (profile, options);
    todo!("plan_build")
}

#[tauri::command]
pub async fn get_bios_settings(profile: HardwareProfile, target: MacOsVersion) -> Result<Vec<BiosSetting>, AppError> {
    let _ = (profile, target);
    todo!("get_bios_settings")
}

/// Full build (task kind "efi-build"): plan → fetch OpenCore/kexts/resources →
/// assemble EFI → generate config.plist from Sample.plist → ocvalidate.
#[tauri::command]
pub async fn build_efi(
    profile: HardwareProfile,
    options: BuildOptions,
    task_registry: State<'_, Arc<TaskRegistry>>,
    paths: State<'_, AppPaths>,
    app: AppHandle,
) -> Result<BuildResult, AppError> {
    let _ = (profile, options, &task_registry, &paths, app);
    todo!("build_efi")
}

/// Validate an EFI folder (directory containing EFI/, or a config.plist path).
#[tauri::command]
pub async fn validate_efi(path: String, paths: State<'_, AppPaths>) -> Result<ValidationResult, AppError> {
    let _ = (path, &paths);
    todo!("validate_efi")
}

/// Copy the built EFI folder to `destination` (a folder) and return the new path.
#[tauri::command]
pub async fn export_efi(efi_path: String, destination: String) -> Result<String, AppError> {
    let _ = (efi_path, destination);
    todo!("export_efi")
}
