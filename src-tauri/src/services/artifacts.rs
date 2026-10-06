//! OpenCore package, OcBinaryData, kext and prebuilt SSDT acquisition + safe
//! extraction.
//!
//! Downloads go through the SHA-256 keyed cache of [`Downloader`]; every
//! archive is extracted once per content hash into `work_dir` (a temporary
//! directory renamed into place, so a half-extracted tree is never reused).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::json;
use walkdir::WalkDir;

use crate::contracts::ArtifactStatus;
use crate::domain::kext_catalog::{self, ArchiveKind, KextCatalogEntry, Pin};
use crate::error::AppError;
use crate::services::http::{is_sha256_hex, sleep_cancellable, Downloader, FetchedBytes, ProgressFn};
use crate::tasks::cancellation::CancellationToken;

/// Marker written into an extraction directory once it is complete.
const COMPLETE_MARKER: &str = ".complete";
/// Limits against zip bombs: total bytes written, one entry, entry count.
const MAX_TOTAL_UNCOMPRESSED: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_UNCOMPRESSED: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
/// Extra attempts for unpinned CI artifacts whose proxy answers 404 while it
/// looks the artifact up.
const NIGHTLY_NOT_FOUND_RETRIES: u32 = 2;

/// An extracted OpenCorePkg release.
#[derive(Debug, Clone)]
pub struct OpenCorePackage {
    pub version: String,
    pub debug: bool,
    /// Extraction root (contains X64/, Docs/, Utilities/).
    pub root: PathBuf,
    pub status: ArtifactStatus,
}

impl OpenCorePackage {
    pub fn x64_efi(&self) -> PathBuf {
        self.root.join("X64").join("EFI")
    }
    pub fn sample_plist(&self) -> PathBuf {
        self.root.join("Docs").join("Sample.plist")
    }
    pub fn acpi_samples(&self) -> PathBuf {
        self.root.join("Docs").join("AcpiSamples").join("Binaries")
    }
    /// Host-native ocvalidate (ocvalidate.exe / ocvalidate.linux / ocvalidate).
    pub fn ocvalidate(&self) -> Option<PathBuf> {
        host_utility(&self.root, "ocvalidate")
    }
    /// Host-native macserial.
    pub fn macserial(&self) -> Option<PathBuf> {
        host_utility(&self.root, "macserial")
    }
}

/// `Utilities/<tool>/<tool>[.exe|.linux]` for the running OS, if it exists
/// and can run here (the Linux build is x86_64 only; the macOS one is
/// universal; the Windows one is 32-bit x86 and runs everywhere).
fn host_utility(root: &Path, tool: &str) -> Option<PathBuf> {
    let file = if cfg!(target_os = "windows") {
        format!("{tool}.exe")
    } else if cfg!(target_os = "linux") {
        if !cfg!(target_arch = "x86_64") {
            return None;
        }
        format!("{tool}.linux")
    } else if cfg!(target_os = "macos") {
        tool.to_string()
    } else {
        return None;
    };
    let path = root.join("Utilities").join(tool).join(file);
    path.is_file().then_some(path)
}

/// Where an artifact comes from: the pin, or a newer GitHub release.
#[derive(Debug, Clone)]
struct Source {
    version: String,
    url: String,
    sha256: Option<String>,
    latest: bool,
}

impl Source {
    fn pinned(pin: &Pin) -> Self {
        Source {
            version: pin.version.to_string(),
            url: pin.url.to_string(),
            sha256: pin.sha256.map(str::to_string),
            latest: false,
        }
    }
}

pub async fn fetch_opencore(
    dl: &Downloader,
    work_dir: &Path,
    debug: bool,
    use_latest: bool,
    cancel: &CancellationToken,
) -> Result<OpenCorePackage, AppError> {
    fetch_opencore_with_progress(dl, work_dir, debug, use_latest, cancel, None).await
}

/// [`fetch_opencore`] reporting download progress (bytes done, total).
pub async fn fetch_opencore_with_progress(
    dl: &Downloader,
    work_dir: &Path,
    debug: bool,
    use_latest: bool,
    cancel: &CancellationToken,
    progress: Option<ProgressFn<'_>>,
) -> Result<OpenCorePackage, AppError> {
    let pin = if debug { kext_catalog::opencore_debug() } else { kext_catalog::opencore_release() };
    if use_latest {
        match latest_opencore(dl, debug, &pin).await {
            Ok(Some(source)) => match install_opencore(dl, work_dir, debug, &source, cancel, progress).await {
                Ok(pkg) => return Ok(pkg),
                Err(e) if e.code == "TASK_CANCELLED" => return Err(e),
                Err(e) => {
                    tracing::warn!(version = %source.version, error = %e, "latest OpenCore unusable, using the pinned release")
                }
            },
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "could not resolve the latest OpenCore release, using the pinned one"),
        }
    }
    install_opencore(dl, work_dir, debug, &Source::pinned(&pin), cancel, progress).await
}

/// Newest OpenCorePkg release when it differs from the pin.
async fn latest_opencore(dl: &Downloader, debug: bool, pin: &Pin) -> Result<Option<Source>, AppError> {
    let release = dl.github_latest_release(kext_catalog::OPENCORE_REPO).await?;
    let flavour = if debug { "DEBUG" } else { "RELEASE" };
    let re = Regex::new(&format!(r"^OpenCore-[0-9][0-9.]*-{flavour}\.zip$"))
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))?;
    let asset =
        release.assets.iter().find(|a| re.is_match(&a.name)).ok_or_else(|| {
            AppError::new("ASSET_NOT_FOUND", format!("OpenCorePkg {} has no {flavour} zip", release.tag))
        })?;
    if asset.url == pin.url {
        return Ok(None);
    }
    Ok(Some(Source {
        version: version_from_tag(&release.tag),
        url: asset.url.clone(),
        sha256: asset.sha256(),
        latest: true,
    }))
}

async fn install_opencore(
    dl: &Downloader,
    work_dir: &Path,
    debug: bool,
    source: &Source,
    cancel: &CancellationToken,
    progress: Option<ProgressFn<'_>>,
) -> Result<OpenCorePackage, AppError> {
    let flavour = if debug { "DEBUG" } else { "RELEASE" };
    let base = work_dir.join("opencore");
    let name_for = |sha: &str| format!("{}-{flavour}-{}", sanitize(&source.version), short(sha));
    let package = |root: PathBuf, status| OpenCorePackage { version: source.version.clone(), debug, root, status };

    if let Some(sha) = &source.sha256 {
        let dir = base.join(name_for(sha));
        if is_complete(&dir) {
            return Ok(package(dir, ArtifactStatus::Cached));
        }
    }
    let fetched = dl.fetch_verified(&source.url, source.sha256.as_deref(), cancel, progress).await?;
    let status = if fetched.from_cache { ArtifactStatus::Cached } else { ArtifactStatus::Downloaded };
    let dir = base.join(name_for(&fetched.sha256));
    if !is_complete(&dir) {
        let bytes = fetched.bytes;
        let target = dir.clone();
        run_blocking(move || {
            extract_into(&target, |tmp| {
                extract_zip(&bytes, tmp, false)?;
                check_opencore_layout(tmp)?;
                make_utilities_executable(tmp);
                Ok(())
            })
        })
        .await?;
        tracing::info!(version = %source.version, flavour, latest = source.latest, "OpenCore extracted");
    }
    Ok(package(dir, status))
}

fn check_opencore_layout(root: &Path) -> Result<(), AppError> {
    let required = [
        ["X64", "EFI", "BOOT", "BOOTx64.efi"].iter().collect::<PathBuf>(),
        ["X64", "EFI", "OC", "OpenCore.efi"].iter().collect(),
        ["Docs", "Sample.plist"].iter().collect(),
    ];
    for rel in required {
        if !root.join(&rel).is_file() {
            return Err(AppError::new(
                "OPENCORE_LAYOUT",
                format!("The OpenCore package does not contain {}", rel.display()),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_utilities_executable(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for tool in ["ocvalidate", "macserial"] {
        for file in [tool.to_string(), format!("{tool}.linux")] {
            let path = root.join("Utilities").join(tool).join(file);
            if path.is_file() {
                if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)) {
                    tracing::warn!(path = %path.display(), error = %e, "chmod failed");
                }
            }
        }
    }
}

#[cfg(not(unix))]
fn make_utilities_executable(_root: &Path) {}

/// Extracted OcBinaryData (Resources/, Drivers/).
pub async fn fetch_ocbinarydata(
    dl: &Downloader,
    work_dir: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, AppError> {
    fetch_ocbinarydata_with_progress(dl, work_dir, cancel, None).await
}

/// [`fetch_ocbinarydata`] reporting download progress (bytes done, total).
pub async fn fetch_ocbinarydata_with_progress(
    dl: &Downloader,
    work_dir: &Path,
    cancel: &CancellationToken,
    progress: Option<ProgressFn<'_>>,
) -> Result<PathBuf, AppError> {
    let pin = kext_catalog::ocbinarydata();
    let dir = work_dir.join("ocbinarydata").join(short(pin.version));
    if !is_complete(&dir) {
        let fetched = dl.fetch_verified(pin.url, pin.sha256, cancel, progress).await?;
        let bytes = fetched.bytes;
        let target = dir.clone();
        run_blocking(move || {
            extract_into(&target, |tmp| {
                extract_zip(&bytes, tmp, false)?;
                ocbinarydata_root(tmp).map(|_| ())
            })
        })
        .await?;
    }
    ocbinarydata_root(&dir)
}

/// The archive wraps everything in `OcBinaryData-<commit>/`.
fn ocbinarydata_root(dir: &Path) -> Result<PathBuf, AppError> {
    let candidates: Vec<PathBuf> =
        std::fs::read_dir(dir)?.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    let root = match candidates.as_slice() {
        [single] => single.clone(),
        _ => dir.to_path_buf(),
    };
    if root.join("Resources").is_dir() && root.join("Drivers").is_dir() {
        Ok(root)
    } else {
        Err(AppError::new("OCBINARYDATA_LAYOUT", "OcBinaryData archive has no Resources/ and Drivers/ folders"))
    }
}

/// A kext archive downloaded and extracted to a staging directory.
#[derive(Debug, Clone)]
pub struct FetchedKext {
    pub catalog_id: String,
    pub version: String,
    pub status: ArtifactStatus,
    /// Directory containing the archive's top-level `.kext` bundles.
    pub bundles_dir: PathBuf,
}

impl FetchedKext {
    /// Bundle names available in `bundles_dir`.
    pub fn bundles(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.bundles_dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| has_kext_ext(n))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

pub async fn fetch_kext(
    dl: &Downloader,
    entry: &KextCatalogEntry,
    work_dir: &Path,
    use_latest: bool,
    cancel: &CancellationToken,
) -> Result<FetchedKext, AppError> {
    fetch_kext_with_progress(dl, entry, work_dir, use_latest, cancel, None).await
}

/// [`fetch_kext`] reporting download progress (bytes done, total).
pub async fn fetch_kext_with_progress(
    dl: &Downloader,
    entry: &KextCatalogEntry,
    work_dir: &Path,
    use_latest: bool,
    cancel: &CancellationToken,
    progress: Option<ProgressFn<'_>>,
) -> Result<FetchedKext, AppError> {
    if use_latest && entry.supports_latest() {
        match latest_kext(dl, entry).await {
            Ok(Some(source)) => match install_kext(dl, entry, work_dir, &source, cancel, progress).await {
                Ok(k) => return Ok(k),
                Err(e) if e.code == "TASK_CANCELLED" => return Err(e),
                Err(e) => {
                    tracing::warn!(kext = entry.id, version = %source.version, error = %e, "latest release unusable, using the pinned one")
                }
            },
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(kext = entry.id, error = %e, "could not resolve the latest release, using the pinned one")
            }
        }
    }
    install_kext(dl, entry, work_dir, &Source::pinned(&entry.pin), cancel, progress).await
}

/// Newest release asset matching the entry's regex, when it differs from the pin.
async fn latest_kext(dl: &Downloader, entry: &KextCatalogEntry) -> Result<Option<Source>, AppError> {
    let release = dl.github_latest_release(entry.repo).await?;
    let re = Regex::new(entry.latest_asset_regex)
        .map_err(|e| AppError::new("INTERNAL", format!("{}: bad asset pattern: {e}", entry.id)))?;
    let asset = release.assets.iter().find(|a| re.is_match(&a.name) && !is_debug_asset(&a.name)).ok_or_else(|| {
        AppError::new("ASSET_NOT_FOUND", format!("{} {} has no matching release asset", entry.repo, release.tag))
    })?;
    if asset.url == entry.pin.url {
        return Ok(None);
    }
    Ok(Some(Source {
        version: version_from_tag(&release.tag),
        url: asset.url.clone(),
        sha256: asset.sha256(),
        latest: true,
    }))
}

fn is_debug_asset(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("DEBUG") || upper.contains("RESEARCH")
}

async fn install_kext(
    dl: &Downloader,
    entry: &KextCatalogEntry,
    work_dir: &Path,
    source: &Source,
    cancel: &CancellationToken,
    progress: Option<ProgressFn<'_>>,
) -> Result<FetchedKext, AppError> {
    let base = work_dir.join("kexts");
    let name_for = |sha: &str| format!("{}-{}-{}", sanitize(entry.id), sanitize(&source.version), short(sha));
    let result = |dir: PathBuf, status| FetchedKext {
        catalog_id: entry.id.to_string(),
        version: source.version.clone(),
        status,
        bundles_dir: dir.join("bundles"),
    };

    if let Some(sha) = &source.sha256 {
        let dir = base.join(name_for(sha));
        if is_complete(&dir) {
            return Ok(result(dir, ArtifactStatus::Cached));
        }
    }
    let nightly = source.sha256.is_none() && entry.archive == ArchiveKind::NestedZip;
    let fetched = fetch_archive(dl, &source.url, source.sha256.as_deref(), nightly, cancel, progress).await?;
    let status = if fetched.from_cache { ArtifactStatus::Cached } else { ArtifactStatus::Downloaded };
    let dir = base.join(name_for(&fetched.sha256));
    if !is_complete(&dir) {
        let bytes = fetched.bytes;
        let target = dir.clone();
        let nested = entry.archive == ArchiveKind::NestedZip;
        let required: Vec<String> = entry.bundles.iter().map(|b| b.to_string()).collect();
        let id = entry.id.to_string();
        run_blocking(move || {
            extract_into(&target, |tmp| {
                let raw = tmp.join("raw");
                let bundles = tmp.join("bundles");
                extract_zip(&bytes, &raw, nested)?;
                collect_bundles(&raw, &bundles)?;
                std::fs::remove_dir_all(&raw)?;
                for name in &required {
                    let path = bundles.join(name);
                    if !path.is_dir() {
                        return Err(AppError::new(
                            "KEXT_BUNDLE_MISSING",
                            format!("{id}: archive does not contain {name}"),
                        )
                        .with_context(json!({ "catalogId": id, "bundle": name })));
                    }
                    validate_bundle(&path)?;
                }
                Ok(())
            })
        })
        .await?;
        tracing::info!(kext = entry.id, version = %source.version, latest = source.latest, "kext archive extracted");
    }
    Ok(result(dir, status))
}

/// Download an archive. Unpinned CI artifacts (`nightly`) come through a
/// proxy that sometimes answers 404 before it finds the artifact, so a 404
/// is retried a few times for them; everything else fails on 404 at once.
async fn fetch_archive(
    dl: &Downloader,
    url: &str,
    sha256: Option<&str>,
    nightly: bool,
    cancel: &CancellationToken,
    progress: Option<ProgressFn<'_>>,
) -> Result<FetchedBytes, AppError> {
    let mut attempt = 0;
    loop {
        match dl.fetch_verified(url, sha256, cancel, progress).await {
            Err(e) if nightly && e.code == "HTTP_NOT_FOUND" && attempt < NIGHTLY_NOT_FOUND_RETRIES => {
                attempt += 1;
                tracing::warn!(url, attempt, "artifact not found yet, asking again");
                sleep_cancellable(dl.retry.delay(attempt), cancel).await?;
            }
            other => return other,
        }
    }
}

/// Download a prebuilt Dortania SSDT at its pinned commit (hash verified,
/// cached) and check that it is an ACPI table.
pub async fn fetch_dortania_ssdt(
    dl: &Downloader,
    file_name: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, AppError> {
    let pin = kext_catalog::dortania_ssdt(file_name).ok_or_else(|| {
        AppError::new("SSDT_NOT_AVAILABLE", format!("Dortania does not provide a prebuilt {file_name}"))
            .with_context(json!({ "file": file_name }))
    })?;
    let bytes = dl.fetch_bytes(pin.url, pin.sha256, cancel, None).await?;
    crate::services::ocvalidate::check_aml(&bytes)
        .map_err(|why| AppError::new("SSDT_INVALID", format!("{file_name}: {why}")))?;
    Ok(bytes)
}

/// Copy one top-level bundle from a fetched archive into EFI/OC/Kexts.
pub fn install_bundle(fetched: &FetchedKext, bundle: &str, kexts_dir: &Path) -> Result<(), AppError> {
    if !is_plain_bundle_name(bundle) {
        return Err(AppError::new("INVALID_BUNDLE_NAME", format!("'{bundle}' is not a top-level .kext bundle name")));
    }
    // Use the archive's spelling, so a request that differs only in case
    // works on case-sensitive file systems and BundlePath matches the disk.
    let name = on_disk_name(&fetched.bundles_dir, bundle).unwrap_or_else(|| bundle.to_string());
    let src = fetched.bundles_dir.join(&name);
    let meta = std::fs::symlink_metadata(&src).map_err(|_| {
        AppError::new("KEXT_BUNDLE_MISSING", format!("{} does not provide {bundle}", fetched.catalog_id))
            .with_context(json!({ "catalogId": fetched.catalog_id, "bundle": bundle, "available": fetched.bundles() }))
    })?;
    if !meta.is_dir() {
        return Err(AppError::new(
            "KEXT_BUNDLE_MISSING",
            format!("{bundle} in {} is not a directory", fetched.catalog_id),
        ));
    }
    std::fs::create_dir_all(kexts_dir)?;
    if let Some(old) = on_disk_name(kexts_dir, &name) {
        remove_path(&kexts_dir.join(old))?;
    }
    let dest = kexts_dir.join(&name);
    if let Err(e) = copy_dir_recursive(&src, &dest) {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    Ok(())
}

/// Spelling of `name` inside `dir` on disk: the exact name when present,
/// otherwise the one entry that matches it ignoring ASCII case.
fn on_disk_name(dir: &Path, name: &str) -> Option<String> {
    let names: Vec<String> =
        std::fs::read_dir(dir).ok()?.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).collect();
    if names.iter().any(|n| n == name) {
        return Some(name.to_string());
    }
    let mut matches = names.into_iter().filter(|n| n.eq_ignore_ascii_case(name));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn is_plain_bundle_name(name: &str) -> bool {
    has_kext_ext(name) && name.len() > ".kext".len() && !name.contains(['/', '\\', ':', '\0']) && !name.starts_with('.')
}

fn has_kext_ext(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 5 && b[b.len() - 5..].eq_ignore_ascii_case(b".kext")
}

fn remove_path(path: &Path) -> Result<(), AppError> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Recursive copy that never follows or recreates symlinks.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dest)?;
    for item in WalkDir::new(src).follow_links(false).min_depth(1) {
        let item = item.map_err(|e| AppError::new("IO_ERROR", e.to_string()))?;
        let rel = item.path().strip_prefix(src).map_err(|e| AppError::new("IO_ERROR", e.to_string()))?;
        let target = dest.join(rel);
        let ft = item.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if ft.is_file() {
            std::fs::copy(item.path(), &target)?;
        } else {
            tracing::debug!(path = %item.path().display(), "skipping non-regular file while copying");
        }
    }
    Ok(())
}

/// Zip-slip-safe extraction (rejects absolute paths, `..`, drive prefixes,
/// symlinks; skips `__MACOSX` and `.dSYM`; caps total size). Recurses into a
/// nested `*-RELEASE.zip` when `nested_release` is true.
///
/// Entries with unsafe names make the whole extraction fail; symlink
/// entries are skipped (never created), so later entries cannot be
/// redirected through them.
pub fn extract_zip(bytes: &[u8], dest: &Path, nested_release: bool) -> Result<(), AppError> {
    extract_zip_with(bytes, dest, nested_release, Limits::DEFAULT)
}

/// Zip-bomb limits: bytes written in total, bytes per entry, entry count.
#[derive(Debug, Clone, Copy)]
struct Limits {
    total: u64,
    entry: u64,
    entries: usize,
}

impl Limits {
    const DEFAULT: Limits =
        Limits { total: MAX_TOTAL_UNCOMPRESSED, entry: MAX_ENTRY_UNCOMPRESSED, entries: MAX_ENTRIES };
}

fn extract_zip_with(bytes: &[u8], dest: &Path, nested_release: bool, limits: Limits) -> Result<(), AppError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(archive_error)?;
    if archive.len() > limits.entries {
        return Err(AppError::new("ARCHIVE_TOO_LARGE", format!("Archive has {} entries", archive.len())));
    }
    if nested_release {
        if let Some(inner) = find_inner_release_zip(&archive) {
            let mut file = archive.by_name(&inner).map_err(archive_error)?;
            let mut buf = Vec::new();
            (&mut file).take(limits.entry + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > limits.entry {
                return Err(AppError::new("ARCHIVE_TOO_LARGE", format!("{inner} is too large")));
            }
            return extract_zip_with(&buf, dest, false, limits);
        }
        if !archive.file_names().any(|n| n.to_ascii_lowercase().contains(".kext/")) {
            return Err(AppError::new("ARCHIVE_NO_RELEASE", "The artifact contains neither a RELEASE zip nor a kext"));
        }
    }

    std::fs::create_dir_all(dest)?;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(archive_error)?;
        let Some(parts) = entry_components(file.name())? else { continue };
        if file.is_symlink() {
            tracing::debug!(entry = file.name(), "skipping symlink in archive");
            continue;
        }
        let out = parts.iter().fold(dest.to_path_buf(), |p, c| p.join(c));
        if !out.starts_with(dest) || passes_through_symlink(dest, &parts) {
            return Err(unsafe_entry(file.name()));
        }
        if file.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&out)?;
        let budget = limits.entry.min(limits.total.saturating_sub(total));
        let written = std::io::copy(&mut (&mut file).take(budget + 1), &mut f)?;
        if written > budget {
            drop(f);
            let _ = std::fs::remove_file(&out);
            return Err(AppError::new("ARCHIVE_TOO_LARGE", "Archive expands beyond the allowed size"));
        }
        f.flush()?;
        total += written;
        set_mode(&out, file.unix_mode());
    }
    Ok(())
}

/// True when an existing path below `dest` on the way to the entry is a
/// symlink (possible only when `dest` was not empty to begin with), so the
/// write would land somewhere else.
fn passes_through_symlink(dest: &Path, parts: &[String]) -> bool {
    let mut cur = dest.to_path_buf();
    for part in parts {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Split an entry name into validated components. `Ok(None)` skips the
/// entry (metadata junk), `Err` rejects an unsafe archive.
fn entry_components(name: &str) -> Result<Option<Vec<String>>, AppError> {
    if name.contains('\0') || name.starts_with('/') || name.starts_with('\\') {
        return Err(unsafe_entry(name));
    }
    let mut parts = Vec::new();
    for seg in name.split(['/', '\\']) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        // "..", and anything Windows would collapse into it ("...", ".. ").
        if seg.chars().all(|c| c == '.' || c == ' ') {
            return Err(unsafe_entry(name));
        }
        // Drive letters ("C:"), alternate data streams, control characters,
        // and DOS device names that Windows opens instead of a file.
        if seg.contains(':') || seg.chars().any(char::is_control) || is_dos_device_name(seg) {
            return Err(unsafe_entry(name));
        }
        parts.push(seg.to_string());
    }
    let Some(last) = parts.last() else { return Ok(None) };
    let skip = parts.first().is_some_and(|f| f == "__MACOSX")
        || parts.iter().any(|p| p.to_ascii_lowercase().ends_with(".dsym"))
        || last.starts_with("._")
        || last == ".DS_Store";
    Ok(if skip { None } else { Some(parts) })
}

/// "CON", "nul.txt", "COM1.kext": names Windows maps to devices.
fn is_dos_device_name(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        s if s.len() == 4 && (s.starts_with("COM") || s.starts_with("LPT")) => s.as_bytes()[3].is_ascii_digit(),
        _ => false,
    }
}

fn find_inner_release_zip<R: Read + std::io::Seek>(archive: &zip::ZipArchive<R>) -> Option<String> {
    let mut names: Vec<&str> = archive
        .file_names()
        .filter(|n| {
            let upper = n.to_ascii_uppercase();
            let file = upper.rsplit(['/', '\\']).next().unwrap_or_default();
            file.ends_with("-RELEASE.ZIP") && !file.contains("RESEARCH") && !upper.contains("__MACOSX")
        })
        .collect();
    names.sort_by_key(|n| (n.matches('/').count(), n.len()));
    names.first().map(|n| n.to_string())
}

fn unsafe_entry(name: &str) -> AppError {
    AppError::new(
        "ARCHIVE_UNSAFE_PATH",
        format!("Archive entry '{}' points outside the target folder", name.escape_debug()),
    )
}

fn archive_error(e: zip::result::ZipError) -> AppError {
    AppError::new("ARCHIVE_INVALID", format!("Could not read the zip archive: {e}"))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if mode.is_some_and(|m| m & 0o111 != 0) { 0o755 } else { 0o644 };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>) {}

/// Move every top-level `.kext` found under `raw` into `out/<Name>.kext`.
/// Bundles inside other bundles are plugins and stay where they are. When
/// an archive ships the same bundle twice (`Release/` and `Debug/`), the
/// copy outside any debug folder wins, then the shallowest one.
fn collect_bundles(raw: &Path, out: &Path) -> Result<(), AppError> {
    let mut found: BTreeMap<String, Vec<(bool, usize, PathBuf)>> = BTreeMap::new();
    let mut walker = WalkDir::new(raw).follow_links(false).min_depth(1).sort_by_file_name().into_iter();
    while let Some(item) = walker.next() {
        let item = item.map_err(|e| AppError::new("IO_ERROR", e.to_string()))?;
        if !item.file_type().is_dir() {
            continue;
        }
        let name = item.file_name().to_string_lossy().to_string();
        if !has_kext_ext(&name) {
            continue;
        }
        walker.skip_current_dir();
        let rel = item.path().strip_prefix(raw).unwrap_or(item.path());
        let debug = rel.parent().is_some_and(|p| {
            p.components().any(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase().contains("debug"))
        });
        found.entry(name).or_default().push((debug, rel.components().count(), item.path().to_path_buf()));
    }
    if found.is_empty() {
        return Err(AppError::new("KEXT_BUNDLE_MISSING", "The archive contains no .kext bundle"));
    }
    std::fs::create_dir_all(out)?;
    for (name, mut candidates) in found {
        candidates.sort_by_key(|c| (c.0, c.1));
        if candidates.len() > 1 {
            tracing::debug!(bundle = %name, copies = candidates.len(), "archive has several copies of a bundle");
        }
        if let Some((_, _, path)) = candidates.into_iter().next() {
            std::fs::rename(&path, out.join(&name))?;
        }
    }
    Ok(())
}

/// A bundle is usable when `Contents/Info.plist` parses and names a bundle id.
fn validate_bundle(path: &Path) -> Result<(), AppError> {
    let plist_path = path.join("Contents").join("Info.plist");
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let invalid = |why: String| AppError::new("KEXT_INVALID_BUNDLE", format!("{name}: {why}"));
    let value = plist::Value::from_file(&plist_path).map_err(|e| invalid(format!("unreadable Info.plist ({e})")))?;
    let id = value
        .as_dictionary()
        .and_then(|d| d.get("CFBundleIdentifier"))
        .and_then(plist::Value::as_string)
        .unwrap_or_default();
    if id.trim().is_empty() {
        return Err(invalid("Info.plist has no CFBundleIdentifier".into()));
    }
    Ok(())
}

/// Run `fill` on a fresh temporary sibling of `dir`, then rename it into
/// place with a completion marker. A concurrent extraction of the same
/// content that finished first wins; ours is discarded.
fn extract_into(dir: &Path, fill: impl FnOnce(&Path) -> Result<(), AppError>) -> Result<(), AppError> {
    let parent =
        dir.parent().ok_or_else(|| AppError::new("INVALID_PATH", format!("{} has no parent", dir.display())))?;
    std::fs::create_dir_all(parent)?;
    let leaf = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = parent.join(format!(".{leaf}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let outcome: Result<(), AppError> = (|| {
        std::fs::create_dir_all(&tmp)?;
        fill(&tmp)?;
        std::fs::write(tmp.join(COMPLETE_MARKER), b"")?;
        if dir.exists() && !is_complete(dir) {
            std::fs::remove_dir_all(dir)?;
        }
        // On Windows a virus scanner still reading a fresh file blocks the
        // rename of its folder for a moment; retry briefly.
        let mut attempt: u64 = 0;
        loop {
            match std::fs::rename(&tmp, dir) {
                Ok(()) => return Ok(()),
                Err(_) if is_complete(dir) => return Ok(()),
                Err(e) if attempt < 5 && !dir.exists() => {
                    attempt += 1;
                    tracing::debug!(dir = %dir.display(), error = %e, attempt, "rename failed, retrying");
                    std::thread::sleep(std::time::Duration::from_millis(200 * attempt));
                }
                Err(e) => return Err(AppError::from(e)),
            }
        }
    })();
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    outcome
}

fn is_complete(dir: &Path) -> bool {
    dir.join(COMPLETE_MARKER).is_file()
}

async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new("INTERNAL", format!("extraction task failed: {e}")))?
}

/// "v.1.2.3" → "1.2.3", "Release368" → "368".
fn version_from_tag(tag: &str) -> String {
    let trimmed = tag.trim_start_matches(|c: char| !c.is_ascii_digit());
    if trimmed.is_empty() {
        tag.to_string()
    } else {
        trimmed.to_string()
    }
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect()
}

fn short(sha: &str) -> String {
    if is_sha256_hex(sha) || sha.len() >= 12 {
        sha.chars().take(12).collect()
    } else {
        sanitize(sha)
    }
}

#[cfg(test)]
mod tests {
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::services::http::sha256_hex;
    use crate::services::http::test_server::{serve, Reply};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-art-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn info_plist(id: &str, exe: Option<&str>) -> Vec<u8> {
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), id.into());
        if let Some(e) = exe {
            d.insert("CFBundleExecutable".into(), e.into());
        }
        let mut out = Vec::new();
        plist::Value::Dictionary(d).to_writer_xml(&mut out).unwrap();
        out
    }

    enum Item<'a> {
        File(&'a str, &'a [u8]),
        Exec(&'a str, &'a [u8]),
        Dir(&'a str),
        Symlink(&'a str, &'a str),
    }

    fn make_zip(items: &[Item]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default();
        for item in items {
            match item {
                Item::File(name, data) => {
                    w.start_file(*name, opts).unwrap();
                    w.write_all(data).unwrap();
                }
                Item::Exec(name, data) => {
                    w.start_file(*name, opts.unix_permissions(0o755)).unwrap();
                    w.write_all(data).unwrap();
                }
                Item::Dir(name) => w.add_directory(*name, opts).unwrap(),
                Item::Symlink(name, target) => w.add_symlink(*name, *target, opts).unwrap(),
            }
        }
        w.finish().unwrap().into_inner()
    }

    fn kext_items(prefix: &str, name: &str, id: &str) -> Vec<(String, Vec<u8>)> {
        vec![
            (format!("{prefix}{name}.kext/Contents/Info.plist"), info_plist(id, Some(name))),
            (format!("{prefix}{name}.kext/Contents/MacOS/{name}"), vec![0xcf, 0xfa, 0xed, 0xfe]),
        ]
    }

    fn zip_of(owned: &[(String, Vec<u8>)]) -> Vec<u8> {
        let items: Vec<Item> = owned.iter().map(|(n, d)| Item::File(n, d)).collect();
        make_zip(&items)
    }

    #[test]
    fn extracts_plain_archive_and_skips_junk() {
        let tmp = TempDir::new();
        let zip = make_zip(&[
            Item::Dir("Lilu.kext/"),
            Item::File("Lilu.kext/Contents/Info.plist", b"plist"),
            Item::Exec("Lilu.kext/Contents/MacOS/Lilu", b"\xcf\xfa\xed\xfe"),
            Item::File("Lilu.kext.dSYM/Contents/Resources/DWARF/Lilu", b"dwarf"),
            Item::File("__MACOSX/Lilu.kext/Contents/._Info.plist", b"appledouble"),
            Item::File("Lilu.kext/Contents/._Info.plist", b"appledouble"),
            Item::File("Lilu.kext/.DS_Store", b"ds"),
        ]);
        extract_zip(&zip, &tmp.0, false).unwrap();
        assert_eq!(std::fs::read(tmp.0.join("Lilu.kext/Contents/Info.plist")).unwrap(), b"plist");
        assert!(tmp.0.join("Lilu.kext/Contents/MacOS/Lilu").is_file());
        assert!(!tmp.0.join("Lilu.kext.dSYM").exists());
        assert!(!tmp.0.join("__MACOSX").exists());
        assert!(!tmp.0.join("Lilu.kext/Contents/._Info.plist").exists());
        assert!(!tmp.0.join("Lilu.kext/.DS_Store").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(tmp.0.join("Lilu.kext/Contents/MacOS/Lilu")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn rejects_parent_dir_escape() {
        let tmp = TempDir::new();
        let dest = tmp.0.join("out");
        for name in ["../evil.txt", "a/../../evil.txt", "a\\..\\..\\evil.txt", "a/.../evil.txt", "a/.. /evil.txt"] {
            let zip = make_zip(&[Item::File("ok.txt", b"ok"), Item::File(name, b"x")]);
            let err = extract_zip(&zip, &dest, false).unwrap_err();
            assert_eq!(err.code, "ARCHIVE_UNSAFE_PATH", "{name}");
        }
        assert!(!tmp.0.join("evil.txt").exists());
    }

    #[test]
    fn rejects_absolute_and_drive_paths() {
        let tmp = TempDir::new();
        for name in
            ["/etc/evil", "\\Windows\\evil", "C:/evil", "C:evil", "Lilu.kext/Contents/C:evil", "a/file.txt:stream"]
        {
            let zip = make_zip(&[Item::File(name, b"x")]);
            let err = extract_zip(&zip, &tmp.0.join("out"), false).unwrap_err();
            assert_eq!(err.code, "ARCHIVE_UNSAFE_PATH", "{name}");
        }
    }

    #[test]
    fn rejects_dos_device_names() {
        let tmp = TempDir::new();
        for name in ["CON", "a/nul.txt", "Lilu.kext/Contents/COM1", "x/LPT9.kext/Info.plist", "aux .txt"] {
            let zip = make_zip(&[Item::File(name, b"x")]);
            let err = extract_zip(&zip, &tmp.0.join("out"), false).unwrap_err();
            assert_eq!(err.code, "ARCHIVE_UNSAFE_PATH", "{name}");
        }
        // Names that merely start like a device stay allowed.
        let zip = make_zip(&[Item::File("Console.kext/Contents/Info.plist", b"x"), Item::File("COM10/x", b"y")]);
        extract_zip(&zip, &tmp.0.join("ok"), false).unwrap();
        assert!(tmp.0.join("ok/Console.kext/Contents/Info.plist").is_file());
    }

    #[test]
    fn skips_symlinks() {
        let tmp = TempDir::new();
        let zip = make_zip(&[Item::Symlink("link", "/etc"), Item::File("Real.kext/Contents/Info.plist", b"x")]);
        extract_zip(&zip, &tmp.0, false).unwrap();
        assert!(std::fs::symlink_metadata(tmp.0.join("link")).is_err());
        assert!(tmp.0.join("Real.kext/Contents/Info.plist").is_file());

        // A file "through" the skipped link becomes a plain folder inside dest.
        let dest = tmp.0.join("through");
        let zip = make_zip(&[Item::Symlink("x.kext", "../outside"), Item::File("x.kext/Contents/Info.plist", b"y")]);
        extract_zip(&zip, &dest, false).unwrap();
        assert!(dest.join("x.kext/Contents/Info.plist").is_file());
        assert!(!tmp.0.join("outside").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_write_through_existing_symlinks() {
        let tmp = TempDir::new();
        let outside = tmp.0.join("outside");
        let dest = tmp.0.join("dest");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::os::unix::fs::symlink(&outside, dest.join("Lilu.kext")).unwrap();
        let zip = make_zip(&[Item::File("Lilu.kext/Contents/Info.plist", b"x")]);
        assert_eq!(extract_zip(&zip, &dest, false).unwrap_err().code, "ARCHIVE_UNSAFE_PATH");
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    }

    #[test]
    fn enforces_size_and_entry_limits() {
        let tmp = TempDir::new();
        let small = Limits { total: 100, entry: 60, entries: 4 };
        let big_entry = make_zip(&[Item::File("a.bin", &[0u8; 61])]);
        let err = extract_zip_with(&big_entry, &tmp.0.join("a"), false, small).unwrap_err();
        assert_eq!(err.code, "ARCHIVE_TOO_LARGE");
        assert!(!tmp.0.join("a/a.bin").exists());

        let too_much = make_zip(&[Item::File("a.bin", &[1u8; 50]), Item::File("b.bin", &[2u8; 51])]);
        let err = extract_zip_with(&too_much, &tmp.0.join("b"), false, small).unwrap_err();
        assert_eq!(err.code, "ARCHIVE_TOO_LARGE");

        let many: Vec<(String, Vec<u8>)> = (0..5).map(|i| (format!("f{i}"), vec![0u8])).collect();
        let err = extract_zip_with(&zip_of(&many), &tmp.0.join("c"), false, small).unwrap_err();
        assert_eq!(err.code, "ARCHIVE_TOO_LARGE");

        // An inner release zip above the per-entry limit is refused as well.
        let inner = make_zip(&[Item::File("X.kext/Contents/Info.plist", &[3u8; 200])]);
        let outer = make_zip(&[Item::File("Release/X-1.0-RELEASE.zip", &inner)]);
        let err = extract_zip_with(&outer, &tmp.0.join("d"), true, small).unwrap_err();
        assert_eq!(err.code, "ARCHIVE_TOO_LARGE");

        let fits = make_zip(&[Item::File("a.bin", &[1u8; 50]), Item::File("b.bin", &[2u8; 50])]);
        extract_zip_with(&fits, &tmp.0.join("e"), false, small).unwrap();
        assert_eq!(std::fs::read(tmp.0.join("e/b.bin")).unwrap(), vec![2u8; 50]);
    }

    #[test]
    fn extracts_nested_release_zip() {
        let tmp = TempDir::new();
        let inner_release = zip_of(&kext_items("", "NootRX", "org.example.NootRX"));
        let inner_debug = make_zip(&[Item::File("NootRX.kext/Contents/Info.plist", b"debug")]);
        let outer = make_zip(&[
            Item::File("Debug/NootRX-1.0.0-DEBUG.zip", &inner_debug),
            Item::File("Release/NootRX-1.0.0-RELEASE.zip", &inner_release),
        ]);
        extract_zip(&outer, &tmp.0, true).unwrap();
        let plist = std::fs::read(tmp.0.join("NootRX.kext/Contents/Info.plist")).unwrap();
        assert_ne!(plist, b"debug");
        assert!(!tmp.0.join("Release").exists());

        // Without the nested flag the inner zips are just files.
        let flat = tmp.0.join("flat");
        extract_zip(&outer, &flat, false).unwrap();
        assert!(flat.join("Release/NootRX-1.0.0-RELEASE.zip").is_file());
    }

    #[test]
    fn nested_mode_ignores_research_builds_and_requires_content() {
        let tmp = TempDir::new();
        let research = make_zip(&[Item::File("X.kext/Contents/Info.plist", b"research")]);
        let outer = make_zip(&[Item::File("X-1.0-RESEARCH_RELEASE.zip", &research)]);
        let err = extract_zip(&outer, &tmp.0, true).unwrap_err();
        assert_eq!(err.code, "ARCHIVE_NO_RELEASE");

        let direct = make_zip(&[Item::File("X.kext/Contents/Info.plist", b"direct")]);
        extract_zip(&direct, &tmp.0, true).unwrap();
        assert!(tmp.0.join("X.kext/Contents/Info.plist").is_file());
    }

    #[test]
    fn rejects_garbage() {
        let tmp = TempDir::new();
        let err = extract_zip(b"<html>captive portal</html>", &tmp.0, false).unwrap_err();
        assert_eq!(err.code, "ARCHIVE_INVALID");
    }

    #[test]
    fn collects_release_copy_over_debug() {
        let tmp = TempDir::new();
        let raw = tmp.0.join("raw");
        let mut items = kext_items("RealtekRTL8111-V2.4.2/Debug/", "RealtekRTL8111", "debug.id");
        items.extend(kext_items("RealtekRTL8111-V2.4.2/Release/", "RealtekRTL8111", "release.id"));
        items.extend(kext_items("Kexts/", "VirtualSMC", "as.vit9696.VirtualSMC"));
        items.push(("VirtualSMC.kext/Contents/PlugIns/Inner.kext/Contents/Info.plist".into(), b"x".to_vec()));
        extract_zip(&zip_of(&items), &raw, false).unwrap();
        let out = tmp.0.join("bundles");
        collect_bundles(&raw, &out).unwrap();
        let plist = plist::Value::from_file(out.join("RealtekRTL8111.kext/Contents/Info.plist")).unwrap();
        let id = plist.as_dictionary().and_then(|d| d.get("CFBundleIdentifier")).and_then(|v| v.as_string());
        assert_eq!(id, Some("release.id"));
        assert!(out.join("VirtualSMC.kext").is_dir());
        assert!(!out.join("Inner.kext").exists());
    }

    #[test]
    fn install_bundle_copies_and_refuses_escapes() {
        let tmp = TempDir::new();
        let bundles = tmp.0.join("bundles");
        let raw = tmp.0.join("raw");
        let mut items = kext_items("", "Lilu", "as.vit9696.Lilu");
        items.push(("Lilu.kext/Contents/PlugIns/P.kext/Contents/Info.plist".into(), b"p".to_vec()));
        extract_zip(&zip_of(&items), &raw, false).unwrap();
        collect_bundles(&raw, &bundles).unwrap();
        let fetched = FetchedKext {
            catalog_id: "Lilu".into(),
            version: "1.7.2".into(),
            status: ArtifactStatus::Downloaded,
            bundles_dir: bundles.clone(),
        };
        assert_eq!(fetched.bundles(), vec!["Lilu.kext".to_string()]);
        let kexts = tmp.0.join("EFI/OC/Kexts");
        install_bundle(&fetched, "Lilu.kext", &kexts).unwrap();
        assert!(kexts.join("Lilu.kext/Contents/MacOS/Lilu").is_file());
        assert!(kexts.join("Lilu.kext/Contents/PlugIns/P.kext/Contents/Info.plist").is_file());
        // Re-installing replaces the previous copy.
        install_bundle(&fetched, "Lilu.kext", &kexts).unwrap();
        // A request in another case installs the archive's spelling.
        install_bundle(&fetched, "LILU.kext", &kexts).unwrap();
        let installed: Vec<String> =
            std::fs::read_dir(&kexts).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(installed, vec!["Lilu.kext".to_string()]);

        for bad in ["../Lilu.kext", "Lilu.kext/../../x.kext", "/abs.kext", "C:x.kext", "Lilu", ".kext", "..\\x.kext"] {
            let err = install_bundle(&fetched, bad, &kexts).unwrap_err();
            assert_eq!(err.code, "INVALID_BUNDLE_NAME", "{bad}");
        }
        assert_eq!(install_bundle(&fetched, "Missing.kext", &kexts).unwrap_err().code, "KEXT_BUNDLE_MISSING");
    }

    #[tokio::test]
    async fn unknown_dortania_ssdt_is_refused_without_network() {
        let tmp = TempDir::new();
        let dl = Downloader::new(tmp.0.join("cache")).unwrap();
        let err = fetch_dortania_ssdt(&dl, "SSDT-GPIO.aml", &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.code, "SSDT_NOT_AVAILABLE");
    }

    #[test]
    fn tag_versions() {
        assert_eq!(version_from_tag("v.1.2.3"), "1.2.3");
        assert_eq!(version_from_tag("V3.0.0"), "3.0.0");
        assert_eq!(version_from_tag("1.0.8"), "1.0.8");
        assert_eq!(version_from_tag("Release368"), "368");
        assert_eq!(version_from_tag("nightly"), "nightly");
    }

    fn test_entry(url: &'static str, sha: Option<&'static str>, archive: ArchiveKind) -> KextCatalogEntry {
        KextCatalogEntry {
            id: "TestKext",
            repo: "",
            pin: Pin { version: "1.0.0", url, sha256: sha },
            archive,
            latest_asset_regex: "",
            bundles: &["TestKext.kext", "TestPlugin.kext"],
            description: "",
        }
    }

    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    #[tokio::test]
    async fn fetch_kext_downloads_extracts_and_reuses() {
        let tmp = TempDir::new();
        let mut items = kext_items("Kexts/", "TestKext", "org.example.TestKext");
        items.extend(kext_items("Kexts/", "TestPlugin", "org.example.TestPlugin"));
        items.push(("dSYM/TestKext.kext.dSYM/Contents/Info.plist".into(), b"dsym".to_vec()));
        let zip = zip_of(&items);
        let sha = leak(sha256_hex(&zip));
        let body = zip.clone();
        let srv = serve(move |_, _| Reply::ok(&body)).await;
        let dl = Downloader::new(tmp.0.join("cache")).unwrap();
        let entry = test_entry(leak(format!("{}/TestKext.zip", srv.base)), Some(sha), ArchiveKind::Zip);
        let work = tmp.0.join("work");
        let cancel = CancellationToken::new();

        let first = fetch_kext(&dl, &entry, &work, false, &cancel).await.unwrap();
        assert_eq!(first.status, ArtifactStatus::Downloaded);
        assert_eq!(first.version, "1.0.0");
        assert_eq!(first.bundles(), vec!["TestKext.kext".to_string(), "TestPlugin.kext".to_string()]);

        let second = fetch_kext(&dl, &entry, &work, false, &cancel).await.unwrap();
        assert_eq!(second.status, ArtifactStatus::Cached);
        assert_eq!(second.bundles_dir, first.bundles_dir);
        assert_eq!(srv.hits.load(std::sync::atomic::Ordering::SeqCst), 1);

        // A wiped work dir is rebuilt from the download cache.
        std::fs::remove_dir_all(&work).unwrap();
        let third = fetch_kext(&dl, &entry, &work, false, &cancel).await.unwrap();
        assert_eq!(third.status, ArtifactStatus::Cached);
        assert!(third.bundles_dir.join("TestPlugin.kext/Contents/Info.plist").is_file());
        assert_eq!(srv.hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fetch_kext_rejects_wrong_hash_and_missing_bundles() {
        let tmp = TempDir::new();
        let zip = zip_of(&kext_items("", "TestKext", "org.example.TestKext"));
        let body = zip.clone();
        let srv = serve(move |_, _| Reply::ok(&body)).await;
        let dl = Downloader::new(tmp.0.join("cache")).unwrap();
        let cancel = CancellationToken::new();
        let url = leak(format!("{}/k.zip", srv.base));

        let wrong = test_entry(url, Some(leak(sha256_hex(b"something else"))), ArchiveKind::Zip);
        assert_eq!(fetch_kext(&dl, &wrong, &tmp.0, false, &cancel).await.unwrap_err().code, "SHA256_MISMATCH");

        // Hash matches but TestPlugin.kext is absent.
        let incomplete = test_entry(url, Some(leak(sha256_hex(&zip))), ArchiveKind::Zip);
        assert_eq!(fetch_kext(&dl, &incomplete, &tmp.0, false, &cancel).await.unwrap_err().code, "KEXT_BUNDLE_MISSING");
        assert!(std::fs::read_dir(tmp.0.join("kexts")).map(|rd| rd.count()).unwrap_or(0) == 0);
    }

    #[tokio::test]
    async fn fetch_kext_nested_artifact_without_hash() {
        let tmp = TempDir::new();
        let mut inner = kext_items("", "TestKext", "org.example.TestKext");
        inner.extend(kext_items("", "TestPlugin", "org.example.TestPlugin"));
        let inner_zip = zip_of(&inner);
        let outer = make_zip(&[
            Item::File("Debug/TestKext-1.0.0-DEBUG.zip", b"not even a zip"),
            Item::File("Release/TestKext-1.0.0-RELEASE.zip", &inner_zip),
        ]);
        let srv = serve(move |_, _| Reply::ok(&outer)).await;
        let dl = Downloader::new(tmp.0.join("cache")).unwrap();
        let entry = test_entry(leak(format!("{}/Artifacts.zip", srv.base)), None, ArchiveKind::NestedZip);
        let k = fetch_kext(&dl, &entry, &tmp.0.join("work"), false, &CancellationToken::new()).await.unwrap();
        assert!(k.bundles_dir.join("TestKext.kext/Contents/MacOS/TestKext").is_file());
    }

    #[tokio::test]
    async fn nightly_artifact_retries_not_found() {
        let tmp = TempDir::new();
        let mut inner = kext_items("", "TestKext", "org.example.TestKext");
        inner.extend(kext_items("", "TestPlugin", "org.example.TestPlugin"));
        let outer = make_zip(&[Item::File("Release/TestKext-1.0.0-RELEASE.zip", &zip_of(&inner))]);
        let srv = serve(move |_, n| if n == 0 { Reply::status(404) } else { Reply::ok(&outer) }).await;
        let mut dl = Downloader::new(tmp.0.join("cache")).unwrap();
        dl.retry.base_delay = std::time::Duration::from_millis(5);
        let entry = test_entry(leak(format!("{}/Artifacts.zip", srv.base)), None, ArchiveKind::NestedZip);
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let p2 = progress.clone();
        let cb = move |done: u64, _: Option<u64>| p2.store(done, std::sync::atomic::Ordering::SeqCst);
        let k = fetch_kext_with_progress(&dl, &entry, &tmp.0.join("work"), false, &CancellationToken::new(), Some(&cb))
            .await
            .unwrap();
        assert!(k.bundles_dir.join("TestKext.kext").is_dir());
        assert_eq!(srv.hits.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(progress.load(std::sync::atomic::Ordering::SeqCst) > 0);

        // Pinned archives fail on the first 404.
        let srv = serve(|_, _| Reply::status(404)).await;
        let pinned = test_entry(leak(format!("{}/k.zip", srv.base)), Some(leak(sha256_hex(b"k"))), ArchiveKind::Zip);
        let err = fetch_kext(&dl, &pinned, &tmp.0.join("work"), false, &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.code, "HTTP_NOT_FOUND");
        assert_eq!(srv.hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn latest_falls_back_to_pin_when_api_fails() {
        let tmp = TempDir::new();
        let mut items = kext_items("", "TestKext", "org.example.TestKext");
        items.extend(kext_items("", "TestPlugin", "org.example.TestPlugin"));
        let zip = zip_of(&items);
        let sha = leak(sha256_hex(&zip));
        let body = zip.clone();
        let srv = serve(move |req, _| {
            if req.path.starts_with("/repos/") {
                Reply::status(403).header("x-ratelimit-remaining", "0")
            } else {
                Reply::ok(&body)
            }
        })
        .await;
        let mut dl = Downloader::new(tmp.0.join("cache")).unwrap();
        dl.github_api = srv.base.clone();
        let mut entry = test_entry(leak(format!("{}/pinned.zip", srv.base)), Some(sha), ArchiveKind::Zip);
        entry.repo = "example/TestKext";
        entry.latest_asset_regex = r"^TestKext-[0-9.]+-RELEASE\.zip$";
        let k = fetch_kext(&dl, &entry, &tmp.0.join("work"), true, &CancellationToken::new()).await.unwrap();
        assert_eq!(k.version, "1.0.0");
        assert_eq!(k.status, ArtifactStatus::Downloaded);
    }

    #[tokio::test]
    async fn latest_release_is_used_when_available() {
        let tmp = TempDir::new();
        let mut items = kext_items("", "TestKext", "org.example.TestKext");
        items.extend(kext_items("", "TestPlugin", "org.example.TestPlugin"));
        let zip = zip_of(&items);
        let sha = sha256_hex(&zip);
        let body = zip.clone();
        let base_cell = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let bc = base_cell.clone();
        let srv = serve(move |req, _| {
            if req.path.starts_with("/repos/") {
                let base = bc.lock().unwrap().clone();
                let json = format!(
                    r#"{{"tag_name":"v2.0.0","html_url":"","assets":[
                        {{"name":"TestKext-2.0.0-DEBUG.zip","browser_download_url":"{base}/debug.zip","size":1}},
                        {{"name":"TestKext-2.0.0-RELEASE.zip","browser_download_url":"{base}/new.zip","size":1,"digest":"sha256:{sha}"}}]}}"#
                );
                Reply::ok(json.as_bytes())
            } else if req.path == "/new.zip" {
                Reply::ok(&body)
            } else {
                Reply::status(404)
            }
        })
        .await;
        *base_cell.lock().unwrap() = srv.base.clone();
        let mut dl = Downloader::new(tmp.0.join("cache")).unwrap();
        dl.github_api = srv.base.clone();
        let mut entry =
            test_entry(leak(format!("{}/old.zip", srv.base)), Some(leak(sha256_hex(b"old"))), ArchiveKind::Zip);
        entry.repo = "example/TestKext";
        entry.latest_asset_regex = r"^TestKext-[0-9.]+-RELEASE\.zip$";
        let k = fetch_kext(&dl, &entry, &tmp.0.join("work"), true, &CancellationToken::new()).await.unwrap();
        assert_eq!(k.version, "2.0.0");
        assert!(srv.requests().iter().all(|r| r.path != "/debug.zip" && r.path != "/old.zip"));
    }

    #[test]
    fn host_utility_paths() {
        let tmp = TempDir::new();
        let pkg = OpenCorePackage {
            version: "1.0.8".into(),
            debug: false,
            root: tmp.0.clone(),
            status: ArtifactStatus::Cached,
        };
        assert!(pkg.ocvalidate().is_none());
        for (dir, file) in
            [("ocvalidate", "ocvalidate"), ("ocvalidate", "ocvalidate.exe"), ("ocvalidate", "ocvalidate.linux")]
        {
            let p = tmp.0.join("Utilities").join(dir);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join(file), b"bin").unwrap();
        }
        let found = pkg.ocvalidate();
        if cfg!(any(target_os = "macos", target_os = "windows"))
            || cfg!(all(target_os = "linux", target_arch = "x86_64"))
        {
            assert!(found.is_some());
        }
        assert!(pkg.macserial().is_none());
    }

    /// Downloads the pinned OpenCore release and runs its ocvalidate on the
    /// bundled Sample.plist. Needs network access.
    #[tokio::test]
    #[ignore]
    async fn network_opencore_sample_validates() {
        let tmp = TempDir::new();
        let dl = Downloader::new(tmp.0.join("cache")).unwrap();
        let cancel = CancellationToken::new();
        let pkg = fetch_opencore(&dl, &tmp.0.join("work"), false, false, &cancel).await.unwrap();
        assert_eq!(pkg.version, "1.0.8");
        assert!(pkg.x64_efi().join("OC/OpenCore.efi").is_file());
        assert!(pkg.acpi_samples().join("SSDT-EC-USBX.aml").is_file());
        let ocv = pkg.ocvalidate().expect("ocvalidate for this host");
        let run = crate::services::ocvalidate::run_ocvalidate(&ocv, &pkg.sample_plist()).await.unwrap();
        let parsed = crate::services::ocvalidate::parse_output(&run.output);
        assert!(parsed.issues.is_empty(), "{}", run.output);
        assert!(parsed.no_issues, "{}", run.output);
        assert_eq!(run.status, Some(0));
        assert!(pkg.macserial().is_some());

        let debug = fetch_opencore(&dl, &tmp.0.join("work"), true, false, &cancel).await.unwrap();
        assert!(debug.debug);
        assert!(debug.root != pkg.root);

        let bin = fetch_ocbinarydata(&dl, &tmp.0.join("work"), &cancel).await.unwrap();
        assert!(bin.join("Drivers/HfsPlus.efi").is_file());
        assert!(bin.join("Resources/Image/Acidanthera/GoldenGate").is_dir());
    }

    /// Downloads every pinned catalog archive (except the ChefKiss ones and
    /// the unpinned nightly) and checks hash, extraction and bundle list.
    #[tokio::test]
    #[ignore]
    async fn network_catalog_pins_resolve() {
        let tmp = TempDir::new();
        let dl = Downloader::new(tmp.0.join("cache")).unwrap();
        let cancel = CancellationToken::new();
        let mut failures = Vec::new();
        for entry in kext_catalog::all() {
            if entry.pin.sha256.is_none() || entry.pin.url.contains("ChefKissInc") {
                continue;
            }
            match fetch_kext(&dl, entry, &tmp.0.join("work"), false, &cancel).await {
                Ok(k) => {
                    for b in entry.bundles {
                        if let Err(e) = install_bundle(&k, b, &tmp.0.join("EFI").join(entry.id)) {
                            failures.push(format!("{}: {b}: {e}", entry.id));
                        }
                    }
                }
                Err(e) => failures.push(format!("{}: {e}", entry.id)),
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
