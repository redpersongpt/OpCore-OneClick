//! Host-platform integration: hardware scanning, ACPI table dumping and
//! removable-disk operations for Windows, Linux and macOS.

pub mod common;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

use std::path::{Path, PathBuf};

use crate::contracts::{DetectedHardware, DiskInfo};
use crate::error::AppError;
use crate::tasks::cancellation::CancellationToken;

/// Scan the machine the app is running on. `acpi_dir` is where ACPI tables
/// are dumped (DSDT.aml, SSDT-*.aml) when the platform allows it.
pub async fn scan(acpi_dir: &Path, cancel: &CancellationToken) -> Result<DetectedHardware, AppError> {
    #[cfg(target_os = "windows")]
    return windows::scanner::scan(acpi_dir, cancel).await;
    #[cfg(target_os = "linux")]
    return linux::scanner::scan(acpi_dir, cancel).await;
    #[cfg(target_os = "macos")]
    return macos::scanner::scan(acpi_dir, cancel).await;
    #[allow(unreachable_code)]
    {
        let _ = (acpi_dir, cancel);
        Err(AppError::new("UNSUPPORTED_PLATFORM", "Scanning is not supported on this OS"))
    }
}

/// External/removable disks that can be offered as flash targets. System and
/// boot disks are included but flagged `is_system_disk` with a reason.
pub async fn list_disks() -> Result<Vec<DiskInfo>, AppError> {
    #[cfg(target_os = "windows")]
    return windows::disk::list_disks().await;
    #[cfg(target_os = "linux")]
    return linux::disk::list_disks().await;
    #[cfg(target_os = "macos")]
    return macos::disk::list_disks().await;
    #[allow(unreachable_code)]
    Err(AppError::new("UNSUPPORTED_PLATFORM", "Disk listing is not supported on this OS"))
}

pub async fn disk_info(device: &str) -> Result<DiskInfo, AppError> {
    list_disks()
        .await?
        .into_iter()
        .find(|d| d.device_path.eq_ignore_ascii_case(device))
        .ok_or_else(|| AppError::new("DISK_NOT_FOUND", format!("Disk {device} is no longer connected")))
}

/// What to write to the target disk.
#[derive(Debug, Clone)]
pub struct FlashJob {
    pub device: String,
    /// Directory containing the `EFI` folder to copy to the volume root.
    pub efi_source: PathBuf,
    /// Directory containing `com.apple.recovery.boot` to copy, if any.
    pub recovery_source: Option<PathBuf>,
    /// FAT32 volume label (max 11 chars).
    pub label: String,
}

/// Progress callback: (phase, fraction 0..1, message).
pub type FlashProgressFn<'a> = &'a (dyn Fn(&str, f64, &str) + Send + Sync);

/// Erase the disk to GPT + one FAT32 partition (capped at 32 GB on Windows so
/// FAT32 formatting works), copy EFI and recovery files, flush and verify.
pub async fn flash(job: &FlashJob, progress: FlashProgressFn<'_>, cancel: &CancellationToken) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    return windows::disk::flash(job, progress, cancel).await;
    #[cfg(target_os = "linux")]
    return linux::disk::flash(job, progress, cancel).await;
    #[cfg(target_os = "macos")]
    return macos::disk::flash(job, progress, cancel).await;
    #[allow(unreachable_code)]
    {
        let _ = (job, progress, cancel);
        Err(AppError::new("UNSUPPORTED_PLATFORM", "Flashing is not supported on this OS"))
    }
}
