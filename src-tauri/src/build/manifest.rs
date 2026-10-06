//! `build.json` next to the `EFI` folder of every build: the build result and
//! which OpenCore package produced it, so the EFI can be validated later with
//! the matching ocvalidate. It sits outside `EFI/` and is never flashed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contracts::BuildResult;
use crate::error::AppError;

use super::staging::write_atomic;

pub const MANIFEST_FILE: &str = "build.json";
const FORMAT: &str = "opcore-oneclick-build";
/// Manifests larger than this are not read.
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildManifest {
    pub format: String,
    pub version: u32,
    pub created_at: String,
    pub opencore_version: String,
    pub opencore_debug: bool,
    /// Extracted OpenCore package used for the build (scratch space; may be
    /// gone after a restart).
    pub opencore_root: Option<PathBuf>,
    pub result: BuildResult,
}

impl BuildManifest {
    pub fn new(result: BuildResult, opencore_debug: bool, opencore_root: Option<PathBuf>) -> Self {
        Self {
            format: FORMAT.into(),
            version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
            opencore_version: result.opencore_version.clone(),
            opencore_debug,
            opencore_root,
            result,
        }
    }
}

pub fn write(build_dir: &Path, manifest: &BuildManifest) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    write_atomic(&build_dir.join(MANIFEST_FILE), &bytes)
}

/// The manifest of `build_dir`, if it has a readable one.
pub fn read(build_dir: &Path) -> Option<BuildManifest> {
    let path = build_dir.join(MANIFEST_FILE);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<BuildManifest>(&bytes) {
        Ok(m) if m.format == FORMAT => Some(m),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "build manifest not readable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::staging::test_dir::TempDir;
    use super::*;
    use crate::contracts::ValidationResult;
    use crate::domain::model::{MacOsVersion, PlatformIdentity};
    use crate::domain::planner::empty_plan;

    fn sample_result() -> BuildResult {
        BuildResult {
            build_id: "20261006-120000-00000000".into(),
            efi_path: "/tmp/b".into(),
            config_plist_path: "/tmp/b/EFI/OC/config.plist".into(),
            target: MacOsVersion::Sequoia,
            opencore_version: "1.0.8".into(),
            identity: PlatformIdentity {
                model: "iMac19,1".into(),
                serial: "C02XXXXXXXXX".into(),
                mlb: "C02000000000000AA".into(),
                system_uuid: "00000000-0000-0000-0000-000000000000".into(),
                rom: "112233445566".into(),
            },
            plan: empty_plan(MacOsVersion::Sequoia),
            kexts: vec![],
            ssdts: vec![],
            validation: ValidationResult { valid: true, ocvalidate_ran: true, ocvalidate_output: None, issues: vec![] },
            warnings: vec![],
        }
    }

    #[test]
    fn round_trip() {
        let tmp = TempDir::new("manifest");
        let manifest = BuildManifest::new(sample_result(), true, Some(tmp.path().join("oc")));
        write(tmp.path(), &manifest).unwrap();
        let back = read(tmp.path()).unwrap();
        assert_eq!(back.opencore_version, "1.0.8");
        assert!(back.opencore_debug);
        assert_eq!(back.result.identity.model, "iMac19,1");

        std::fs::write(tmp.path().join(MANIFEST_FILE), b"{\"format\":\"other\"}").unwrap();
        assert!(read(tmp.path()).is_none());
        assert!(read(&tmp.path().join("missing")).is_none());
    }
}
