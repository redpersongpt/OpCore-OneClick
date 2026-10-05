//! USB target selection and flashing commands.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::contracts::{DiskInfo, FlashConfirmation, PrivilegeStatus};
use crate::domain::model::MacOsVersion;
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::safety::flash_auth::FlashSecurityContext;
use crate::tasks::registry::TaskRegistry;

#[tauri::command]
pub async fn list_usb_devices() -> Result<Vec<DiskInfo>, AppError> {
    todo!("list_usb_devices")
}

#[tauri::command]
pub async fn get_disk_info(device: String) -> Result<DiskInfo, AppError> {
    let _ = device;
    todo!("get_disk_info")
}

#[tauri::command]
pub async fn check_privileges() -> Result<PrivilegeStatus, AppError> {
    todo!("check_privileges")
}

/// Issue a short-lived, single-use token binding (disk identity, EFI hash,
/// recovery choice) that `flash_usb` must present.
#[tauri::command]
pub async fn flash_prepare_confirmation(
    device: String,
    efi_path: String,
    recovery: Option<MacOsVersion>,
    security: State<'_, Arc<FlashSecurityContext>>,
    paths: State<'_, AppPaths>,
) -> Result<FlashConfirmation, AppError> {
    let _ = (device, efi_path, recovery, &security, &paths);
    todo!("flash_prepare_confirmation")
}

/// Erase `device`, write the EFI and (optionally) the cached recovery image.
/// Emits `flash:progress` events (task kind "usb-flash").
#[tauri::command]
pub async fn flash_usb(
    device: String,
    efi_path: String,
    token: String,
    recovery: Option<MacOsVersion>,
    task_registry: State<'_, Arc<TaskRegistry>>,
    security: State<'_, Arc<FlashSecurityContext>>,
    paths: State<'_, AppPaths>,
    app: AppHandle,
) -> Result<(), AppError> {
    let _ = (device, efi_path, token, recovery, &task_registry, &security, &paths, app);
    todo!("flash_usb")
}
