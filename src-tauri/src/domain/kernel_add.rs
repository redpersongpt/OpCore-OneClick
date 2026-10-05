//! Kernel->Add entry generation from the kexts actually placed in EFI/OC/Kexts:
//! reads each bundle's Info.plist (CFBundleIdentifier, CFBundleExecutable,
//! OSBundleLibraries), computes ExecutablePath ("" for codeless kexts), adds
//! one entry per selected plugin, keeps exactly one enabled VoodooInput, and
//! orders entries so every dependency loads first (stable topological sort;
//! Lilu first, VirtualSMC second).

use std::path::Path;

use crate::domain::model::KextSelection;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelAddEntry {
    pub arch: String,
    pub bundle_path: String,
    pub comment: String,
    pub enabled: bool,
    pub executable_path: String,
    pub max_kernel: String,
    pub min_kernel: String,
    pub plist_path: String,
    /// CFBundleIdentifier (for diagnostics / dependency checks).
    pub bundle_id: String,
}

pub fn build_kernel_add(selections: &[KextSelection], kexts_dir: &Path) -> Result<Vec<KernelAddEntry>, AppError> {
    todo!("build_kernel_add {} {}", selections.len(), kexts_dir.display())
}
