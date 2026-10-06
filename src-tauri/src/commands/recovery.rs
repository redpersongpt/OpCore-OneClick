//! macOS recovery download, following OpenCorePkg `macrecovery.py` (1.0.8):
//! a session cookie from `osrecovery.apple.com`, a `RecoveryImage` query
//! for a board-id whose newest release is the wanted one, then the chunklist
//! and the DMG from Apple's CDN with an `AssetToken` cookie.
//!
//! The CDN only serves plain HTTP, so the chunklist's RSA signature (Apple
//! EFI ROM key) and its per-chunk SHA-256 are what make the image
//! trustworthy. Every byte is checked as it arrives; an interrupted download
//! resumes from the longest verified prefix, whatever left it behind.
//!
//! Layout: `recovery/<id>/com.apple.recovery.boot/<name>.{dmg,chunklist}`
//! with Apple's file names, plus `recovery/<id>/recovery.json` once the
//! image is complete and verified.

use std::collections::HashMap;
use std::future::Future;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use reqwest::header::{HeaderMap, CONTENT_RANGE, CONTENT_TYPE, COOKIE, RANGE, RETRY_AFTER, SET_COOKIE};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tracing::{info, warn};

use crate::contracts::{RecoveryCacheInfo, RecoveryProgress};
use crate::domain::macos_db::{self, RecoveryRequest};
use crate::domain::model::MacOsVersion;
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::safety::chunklist::{self, hex_encode, ChunkVerifier, Chunklist};
use crate::safety::payload::{self, modified_stamp, write_atomic, RecoveryMarker, RECOVERY_DIR_NAME, RECOVERY_MARKER};
use crate::tasks::cancellation::CancellationToken;
use crate::tasks::registry::TaskRegistry;

/// Apple's servers answer only to this user agent (prefix `InternetRecovery/`).
const USER_AGENT: &str = "InternetRecovery/1.0";
/// HTTPS works for the session and the query; plain HTTP is what
/// macrecovery uses and stays as the fallback.
const RECOVERY_SERVERS: [&str; 2] = ["https://osrecovery.apple.com", "http://osrecovery.apple.com"];
const IMAGE_ENDPOINT: &str = "/InstallationPayload/RecoveryImage";
/// Keys macrecovery requires in the query response.
const REQUIRED_KEYS: [&str; 7] = ["AP", "AU", "AH", "AT", "CU", "CH", "CT"];

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Per-read inactivity limit; a whole-body deadline would cut off slow links.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
const QUERY_ATTEMPTS: u32 = 4;
/// Consecutive failed attempts without new data before giving up.
const DOWNLOAD_RETRIES: u32 = 6;
/// Fresh session + query rounds (asset tokens expire after 45 minutes).
const TOKEN_REFRESHES: u32 = 8;
const MAX_CHUNKLIST_BYTES: u64 = 1 << 20;
/// OpenCore stores the DMG size in 32 bits and FAT32 caps files below 4 GiB.
const MAX_IMAGE_BYTES: u64 = u32::MAX as u64;
const WRITE_BATCH: usize = 4 << 20;
const SPACE_MARGIN: u64 = 64 << 20;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

static DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// Held while a download runs (or the cache is being cleared).
struct DownloadGuard;

impl DownloadGuard {
    fn acquire() -> Option<Self> {
        DOWNLOADING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| DownloadGuard)
    }
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        DOWNLOADING.store(false, Ordering::SeqCst);
    }
}

/// True while a recovery image is being downloaded.
pub(crate) fn download_in_progress() -> bool {
    DOWNLOADING.load(Ordering::SeqCst)
}

fn busy_downloading() -> AppError {
    AppError::new("RECOVERY_IN_PROGRESS", "A macOS recovery image is already being downloaded").recoverable()
}

// ─── Protocol pieces ────────────────────────────────────────────────────────

/// Upper-case hex id of `len` characters (`cid`, `k`, `fg`).
fn random_hex(len: usize) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut rng = rand::rng();
    (0..len).map(|_| char::from(DIGITS[rng.random_range(0..16)])).collect()
}

/// Body of the `RecoveryImage` query: `key=value` lines joined by `\n`, in
/// macrecovery's order, without a trailing newline.
pub fn image_request_body(request: &RecoveryRequest, cid: &str, k: &str, fg: &str) -> String {
    [("cid", cid), ("sn", request.mlb), ("bid", request.board_id), ("k", k), ("fg", fg), ("os", request.os_type)]
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `session=…` part of the session response's cookies.
fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .find(|part| part.len() > "session=".len() && part.starts_with("session="))
        .map(str::to_string)
}

/// Answer of the `RecoveryImage` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    /// `AP`: Apple's product id.
    pub product: String,
    /// `AU` / `AT`: DMG URL and its asset token.
    pub image_url: String,
    pub image_token: String,
    /// `CU` / `CT`: chunklist URL and its asset token.
    pub chunklist_url: String,
    pub chunklist_token: String,
}

/// Parse the `KEY: VALUE` lines of the query response; all seven keys
/// macrecovery requires must be present.
pub fn parse_image_info(text: &str) -> Result<AssetInfo, AppError> {
    let mut values: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(": ") {
            values.insert(key.trim(), value.trim());
        }
    }
    for key in REQUIRED_KEYS {
        if values.get(key).is_none_or(|v| v.is_empty()) {
            return Err(AppError::new("APPLE_EMPTY_RESPONSE", format!("Apple's recovery server did not send {key}"))
                .recoverable()
                .with_suggestion("Try again in a few minutes."));
        }
    }
    let get = |key: &str| values.get(key).map(|v| v.to_string()).unwrap_or_default();
    Ok(AssetInfo {
        product: get("AP"),
        image_url: get("AU"),
        image_token: get("AT"),
        chunklist_url: get("CU"),
        chunklist_token: get("CT"),
    })
}

/// Product id in an asset path (`/content/downloads/60/36/<product>/…`).
/// Not always equal to `AP`, but stable for an image.
pub fn image_product(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let segments: Vec<&str> = parsed.path_segments()?.collect();
    let at = segments.iter().position(|s| *s == "downloads")?;
    segments.get(at + 3).filter(|s| !s.is_empty()).map(|s| s.to_string())
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// File name of an asset URL (`BaseSystem.dmg`), checking the scheme, the
/// host (`hosts`: exact names or parent domains) and the extension.
pub fn asset_file_name(url: &str, hosts: &[String], extension: &str) -> Result<String, AppError> {
    let bad = |why: &str| AppError::new("APPLE_BAD_RESPONSE", format!("Unexpected recovery asset URL {url}: {why}"));
    let parsed = reqwest::Url::parse(url).map_err(|_| bad("not a URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(bad("unsupported scheme"));
    }
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    if !hosts.iter().any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}"))) {
        return Err(bad("unexpected host"));
    }
    let name = parsed.path_segments().and_then(|mut s| s.next_back()).unwrap_or_default().to_string();
    if !safe_file_name(&name) || name.len() <= extension.len() || !name.ends_with(extension) {
        return Err(bad("unexpected file name"));
    }
    Ok(name)
}

/// `bytes <start>-<end>/<total|*>` → (start, total).
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        text => Some(text.parse().ok()?),
    };
    Some((start, total))
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds.min(60)))
}

/// Largest chunk boundary that is not past `offset`.
fn chunk_floor(list: &Chunklist, offset: u64) -> u64 {
    let mut boundary = 0u64;
    for chunk in &list.chunks {
        let next = boundary + u64::from(chunk.size);
        if next > offset {
            break;
        }
        boundary = next;
    }
    boundary
}

// ─── Client ─────────────────────────────────────────────────────────────────

/// Where to ask and what to trust.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Recovery servers, preferred first (`https://osrecovery.apple.com`).
    pub servers: Vec<String>,
    /// Hosts (or parent domains) the DMG and chunklist may come from.
    pub asset_hosts: Vec<String>,
    /// RSA modulus (big-endian) the chunklist must be signed with; `None`
    /// means Apple's EFI ROM key.
    pub signing_key: Option<Vec<u8>>,
    /// Base delay between retries (doubles each time, capped at 30 s).
    pub retry_delay: Duration,
}

impl FetchOptions {
    pub fn apple() -> Self {
        Self {
            servers: RECOVERY_SERVERS.iter().map(|s| s.to_string()).collect(),
            asset_hosts: vec!["apple.com".to_string()],
            signing_key: None,
            retry_delay: Duration::from_secs(2),
        }
    }
}

/// Progress of a fetch: phase ("resolving" | "downloading" | "verifying"),
/// bytes of the DMG on disk and its size once known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchEvent {
    pub phase: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub type FetchEvents<'a> = &'a mut (dyn FnMut(FetchEvent) + Send);

/// How a download attempt ended.
enum Failure {
    /// Transient (network, server error, corrupt chunk): try again.
    Retry(AppError),
    /// The asset token or URL went stale: query Apple again.
    Refresh(AppError),
    Fatal(AppError),
}

impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Failure::Fatal(error)
    }
}

/// The image Apple chose for a request, with the local file names.
#[derive(Debug, Clone)]
struct ResolvedImage {
    info: AssetInfo,
    dmg_name: String,
    chunklist_name: String,
    /// URL path of the DMG; identifies the image across sessions.
    image_path: String,
}

enum QueryOutcome {
    Found(AssetInfo),
    /// The server refused this board-id; another may work.
    Rejected(AppError),
}

/// Await `future`, giving up when the task is cancelled.
async fn cancellable<F: Future>(cancel: &CancellationToken, future: F) -> Result<F::Output, AppError> {
    tokio::pin!(future);
    loop {
        cancel.check()?;
        tokio::select! {
            output = &mut future => return Ok(output),
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
}

async fn pause(cancel: &CancellationToken, duration: Duration) -> Result<(), AppError> {
    cancellable(cancel, tokio::time::sleep(duration)).await
}

/// reqwest error with its whole cause chain.
fn network_error(what: &str, error: reqwest::Error) -> AppError {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    let code = if error.is_timeout() {
        "NETWORK_TIMEOUT"
    } else if error.is_connect() {
        "NETWORK_CONNECT"
    } else {
        "NETWORK_ERROR"
    };
    AppError::new(code, format!("{what}: {message}"))
        .recoverable()
        .with_suggestion("Check the internet connection and try again; the download continues where it stopped.")
}

fn http_error(what: &str, status: StatusCode) -> AppError {
    AppError::new("APPLE_HTTP_ERROR", format!("{what}: HTTP {}", status.as_u16()))
        .recoverable()
        .with_context(serde_json::json!({ "status": status.as_u16() }))
}

fn io_error(what: &str, error: std::io::Error) -> AppError {
    if payload::is_disk_full(&error) {
        AppError::new("DISK_FULL", format!("{what}: the disk is full")).with_suggestion("Free up some disk space and try again.")
    } else {
        AppError::new("IO_ERROR", format!("{what}: {error}"))
    }
}

async fn blocking<T: Send + 'static>(job: impl FnOnce() -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
    tokio::task::spawn_blocking(job).await.map_err(|e| AppError::new("INTERNAL_ERROR", e.to_string()))?
}

pub struct RecoveryClient {
    http: reqwest::Client,
    options: FetchOptions,
    /// Index into `options.servers`; moves on after a connection failure.
    server: usize,
}

impl RecoveryClient {
    pub fn new(options: FetchOptions) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .map_err(|e| AppError::new("NETWORK_ERROR", format!("Cannot set up the HTTP client: {e}")))?;
        Ok(Self::with_http(options, http))
    }

    /// Use a prepared client (it must send the `InternetRecovery/` user agent).
    pub fn with_http(options: FetchOptions, http: reqwest::Client) -> Self {
        Self { http, options, server: 0 }
    }

    fn server(&self) -> &str {
        let index = self.server.min(self.options.servers.len().saturating_sub(1));
        self.options.servers.get(index).map(String::as_str).unwrap_or(RECOVERY_SERVERS[0])
    }

    /// After a connection-level failure, use the next server (HTTPS → HTTP).
    fn after_failure(&mut self, error: &AppError) {
        if error.code == "NETWORK_CONNECT" && self.server + 1 < self.options.servers.len() {
            self.server += 1;
            warn!(server = %self.server(), "Switching recovery server");
        }
    }

    fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32 << attempt.saturating_sub(1).min(4);
        (self.options.retry_delay * factor).min(Duration::from_secs(30))
    }

    async fn session(&self, cancel: &CancellationToken) -> Result<String, AppError> {
        let request = self.http.get(format!("{}/", self.server())).timeout(QUERY_TIMEOUT).send();
        let response = cancellable(cancel, request)
            .await?
            .map_err(|e| network_error("Contacting Apple's recovery server", e))?;
        if !response.status().is_success() {
            return Err(http_error("Apple's recovery server refused a session", response.status()));
        }
        session_cookie(response.headers()).ok_or_else(|| {
            AppError::new("APPLE_EMPTY_SESSION", "Apple's recovery server did not open a session").recoverable()
        })
    }

    /// One board-id: a fresh session (they last five minutes) and the query,
    /// retried on transient failures. Each attempt reports "resolving", which
    /// also keeps the task watchdog fed.
    async fn query_image(
        &mut self,
        request: &RecoveryRequest,
        events: FetchEvents<'_>,
        cancel: &CancellationToken,
    ) -> Result<QueryOutcome, AppError> {
        let mut last: Option<AppError> = None;
        for attempt in 0..QUERY_ATTEMPTS {
            if attempt > 0 {
                pause(cancel, self.delay(attempt)).await?;
            }
            events(FetchEvent { phase: "resolving", downloaded: 0, total: None });
            let session = match self.session(cancel).await {
                Ok(session) => session,
                Err(error) if error.code == "TASK_CANCELLED" => return Err(error),
                Err(error) => {
                    warn!(attempt, "Recovery session failed: {}", error.message);
                    self.after_failure(&error);
                    last = Some(error);
                    continue;
                }
            };
            let body = image_request_body(request, &random_hex(16), &random_hex(64), &random_hex(64));
            let send = self
                .http
                .post(format!("{}{IMAGE_ENDPOINT}", self.server()))
                .header(COOKIE, session)
                .header(CONTENT_TYPE, "text/plain")
                .body(body)
                .timeout(QUERY_TIMEOUT)
                .send();
            let response = match cancellable(cancel, send).await? {
                Ok(response) => response,
                Err(error) => {
                    let error = network_error("Asking Apple for the recovery image", error);
                    self.after_failure(&error);
                    last = Some(error);
                    continue;
                }
            };
            let status = response.status();
            if status.is_success() {
                let text = match cancellable(cancel, response.text()).await? {
                    Ok(text) => text,
                    Err(error) => {
                        last = Some(network_error("Reading Apple's answer", error));
                        continue;
                    }
                };
                match parse_image_info(&text) {
                    Ok(info) => return Ok(QueryOutcome::Found(info)),
                    Err(error) => {
                        last = Some(error);
                        continue;
                    }
                }
            }
            match status {
                // The session expired between the two requests.
                StatusCode::UNAUTHORIZED => last = Some(http_error("Apple's recovery session expired", status)),
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
                    return Ok(QueryOutcome::Rejected(http_error(
                        &format!("Apple's recovery server rejected board {}", request.board_id),
                        status,
                    )));
                }
                StatusCode::BAD_REQUEST => {
                    return Err(AppError::new("APPLE_BAD_REQUEST", "Apple's recovery server did not accept the request")
                        .with_context(serde_json::json!({ "board": request.board_id })));
                }
                s if s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() => {
                    if let Some(wait) = retry_after(response.headers()) {
                        pause(cancel, wait).await?;
                    }
                    last = Some(http_error("Apple's recovery server is busy", status));
                }
                _ => return Err(http_error("Apple's recovery server answered unexpectedly", status)),
            }
        }
        Err(last.unwrap_or_else(|| AppError::new("APPLE_UNREACHABLE", "Apple's recovery server could not be reached").recoverable()))
    }

    /// Ask for `version` with each known board-id until Apple serves an
    /// image of that release.
    async fn resolve(
        &mut self,
        version: MacOsVersion,
        events: FetchEvents<'_>,
        cancel: &CancellationToken,
    ) -> Result<ResolvedImage, AppError> {
        let mut problems = Vec::new();
        for request in macos_db::recovery_requests(version) {
            let info = match self.query_image(request, &mut *events, cancel).await? {
                QueryOutcome::Found(info) => info,
                QueryOutcome::Rejected(error) => {
                    warn!(board = request.board_id, "{}", error.message);
                    problems.push(error);
                    continue;
                }
            };
            let product = image_product(&info.image_url);
            match product.as_deref().and_then(macos_db::recovery_product_version) {
                Some(served) if served != version => {
                    warn!(board = request.board_id, product = ?product, "Board served {} instead of {}", served.id(), version.id());
                    problems.push(AppError::new(
                        "RECOVERY_WRONG_RELEASE",
                        format!("Apple served the {} recovery instead of {}", served.display_name(), version.display_name()),
                    ));
                    continue;
                }
                Some(_) => {}
                None => info!(product = ?product, ap = %info.product, "Recovery image not in the known list (newer build?)"),
            }
            let dmg_name = asset_file_name(&info.image_url, &self.options.asset_hosts, ".dmg")?;
            asset_file_name(&info.chunklist_url, &self.options.asset_hosts, ".chunklist")?;
            // OpenCore looks for `<dmg base name>.chunklist`.
            let chunklist_name = format!("{}.chunklist", &dmg_name[..dmg_name.len() - ".dmg".len()]);
            let image_path = reqwest::Url::parse(&info.image_url).map(|u| u.path().to_string()).unwrap_or_default();
            info!(board = request.board_id, os = request.os_type, ap = %info.product, path = %image_path, "Recovery image resolved");
            return Ok(ResolvedImage { info, dmg_name, chunklist_name, image_path });
        }
        Err(problems.pop().unwrap_or_else(|| {
            AppError::new("RECOVERY_UNAVAILABLE", format!("Apple did not offer a {} recovery image", version.display_name()))
        }))
    }

    async fn fetch_chunklist(&self, image: &ResolvedImage, cancel: &CancellationToken) -> Result<Vec<u8>, Failure> {
        let send = self
            .http
            .get(&image.info.chunklist_url)
            .header(COOKIE, format!("AssetToken={}", image.info.chunklist_token))
            .timeout(QUERY_TIMEOUT)
            .send();
        let mut response = cancellable(cancel, send)
            .await?
            .map_err(|e| Failure::Retry(network_error("Downloading the recovery chunklist", e)))?;
        let status = response.status();
        match status {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::GONE => {
                return Err(Failure::Refresh(http_error("The recovery chunklist link expired", status)));
            }
            s if s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() => {
                return Err(Failure::Retry(http_error("Apple's download server is busy", status)));
            }
            _ => return Err(Failure::Fatal(http_error("Downloading the recovery chunklist failed", status))),
        }
        let mut bytes = Vec::new();
        while let Some(piece) = cancellable(cancel, response.chunk())
            .await?
            .map_err(|e| Failure::Retry(network_error("Downloading the recovery chunklist", e)))?
        {
            bytes.extend_from_slice(&piece);
            if bytes.len() as u64 > MAX_CHUNKLIST_BYTES {
                return Err(Failure::Fatal(AppError::new("CHUNKLIST_INVALID", "The recovery chunklist is unexpectedly large")));
            }
        }
        Ok(bytes)
    }

    fn check_signature(&self, list: &Chunklist) -> Result<(), AppError> {
        match &self.options.signing_key {
            Some(modulus) => list.verify_signature_with(modulus),
            None => list.verify_signature(),
        }
    }

    /// Download the DMG from `offset` (a verified chunk boundary) to the end,
    /// checking every chunk as it arrives. On failure the file is cut back to
    /// its verified length, so the next attempt can resume from there.
    async fn download_image(
        &self,
        image: &ResolvedImage,
        part: &Path,
        list: &Chunklist,
        offset: u64,
        events: FetchEvents<'_>,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let total = list.total_size();
        let offset = chunk_floor(list, offset);
        if offset >= total {
            return Ok(());
        }
        events(FetchEvent { phase: "downloading", downloaded: offset, total: Some(total) });
        let mut request = self.http.get(&image.info.image_url).header(COOKIE, format!("AssetToken={}", image.info.image_token));
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let mut response = cancellable(cancel, request.send())
            .await?
            .map_err(|e| Failure::Retry(network_error("Downloading the recovery image", e)))?;
        let status = response.status();
        let start = match status {
            // A full answer: the server ignored the range.
            StatusCode::OK => 0,
            StatusCode::PARTIAL_CONTENT => {
                let range = response.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range);
                match range {
                    Some((start, size)) if start == offset && size.is_none_or(|s| s == total) => offset,
                    _ => {
                        return Err(Failure::Retry(AppError::new(
                            "RECOVERY_RANGE",
                            "Apple's download server sent an unexpected part of the image",
                        )))
                    }
                }
            }
            StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::NOT_FOUND
            | StatusCode::GONE
            | StatusCode::RANGE_NOT_SATISFIABLE => {
                return Err(Failure::Refresh(http_error("The recovery download link expired", status)));
            }
            s if s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() => {
                return Err(Failure::Retry(http_error("Apple's download server is busy", status)));
            }
            _ => return Err(Failure::Fatal(http_error("Downloading the recovery image failed", status))),
        };
        if let Some(length) = response.content_length() {
            if start + length != total {
                return Err(Failure::Retry(AppError::new(
                    "RECOVERY_SIZE_MISMATCH",
                    format!("The server offers {} bytes, the chunklist describes {total}", start + length),
                )));
            }
        }

        let what = "Writing the recovery image";
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(part)
            .await
            .map_err(|e| io_error(what, e))?;
        file.set_len(start).await.map_err(|e| io_error(what, e))?;
        file.seek(SeekFrom::Start(start)).await.map_err(|e| io_error(what, e))?;
        let mut verifier = ChunkVerifier::resume_at(list, start)?;
        let mut pending: Vec<u8> = Vec::with_capacity(WRITE_BATCH);
        let mut written = start;

        let streamed: Result<(), Failure> = async {
            loop {
                let piece = cancellable(cancel, response.chunk())
                    .await?
                    .map_err(|e| Failure::Retry(network_error("Downloading the recovery image", e)))?;
                let Some(piece) = piece else { break };
                // Keep the bytes before checking them, so a failure can still
                // save everything up to the last good chunk.
                pending.extend_from_slice(&piece);
                verifier.update(&piece).map_err(Failure::Retry)?;
                if pending.len() >= WRITE_BATCH {
                    let result = file.write_all(&pending).await;
                    let size = pending.len() as u64;
                    // A failed batch is never written again at a shifted position.
                    pending.clear();
                    result.map_err(|e| io_error(what, e))?;
                    written += size;
                }
                events(FetchEvent { phase: "downloading", downloaded: written + pending.len() as u64, total: Some(total) });
            }
            if verifier.is_complete() {
                Ok(())
            } else {
                Err(Failure::Retry(AppError::new("RECOVERY_INCOMPLETE", "The connection closed before the image was complete")))
            }
        }
        .await;

        let flushed = if pending.is_empty() {
            Ok(())
        } else {
            match file.write_all(&pending).await {
                Ok(()) => {
                    written += pending.len() as u64;
                    Ok(())
                }
                Err(e) => Err(io_error(what, e)),
            }
        };
        let failure = match (streamed, flushed) {
            (Ok(()), Ok(())) => {
                file.flush().await.map_err(|e| io_error(what, e))?;
                file.sync_all().await.map_err(|e| io_error(what, e))?;
                return Ok(());
            }
            (_, Err(error)) => Failure::Fatal(error),
            (Err(failure), Ok(())) => failure,
        };
        // Keep only what verified (and was written), so the next attempt
        // resumes from a chunk boundary.
        let keep = chunk_floor(list, verifier.verified_bytes().min(written));
        file.set_len(keep).await.map_err(|e| io_error(what, e))?;
        let _ = file.sync_all().await;
        Err(failure)
    }
}

// ─── Cache folder ───────────────────────────────────────────────────────────

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn part_name(dmg_name: &str) -> String {
    format!("{dmg_name}.part")
}

/// Make `boot_dir` hold only this image's chunklist and partial DMG: a
/// finished DMG of the same name becomes the partial (it is re-verified),
/// anything else is removed so OpenCore never sees two DMGs.
async fn prepare_folder(boot_dir: &Path, image: &ResolvedImage, chunklist: &[u8]) -> Result<PathBuf, AppError> {
    let dir = boot_dir.to_path_buf();
    let dmg_name = image.dmg_name.clone();
    let chunklist_name = image.chunklist_name.clone();
    let chunklist = chunklist.to_vec();
    blocking(move || {
        let what = "Preparing the recovery folder";
        std::fs::create_dir_all(&dir).map_err(|e| io_error(what, e))?;
        let part = dir.join(part_name(&dmg_name));
        let finished = dir.join(&dmg_name);
        if !part.exists() && finished.is_file() {
            std::fs::rename(&finished, &part).map_err(|e| io_error(what, e))?;
        }
        let keep = [part_name(&dmg_name), chunklist_name.clone()];
        for entry in std::fs::read_dir(&dir).map_err(|e| io_error(what, e))? {
            let entry = entry.map_err(|e| io_error(what, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            if is_file && !keep.contains(&name) {
                std::fs::remove_file(entry.path()).map_err(|e| io_error(what, e))?;
            }
        }
        write_atomic(&dir.join(&chunklist_name), &chunklist)?;
        Ok(part)
    })
    .await
}

/// Cut the partial DMG back to its longest verified prefix.
async fn verified_resume_point(part: &Path, list: &Chunklist) -> Result<u64, AppError> {
    let part = part.to_path_buf();
    let list = list.clone();
    blocking(move || {
        let what = "Checking the partial download";
        let good = chunklist::verified_prefix(&part, &list).map_err(|e| io_error(what, e))?;
        if part.exists() {
            let file = std::fs::OpenOptions::new().write(true).open(&part).map_err(|e| io_error(what, e))?;
            file.set_len(good).map_err(|e| io_error(what, e))?;
        }
        Ok(good)
    })
    .await
}

/// Free bytes on the volume holding `dir`, when the OS tells.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn available_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is plain old data; all-zero is a valid value.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated and `stat` is a valid out pointer.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(windows)]
fn available_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is NUL-terminated; the optional out pointers may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

#[cfg(not(any(unix, windows)))]
fn available_space(_dir: &Path) -> Option<u64> {
    None
}

fn ensure_space(dir: &Path, needed: u64) -> Result<(), AppError> {
    match available_space(dir) {
        Some(free) if free < needed.saturating_add(SPACE_MARGIN) => Err(AppError::new(
            "DISK_FULL",
            format!("The recovery image needs {} MB of free space; {} MB are free", needed / 1_000_000, free / 1_000_000),
        )
        .with_suggestion("Free up some disk space and try again.")),
        _ => Ok(()),
    }
}

/// Rename the verified partial file and record the verification.
async fn finish(
    version: MacOsVersion,
    version_dir: &Path,
    image: &ResolvedImage,
    chunklist: &[u8],
    part: &Path,
    total: u64,
) -> Result<RecoveryMarker, AppError> {
    let what = "Saving the recovery image";
    let size = file_len(part);
    if size != total {
        return Err(AppError::new("RECOVERY_IMAGE_CORRUPT", format!("The recovery image has {size} bytes instead of {total}")));
    }
    let final_path = version_dir.join(RECOVERY_DIR_NAME).join(&image.dmg_name);
    tokio::fs::rename(part, &final_path).await.map_err(|e| io_error(what, e))?;
    let meta = tokio::fs::metadata(&final_path).await.map_err(|e| io_error(what, e))?;
    let marker = RecoveryMarker {
        version: version.id().to_string(),
        product: image.info.product.clone(),
        image_path: image.image_path.clone(),
        dmg_name: image.dmg_name.clone(),
        chunklist_name: image.chunklist_name.clone(),
        dmg_size: meta.len(),
        dmg_modified: modified_stamp(&meta),
        chunklist_sha256: hex_encode(&Sha256::digest(chunklist)),
        verified_at: chrono::Utc::now().timestamp_millis(),
    };
    let dir = version_dir.to_path_buf();
    let saved = marker.clone();
    blocking(move || saved.save(&dir)).await?;
    Ok(marker)
}

/// Download (or resume) the recovery image of `version` into `version_dir`
/// and verify it. Returns the marker written after verification.
pub async fn fetch_recovery(
    client: &mut RecoveryClient,
    version: MacOsVersion,
    version_dir: &Path,
    events: FetchEvents<'_>,
    cancel: &CancellationToken,
) -> Result<RecoveryMarker, AppError> {
    let boot_dir = version_dir.join(RECOVERY_DIR_NAME);
    tokio::fs::create_dir_all(&boot_dir).await.map_err(|e| io_error("Creating the recovery folder", e))?;
    // The files are about to change: they are unverified until the end.
    match tokio::fs::remove_file(version_dir.join(RECOVERY_MARKER)).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io_error("Resetting the recovery folder", e)),
        _ => {}
    }

    let mut refreshes = 0u32;
    // Partial length when the last refresh started.
    let mut refresh_mark: Option<u64> = None;
    let mut retries = 0u32;
    // Chunklist digest and verified length from the previous round, so a
    // token refresh does not re-hash the whole partial file.
    let mut known: Option<(String, u64)> = None;
    'query: loop {
        cancel.check()?;
        let image = client.resolve(version, &mut *events, cancel).await?;
        let chunklist_bytes = match client.fetch_chunklist(&image, cancel).await {
            Ok(bytes) => bytes,
            Err(Failure::Fatal(error)) => return Err(error),
            Err(Failure::Refresh(error) | Failure::Retry(error)) => {
                retries += 1;
                if retries > DOWNLOAD_RETRIES {
                    return Err(error);
                }
                warn!("Chunklist download failed, asking again: {}", error.message);
                pause(cancel, client.delay(retries)).await?;
                continue 'query;
            }
        };
        let list = Chunklist::parse(&chunklist_bytes)?;
        client.check_signature(&list)?;
        let total = list.total_size();
        if total > MAX_IMAGE_BYTES {
            return Err(AppError::new("RECOVERY_TOO_LARGE", "The recovery image is too large for a FAT32 USB drive"));
        }
        let digest = hex_encode(&Sha256::digest(&chunklist_bytes));
        let part = prepare_folder(&boot_dir, &image, &chunklist_bytes).await?;
        let mut offset = match known.take() {
            Some((previous, length)) if previous == digest && file_len(&part) == length => length,
            _ => {
                if part.exists() {
                    events(FetchEvent { phase: "verifying", downloaded: 0, total: Some(total) });
                }
                verified_resume_point(&part, &list).await?
            }
        };
        if offset > 0 {
            info!(offset, total, "Resuming the recovery download");
        }
        ensure_space(&boot_dir, total - offset)?;

        loop {
            match client.download_image(&image, &part, &list, offset, &mut *events, cancel).await {
                Ok(()) => {
                    events(FetchEvent { phase: "verifying", downloaded: total, total: Some(total) });
                    let marker = finish(version, version_dir, &image, &chunklist_bytes, &part, total).await?;
                    info!(version = version.id(), product = %marker.product, size = marker.dmg_size, "Recovery image verified");
                    return Ok(marker);
                }
                Err(Failure::Fatal(error)) => return Err(error),
                Err(Failure::Retry(error)) => {
                    let now = chunk_floor(&list, file_len(&part));
                    if now > offset {
                        retries = 0;
                    }
                    offset = now;
                    retries += 1;
                    if retries > DOWNLOAD_RETRIES {
                        return Err(error);
                    }
                    warn!(retries, offset, "Recovery download interrupted: {}", error.message);
                    events(FetchEvent { phase: "downloading", downloaded: offset, total: Some(total) });
                    pause(cancel, client.delay(retries)).await?;
                }
                Err(Failure::Refresh(error)) => {
                    refreshes += 1;
                    if refreshes > TOKEN_REFRESHES {
                        return Err(error);
                    }
                    info!("Recovery download link expired, asking Apple again: {}", error.message);
                    let length = file_len(&part);
                    // An expired token is normal on slow links; a fresh link
                    // that fails again without progress is not, so back off.
                    if refresh_mark.is_some_and(|mark| length <= mark) {
                        pause(cancel, client.delay(refreshes)).await?;
                    }
                    refresh_mark = Some(length);
                    known = Some((digest.clone(), length));
                    continue 'query;
                }
            }
        }
    }
}

// ─── Cache info ─────────────────────────────────────────────────────────────

fn unavailable(version: MacOsVersion, partial: Option<u64>) -> RecoveryCacheInfo {
    RecoveryCacheInfo {
        available: false,
        version: Some(version),
        dmg_path: None,
        chunklist_path: None,
        size_bytes: partial,
        verified: false,
    }
}

/// What is cached for `version`: the verified image, or the size of a
/// partial download that the next attempt resumes.
fn cache_info_blocking(version_dir: &Path, version: MacOsVersion) -> RecoveryCacheInfo {
    match payload::inspect_recovery(version_dir) {
        Ok(found) if found.version == version.id() => RecoveryCacheInfo {
            available: true,
            version: Some(version),
            dmg_path: Some(found.dmg.source.to_string_lossy().into_owned()),
            chunklist_path: Some(found.chunklist.source.to_string_lossy().into_owned()),
            size_bytes: Some(found.dmg.size),
            verified: true,
        },
        _ => {
            let partial = std::fs::read_dir(version_dir.join(RECOVERY_DIR_NAME))
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".dmg.part"))
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .max();
            unavailable(version, partial)
        }
    }
}

async fn cache_info(version_dir: &Path, version: MacOsVersion) -> Result<RecoveryCacheInfo, AppError> {
    let dir = version_dir.to_path_buf();
    blocking(move || Ok(cache_info_blocking(&dir, version))).await
}

// ─── Progress reporting ─────────────────────────────────────────────────────

/// Sends throttled `recovery:progress` events and forwards progress to the
/// task registry.
struct ProgressSink {
    app: AppHandle,
    task_id: String,
    version: MacOsVersion,
    updates: tokio::sync::mpsc::UnboundedSender<(f64, String)>,
    phase: &'static str,
    last_emit: Option<Instant>,
    last: FetchEvent,
}

impl ProgressSink {
    fn handle(&mut self, mut event: FetchEvent) {
        // A token refresh asks Apple again mid-download: keep showing the
        // bytes already on disk instead of dropping back to zero.
        if event.phase == "resolving" && event.total.is_none() && self.last.total.is_some() {
            event.downloaded = self.last.downloaded;
            event.total = self.last.total;
        }
        let changed = event.phase != self.phase;
        let due = self.last_emit.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL);
        self.last = event;
        if !changed && !due {
            return;
        }
        self.phase = event.phase;
        self.last_emit = Some(Instant::now());
        let fraction = event.total.filter(|t| *t > 0).map(|t| (event.downloaded as f64 / t as f64).clamp(0.0, 1.0));
        self.emit(event.phase, event.downloaded, event.total, fraction, None);
        let mb = |bytes: u64| bytes / 1_000_000;
        let message = match event.phase {
            "resolving" => "Asking Apple for the recovery image".to_string(),
            "verifying" => "Checking the downloaded data".to_string(),
            _ => match event.total {
                Some(total) => format!("Downloading the recovery image: {} of {} MB", mb(event.downloaded), mb(total)),
                None => "Downloading the recovery image".to_string(),
            },
        };
        let _ = self.updates.send((fraction.unwrap_or(0.0), message));
    }

    fn emit(&self, phase: &str, downloaded: u64, total: Option<u64>, progress: Option<f64>, error: Option<String>) {
        let event = RecoveryProgress {
            task_id: self.task_id.clone(),
            version: self.version,
            phase: phase.to_string(),
            downloaded,
            total,
            progress,
            error,
        };
        if let Err(e) = self.app.emit("recovery:progress", &event) {
            warn!("recovery:progress could not be sent: {e}");
        }
    }
}

// ─── Commands ───────────────────────────────────────────────────────────────

/// Download BaseSystem.dmg + BaseSystem.chunklist for `version` into
/// `paths.recovery_dir(version)/com.apple.recovery.boot`, resumable, verified
/// against the chunklist. Emits `recovery:progress` (task kind "recovery-download").
#[tauri::command]
pub async fn download_recovery(
    version: MacOsVersion,
    task_registry: State<'_, Arc<TaskRegistry>>,
    paths: State<'_, AppPaths>,
    app: AppHandle,
) -> Result<RecoveryCacheInfo, AppError> {
    let _guard = DownloadGuard::acquire().ok_or_else(busy_downloading)?;
    if crate::commands::disk::flash_in_progress() {
        return Err(AppError::new("BUSY", "Wait until the USB drive is written before downloading").recoverable());
    }
    let version_dir = paths.recovery_dir(version.id());
    let cached = cache_info(&version_dir, version).await?;
    if cached.verified {
        return Ok(cached);
    }

    let registry: Arc<TaskRegistry> = Arc::clone(&task_registry);
    let (task_id, cancel) = registry.create("recovery-download").await;
    let (updates, mut receiver) = tokio::sync::mpsc::unbounded_channel::<(f64, String)>();
    let forwarder = {
        let registry = Arc::clone(&registry);
        let task_id = task_id.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((progress, message)) = receiver.recv().await {
                registry.update_progress(&task_id, progress, Some(message)).await;
            }
        })
    };
    let mut sink = ProgressSink {
        app,
        task_id: task_id.clone(),
        version,
        updates,
        phase: "",
        last_emit: None,
        last: FetchEvent { phase: "resolving", downloaded: 0, total: None },
    };
    info!(task = %task_id, version = version.id(), "Downloading macOS recovery");
    let result = match RecoveryClient::new(FetchOptions::apple()) {
        Ok(mut client) => fetch_recovery(&mut client, version, &version_dir, &mut |event| sink.handle(event), &cancel).await,
        Err(error) => Err(error),
    };
    let last = sink.last;
    match &result {
        Ok(marker) => sink.emit("complete", marker.dmg_size, Some(marker.dmg_size), Some(1.0), None),
        Err(error) => {
            warn!(task = %task_id, code = %error.code, "Recovery download failed: {}", error.message);
            sink.emit("failed", last.downloaded, last.total, None, Some(error.message.clone()));
        }
    }
    drop(sink);
    let _ = forwarder.await;
    match &result {
        Ok(_) => registry.complete(&task_id).await,
        // cancel() already recorded the cancellation; the partial file stays.
        Err(error) if error.code == "TASK_CANCELLED" => {}
        Err(error) => registry.fail(&task_id, &error.message).await,
    }
    result?;
    cache_info(&version_dir, version).await
}

#[tauri::command]
pub async fn get_cached_recovery_info(version: MacOsVersion, paths: State<'_, AppPaths>) -> Result<RecoveryCacheInfo, AppError> {
    cache_info(&paths.recovery_dir(version.id()), version).await
}

#[tauri::command]
pub async fn clear_recovery_cache(paths: State<'_, AppPaths>) -> Result<(), AppError> {
    let _guard = DownloadGuard::acquire().ok_or_else(busy_downloading)?;
    if crate::commands::disk::flash_in_progress() {
        return Err(AppError::new("BUSY", "The recovery image is being written to a USB drive").recoverable());
    }
    let dir = paths.recovery.clone();
    blocking(move || {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| io_error("Deleting the recovery cache", e))?;
        }
        std::fs::create_dir_all(&dir).map_err(|e| io_error("Deleting the recovery cache", e))?;
        Ok(())
    })
    .await?;
    info!("Recovery cache cleared");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::chunklist::tests::{build_chunklist, test_modulus};
    use crate::safety::payload::tests::TempDir;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, TcpStream};

    #[test]
    fn request_body_matches_macrecovery() {
        let request = macos_db::recovery_request(MacOsVersion::Tahoe);
        let body = image_request_body(&request, "3076CE439155BA14", &"4B".repeat(32), &"B2".repeat(32));
        assert_eq!(
            body,
            format!(
                "cid=3076CE439155BA14\nsn=00000000000000000\nbid=Mac-CFF7D910A743CAAF\nk={}\nfg={}\nos=latest",
                "4B".repeat(32),
                "B2".repeat(32)
            )
        );
        let id = random_hex(64);
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)));
        assert_eq!(random_hex(16).len(), 16);
    }

    const HIGH_SIERRA_RESPONSE: &str = "AP: 091-63921\n\
AU: http://oscdn.apple.com/content/downloads/60/36/091-63921/feit2vcc2ndwm48lmwgmbod6sr81qae7vy/RecoveryImage/BaseSystem.dmg\n\
AH: F30A5985EF7FCF6159BBDA3BBCD851FDE413905EA51A814F158C5C5DB71F2B88\n\
AT: expires=1791228241~access=/content/downloads/60/36/091-63921/feit2vcc2ndwm48lmwgmbod6sr81qae7vy/RecoveryImage/BaseSystem.dmg~md5=b9e0715000517f1b5381ea2cf28d963f\n\
CU: http://oscdn.apple.com/content/downloads/60/36/091-63921/feit2vcc2ndwm48lmwgmbod6sr81qae7vy/RecoveryImage/BaseSystem.chunklist\n\
CH: 9B94DA13D1359F84D57AECC0076DBD40B7844E19E626571BB2C355E0EB46F4D2\n\
CT: expires=1791228241~access=/content/downloads/60/36/091-63921/feit2vcc2ndwm48lmwgmbod6sr81qae7vy/RecoveryImage/BaseSystem.chunklist~md5=acf20b17231cdf2b13457ce51077f8a2\n";

    #[test]
    fn parses_the_recovery_response() {
        let info = parse_image_info(&HIGH_SIERRA_RESPONSE.replace('\n', "\r\n")).unwrap();
        assert_eq!(info.product, "091-63921");
        assert!(info.image_url.ends_with("/RecoveryImage/BaseSystem.dmg"));
        assert!(info.image_token.starts_with("expires=1791228241~access="));
        assert!(info.chunklist_url.ends_with("/BaseSystem.chunklist"));
        assert!(info.chunklist_token.ends_with("md5=acf20b17231cdf2b13457ce51077f8a2"));
        assert_eq!(image_product(&info.image_url).as_deref(), Some("091-63921"));
        assert_eq!(macos_db::recovery_product_version("091-63921"), Some(MacOsVersion::HighSierra));

        let apple = FetchOptions::apple().asset_hosts;
        assert_eq!(asset_file_name(&info.image_url, &apple, ".dmg").unwrap(), "BaseSystem.dmg");
        assert_eq!(asset_file_name(&info.chunklist_url, &apple, ".chunklist").unwrap(), "BaseSystem.chunklist");

        let missing = HIGH_SIERRA_RESPONSE.lines().filter(|l| !l.starts_with("CT:")).collect::<Vec<_>>().join("\n");
        let error = parse_image_info(&missing).unwrap_err();
        assert_eq!(error.code, "APPLE_EMPTY_RESPONSE");
        assert!(error.message.contains("CT"));
        assert!(parse_image_info("").is_err());
    }

    #[test]
    fn asset_urls_are_checked() {
        let apple = FetchOptions::apple().asset_hosts;
        let ok = "http://oscdn.apple.com/content/downloads/04/36/041-94410/x/RecoveryImage/RecoveryImage.dmg";
        assert_eq!(asset_file_name(ok, &apple, ".dmg").unwrap(), "RecoveryImage.dmg");
        for bad in [
            "http://oscdn.apple.com.evil.example/a/BaseSystem.dmg",
            "http://evilapple.com/a/BaseSystem.dmg",
            "ftp://oscdn.apple.com/a/BaseSystem.dmg",
            "http://oscdn.apple.com/a/BaseSystem.img",
            "http://oscdn.apple.com/a/.dmg",
            "http://oscdn.apple.com/a/Base%20System.dmg",
            "not a url",
        ] {
            assert!(asset_file_name(bad, &apple, ".dmg").is_err(), "{bad}");
        }
        assert_eq!(image_product("http://oscdn.apple.com/content/downloads/31/41/140-93589/tok/RecoveryImage/BaseSystem.dmg").as_deref(), Some("140-93589"));
        assert_eq!(image_product("http://oscdn.apple.com/other/BaseSystem.dmg"), None);
    }

    #[test]
    fn session_cookie_is_found_in_any_header() {
        let mut headers = HeaderMap::new();
        headers.append(SET_COOKIE, "other=1; Path=/".parse().unwrap());
        headers.append(SET_COOKIE, "session=1791226740~8C5E04; Domain=osrecovery.apple.com; Path=/; HttpOnly".parse().unwrap());
        assert_eq!(session_cookie(&headers).as_deref(), Some("session=1791226740~8C5E04"));
        let mut empty = HeaderMap::new();
        empty.append(SET_COOKIE, "session=; Path=/".parse().unwrap());
        assert_eq!(session_cookie(&empty), None);
    }

    #[test]
    fn content_range_and_chunk_floor() {
        assert_eq!(parse_content_range("bytes 0-15/960530321"), Some((0, Some(960_530_321))));
        assert_eq!(parse_content_range("bytes 2000-7499/*"), Some((2000, None)));
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes x-1/2"), None);
        let list = Chunklist::parse(&build_chunklist(&[1u8; 2500], 1000, 2)).unwrap();
        assert_eq!(chunk_floor(&list, 0), 0);
        assert_eq!(chunk_floor(&list, 999), 0);
        assert_eq!(chunk_floor(&list, 1000), 1000);
        assert_eq!(chunk_floor(&list, 2499), 2000);
        assert_eq!(chunk_floor(&list, 2500), 2500);
        assert_eq!(chunk_floor(&list, 9999), 2500);
    }

    // ── Local stand-in for Apple's servers ──────────────────────────────────

    #[derive(Default)]
    struct Script {
        /// Product id in the DMG path, per board-id (default 140-93589).
        products: HashMap<String, String>,
        /// Per DMG request (by index): send only this many body bytes.
        cut_after: HashMap<usize, usize>,
        /// DMG request indexes answered with 403.
        expired: Vec<usize>,
        /// DMG request index whose body gets one flipped byte at this offset
        /// of the image.
        corrupt: Option<(usize, usize)>,
    }

    struct Server {
        image: Vec<u8>,
        chunklist: Vec<u8>,
        script: Script,
        port: u16,
        posts: Mutex<Vec<String>>,
        ranges: Mutex<Vec<Option<u64>>>,
        dmg_requests: AtomicUsize,
    }

    async fn read_request(stream: &mut TcpStream) -> Option<(String, HashMap<String, String>, String)> {
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut buffer).await.ok()?;
            if read == 0 {
                return None;
            }
            data.extend_from_slice(&buffer[..read]);
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..header_end]).into_owned();
        let mut lines = head.split("\r\n");
        let request_line = lines.next()?.to_string();
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
            .collect();
        let length: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut body = data[header_end..].to_vec();
        while body.len() < length {
            let read = stream.read(&mut buffer).await.ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..read]);
        }
        Some((request_line, headers, String::from_utf8_lossy(&body).into_owned()))
    }

    async fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &[u8], send: usize) {
        let mut head = format!("HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(&body[..send.min(body.len())]).await;
        let _ = stream.flush().await;
        let _ = stream.shutdown().await;
    }

    async fn serve(state: Arc<Server>, mut stream: TcpStream) {
        let Some((request_line, headers, body)) = read_request(&mut stream).await else { return };
        let mut parts = request_line.split_whitespace();
        let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let cookie = headers.get("cookie").cloned().unwrap_or_default();
        assert!(headers.get("user-agent").is_some_and(|ua| ua.starts_with("InternetRecovery/")));
        match (method, path) {
            ("GET", "/") => {
                respond(&mut stream, "200 OK", &[("Set-Cookie", "session=1791226740~ABCDEF; Path=/; HttpOnly".into())], b"", 0).await
            }
            ("POST", "/InstallationPayload/RecoveryImage") => {
                assert_eq!(cookie, "session=1791226740~ABCDEF");
                assert_eq!(headers.get("content-type").map(String::as_str), Some("text/plain"));
                let board = body.lines().find_map(|l| l.strip_prefix("bid=")).unwrap_or_default().to_string();
                let round = {
                    let mut posts = state.posts.lock().unwrap();
                    posts.push(body.clone());
                    posts.len()
                };
                let product = state.script.products.get(&board).cloned().unwrap_or_else(|| "140-93589".into());
                let base = format!("http://127.0.0.1:{}/content/downloads/31/41/{product}/tok/RecoveryImage", state.port);
                let answer = format!(
                    "AP: {product}\nAU: {base}/BaseSystem.dmg\nAH: 00\nAT: dmg-{round}\nCU: {base}/BaseSystem.chunklist\nCH: 00\nCT: cnk-{round}\n"
                );
                respond(&mut stream, "200 OK", &[("Content-Type", "text/plain".into())], answer.as_bytes(), answer.len()).await
            }
            ("GET", p) if p.ends_with("/BaseSystem.chunklist") => {
                assert!(cookie.starts_with("AssetToken=cnk-"));
                let body = state.chunklist.clone();
                respond(&mut stream, "200 OK", &[], &body, body.len()).await
            }
            ("GET", p) if p.ends_with("/BaseSystem.dmg") => {
                assert!(cookie.starts_with("AssetToken=dmg-"));
                let index = state.dmg_requests.fetch_add(1, Ordering::SeqCst);
                let start: Option<u64> = headers
                    .get("range")
                    .and_then(|r| r.strip_prefix("bytes="))
                    .and_then(|r| r.trim_end_matches('-').parse().ok());
                state.ranges.lock().unwrap().push(start);
                if state.script.expired.contains(&index) {
                    return respond(&mut stream, "403 Forbidden", &[], b"", 0).await;
                }
                let from = start.unwrap_or(0) as usize;
                let mut body = state.image[from..].to_vec();
                if let Some((at, position)) = state.script.corrupt {
                    if at == index && position >= from {
                        body[position - from] ^= 0xFF;
                    }
                }
                let send = state.script.cut_after.get(&index).copied().unwrap_or(body.len());
                let total = state.image.len();
                if start.is_some() {
                    let range = format!("bytes {from}-{}/{total}", total - 1);
                    respond(&mut stream, "206 Partial Content", &[("Content-Range", range)], &body, send).await
                } else {
                    respond(&mut stream, "200 OK", &[], &body, send).await
                }
            }
            _ => respond(&mut stream, "404 Not Found", &[], b"", 0).await,
        }
    }

    fn sample_image(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 253) as u8).collect()
    }

    async fn start_server(image: Vec<u8>, script: Script) -> Arc<Server> {
        start_server_with_chunks(image, 1000, script).await
    }

    async fn start_server_with_chunks(image: Vec<u8>, chunk: usize, script: Script) -> Arc<Server> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let chunklist = build_chunklist(&image, chunk, 1);
        let state = Arc::new(Server {
            image,
            chunklist,
            script,
            port,
            posts: Mutex::new(Vec::new()),
            ranges: Mutex::new(Vec::new()),
            dmg_requests: AtomicUsize::new(0),
        });
        let accept_state = Arc::clone(&state);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(Arc::clone(&accept_state), stream));
            }
        });
        state
    }

    fn client_for(server: &Server, signing_key: Option<Vec<u8>>) -> RecoveryClient {
        let options = FetchOptions {
            servers: vec![format!("http://127.0.0.1:{}", server.port)],
            asset_hosts: vec!["127.0.0.1".into()],
            signing_key,
            retry_delay: Duration::from_millis(1),
        };
        let http = reqwest::Client::builder().user_agent(USER_AGENT).no_proxy().build().unwrap();
        RecoveryClient::with_http(options, http)
    }

    async fn fetch(client: &mut RecoveryClient, version: MacOsVersion, dir: &Path) -> (Result<RecoveryMarker, AppError>, Vec<FetchEvent>) {
        let mut events = Vec::new();
        let result = fetch_recovery(client, version, dir, &mut |e| events.push(e), &CancellationToken::new()).await;
        (result, events)
    }

    #[tokio::test]
    async fn downloads_resumes_refreshes_and_verifies() {
        let image = sample_image(7_500);
        let script = Script {
            // 1st request dies after 2500 bytes, 2nd finds the token expired,
            // 3rd delivers a corrupt 4th chunk, 4th completes.
            cut_after: HashMap::from([(0, 2_500)]),
            expired: vec![1],
            corrupt: Some((2, 3_500)),
            ..Default::default()
        };
        let server = start_server(image.clone(), script).await;
        let dir = TempDir::new();
        let version_dir = dir.0.join("26");
        let mut client = client_for(&server, Some(test_modulus()));
        let (result, events) = fetch(&mut client, MacOsVersion::Tahoe, &version_dir).await;
        let marker = result.unwrap();

        assert_eq!(*server.ranges.lock().unwrap(), vec![None, Some(2_000), Some(2_000), Some(3_000)]);
        assert_eq!(server.posts.lock().unwrap().len(), 2, "the expired token triggers one new query");
        assert!(server.posts.lock().unwrap()[0].contains("bid=Mac-CFF7D910A743CAAF\n"));
        assert!(server.posts.lock().unwrap()[0].ends_with("os=latest"));

        let boot = version_dir.join(RECOVERY_DIR_NAME);
        assert_eq!(std::fs::read(boot.join("BaseSystem.dmg")).unwrap(), image);
        assert!(!boot.join("BaseSystem.dmg.part").exists());
        assert_eq!(marker.version, "26");
        assert_eq!(marker.product, "140-93589");
        assert_eq!(marker.dmg_size, 7_500);
        assert!(marker.image_path.ends_with("/140-93589/tok/RecoveryImage/BaseSystem.dmg"));
        // The flash side accepts what was written.
        let payload = payload::inspect_recovery(&version_dir).unwrap();
        assert_eq!(payload.dmg.rel, "com.apple.recovery.boot/BaseSystem.dmg");
        assert!(cache_info_blocking(&version_dir, MacOsVersion::Tahoe).verified);

        assert_eq!(events.first().map(|e| e.phase), Some("resolving"));
        assert!(events.iter().any(|e| e.phase == "downloading" && e.downloaded == 7_500));
        assert_eq!(events.last().map(|e| e.phase), Some("verifying"));
    }

    #[tokio::test]
    async fn large_images_are_written_in_batches_and_resume_mid_batch() {
        const MIB: usize = 1 << 20;
        let image = sample_image(9 * MIB + 123);
        let script = Script { cut_after: HashMap::from([(0, 5 * MIB + 300_000)]), ..Default::default() };
        let server = start_server_with_chunks(image.clone(), MIB, script).await;
        let dir = TempDir::new();
        let version_dir = dir.0.join("26");
        let mut client = client_for(&server, Some(test_modulus()));
        let (result, _) = fetch(&mut client, MacOsVersion::Tahoe, &version_dir).await;
        result.unwrap();
        assert_eq!(*server.ranges.lock().unwrap(), vec![None, Some(5 * MIB as u64)]);
        assert_eq!(std::fs::read(version_dir.join(RECOVERY_DIR_NAME).join("BaseSystem.dmg")).unwrap(), image);
    }

    #[tokio::test]
    async fn resumes_from_a_verified_prefix_and_cleans_the_folder() {
        let image = sample_image(6_200);
        let server = start_server(image.clone(), Script::default()).await;
        let dir = TempDir::new();
        let version_dir = dir.0.join("26");
        let boot = version_dir.join(RECOVERY_DIR_NAME);
        std::fs::create_dir_all(&boot).unwrap();
        let mut partial = image[..4_200].to_vec();
        partial.extend_from_slice(&[0u8; 300]); // torn tail
        partial[4_100] ^= 1;
        std::fs::write(boot.join("BaseSystem.dmg.part"), &partial).unwrap();
        std::fs::write(boot.join("RecoveryImage.dmg"), b"stale").unwrap();
        std::fs::write(version_dir.join(RECOVERY_MARKER), b"{}").unwrap();

        let mut client = client_for(&server, Some(test_modulus()));
        let (result, events) = fetch(&mut client, MacOsVersion::Tahoe, &version_dir).await;
        result.unwrap();
        assert_eq!(*server.ranges.lock().unwrap(), vec![Some(4_000)]);
        assert!(events.iter().any(|e| e.phase == "verifying" && e.downloaded == 0));
        let mut names: Vec<String> =
            std::fs::read_dir(&boot).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["BaseSystem.chunklist", "BaseSystem.dmg"]);
        assert_eq!(std::fs::read(boot.join("BaseSystem.dmg")).unwrap(), image);
    }

    #[tokio::test]
    async fn a_complete_unmarked_image_is_reverified_without_downloading() {
        let image = sample_image(3_000);
        let server = start_server(image.clone(), Script::default()).await;
        let dir = TempDir::new();
        let version_dir = dir.0.join("26");
        let boot = version_dir.join(RECOVERY_DIR_NAME);
        std::fs::create_dir_all(&boot).unwrap();
        std::fs::write(boot.join("BaseSystem.dmg"), &image).unwrap();
        assert!(!cache_info_blocking(&version_dir, MacOsVersion::Tahoe).available);

        let mut client = client_for(&server, Some(test_modulus()));
        let (result, _) = fetch(&mut client, MacOsVersion::Tahoe, &version_dir).await;
        result.unwrap();
        assert!(server.ranges.lock().unwrap().is_empty());
        let info = cache_info_blocking(&version_dir, MacOsVersion::Tahoe);
        assert!(info.available && info.verified);
        assert_eq!(info.size_bytes, Some(3_000));
    }

    #[tokio::test]
    async fn a_board_serving_another_release_is_skipped() {
        let image = sample_image(2_000);
        let script = Script {
            products: HashMap::from([("Mac-CFF7D910A743CAAF".to_string(), "082-33203".to_string())]),
            ..Default::default()
        };
        let server = start_server(image, script).await;
        let dir = TempDir::new();
        let mut client = client_for(&server, Some(test_modulus()));
        let (result, _) = fetch(&mut client, MacOsVersion::Tahoe, &dir.0.join("26")).await;
        result.unwrap();
        {
            let posts = server.posts.lock().unwrap();
            assert_eq!(posts.len(), 2);
            assert!(posts[1].contains("bid=Mac-27AD2F918AE68F61\n"));
        }

        // Sequoia from every Tahoe board: nothing usable.
        let script = Script {
            products: macos_db::recovery_requests(MacOsVersion::Tahoe)
                .iter()
                .map(|r| (r.board_id.to_string(), "082-33203".to_string()))
                .collect(),
            ..Default::default()
        };
        let server = start_server(sample_image(2_000), script).await;
        let mut client = client_for(&server, Some(test_modulus()));
        let (result, _) = fetch(&mut client, MacOsVersion::Tahoe, &dir.0.join("26b")).await;
        assert_eq!(result.unwrap_err().code, "RECOVERY_WRONG_RELEASE");
    }

    #[tokio::test]
    async fn a_chunklist_not_signed_by_apple_is_refused() {
        let server = start_server(sample_image(2_000), Script::default()).await;
        let dir = TempDir::new();
        let version_dir = dir.0.join("26");
        // Apple's key, but the test server signs with the test key.
        let mut client = client_for(&server, None);
        let (result, _) = fetch(&mut client, MacOsVersion::Tahoe, &version_dir).await;
        assert_eq!(result.unwrap_err().code, "CHUNKLIST_SIGNATURE");
        assert!(server.ranges.lock().unwrap().is_empty());
        assert!(!cache_info_blocking(&version_dir, MacOsVersion::Tahoe).available);
    }

    #[tokio::test]
    async fn cancellation_stops_before_any_request() {
        let server = start_server(sample_image(2_000), Script::default()).await;
        let dir = TempDir::new();
        let mut client = client_for(&server, Some(test_modulus()));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = fetch_recovery(&mut client, MacOsVersion::Tahoe, &dir.0.join("26"), &mut |_| {}, &cancel).await;
        assert_eq!(result.unwrap_err().code, "TASK_CANCELLED");
        assert!(server.posts.lock().unwrap().is_empty());
    }

    /// Live check against Apple (network, about 10 MB):
    /// `cargo test -- --ignored apple_serves_every_release`.
    #[tokio::test]
    #[ignore]
    async fn apple_serves_every_release() {
        let mut client = RecoveryClient::new(FetchOptions::apple()).unwrap();
        let cancel = CancellationToken::new();
        for version in MacOsVersion::ALL {
            let image = client.resolve(version, &mut |_| {}, &cancel).await.unwrap();
            let bytes = match client.fetch_chunklist(&image, &cancel).await {
                Ok(bytes) => bytes,
                Err(_) => panic!("chunklist for {version:?}"),
            };
            let list = Chunklist::parse(&bytes).unwrap();
            list.verify_signature().unwrap();
            println!("{} {} {} bytes", version.id(), image.image_path, list.total_size());
            if version == MacOsVersion::Tahoe {
                // The first chunk through the real CDN path: token cookie + Range.
                let first = u64::from(list.chunks[0].size);
                let response = client
                    .http
                    .get(&image.info.image_url)
                    .header(COOKIE, format!("AssetToken={}", image.info.image_token))
                    .header(RANGE, format!("bytes=0-{}", first - 1))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
                let body = response.bytes().await.unwrap();
                let digest: [u8; 32] = Sha256::digest(&body).into();
                assert_eq!(digest, list.chunks[0].sha256);
            }
        }
    }
}
