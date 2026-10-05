//! AMD_Vanilla kernel patches (AMD-OSX/AMD_Vanilla). The upstream repository
//! carries no licence, so `patches.plist` is downloaded at build time from a
//! pinned commit (see `AMD_VANILLA`) instead of being vendored.

use crate::domain::kext_catalog::Pin;
use crate::error::AppError;

use super::model::BinaryPatch;

/// Pinned `patches.plist` (raw.githubusercontent.com URL at a fixed commit + SHA-256).
pub const AMD_VANILLA: Pin = Pin {
    version: "pinned",
    url: "",
    sha256: None,
};

/// Parse AMD_Vanilla `patches.plist` (its `Kernel/Patch` array) into
/// `BinaryPatch`es, keeping upstream order and every field exactly, and set the
/// core-count byte of the `algrey - Force cpuid_cores_per_package` patches'
/// Replace data to `core_count` (physical cores per package, 1..=255).
/// Rejects `core_count == 0`.
pub fn amd_vanilla_patches(patches_plist: &[u8], core_count: u32) -> Result<Vec<BinaryPatch>, AppError> {
    todo!("amd_vanilla_patches {} {core_count}", patches_plist.len())
}
