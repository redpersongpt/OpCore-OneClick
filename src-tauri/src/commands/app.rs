//! App metadata and update check.

use std::cmp::Ordering;

use tauri::State;

use crate::contracts::{AppVersionInfo, UpdateInfo};
use crate::domain::kext_catalog;
use crate::error::AppError;
use crate::paths::AppPaths;
use crate::services::http::Downloader;
use crate::APP_VERSION;

/// GitHub repository whose releases are the app's updates.
const RELEASES_REPO: &str = "redpersongpt/OpCore-OneClick";
/// Release notes longer than this are cut (the UI links to the full page).
const MAX_NOTES_CHARS: usize = 4000;

#[tauri::command]
pub async fn get_app_info() -> Result<AppVersionInfo, AppError> {
    Ok(app_info())
}

pub fn app_info() -> AppVersionInfo {
    AppVersionInfo {
        version: APP_VERSION.to_string(),
        opencore_version: kext_catalog::opencore_release().version.to_string(),
        host_os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

/// Compare the running version with the latest GitHub release (semver).
#[tauri::command]
pub async fn check_for_updates(paths: State<'_, AppPaths>) -> Result<UpdateInfo, AppError> {
    let mut downloader = Downloader::new(paths.cache.clone())?;
    // An update check should answer quickly, also when offline.
    downloader.retry.max_retries = 1;
    let release = match downloader.github_latest_release(RELEASES_REPO).await {
        Ok(release) => release,
        Err(e) if e.code == "GITHUB_RELEASE_NOT_FOUND" => {
            return Ok(UpdateInfo {
                current: APP_VERSION.into(),
                latest: None,
                update_available: false,
                url: None,
                notes: None,
            })
        }
        Err(e) => return Err(offline_friendly(e)),
    };
    let latest = release.tag.trim().trim_start_matches(['v', 'V']).to_string();
    let update_available = is_newer(&latest, APP_VERSION);
    tracing::info!(current = APP_VERSION, latest = %latest, update_available, "update check");
    Ok(UpdateInfo {
        current: APP_VERSION.into(),
        latest: Some(latest),
        update_available,
        url: Some(release.html_url),
        notes: release.body.map(|b| truncate(b.trim(), MAX_NOTES_CHARS)).filter(|b| !b.is_empty()),
    })
}

fn offline_friendly(err: AppError) -> AppError {
    match err.code.as_str() {
        "RATE_LIMITED" => err,
        "NETWORK_ERROR" | "HTTP_ERROR" | "HTTP_NOT_FOUND" | "GITHUB_BAD_RESPONSE" => {
            tracing::info!(error = %err, "update check failed");
            AppError::new("UPDATE_CHECK_FAILED", "Could not reach GitHub to check for updates")
                .recoverable()
                .with_suggestion("Check the internet connection and try again later. Building with the pinned versions works offline once they are cached.")
        }
        _ => err,
    }
}

/// `latest` is a higher semantic version than `current`. Pre-releases sort
/// below their release; unparsable versions never count as newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => compare(&l, &c) == Ordering::Greater,
        _ => false,
    }
}

type Semver = ([u64; 3], Option<String>);

fn parse_semver(s: &str) -> Option<Semver> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split('+').next()?;
    let (core, pre) = match s.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_string())),
        Some(_) => return None,
        None => (s, None),
    };
    let mut parts = core.split('.');
    let mut nums = [0u64; 3];
    for (i, slot) in nums.iter_mut().enumerate() {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None if i > 0 => break,
            None => return None,
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some((nums, pre))
}

fn compare(a: &Semver, b: &Semver) -> Ordering {
    a.0.cmp(&b.0).then_with(|| match (&a.1, &b.1) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => compare_pre(x, y),
    })
}

/// Dot-separated identifiers; numeric ones compare numerically and sort
/// before alphanumeric ones (semver §11).
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut ia = a.split('.');
    let mut ib = b.split('.');
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison() {
        assert!(is_newer("5.0.1", "5.0.0"));
        assert!(is_newer("v5.1.0", "5.0.9"));
        assert!(is_newer("6", "5.9.9"));
        assert!(is_newer("5.10.0", "5.9.0"));
        assert!(!is_newer("5.0.0", "5.0.0"));
        assert!(!is_newer("v4.9.9", "5.0.0"));
        assert!(is_newer("5.0.0", "5.0.0-beta.2"));
        assert!(!is_newer("5.0.0-beta.2", "5.0.0"));
        assert!(is_newer("5.0.0-beta.10", "5.0.0-beta.2"));
        assert!(is_newer("5.0.0-rc.1", "5.0.0-beta.9"));
        assert!(is_newer("5.0.1+build.7", "5.0.0"));
        assert!(!is_newer("nightly", "5.0.0"));
        assert!(!is_newer("5.0.0.1", "5.0.0"));
        assert!(!is_newer("5.0.0-", "4.0.0"));
    }

    #[test]
    fn app_info_reports_the_pins() {
        let info = app_info();
        assert_eq!(info.version, APP_VERSION);
        assert_eq!(info.opencore_version, "1.0.8");
        assert!(!info.host_os.is_empty() && !info.arch.is_empty());
    }

    #[test]
    fn notes_are_cut_on_a_character_boundary() {
        assert_eq!(truncate("héllo", 2), "hé…");
        assert_eq!(truncate("abc", 5), "abc");
    }

    #[test]
    fn network_errors_become_friendly() {
        let e = offline_friendly(AppError::new("NETWORK_ERROR", "https://api.github.com: could not connect"));
        assert_eq!(e.code, "UPDATE_CHECK_FAILED");
        assert!(e.recoverable);
        assert_eq!(offline_friendly(AppError::new("RATE_LIMITED", "x")).code, "RATE_LIMITED");
    }
}
