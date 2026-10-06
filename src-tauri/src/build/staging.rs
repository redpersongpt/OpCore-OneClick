//! Build directory lifecycle: chronologically sortable build ids, a staging
//! directory that either becomes the build directory with one rename or is
//! removed, atomic file writes and symlink-free copies.

use std::io::Write;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::AppError;

/// Prefix of in-progress build directories inside `builds/`.
pub const STAGING_PREFIX: &str = ".staging-";

/// New id: UTC timestamp plus a random suffix ("20261006-081512-1a2b3c4d"),
/// so plain string order is creation order.
pub fn new_build_id() -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"), &suffix[..8])
}

/// True for ids produced by [`new_build_id`].
pub fn is_build_id(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 24
        && b[8] == b'-'
        && b[15] == b'-'
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[16..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}

/// A directory filled by a build and renamed to `builds/<id>` on success.
/// Dropping it without [`commit`](Self::commit) deletes it.
#[derive(Debug)]
pub struct StagingDir {
    path: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl StagingDir {
    pub fn create(builds_dir: &Path, build_id: &str) -> Result<Self, AppError> {
        if build_id.is_empty() || build_id.starts_with('.') || build_id.contains(['/', '\\', ':', '\0']) {
            return Err(AppError::new("INVALID_BUILD_ID", format!("'{build_id}' is not a build id")));
        }
        std::fs::create_dir_all(builds_dir)?;
        let target = builds_dir.join(build_id);
        if target.exists() {
            return Err(AppError::new("BUILD_EXISTS", format!("{} already exists", target.display())));
        }
        let path = builds_dir.join(format!("{STAGING_PREFIX}{build_id}"));
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        std::fs::create_dir(&path)?;
        Ok(Self { path, target, committed: false })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Final location once committed.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Move the staging directory to its final name.
    pub fn commit(mut self) -> Result<PathBuf, AppError> {
        rename_with_retry(&self.path, &self.target).map_err(|e| {
            AppError::new("BUILD_SAVE_FAILED", format!("Could not move the build into {}: {e}", self.target.display()))
        })?;
        self.committed = true;
        Ok(self.target.clone())
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            if self.path.exists() {
                tracing::warn!(dir = %self.path.display(), error = %e, "could not remove the unfinished build");
            }
        }
    }
}

/// Remove staging directories left behind by a crash. Call only while no
/// build is running.
pub fn clean_stale_staging(builds_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(builds_dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(STAGING_PREFIX) {
            if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!(dir = %entry.path().display(), error = %e, "could not remove a stale staging directory");
            }
        }
    }
}

/// On Windows a virus scanner still reading a fresh file blocks the rename
/// of its folder for a moment.
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt: u64 = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 5 && cfg!(windows) && !to.exists() => {
                attempt += 1;
                tracing::debug!(from = %from.display(), error = %e, attempt, "rename failed, retrying");
                std::thread::sleep(std::time::Duration::from_millis(150 * attempt));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Write `bytes` to a temporary sibling, flush it to disk and rename it over
/// `path`, so readers never see a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::new("INVALID_PATH", format!("{} is not a file path", path.display())))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&parent)?;
    let tmp = parent.join(format!(".{name}.{}.tmp", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rename_with_retry(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| AppError::new("IO_ERROR", format!("Could not write {}: {e}", path.display())))
}

/// Copy one regular file, creating the destination folder.
pub fn copy_file(src: &Path, dest: &Path) -> Result<(), AppError> {
    let meta =
        std::fs::symlink_metadata(src).map_err(|e| AppError::new("IO_ERROR", format!("{}: {e}", src.display())))?;
    if !meta.is_file() {
        return Err(AppError::new("IO_ERROR", format!("{} is not a regular file", src.display())));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dest)
        .map_err(|e| AppError::new("IO_ERROR", format!("Could not copy {}: {e}", src.display())))?;
    Ok(())
}

/// Recursive copy that skips symlinks and Finder metadata (`.DS_Store`,
/// `._*`). Returns the number of files copied.
pub fn copy_tree(src: &Path, dest: &Path) -> Result<usize, AppError> {
    std::fs::create_dir_all(dest)?;
    let mut files = 0;
    for item in WalkDir::new(src).follow_links(false).min_depth(1) {
        let item = item.map_err(|e| AppError::new("IO_ERROR", format!("Cannot read {}: {e}", src.display())))?;
        let rel = item.path().strip_prefix(src).map_err(|e| AppError::new("IO_ERROR", e.to_string()))?;
        let name = item.file_name().to_string_lossy();
        if name == ".DS_Store" || name.starts_with("._") {
            continue;
        }
        let target = dest.join(rel);
        let kind = item.file_type();
        if kind.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if kind.is_file() {
            std::fs::copy(item.path(), &target)
                .map_err(|e| AppError::new("IO_ERROR", format!("Could not copy {}: {e}", item.path().display())))?;
            files += 1;
        } else {
            tracing::debug!(path = %item.path().display(), "skipping a link while copying");
        }
    }
    Ok(files)
}

/// Entry of `dir` named `name` with its on-disk spelling: the exact name
/// when present, otherwise the single entry that matches ignoring ASCII case.
pub fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let names: Vec<String> =
        std::fs::read_dir(dir).ok()?.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    if names.iter().any(|n| n == name) {
        return Some(dir.join(name));
    }
    let mut matches = names.iter().filter(|n| n.eq_ignore_ascii_case(name));
    let first = matches.next()?;
    matches.next().is_none().then(|| dir.join(first))
}

#[cfg(test)]
pub(crate) mod test_dir {
    use std::path::{Path, PathBuf};

    /// Temporary directory removed on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-{tag}-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_dir::TempDir;
    use super::*;

    #[test]
    fn build_ids_sort_and_validate() {
        let id = new_build_id();
        assert!(is_build_id(&id), "{id}");
        assert!(!is_build_id("20261006-081512-1A2B3C4D"));
        assert!(!is_build_id("2026100-0815120-1a2b3c4d"));
        assert!(!is_build_id(".staging-20261006-081512-1a2b3c4d"));
        assert!(!is_build_id("0c0c3b0e-7a35-4b67-9d25-0ad8f7b1d4b2"));
        assert!("20261006-081512-ffffffff" < "20261006-081513-00000000");
    }

    #[test]
    fn staging_commits_with_one_rename() {
        let tmp = TempDir::new("staging");
        let builds = tmp.path().join("builds");
        let id = new_build_id();
        let staging = StagingDir::create(&builds, &id).unwrap();
        assert!(staging.path().file_name().unwrap().to_string_lossy().starts_with(STAGING_PREFIX));
        std::fs::create_dir_all(staging.path().join("EFI/OC")).unwrap();
        std::fs::write(staging.path().join("EFI/OC/config.plist"), b"x").unwrap();
        let target = staging.target().to_path_buf();
        assert!(!target.exists());
        let done = staging.commit().unwrap();
        assert_eq!(done, target);
        assert!(done.join("EFI/OC/config.plist").is_file());
        let names: Vec<String> = std::fs::read_dir(&builds)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![id.clone()]);
        // The same id cannot be built twice.
        assert_eq!(StagingDir::create(&builds, &id).unwrap_err().code, "BUILD_EXISTS");
    }

    #[test]
    fn unfinished_staging_is_removed() {
        let tmp = TempDir::new("staging");
        let builds = tmp.path().join("builds");
        let path = {
            let staging = StagingDir::create(&builds, &new_build_id()).unwrap();
            std::fs::create_dir_all(staging.path().join("EFI/OC/Kexts/Lilu.kext")).unwrap();
            staging.path().to_path_buf()
        };
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&builds).unwrap().count(), 0);
    }

    #[test]
    fn stale_staging_directories_are_cleaned() {
        let tmp = TempDir::new("staging");
        let builds = tmp.path();
        std::fs::create_dir_all(builds.join(".staging-20200101-000000-00000000/EFI")).unwrap();
        std::fs::create_dir_all(builds.join("20200101-000000-00000000/EFI")).unwrap();
        clean_stale_staging(builds);
        assert!(!builds.join(".staging-20200101-000000-00000000").exists());
        assert!(builds.join("20200101-000000-00000000").exists());
    }

    #[test]
    fn invalid_build_ids_are_refused() {
        let tmp = TempDir::new("staging");
        for id in ["", "../x", ".hidden", "a/b", "C:x"] {
            assert_eq!(StagingDir::create(tmp.path(), id).unwrap_err().code, "INVALID_BUILD_ID", "{id}");
        }
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let tmp = TempDir::new("atomic");
        let file = tmp.path().join("sub/state.json");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path().join("sub")).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1);
        // A directory in the way makes the write fail without a stray temp file.
        let dir = tmp.path().join("busy");
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        assert!(write_atomic(&dir, b"x").is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 2);
    }

    #[test]
    fn tree_copy_skips_metadata_and_links() {
        let tmp = TempDir::new("copy");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("OC/Kexts/Lilu.kext/Contents")).unwrap();
        std::fs::write(src.join("OC/Kexts/Lilu.kext/Contents/Info.plist"), b"plist").unwrap();
        std::fs::write(src.join("OC/.DS_Store"), b"junk").unwrap();
        std::fs::write(src.join("OC/._config.plist"), b"junk").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", src.join("OC/link")).unwrap();
        let dest = tmp.path().join("dest");
        assert_eq!(copy_tree(&src, &dest).unwrap(), 1);
        assert!(dest.join("OC/Kexts/Lilu.kext/Contents/Info.plist").is_file());
        assert!(!dest.join("OC/.DS_Store").exists());
        assert!(!dest.join("OC/._config.plist").exists());
        assert!(std::fs::symlink_metadata(dest.join("OC/link")).is_err());
    }

    #[test]
    fn case_insensitive_lookup() {
        let tmp = TempDir::new("ci");
        std::fs::write(tmp.path().join("OpenRuntime.efi"), b"x").unwrap();
        assert_eq!(find_ci(tmp.path(), "openruntime.EFI").unwrap(), tmp.path().join("OpenRuntime.efi"));
        assert!(find_ci(tmp.path(), "OpenCanopy.efi").is_none());
        assert!(copy_file(&tmp.path().join("missing.efi"), &tmp.path().join("out.efi")).is_err());
    }
}
