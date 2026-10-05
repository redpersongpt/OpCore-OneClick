//! HTTP client with timeouts, retries with backoff, cancellation, progress,
//! resumable downloads, an on-disk content cache keyed by SHA-256, and GitHub
//! release lookup with rate-limit detection.

use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::tasks::cancellation::CancellationToken;

pub type ProgressFn<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

pub struct Downloader {
    pub client: reqwest::Client,
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// GitHub-provided "sha256:..." digest when present.
    pub digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub assets: Vec<ReleaseAsset>,
    pub html_url: String,
    pub body: Option<String>,
}

impl Downloader {
    pub fn new(cache_dir: PathBuf) -> Result<Self, AppError> {
        todo!("Downloader::new {}", cache_dir.display())
    }

    /// Download `url` fully into memory. If `sha256` is given the bytes are
    /// verified and served from / stored in the cache.
    pub async fn fetch_bytes(
        &self,
        url: &str,
        sha256: Option<&str>,
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> Result<Vec<u8>, AppError> {
        todo!("fetch_bytes {url} {sha256:?} {} {}", cancel.is_cancelled(), progress.is_some())
    }

    /// Stream `url` to `dest` with HTTP Range resume, retries and progress.
    pub async fn fetch_to_file(
        &self,
        url: &str,
        dest: &Path,
        headers: &[(&str, &str)],
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> Result<u64, AppError> {
        todo!("fetch_to_file {url} {} {} {} {}", dest.display(), headers.len(), cancel.is_cancelled(), progress.is_some())
    }

    /// GET /repos/{repo}/releases/latest (60 req/h unauthenticated; returns a
    /// recoverable RATE_LIMITED error with reset time on 403/429).
    pub async fn github_latest_release(&self, repo: &str) -> Result<ReleaseInfo, AppError> {
        todo!("github_latest_release {repo}")
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
