//! What `validate_efi` checks: a build directory, an EFI folder or a single
//! config.plist, and the ocvalidate that matches the OpenCore release which
//! produced it (taken from the local cache only, never downloaded).

use std::path::{Path, PathBuf};

use plist::Value;

use crate::contracts::{ArtifactStatus, ValidationIssue, ValidationResult};
use crate::domain::kext_catalog;
use crate::domain::model::NoteLevel;
use crate::error::AppError;
use crate::services::artifacts::{self, OpenCorePackage};
use crate::services::http::Downloader;
use crate::services::ocvalidate::{parse_output, run_ocvalidate};
use crate::tasks::cancellation::CancellationToken;

use super::manifest;
use super::staging::find_ci;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationTarget {
    /// The EFI folder (with `OC/` and `BOOT/`) when the path belongs to one.
    pub efi_dir: Option<PathBuf>,
    /// The config.plist that will be checked.
    pub config: PathBuf,
    /// The folder that contains `EFI/` (a build directory), if known.
    pub build_dir: Option<PathBuf>,
}

/// Accept a build directory (containing `EFI`), an `EFI` folder, an `OC`
/// folder or a config.plist file.
pub fn resolve_target(path: &Path) -> Result<ValidationTarget, AppError> {
    let meta = std::fs::metadata(path).map_err(|_| {
        AppError::new("PATH_NOT_FOUND", format!("{} does not exist", path.display()))
            .recoverable()
            .with_suggestion("Pick the folder that contains EFI, the EFI folder itself, or a config.plist.")
    })?;
    let is_named = |p: &Path, name: &str| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name));

    if meta.is_file() {
        let is_plist = path.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("plist"));
        if !is_plist {
            return Err(AppError::new("NOT_A_CONFIG", format!("{} is not a .plist file", path.display())).recoverable());
        }
        let oc = path.parent().filter(|p| is_named(p, "OC"));
        let efi = oc.and_then(Path::parent).filter(|_| is_named(path, "config.plist")).map(Path::to_path_buf);
        let build_dir = efi.as_deref().filter(|e| is_named(e, "EFI")).and_then(Path::parent).map(Path::to_path_buf);
        return Ok(ValidationTarget { efi_dir: efi, config: path.to_path_buf(), build_dir });
    }

    let has_oc = |p: &Path| find_ci(p, "OC").is_some_and(|oc| oc.is_dir());
    let (efi, build_dir) = if let Some(efi) = find_ci(path, "EFI").filter(|e| e.is_dir() && has_oc(e)) {
        (efi, Some(path.to_path_buf()))
    } else if has_oc(path) {
        let build = is_named(path, "EFI").then(|| path.parent().map(Path::to_path_buf)).flatten();
        (path.to_path_buf(), build)
    } else if is_named(path, "OC") && find_ci(path, "config.plist").is_some() {
        let efi = path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf());
        let build = is_named(&efi, "EFI").then(|| efi.parent().map(Path::to_path_buf)).flatten();
        (efi, build)
    } else {
        return Err(AppError::new("EFI_NOT_FOUND", format!("No EFI folder in {}", path.display()))
            .recoverable()
            .with_suggestion("Pick the folder that contains EFI, the EFI folder itself, or a config.plist."));
    };
    let oc = find_ci(&efi, "OC").unwrap_or_else(|| efi.join("OC"));
    let config = find_ci(&oc, "config.plist").unwrap_or_else(|| oc.join("config.plist"));
    Ok(ValidationTarget { efi_dir: Some(efi), config, build_dir })
}

/// ocvalidate of the OpenCore release that built `build_dir` (from its
/// manifest; the pinned release otherwise), if that package is in the local
/// cache. Never touches the network.
pub async fn locate_ocvalidate(build_dir: Option<&Path>, dl: &Downloader, work_dir: &Path) -> Option<PathBuf> {
    let pin = kext_catalog::opencore_release();
    let manifest = build_dir.and_then(manifest::read);
    let (version, debug) =
        manifest.as_ref().map_or((pin.version.to_string(), false), |m| (m.opencore_version.clone(), m.opencore_debug));

    // The manifest names the package the build used. Only a package inside
    // our own scratch space is trusted: build.json could come from anywhere,
    // and the binary it points at is executed.
    let trusted = |root: &Path| match (root.canonicalize(), work_dir.canonicalize()) {
        (Ok(root), Ok(work)) => root.starts_with(work),
        _ => false,
    };
    if let Some(root) = manifest.as_ref().and_then(|m| m.opencore_root.clone()).filter(|r| trusted(r)) {
        let package = OpenCorePackage { version: version.clone(), debug, root, status: ArtifactStatus::Cached };
        if let Some(bin) = package.ocvalidate() {
            return Some(bin);
        }
    }
    if version != pin.version {
        tracing::info!(version, "no cached ocvalidate for this OpenCore release");
        return None;
    }
    // ocvalidate is identical in the RELEASE and DEBUG packages.
    for flavour in [debug, !debug] {
        let pin = if flavour { kext_catalog::opencore_debug() } else { kext_catalog::opencore_release() };
        if !pin.sha256.is_some_and(|sha| dl.is_cached(sha)) {
            continue;
        }
        match artifacts::fetch_opencore(dl, work_dir, flavour, false, &CancellationToken::new()).await {
            Ok(package) => {
                if let Some(bin) = package.ocvalidate() {
                    return Some(bin);
                }
            }
            Err(e) => tracing::warn!(error = %e, "cached OpenCore package could not be extracted"),
        }
    }
    None
}

/// Check a config.plist that is not inside an EFI folder: plist syntax and
/// ocvalidate only (referenced files cannot be checked).
pub async fn validate_config_only(config: &Path, ocvalidate: Option<&Path>) -> ValidationResult {
    let mut issues = Vec::new();
    if let Err(e) = Value::from_file(config)
        .map_err(|e| e.to_string())
        .and_then(|v| v.as_dictionary().map(|_| ()).ok_or_else(|| "the root is not a dictionary".to_string()))
    {
        issues.push(issue(NoteLevel::Blocking, "config", format!("config.plist cannot be parsed: {e}")));
        return ValidationResult { valid: false, ocvalidate_ran: false, ocvalidate_output: None, issues };
    }
    let mut ran = false;
    let mut output = None;
    match ocvalidate {
        None => issues.push(issue(
            NoteLevel::Info,
            "ocvalidate",
            "ocvalidate is not available for this OpenCore release; OpenCore's schema check was skipped".into(),
        )),
        Some(bin) => match run_ocvalidate(bin, config).await {
            Ok(run) => {
                ran = true;
                let parsed = parse_output(&run.output);
                issues.extend(parsed.issues.iter().map(|l| issue(NoteLevel::Blocking, "ocvalidate", l.clone())));
                if parsed.issues.is_empty() && run.status.is_some_and(|s| s != 0) {
                    issues.push(issue(
                        NoteLevel::Blocking,
                        "ocvalidate",
                        format!("ocvalidate exited with status {} without listing a problem", run.status.unwrap_or(-1)),
                    ));
                }
                output = Some(run.output);
            }
            Err(e) => {
                issues.push(issue(NoteLevel::Warning, "ocvalidate", format!("ocvalidate could not run: {}", e.message)))
            }
        },
    }
    issues.push(issue(
        NoteLevel::Info,
        "layout",
        "Only config.plist was checked: it is not inside an EFI/OC folder, so the files it references were not.".into(),
    ));
    let valid = !issues.iter().any(|i| i.level == NoteLevel::Blocking);
    ValidationResult { valid, ocvalidate_ran: ran, ocvalidate_output: output, issues }
}

fn issue(level: NoteLevel, source: &str, message: String) -> ValidationIssue {
    ValidationIssue { level, source: source.into(), message, path: None }
}

#[cfg(test)]
mod tests {
    use super::super::staging::test_dir::TempDir;
    use super::*;

    fn efi_layout(root: &Path) -> PathBuf {
        let oc = root.join("EFI/OC");
        std::fs::create_dir_all(&oc).unwrap();
        std::fs::create_dir_all(root.join("EFI/BOOT")).unwrap();
        std::fs::write(oc.join("config.plist"), b"<plist version=\"1.0\"><dict/></plist>").unwrap();
        oc.join("config.plist")
    }

    #[test]
    fn resolves_builds_efi_folders_and_configs() {
        let tmp = TempDir::new("validate");
        let build = tmp.path().join("20261006-120000-00000000");
        let config = efi_layout(&build);
        let expected = ValidationTarget {
            efi_dir: Some(build.join("EFI")),
            config: config.clone(),
            build_dir: Some(build.clone()),
        };
        assert_eq!(resolve_target(&build).unwrap(), expected);
        assert_eq!(resolve_target(&build.join("EFI")).unwrap(), expected);
        assert_eq!(resolve_target(&build.join("EFI/OC")).unwrap(), expected);
        assert_eq!(resolve_target(&config).unwrap(), expected);

        let loose = tmp.path().join("my-config.plist");
        std::fs::write(&loose, b"<plist version=\"1.0\"><dict/></plist>").unwrap();
        let target = resolve_target(&loose).unwrap();
        assert_eq!(target.efi_dir, None);
        assert_eq!(target.build_dir, None);

        std::fs::write(tmp.path().join("notes.txt"), b"x").unwrap();
        assert_eq!(resolve_target(&tmp.path().join("notes.txt")).unwrap_err().code, "NOT_A_CONFIG");
        assert_eq!(resolve_target(&tmp.path().join("nope")).unwrap_err().code, "PATH_NOT_FOUND");
        std::fs::create_dir_all(tmp.path().join("empty")).unwrap();
        assert_eq!(resolve_target(&tmp.path().join("empty")).unwrap_err().code, "EFI_NOT_FOUND");
    }

    #[tokio::test]
    async fn loose_config_without_ocvalidate() {
        let tmp = TempDir::new("validate");
        let good = tmp.path().join("config.plist");
        std::fs::write(&good, b"<plist version=\"1.0\"><dict/></plist>").unwrap();
        let result = validate_config_only(&good, None).await;
        assert!(result.valid);
        assert!(!result.ocvalidate_ran);
        assert_eq!(result.issues.len(), 2);

        let bad = tmp.path().join("bad.plist");
        std::fs::write(&bad, b"<html>").unwrap();
        let result = validate_config_only(&bad, None).await;
        assert!(!result.valid);
        assert_eq!(result.issues[0].source, "config");
    }

    #[tokio::test]
    async fn no_cached_package_means_no_ocvalidate() {
        let tmp = TempDir::new("validate");
        let dl = Downloader::new(tmp.path().join("cache")).unwrap();
        assert!(locate_ocvalidate(None, &dl, &tmp.path().join("work")).await.is_none());
    }

    #[tokio::test]
    async fn manifests_cannot_point_at_foreign_binaries() {
        use crate::build::manifest::{self, BuildManifest};
        use crate::contracts::{BuildResult, ValidationResult};
        use crate::domain::model::{MacOsVersion, PlatformIdentity};

        let tmp = TempDir::new("validate");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        // A fake package outside the scratch space with a host ocvalidate.
        let foreign = tmp.path().join("elsewhere");
        let tool = if cfg!(windows) {
            "ocvalidate.exe"
        } else if cfg!(target_os = "linux") {
            "ocvalidate.linux"
        } else {
            "ocvalidate"
        };
        std::fs::create_dir_all(foreign.join("Utilities/ocvalidate")).unwrap();
        std::fs::write(foreign.join("Utilities/ocvalidate").join(tool), b"#!/bin/sh\n").unwrap();
        let build = tmp.path().join("build");
        efi_layout(&build);
        let result = BuildResult {
            build_id: "x".into(),
            efi_path: String::new(),
            config_plist_path: String::new(),
            target: MacOsVersion::Sequoia,
            opencore_version: "1.0.8".into(),
            identity: PlatformIdentity {
                model: "iMac19,1".into(),
                serial: String::new(),
                mlb: String::new(),
                system_uuid: String::new(),
                rom: String::new(),
            },
            plan: crate::domain::planner::empty_plan(MacOsVersion::Sequoia),
            kexts: vec![],
            ssdts: vec![],
            validation: ValidationResult {
                valid: true,
                ocvalidate_ran: false,
                ocvalidate_output: None,
                issues: vec![],
            },
            warnings: vec![],
        };
        manifest::write(&build, &BuildManifest::new(result, false, Some(foreign.clone()))).unwrap();
        let dl = Downloader::new(tmp.path().join("cache")).unwrap();
        assert!(locate_ocvalidate(Some(&build), &dl, &work).await.is_none());

        // The same package inside the scratch space is used.
        let inside = work.join("opencore/1.0.8-RELEASE-test");
        std::fs::create_dir_all(inside.join("Utilities/ocvalidate")).unwrap();
        std::fs::write(inside.join("Utilities/ocvalidate").join(tool), b"#!/bin/sh\n").unwrap();
        let mut m = manifest::read(&build).unwrap();
        m.opencore_root = Some(inside.clone());
        manifest::write(&build, &m).unwrap();
        let found = locate_ocvalidate(Some(&build), &dl, &work).await;
        if !(cfg!(target_os = "linux") && !cfg!(target_arch = "x86_64")) {
            assert_eq!(found, Some(inside.join("Utilities/ocvalidate").join(tool)));
        }
    }
}
