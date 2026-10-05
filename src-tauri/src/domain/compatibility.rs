//! Compatibility assessment: per-component support and the list of macOS
//! releases this machine can run, with reasons.

use crate::contracts::CompatibilityReport;

use super::model::{HardwareProfile, MacOsVersion};

/// Assess `profile`. When `target` is None the report is computed for the
/// recommended release. A release is "supported" when the CPU platform allows
/// it AND at least one display path (iGPU or dGPU, not disabled) is natively
/// supported on it (VMs excepted). Releases that need OCLP root patches for
/// graphics are listed with `needs_root_patch = true` and not recommended.
/// The recommended release is the newest fully supported one, preferring
/// Sequoia over Tahoe when Tahoe loses features this machine needs (analog
/// audio via AppleHDA, Intel Wi-Fi menu, Broadcom Wi-Fi without root patch).
pub fn assess(profile: &HardwareProfile, target: Option<MacOsVersion>) -> CompatibilityReport {
    todo!("assess {} {target:?}", profile.cpu.name)
}
