//! OpenCore package, OcBinaryData and kext acquisition + safe extraction.

use std::path::{Path, PathBuf};

use crate::contracts::ArtifactStatus;
use crate::domain::kext_catalog::KextCatalogEntry;
use crate::error::AppError;
use crate::services::http::Downloader;
use crate::tasks::cancellation::CancellationToken;

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
        todo!("ocvalidate in {}", self.root.display())
    }
    /// Host-native macserial.
    pub fn macserial(&self) -> Option<PathBuf> {
        todo!("macserial in {}", self.root.display())
    }
}

pub async fn fetch_opencore(
    dl: &Downloader,
    work_dir: &Path,
    debug: bool,
    use_latest: bool,
    cancel: &CancellationToken,
) -> Result<OpenCorePackage, AppError> {
    todo!("fetch_opencore {} {debug} {use_latest} {} {}", work_dir.display(), dl.cache_dir.display(), cancel.is_cancelled())
}

/// Extracted OcBinaryData (Resources/, Drivers/).
pub async fn fetch_ocbinarydata(dl: &Downloader, work_dir: &Path, cancel: &CancellationToken) -> Result<PathBuf, AppError> {
    todo!("fetch_ocbinarydata {} {} {}", work_dir.display(), dl.cache_dir.display(), cancel.is_cancelled())
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

pub async fn fetch_kext(
    dl: &Downloader,
    entry: &KextCatalogEntry,
    work_dir: &Path,
    use_latest: bool,
    cancel: &CancellationToken,
) -> Result<FetchedKext, AppError> {
    todo!("fetch_kext {} {} {use_latest} {} {}", entry.id, work_dir.display(), dl.cache_dir.display(), cancel.is_cancelled())
}

/// Copy one top-level bundle from a fetched archive into EFI/OC/Kexts.
pub fn install_bundle(fetched: &FetchedKext, bundle: &str, kexts_dir: &Path) -> Result<(), AppError> {
    todo!("install_bundle {} {bundle} {}", fetched.catalog_id, kexts_dir.display())
}

/// Zip-slip-safe extraction (rejects absolute paths, `..`, drive prefixes,
/// symlinks; skips `__MACOSX` and `.dSYM`; caps total size). Recurses into a
/// nested `*-RELEASE.zip` when `nested_release` is true.
pub fn extract_zip(bytes: &[u8], dest: &Path, nested_release: bool) -> Result<(), AppError> {
    todo!("extract_zip {} {} {nested_release}", bytes.len(), dest.display())
}
