//! Keeps `builds/` (and the per-scan ACPI dump folders) from growing without
//! bound: only the newest few directories survive.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::staging::is_build_id;

/// Builds kept after a successful build, the new one included.
pub const KEEP_BUILDS: usize = 5;

/// Delete all but the newest `keep` builds in `builds_dir`. Only directories
/// named by `staging::new_build_id` and the UUID-named folders of older app
/// versions are considered; anything else is left alone, as is `protect`.
/// Returns the removed directories.
pub fn prune_builds(builds_dir: &Path, keep: usize, protect: Option<&Path>) -> Vec<PathBuf> {
    // (has a sortable id, id, modification time, path); legacy folders sort
    // before every current build and among themselves by age.
    let mut builds: Vec<(bool, String, SystemTime, PathBuf)> = list_dirs(builds_dir)
        .into_iter()
        .filter_map(|(name, path, modified)| {
            if is_build_id(&name) {
                Some((true, name, modified, path))
            } else if is_legacy_build_name(&name) {
                Some((false, String::new(), modified, path))
            } else {
                None
            }
        })
        .collect();
    builds.sort_by(|a, b| (b.0, &b.1, b.2).cmp(&(a.0, &a.1, a.2)));
    remove_beyond(builds.into_iter().map(|(_, _, _, p)| p).collect(), keep, protect)
}

/// Delete all but the newest `keep` directories of `parent` whose name starts
/// with `prefix` followed by a build id ("scan-20261006-081512-1a2b3c4d").
pub fn prune_prefixed(parent: &Path, prefix: &str, keep: usize, protect: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<(String, PathBuf)> = list_dirs(parent)
        .into_iter()
        .filter(|(name, _, _)| name.strip_prefix(prefix).is_some_and(is_build_id))
        .map(|(name, path, _)| (name, path))
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    remove_beyond(dirs.into_iter().map(|(_, p)| p).collect(), keep, protect)
}

/// `newest_first` minus the first `keep`, never touching `protect`.
fn remove_beyond(newest_first: Vec<PathBuf>, keep: usize, protect: Option<&Path>) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for path in newest_first.into_iter().skip(keep) {
        if protect.is_some_and(|p| same_dir(p, &path)) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => removed.push(path),
            Err(e) => tracing::warn!(dir = %path.display(), error = %e, "could not remove an old directory"),
        }
    }
    removed
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Real directories (not links) directly inside `parent`.
fn list_dirs(parent: &Path) -> Vec<(String, PathBuf, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(parent) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let meta = std::fs::symlink_metadata(e.path()).ok()?;
            if !meta.is_dir() {
                return None;
            }
            let name = e.file_name().into_string().ok()?;
            Some((name, e.path(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}

/// Builds of app versions before 5.1 were named by a v4 UUID.
fn is_legacy_build_name(name: &str) -> bool {
    uuid::Uuid::parse_str(name).is_ok() && name.len() == 36
}

#[cfg(test)]
mod tests {
    use super::super::staging::test_dir::TempDir;
    use super::*;

    fn mk(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(p.join("EFI")).unwrap();
        p
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn keeps_the_newest_builds() {
        let tmp = TempDir::new("retention");
        let dir = tmp.path();
        for i in 0..7 {
            mk(dir, &format!("2026100{i}-120000-0000000{i}"));
        }
        mk(dir, "0c0c3b0e-7a35-4b67-9d25-0ad8f7b1d4b2");
        mk(dir, "my-backup");
        mk(dir, ".staging-20261009-120000-00000009");
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();

        let removed = prune_builds(dir, 5, None);
        assert_eq!(removed.len(), 3);
        assert_eq!(
            names(dir),
            vec![
                ".staging-20261009-120000-00000009",
                "20261002-120000-00000002",
                "20261003-120000-00000003",
                "20261004-120000-00000004",
                "20261005-120000-00000005",
                "20261006-120000-00000006",
                "my-backup",
                "notes.txt",
            ]
        );
    }

    #[test]
    fn legacy_builds_go_first_and_protected_survives() {
        let tmp = TempDir::new("retention");
        let dir = tmp.path();
        let legacy = mk(dir, "0c0c3b0e-7a35-4b67-9d25-0ad8f7b1d4b2");
        let newest = mk(dir, "20261006-120000-0000000a");
        let removed = prune_builds(dir, 1, Some(&newest));
        assert_eq!(removed, vec![legacy]);
        assert!(newest.exists());

        // Even when `keep` is 0 the protected build stays.
        assert!(prune_builds(dir, 0, Some(&newest)).is_empty());
        assert!(newest.exists());
    }

    #[test]
    fn prefixed_dirs_are_pruned_by_id() {
        let tmp = TempDir::new("retention");
        let dir = tmp.path();
        mk(dir, "scan-20261001-000000-00000001");
        mk(dir, "scan-20261002-000000-00000002");
        let current = mk(dir, "scan-20261003-000000-00000003");
        mk(dir, "import-20261004-000000-00000004");
        mk(dir, "scan-latest");
        let removed = prune_prefixed(dir, "scan-", 2, Some(&current));
        assert_eq!(removed.len(), 1);
        assert_eq!(
            names(dir),
            vec![
                "import-20261004-000000-00000004",
                "scan-20261002-000000-00000002",
                "scan-20261003-000000-00000003",
                "scan-latest"
            ]
        );
    }

    #[test]
    fn missing_directory_is_fine() {
        let tmp = TempDir::new("retention");
        assert!(prune_builds(&tmp.path().join("none"), 5, None).is_empty());
    }
}
