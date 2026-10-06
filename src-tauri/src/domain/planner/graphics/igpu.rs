//! Intel iGPU framebuffer recipes: platform id, device-id and memory patches
//! per generation and role.
//!
//! Values follow the Dortania config.plist pages (desktop and laptop), the
//! Dortania Ventura page (Skylake run as Kaby Lake) and the WhateverGreen
//! Intel HD FAQ. Platform ids are numbers here; the caller writes them little
//! endian (0x3E9B0007 → `07009B3E`).

use crate::domain::chipset_db::ChipsetInfo;
use crate::domain::gpu_db;
use crate::domain::model::{CpuVendor, GpuFamily, MacOsVersion, ProfileGpu};

pub(super) const IG_PLATFORM_ID: &str = "AAPL,ig-platform-id";
pub(super) const SNB_PLATFORM_ID: &str = "AAPL,snb-platform-id";
pub(super) const ONE: &str = "01000000";

/// What the iGPU is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// Desktop iGPU driving monitors.
    Desktop,
    /// Desktop iGPU next to a dGPU, without connectors (Quick Sync only).
    Headless,
    /// Internal panel of a laptop or all-in-one.
    Panel,
    /// NUC-class mini PC with a mobile CPU and no panel.
    Nuc,
}

/// Framebuffer memory patches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mem {
    None,
    /// framebuffer-patch-enable + framebuffer-stolenmem 19 MB.
    Stolen,
    /// As `Stolen` plus framebuffer-fbmem 9 MB, for firmware stuck at 32 MB DVMT.
    StolenFb,
    /// Haswell laptops: framebuffer-cursormem 9 MB against cursor glitches.
    Cursor,
    /// Arrandale: single-link LVDS panel.
    SingleLink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recipe {
    /// Property name and platform id; None for Arrandale, which takes none.
    pub platform: Option<(&'static str, u32)>,
    /// `device-id` to inject (None = keep the real id).
    pub device_id: Option<u16>,
    pub mem: Mem,
}

/// The framebuffer recipe for `gpu` in `role` on `target`, or None when the
/// generation has no documented recipe for that role (no driver, no
/// connector-less framebuffer, ...).
pub(super) fn recipe(gpu: &ProfileGpu, role: Role, target: MacOsVersion) -> Option<Recipe> {
    use GpuFamily::*;
    use Role::*;

    let real = parse_id(gpu.device_id.as_deref());
    let spoof = gpu_db::device_id_for(gpu, target).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let effective = spoof.or(real);
    let name = gpu.name.to_ascii_lowercase();
    let ig = |id: u32, mem: Mem| {
        Some(Recipe {
            platform: Some((IG_PLATFORM_ID, id)),
            device_id: spoof,
            mem,
        })
    };

    match gpu.family {
        // Dortania laptop arrandale.md: no platform id, single-link LVDS.
        IntelIronLake => (role == Panel).then_some(Recipe {
            platform: None,
            device_id: None,
            mem: Mem::SingleLink,
        }),
        // Dortania sandy-bridge.md: the property is AAPL,snb-platform-id.
        // Desktops always fake 0x0126 (display) or 0x0102 (empty framebuffer).
        IntelSandyBridge => {
            let (id, device_id) = match role {
                Desktop | Nuc => (
                    0x0003_0010,
                    if role == Desktop { Some(0x0126) } else { spoof },
                ),
                Headless => (0x0005_0000, Some(0x0102)),
                Panel => (0x0001_0000, spoof),
            };
            Some(Recipe {
                platform: Some((SNB_PLATFORM_ID, id)),
                device_id,
                mem: Mem::None,
            })
        }
        // Dortania ivy-bridge.md; laptops default to the <=1366x768 id 0x01660003.
        IntelIvyBridge => ig(
            match role {
                Desktop => 0x0166_000A,
                Headless => 0x0162_0007,
                Panel => 0x0166_0003,
                Nuc => 0x0166_000B,
            },
            Mem::None,
        ),
        // Dortania haswell.md (desktop) and laptop haswell.md: HD 4200/4400/4600
        // run as HD 4600 (0x0412) on 0x0A260006, HD 5000 / Iris on 0x0A260005.
        IntelHaswell => match role {
            Desktop | Nuc => ig(0x0D22_0003, Mem::StolenFb),
            Headless => ig(0x0412_0004, Mem::None),
            Panel => ig(
                if effective == Some(0x0412) {
                    0x0A26_0006
                } else {
                    0x0A26_0005
                },
                Mem::Cursor,
            ),
        },
        // No Broadwell framebuffer is connector-less (WhateverGreen FAQ), so a
        // headless desktop keeps the display id with no monitor attached.
        IntelBroadwell => ig(
            match role {
                Desktop | Headless => 0x1622_0007,
                Panel => 0x1626_0006,
                Nuc => 0x1616_0002,
            },
            Mem::StolenFb,
        ),
        // macOS 13+: device-id and platform id of the closest Kaby Lake model
        // (Dortania ventura.md, WhateverGreen 1.6.1+).
        IntelSkylake if effective.is_some_and(is_kaby_lake_id) => {
            kaby_lake(role, effective, real, &name, spoof)
        }
        IntelSkylake => match role {
            Desktop => ig(0x1912_0000, Mem::StolenFb),
            Headless => ig(0x1912_0001, Mem::None),
            // HD 510 laptops: 0x191B0000 with device-id 0x1902 (Dortania laptop skylake.md).
            Panel => {
                let hd510 =
                    matches!(effective, Some(0x1902 | 0x1906 | 0x190B)) || name.contains("510");
                ig(if hd510 { 0x191B_0000 } else { 0x1916_0000 }, Mem::StolenFb)
            }
            Nuc => ig(
                match effective {
                    Some(0x191E) => 0x191E_0000,
                    Some(0x1926 | 0x1927) => 0x1926_0002,
                    Some(0x1932 | 0x193B) => 0x193B_0005,
                    _ => 0x1916_0002,
                },
                Mem::StolenFb,
            ),
        },
        IntelKabyLake => kaby_lake(role, effective, real, &name, spoof),
        // Dortania coffee-lake.md / comet-lake.md (desktop) and laptop
        // coffee-lake(-plus).md: UHD 620 runs as 0x3E9B on 0x3E9B0000, UHD 630
        // and Iris Plus keep their id on 0x3EA50009.
        IntelCoffeeLake | IntelCometLake => match role {
            Desktop => ig(0x3E9B_0007, Mem::Stolen),
            Headless => ig(
                if gpu.family == IntelCometLake {
                    0x9BC8_0003
                } else {
                    0x3E91_0003
                },
                Mem::None,
            ),
            Panel => ig(
                if spoof == Some(0x3E9B) {
                    0x3E9B_0000
                } else {
                    0x3EA5_0009
                },
                Mem::StolenFb,
            ),
            Nuc => ig(
                if effective == Some(0x3EA5) {
                    0x3EA5_0000
                } else {
                    0x3E9B_0007
                },
                Mem::StolenFb,
            ),
        },
        // Dortania laptop icelake.md: 0x8A520000 with the 32 MB DVMT patch.
        IntelIceLake => match role {
            Headless => None,
            _ => ig(0x8A52_0000, Mem::StolenFb),
        },
        _ => None,
    }
}

/// Dortania kaby-lake.md (desktop) and laptop kaby-lake.md. Amber Lake UHD
/// 617 and Kaby Lake-R UHD 620 (faked to 0x5916) use 0x87C00000.
fn kaby_lake(
    role: Role,
    effective: Option<u16>,
    real: Option<u16>,
    name: &str,
    spoof: Option<u16>,
) -> Option<Recipe> {
    let (id, mem) = match role {
        Role::Desktop => (0x5912_0000, Mem::Stolen),
        Role::Headless => (0x5912_0003, Mem::None),
        Role::Panel => {
            let amber_or_r = effective == Some(0x87C0)
                || real == Some(0x5917)
                || (real.is_none()
                    && name.contains("uhd")
                    && (name.contains("617") || name.contains("620")));
            (
                if amber_or_r { 0x87C0_0000 } else { 0x591B_0000 },
                Mem::StolenFb,
            )
        }
        Role::Nuc => (
            match effective {
                Some(0x591E) => 0x591E_0000,
                Some(0x5926 | 0x5927) => 0x5926_0002,
                _ => 0x591B_0000,
            },
            Mem::StolenFb,
        ),
    };
    Some(Recipe {
        platform: Some((IG_PLATFORM_ID, id)),
        device_id: spoof,
        mem,
    })
}

fn is_kaby_lake_id(id: u16) -> bool {
    (0x5900..=0x59FF).contains(&id) || id == 0x87C0
}

/// Memory patch properties (Data, little endian).
pub(super) fn mem_properties(mem: Mem) -> &'static [(&'static str, &'static str)] {
    const PATCH: &str = "framebuffer-patch-enable";
    const STOLEN: (&str, &str) = ("framebuffer-stolenmem", "00003001");
    const FBMEM: (&str, &str) = ("framebuffer-fbmem", "00009000");
    match mem {
        Mem::None => &[],
        Mem::Stolen => &[(PATCH, ONE), STOLEN],
        Mem::StolenFb => &[(PATCH, ONE), STOLEN, FBMEM],
        Mem::Cursor => &[(PATCH, ONE), ("framebuffer-cursormem", "00009000")],
        Mem::SingleLink => &[(PATCH, ONE), ("framebuffer-singlelink", ONE)],
    }
}

/// Backlight register fix for an internal panel on the iGPU (Dortania laptop
/// troubleshooting, WhateverGreen README): Coffee Lake and newer. From macOS
/// 13.4 Kaby/Coffee/Comet Lake need the alternative fix (every Ventura
/// installer is 13.4 or newer); Ice Lake keeps the original one.
pub(super) fn backlight_property(family: GpuFamily, target: MacOsVersion) -> Option<&'static str> {
    use GpuFamily::*;
    match family {
        IntelCoffeeLake | IntelCometLake if target >= MacOsVersion::Ventura => {
            Some("enable-backlight-registers-alternative-fix")
        }
        IntelCoffeeLake | IntelCometLake | IntelIceLake => Some("enable-backlight-registers-fix"),
        _ => None,
    }
}

/// IMEI `device-id` for a CPU on a PCH of the other generation (Dortania
/// sandy-bridge.md / ivy-bridge.md): Sandy Bridge on 7-series, Ivy Bridge on
/// 6-series. The graphics driver checks the IMEI id.
pub(super) fn imei_device_id(
    family: GpuFamily,
    chipset: Option<&ChipsetInfo>,
) -> Option<&'static str> {
    let chipset = chipset.filter(|c| c.vendor == CpuVendor::Intel)?;
    match (family, chipset.series) {
        (GpuFamily::IntelSandyBridge, 7) => Some("3A1C0000"),
        (GpuFamily::IntelIvyBridge, 6) => Some("3A1E0000"),
        _ => None,
    }
}

/// "3e9b", "0x3E9B" → 0x3E9B.
pub(super) fn parse_id(id: Option<&str>) -> Option<u16> {
    let t = id?.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 4 {
        return None;
    }
    u16::from_str_radix(t, 16).ok()
}
