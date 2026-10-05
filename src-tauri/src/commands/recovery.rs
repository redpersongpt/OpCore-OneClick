//! macOS recovery download commands (macrecovery protocol).

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::contracts::RecoveryCacheInfo;
use crate::domain::model::MacOsVersion;
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::tasks::registry::TaskRegistry;

/// Download BaseSystem.dmg + BaseSystem.chunklist for `version` into
/// `paths.recovery_dir(version)/com.apple.recovery.boot`, resumable, verified
/// against the chunklist. Emits `recovery:progress` (task kind "recovery-download").
#[tauri::command]
pub async fn download_recovery(
    version: MacOsVersion,
    task_registry: State<'_, Arc<TaskRegistry>>,
    paths: State<'_, AppPaths>,
    app: AppHandle,
) -> Result<RecoveryCacheInfo, AppError> {
    let _ = (version, &task_registry, &paths, app);
    todo!("download_recovery")
}

#[tauri::command]
pub async fn get_cached_recovery_info(version: MacOsVersion, paths: State<'_, AppPaths>) -> Result<RecoveryCacheInfo, AppError> {
    let _ = (version, &paths);
    todo!("get_cached_recovery_info")
}

#[tauri::command]
pub async fn clear_recovery_cache(paths: State<'_, AppPaths>) -> Result<(), AppError> {
    let _ = &paths;
    todo!("clear_recovery_cache")
}
