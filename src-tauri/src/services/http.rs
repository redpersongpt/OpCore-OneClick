//! HTTP client with timeouts, retries with backoff, cancellation, progress,
//! resumable downloads, an on-disk content cache keyed by SHA-256, and GitHub
//! release lookup with rate-limit detection.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE, RETRY_AFTER,
};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::AppError;
use crate::tasks::cancellation::CancellationToken;

pub type ProgressFn<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// Default GitHub REST endpoint.
pub const GITHUB_API: &str = "https://api.github.com";

/// In-memory downloads (archives) larger than this are refused.
const MAX_IN_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
/// How often a running transfer looks at its cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(100);
/// Longest server-requested `Retry-After` we are willing to wait.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// Exponential backoff for transient failures (5xx, 408/429, timeouts,
/// connection resets). 4xx answers other than 408/429 are never retried.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 3, base_delay: Duration::from_millis(750), max_delay: Duration::from_secs(8) }
    }
}

impl RetryPolicy {
    /// Delay before retry number `attempt` (0-based), with ±20 % jitter.
    pub fn delay(&self, attempt: u32) -> Duration {
        let exp = self.base_delay.saturating_mul(1u32 << attempt.min(16));
        let jitter = 0.8 + 0.4 * rand::random::<f64>();
        // try_from: a huge configured delay must saturate, not panic.
        Duration::try_from_secs_f64(exp.as_secs_f64() * jitter).unwrap_or(self.max_delay).min(self.max_delay)
    }
}

pub struct Downloader {
    pub client: reqwest::Client,
    pub cache_dir: PathBuf,
    pub retry: RetryPolicy,
    /// Base URL of the GitHub REST API (overridable for mirrors and tests).
    pub github_api: String,
}

#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// GitHub-provided "sha256:..." digest when present.
    pub digest: Option<String>,
}

impl ReleaseAsset {
    /// The SHA-256 from `digest`, lowercase, when GitHub published one.
    pub fn sha256(&self) -> Option<String> {
        let hex = self.digest.as_deref()?.strip_prefix("sha256:")?.to_ascii_lowercase();
        is_sha256_hex(&hex).then_some(hex)
    }
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub assets: Vec<ReleaseAsset>,
    pub html_url: String,
    pub body: Option<String>,
}

/// Bytes returned by [`Downloader::fetch_verified`].
#[derive(Debug, Clone)]
pub struct FetchedBytes {
    pub bytes: Vec<u8>,
    /// Lowercase hex SHA-256 of `bytes`.
    pub sha256: String,
    /// Served from the on-disk cache without touching the network.
    pub from_cache: bool,
}

/// Outcome of one attempt inside a retry loop.
enum Step<T> {
    Done(T),
    Retry(AppError, Option<Duration>),
    Fail(AppError),
}

impl Downloader {
    pub fn new(cache_dir: PathBuf) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::new("HTTP_CLIENT_ERROR", format!("Could not create the HTTP client: {e}")))?;
        Ok(Self::with_client(client, cache_dir))
    }

    pub fn with_client(client: reqwest::Client, cache_dir: PathBuf) -> Self {
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            tracing::warn!(dir = %cache_dir.display(), error = %e, "cannot create download cache directory");
        }
        Self { client, cache_dir, retry: RetryPolicy::default(), github_api: GITHUB_API.to_string() }
    }

    /// Cache location for a content hash (None if `sha256` is not a hash).
    pub fn cache_path(&self, sha256: &str) -> Option<PathBuf> {
        let hex = sha256.trim().to_ascii_lowercase();
        is_sha256_hex(&hex).then(|| self.cache_dir.join(hex))
    }

    /// True when a verified copy of `sha256` is in the cache.
    pub fn is_cached(&self, sha256: &str) -> bool {
        self.cache_path(sha256).is_some_and(|p| p.is_file())
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
        self.fetch_verified(url, sha256, cancel, progress).await.map(|f| f.bytes)
    }

    /// Like [`fetch_bytes`](Self::fetch_bytes), also reporting the computed
    /// hash and whether the cache answered.
    pub async fn fetch_verified(
        &self,
        url: &str,
        sha256: Option<&str>,
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> Result<FetchedBytes, AppError> {
        let expected = match sha256 {
            Some(s) => {
                let hex = s.trim().to_ascii_lowercase();
                if !is_sha256_hex(&hex) {
                    return Err(AppError::new("INVALID_SHA256", format!("'{s}' is not a SHA-256 hash")));
                }
                Some(hex)
            }
            None => None,
        };
        cancel.check()?;

        if let Some(hex) = &expected {
            if let Some(bytes) = self.read_cache(hex).await {
                if let Some(cb) = progress {
                    cb(bytes.len() as u64, Some(bytes.len() as u64));
                }
                return Ok(FetchedBytes { bytes, sha256: hex.clone(), from_cache: true });
            }
        }

        let bytes = self.download_to_memory(url, cancel, progress).await?;
        let actual = sha256_hex(&bytes);
        if let Some(hex) = &expected {
            if *hex != actual {
                return Err(AppError::new(
                    "SHA256_MISMATCH",
                    format!("Integrity check failed for {url}: expected SHA-256 {hex}, got {actual}"),
                )
                .with_suggestion("The file on the server changed since this version was pinned. Update the app, or enable \"use latest releases\".")
                .with_context(json!({ "url": url, "expected": hex, "actual": actual })));
            }
            self.write_cache(hex, &bytes).await;
        }
        Ok(FetchedBytes { bytes, sha256: actual, from_cache: false })
    }

    /// Stream `url` to `dest` with HTTP Range resume, retries and progress.
    ///
    /// Data goes to `<dest>.part` first and is renamed when complete, so an
    /// interrupted transfer (error, cancellation, app exit) resumes from the
    /// partial file on the next call. The server's ETag/Last-Modified is kept
    /// next to it and sent as `If-Range`, so a file that changed on the server
    /// is downloaded again instead of being spliced. Returns the final size.
    pub async fn fetch_to_file(
        &self,
        url: &str,
        dest: &Path,
        headers: &[(&str, &str)],
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> Result<u64, AppError> {
        let part = part_path(dest)?;
        let validator = validator_path(&part);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let extra = header_map(headers)?;
        // Some servers mishandle ranges; after one bad answer stop asking.
        let mut range_ok = true;
        // Furthest the partial file ever got: retries only reset the attempt
        // counter when a transfer gets past it.
        let mut high_water: u64 = 0;
        let mut attempt: u32 = 0;
        loop {
            cancel.check()?;
            let offset = if range_ok { file_len(&part).await } else { 0 };
            let step = self.fetch_to_file_once(url, &part, &validator, offset, &extra, cancel, progress).await;
            let (err, hint, reached) = match step {
                FileStep::Complete(total) => {
                    replace_file(&part, dest).await?;
                    let _ = remove_if_exists(&validator).await;
                    return Ok(total);
                }
                FileStep::RestartWithoutRange if offset == 0 => {
                    return Err(AppError::new(
                        "HTTP_BAD_RANGE",
                        format!("{url}: the server sent a partial response that was not requested"),
                    ));
                }
                FileStep::RestartWithoutRange => {
                    tracing::debug!(url, "server mishandled the resume range; restarting from zero");
                    range_ok = false;
                    remove_if_exists(&part).await?;
                    remove_if_exists(&validator).await?;
                    continue;
                }
                FileStep::StalePart => {
                    tracing::debug!(url, "partial file does not match the remote file; restarting from zero");
                    remove_if_exists(&part).await?;
                    remove_if_exists(&validator).await?;
                    continue;
                }
                FileStep::Fail(e) => return Err(e),
                FileStep::Retry { error, retry_after, reached } => (error, retry_after, reached),
            };
            if reached > high_water {
                // A flaky link that keeps making progress is not a dead server.
                high_water = reached;
                attempt = 0;
            } else if attempt >= self.retry.max_retries {
                return Err(exhausted(err, url));
            } else {
                attempt += 1;
            }
            let delay = hint.unwrap_or_else(|| self.retry.delay(attempt.saturating_sub(1)));
            tracing::warn!(url, attempt, error = %err, delay_ms = delay.as_millis() as u64, "download interrupted, retrying");
            sleep_cancellable(delay, cancel).await?;
        }
    }

    /// GET /repos/{repo}/releases/latest (60 req/h unauthenticated; returns a
    /// recoverable RATE_LIMITED error with reset time on 403/429).
    pub async fn github_latest_release(&self, repo: &str) -> Result<ReleaseInfo, AppError> {
        if !is_valid_repo(repo) {
            return Err(AppError::new("INVALID_REPO", format!("'{repo}' is not a GitHub owner/repo")));
        }
        let url = format!("{}/repos/{repo}/releases/latest", self.github_api.trim_end_matches('/'));
        let token = github_token().filter(|_| self.github_api.trim_end_matches('/') == GITHUB_API);
        let mut attempt: u32 = 0;
        loop {
            let mut req = self
                .client
                .get(&url)
                .header(ACCEPT, "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            let step = match req.send().await {
                Err(e) => transport_step(e, &url),
                Ok(resp) => self.release_step(resp, repo, &url).await,
            };
            match step {
                Step::Done(info) => return Ok(info),
                Step::Fail(e) => return Err(e),
                Step::Retry(e, hint) => {
                    if attempt >= self.retry.max_retries {
                        return Err(exhausted(e, &url));
                    }
                    let delay = hint.unwrap_or_else(|| self.retry.delay(attempt));
                    tracing::warn!(repo, attempt, error = %e, "GitHub API request failed, retrying");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn release_step(&self, resp: Response, repo: &str, url: &str) -> Step<ReleaseInfo> {
        let status = resp.status();
        if status.is_success() {
            return match resp.json::<GhRelease>().await {
                Ok(r) => Step::Done(r.into()),
                Err(e) if e.is_decode() => Step::Fail(AppError::new(
                    "GITHUB_BAD_RESPONSE",
                    format!("Unexpected GitHub API response for {repo}: {e}"),
                )),
                Err(e) => transport_step(e, url),
            };
        }
        if let Some(err) = rate_limit_error(status, resp.headers(), repo) {
            return Step::Fail(err);
        }
        match status {
            StatusCode::NOT_FOUND => Step::Fail(
                AppError::new("GITHUB_RELEASE_NOT_FOUND", format!("{repo} has no published release"))
                    .with_context(json!({ "repo": repo })),
            ),
            s if is_retryable_status(s) => Step::Retry(http_error(s, url), retry_after(resp.headers())),
            s => Step::Fail(http_error(s, url)),
        }
    }

    async fn download_to_memory(
        &self,
        url: &str,
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> Result<Vec<u8>, AppError> {
        let mut attempt: u32 = 0;
        loop {
            cancel.check()?;
            match self.download_once(url, cancel, progress).await {
                Step::Done(bytes) => return Ok(bytes),
                Step::Fail(e) => return Err(e),
                Step::Retry(e, hint) => {
                    if attempt >= self.retry.max_retries {
                        return Err(exhausted(e, url));
                    }
                    let delay = hint.unwrap_or_else(|| self.retry.delay(attempt));
                    tracing::warn!(url, attempt, error = %e, "download failed, retrying");
                    sleep_cancellable(delay, cancel).await?;
                    attempt += 1;
                }
            }
        }
    }

    async fn download_once(
        &self,
        url: &str,
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> Step<Vec<u8>> {
        let resp = match send_cancellable(self.client.get(url), cancel).await {
            Ok(r) => r,
            Err(SendError::Cancelled(e)) => return Step::Fail(e),
            Err(SendError::Transport(e)) => return transport_step(e, url),
        };
        let status = resp.status();
        if !status.is_success() {
            return status_step(status, resp.headers(), url);
        }
        let total = resp.content_length();
        if total.is_some_and(|t| t > MAX_IN_MEMORY_BYTES) {
            return Step::Fail(too_large(url));
        }
        let capacity = total.unwrap_or(0).min(64 * 1024 * 1024) as usize;
        let mut buf = Vec::with_capacity(capacity);
        let mut stream = resp.bytes_stream();
        loop {
            match next_chunk(&mut stream, cancel).await {
                Err(SendError::Cancelled(e)) => return Step::Fail(e),
                Err(SendError::Transport(e)) => return transport_step(e, url),
                Ok(None) => break,
                Ok(Some(chunk)) => {
                    if buf.len() as u64 + chunk.len() as u64 > MAX_IN_MEMORY_BYTES {
                        return Step::Fail(too_large(url));
                    }
                    buf.extend_from_slice(&chunk);
                    if let Some(cb) = progress {
                        cb(buf.len() as u64, total);
                    }
                }
            }
        }
        if let Some(t) = total {
            if buf.len() as u64 != t {
                return Step::Retry(
                    AppError::new("DOWNLOAD_TRUNCATED", format!("{url}: received {} of {t} bytes", buf.len()))
                        .recoverable(),
                    None,
                );
            }
        }
        Step::Done(buf)
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_to_file_once(
        &self,
        url: &str,
        part: &Path,
        validator: &Path,
        offset: u64,
        extra: &HeaderMap,
        cancel: &CancellationToken,
        progress: Option<ProgressFn<'_>>,
    ) -> FileStep {
        let mut req = self.client.get(url).headers(extra.clone());
        if offset > 0 {
            req = req.header(RANGE, format!("bytes={offset}-"));
            if let Some(v) = read_validator(validator).await {
                req = req.header(IF_RANGE, v);
            }
        }
        let resp = match send_cancellable(req, cancel).await {
            Ok(r) => r,
            Err(SendError::Cancelled(e)) => return FileStep::Fail(e),
            Err(SendError::Transport(e)) => return FileStep::from_step(transport_step(e, url), offset),
        };
        let status = resp.status();
        let (start, total) = match status {
            StatusCode::PARTIAL_CONTENT if offset > 0 => match parse_content_range(resp.headers()) {
                Some(cr) if cr.start == offset => (offset, cr.total.or(resp.content_length().map(|l| offset + l))),
                _ => return FileStep::RestartWithoutRange,
            },
            StatusCode::PARTIAL_CONTENT => match parse_content_range(resp.headers()) {
                Some(cr) if cr.start == 0 => (0, cr.total.or(resp.content_length())),
                _ => return FileStep::RestartWithoutRange,
            },
            StatusCode::RANGE_NOT_SATISFIABLE if offset > 0 => {
                // `bytes */<size>`: the partial file may already be complete;
                // otherwise it belongs to a different (changed) remote file.
                return match unsatisfied_range_total(resp.headers()) {
                    Some(size) if size == offset => {
                        if let Some(cb) = progress {
                            cb(offset, Some(offset));
                        }
                        FileStep::Complete(offset)
                    }
                    _ => FileStep::StalePart,
                };
            }
            s if s.is_success() => (0, resp.content_length()),
            s => return FileStep::from_step(status_step(s, resp.headers(), url), offset),
        };

        if start == 0 {
            store_validator(validator, resp.headers()).await;
        }
        let file = if start == 0 {
            tokio::fs::File::create(part).await
        } else {
            tokio::fs::OpenOptions::new().append(true).open(part).await
        };
        let mut file = match file {
            Ok(f) => f,
            Err(e) => return FileStep::Fail(io_error(e, part)),
        };
        let mut written = start;
        let mut stream = resp.bytes_stream();
        loop {
            let chunk = match next_chunk(&mut stream, cancel).await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(SendError::Cancelled(e)) => {
                    let _ = file.flush().await;
                    return FileStep::Fail(e);
                }
                Err(SendError::Transport(e)) => {
                    let _ = file.flush().await;
                    return FileStep::from_step(transport_step(e, url), written);
                }
            };
            if let Err(e) = file.write_all(&chunk).await {
                return FileStep::Fail(io_error(e, part));
            }
            written += chunk.len() as u64;
            if let Some(cb) = progress {
                cb(written, total);
            }
        }
        if let Err(e) = file.flush().await {
            return FileStep::Fail(io_error(e, part));
        }
        if let Err(e) = file.sync_all().await {
            return FileStep::Fail(io_error(e, part));
        }
        drop(file);
        if let Some(t) = total {
            if written != t {
                return FileStep::Retry {
                    error: AppError::new("DOWNLOAD_TRUNCATED", format!("{url}: received {written} of {t} bytes"))
                        .recoverable(),
                    retry_after: None,
                    reached: written,
                };
            }
        }
        FileStep::Complete(written)
    }

    async fn read_cache(&self, sha256: &str) -> Option<Vec<u8>> {
        let path = self.cache_path(sha256)?;
        let bytes = tokio::fs::read(&path).await.ok()?;
        if sha256_hex(&bytes) == sha256 {
            tracing::debug!(sha256, "download cache hit");
            return Some(bytes);
        }
        tracing::warn!(path = %path.display(), "cached file is corrupt, removing it");
        let _ = tokio::fs::remove_file(&path).await;
        None
    }

    /// Atomic cache insert (temp file + rename). Failures only cost a re-download.
    async fn write_cache(&self, sha256: &str, bytes: &[u8]) {
        let Some(path) = self.cache_path(sha256) else { return };
        let tmp = self.cache_dir.join(format!(".{sha256}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = async {
            tokio::fs::create_dir_all(&self.cache_dir).await?;
            let mut f = tokio::fs::File::create(&tmp).await?;
            f.write_all(bytes).await?;
            f.sync_all().await?;
            drop(f);
            tokio::fs::rename(&tmp, &path).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(path = %path.display(), error = %e, "could not store download in cache");
            let _ = tokio::fs::remove_file(&tmp).await;
        }
    }
}

enum FileStep {
    Complete(u64),
    /// The server answered a range request with a wrong range: stop asking.
    RestartWithoutRange,
    /// 416 for a partial file that does not fit the remote size.
    StalePart,
    /// `reached`: size of the partial file when the attempt ended.
    Retry {
        error: AppError,
        retry_after: Option<Duration>,
        reached: u64,
    },
    Fail(AppError),
}

impl FileStep {
    fn from_step(step: Step<()>, reached: u64) -> Self {
        match step {
            Step::Done(()) => FileStep::Fail(AppError::new("INTERNAL", "unexpected download state")),
            Step::Retry(error, retry_after) => FileStep::Retry { error, retry_after, reached },
            Step::Fail(e) => FileStep::Fail(e),
        }
    }
}

enum SendError {
    Cancelled(AppError),
    Transport(reqwest::Error),
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn user_agent() -> String {
    format!("OpCore-OneClick/{}", env!("CARGO_PKG_VERSION"))
}

/// Optional token for the GitHub API (raises the limit to 5000 req/h).
fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn is_valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |s: Option<&str>| {
        s.is_some_and(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Wait until `cancel` fires (polling; the token has no async notifier).
async fn cancelled(cancel: &CancellationToken) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

/// Sleep for `delay`, returning early with TASK_CANCELLED when `cancel` fires.
pub(crate) async fn sleep_cancellable(delay: Duration, cancel: &CancellationToken) -> Result<(), AppError> {
    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(()),
        _ = cancelled(cancel) => cancel.check(),
    }
}

async fn send_cancellable(req: reqwest::RequestBuilder, cancel: &CancellationToken) -> Result<Response, SendError> {
    tokio::select! {
        r = req.send() => r.map_err(SendError::Transport),
        _ = cancelled(cancel) => Err(SendError::Cancelled(cancelled_error())),
    }
}

async fn next_chunk<S, B>(stream: &mut S, cancel: &CancellationToken) -> Result<Option<B>, SendError>
where
    S: futures_util::Stream<Item = reqwest::Result<B>> + Unpin,
{
    if cancel.is_cancelled() {
        return Err(SendError::Cancelled(cancelled_error()));
    }
    tokio::select! {
        c = stream.next() => match c {
            None => Ok(None),
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(SendError::Transport(e)),
        },
        _ = cancelled(cancel) => Err(SendError::Cancelled(cancelled_error())),
    }
}

fn cancelled_error() -> AppError {
    AppError::new("TASK_CANCELLED", "Operation was cancelled by user")
}

fn transport_step<T>(err: reqwest::Error, url: &str) -> Step<T> {
    let kind = if err.is_timeout() {
        "timed out"
    } else if err.is_connect() {
        "could not connect"
    } else {
        "connection failed"
    };
    let app = AppError::new("NETWORK_ERROR", format!("{url}: {kind} ({err})"))
        .recoverable()
        .with_context(json!({ "url": url }));
    if err.is_builder() || err.is_redirect() {
        Step::Fail(app)
    } else {
        Step::Retry(app, None)
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::TOO_MANY_REQUESTS
}

fn status_step<T>(status: StatusCode, headers: &HeaderMap, url: &str) -> Step<T> {
    if is_retryable_status(status) {
        Step::Retry(http_error(status, url), retry_after(headers))
    } else {
        Step::Fail(http_error(status, url))
    }
}

fn http_error(status: StatusCode, url: &str) -> AppError {
    let code = if status == StatusCode::NOT_FOUND { "HTTP_NOT_FOUND" } else { "HTTP_ERROR" };
    let err = AppError::new(code, format!("GET {url} returned HTTP {status}"))
        .with_context(json!({ "url": url, "status": status.as_u16() }));
    if is_retryable_status(status) {
        err.recoverable()
    } else {
        err
    }
}

fn exhausted(err: AppError, url: &str) -> AppError {
    let message = format!("{} (gave up after several attempts)", err.message);
    let mut out = AppError::new(err.code, message)
        .recoverable()
        .with_suggestion("Check the internet connection (proxy, firewall, VPN) and try again.");
    out.context = err.context.or_else(|| Some(json!({ "url": url })));
    out
}

fn too_large(url: &str) -> AppError {
    AppError::new("DOWNLOAD_TOO_LARGE", format!("{url} is larger than {} MiB", MAX_IN_MEMORY_BYTES >> 20))
}

fn io_error(err: io::Error, path: &Path) -> AppError {
    AppError::new("IO_ERROR", format!("{}: {err}", path.display()))
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs: u64 = headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER))
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// GitHub signals rate limiting with 403/429 plus `x-ratelimit-remaining: 0`
/// (primary limit) or `retry-after` (secondary limit).
fn rate_limit_error(status: StatusCode, headers: &HeaderMap, repo: &str) -> Option<AppError> {
    if status != StatusCode::FORBIDDEN && status != StatusCode::TOO_MANY_REQUESTS {
        return None;
    }
    let remaining = header_u64(headers, "x-ratelimit-remaining");
    let limited = status == StatusCode::TOO_MANY_REQUESTS || remaining == Some(0) || headers.contains_key(RETRY_AFTER);
    if !limited {
        return None;
    }
    let limit = header_u64(headers, "x-ratelimit-limit");
    let reset = header_u64(headers, "x-ratelimit-reset")
        .or_else(|| retry_after(headers).map(|d| chrono::Utc::now().timestamp().max(0) as u64 + d.as_secs()));
    let reset_text = reset
        .and_then(|r| chrono::DateTime::from_timestamp(r as i64, 0))
        .map(|t| format!("; it resets at {} UTC", t.format("%H:%M")))
        .unwrap_or_default();
    Some(
        AppError::new("RATE_LIMITED", format!("GitHub API rate limit reached{reset_text}"))
            .recoverable()
            .with_suggestion(
                "Wait for the limit to reset, set a GITHUB_TOKEN environment variable, or turn off \"use latest releases\" to build with the pinned versions.",
            )
            .with_context(json!({ "repo": repo, "resetAt": reset, "remaining": remaining, "limit": limit })),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    total: Option<u64>,
}

/// Parse `Content-Range: bytes <start>-<end>/<total|*>`.
fn parse_content_range(headers: &HeaderMap) -> Option<ContentRange> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?.trim();
    let rest = v.strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    if end < start {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse::<u64>().ok()?),
    };
    Some(ContentRange { start, total })
}

/// Total size from a 416 answer (`Content-Range: bytes */<total>`).
fn unsatisfied_range_total(headers: &HeaderMap) -> Option<u64> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?.trim();
    v.strip_prefix("bytes")?.trim_start().strip_prefix("*/")?.trim().parse().ok()
}

fn header_map(headers: &[(&str, &str)]) -> Result<HeaderMap, AppError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let n = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::new("INVALID_HEADER", format!("Invalid HTTP header name '{name}'")))?;
        let v = HeaderValue::from_str(value)
            .map_err(|_| AppError::new("INVALID_HEADER", format!("Invalid value for HTTP header '{name}'")))?;
        map.insert(n, v);
    }
    Ok(map)
}

fn part_path(dest: &Path) -> Result<PathBuf, AppError> {
    let name = dest
        .file_name()
        .ok_or_else(|| AppError::new("INVALID_PATH", format!("{} is not a file path", dest.display())))?;
    let mut part = name.to_os_string();
    part.push(".part");
    Ok(dest.with_file_name(part))
}

/// Sidecar holding the resume validator of a `.part` file.
fn validator_path(part: &Path) -> PathBuf {
    let mut name = part.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".validator");
    part.with_file_name(name)
}

/// A strong ETag, else Last-Modified (weak ETags are not allowed in If-Range).
fn resume_validator(headers: &HeaderMap) -> Option<String> {
    let etag = headers.get(ETAG).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(e) = etag.filter(|e| e.starts_with('"') && e.len() > 1) {
        return Some(e.to_string());
    }
    headers
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn read_validator(path: &Path) -> Option<HeaderValue> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    HeaderValue::from_str(text.trim()).ok().filter(|v| !v.is_empty())
}

/// Remember the validator of a transfer that starts from zero (or forget a
/// stale one when the server sends none). Failures only cost resumability.
async fn store_validator(path: &Path, headers: &HeaderMap) {
    let result = match resume_validator(headers) {
        Some(v) => tokio::fs::write(path, v).await,
        None => match tokio::fs::remove_file(path).await {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = result {
        tracing::debug!(path = %path.display(), error = %e, "could not update the resume validator");
    }
}

async fn file_len(path: &Path) -> u64 {
    tokio::fs::metadata(path).await.map(|m| m.len()).unwrap_or(0)
}

async fn remove_if_exists(path: &Path) -> Result<(), AppError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(e, path)),
    }
}

async fn replace_file(from: &Path, to: &Path) -> Result<(), AppError> {
    // rename() replaces an existing file on Unix and Windows alike. On
    // Windows a virus scanner that still has the fresh file open makes it
    // fail for a moment, so try a few times before giving up.
    let mut attempt = 0;
    loop {
        match tokio::fs::rename(from, to).await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 5 && e.kind() != io::ErrorKind::NotFound => {
                attempt += 1;
                tracing::debug!(path = %to.display(), error = %e, attempt, "rename failed, retrying");
                tokio::time::sleep(Duration::from_millis(200 * attempt)).await;
            }
            Err(e) => return Err(io_error(e, to)),
        }
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

impl From<GhRelease> for ReleaseInfo {
    fn from(r: GhRelease) -> Self {
        ReleaseInfo {
            tag: r.tag_name,
            html_url: r.html_url,
            body: r.body,
            assets: r
                .assets
                .into_iter()
                .map(|a| ReleaseAsset { name: a.name, url: a.browser_download_url, size: a.size, digest: a.digest })
                .collect(),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    //! Minimal HTTP/1.1 server on 127.0.0.1 for exercising the downloader
    //! without network access.

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Clone, Debug)]
    pub struct Reply {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
        /// Close the connection after this many body bytes (simulated reset).
        pub cut_after: Option<usize>,
        /// Send this many body bytes, then hang (simulated stalled transfer).
        pub stall_after: Option<usize>,
    }

    impl Reply {
        pub fn ok(body: &[u8]) -> Self {
            Reply { status: 200, headers: vec![], body: body.to_vec(), cut_after: None, stall_after: None }
        }
        pub fn status(status: u16) -> Self {
            Reply { status, headers: vec![], body: Vec::new(), cut_after: None, stall_after: None }
        }
        pub fn header(mut self, k: &str, v: &str) -> Self {
            self.headers.push((k.to_string(), v.to_string()));
            self
        }
    }

    #[derive(Debug, Clone)]
    pub struct Seen {
        pub path: String,
        pub headers: HashMap<String, String>,
    }

    type Handler = dyn Fn(&Seen, usize) -> Reply + Send + Sync;

    pub struct TestServer {
        pub base: String,
        pub hits: Arc<AtomicUsize>,
        pub seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl TestServer {
        pub fn requests(&self) -> Vec<Seen> {
            self.seen.lock().map(|s| s.clone()).unwrap_or_default()
        }
    }

    /// `handler(request, n)` answers the n-th request (0-based).
    pub async fn serve(handler: impl Fn(&Seen, usize) -> Reply + Send + Sync + 'static) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let (h2, s2) = (hits.clone(), seen.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (handler, hits, seen) = (handler.clone(), h2.clone(), s2.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let mut lines = text.split("\r\n");
                    let path = lines.next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
                    let headers = lines
                        .take_while(|l| !l.is_empty())
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                        .collect();
                    let req = Seen { path, headers };
                    let n = hits.fetch_add(1, Ordering::SeqCst);
                    if let Ok(mut s) = seen.lock() {
                        s.push(req.clone());
                    }
                    let reply = handler(&req, n);
                    let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
                    let has_len = reply.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
                    for (k, v) in &reply.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    if !has_len {
                        head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                    }
                    head.push_str("\r\n");
                    let _ = sock.write_all(head.as_bytes()).await;
                    let body = match reply.cut_after.or(reply.stall_after) {
                        Some(n) => &reply.body[..n.min(reply.body.len())],
                        None => &reply.body[..],
                    };
                    let _ = sock.write_all(body).await;
                    if reply.stall_after.is_some() {
                        let _ = sock.flush().await;
                        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                    }
                    let _ = sock.flush().await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        TestServer { base: format!("http://{addr}"), hits, seen }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use super::test_server::{serve, Reply};
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-http-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn downloader(dir: &Path) -> Downloader {
        let mut dl = Downloader::new(dir.join("cache")).unwrap();
        dl.retry =
            RetryPolicy { max_retries: 3, base_delay: Duration::from_millis(5), max_delay: Duration::from_millis(20) };
        dl
    }

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn sha256_helpers() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(is_sha256_hex(&sha256_hex(b"")));
        assert!(!is_sha256_hex("ABC"));
        let asset = ReleaseAsset {
            name: "a".into(),
            url: "u".into(),
            size: 1,
            digest: Some(format!("sha256:{}", sha256_hex(b"x").to_uppercase())),
        };
        assert_eq!(asset.sha256(), Some(sha256_hex(b"x")));
        let bad = ReleaseAsset { digest: Some("md5:abc".into()), ..asset };
        assert_eq!(bad.sha256(), None);
    }

    #[test]
    fn parses_content_range() {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 100-199/200"));
        assert_eq!(parse_content_range(&h), Some(ContentRange { start: 100, total: Some(200) }));
        h.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 5-9/*"));
        assert_eq!(parse_content_range(&h), Some(ContentRange { start: 5, total: None }));
        h.insert(CONTENT_RANGE, HeaderValue::from_static("bytes */200"));
        assert_eq!(parse_content_range(&h), None);
        assert_eq!(unsatisfied_range_total(&h), Some(200));
        h.insert(CONTENT_RANGE, HeaderValue::from_static("garbage"));
        assert_eq!(parse_content_range(&h), None);
    }

    #[test]
    fn repo_names_are_validated() {
        assert!(is_valid_repo("acidanthera/Lilu"));
        assert!(is_valid_repo("Carnations-Botanica/iBridged"));
        assert!(!is_valid_repo("acidanthera"));
        assert!(!is_valid_repo("a/b/c"));
        assert!(!is_valid_repo("../x"));
        assert!(!is_valid_repo("a/b?x=1"));
    }

    #[test]
    fn retry_delay_grows_and_is_capped() {
        let p = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
        };
        assert!(p.delay(0) <= Duration::from_millis(120));
        assert!(p.delay(1) >= Duration::from_millis(160));
        assert_eq!(p.delay(10), Duration::from_millis(500));
        // A policy with an absurd base delay saturates instead of panicking.
        let huge = RetryPolicy { max_retries: 1, base_delay: Duration::MAX, max_delay: Duration::from_secs(2) };
        assert_eq!(huge.delay(16), Duration::from_secs(2));
    }

    #[tokio::test]
    async fn cache_miss_then_hit() {
        let tmp = TempDir::new();
        let data = payload(10_000);
        let sha = sha256_hex(&data);
        let body = data.clone();
        let srv = serve(move |_, _| Reply::ok(&body)).await;
        let dl = downloader(&tmp.0);
        let cancel = CancellationToken::new();
        let url = format!("{}/a.zip", srv.base);

        let first = dl.fetch_verified(&url, Some(&sha), &cancel, None).await.unwrap();
        assert!(!first.from_cache);
        assert_eq!(first.bytes, data);
        assert!(dl.is_cached(&sha));

        let calls = Arc::new(Mutex::new(Vec::new()));
        let c2 = calls.clone();
        let cb = move |done: u64, total: Option<u64>| c2.lock().unwrap().push((done, total));
        let second = dl.fetch_verified(&url, Some(&sha.to_uppercase()), &cancel, Some(&cb)).await.unwrap();
        assert!(second.from_cache);
        assert_eq!(second.bytes, data);
        assert_eq!(srv.hits.load(Ordering::SeqCst), 1);
        assert_eq!(calls.lock().unwrap().last(), Some(&(10_000, Some(10_000))));
    }

    #[tokio::test]
    async fn corrupt_cache_entry_is_replaced() {
        let tmp = TempDir::new();
        let data = payload(512);
        let sha = sha256_hex(&data);
        let body = data.clone();
        let srv = serve(move |_, _| Reply::ok(&body)).await;
        let dl = downloader(&tmp.0);
        std::fs::write(dl.cache_path(&sha).unwrap(), b"tampered").unwrap();
        let got =
            dl.fetch_bytes(&format!("{}/x", srv.base), Some(&sha), &CancellationToken::new(), None).await.unwrap();
        assert_eq!(got, data);
        assert_eq!(std::fs::read(dl.cache_path(&sha).unwrap()).unwrap(), data);
    }

    #[tokio::test]
    async fn sha_mismatch_fails_and_is_not_cached() {
        let tmp = TempDir::new();
        let srv = serve(|_, _| Reply::ok(b"other bytes")).await;
        let dl = downloader(&tmp.0);
        let wanted = sha256_hex(b"expected bytes");
        let err = dl
            .fetch_bytes(&format!("{}/x", srv.base), Some(&wanted), &CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, "SHA256_MISMATCH");
        assert!(!dl.is_cached(&wanted));
        // A hash mismatch is not a transient error: one request only.
        assert_eq!(srv.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_server_errors_but_not_404() {
        let tmp = TempDir::new();
        let srv = serve(|_, n| if n < 2 { Reply::status(503) } else { Reply::ok(b"fine") }).await;
        let dl = downloader(&tmp.0);
        let got = dl.fetch_bytes(&format!("{}/x", srv.base), None, &CancellationToken::new(), None).await.unwrap();
        assert_eq!(got, b"fine");
        assert_eq!(srv.hits.load(Ordering::SeqCst), 3);

        let srv = serve(|_, _| Reply::status(404)).await;
        let err = dl.fetch_bytes(&format!("{}/x", srv.base), None, &CancellationToken::new(), None).await.unwrap_err();
        assert_eq!(err.code, "HTTP_NOT_FOUND");
        assert_eq!(srv.hits.load(Ordering::SeqCst), 1);

        let srv = serve(|_, _| Reply::status(500)).await;
        let err = dl.fetch_bytes(&format!("{}/x", srv.base), None, &CancellationToken::new(), None).await.unwrap_err();
        assert_eq!(err.code, "HTTP_ERROR");
        assert!(err.recoverable);
        assert_eq!(srv.hits.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn retries_truncated_bodies() {
        let tmp = TempDir::new();
        let data = payload(4096);
        let body = data.clone();
        let srv = serve(move |_, n| {
            let mut r = Reply::ok(&body);
            if n == 0 {
                r.cut_after = Some(1000);
            }
            r
        })
        .await;
        let dl = downloader(&tmp.0);
        let got = dl
            .fetch_bytes(&format!("{}/x", srv.base), Some(&sha256_hex(&data)), &CancellationToken::new(), None)
            .await
            .unwrap();
        assert_eq!(got, data);
        assert_eq!(srv.hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancelled_token_stops_before_request() {
        let tmp = TempDir::new();
        let srv = serve(|_, _| Reply::ok(b"x")).await;
        let dl = downloader(&tmp.0);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = dl.fetch_bytes(&format!("{}/x", srv.base), None, &cancel, None).await.unwrap_err();
        assert_eq!(err.code, "TASK_CANCELLED");
        assert_eq!(srv.hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn file_download_resumes_with_range() {
        let tmp = TempDir::new();
        let data = payload(5000);
        let body = data.clone();
        let srv = serve(move |req, n| {
            if n == 0 {
                let mut r = Reply::ok(&body);
                r.cut_after = Some(1500);
                return r;
            }
            let range = req.headers.get("range").cloned().unwrap_or_default();
            let start: usize = range.trim_start_matches("bytes=").trim_end_matches('-').parse().unwrap_or(0);
            Reply {
                status: 206,
                headers: vec![("Content-Range".into(), format!("bytes {start}-{}/{}", body.len() - 1, body.len()))],
                body: body[start..].to_vec(),
                cut_after: None,
                stall_after: None,
            }
        })
        .await;
        let dl = downloader(&tmp.0);
        let dest = tmp.0.join("out").join("BaseSystem.dmg");
        let n = dl
            .fetch_to_file(
                &format!("{}/big", srv.base),
                &dest,
                &[("Cookie", "AssetToken=abc")],
                &CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(n, 5000);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!part_path(&dest).unwrap().exists());
        let reqs = srv.requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].headers.get("cookie").map(String::as_str), Some("AssetToken=abc"));
        assert!(!reqs[0].headers.contains_key("range"));
        assert_eq!(reqs[1].headers.get("range").map(String::as_str), Some("bytes=1500-"));
    }

    #[tokio::test]
    async fn file_download_resume_sends_if_range_and_restarts_on_change() {
        let tmp = TempDir::new();
        let old = payload(4000);
        let new: Vec<u8> = payload(4500).into_iter().map(|b| b.wrapping_add(7)).collect();
        let (o, n2) = (old.clone(), new.clone());
        let srv = serve(move |_, n| {
            if n == 0 {
                // First transfer of the old file breaks off.
                let mut r = Reply::ok(&o).header("ETag", "\"v1\"");
                r.cut_after = Some(1200);
                r
            } else {
                // The file changed meanwhile: If-Range no longer matches, so
                // the whole new file comes back with 200.
                Reply::ok(&n2).header("ETag", "\"v2\"")
            }
        })
        .await;
        let dl = downloader(&tmp.0);
        let dest = tmp.0.join("image.dmg");
        let size =
            dl.fetch_to_file(&format!("{}/img", srv.base), &dest, &[], &CancellationToken::new(), None).await.unwrap();
        assert_eq!(size, 4500);
        assert_eq!(std::fs::read(&dest).unwrap(), new);
        let reqs = srv.requests();
        assert_eq!(reqs.len(), 2);
        assert!(!reqs[0].headers.contains_key("if-range"));
        assert_eq!(reqs[1].headers.get("if-range").map(String::as_str), Some("\"v1\""));
        assert_eq!(reqs[1].headers.get("range").map(String::as_str), Some("bytes=1200-"));
        let part = part_path(&dest).unwrap();
        assert!(!part.exists());
        assert!(!validator_path(&part).exists());
    }

    #[test]
    fn resume_validator_prefers_strong_etag() {
        let mut h = HeaderMap::new();
        h.insert(LAST_MODIFIED, HeaderValue::from_static("Wed, 01 Oct 2026 10:00:00 GMT"));
        assert_eq!(resume_validator(&h).as_deref(), Some("Wed, 01 Oct 2026 10:00:00 GMT"));
        h.insert(ETAG, HeaderValue::from_static("W/\"weak\""));
        assert_eq!(resume_validator(&h).as_deref(), Some("Wed, 01 Oct 2026 10:00:00 GMT"));
        h.insert(ETAG, HeaderValue::from_static("\"strong\""));
        assert_eq!(resume_validator(&h).as_deref(), Some("\"strong\""));
        assert_eq!(resume_validator(&HeaderMap::new()), None);
    }

    #[tokio::test]
    async fn file_download_restarts_when_range_is_ignored() {
        let tmp = TempDir::new();
        let data = payload(3000);
        let body = data.clone();
        let srv = serve(move |_, _| Reply::ok(&body)).await;
        let dl = downloader(&tmp.0);
        let dest = tmp.0.join("f.bin");
        std::fs::write(part_path(&dest).unwrap(), &data[..700]).unwrap();
        let n =
            dl.fetch_to_file(&format!("{}/f", srv.base), &dest, &[], &CancellationToken::new(), None).await.unwrap();
        assert_eq!(n, 3000);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(srv.requests()[0].headers.get("range").map(String::as_str), Some("bytes=700-"));
    }

    #[tokio::test]
    async fn file_download_416_with_complete_part_finishes() {
        let tmp = TempDir::new();
        let data = payload(2048);
        let srv = serve(|_, _| Reply::status(416).header("Content-Range", "bytes */2048")).await;
        let dl = downloader(&tmp.0);
        let dest = tmp.0.join("done.bin");
        std::fs::write(part_path(&dest).unwrap(), &data).unwrap();
        let n =
            dl.fetch_to_file(&format!("{}/f", srv.base), &dest, &[], &CancellationToken::new(), None).await.unwrap();
        assert_eq!(n, 2048);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn file_download_416_with_stale_part_restarts() {
        let tmp = TempDir::new();
        let data = payload(1000);
        let body = data.clone();
        let srv = serve(move |req, _| {
            if req.headers.contains_key("range") {
                Reply::status(416).header("Content-Range", "bytes */1000")
            } else {
                Reply::ok(&body)
            }
        })
        .await;
        let dl = downloader(&tmp.0);
        let dest = tmp.0.join("stale.bin");
        std::fs::write(part_path(&dest).unwrap(), payload(4000)).unwrap();
        let n =
            dl.fetch_to_file(&format!("{}/f", srv.base), &dest, &[], &CancellationToken::new(), None).await.unwrap();
        assert_eq!(n, 1000);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(srv.hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn file_download_404_is_not_retried_and_keeps_part() {
        let tmp = TempDir::new();
        let srv = serve(|_, _| Reply::status(404)).await;
        let dl = downloader(&tmp.0);
        let dest = tmp.0.join("x.bin");
        std::fs::write(part_path(&dest).unwrap(), b"partial").unwrap();
        let err = dl
            .fetch_to_file(&format!("{}/f", srv.base), &dest, &[], &CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, "HTTP_NOT_FOUND");
        assert_eq!(srv.hits.load(Ordering::SeqCst), 1);
        assert!(!dest.exists());
        assert!(part_path(&dest).unwrap().exists());
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_stalled_transfer() {
        let tmp = TempDir::new();
        let srv = serve(|_, _| {
            let mut r = Reply::ok(&payload(10_000));
            r.stall_after = Some(100);
            r
        })
        .await;
        let dl = downloader(&tmp.0);
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c2.cancel();
        });
        let started = std::time::Instant::now();
        let dest = tmp.0.join("stalled.bin");
        let err = dl.fetch_to_file(&format!("{}/s", srv.base), &dest, &[], &cancel, None).await.unwrap_err();
        assert_eq!(err.code, "TASK_CANCELLED");
        assert!(started.elapsed() < Duration::from_secs(5));
        // The partial file stays for a later resume.
        assert_eq!(std::fs::metadata(part_path(&dest).unwrap()).unwrap().len(), 100);
    }

    #[tokio::test]
    async fn rejects_invalid_headers() {
        let tmp = TempDir::new();
        let dl = downloader(&tmp.0);
        let err = dl
            .fetch_to_file(
                "http://127.0.0.1:9/x",
                &tmp.0.join("y"),
                &[("Bad Header", "v")],
                &CancellationToken::new(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID_HEADER");
    }

    #[tokio::test]
    async fn github_latest_release_parses_assets() {
        let tmp = TempDir::new();
        let json = r#"{"tag_name":"1.7.2","html_url":"https://github.com/acidanthera/Lilu/releases/tag/1.7.2","body":"notes",
            "assets":[{"name":"Lilu-1.7.2-DEBUG.zip","browser_download_url":"https://x/d.zip","size":10,"digest":null},
                      {"name":"Lilu-1.7.2-RELEASE.zip","browser_download_url":"https://x/r.zip","size":20,
                       "digest":"sha256:53967d7dcfaab01023a33df2e969a89522f13d6654a6a56ac4711b62dabf3ab8"}]}"#;
        let srv = serve(move |_, _| Reply::ok(json.as_bytes()).header("Content-Type", "application/json")).await;
        let mut dl = downloader(&tmp.0);
        dl.github_api = srv.base.clone();
        let rel = dl.github_latest_release("acidanthera/Lilu").await.unwrap();
        assert_eq!(rel.tag, "1.7.2");
        assert_eq!(rel.assets.len(), 2);
        assert_eq!(
            rel.assets[1].sha256().as_deref(),
            Some("53967d7dcfaab01023a33df2e969a89522f13d6654a6a56ac4711b62dabf3ab8")
        );
        let req = &srv.requests()[0];
        assert_eq!(req.path, "/repos/acidanthera/Lilu/releases/latest");
        assert!(req.headers.get("user-agent").is_some_and(|ua| ua.starts_with("OpCore-OneClick/")));
        // Tokens are only ever sent to api.github.com.
        assert!(!req.headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn github_rate_limit_is_reported() {
        let tmp = TempDir::new();
        let srv = serve(|_, _| {
            Reply::status(403)
                .header("x-ratelimit-remaining", "0")
                .header("x-ratelimit-limit", "60")
                .header("x-ratelimit-reset", "1790000000")
        })
        .await;
        let mut dl = downloader(&tmp.0);
        dl.github_api = srv.base.clone();
        let err = dl.github_latest_release("acidanthera/Lilu").await.unwrap_err();
        assert_eq!(err.code, "RATE_LIMITED");
        assert!(err.recoverable);
        assert!(err.suggestion.is_some());
        assert!(err.message.contains("UTC"));
        assert_eq!(srv.hits.load(Ordering::SeqCst), 1);

        let srv = serve(|_, _| Reply::status(429).header("retry-after", "60")).await;
        dl.github_api = srv.base.clone();
        assert_eq!(dl.github_latest_release("a/b").await.unwrap_err().code, "RATE_LIMITED");

        let srv = serve(|_, _| Reply::status(403)).await;
        dl.github_api = srv.base.clone();
        assert_eq!(dl.github_latest_release("a/b").await.unwrap_err().code, "HTTP_ERROR");

        let srv = serve(|_, _| Reply::status(404)).await;
        dl.github_api = srv.base.clone();
        assert_eq!(dl.github_latest_release("a/b").await.unwrap_err().code, "GITHUB_RELEASE_NOT_FOUND");
    }
}
