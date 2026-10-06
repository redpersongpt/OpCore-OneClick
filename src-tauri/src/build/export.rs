//! Copy a built EFI folder somewhere the user picked (another disk, a mounted
//! ESP, a USB stick). An existing non-empty `EFI` is never overwritten: the
//! copy goes to `EFI-2`, `EFI-3`, ... instead. The copy is written under a
//! temporary name and renamed when complete.

use std::path::{Path, PathBuf};

use crate::error::AppError;

use super::staging::{copy_tree, find_ci};

const MAX_SUFFIX: u32 = 99;

/// The `EFI` folder of a build directory, or `path` itself when it is one.
pub fn source_efi(path: &Path) -> Result<PathBuf, AppError> {
    let candidate = match find_ci(path, "EFI").filter(|p| p.is_dir()) {
        Some(efi) => efi,
        None => path.to_path_buf(),
    };
    if find_ci(&candidate, "OC").is_some_and(|oc| oc.is_dir()) {
        Ok(candidate)
    } else {
        Err(AppError::new("EFI_NOT_FOUND", format!("No EFI folder in {}", path.display()))
            .recoverable()
            .with_suggestion("Build the EFI first."))
    }
}

/// Copy the EFI folder of `source` into `destination` and return the new
/// folder (`destination/EFI`, or a numbered sibling when that is taken).
pub fn export_efi(source: &Path, destination: &Path) -> Result<PathBuf, AppError> {
    let src = source_efi(source)?.canonicalize()?;
    let dest_meta = std::fs::metadata(destination).map_err(|_| {
        AppError::new("DESTINATION_NOT_FOUND", format!("{} does not exist", destination.display())).recoverable()
    })?;
    if !dest_meta.is_dir() {
        return Err(AppError::new("DESTINATION_NOT_A_FOLDER", format!("{} is not a folder", destination.display()))
            .recoverable());
    }
    let dest = destination.canonicalize()?;
    if dest.starts_with(&src) {
        return Err(AppError::new(
            "DESTINATION_INSIDE_SOURCE",
            "The destination is inside the EFI folder being copied",
        )
        .recoverable()
        .with_suggestion("Pick a folder outside the build."));
    }

    let target = free_target(&dest)?;
    let staging = dest.join(format!(".EFI-export-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let outcome = copy_tree(&src, &staging).and_then(|_| {
        if target.is_dir() {
            // Only an empty folder gets here (see free_target).
            std::fs::remove_dir(&target)?;
        }
        std::fs::rename(&staging, &target)
            .map_err(|e| AppError::new("EXPORT_FAILED", format!("Could not create {}: {e}", target.display())))
    });
    if let Err(e) = outcome {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    tracing::info!(from = %src.display(), to = %target.display(), "EFI exported");
    Ok(plain_path(target))
}

/// `canonicalize` returns verbatim paths on Windows (`\\?\E:\EFI`); give the
/// user the familiar form (`E:\EFI`, `\\server\share\EFI`).
fn plain_path(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else { return path };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// `EFI` when free (absent or an empty folder), else the first free `EFI-n`.
fn free_target(dest: &Path) -> Result<PathBuf, AppError> {
    let usable = |p: &Path| match std::fs::symlink_metadata(p) {
        Err(_) => true,
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|mut rd| rd.next().is_none()).unwrap_or(false),
        Ok(_) => false,
    };
    // A case-insensitive volume reports "efi" as taken too.
    let efi = find_ci(dest, "EFI").unwrap_or_else(|| dest.join("EFI"));
    if usable(&efi) {
        return Ok(dest.join("EFI"));
    }
    for n in 2..=MAX_SUFFIX {
        let name = format!("EFI-{n}");
        let candidate = find_ci(dest, &name).unwrap_or_else(|| dest.join(&name));
        if usable(&candidate) {
            return Ok(dest.join(name));
        }
    }
    Err(AppError::new("DESTINATION_FULL", format!("{} already has EFI to EFI-{MAX_SUFFIX}", dest.display()))
        .recoverable())
}

#[cfg(test)]
mod tests {
    use super::super::staging::test_dir::TempDir;
    use super::*;

    fn build(root: &Path) -> PathBuf {
        let dir = root.join("20261006-120000-00000000");
        std::fs::create_dir_all(dir.join("EFI/OC/Kexts/Lilu.kext")).unwrap();
        std::fs::create_dir_all(dir.join("EFI/BOOT")).unwrap();
        std::fs::write(dir.join("EFI/OC/config.plist"), b"cfg").unwrap();
        std::fs::write(dir.join("EFI/BOOT/BOOTx64.efi"), b"boot").unwrap();
        std::fs::write(dir.join("build.json"), b"{}").unwrap();
        dir
    }

    #[test]
    fn exports_without_overwriting() {
        let tmp = TempDir::new("export");
        let src = build(tmp.path());
        let dest = tmp.path().join("usb");
        std::fs::create_dir_all(&dest).unwrap();

        let first = export_efi(&src, &dest).unwrap();
        assert_eq!(first.file_name().unwrap(), "EFI");
        assert_eq!(std::fs::read(first.join("OC/config.plist")).unwrap(), b"cfg");
        assert!(!first.join("build.json").exists());

        // The EFI folder itself works as a source too; the second copy gets a suffix.
        let second = export_efi(&src.join("EFI"), &dest).unwrap();
        assert_eq!(second.file_name().unwrap(), "EFI-2");
        assert_eq!(std::fs::read(first.join("OC/config.plist")).unwrap(), b"cfg");

        // An empty EFI folder is reused.
        let other = tmp.path().join("other");
        std::fs::create_dir_all(other.join("EFI")).unwrap();
        assert_eq!(export_efi(&src, &other).unwrap().file_name().unwrap(), "EFI");

        let leftovers: Vec<String> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn verbatim_prefixes_are_removed_for_display() {
        assert_eq!(plain_path(PathBuf::from(r"\\?\E:\EFI")), PathBuf::from(r"E:\EFI"));
        assert_eq!(plain_path(PathBuf::from(r"\\?\UNC\nas\share\EFI")), PathBuf::from(r"\\nas\share\EFI"));
        assert_eq!(plain_path(PathBuf::from(r"\\?\Volume{1234}\EFI")), PathBuf::from(r"\\?\Volume{1234}\EFI"));
        assert_eq!(plain_path(PathBuf::from("/Volumes/USB/EFI")), PathBuf::from("/Volumes/USB/EFI"));
    }

    #[test]
    fn refuses_bad_destinations() {
        let tmp = TempDir::new("export");
        let src = build(tmp.path());
        assert_eq!(export_efi(&src, &tmp.path().join("missing")).unwrap_err().code, "DESTINATION_NOT_FOUND");
        assert_eq!(export_efi(&src, &src.join("build.json")).unwrap_err().code, "DESTINATION_NOT_A_FOLDER");
        assert_eq!(export_efi(&src, &src.join("EFI/OC")).unwrap_err().code, "DESTINATION_INSIDE_SOURCE");
        assert_eq!(export_efi(&tmp.path().join("nothing"), tmp.path()).unwrap_err().code, "EFI_NOT_FOUND");
    }
}
