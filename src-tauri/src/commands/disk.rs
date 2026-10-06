//! USB target selection and flashing commands.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};
use tracing::{error, info, warn};

use crate::contracts::{DiskInfo, FlashConfirmation, FlashProgress, PrivilegeStatus};
use crate::domain::model::MacOsVersion;
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::platform::{self, FlashJob};
use crate::safety::device_path::validate_host_disk;
use crate::safety::disk_identity::{build_fingerprint, confirm_target, find_collisions};
use crate::safety::disks::{disk_display, is_blocked, path_on_disk};
use crate::safety::flash_auth::{FlashBinding, FlashSecurityContext};
use crate::safety::flash_plan::VOLUME_LABEL;
use crate::safety::payload::{self, EfiPayload};
use crate::services::process;
use crate::tasks::registry::TaskRegistry;

/// One flash at a time, app-wide.
static FLASHING: AtomicBool = AtomicBool::new(false);

/// Held while a USB drive is being written.
struct FlashGuard;

impl FlashGuard {
    fn acquire() -> Option<Self> {
        FLASHING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| FlashGuard)
    }
}

impl Drop for FlashGuard {
    fn drop(&mut self) {
        FLASHING.store(false, Ordering::SeqCst);
    }
}

/// True while a USB drive is being written.
pub(crate) fn flash_in_progress() -> bool {
    FLASHING.load(Ordering::SeqCst)
}

const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

#[tauri::command]
pub async fn list_usb_devices() -> Result<Vec<DiskInfo>, AppError> {
    // The platform layer returns selectable disks first, system disks flagged.
    platform::list_disks().await
}

#[tauri::command]
pub async fn get_disk_info(device: String) -> Result<DiskInfo, AppError> {
    platform::disk_info(&device).await
}

#[tauri::command]
pub async fn check_privileges() -> Result<PrivilegeStatus, AppError> {
    let elevated = process::is_elevated();
    let method = process::elevation_method();
    let (can_elevate, detail) = if elevated {
        (true, "Running with administrator rights.".to_string())
    } else if cfg!(windows) {
        (false, "Writing USB drives needs administrator rights: restart OpCore-OneClick with \"Run as administrator\".".to_string())
    } else {
        match method {
            Some("pkexec") => (true, "You will be asked for your password when the USB drive is written.".to_string()),
            Some("sudo") => (
                true,
                "sudo will be used to write the USB drive; it works only without a password prompt. Installing polkit (pkexec) is recommended."
                    .to_string(),
            ),
            Some(_) => (true, "macOS asks for your password if erasing the drive needs it.".to_string()),
            None => (false, "Neither pkexec nor sudo is available to write the USB drive.".to_string()),
        }
    };
    Ok(PrivilegeStatus { elevated, can_elevate, detail })
}

/// Fail early (before a confirmation is issued) when the disk cannot be written.
fn ensure_can_write() -> Result<(), AppError> {
    if cfg!(windows) && !process::is_elevated() {
        return Err(process::admin_required());
    }
    if process::elevation_method().is_none() {
        return Err(AppError::new("ELEVATION_UNAVAILABLE", "No way to obtain administrator rights for writing the USB drive")
            .with_suggestion("Install polkit (pkexec) and run OpCore-OneClick from a desktop session."));
    }
    Ok(())
}

/// The EFI folder must be a finished build of this app.
fn resolve_efi(efi_path: &str, paths: &AppPaths) -> Result<PathBuf, AppError> {
    let root = payload::resolve_efi_root(Path::new(efi_path))?;
    let builds = paths.builds.canonicalize()?;
    if !root.starts_with(&builds) {
        return Err(AppError::new("EFI_OUTSIDE_BUILDS", "Only EFI folders built by OpCore-OneClick can be flashed")
            .with_context(serde_json::json!({ "path": efi_path })));
    }
    Ok(root)
}

fn find_disk<'a>(all: &'a [DiskInfo], device: &str) -> Result<&'a DiskInfo, AppError> {
    all.iter()
        .find(|d| d.device_path.eq_ignore_ascii_case(device.trim()))
        .ok_or_else(|| AppError::new("DISK_NOT_FOUND", format!("Disk {device} is no longer connected")).recoverable())
}

/// Everything the flash depends on, as it is right now.
struct FlashState {
    disk: DiskInfo,
    binding: FlashBinding,
    efi: EfiPayload,
    recovery_dir: Option<PathBuf>,
}

async fn current_state(
    all: &[DiskInfo],
    device: &str,
    efi_path: &str,
    recovery: Option<MacOsVersion>,
    paths: &AppPaths,
) -> Result<FlashState, AppError> {
    let disk = find_disk(all, device)?.clone();
    validate_host_disk(&disk.device_path)?;
    if is_blocked(&disk) {
        return Err(AppError::new(
            "DISK_BLOCKED",
            format!(
                "{} cannot be used: {}",
                disk.device_path,
                disk.blocked_reason.clone().unwrap_or_else(|| "it holds the running system".into())
            ),
        ));
    }
    let efi_root = resolve_efi(efi_path, paths)?;
    let recovery_dir = recovery.map(|v| paths.recovery_dir(v.id()));
    let (efi, recovery_payload) = payload::load_payload(&efi_root, recovery_dir.as_deref()).await?;
    if let (Some(version), Some(found)) = (recovery, &recovery_payload) {
        if found.version != version.id() {
            return Err(AppError::new("RECOVERY_NOT_READY", "The cached recovery image belongs to another macOS release"));
        }
    }
    let mut sources = vec![efi.root.clone()];
    sources.extend(recovery_payload.as_ref().map(|r| r.dir.clone()));
    if sources.iter().any(|p| path_on_disk(&disk, p)) {
        return Err(AppError::new("SOURCE_ON_TARGET", "The files to copy are stored on the USB drive that would be erased")
            .with_suggestion("Choose a different USB drive."));
    }
    let needed = payload::required_capacity(&efi, recovery_payload.as_ref());
    if disk.size_bytes < needed {
        return Err(AppError::new(
            "DISK_TOO_SMALL",
            format!("{} is too small: {} MB are needed", disk.device_path, needed / 1_000_000),
        ));
    }
    let binding = FlashBinding {
        device: disk.device_path.clone(),
        disk_fingerprint: build_fingerprint(&disk),
        efi_path: efi.root.to_string_lossy().into_owned(),
        efi_state_hash: efi.tree_hash.clone(),
        recovery: recovery.map(|v| v.id().to_string()),
        payload_state_hash: recovery_payload.as_ref().map(|r| r.binding.clone()),
    };
    Ok(FlashState { disk, binding, efi, recovery_dir })
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
    ensure_can_write()?;
    let all = platform::list_disks().await?;
    let state = current_state(&all, &device, &efi_path, recovery, &paths).await?;
    let collisions = find_collisions(&state.binding.disk_fingerprint, &all, &state.disk.device_path);
    if !collisions.is_empty() {
        return Err(AppError::new(
            "DISK_AMBIGUOUS",
            format!("{} cannot be told apart from {}", state.disk.device_path, collisions.join(", ")),
        )
        .with_suggestion("Unplug the other identical drive before flashing."));
    }
    let issued = security.issue_token(&state.binding)?;
    info!(device = %state.disk.device_path, recovery = ?recovery, "Flash confirmation prepared");
    Ok(FlashConfirmation {
        token: issued.token,
        device: state.disk.device_path.clone(),
        expires_at: issued.expires_at,
        disk_display: disk_display(&state.disk),
        efi_hash: state.efi.tree_hash.clone(),
        recovery,
    })
}

/// Sends `flash:progress` events (throttled) and forwards progress to the
/// task registry in order.
struct FlashReporter {
    app: AppHandle,
    task_id: String,
    updates: tokio::sync::mpsc::UnboundedSender<(f64, String)>,
    last: Mutex<(String, Option<Instant>)>,
}

impl FlashReporter {
    fn report(&self, phase: &str, progress: f64, message: &str) {
        let mut last = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = last.0 != phase;
        let due = match last.1 {
            None => true,
            Some(at) => at.elapsed() >= PROGRESS_INTERVAL,
        };
        if !changed && !due && progress < 1.0 {
            return;
        }
        *last = (phase.to_string(), Some(Instant::now()));
        drop(last);
        self.emit(phase, progress, message, None);
        let _ = self.updates.send((progress, message.to_string()));
    }

    fn emit(&self, phase: &str, progress: f64, message: &str, error: Option<String>) {
        let event = FlashProgress {
            task_id: self.task_id.clone(),
            phase: phase.to_string(),
            progress: progress.clamp(0.0, 1.0),
            message: message.to_string(),
            error,
        };
        if let Err(e) = self.app.emit("flash:progress", &event) {
            warn!("flash:progress could not be sent: {e}");
        }
    }

    fn last_phase(&self) -> String {
        self.last.lock().map(|l| l.0.clone()).unwrap_or_default()
    }
}

/// Erase `device`, write the EFI and (optionally) the cached recovery image.
/// Emits `flash:progress` events (task kind "usb-flash").
#[allow(clippy::too_many_arguments)]
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
    let _guard = FlashGuard::acquire()
        .ok_or_else(|| AppError::new("FLASH_IN_PROGRESS", "Another USB drive is being written").recoverable())?;
    if crate::commands::recovery::download_in_progress() {
        return Err(AppError::new("BUSY", "Wait for the recovery download to finish before writing the USB drive").recoverable());
    }
    // The confirmation is checked and consumed before any disk is touched.
    let claims = security.redeem_token(&token)?;
    ensure_can_write()?;
    let all = platform::list_disks().await?;
    let state = current_state(&all, &device, &efi_path, recovery, &paths).await?;
    claims.check_binding(&state.binding)?;
    // The platform code re-lists the disk and checks it against this right
    // before erasing it.
    let _confirmed = confirm_target(&state.binding.device, state.binding.disk_fingerprint.clone());

    let job = FlashJob {
        device: state.binding.device.clone(),
        efi_source: state.efi.root.clone(),
        recovery_source: state.recovery_dir.clone(),
        label: VOLUME_LABEL.to_string(),
    };
    drop(state);

    let registry: Arc<TaskRegistry> = Arc::clone(&task_registry);
    let (task_id, cancel) = registry.create("usb-flash").await;
    let (updates, mut receiver) = tokio::sync::mpsc::unbounded_channel::<(f64, String)>();
    let forwarder = {
        let registry = Arc::clone(&registry);
        let task_id = task_id.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((progress, message)) = receiver.recv().await {
                registry.update_progress(&task_id, progress, Some(message)).await;
            }
        })
    };
    let reporter = FlashReporter { app, task_id: task_id.clone(), updates, last: Mutex::new((String::new(), None)) };
    info!(task = %task_id, device = %job.device, recovery = ?recovery, "Flashing USB drive");
    let result = platform::flash(&job, &|phase, progress, message| reporter.report(phase, progress, message), &cancel).await;

    match &result {
        Ok(()) => {}
        Err(e) => {
            let phase = reporter.last_phase();
            error!(task = %task_id, phase = %phase, code = %e.code, "Flash failed: {}", e.message);
            reporter.emit("failed", 0.0, &format!("Failed during {phase}"), Some(e.message.clone()));
        }
    }
    drop(reporter);
    let _ = forwarder.await;
    match &result {
        Ok(()) => registry.complete(&task_id).await,
        // cancel() already recorded the cancellation.
        Err(e) if e.code == "TASK_CANCELLED" => {}
        Err(e) => registry.fail(&task_id, &e.message).await,
    }
    result
}
