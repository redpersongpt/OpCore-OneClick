//! AMD_Vanilla kernel patches (AMD-OSX/AMD_Vanilla, pinned) with the
//! core-count patches filled for the detected physical core count.

use super::model::BinaryPatch;

/// All AMD_Vanilla patches for families 15h/16h/17h/19h/1Ah, with the four
/// `algrey - Force cpuid_cores_per_package` patches' Replace byte set to
/// `core_count` (physical cores per package, 1..=255). Patch order, Find,
/// Mask, Replace, ReplaceMask, Base, Count/Limit/Skip and Min/MaxKernel must
/// match upstream `patches.plist` exactly.
pub fn amd_vanilla_patches(core_count: u32) -> Vec<BinaryPatch> {
    todo!("amd_vanilla_patches {core_count}")
}

/// Pinned AMD_Vanilla commit the patches were taken from.
pub const AMD_VANILLA_SOURCE: &str = "AMD-OSX/AMD_Vanilla master";
