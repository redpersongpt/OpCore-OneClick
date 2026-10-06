//! EFI build pipeline: turns a `BuildPlan` into a finished, validated EFI
//! folder under `builds/<build-id>`.
//!
//! The pipeline is independent of Tauri: commands hand it the paths, a
//! downloader, a cancellation token and a progress sink. Everything is written
//! into a staging directory that becomes the build directory with a single
//! rename, so a failed or cancelled build never leaves a half-written EFI.

pub mod amd;
pub mod assemble;
pub mod export;
pub mod kexts;
pub mod manifest;
pub mod pipeline;
pub mod progress;
pub mod retention;
pub mod ssdt;
pub mod staging;
pub mod validate;

use crate::error::AppError;

pub use pipeline::{run, BuildEnv};
pub use progress::{BuildProgress, Phase, ProgressSink};

/// Run blocking file-system work off the async runtime. A panic in `job`
/// becomes an error instead of tearing down the caller.
pub async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(job).await.map_err(|e| {
        let what = if e.is_panic() { "stopped unexpectedly" } else { "was interrupted" };
        AppError::new("INTERNAL_ERROR", format!("A background step {what}: {e}"))
    })?
}

/// Await `work`, turning a panic inside it into an error, so a command always
/// answers the frontend and always finishes its task.
pub async fn guarded<T>(work: impl std::future::Future<Output = Result<T, AppError>>) -> Result<T, AppError> {
    use futures_util::FutureExt;
    match std::panic::AssertUnwindSafe(work).catch_unwind().await {
        Ok(result) => result,
        Err(panic) => {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            tracing::error!(panic = %what, "a step stopped unexpectedly");
            Err(AppError::new("INTERNAL_ERROR", format!("A step stopped unexpectedly: {what}")))
        }
    }
}

/// Prefix the message of a failed step with what was being done, keeping the
/// code, suggestion and context. Cancellation passes through unchanged.
pub fn while_doing(err: AppError, what: &str) -> AppError {
    if err.code == "TASK_CANCELLED" {
        return err;
    }
    AppError { message: format!("{what}: {}", err.message), ..err }
}

/// A single path component with the given extension (".efi", ".aml"): no
/// separators, no drive letters, not hidden.
pub fn is_plain_file_name(name: &str, extension: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.len() > extension.len()
        && lower.ends_with(&extension.to_ascii_lowercase())
        && !name.contains(['/', '\\', ':', '\0'])
        && !name.starts_with('.')
        && name.trim() == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_file_names() {
        assert!(is_plain_file_name("OpenRuntime.efi", ".efi"));
        assert!(is_plain_file_name("SSDT-PLUG.AML", ".aml"));
        assert!(!is_plain_file_name(".efi", ".efi"));
        assert!(!is_plain_file_name("../OpenRuntime.efi", ".efi"));
        assert!(!is_plain_file_name("Sub\\Driver.efi", ".efi"));
        assert!(!is_plain_file_name("C:Driver.efi", ".efi"));
        assert!(!is_plain_file_name(".hidden.efi", ".efi"));
        assert!(!is_plain_file_name("Driver.efi ", ".efi"));
        assert!(!is_plain_file_name("Driver.aml", ".efi"));
    }

    #[test]
    fn step_errors_keep_their_code() {
        let err = AppError::new("NETWORK_ERROR", "timed out").recoverable().with_suggestion("retry");
        let wrapped = while_doing(err, "Downloading Lilu");
        assert_eq!(wrapped.code, "NETWORK_ERROR");
        assert_eq!(wrapped.message, "Downloading Lilu: timed out");
        assert!(wrapped.recoverable);
        assert_eq!(wrapped.suggestion.as_deref(), Some("retry"));

        let cancelled = while_doing(AppError::new("TASK_CANCELLED", "cancelled"), "Downloading");
        assert_eq!(cancelled.message, "cancelled");
    }

    #[tokio::test]
    async fn blocking_turns_panics_into_errors() {
        let err = blocking::<()>(|| panic!("boom")).await.unwrap_err();
        assert_eq!(err.code, "INTERNAL_ERROR");
        assert_eq!(blocking(|| Ok(7)).await.unwrap(), 7);
    }

    #[tokio::test]
    async fn guarded_turns_async_panics_into_errors() {
        let staging = std::env::temp_dir().join(format!("oneclick-guarded-{}", uuid::Uuid::new_v4().simple()));
        let err = guarded::<()>(async {
            let dir = staging::StagingDir::create(&staging, &staging::new_build_id())?;
            tokio::task::yield_now().await;
            let _keep = &dir;
            panic!("not yet implemented: config");
        })
        .await
        .unwrap_err();
        assert_eq!(err.code, "INTERNAL_ERROR");
        assert!(err.message.contains("not yet implemented"), "{}", err.message);
        // The unwound build still removed its staging directory.
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&staging);
        assert_eq!(guarded(async { Ok(3) }).await.unwrap(), 3);
    }
}
