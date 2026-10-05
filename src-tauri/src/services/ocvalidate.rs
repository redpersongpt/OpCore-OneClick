//! Validation of a generated EFI: OpenCore's own `ocvalidate` (schema and
//! semantic checks) plus layout checks (every referenced file exists, every
//! kext has its executable/Info.plist, required binaries present).

use std::path::Path;

use crate::contracts::ValidationResult;

/// Validate the EFI rooted at `efi_dir` (the directory that contains `OC/`
/// and `BOOT/`). `ocvalidate` is the host binary from the matching OpenCore
/// package, when available.
pub async fn validate_efi(efi_dir: &Path, ocvalidate: Option<&Path>) -> ValidationResult {
    todo!("validate_efi {} {ocvalidate:?}", efi_dir.display())
}
