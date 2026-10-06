//! AMD_Vanilla kernel patches for AMD builds: the pinned `patches.plist` for
//! the target is downloaded by the pipeline; here the core count is filled
//! in and the board-specific switches are applied.
//!
//! - PAT (`_mtrr_update_action`): the Algrey variant upstream enables by
//!   default works with every GPU; TRX40 runs with no PAT patch at all
//!   (AMD_Vanilla README, research-amd §3 #23-26).
//! - `IOPCIIsHotplugPort` stays off upstream; it is only for AM5 boards with
//!   on-board Thunderbolt/USB4 and Wi-Fi whose PCI devices disappear (the
//!   README names the ASUS X670E Hero/Gene/Extreme and ProArt X670E-Creator,
//!   research-amd §3 #19).

use crate::domain::amd_patches::{amd_vanilla_patches, select_pat_patch, set_hotplug_port_fix, PatPatch};
use crate::domain::chipset_db::{self, ChipsetInfo};
use crate::domain::model::{BinaryPatch, HardwareProfile, NoteLevel, PlanNote};
use crate::error::AppError;

/// Boards the AMD_Vanilla README lists for the hot-plug port patch.
const HOTPLUG_BOARDS: &[&str] =
    &["CROSSHAIR X670E HERO", "CROSSHAIR X670E GENE", "CROSSHAIR X670E EXTREME", "PROART X670E-CREATOR"];

/// Chipset of the board, from the profile's chipset name or the board model.
pub fn chipset(profile: &HardwareProfile) -> Option<ChipsetInfo> {
    profile
        .chipset
        .as_deref()
        .and_then(|c| chipset_db::from_name(c).or_else(|| chipset_db::from_board_name(c)))
        .or_else(|| chipset_db::from_board_name(&profile.motherboard_model))
}

pub fn pat_choice(chipset: Option<&ChipsetInfo>) -> PatPatch {
    match chipset {
        Some(c) if c.name.eq_ignore_ascii_case("TRX40") => PatPatch::Disabled,
        _ => PatPatch::Algrey,
    }
}

/// The board is one of those the hot-plug port patch was made for.
pub fn wants_hotplug_fix(profile: &HardwareProfile, chipset: Option<&ChipsetInfo>) -> bool {
    let am5 = chipset.is_some_and(ChipsetInfo::is_am5);
    let board = profile.motherboard_model.to_ascii_uppercase().replace("ROG ", "");
    am5 && profile.wifi.is_some() && HOTPLUG_BOARDS.iter().any(|b| board.contains(b))
}

/// Patches ready to append to `Kernel->Patch`, plus notes for the user.
pub fn prepare(
    patches_plist: &[u8],
    core_count: u32,
    profile: &HardwareProfile,
) -> Result<(Vec<BinaryPatch>, Vec<PlanNote>), AppError> {
    let mut patches = amd_vanilla_patches(patches_plist, core_count)?;
    let chipset = chipset(profile);
    let pat = pat_choice(chipset.as_ref());
    if select_pat_patch(&mut patches, pat) == 0 {
        tracing::warn!("AMD_Vanilla patch set has no PAT patch");
    }
    let mut notes = Vec::new();
    let hotplug = wants_hotplug_fix(profile, chipset.as_ref());
    let has_hotplug = set_hotplug_port_fix(&mut patches, hotplug);
    if has_hotplug && !hotplug && chipset.as_ref().is_some_and(ChipsetInfo::is_am5) {
        notes.push(PlanNote {
            level: NoteLevel::Info,
            component: "cpu".into(),
            title: "PCI devices missing on AM5".into(),
            detail: "If PCI devices (GPU, NVMe, network) are missing in macOS on a board with on-board \
                     Thunderbolt/USB4 and Wi-Fi, enable the AMD_Vanilla kernel patch \"IOPCIIsHotplugPort\" \
                     in config.plist (Kernel > Patch)."
                .into(),
        });
    }
    if pat == PatPatch::Disabled {
        tracing::info!("TRX40: AMD_Vanilla PAT patches disabled");
    }
    Ok((patches, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::ProfileNic;

    fn profile(chipset: &str, board: &str, wifi: bool) -> HardwareProfile {
        HardwareProfile {
            chipset: Some(chipset.into()),
            motherboard_model: board.into(),
            wifi: wifi.then(ProfileNic::default),
            ..HardwareProfile::default()
        }
    }

    #[test]
    fn pat_is_off_only_on_trx40() {
        let trx40 = profile("TRX40", "TRX40 AORUS XTREME", false);
        assert_eq!(pat_choice(chipset(&trx40).as_ref()), PatPatch::Disabled);
        let x570 = profile("X570", "ROG STRIX X570-F GAMING", false);
        assert_eq!(pat_choice(chipset(&x570).as_ref()), PatPatch::Algrey);
        assert_eq!(pat_choice(None), PatPatch::Algrey);
    }

    #[test]
    fn hotplug_only_for_the_listed_am5_boards_with_wifi() {
        let hero = profile("X670E", "ROG CROSSHAIR X670E HERO", true);
        assert!(wants_hotplug_fix(&hero, chipset(&hero).as_ref()));
        let hero_no_wifi = profile("X670E", "ROG CROSSHAIR X670E HERO", false);
        assert!(!wants_hotplug_fix(&hero_no_wifi, chipset(&hero_no_wifi).as_ref()));
        let creator = profile("X670E", "ProArt X670E-CREATOR WIFI", true);
        assert!(wants_hotplug_fix(&creator, chipset(&creator).as_ref()));
        let other = profile("B650", "B650 AORUS ELITE AX", true);
        assert!(!wants_hotplug_fix(&other, chipset(&other).as_ref()));
        let am4 = profile("X570", "CROSSHAIR X670E HERO", true);
        assert!(!wants_hotplug_fix(&am4, chipset(&am4).as_ref()));
    }
}
