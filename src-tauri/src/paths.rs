//! Well-known directories used by the app. Created once at startup and
//! managed as Tauri state.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    /// Persistent app data (state, builds).
    pub data: PathBuf,
    /// Download cache (content addressed by SHA-256) — safe to delete.
    pub cache: PathBuf,
    /// Finished EFI builds: builds/<build-id>/EFI.
    pub builds: PathBuf,
    /// Recovery images: recovery/<version-id>/com.apple.recovery.boot.
    pub recovery: PathBuf,
    /// Dumped ACPI tables of this machine.
    pub acpi: PathBuf,
    /// Scratch space for extraction (cleared at startup).
    pub work: PathBuf,
}

impl AppPaths {
    pub fn new(data: &Path, cache: &Path) -> Self {
        let paths = Self {
            data: data.to_path_buf(),
            cache: cache.join("downloads"),
            builds: data.join("builds"),
            recovery: data.join("recovery"),
            acpi: data.join("acpi"),
            work: cache.join("work"),
        };
        for dir in [&paths.data, &paths.cache, &paths.builds, &paths.recovery, &paths.acpi, &paths.work] {
            let _ = std::fs::create_dir_all(dir);
        }
        paths
    }

    pub fn recovery_dir(&self, version_id: &str) -> PathBuf {
        self.recovery.join(version_id)
    }
}
