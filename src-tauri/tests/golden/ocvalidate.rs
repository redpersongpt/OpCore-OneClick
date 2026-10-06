//! OpenCore 1.0.8 tools for the ocvalidate pass: downloaded once through
//! `services::artifacts` into a cache under the target directory. A local
//! copy of the release zip (`OPENCORE_RELEASE_ZIP`) seeds that cache, and
//! `OCVALIDATE` points at an ocvalidate binary directly.

use std::path::{Path, PathBuf};

use app_lib::domain::kext_catalog;
use app_lib::services::artifacts::{self, OpenCorePackage};
use app_lib::services::http::Downloader;
use app_lib::tasks::cancellation::CancellationToken;

pub fn cache_root() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-opencore")
}

pub fn downloader() -> Downloader {
    Downloader::new(cache_root().join("downloads")).expect("HTTP client")
}

/// The pinned OpenCore release, extracted (from the cache when present).
pub async fn opencore(dl: &Downloader) -> OpenCorePackage {
    let pin = kext_catalog::opencore_release();
    if let (Some(zip), Some(sha)) = (std::env::var_os("OPENCORE_RELEASE_ZIP"), pin.sha256) {
        if let Some(cached) = dl.cache_path(sha).filter(|p| !p.is_file()) {
            std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
            std::fs::copy(&zip, &cached).expect("copy OPENCORE_RELEASE_ZIP into the cache");
        }
    }
    artifacts::fetch_opencore(
        dl,
        &cache_root().join("work"),
        false,
        false,
        &CancellationToken::new(),
    )
    .await
    .expect("OpenCore 1.0.8 package")
}

/// AMD_Vanilla patches.plist for `pin` (verified, cached).
pub async fn fetch(dl: &Downloader, pin: kext_catalog::Pin) -> Vec<u8> {
    dl.fetch_bytes(pin.url, pin.sha256, &CancellationToken::new(), None)
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", pin.url))
}

pub fn run(bin: &Path, config: &Path) -> (bool, String) {
    let out = std::process::Command::new(bin)
        .arg(config)
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (text.contains("No issues found"), text)
}
