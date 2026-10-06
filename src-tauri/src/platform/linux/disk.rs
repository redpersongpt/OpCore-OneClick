//! Removable disk enumeration and flashing. See `platform::flash`.
//!
//! Linux: lsblk inventory; every destructive step (unmount, wipe, GPT,
//! mkfs.vfat, copy, read-back) runs in one script as root through pkexec,
//! with progress read from the script's progress file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tracing::info;

use crate::contracts::DiskInfo;
use crate::error::AppError;
use crate::platform::{FlashJob, FlashProgressFn};
use crate::safety::device_path::validate_linux_disk;
use crate::safety::disk_identity::ensure_confirmed;
use crate::safety::disks::linux::{parse_lsblk, LinuxHost, LSBLK_FALLBACK_COLUMNS};
use crate::safety::disks;
use crate::safety::flash_plan::{linux_script_error, FlashPhase, LinuxFlashScript, VOLUME_LABEL};
use crate::safety::payload::{
    load_payload, parse_script_report, required_capacity, verify_target, RECOVERY_DIR_NAME,
};
use crate::services::process::{self, ElevatedScratch};
use crate::tasks::cancellation::CancellationToken;

const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const FLASH_TIMEOUT: Duration = Duration::from_secs(60 * 60);

fn tool(name: &str) -> String {
    process::find_in_path(name).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string())
}

fn host_facts() -> LinuxHost {
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    let mut guarded_paths: Vec<PathBuf> = [std::env::current_exe().ok(), dirs::data_dir(), dirs::cache_dir()]
        .into_iter()
        .flatten()
        .collect();
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        guarded_paths.push(PathBuf::from(appimage));
    }
    let mut mmc_types = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/sys/block") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("mmcblk") {
                if let Ok(kind) = std::fs::read_to_string(entry.path().join("device/type")) {
                    mmc_types.insert(name, kind.trim().to_string());
                }
            }
        }
    }
    LinuxHost { swaps, guarded_paths, mmc_types }
}

pub async fn list_disks() -> Result<Vec<DiskInfo>, AppError> {
    let lsblk = tool("lsblk");
    let mut output = process::run(&lsblk, &["-J", "-b", "-O"], LIST_TIMEOUT).await?;
    if !output.success() {
        output = process::run(&lsblk, &["-J", "-b", "-o", LSBLK_FALLBACK_COLUMNS], LIST_TIMEOUT)
            .await?
            .ensure_success("lsblk")?;
    }
    let mut disks = parse_lsblk(&output.stdout, &host_facts())?;
    disks::sort_disks(&mut disks);
    info!(count = disks.len(), "External disks enumerated");
    Ok(disks)
}

fn phase_message(phase: FlashPhase) -> &'static str {
    match phase {
        FlashPhase::Prepare => "Unmounting the USB drive",
        FlashPhase::Partition => "Creating a GPT partition",
        FlashPhase::Format => "Formatting FAT32",
        FlashPhase::CopyEfi => "Copying EFI folder",
        FlashPhase::CopyRecovery => "Copying macOS recovery",
        FlashPhase::Verify => "Verifying the USB drive",
        FlashPhase::Complete => "The USB drive is ready",
    }
}

pub async fn flash(job: &FlashJob, progress: FlashProgressFn<'_>, cancel: &CancellationToken) -> Result<(), AppError> {
    let device = validate_linux_disk(&job.device)?.to_string();
    let with_recovery = job.recovery_source.is_some();
    let report = |phase: FlashPhase, fraction: f64, message: &str| {
        progress(phase.as_str(), phase.overall(fraction, with_recovery), message);
    };

    report(FlashPhase::Prepare, 0.0, "Checking the USB drive");
    let disk = crate::platform::disk_info(&device).await?;
    if disks::is_blocked(&disk) {
        return Err(AppError::new("DISK_BLOCKED", disk.blocked_reason.clone().unwrap_or_else(|| "This disk cannot be erased".into())));
    }
    // The script re-checks size and serial of this listing, so it must be the confirmed disk.
    ensure_confirmed(&disk)?;
    let (efi, recovery) = load_payload(&job.efi_source, job.recovery_source.as_deref()).await?;
    let needed = required_capacity(&efi, recovery.as_ref());
    if disk.size_bytes < needed {
        return Err(AppError::new("DISK_TOO_SMALL", format!("The USB drive is too small; {} MB are needed", needed / 1_000_000)));
    }
    if process::elevation_method().is_none() {
        return Err(AppError::new("ELEVATION_UNAVAILABLE", "Writing the USB drive needs root rights, but neither pkexec nor sudo is available")
            .with_suggestion("Install polkit (pkexec) and run OpCore-OneClick from a desktop session."));
    }

    let scratch = ElevatedScratch::new()?;
    let mount_dir = scratch.subdir("mnt")?;
    // SAFETY: getuid/getgid have no preconditions and cannot fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let efi_dir = efi.root.join("EFI");
    let mut hash_files: Vec<&str> = efi.files.iter().filter(|f| f.sha256.is_some()).map(|f| f.rel.as_str()).collect();
    let mut recovery_files = Vec::new();
    if let Some(recovery) = &recovery {
        for file in recovery.files() {
            let name = file.rel.rsplit('/').next().unwrap_or(&file.rel);
            recovery_files.push((file.source.as_path(), name));
            if file.sha256.is_some() {
                hash_files.push(file.rel.as_str());
            }
        }
    }
    let script = LinuxFlashScript {
        device: &device,
        expected_size: disk.size_bytes,
        expected_serial: disk.serial_number.as_deref(),
        efi_dir: &efi_dir,
        recovery_files,
        mount_dir: &mount_dir,
        owner_uid: uid,
        owner_gid: gid,
        label: VOLUME_LABEL,
        hash_files,
    }
    .render()?;

    // Last point where cancelling leaves the drive untouched.
    cancel.check()?;
    report(FlashPhase::Prepare, 0.5, "Waiting for administrator approval");

    let progress_file = scratch.file("progress");
    let dmg_target = recovery.as_ref().map(|r| {
        let name = r.dmg.rel.rsplit('/').next().unwrap_or(&r.dmg.rel).to_string();
        mount_dir.join(RECOVERY_DIR_NAME).join(name)
    });
    let recovery_total = recovery.as_ref().map(|r| r.total_bytes()).unwrap_or(0).max(1);
    let run = process::run_elevated_script_in(&scratch, &script, FLASH_TIMEOUT);
    tokio::pin!(run);
    let mut seen_lines = 0usize;
    let mut phase = FlashPhase::Prepare;
    let mut poll = |phase: &mut FlashPhase| {
        if let Ok(text) = std::fs::read_to_string(&progress_file) {
            let lines: Vec<&str> = text.lines().collect();
            for line in lines.iter().skip(seen_lines) {
                if let Some(next) = line.strip_prefix("STEP ").and_then(FlashPhase::parse) {
                    *phase = next;
                    if next != FlashPhase::Complete {
                        report(next, 0.0, phase_message(next));
                    }
                }
            }
            seen_lines = lines.len();
        }
        if *phase == FlashPhase::CopyRecovery {
            if let Some(size) = dmg_target.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()) {
                report(FlashPhase::CopyRecovery, size as f64 / recovery_total as f64, phase_message(FlashPhase::CopyRecovery));
            }
        }
    };
    let output = loop {
        tokio::select! {
            result = &mut run => break result?,
            _ = tokio::time::sleep(Duration::from_millis(500)) => poll(&mut phase),
        }
    };
    poll(&mut phase);

    if !output.success() {
        return Err(linux_script_error(&output.stderr, output.status));
    }
    let target = parse_script_report(&output.stdout);
    verify_target(&efi, recovery.as_ref(), &target)?;
    report(FlashPhase::Verify, 1.0, "Verified");
    report(FlashPhase::Complete, 1.0, phase_message(FlashPhase::Complete));
    info!(device = %device, "USB drive written");
    Ok(())
}
