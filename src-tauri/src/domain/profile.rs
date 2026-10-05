//! Interpretation layer: raw scanner output → canonical `HardwareProfile`.

use crate::contracts::DetectedHardware;

use super::model::HardwareProfile;

/// Build the canonical profile from a scan. Must never panic on partial data:
/// unknown fields stay Unknown/None and lower `scan_confidence`.
///
/// Rules: CPU via `cpu_db::identify`; GPUs via `gpu_db::identify` (ignore
/// virtual/remote display adapters on bare metal); audio = the first analog
/// HDA codec (skip HDMI/DP codecs); ethernet = every wired NIC; wifi/bluetooth
/// = first of each; form factor from chassis types (8,9,10,14,30,31,32 →
/// laptop; 13 → all-in-one; 35/36 → mini PC) + battery/lid; chipset from the
/// LPC id then the board name; RAM in GB; VM from the hypervisor field.
pub fn build_profile(detected: &DetectedHardware) -> HardwareProfile {
    todo!("build_profile {}", detected.cpu.name)
}

/// Re-derive classifications after the user edited the profile manually
/// (e.g. re-identify GPU families from ids, fill CPU flags for a newly chosen
/// platform). User-chosen values (platform, form factor, layout id, disabled
/// GPUs) are kept.
pub fn refresh_profile(profile: HardwareProfile) -> HardwareProfile {
    todo!("refresh_profile {}", profile.cpu.name)
}
