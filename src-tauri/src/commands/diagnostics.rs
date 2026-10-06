//! Diagnostics and logging commands.

use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager};
use tracing::info;

use crate::error::AppError;

/// Session ID generated once at startup.
static SESSION_ID: Lazy<String> = Lazy::new(|| uuid::Uuid::new_v4().to_string());

#[tauri::command]
pub async fn log_get_session_id() -> Result<String, AppError> {
    Ok(SESSION_ID.clone())
}

#[tauri::command]
pub async fn log_get_tail(
    lines: Option<u32>,
    app: AppHandle,
) -> Result<String, AppError> {
    let line_count = lines.unwrap_or(100).min(10000);

    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| AppError::new("PATH_ERROR", format!("Cannot resolve log dir: {}", e)))?;

    // Find the most recent log file
    let log_file = find_latest_log_file(&log_dir)?;

    let content = tokio::task::spawn_blocking({
        let log_file = log_file.clone();
        let count = line_count;
        move || read_tail_lines(&log_file, count as usize)
    })
    .await
    .map_err(|e| AppError::new("TASK_ERROR", format!("Log read task failed: {}", e)))??;

    Ok(content)
}

#[tauri::command]
pub async fn save_support_log(
    path: String,
    app: AppHandle,
) -> Result<(), AppError> {
    info!(destination = %path, "Saving support log");

    let dest = std::path::Path::new(&path);

    // Ensure parent directory exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| AppError::new("PATH_ERROR", format!("Cannot resolve log dir: {}", e)))?;

    let mut output = String::new();

    // System info header
    output.push_str("=== OpCore-OneClick Support Log ===\n");
    output.push_str(&format!("Session ID: {}\n", *SESSION_ID));
    output.push_str(&format!("Timestamp: {}\n", chrono::Utc::now().to_rfc3339()));
    output.push_str(&format!("Platform: {}\n", std::env::consts::OS));
    output.push_str(&format!("Architecture: {}\n", std::env::consts::ARCH));
    output.push_str(&format!(
        "Hostname: {}\n",
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into())
    ));
    output.push_str(&format!("App Version: {}\n", env!("CARGO_PKG_VERSION")));
    output.push_str("\n=== Log Output ===\n\n");

    // Append all log files
    if log_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&log_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "log")
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let file_path = entry.path();
            output.push_str(&format!(
                "--- {} ---\n",
                file_path.file_name().unwrap_or_default().to_string_lossy()
            ));
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    output.push_str(&content);
                    if !content.ends_with('\n') {
                        output.push('\n');
                    }
                }
                Err(e) => {
                    output.push_str(&format!("[Error reading file: {}]\n", e));
                }
            }
            output.push('\n');
        }
    } else {
        output.push_str("[No log directory found]\n");
    }

    std::fs::write(dest, &output)?;

    info!(
        destination = %path,
        size = output.len(),
        "Support log saved"
    );

    Ok(())
}

#[tauri::command]
pub async fn clear_app_cache(paths: tauri::State<'_, crate::paths::AppPaths>) -> Result<(), AppError> {
    if crate::commands::recovery::download_in_progress() || crate::commands::disk::flash_in_progress() {
        return Err(AppError::new(
            "BUSY",
            "A recovery download or USB write is running. Wait for it to finish before clearing the cache.",
        )
        .recoverable());
    }

    for dir in [&paths.builds, &paths.recovery, &paths.cache, &paths.work] {
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| {
                AppError::new("IO_ERROR", format!("Failed to clear {}: {}", dir.display(), e))
            })?;
        }
        std::fs::create_dir_all(dir)?;
    }

    info!("Application cache cleared");
    Ok(())
}

/// Find the most recent .log file in a directory.
fn find_latest_log_file(log_dir: &std::path::Path) -> Result<std::path::PathBuf, AppError> {
    if !log_dir.exists() {
        return Err(AppError::new("LOG_NOT_FOUND", "Log directory does not exist"));
    }

    let mut entries: Vec<_> = std::fs::read_dir(log_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();

    if entries.is_empty() {
        return Err(AppError::new("LOG_NOT_FOUND", "No log files found"));
    }

    // Sort by modified time (newest first)
    entries.sort_by(|a, b| {
        let a_time = a.metadata().and_then(|m| m.modified()).ok();
        let b_time = b.metadata().and_then(|m| m.modified()).ok();
        b_time.cmp(&a_time)
    });

    Ok(entries[0].path())
}

/// Read the last N lines from a file.
fn read_tail_lines(path: &std::path::Path, n: usize) -> Result<String, AppError> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    if lines.len() <= n {
        Ok(content)
    } else {
        Ok(lines[lines.len() - n..].join("\n"))
    }
}
