//! Compatibility, planning, EFI build/validation/export commands.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::build::progress::{BuildProgress, Phase};
use crate::build::{self, blocking, retention, validate, BuildEnv, ProgressSink};
use crate::contracts::{BuildResult, CompatibilityReport, ValidationResult};
use crate::domain::model::{BiosSetting, BuildOptions, BuildPlan, HardwareProfile, MacOsVersion};
use crate::domain::{bios, compatibility, planner};
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::services::http::Downloader;
use crate::services::ocvalidate;
use crate::tasks::cancellation::CancellationToken;
use crate::tasks::registry::TaskRegistry;

static BUILDING: AtomicBool = AtomicBool::new(false);

/// Held while an EFI build runs.
struct BuildGuard;

impl BuildGuard {
    fn acquire() -> Option<Self> {
        BUILDING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| BuildGuard)
    }
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        BUILDING.store(false, Ordering::SeqCst);
    }
}

/// True while an EFI build is running (its staging directory is in `builds/`).
pub fn build_in_progress() -> bool {
    BUILDING.load(Ordering::SeqCst)
}

#[tauri::command]
pub async fn check_compatibility(
    profile: HardwareProfile,
    target: Option<MacOsVersion>,
) -> Result<CompatibilityReport, AppError> {
    blocking(move || Ok(compatibility::assess(&profile, target))).await
}

/// Preview the plan (SMBIOS, kexts, SSDTs, quirks, boot-args) without downloading anything.
#[tauri::command]
pub async fn plan_build(profile: HardwareProfile, options: BuildOptions) -> Result<BuildPlan, AppError> {
    blocking(move || planner::plan(&profile, &options)).await
}

#[tauri::command]
pub async fn get_bios_settings(profile: HardwareProfile, target: MacOsVersion) -> Result<Vec<BiosSetting>, AppError> {
    blocking(move || Ok(bios::recommended_settings(&profile, target))).await
}

/// Sends build progress to the task registry without blocking the build.
struct TaskSink(tokio::sync::mpsc::UnboundedSender<BuildProgress>);

impl ProgressSink for TaskSink {
    fn report(&self, progress: BuildProgress) {
        let _ = self.0.send(progress);
    }
}

/// Full build (task kind "efi-build"): plan → fetch OpenCore/kexts/resources →
/// assemble EFI → generate config.plist from Sample.plist → ocvalidate.
/// Progress is reported through `TaskUpdate.detail` as `{phase, step, total}`
/// (plus `item`, `index`, `count`, `downloaded`, `size` while downloading).
#[tauri::command]
pub async fn build_efi(
    profile: HardwareProfile,
    options: BuildOptions,
    task_registry: State<'_, Arc<TaskRegistry>>,
    paths: State<'_, AppPaths>,
    _app: AppHandle,
) -> Result<BuildResult, AppError> {
    let _guard = BuildGuard::acquire()
        .ok_or_else(|| AppError::new("BUILD_IN_PROGRESS", "An EFI build is already running").recoverable())?;
    let registry: Arc<TaskRegistry> = Arc::clone(&task_registry);
    let (task_id, cancel) = registry.create("efi-build").await;
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<BuildProgress>();
    let forwarder = {
        let registry = Arc::clone(&registry);
        let task_id = task_id.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(p) = receiver.recv().await {
                if p.phase == Phase::Save {
                    // From here on the build exists; a cancel could no longer undo it.
                    registry.set_cancellable(&task_id, false).await;
                }
                registry.update_detail(&task_id, p.fraction, Some(p.message.clone()), p.detail()).await;
            }
        })
    };
    let sink = TaskSink(sender);
    tracing::info!(task = %task_id, target = options.target.id(), "EFI build requested");

    let result = build::guarded(async {
        sink.report(BuildProgress {
            phase: Phase::Plan,
            fraction: Phase::Plan.at(0.0),
            message: format!("Planning the EFI for {}", options.target.display_name()),
            item: None,
            index: None,
            bytes: None,
        });
        let plan = {
            let (profile, options) = (profile.clone(), options.clone());
            blocking(move || planner::plan(&profile, &options)).await?
        };
        cancel.check()?;
        let downloader = Downloader::new(paths.cache.clone())?;
        let env = BuildEnv {
            builds_dir: &paths.builds,
            work_dir: &paths.work,
            downloader: &downloader,
            cancel: &cancel,
            progress: &sink,
            // Old builds are pruned below, once the build is final.
            keep_builds: 0,
        };
        build::run(&env, &profile, &options, plan).await
    })
    .await;

    drop(sink);
    let _ = forwarder.await;
    let result = discard_if_cancelled(result, &cancel).await;
    if let Ok(build) = &result {
        // Never delete builds while one of them may be on its way to a USB drive.
        if !crate::commands::disk::flash_in_progress() {
            let (builds_dir, current) = (paths.builds.clone(), std::path::PathBuf::from(&build.efi_path));
            let removed = blocking(move || {
                Ok(retention::prune_builds(&builds_dir, retention::KEEP_BUILDS, Some(&current)).len())
            })
            .await
            .unwrap_or(0);
            if removed > 0 {
                tracing::info!(count = removed, "old builds removed");
            }
        }
    }
    match &result {
        Ok(build) => {
            tracing::info!(task = %task_id, build = %build.build_id, "EFI build completed");
            registry.complete(&task_id).await;
        }
        // cancel() already recorded the cancellation (or the watchdog its failure).
        Err(e) if e.code == "TASK_CANCELLED" => tracing::info!(task = %task_id, "EFI build cancelled"),
        Err(e) => {
            tracing::warn!(task = %task_id, code = %e.code, "EFI build failed: {}", e.message);
            registry.fail(&task_id, &e.message).await;
        }
    }
    result
}

/// A cancel that reached the registry while the build was being saved, before
/// the save phase made the task non-cancellable, leaves a finished build the
/// task already reports as cancelled. Remove it so both agree. Once every
/// progress update has been forwarded the token can no longer change.
async fn discard_if_cancelled(
    result: Result<BuildResult, AppError>,
    cancel: &CancellationToken,
) -> Result<BuildResult, AppError> {
    match result {
        Ok(build) if cancel.is_cancelled() => {
            let dir = std::path::PathBuf::from(&build.efi_path);
            tracing::info!(build = %build.build_id, "build finished after it was cancelled, removing it");
            let _ = blocking(move || {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    tracing::warn!(dir = %dir.display(), error = %e, "could not remove the cancelled build");
                }
                Ok(())
            })
            .await;
            Err(AppError::new("TASK_CANCELLED", "Operation was cancelled by user"))
        }
        other => other,
    }
}

/// Validate an EFI: a build directory (containing `EFI`), an `EFI` folder or
/// a config.plist. Uses the ocvalidate of the OpenCore release that built it
/// when that package is in the local cache.
#[tauri::command]
pub async fn validate_efi(path: String, paths: State<'_, AppPaths>) -> Result<ValidationResult, AppError> {
    let input = path.trim().to_string();
    if input.is_empty() {
        return Err(AppError::new("PATH_NOT_FOUND", "No path given").recoverable());
    }
    let target = blocking(move || validate::resolve_target(Path::new(&input))).await?;
    let downloader = Downloader::new(paths.cache.clone())?;
    let ocvalidate_bin = validate::locate_ocvalidate(target.build_dir.as_deref(), &downloader, &paths.work).await;
    Ok(match &target.efi_dir {
        Some(efi) => ocvalidate::validate_efi(efi, ocvalidate_bin.as_deref()).await,
        None => validate::validate_config_only(&target.config, ocvalidate_bin.as_deref()).await,
    })
}

/// Copy the built EFI folder to `destination` (a folder) and return the new
/// path (`destination/EFI`, or `EFI-2`... when a non-empty EFI is there).
#[tauri::command]
pub async fn export_efi(efi_path: String, destination: String) -> Result<String, AppError> {
    blocking(move || {
        build::export::export_efi(Path::new(efi_path.trim()), Path::new(destination.trim()))
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ValidationResult;
    use crate::domain::model::PlatformIdentity;
    use crate::domain::planner::empty_plan;

    fn finished(dir: &Path) -> BuildResult {
        BuildResult {
            build_id: "20261006-120000-00000000".into(),
            efi_path: dir.to_string_lossy().into_owned(),
            config_plist_path: String::new(),
            target: MacOsVersion::Sequoia,
            opencore_version: "1.0.8".into(),
            identity: PlatformIdentity {
                model: "iMac19,1".into(),
                serial: String::new(),
                mlb: String::new(),
                system_uuid: String::new(),
                rom: String::new(),
            },
            plan: empty_plan(MacOsVersion::Sequoia),
            kexts: vec![],
            ssdts: vec![],
            validation: ValidationResult { valid: true, ocvalidate_ran: false, ocvalidate_output: None, issues: vec![] },
            warnings: vec![],
        }
    }

    #[tokio::test]
    async fn a_late_cancel_removes_the_finished_build() {
        let root = std::env::temp_dir().join(format!("oneclick-efi-{}", uuid::Uuid::new_v4().simple()));
        let dir = root.join("20261006-120000-00000000");
        std::fs::create_dir_all(dir.join("EFI/OC")).unwrap();

        let token = CancellationToken::new();
        let kept = discard_if_cancelled(Ok(finished(&dir)), &token).await;
        assert!(kept.is_ok());
        assert!(dir.is_dir());

        token.cancel();
        let err = discard_if_cancelled(Ok(finished(&dir)), &token).await.unwrap_err();
        assert_eq!(err.code, "TASK_CANCELLED");
        assert!(!dir.exists(), "the build the task reports as cancelled is removed");

        let failed = discard_if_cancelled(Err(AppError::new("NETWORK_ERROR", "offline")), &token).await;
        assert_eq!(failed.unwrap_err().code, "NETWORK_ERROR");
        let _ = std::fs::remove_dir_all(&root);
    }
}
