//! Removable disk enumeration and flashing. See `platform::flash`.
//!
//! macOS: diskutil inventory, `diskutil eraseDisk FAT32 OPENCORE GPT`, copy
//! to the mounted volume (without AppleDouble files), verify, eject.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{info, warn};

use crate::contracts::DiskInfo;
use crate::error::AppError;
use crate::platform::{FlashJob, FlashProgressFn};
use crate::safety::device_path::validate_macos_disk;
use crate::safety::disk_identity::ensure_confirmed;
use crate::safety::disks::macos::{
    boot_disks, build_disk, fat_partition, mounts_by_disk, parse_info, parse_list, whole_disks, MacHost,
};
use crate::safety::disks;
use crate::safety::flash_plan::{FlashPhase, VOLUME_LABEL};
use crate::safety::payload::{
    copy_payload, load_payload, read_target, required_capacity, verify_target, CopyStage,
};
use crate::services::process::{self, sh_quote};
use crate::tasks::cancellation::CancellationToken;

const DISKUTIL: &str = "/usr/sbin/diskutil";
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
const ERASE_TIMEOUT: Duration = Duration::from_secs(600);
/// `eraseDisk … GPT` puts a 200 MiB EFI partition in front of the volume.
const EFI_PARTITION_BYTES: u64 = 210 * 1024 * 1024;

async fn diskutil(args: &[&str]) -> Result<String, AppError> {
    let output = process::run(DISKUTIL, args, QUERY_TIMEOUT).await?;
    Ok(output.ensure_success(&format!("diskutil {}", args.first().copied().unwrap_or("")))?.stdout)
}

fn host_facts(root_info: Option<&str>) -> MacHost {
    let boot = root_info.and_then(|xml| parse_info(xml.as_bytes()).ok()).map(|info| boot_disks(&info)).unwrap_or_default();
    let guarded_paths: Vec<PathBuf> =
        [std::env::current_exe().ok(), dirs::data_dir(), dirs::cache_dir()].into_iter().flatten().collect();
    MacHost { boot_disks: boot, guarded_paths }
}

pub async fn list_disks() -> Result<Vec<DiskInfo>, AppError> {
    let external = diskutil(&["list", "-plist", "external", "physical"]).await?;
    let list = parse_list(external.as_bytes())?;
    let all = diskutil(&["list", "-plist"]).await.ok().and_then(|xml| parse_list(xml.as_bytes()).ok());
    let mounts = all.as_ref().map(mounts_by_disk).unwrap_or_default();
    let root = diskutil(&["info", "-plist", "/"]).await.ok();
    let host = host_facts(root.as_deref());

    let mut result = Vec::new();
    for whole in whole_disks(&list) {
        let xml = match diskutil(&["info", "-plist", &whole]).await {
            Ok(xml) => xml,
            Err(error) => {
                warn!(disk = %whole, "diskutil info failed: {}", error.message);
                continue;
            }
        };
        let info = parse_info(xml.as_bytes())?;
        let partitions: Vec<_> =
            list.entries.iter().filter(|e| e.device_identifier == whole).flat_map(|e| e.partitions.clone()).collect();
        let disk_mounts = mounts.get(&whole).cloned().unwrap_or_default();
        if let Some(disk) = build_disk(&info, &partitions, &disk_mounts, &host) {
            result.push(disk);
        }
    }
    disks::sort_disks(&mut result);
    info!(count = result.len(), "External disks enumerated");
    Ok(result)
}

fn needs_admin(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("permission") || lower.contains("not permitted") || lower.contains("privilege") || lower.contains("-69877")
}

async fn erase(device: &str) -> Result<(), AppError> {
    let args = ["eraseDisk", "FAT32", VOLUME_LABEL, "GPT", device];
    let output = process::run(DISKUTIL, &args, ERASE_TIMEOUT).await?;
    if output.success() {
        return Ok(());
    }
    let first = output.summary();
    if !needs_admin(&first) {
        return Err(erase_error(&first));
    }
    info!("diskutil eraseDisk needs administrator rights, asking");
    let script = format!("{DISKUTIL} eraseDisk FAT32 {VOLUME_LABEL} GPT {}\n", sh_quote(device));
    let elevated = process::run_elevated_script(&script, ERASE_TIMEOUT).await?;
    if elevated.success() {
        Ok(())
    } else {
        Err(erase_error(&elevated.summary()))
    }
}

fn erase_error(output: &str) -> AppError {
    let lower = output.to_lowercase();
    let suggestion = if lower.contains("unmount") || lower.contains("busy") {
        "Close any window or app that uses the USB drive and try again."
    } else {
        "Reconnect the USB drive and try again, or try another drive."
    };
    AppError::new("FORMAT_FAILED", format!("Erasing the USB drive failed: {output}")).with_suggestion(suggestion)
}

/// Mount point of the new FAT32 volume on `whole` ("disk4").
async fn volume_root(whole: &str) -> Result<PathBuf, AppError> {
    for attempt in 0..10 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let Ok(xml) = diskutil(&["list", "-plist", whole]).await else { continue };
        let Ok(list) = parse_list(xml.as_bytes()) else { continue };
        let Some(partition) = fat_partition(&list, whole) else { continue };
        let Ok(info_xml) = diskutil(&["info", "-plist", &partition]).await else { continue };
        let Ok(info) = parse_info(info_xml.as_bytes()) else { continue };
        match info.mount_point.filter(|m| !m.is_empty()) {
            Some(mount) => return Ok(PathBuf::from(mount)),
            None => {
                let _ = diskutil(&["mount", &partition]).await;
            }
        }
    }
    Err(AppError::new("MOUNT_FAILED", "macOS did not mount the new FAT32 volume"))
}

/// The disk at `device` as listed now: selectable and the one the user confirmed.
async fn confirmed_disk(device: &str) -> Result<DiskInfo, AppError> {
    let disk = crate::platform::disk_info(device).await?;
    if disks::is_blocked(&disk) {
        return Err(AppError::new("DISK_BLOCKED", disk.blocked_reason.clone().unwrap_or_else(|| "This disk cannot be erased".into())));
    }
    ensure_confirmed(&disk)?;
    Ok(disk)
}

/// Remove AppleDouble (`._*`) files: OpenCore would take `._BaseSystem.dmg`
/// for a second DMG.
fn remove_apple_double(root: &Path) {
    for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_file() && entry.file_name().to_string_lossy().starts_with("._") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub async fn flash(job: &FlashJob, progress: FlashProgressFn<'_>, cancel: &CancellationToken) -> Result<(), AppError> {
    validate_macos_disk(&job.device)?;
    let whole = job.device.trim_start_matches("/dev/").to_string();
    let with_recovery = job.recovery_source.is_some();
    let report = |phase: FlashPhase, fraction: f64, message: &str| {
        progress(phase.as_str(), phase.overall(fraction, with_recovery), message);
    };

    report(FlashPhase::Prepare, 0.0, "Checking the USB drive");
    let disk = confirmed_disk(&job.device).await?;
    let (efi, recovery) = load_payload(&job.efi_source, job.recovery_source.as_deref()).await?;
    let needed = required_capacity(&efi, recovery.as_ref());
    if disk.size_bytes.saturating_sub(EFI_PARTITION_BYTES) < needed {
        return Err(AppError::new("DISK_TOO_SMALL", format!("The USB drive is too small; {} MB are needed", needed / 1_000_000)));
    }
    // Last point where cancelling leaves the drive untouched.
    cancel.check()?;
    // diskutil erases whatever /dev/diskN is now; look once more right before.
    confirmed_disk(&job.device).await?;

    report(FlashPhase::Partition, 0.0, "Erasing the USB drive (GPT, FAT32)");
    erase(&job.device).await?;
    report(FlashPhase::Format, 0.5, "Mounting the new volume");
    let root = volume_root(&whole).await?;
    info!(root = %root.display(), "FAT32 volume ready");
    // Keep Spotlight from indexing the installer volume.
    let _ = std::fs::write(root.join(".metadata_never_index"), b"");

    copy_payload(&root, &efi, recovery.as_ref(), &|stage, fraction, message| match stage {
        CopyStage::Efi => report(FlashPhase::CopyEfi, fraction, message),
        CopyStage::Recovery => report(FlashPhase::CopyRecovery, fraction, message),
    })
    .await?;

    report(FlashPhase::Verify, 0.0, "Verifying the USB drive");
    let cleanup_root = root.clone();
    let _ = tokio::task::spawn_blocking(move || remove_apple_double(&cleanup_root)).await;
    let target = read_target(&root, &efi, recovery.as_ref()).await?;
    verify_target(&efi, recovery.as_ref(), &target)?;
    report(FlashPhase::Verify, 0.8, "Ejecting");
    if let Err(error) = diskutil(&["eject", &job.device]).await {
        warn!("Eject failed: {}", error.message);
    }
    report(FlashPhase::Verify, 1.0, "Verified");
    report(FlashPhase::Complete, 1.0, "The USB drive is ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::payload::tests::{make_efi, make_recovery, TempDir};
    use crate::safety::payload::{inspect_efi, inspect_recovery};

    /// Copy and read-back on a real FAT32 volume, for example a disk image
    /// erased with `diskutil eraseDisk FAT32 OPENCORE GPT`:
    /// `OPCORE_FAT_VOLUME=/Volumes/OPENCORE cargo test -- --ignored real_fat_volume`
    #[tokio::test]
    #[ignore]
    async fn copy_and_verify_on_a_real_fat_volume() {
        let root = PathBuf::from(std::env::var("OPCORE_FAT_VOLUME").unwrap());
        let src = TempDir::new();
        make_efi(&src.0);
        make_recovery(&src.0.join("rec"));
        let efi = inspect_efi(&src.0).unwrap();
        let recovery = inspect_recovery(&src.0.join("rec")).unwrap();
        copy_payload(&root, &efi, Some(&recovery), &|_, _, _| {}).await.unwrap();
        remove_apple_double(&root);
        let report = read_target(&root, &efi, Some(&recovery)).await.unwrap();
        verify_target(&efi, Some(&recovery), &report).unwrap();
        assert_eq!(report.dmg_count, Some(1));
    }
}
