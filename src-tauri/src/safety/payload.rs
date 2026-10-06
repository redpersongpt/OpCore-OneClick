//! What gets written to the USB drive and how we prove it arrived intact:
//! EFI folder validation and hashing, the verified recovery image, chunked
//! copy with progress, and read-back verification of the target volume.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::chunklist::hex_encode;
use crate::error::AppError;

/// Folder OpenCore scans for a recovery DMG at the root of each volume.
pub const RECOVERY_DIR_NAME: &str = "com.apple.recovery.boot";
/// Written next to the recovery folder once the DMG matched its chunklist.
pub const RECOVERY_MARKER: &str = "recovery.json";
/// Files without which OpenCore cannot start.
pub const EFI_REQUIRED: [&str; 3] = ["EFI/BOOT/BOOTx64.efi", "EFI/OC/OpenCore.efi", "EFI/OC/config.plist"];
/// FAT32 cannot store files of 4 GiB or more.
const FAT32_MAX_FILE: u64 = 4 * 1024 * 1024 * 1024 - 1;
const COPY_BUFFER: usize = 4 << 20;

#[derive(Debug, Clone)]
pub struct PayloadFile {
    /// Path relative to the volume root, `/`-separated ("EFI/OC/config.plist").
    pub rel: String,
    pub source: PathBuf,
    pub size: u64,
    /// SHA-256 for files that are re-hashed on the target.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EfiPayload {
    /// Directory that contains `EFI`.
    pub root: PathBuf,
    pub files: Vec<PayloadFile>,
    /// Sub-directories relative to the volume root (created even when empty).
    pub dirs: Vec<String>,
    pub total_bytes: u64,
    /// SHA-256 over every file path and content.
    pub tree_hash: String,
}

/// Written after the recovery DMG verified against its signed chunklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryMarker {
    pub version: String,
    /// Apple product id of the image (`AP`).
    pub product: String,
    /// Path part of the DMG URL; identifies the image across sessions.
    pub image_path: String,
    pub dmg_name: String,
    pub chunklist_name: String,
    pub dmg_size: u64,
    /// Modification time of the verified DMG (seconds, nanoseconds).
    pub dmg_modified: (u64, u32),
    pub chunklist_sha256: String,
    /// Unix milliseconds.
    pub verified_at: i64,
}

impl RecoveryMarker {
    pub fn load(version_dir: &Path) -> Option<Self> {
        let bytes = std::fs::read(version_dir.join(RECOVERY_MARKER)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Atomic write (temp file + rename).
    pub fn save(&self, version_dir: &Path) -> Result<(), AppError> {
        let bytes = serde_json::to_vec_pretty(self)?;
        write_atomic(&version_dir.join(RECOVERY_MARKER), &bytes)
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn modified_stamp(meta: &std::fs::Metadata) -> (u64, u32) {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0))
}

#[derive(Debug, Clone)]
pub struct RecoveryPayload {
    pub version: String,
    /// Source `com.apple.recovery.boot` folder.
    pub dir: PathBuf,
    pub dmg: PayloadFile,
    pub chunklist: PayloadFile,
    pub product: String,
    /// Identity of the verified files, bound into the flash confirmation.
    pub binding: String,
}

impl RecoveryPayload {
    pub fn files(&self) -> [&PayloadFile; 2] {
        [&self.chunklist, &self.dmg]
    }

    pub fn total_bytes(&self) -> u64 {
        self.dmg.size + self.chunklist.size
    }
}

fn sha256_file(path: &Path) -> Result<String, AppError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn rel_string(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Accept either the build directory (containing `EFI`) or the `EFI` folder.
pub fn resolve_efi_root(path: &Path) -> Result<PathBuf, AppError> {
    let candidate = if path.join("EFI").is_dir() {
        path.to_path_buf()
    } else if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("EFI")) && path.join("OC").is_dir() {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        return Err(AppError::new("EFI_NOT_FOUND", format!("No EFI folder in {}", path.display()))
            .with_suggestion("Build the EFI first, then flash."));
    };
    Ok(candidate.canonicalize()?)
}

/// Validate the EFI folder (required files present, no symlinks, no file too
/// big for FAT32) and hash it. Blocking; call from `spawn_blocking`.
pub fn inspect_efi(path: &Path) -> Result<EfiPayload, AppError> {
    let root = resolve_efi_root(path)?;
    // The walk below follows its root, so the EFI folder itself must not be a link.
    let efi_meta = std::fs::symlink_metadata(root.join("EFI"))?;
    if !efi_meta.is_dir() {
        return Err(AppError::new("EFI_INVALID", "The EFI folder is a link, not a folder"));
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in walkdir::WalkDir::new(root.join("EFI")).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|e| AppError::new("IO_ERROR", format!("Cannot read the EFI folder: {e}")))?;
        let rel_path = entry.path().strip_prefix(&root).map_err(|_| AppError::new("IO_ERROR", "EFI path escape"))?;
        let rel = rel_string(rel_path);
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(AppError::new("EFI_INVALID", format!("{rel} is a symbolic link; FAT32 cannot store it")));
        }
        if file_type.is_dir() {
            dirs.push(rel);
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name == ".DS_Store" || name.starts_with("._") {
            continue;
        }
        let size = entry.metadata().map_err(|e| AppError::new("IO_ERROR", e.to_string()))?.len();
        if size > FAT32_MAX_FILE {
            return Err(AppError::new("EFI_INVALID", format!("{rel} is larger than FAT32 allows")));
        }
        files.push(PayloadFile { rel, source: entry.path().to_path_buf(), size, sha256: None });
    }
    for required in EFI_REQUIRED {
        if !files.iter().any(|f| f.rel.eq_ignore_ascii_case(required)) {
            return Err(AppError::new("EFI_INCOMPLETE", format!("{required} is missing from the EFI folder"))
                .with_suggestion("Rebuild the EFI before flashing."));
        }
    }
    let mut tree = Sha256::new();
    let mut total = 0u64;
    for file in &mut files {
        let digest = sha256_file(&file.source)?;
        tree.update(file.rel.as_bytes());
        tree.update([0]);
        tree.update(file.size.to_le_bytes());
        tree.update(digest.as_bytes());
        tree.update([0]);
        total += file.size;
        if EFI_REQUIRED.iter().any(|r| r.eq_ignore_ascii_case(&file.rel)) {
            file.sha256 = Some(digest);
        }
    }
    Ok(EfiPayload { root, files, dirs, total_bytes: total, tree_hash: hex_encode(&tree.finalize()) })
}

/// Load the verified recovery image of `version_dir`
/// (`recovery/<id>/com.apple.recovery.boot` + `recovery.json`).
pub fn inspect_recovery(version_dir: &Path) -> Result<RecoveryPayload, AppError> {
    let not_ready = |why: &str| {
        AppError::new("RECOVERY_NOT_READY", format!("The macOS recovery image is not ready: {why}"))
            .with_suggestion("Download the recovery image again; a partial download resumes where it stopped.")
    };
    let marker = RecoveryMarker::load(version_dir).ok_or_else(|| not_ready("it was never verified"))?;
    let dir = version_dir.join(RECOVERY_DIR_NAME);
    let valid_name = |n: &str| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid_name(&marker.dmg_name) || !valid_name(&marker.chunklist_name) {
        return Err(not_ready("its file names are invalid"));
    }
    let dmg_path = dir.join(&marker.dmg_name);
    let chunklist_path = dir.join(&marker.chunklist_name);
    let dmg_meta = std::fs::metadata(&dmg_path).map_err(|_| not_ready("the DMG is missing"))?;
    if dmg_meta.len() != marker.dmg_size || modified_stamp(&dmg_meta) != marker.dmg_modified {
        return Err(not_ready("the DMG changed after it was verified"));
    }
    let chunklist_sha = sha256_file(&chunklist_path).map_err(|_| not_ready("the chunklist is missing"))?;
    if chunklist_sha != marker.chunklist_sha256 {
        return Err(not_ready("the chunklist changed after it was verified"));
    }
    let chunklist_size = std::fs::metadata(&chunklist_path)?.len();
    let dmg_count = std::fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".dmg"))
        .count();
    if dmg_count != 1 {
        return Err(not_ready("the recovery folder must contain exactly one DMG"));
    }
    let marker_bytes = std::fs::read(version_dir.join(RECOVERY_MARKER))?;
    let mut binding = Sha256::new();
    binding.update(marker.version.as_bytes());
    binding.update([0]);
    binding.update(&marker_bytes);
    binding.update([0]);
    binding.update(chunklist_sha.as_bytes());
    let binding = hex_encode(&binding.finalize());
    let rel = |name: &str| format!("{RECOVERY_DIR_NAME}/{name}");
    Ok(RecoveryPayload {
        version: marker.version.clone(),
        dmg: PayloadFile { rel: rel(&marker.dmg_name), source: dmg_path, size: marker.dmg_size, sha256: None },
        chunklist: PayloadFile {
            rel: rel(&marker.chunklist_name),
            source: chunklist_path,
            size: chunklist_size,
            sha256: Some(chunklist_sha),
        },
        dir,
        product: marker.product,
        binding,
    })
}

/// Inspect the EFI folder and (optionally) the recovery folder off the async
/// executor.
pub async fn load_payload(efi_source: &Path, recovery_source: Option<&Path>) -> Result<(EfiPayload, Option<RecoveryPayload>), AppError> {
    let efi_source = efi_source.to_path_buf();
    let recovery_source = recovery_source.map(Path::to_path_buf);
    tokio::task::spawn_blocking(move || {
        let efi = inspect_efi(&efi_source)?;
        let recovery = recovery_source.as_deref().map(inspect_recovery).transpose()?;
        Ok((efi, recovery))
    })
    .await
    .map_err(|e| AppError::new("INTERNAL_ERROR", e.to_string()))?
}

/// Bytes the target volume needs (files plus FAT overhead and slack).
pub fn required_capacity(efi: &EfiPayload, recovery: Option<&RecoveryPayload>) -> u64 {
    let files = efi.total_bytes + recovery.map(RecoveryPayload::total_bytes).unwrap_or(0);
    let count = efi.files.len() as u64 + 2;
    // Cluster rounding (up to 32 KiB per file) plus 64 MiB for FAT tables.
    files + count * 32 * 1024 + 64 * 1024 * 1024
}

fn target_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |path, part| path.join(part))
}

/// The volume ran out of space (ENOSPC, ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL).
pub fn is_disk_full(error: &std::io::Error) -> bool {
    let Some(code) = error.raw_os_error() else { return false };
    #[cfg(unix)]
    return code == libc::ENOSPC;
    #[cfg(windows)]
    return code == 112 || code == 39;
    #[allow(unreachable_code)]
    {
        let _ = code;
        false
    }
}

fn copy_error(rel: &str, err: std::io::Error) -> AppError {
    if is_disk_full(&err) {
        AppError::new("TARGET_FULL", format!("The USB drive ran out of space while writing {rel}"))
    } else {
        AppError::new("COPY_FAILED", format!("Writing {rel} to the USB drive failed: {err}"))
            .with_suggestion("Try another USB port or drive.")
    }
}

async fn copy_file(source: &Path, dest: &Path, rel: &str, mut on_bytes: impl FnMut(u64)) -> Result<(), AppError> {
    let mut reader = tokio::fs::File::open(source)
        .await
        .map_err(|e| AppError::new("SOURCE_MISSING", format!("Cannot read {}: {e}", source.display())))?;
    let mut writer = tokio::fs::File::create(dest).await.map_err(|e| copy_error(rel, e))?;
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        let read = reader.read(&mut buffer).await.map_err(|e| AppError::new("IO_ERROR", e.to_string()))?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).await.map_err(|e| copy_error(rel, e))?;
        on_bytes(read as u64);
    }
    writer.flush().await.map_err(|e| copy_error(rel, e))?;
    writer.sync_all().await.map_err(|e| copy_error(rel, e))?;
    Ok(())
}

/// Which part of the payload a copy progress report belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyStage {
    Efi,
    Recovery,
}

/// Copy the EFI folder and the recovery files to the mounted volume root.
/// `progress(stage, fraction_of_stage, message)`.
pub async fn copy_payload(
    target_root: &Path,
    efi: &EfiPayload,
    recovery: Option<&RecoveryPayload>,
    progress: &(dyn Fn(CopyStage, f64, &str) + Send + Sync),
) -> Result<(), AppError> {
    for dir in &efi.dirs {
        tokio::fs::create_dir_all(target_path(target_root, dir)).await.map_err(|e| copy_error(dir, e))?;
    }
    let total = efi.total_bytes.max(1) as f64;
    let mut done = 0u64;
    progress(CopyStage::Efi, 0.0, "Copying EFI folder");
    for file in &efi.files {
        let dest = target_path(target_root, &file.rel);
        copy_file(&file.source, &dest, &file.rel, |n| done += n).await?;
        progress(CopyStage::Efi, done as f64 / total, &file.rel);
    }
    if let Some(recovery) = recovery {
        let dir = target_path(target_root, RECOVERY_DIR_NAME);
        tokio::fs::create_dir_all(&dir).await.map_err(|e| copy_error(RECOVERY_DIR_NAME, e))?;
        let total = recovery.total_bytes().max(1) as f64;
        let mut done = 0u64;
        let mut last_report = 0u64;
        progress(CopyStage::Recovery, 0.0, "Copying macOS recovery");
        for file in recovery.files() {
            let dest = target_path(target_root, &file.rel);
            copy_file(&file.source, &dest, &file.rel, |n| {
                done += n;
                if done - last_report >= 16 << 20 {
                    last_report = done;
                    progress(CopyStage::Recovery, done as f64 / total, &file.rel);
                }
            })
            .await?;
        }
        progress(CopyStage::Recovery, 1.0, "Recovery copied");
    }
    Ok(())
}

/// What was found on the target volume.
#[derive(Debug, Default, Clone)]
pub struct TargetReport {
    /// Lower-cased relative path → size.
    pub sizes: HashMap<String, u64>,
    /// Lower-cased relative path → SHA-256.
    pub hashes: HashMap<String, String>,
    /// Number of `.dmg` files in the target recovery folder.
    pub dmg_count: Option<usize>,
}

/// Read back sizes of every payload file and hashes of the critical ones.
pub async fn read_target(target_root: &Path, efi: &EfiPayload, recovery: Option<&RecoveryPayload>) -> Result<TargetReport, AppError> {
    let mut report = TargetReport::default();
    let mut files: Vec<&PayloadFile> = efi.files.iter().collect();
    if let Some(recovery) = recovery {
        files.extend(recovery.files());
    }
    for file in files {
        let path = target_path(target_root, &file.rel);
        let Ok(meta) = tokio::fs::metadata(&path).await else { continue };
        report.sizes.insert(file.rel.to_lowercase(), meta.len());
        if file.sha256.is_some() {
            let path = path.clone();
            let digest = tokio::task::spawn_blocking(move || sha256_file(&path))
                .await
                .map_err(|e| AppError::new("INTERNAL_ERROR", e.to_string()))??;
            report.hashes.insert(file.rel.to_lowercase(), digest);
        }
    }
    if recovery.is_some() {
        let mut count = 0;
        // A missing folder is reported by `verify_target` as missing files.
        if let Ok(mut entries) = tokio::fs::read_dir(target_path(target_root, RECOVERY_DIR_NAME)).await {
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_name().to_string_lossy().to_lowercase().ends_with(".dmg") {
                    count += 1;
                }
            }
        }
        report.dmg_count = Some(count);
    }
    Ok(report)
}

/// Parse `HASH <sha256> <rel>`, `SIZE <bytes> <rel>` and `DMGCOUNT <n>` lines
/// printed by the Linux flash script.
pub fn parse_script_report(stdout: &str) -> TargetReport {
    let mut report = TargetReport::default();
    for line in stdout.lines() {
        let mut parts = line.trim().splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("HASH"), Some(hash), Some(rel)) if hash.len() == 64 => {
                report.hashes.insert(rel.trim().to_lowercase(), hash.to_lowercase());
            }
            (Some("SIZE"), Some(size), Some(rel)) => {
                if let Ok(size) = size.parse() {
                    report.sizes.insert(rel.trim().to_lowercase(), size);
                }
            }
            (Some("DMGCOUNT"), Some(count), None) => report.dmg_count = count.parse().ok(),
            _ => {}
        }
    }
    report
}

/// Compare the target with the source: every file present with the same
/// size, critical files with the same SHA-256, exactly one DMG.
pub fn verify_target(efi: &EfiPayload, recovery: Option<&RecoveryPayload>, report: &TargetReport) -> Result<(), AppError> {
    let fail = |what: String| {
        AppError::new("VERIFY_FAILED", format!("Verification of the USB drive failed: {what}"))
            .with_suggestion("The drive may be faulty or counterfeit. Try another USB drive.")
    };
    let mut files: Vec<&PayloadFile> = efi.files.iter().collect();
    if let Some(recovery) = recovery {
        files.extend(recovery.files());
    }
    for file in files {
        let key = file.rel.to_lowercase();
        match report.sizes.get(&key) {
            None => return Err(fail(format!("{} is missing", file.rel))),
            Some(size) if *size != file.size => {
                return Err(fail(format!("{} has {size} bytes instead of {}", file.rel, file.size)))
            }
            _ => {}
        }
        if let Some(expected) = &file.sha256 {
            if report.hashes.get(&key) != Some(expected) {
                return Err(fail(format!("{} does not match the source", file.rel)));
            }
        }
    }
    if recovery.is_some() && report.dmg_count != Some(1) {
        return Err(fail("the recovery folder must contain exactly one DMG".to_string()));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("opcore-test-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn write(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    pub(crate) fn make_efi(root: &Path) {
        write(&root.join("EFI/BOOT/BOOTx64.efi"), b"boot");
        write(&root.join("EFI/OC/OpenCore.efi"), b"opencore");
        write(&root.join("EFI/OC/config.plist"), b"<plist/>");
        write(&root.join("EFI/OC/Drivers/OpenRuntime.efi"), b"runtime");
        write(&root.join("EFI/OC/Kexts/Lilu.kext/Contents/Info.plist"), b"lilu");
        std::fs::create_dir_all(root.join("EFI/OC/Tools")).unwrap();
    }

    pub(crate) fn make_recovery(version_dir: &Path) -> RecoveryMarker {
        let dir = version_dir.join(RECOVERY_DIR_NAME);
        write(&dir.join("BaseSystem.dmg"), &[7u8; 3000]);
        write(&dir.join("BaseSystem.chunklist"), b"CNKL-test");
        let meta = std::fs::metadata(dir.join("BaseSystem.dmg")).unwrap();
        let marker = RecoveryMarker {
            version: "26".into(),
            product: "140-93589".into(),
            image_path: "/content/downloads/31/41/140-93589/x/RecoveryImage/BaseSystem.dmg".into(),
            dmg_name: "BaseSystem.dmg".into(),
            chunklist_name: "BaseSystem.chunklist".into(),
            dmg_size: 3000,
            dmg_modified: modified_stamp(&meta),
            chunklist_sha256: sha256_file(&dir.join("BaseSystem.chunklist")).unwrap(),
            verified_at: 1,
        };
        marker.save(version_dir).unwrap();
        marker
    }

    #[test]
    fn efi_is_validated_and_hashed() {
        let tmp = TempDir::new();
        make_efi(&tmp.0);
        std::fs::write(tmp.0.join("EFI/OC/.DS_Store"), b"junk").unwrap();
        let efi = inspect_efi(&tmp.0).unwrap();
        assert_eq!(efi.files.len(), 5);
        assert!(efi.dirs.contains(&"EFI/OC/Tools".to_string()));
        assert_eq!(efi.total_bytes, 4 + 8 + 8 + 7 + 4);
        let boot = efi.files.iter().find(|f| f.rel == "EFI/BOOT/BOOTx64.efi").unwrap();
        assert!(boot.sha256.is_some());
        // Passing the EFI folder itself resolves to the same root and hash.
        let again = inspect_efi(&tmp.0.join("EFI")).unwrap();
        assert_eq!(again.tree_hash, efi.tree_hash);
        // Any content change changes the hash.
        std::fs::write(tmp.0.join("EFI/OC/Kexts/Lilu.kext/Contents/Info.plist"), b"LILU").unwrap();
        assert_ne!(inspect_efi(&tmp.0).unwrap().tree_hash, efi.tree_hash);
    }

    #[test]
    fn incomplete_or_missing_efi_is_rejected() {
        let tmp = TempDir::new();
        assert_eq!(inspect_efi(&tmp.0).unwrap_err().code, "EFI_NOT_FOUND");
        make_efi(&tmp.0);
        std::fs::remove_file(tmp.0.join("EFI/OC/OpenCore.efi")).unwrap();
        assert_eq!(inspect_efi(&tmp.0).unwrap_err().code, "EFI_INCOMPLETE");
        // A plain file is never accepted as an EFI source.
        assert!(inspect_efi(&tmp.0.join("EFI/OC/config.plist")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_rejected() {
        let tmp = TempDir::new();
        make_efi(&tmp.0);
        std::os::unix::fs::symlink("/etc/hosts", tmp.0.join("EFI/OC/link")).unwrap();
        assert_eq!(inspect_efi(&tmp.0).unwrap_err().code, "EFI_INVALID");

        // A build folder whose EFI is a link to somewhere else.
        let real = TempDir::new();
        make_efi(&real.0);
        let build = TempDir::new();
        std::os::unix::fs::symlink(real.0.join("EFI"), build.0.join("EFI")).unwrap();
        assert_eq!(inspect_efi(&build.0).unwrap_err().code, "EFI_INVALID");
    }

    #[test]
    fn recovery_requires_a_matching_marker() {
        let tmp = TempDir::new();
        assert_eq!(inspect_recovery(&tmp.0).unwrap_err().code, "RECOVERY_NOT_READY");
        make_recovery(&tmp.0);
        let recovery = inspect_recovery(&tmp.0).unwrap();
        assert_eq!(recovery.dmg.rel, "com.apple.recovery.boot/BaseSystem.dmg");
        assert_eq!(recovery.total_bytes(), 3000 + 9);
        let binding = recovery.binding.clone();
        assert_eq!(inspect_recovery(&tmp.0).unwrap().binding, binding);

        // A changed chunklist invalidates the cache.
        std::fs::write(tmp.0.join(RECOVERY_DIR_NAME).join("BaseSystem.chunklist"), b"CNKL-other").unwrap();
        assert!(inspect_recovery(&tmp.0).is_err());
        make_recovery(&tmp.0);
        // A second DMG would confuse OpenCore.
        std::fs::write(tmp.0.join(RECOVERY_DIR_NAME).join("Other.dmg"), b"x").unwrap();
        assert!(inspect_recovery(&tmp.0).is_err());
    }

    #[tokio::test]
    async fn copy_then_verify_round_trip() {
        let src = TempDir::new();
        let target = TempDir::new();
        make_efi(&src.0);
        make_recovery(&src.0.join("rec"));
        let efi = inspect_efi(&src.0).unwrap();
        let recovery = inspect_recovery(&src.0.join("rec")).unwrap();
        let stages = std::sync::Mutex::new(Vec::new());
        copy_payload(&target.0, &efi, Some(&recovery), &|stage, fraction, _| {
            stages.lock().unwrap().push((stage, fraction));
        })
        .await
        .unwrap();
        assert!(target.0.join("EFI/OC/Tools").is_dir());
        assert!(target.0.join("com.apple.recovery.boot/BaseSystem.dmg").is_file());
        let stages = stages.into_inner().unwrap();
        assert_eq!(stages.last(), Some(&(CopyStage::Recovery, 1.0)));

        let report = read_target(&target.0, &efi, Some(&recovery)).await.unwrap();
        verify_target(&efi, Some(&recovery), &report).unwrap();

        // Corrupt a critical file with the same size.
        std::fs::write(target.0.join("EFI/OC/config.plist"), b"<PLIST/>").unwrap();
        let report = read_target(&target.0, &efi, Some(&recovery)).await.unwrap();
        assert_eq!(verify_target(&efi, Some(&recovery), &report).unwrap_err().code, "VERIFY_FAILED");

        // Truncated DMG.
        std::fs::write(target.0.join("EFI/OC/config.plist"), b"<plist/>").unwrap();
        std::fs::write(target.0.join("com.apple.recovery.boot/BaseSystem.dmg"), [7u8; 10]).unwrap();
        let report = read_target(&target.0, &efi, Some(&recovery)).await.unwrap();
        assert!(verify_target(&efi, Some(&recovery), &report).unwrap_err().message.contains("BaseSystem.dmg"));

        // The recovery folder never arrived.
        std::fs::remove_dir_all(target.0.join(RECOVERY_DIR_NAME)).unwrap();
        let report = read_target(&target.0, &efi, Some(&recovery)).await.unwrap();
        assert_eq!(report.dmg_count, Some(0));
        assert_eq!(verify_target(&efi, Some(&recovery), &report).unwrap_err().code, "VERIFY_FAILED");
    }

    #[test]
    fn script_report_parsing() {
        let tmp = TempDir::new();
        make_efi(&tmp.0);
        let efi = inspect_efi(&tmp.0).unwrap();
        let mut stdout = String::from("STEP verify\n");
        for file in &efi.files {
            if let Some(hash) = &file.sha256 {
                stdout.push_str(&format!("HASH {hash} {}\n", file.rel));
            }
            stdout.push_str(&format!("SIZE {} {}\n", file.size, file.rel.to_uppercase()));
        }
        let report = parse_script_report(&stdout);
        verify_target(&efi, None, &report).unwrap();
        let report = parse_script_report("SIZE 4 EFI/BOOT/BOOTx64.efi\nDMGCOUNT 2\n");
        assert_eq!(report.dmg_count, Some(2));
        assert!(verify_target(&efi, None, &report).is_err());
    }

    #[test]
    fn capacity_includes_overhead() {
        let tmp = TempDir::new();
        make_efi(&tmp.0);
        let efi = inspect_efi(&tmp.0).unwrap();
        assert!(required_capacity(&efi, None) > 64 * 1024 * 1024);
    }
}
