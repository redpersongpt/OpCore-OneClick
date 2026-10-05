//! App metadata and update check.

use crate::contracts::{AppVersionInfo, UpdateInfo};
use crate::error::AppError;

#[tauri::command]
pub async fn get_app_info() -> Result<AppVersionInfo, AppError> {
    todo!("get_app_info")
}

/// Compare the running version with the latest GitHub release (semver).
#[tauri::command]
pub async fn check_for_updates() -> Result<UpdateInfo, AppError> {
    todo!("check_for_updates")
}
