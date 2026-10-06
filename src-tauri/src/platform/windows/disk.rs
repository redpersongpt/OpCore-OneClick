//! Removable disk enumeration and flashing. See `platform::flash`.
//!
//! Windows: Get-Disk inventory; one diskpart run (identity re-checked first)
//! that wipes the disk, creates a GPT partition capped at 32 000 MB, formats
//! it FAT32 and assigns a letter; copy and read-back verification
//! in-process. The app runs elevated through its manifest.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{info, warn};

use crate::contracts::DiskInfo;
use crate::error::AppError;
use crate::platform::{FlashJob, FlashProgressFn};
use crate::safety::device_path::validate_windows_disk;
use crate::safety::disk_identity::ensure_confirmed;
use crate::safety::disks::windows::{parse_inventory, WindowsHost, INVENTORY_SCRIPT};
use crate::safety::disks::{self, drive_letter};
use crate::safety::flash_plan::{
    diskpart_error_is_size, diskpart_error_is_transient, diskpart_flash_script, diskpart_rescan_script,
    parse_diskpart_result, parse_partition_number, parse_volume_root, partition_capacity, windows_diskpart_ps,
    windows_partition_query_ps, windows_partition_sizes, windows_volume_query_ps, DiskpartResult, FlashPhase,
    WindowsIdentity, VOLUME_LABEL,
};
use crate::safety::payload::{
    copy_payload, load_payload, read_target, required_capacity, verify_target, CopyStage,
};
use crate::services::process;
use crate::tasks::cancellation::CancellationToken;

const PS_TIMEOUT: Duration = Duration::from_secs(90);
const DISKPART_TIMEOUT: Duration = Duration::from_secs(600);
const DISKPART_ATTEMPTS: usize = 3;

/// Drive letters of the running app, its data and the temp folder.
fn app_drives() -> Vec<char> {
    let candidates: Vec<Option<PathBuf>> = vec![
        std::env::current_exe().ok(),
        Some(std::env::temp_dir()),
        dirs::data_dir(),
        dirs::data_local_dir(),
        dirs::cache_dir(),
    ];
    let mut letters: Vec<char> = candidates.into_iter().flatten().filter_map(|p| drive_letter(&p)).collect();
    letters.sort_unstable();
    letters.dedup();
    letters
}

pub async fn list_disks() -> Result<Vec<DiskInfo>, AppError> {
    if !process::is_elevated() {
        return Err(process::admin_required());
    }
    let output = process::powershell(INVENTORY_SCRIPT, PS_TIMEOUT).await?.ensure_success("Reading the disk list")?;
    let mut disks = parse_inventory(&output.stdout, &WindowsHost { app_drives: app_drives() })?;
    disks::sort_disks(&mut disks);
    info!(count = disks.len(), "External disks enumerated");
    Ok(disks)
}

async fn run_diskpart(disk: u32, script: &str, identity: Option<&WindowsIdentity<'_>>) -> Result<DiskpartResult, AppError> {
    let ps = windows_diskpart_ps(disk, script, identity);
    let output = process::powershell(&ps, DISKPART_TIMEOUT).await?;
    if !output.success() {
        return Err(AppError::new("DISKPART_FAILED", format!("diskpart could not be started: {}", output.summary())));
    }
    Ok(parse_diskpart_result(&output.stdout))
}

/// Run a destructive diskpart script, retrying transient Virtual Disk
/// Service failures after a rescan.
async fn diskpart_with_retries(
    disk: u32,
    script: &str,
    identity: Option<&WindowsIdentity<'_>>,
    what: &str,
) -> Result<(), AppError> {
    let mut last = String::new();
    for attempt in 1..=DISKPART_ATTEMPTS {
        match run_diskpart(disk, script, identity).await? {
            DiskpartResult::IdentityChanged => {
                return Err(AppError::new("DISK_IDENTITY_CHANGED", "The disk no longer matches the one you confirmed")
                    .with_suggestion("Select the USB drive again."));
            }
            DiskpartResult::Finished { exit_code: 0, .. } => return Ok(()),
            DiskpartResult::Finished { exit_code, output } => {
                warn!(attempt, exit_code, "{what} failed: {output}");
                if diskpart_error_is_size(&output) {
                    return Err(AppError::new("FAT32_TOO_LARGE", "Windows refused to format a FAT32 volume this large"));
                }
                last = output.clone();
                if !diskpart_error_is_transient(&output, exit_code) || attempt == DISKPART_ATTEMPTS {
                    break;
                }
                let _ = run_diskpart(disk, &diskpart_rescan_script(), None).await;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
    Err(AppError::new("DISKPART_FAILED", format!("{what} failed: {}", tail(&last)))
        .with_suggestion("Unplug and reconnect the USB drive, close Explorer windows showing it, and try again."))
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join(" ")
}

async fn find_partition(disk: u32) -> Result<u32, AppError> {
    for _ in 0..8 {
        let output = process::powershell(&windows_partition_query_ps(disk), PS_TIMEOUT).await?;
        if let Some(number) = parse_partition_number(&output.stdout) {
            return Ok(number);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(AppError::new("PARTITION_FAILED", "The new partition did not appear"))
}

async fn find_volume_root(disk: u32, partition: u32) -> Result<PathBuf, AppError> {
    for _ in 0..15 {
        let output = process::powershell(&windows_volume_query_ps(disk, partition), PS_TIMEOUT).await?;
        if let Some(root) = parse_volume_root(&output.stdout) {
            let root = PathBuf::from(root);
            if root.is_dir() {
                return Ok(root);
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(AppError::new("MOUNT_FAILED", "Windows did not mount the new FAT32 volume")
        .with_suggestion("Check that automatic mounting is enabled (mountvol /E) and try again."))
}

/// Erase, partition and format in one diskpart run, then locate the new
/// volume through Get-Partition; returns the volume root.
async fn prepare_volume(
    disk: u32,
    identity: &WindowsIdentity<'_>,
    size_mb: u64,
    report: &(dyn Fn(FlashPhase, f64, &str) + Send + Sync),
) -> Result<PathBuf, AppError> {
    report(FlashPhase::Partition, 0.0, "Erasing the USB drive, creating a GPT partition and formatting it FAT32");
    let script = diskpart_flash_script(disk, size_mb, VOLUME_LABEL)?;
    diskpart_with_retries(disk, &script, Some(identity), "Erasing and formatting the USB drive").await?;
    report(FlashPhase::Format, 0.5, "Waiting for the new FAT32 volume");
    let partition = find_partition(disk).await?;
    let root = find_volume_root(disk, partition).await?;
    report(FlashPhase::Format, 1.0, "FAT32 volume ready");
    Ok(root)
}

async fn flush_volume(root: &Path) {
    if let Some(letter) = drive_letter(root) {
        let script = format!("Write-VolumeCache -DriveLetter {letter} -ErrorAction SilentlyContinue");
        if let Err(error) = process::powershell(&script, PS_TIMEOUT).await {
            warn!("Write-VolumeCache failed: {error}");
        }
    }
}

pub async fn flash(job: &FlashJob, progress: FlashProgressFn<'_>, cancel: &CancellationToken) -> Result<(), AppError> {
    if !process::is_elevated() {
        return Err(process::admin_required());
    }
    let number = validate_windows_disk(&job.device)?;
    let with_recovery = job.recovery_source.is_some();
    let report = |phase: FlashPhase, fraction: f64, message: &str| {
        progress(phase.as_str(), phase.overall(fraction, with_recovery), message);
    };

    report(FlashPhase::Prepare, 0.0, "Checking the USB drive");
    let disk = crate::platform::disk_info(&job.device).await?;
    if disks::is_blocked(&disk) {
        return Err(AppError::new("DISK_BLOCKED", disk.blocked_reason.clone().unwrap_or_else(|| "This disk cannot be erased".into())));
    }
    // diskpart re-checks size and serial of this listing, so it must be the confirmed disk.
    ensure_confirmed(&disk)?;
    let (efi, recovery) = load_payload(&job.efi_source, job.recovery_source.as_deref()).await?;
    let needed = required_capacity(&efi, recovery.as_ref());
    let sizes: Vec<u64> =
        windows_partition_sizes(disk.size_bytes).into_iter().filter(|size| partition_capacity(*size) >= needed).collect();
    if sizes.is_empty() {
        return Err(AppError::new("DISK_TOO_SMALL", format!("The USB drive is too small; {} MB are needed", needed / 1_000_000)));
    }
    let identity = WindowsIdentity { size_bytes: disk.size_bytes, serial: disk.serial_number.as_deref() };
    // Last point where cancelling leaves the drive untouched.
    cancel.check()?;
    report(FlashPhase::Prepare, 1.0, "Ready to erase");

    let mut root = None;
    let mut last_error = None;
    for size in sizes {
        match prepare_volume(number, &identity, size, &report).await {
            Ok(path) => {
                root = Some(path);
                break;
            }
            Err(error) if error.code == "FAT32_TOO_LARGE" || error.code == "DISKPART_FAILED" => {
                warn!(size_mb = size, "Retrying with a smaller partition: {}", error.message);
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    let Some(root) = root else {
        return Err(last_error.unwrap_or_else(|| AppError::new("FORMAT_FAILED", "Formatting the USB drive failed")));
    };
    info!(root = %root.display(), "FAT32 volume ready");

    copy_payload(&root, &efi, recovery.as_ref(), &|stage, fraction, message| match stage {
        CopyStage::Efi => report(FlashPhase::CopyEfi, fraction, message),
        CopyStage::Recovery => report(FlashPhase::CopyRecovery, fraction, message),
    })
    .await?;

    report(FlashPhase::Verify, 0.0, "Verifying the USB drive");
    flush_volume(&root).await;
    let target = read_target(&root, &efi, recovery.as_ref()).await?;
    verify_target(&efi, recovery.as_ref(), &target)?;
    report(FlashPhase::Verify, 1.0, "Verified");
    report(FlashPhase::Complete, 1.0, "The USB drive is ready");
    Ok(())
}
