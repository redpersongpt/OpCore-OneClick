//! Wizard state persisted across restarts (profile, target, identity, last
//! build, current step). Entries older than 7 days are discarded, writes are
//! atomic, and `efi_path` is only kept while it points at an existing build
//! under `builds/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::State;
use tokio::sync::{Mutex, RwLock};

use crate::build::{blocking, staging::write_atomic};
use crate::contracts::PersistedState;
use crate::error::AppError;

const STATE_FILE: &str = "app_state.json";
/// Saved state older than this is ignored.
const MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;
/// Clock skew tolerated for timestamps in the future.
const MAX_FUTURE_SECS: i64 = 24 * 60 * 60;
/// A state file larger than this is not read.
const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STEP_CHARS: usize = 64;

pub struct AppStateManager {
    state: RwLock<PersistedState>,
    path: PathBuf,
    builds_dir: PathBuf,
    /// Serialises writers so an older snapshot never replaces a newer one.
    write_lock: Mutex<()>,
}

impl AppStateManager {
    pub fn new(app_data_dir: PathBuf, builds_dir: PathBuf) -> Arc<Self> {
        let path = app_data_dir.join(STATE_FILE);
        let state = load(&path, &builds_dir, chrono::Utc::now().timestamp());
        Arc::new(Self { state: RwLock::new(state), path, builds_dir, write_lock: Mutex::new(()) })
    }

    async fn persist(&self) -> Result<(), AppError> {
        let _writer = self.write_lock.lock().await;
        let bytes = serde_json::to_vec_pretty(&*self.state.read().await)?;
        let path = self.path.clone();
        blocking(move || write_atomic(&path, &bytes)).await
    }
}

/// Read the saved state; anything unreadable, expired or dangling is dropped.
fn load(path: &Path, builds_dir: &Path, now: i64) -> PersistedState {
    let Ok(meta) = std::fs::metadata(path) else { return PersistedState::default() };
    if !meta.is_file() || meta.len() > MAX_STATE_BYTES {
        return PersistedState::default();
    }
    let parsed = std::fs::read(path).ok().and_then(|bytes| {
        serde_json::from_slice::<PersistedState>(&bytes)
            .map_err(|e| {
                tracing::warn!(error = %e, "saved app state is not readable, starting fresh");
            })
            .ok()
    });
    match parsed {
        Some(state) if is_fresh(state.timestamp, now) => sanitize(state, builds_dir),
        Some(_) => {
            tracing::info!("saved app state expired");
            PersistedState::default()
        }
        None => PersistedState::default(),
    }
}

fn is_fresh(timestamp: Option<i64>, now: i64) -> bool {
    timestamp.is_some_and(|ts| ts <= now + MAX_FUTURE_SECS && now - ts <= MAX_AGE_SECS)
}

/// Keep `efi_path` only when it is an existing folder inside `builds/`, and
/// bound the step name.
fn sanitize(mut state: PersistedState, builds_dir: &Path) -> PersistedState {
    if let Some(path) = state.efi_path.take() {
        let inside = match (Path::new(&path).canonicalize(), builds_dir.canonicalize()) {
            (Ok(p), Ok(builds)) => p.starts_with(&builds) && p != builds && p.is_dir(),
            _ => false,
        };
        if inside {
            state.efi_path = Some(path);
        } else {
            tracing::info!("saved EFI path is gone or outside the builds folder, dropping it");
        }
    }
    if let Some(step) = &state.current_step {
        if step.chars().count() > MAX_STEP_CHARS {
            state.current_step = None;
        }
    }
    state
}

#[tauri::command]
pub async fn get_persisted_state(manager: State<'_, Arc<AppStateManager>>) -> Result<PersistedState, AppError> {
    let mut state = manager.state.write().await;
    let now = chrono::Utc::now().timestamp();
    if state.timestamp.is_some() && !is_fresh(state.timestamp, now) {
        *state = PersistedState::default();
    } else {
        // The build may have been deleted (cache cleared, retention) since.
        *state = sanitize(state.clone(), &manager.builds_dir);
    }
    Ok(state.clone())
}

#[tauri::command]
pub async fn save_state(manager: State<'_, Arc<AppStateManager>>, state: PersistedState) -> Result<(), AppError> {
    let mut clean = sanitize(state, &manager.builds_dir);
    clean.timestamp = Some(chrono::Utc::now().timestamp());
    *manager.state.write().await = clean;
    manager.persist().await
}

#[tauri::command]
pub async fn clear_state(manager: State<'_, Arc<AppStateManager>>) -> Result<(), AppError> {
    *manager.state.write().await = PersistedState::default();
    manager.persist().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{HardwareProfile, MacOsVersion};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-state-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn saved(dir: &Path, state: &PersistedState) -> PathBuf {
        let path = dir.join(STATE_FILE);
        std::fs::write(&path, serde_json::to_vec(state).unwrap()).unwrap();
        path
    }

    #[test]
    fn fresh_state_is_restored_and_expired_dropped() {
        let tmp = TempDir::new();
        let builds = tmp.0.join("builds");
        let build = builds.join("20261006-120000-00000000");
        std::fs::create_dir_all(build.join("EFI")).unwrap();
        let now = 1_800_000_000;
        let state = PersistedState {
            current_step: Some("build".into()),
            profile: Some(HardwareProfile::default()),
            target: Some(MacOsVersion::Sequoia),
            identity: None,
            efi_path: Some(build.to_string_lossy().into_owned()),
            timestamp: Some(now - 3600),
        };
        let path = saved(&tmp.0, &state);
        let loaded = load(&path, &builds, now);
        assert_eq!(loaded.target, Some(MacOsVersion::Sequoia));
        assert_eq!(loaded.current_step.as_deref(), Some("build"));
        assert_eq!(loaded.efi_path, state.efi_path);

        assert!(load(&path, &builds, now + MAX_AGE_SECS + 3600).target.is_none(), "older than 7 days");
        let future = PersistedState { timestamp: Some(now + 3 * MAX_FUTURE_SECS), ..state.clone() };
        let path = saved(&tmp.0, &future);
        assert!(load(&path, &builds, now).target.is_none(), "timestamps far in the future are not trusted");
        let undated = PersistedState { timestamp: None, ..state };
        let path = saved(&tmp.0, &undated);
        assert!(load(&path, &builds, now).target.is_none());
    }

    #[test]
    fn efi_paths_outside_builds_are_dropped() {
        let tmp = TempDir::new();
        let builds = tmp.0.join("builds");
        std::fs::create_dir_all(&builds).unwrap();
        let outside = tmp.0.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        for path in [
            outside.to_string_lossy().into_owned(),
            builds.join("gone").to_string_lossy().into_owned(),
            builds.to_string_lossy().into_owned(),
        ] {
            let state = PersistedState { efi_path: Some(path.clone()), ..PersistedState::default() };
            assert_eq!(sanitize(state, &builds).efi_path, None, "{path}");
        }
        let long = PersistedState { current_step: Some("x".repeat(500)), ..PersistedState::default() };
        assert_eq!(sanitize(long, &builds).current_step, None);
    }

    #[test]
    fn unreadable_files_start_fresh() {
        let tmp = TempDir::new();
        let path = tmp.0.join(STATE_FILE);
        std::fs::write(&path, b"{ truncated").unwrap();
        assert!(load(&path, &tmp.0, 0).profile.is_none());
        assert!(load(&tmp.0.join("missing.json"), &tmp.0, 0).profile.is_none());
    }

    #[tokio::test]
    async fn persist_writes_atomically() {
        let tmp = TempDir::new();
        let manager = AppStateManager::new(tmp.0.clone(), tmp.0.join("builds"));
        *manager.state.write().await = PersistedState {
            target: Some(MacOsVersion::Tahoe),
            timestamp: Some(chrono::Utc::now().timestamp()),
            ..PersistedState::default()
        };
        manager.persist().await.unwrap();
        let again = AppStateManager::new(tmp.0.clone(), tmp.0.join("builds"));
        assert_eq!(again.state.read().await.target, Some(MacOsVersion::Tahoe));
        let names: Vec<String> =
            std::fs::read_dir(&tmp.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, [STATE_FILE]);
    }
}
