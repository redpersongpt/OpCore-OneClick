//! GPU identification and macOS support, keyed by PCI vendor/device id with
//! name parsing as fallback.
//!
//! Support facts come from Apple's driver match lists (as collected by OCLP
//! `pci_data.py`), WhateverGreen's fake-id table and FAQs, the Dortania
//! install and GPU buyers guides, the ChefKiss NootRX/NootedRed release
//! notes and OCLP 3.0 root-patch coverage. "Native" means no root patch: a
//! documented device-id spoof or a Lilu plugin (NootRX, NootedRed) still
//! counts as native.

mod ids;
mod names;
#[cfg(test)]
mod tests;

use super::model::{GpuFamily, GpuVendor, MacOsVersion, ProfileGpu};

use ids::{Hit, Kind};
use GpuFamily::*;
use MacOsVersion::{BigSur, Catalina, HighSierra, Mojave, Monterey, Tahoe, Ventura};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuIdentity {
    pub vendor: GpuVendor,
    pub family: GpuFamily,
    pub is_igpu: bool,
    /// Friendly model name when known from the id table ("Radeon RX 6600 XT").
    pub model_name: Option<String>,
}

/// Identify a GPU from PCI ids (lowercase hex, no prefix) and/or its name.
pub fn identify(vendor_id: Option<&str>, device_id: Option<&str>, name: &str) -> GpuIdentity {
    identify_with_revision(vendor_id, device_id, None, name)
}

/// Like [`identify`], with the PCI revision id ("c1", "0xdf") when the scanner
/// has it. The revision tells some Navi 2x models apart, e.g. RX 6750 GRE
/// 10GB is Navi 22 behind the Navi 23 id 0x73FF (revision 0xDF).
pub fn identify_with_revision(
    vendor_id: Option<&str>,
    device_id: Option<&str>,
    revision: Option<&str>,
    name: &str,
) -> GpuIdentity {
    let vendor = vendor_id.and_then(parse_hex16);
    let device = device_id.and_then(parse_hex16);
    let rev = revision.and_then(parse_hex8);
    let norm = names::normalize(name);

    let Some(vendor) = vendor else {
        return from_name(&norm, None);
    };
    match vendor {
        0x8086 => from_table(GpuVendor::Intel, device.and_then(ids::intel), &norm),
        0x1002 | 0x1022 => {
            let mut identity = from_table(GpuVendor::Amd, device.and_then(ids::amd), &norm);
            if let Some(id) = device {
                refine_navi(&mut identity, id, rev, &norm);
            }
            identity
        }
        0x10DE => from_table(GpuVendor::Nvidia, device.and_then(ids::nvidia), &norm),
        _ => match virtual_adapter(vendor, device) {
            Some(model) => GpuIdentity {
                vendor: GpuVendor::Virtual,
                family: VirtualDisplay,
                is_igpu: false,
                model_name: Some(model.to_string()),
            },
            None => match other_adapter(vendor) {
                Some(model) => GpuIdentity {
                    vendor: GpuVendor::Unknown,
                    family: GpuFamily::Unknown,
                    is_igpu: false,
                    model_name: Some(model.to_string()),
                },
                None => from_name(&norm, None),
            },
        },
    }
}

fn from_table(vendor: GpuVendor, hit: Option<Hit>, norm: &str) -> GpuIdentity {
    match hit {
        Some(hit) => GpuIdentity {
            vendor,
            family: hit.family,
            is_igpu: hit.igpu,
            model_name: Some(hit.name.to_string()),
        },
        None => from_name(norm, Some(vendor)),
    }
}

fn from_name(norm: &str, vendor: Option<GpuVendor>) -> GpuIdentity {
    let guess = match vendor {
        Some(vendor) => names::guess_for_vendor(norm, vendor),
        None => names::guess(norm),
    };
    GpuIdentity {
        vendor: guess.vendor,
        family: guess.family,
        is_igpu: guess.igpu,
        model_name: None,
    }
}

/// Revision-specific Navi 2x names, and RX 6750 GRE 10GB (Navi 22 on 0x73FF).
fn refine_navi(identity: &mut GpuIdentity, id: u16, rev: Option<u8>, norm: &str) {
    if let Some(name) = rev.and_then(|rev| ids::navi_revision_name(id, rev)) {
        identity.model_name = Some(name.to_string());
    }
    if id == 0x73FF && (rev == Some(0xDF) || norm.contains("6750 gre")) {
        identity.family = AmdNavi22;
        identity.model_name = Some("Radeon RX 6750 GRE 10GB".to_string());
    }
}

fn virtual_adapter(vendor: u16, device: Option<u16>) -> Option<&'static str> {
    Some(match vendor {
        0x15AD => "VMware SVGA",
        0x1AF4 => "Virtio GPU",
        0x1234 => "QEMU standard VGA (Bochs)",
        0x1B36 => "QXL paravirtual graphics",
        0x1414 => "Hyper-V video",
        0x80EE => "VirtualBox graphics adapter",
        0x1AB8 => "Parallels display adapter",
        0x5853 => "Xen platform graphics",
        // QEMU's emulated Cirrus GD5446.
        0x1013 if device == Some(0x00B8) => "QEMU Cirrus VGA",
        _ => return None,
    })
}

fn other_adapter(vendor: u16) -> Option<&'static str> {
    Some(match vendor {
        0x1A03 => "ASPEED BMC graphics",
        0x102B => "Matrox graphics (server BMC)",
        0x18CA => "XGI graphics",
        _ => return None,
    })
}

/// Parse "8086", "0x8086" or "8086 " into a u16.
fn parse_hex16(s: &str) -> Option<u16> {
    let t = s.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 4 {
        return None;
    }
    u16::from_str_radix(t, 16).ok()
}

fn parse_hex8(s: &str) -> Option<u8> {
    let t = s.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() || t.len() > 2 {
        return None;
    }
    u8::from_str_radix(t, 16).ok()
}

/// Extra requirement to drive this GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuRequirement {
    /// WhateverGreen + standard properties.
    Standard,
    /// NootRX replaces WhateverGreen (Navi 21/22/23 variants Apple never shipped).
    NootRx,
    /// NootedRed replaces WhateverGreen (Vega APUs).
    NootedRed,
    /// Needs a `device-id` spoof (bytes, little endian) to a supported id.
    DeviceIdSpoof([u8; 4]),
}

#[derive(Debug, Clone)]
pub struct GpuSupport {
    pub family: GpuFamily,
    /// Can macOS show a picture through this GPU on some release.
    pub display_capable: bool,
    /// First release with drivers.
    pub min_native: Option<MacOsVersion>,
    /// Last release with native (unpatched) drivers; None = still supported in Tahoe.
    pub max_native: Option<MacOsVersion>,
    /// Newest release reachable with OCLP root patches after install (None = no patch path).
    pub max_with_root_patch: Option<MacOsVersion>,
    pub requirement: GpuRequirement,
    /// Recommended boot-args for this GPU (e.g. "agdpmod=pikera" for Navi).
    pub boot_args: Vec<&'static str>,
    pub notes: Vec<String>,
}

/// Support facts for one concrete GPU (device-id specific where it matters,
/// e.g. RX 6600 vs RX 6500, Navi 21 XTXH, Lexa spoof, UHD 630 desktop vs mobile).
pub fn support(gpu: &ProfileGpu) -> GpuSupport {
    let mut s = family_support(gpu.family);
    match device_hit(gpu) {
        Some((id, hit)) => apply_device(&mut s, id, hit.kind),
        None => apply_name_hints(&mut s, gpu),
    }
    if s.display_capable && names::is_compute_card(&names::normalize(&gpu.name)) {
        make_headless(&mut s, NOTE_COMPUTE);
    }
    s
}

/// Native (no root patch) support of `gpu` on `version`.
pub fn natively_supported_on(gpu: &ProfileGpu, version: MacOsVersion) -> bool {
    let s = support(gpu);
    s.display_capable
        && !matches!(s.min_native, Some(min) if version < min)
        && !matches!(s.max_native, Some(max) if version > max)
}

/// The `device-id` property to inject for `target`, little endian (None =
/// keep the real id). Unlike `support().requirement` this is release aware:
/// Skylake keeps its own (or WhateverGreen's) id up to Monterey and takes the
/// Kaby Lake spoof from Ventura on.
pub fn device_id_for(gpu: &ProfileGpu, target: MacOsVersion) -> Option<[u8; 4]> {
    if gpu.family == IntelSkylake && target < Ventura {
        return match device_hit(gpu) {
            Some((_, hit)) => match hit.kind {
                Kind::Fake(t) | Kind::FakeGuess(t) | Kind::Gt1(Some(t)) => Some(le(t)),
                _ => None,
            },
            None => None,
        };
    }
    match support(gpu).requirement {
        GpuRequirement::DeviceIdSpoof(bytes) => Some(bytes),
        _ => None,
    }
}

/// For GPUs whose requirement is NootRX but that also run with WhateverGreen
/// plus a device-id spoof (RX 6950 XT, RX 6900 XT XTXH, RX 6650 XT, W6600M):
/// the spoof id, little endian. That path keeps WhateverGreen for an Intel
/// iGPU; it needs `-radcodec` and, with iMac SMBIOS, `agdpmod=pikera`.
pub fn weg_spoof_alternative(gpu: &ProfileGpu) -> Option<[u8; 4]> {
    let id = match device_hit(gpu) {
        Some((_, hit)) => match hit.kind {
            Kind::NootRx(alt) => alt,
            _ => None,
        },
        None => nootrx_name_variant(gpu).flatten(),
    }?;
    Some(le(id))
}

/// All families in UI order with labels (for the manual editor).
pub fn all_families() -> &'static [(GpuFamily, &'static str)] {
    FAMILY_LABELS
}

/// UI label of one family.
pub fn family_label(family: GpuFamily) -> &'static str {
    FAMILY_LABELS
        .iter()
        .find(|(f, _)| *f == family)
        .map_or("Unknown", |(_, label)| label)
}

static FAMILY_LABELS: &[(GpuFamily, &str)] = &[
    (
        IntelGma,
        "Intel GMA / chipset graphics (before HD Graphics)",
    ),
    (
        IntelIronLake,
        "Intel HD Graphics, 1st gen (Clarkdale / Arrandale)",
    ),
    (IntelSandyBridge, "Intel HD 2000 / 3000 (Sandy Bridge)"),
    (IntelIvyBridge, "Intel HD 2500 / 4000 (Ivy Bridge)"),
    (
        IntelHaswell,
        "Intel HD 4200-5000 / Iris 5100 / Iris Pro 5200 (Haswell)",
    ),
    (
        IntelBroadwell,
        "Intel HD 5300-6000 / Iris 6100 / Iris Pro 6200 (Broadwell)",
    ),
    (IntelSkylake, "Intel HD 510-530 / Iris 540-580 (Skylake)"),
    (
        IntelKabyLake,
        "Intel HD 610-630 / UHD 617-620 / Iris Plus 640-650 (Kaby Lake, Amber Lake)",
    ),
    (
        IntelCoffeeLake,
        "Intel UHD 610-630 / Iris Plus 645-655 (Coffee Lake, Whiskey Lake)",
    ),
    (IntelCometLake, "Intel UHD 610-630 (Comet Lake)"),
    (IntelIceLake, "Intel Iris Plus G4 / G7 (Ice Lake)"),
    (
        IntelLowPower,
        "Intel Atom / Celeron / Pentium Silver / N-series graphics",
    ),
    (
        IntelXe,
        "Intel Iris Xe / UHD 7xx / Arc iGPU (Tiger Lake and newer)",
    ),
    (IntelArc, "Intel Arc / Iris Xe MAX (discrete)"),
    (AmdTeraScale, "AMD Radeon HD 2000-6000 (TeraScale)"),
    (AmdGcn1, "AMD Radeon HD 7700-7900 / R7 / R9 270-280 (GCN 1)"),
    (
        AmdGcn2,
        "AMD Radeon HD 7790 / R7 260 / R9 290 / 390 (GCN 2)",
    ),
    (AmdGcn3, "AMD Radeon R9 285 / 380 / Fury / Nano (GCN 3)"),
    (
        AmdPolaris,
        "AMD Radeon RX 460-590 / Pro WX 4100-7100 (Polaris)",
    ),
    (
        AmdLexa,
        "AMD Radeon RX 540 / 550 / 640 / Pro WX 2100-3200 (Polaris 12 Lexa)",
    ),
    (
        AmdVega10,
        "AMD Radeon RX Vega 56 / 64 / Pro Vega (Vega 10 / 12)",
    ),
    (
        AmdVega20,
        "AMD Radeon VII / Pro VII / Pro Vega II (Vega 20)",
    ),
    (AmdNavi10, "AMD Radeon RX 5600 / 5700 (Navi 10)"),
    (AmdNavi12, "AMD Radeon Pro 5600M (Navi 12)"),
    (AmdNavi14, "AMD Radeon RX 5300 / 5500 (Navi 14)"),
    (AmdNavi21, "AMD Radeon RX 6800 / 6900 / 6950 (Navi 21)"),
    (AmdNavi22, "AMD Radeon RX 6700 / 6750 / 6800M (Navi 22)"),
    (AmdNavi23, "AMD Radeon RX 6600 / 6650 (Navi 23)"),
    (AmdNavi24, "AMD Radeon RX 6300 / 6400 / 6500 (Navi 24)"),
    (AmdRdna3Plus, "AMD Radeon RX 7000 / 9000 (RDNA 3 / 4)"),
    (
        AmdApuVega,
        "AMD Radeon Vega APU graphics (Ryzen 2000-5000 G/U/H)",
    ),
    (AmdApuRdna, "AMD Radeon 610M-890M APU graphics (RDNA 2 / 3)"),
    (
        AmdApuLegacy,
        "AMD APU graphics before Vega (A-series, Athlon)",
    ),
    (NvidiaTesla, "NVIDIA GeForce 8000-300 (Tesla)"),
    (NvidiaFermi, "NVIDIA GeForce 400-500 (Fermi)"),
    (
        NvidiaKepler,
        "NVIDIA GeForce GT 710-740 / GTX 650-780 / Titan (Kepler)",
    ),
    (NvidiaMaxwell, "NVIDIA GeForce GTX 745-980 Ti (Maxwell)"),
    (
        NvidiaPascal,
        "NVIDIA GeForce GT 1030 / GTX 1050-1080 Ti (Pascal)",
    ),
    (
        NvidiaModern,
        "NVIDIA GeForce GTX 16 / RTX (Turing and newer)",
    ),
    (VirtualDisplay, "Virtual machine display adapter"),
    (GpuFamily::Unknown, "Unknown"),
];

// ── Support facts ───────────────────────────────────────────────────────────

const NOTE_NON_METAL: &str =
    "Non-Metal GPU: OCLP can root-patch it onto newer releases after install, but without Metal \
     acceleration the UI and many apps misbehave, so that path is not offered.";
const NOTE_ROOT_PATCH: &str =
    "Releases after the native range need OCLP root patches after install (lowered SIP, \
     AMFIPass, SecureBootModel Disabled; macOS 26 needs OCLP 3.0). Not recommended for daily use.";
const NOTE_AVX2: &str =
    "macOS 13 and newer need an AVX2 CPU (Intel Haswell or newer, or AMD Ryzen) for this \
     GPU's drivers; older CPUs stop at macOS 12 unless OCLP root patches are used.";
const NOTE_TAHOE_WEG: &str =
    "macOS 26 needs Lilu 1.7.2 and WhateverGreen 1.7.1 or newer with AMD GPUs.";
const NOTE_AGDP: &str = "agdpmod=pikera avoids the black screen caused by the AppleGraphicsDevicePolicy board-id \
     check with iMac/Macmini SMBIOS; MacPro7,1 and iMacPro1,1 do not need it. On macOS 26 the WhateverGreen \
     maintainer recommends agdpmod=ignore instead.";
const NOTE_NOOTRX_BUILDS: &str =
    "NootRX has no tagged releases; it comes from the project's nightly builds.";
const NOTE_NO_PIKERA: &str = "Do not use agdpmod=pikera with Polaris or Vega cards.";
const NOTE_DISABLE: &str =
    "macOS has no driver for this GPU. Disable it: -wegnoegpu when an iGPU drives the \
     display, otherwise the disable-gpu property or an SSDT.";
const NOTE_COMPUTE: &str =
    "Compute / data-centre board without display outputs; it cannot drive a screen.";
const NOTE_HEADLESS: &str =
    "This iGPU cannot drive a display under macOS; it is only usable headless (Quick Sync) \
     next to a supported dGPU.";
const NOTE_GT1: &str = "Entry-level GT1 iGPU: Apple never shipped it in a Mac and the Dortania guides list \
     it as unsupported. Expect partial acceleration with glitches at best; use a supported dGPU if possible.";
const NOTE_SKL_ROOT_PATCH: &str =
    "No Kaby Lake id matches this Skylake model: macOS 13 and newer only through the OCLP Skylake \
     root patch (Metal 31001; macOS 26 needs OCLP 3.0). Not recommended for daily use.";

fn supported(family: GpuFamily, min: MacOsVersion, max: Option<MacOsVersion>) -> GpuSupport {
    GpuSupport {
        family,
        display_capable: true,
        min_native: Some(min),
        max_native: max,
        max_with_root_patch: None,
        requirement: GpuRequirement::Standard,
        boot_args: Vec::new(),
        notes: Vec::new(),
    }
}

fn unsupported(family: GpuFamily, note: &str) -> GpuSupport {
    GpuSupport {
        family,
        display_capable: false,
        min_native: None,
        max_native: None,
        max_with_root_patch: None,
        requirement: GpuRequirement::Standard,
        boot_args: Vec::new(),
        notes: vec![note.to_string()],
    }
}

fn with_root_patch(mut s: GpuSupport, note: &str) -> GpuSupport {
    s.max_with_root_patch = Some(Tahoe);
    s.notes.push(note.to_string());
    s
}

fn note(mut s: GpuSupport, text: &str) -> GpuSupport {
    s.notes.push(text.to_string());
    s
}

fn le(id: u16) -> [u8; 4] {
    let [lo, hi] = id.to_le_bytes();
    [lo, hi, 0, 0]
}

/// Facts that hold for every member of a family; `apply_device` refines them.
fn family_support(family: GpuFamily) -> GpuSupport {
    match family {
        IntelGma => unsupported(family, "Intel GMA graphics have no drivers in macOS 10.13 or newer."),
        IntelIronLake => note(
            note(
                supported(family, HighSierra, Some(HighSierra)),
                "Only the mobile Arrandale iGPU has a driver, and only for LVDS panels (not eDP).",
            ),
            NOTE_NON_METAL,
        ),
        IntelSandyBridge => note(
            note(
                supported(family, HighSierra, Some(HighSierra)),
                "HD 3000 is native up to macOS 10.13. Desktops use AAPL,snb-platform-id with device-id 0x0126 \
                 (display) or 0x0102 (headless); HD 2000 cannot drive a display.",
            ),
            NOTE_NON_METAL,
        ),
        IntelIvyBridge => with_root_patch(
            note(
                supported(family, HighSierra, Some(BigSur)),
                "HD 4000 is native up to macOS 11. HD 2500 cannot drive a display.",
            ),
            "Monterey and newer only through OCLP root patches (Metal 3802; macOS 15 needs MetallibSupportPkg, \
             macOS 26 needs OCLP 3.0). Not recommended for daily use.",
        ),
        IntelHaswell => with_root_patch(
            note(
                supported(family, HighSierra, Some(Monterey)),
                "Haswell graphics are native up to macOS 12. HD 4200/4400/4600 variants run with the HD 4600 \
                 device-id 0x0412; HD 5000, Iris 5100 and Iris Pro 5200 are native.",
            ),
            NOTE_ROOT_PATCH,
        ),
        IntelBroadwell => with_root_patch(
            note(supported(family, HighSierra, Some(Monterey)), "Broadwell graphics are native up to macOS 12."),
            NOTE_ROOT_PATCH,
        ),
        // Until the model is known only the native range is certain; the
        // Kaby Lake spoof (device or name) lifts the cap.
        IntelSkylake => {
            let mut s = note(
                supported(family, HighSierra, Some(Monterey)),
                "Skylake drivers are native up to macOS 12. Ventura and newer run it as Kaby Lake: spoof device-id \
                 and AAPL,ig-platform-id to the closest Kaby Lake model (WhateverGreen 1.6.1 or newer); add \
                 -igfxsklaskbl when the same EFI also boots macOS 12 or older.",
            );
            s.max_with_root_patch = Some(Tahoe);
            s
        }
        IntelKabyLake => note(
            supported(family, HighSierra, None),
            "Kaby Lake drivers are still present in macOS 26.",
        ),
        IntelCoffeeLake => note(
            supported(family, Mojave, None),
            "Desktop UHD 630 is native from macOS 10.14 (laptop Coffee Lake from 10.13.6); on 10.13.6 a desktop \
             UHD 630 only runs with the Kaby Lake HD 630 device-id 0x5912. 300-series boards may need BusID \
             patching for a black screen after boot.",
        ),
        IntelCometLake => note(
            supported(family, Catalina, None),
            "Comet Lake graphics are native from macOS 10.15.4. Desktop UHD 630 (0x9BC5/0x9BC8) needs no device-id; \
             spoofing it to 0x3E9B is a community workaround for black screens only.",
        ),
        IntelIceLake => {
            let mut s = note(
                supported(family, Catalina, None),
                "Ice Lake Iris Plus is native from macOS 10.15.4. Set DVMT pre-allocated to 256 MB (or patch \
                 stolenmem/fbmem); HDMI is not supported by Apple's Ice Lake driver, use DP/USB-C outputs. \
                 MacBookAir9,1 stops at macOS 15, use MacBookPro16,2 for macOS 26.",
            );
            s.boot_args = vec!["-igfxcdc", "-igfxdvmt"];
            s
        }
        IntelLowPower => unsupported(
            family,
            "Atom-class graphics (Celeron/Pentium N and J, Pentium Silver, Alder Lake-N) have no macOS driver.",
        ),
        IntelXe => unsupported(
            family,
            "Tiger Lake and newer Intel graphics (Iris Xe, UHD 7xx, Arc iGPU) have no macOS driver. Desktops \
             need a supported AMD dGPU; laptops cannot run accelerated graphics.",
        ),
        IntelArc => note(
            unsupported(family, "Intel Arc / Iris Xe MAX discrete GPUs have no macOS driver."),
            NOTE_DISABLE,
        ),
        AmdTeraScale => note(
            note(
                supported(family, HighSierra, Some(HighSierra)),
                "TeraScale cards are non-Metal and stop at macOS 10.13; only chips Apple shipped are matched by its \
                 drivers.",
            ),
            NOTE_NON_METAL,
        ),
        AmdGcn1 | AmdGcn2 | AmdGcn3 => with_root_patch(
            note(
                supported(family, HighSierra, Some(Monterey)),
                "GCN 1-3 cards are native up to macOS 12 (Lilu + WhateverGreen).",
            ),
            "Ventura and newer only through the OCLP \"AMD Legacy GCN\" root patch (Metal 31001, KDK required; \
             macOS 26 needs OCLP 3.0). Not recommended for daily use.",
        ),
        AmdPolaris => {
            let mut s = supported(family, HighSierra, None);
            s.notes = vec![
                NOTE_TAHOE_WEG.to_string(),
                NOTE_NO_PIKERA.to_string(),
                NOTE_AVX2.to_string(),
                "Some RX 570/580 users reported black screens after macOS 26 point updates that cleared after a \
                 kext update or reinstall."
                    .to_string(),
            ];
            s
        }
        AmdLexa => {
            let mut s = supported(family, HighSierra, None);
            s.requirement = GpuRequirement::DeviceIdSpoof(le(0x67FF));
            s.boot_args = vec!["-radcodec"];
            s.notes = vec![
                "Polaris 12 (Lexa) is not in Apple's id list: spoof device-id to Baffin 0x67FF (RX 560); \
                 -radcodec keeps hardware video encoding. Some cards also need the no-gfx-spoof property."
                    .to_string(),
                NOTE_TAHOE_WEG.to_string(),
                NOTE_AVX2.to_string(),
            ];
            s
        }
        AmdVega10 | AmdVega20 => {
            let min = if family == AmdVega20 { Mojave } else { HighSierra };
            let mut s = supported(family, min, None);
            s.notes = vec![NOTE_TAHOE_WEG.to_string(), NOTE_NO_PIKERA.to_string(), NOTE_AVX2.to_string()];
            if family == AmdVega20 {
                s.notes.push("Vega 20 (Radeon VII, Pro VII, Pro Vega II) is native from macOS 10.14.5.".to_string());
            }
            s
        }
        AmdNavi10 | AmdNavi14 | AmdNavi21 | AmdNavi23 => {
            let min = match family {
                AmdNavi21 => BigSur,
                AmdNavi23 => Monterey,
                _ => Catalina,
            };
            let mut s = supported(family, min, None);
            s.boot_args = vec!["agdpmod=pikera"];
            s.notes = vec![NOTE_AGDP.to_string(), NOTE_TAHOE_WEG.to_string(), NOTE_AVX2.to_string()];
            match family {
                AmdNavi21 => s.notes.push(
                    "Navi 21 is native from macOS 11.4. Many boards sit behind an extra PCIe bridge, so the \
                     DeviceProperties path is longer and SSDT-BRG0 may be needed."
                        .to_string(),
                ),
                AmdNavi23 => s.notes.push("Navi 23 is native from macOS 12.1.".to_string()),
                _ => s.notes.push("Navi 10/14 are native from macOS 10.15.1.".to_string()),
            }
            s
        }
        AmdNavi12 => {
            let mut s = supported(family, Catalina, None);
            s.notes = vec![
                "Navi 12 (Radeon Pro 5600M) only shipped in the MacBookPro16,4.".to_string(),
                NOTE_TAHOE_WEG.to_string(),
                NOTE_AVX2.to_string(),
            ];
            s
        }
        AmdNavi22 => {
            let mut s = supported(family, Monterey, None);
            s.requirement = GpuRequirement::NootRx;
            s.notes = vec![
                "Apple never shipped Navi 22: it needs NootRX (macOS 12 to 26), which replaces WhateverGreen. \
                 Known issues: green artefacts in some 3D apps and black screens with DRM video."
                    .to_string(),
                NOTE_NOOTRX_BUILDS.to_string(),
                NOTE_AVX2.to_string(),
            ];
            s
        }
        AmdNavi24 => unsupported(family, NOTE_DISABLE),
        AmdRdna3Plus => unsupported(family, NOTE_DISABLE),
        AmdApuVega => {
            let mut s = supported(family, Catalina, None);
            s.requirement = GpuRequirement::NootedRed;
            s.notes = vec![
                "Vega APU graphics need NootedRed (macOS 10.15 to 26), which replaces WhateverGreen. Use \
                 MacBookPro16,2 or iMac20,1 SMBIOS and set UMA frame buffer to 512 MB or more (1 GB recommended)."
                    .to_string(),
                "No other GCN 5 / RDNA dGPU may stay enabled next to NootedRed. If the macOS 26 installer stalls, \
                 install with NootedRed disabled and enable it afterwards."
                    .to_string(),
            ];
            s
        }
        AmdApuRdna => unsupported(
            family,
            "RDNA 2/3 APU graphics (Rembrandt, Raphael, Phoenix, Strix and newer) have no macOS driver; desktops \
             need a supported dGPU.",
        ),
        AmdApuLegacy => unsupported(
            family,
            "AMD APU graphics before Vega (A-series, Athlon, chipset graphics) have no usable macOS driver.",
        ),
        NvidiaTesla => note(
            note(
                supported(family, HighSierra, Some(HighSierra)),
                "Tesla-class GeForce cards are non-Metal and stop at macOS 10.13.",
            ),
            NOTE_NON_METAL,
        ),
        NvidiaFermi => note(
            note(
                supported(family, HighSierra, Some(HighSierra)),
                "Fermi cards are non-Metal and stop at macOS 10.13.",
            ),
            NOTE_NON_METAL,
        ),
        NvidiaKepler => with_root_patch(
            note(
                supported(family, HighSierra, Some(BigSur)),
                "Kepler is native up to macOS 11 Big Sur. The card needs a UEFI GOP VBIOS.",
            ),
            "Monterey and newer only through OCLP root patches (Metal 3802; macOS 15 needs MetallibSupportPkg, \
             macOS 26 needs OCLP 3.0). Not recommended for daily use.",
        ),
        NvidiaMaxwell | NvidiaPascal => {
            let mut s = note(
                note(
                    supported(family, HighSierra, Some(HighSierra)),
                    "Only macOS 10.13.6 (build 17G14042) with NVIDIA Web Driver 387.10.10.10.40.140; set NVRAM \
                     nvda_drv=1 (7C436110-AB2A-4BBB-A880-FE41995C9F82). Apple never shipped drivers for it.",
                ),
                "OCLP's web-driver root patch reaches newer releases with OpenGL only (no Metal); it is \
                 experimental, many apps break, and it is not offered.",
            );
            s.boot_args = vec!["nvda_drv_vrl=1"];
            s
        }
        NvidiaModern => unsupported(family, NOTE_DISABLE),
        VirtualDisplay => {
            let mut s = supported(family, HighSierra, None);
            s.min_native = None;
            s.notes = vec![
                "Virtual display adapter: macOS drives it as a plain framebuffer without Metal acceleration \
                 unless a real GPU is passed through."
                    .to_string(),
            ];
            s
        }
        GpuFamily::Unknown => unsupported(
            family,
            "The GPU could not be identified; pick its family manually if you know it.",
        ),
    }
}

/// The id-table entry for this GPU, if it belongs to the profile's family.
/// A family the user changed by hand wins over the table.
fn device_hit(gpu: &ProfileGpu) -> Option<(u16, Hit)> {
    let device = gpu.device_id.as_deref().and_then(parse_hex16)?;
    let vendor = gpu
        .vendor_id
        .as_deref()
        .and_then(parse_hex16)
        .or(match gpu.vendor {
            GpuVendor::Intel => Some(0x8086),
            GpuVendor::Amd => Some(0x1002),
            GpuVendor::Nvidia => Some(0x10DE),
            _ => None,
        })?;
    let hit = match vendor {
        0x8086 => ids::intel(device),
        0x1002 | 0x1022 => ids::amd(device),
        0x10DE => ids::nvidia(device),
        _ => None,
    }?;
    (hit.family == gpu.family).then_some((device, hit))
}

fn make_headless(s: &mut GpuSupport, text: &str) {
    s.display_capable = false;
    s.requirement = GpuRequirement::Standard;
    s.boot_args.clear();
    s.notes.insert(0, text.to_string());
}

fn make_unsupported(s: &mut GpuSupport, reason: &str) {
    s.display_capable = false;
    s.min_native = None;
    s.max_native = None;
    s.max_with_root_patch = None;
    s.requirement = GpuRequirement::Standard;
    s.boot_args.clear();
    s.notes = vec![reason.to_string()];
}

fn is_amd(family: GpuFamily) -> bool {
    matches!(
        family,
        AmdTeraScale
            | AmdGcn1
            | AmdGcn2
            | AmdGcn3
            | AmdPolaris
            | AmdLexa
            | AmdVega10
            | AmdVega20
            | AmdNavi10
            | AmdNavi12
            | AmdNavi14
            | AmdNavi21
            | AmdNavi22
            | AmdNavi23
    )
}

fn apply_spoof(s: &mut GpuSupport, target: u16, inferred: bool) {
    s.requirement = GpuRequirement::DeviceIdSpoof(le(target));
    let mut text = format!(
        "Inject device-id 0x{target:04X}: the real id is missing from Apple's driver list or works \
         poorly with it."
    );
    if inferred {
        text.push_str(" This target is inferred from a related model, not documented upstream; it may not work.");
    }
    s.notes.insert(0, text);
    if is_amd(s.family) && !s.boot_args.contains(&"-radcodec") {
        s.boot_args.push("-radcodec");
    }
}

fn apply_nootrx(s: &mut GpuSupport, alternative: Option<u16>) {
    let min = if s.family == AmdNavi21 {
        BigSur
    } else {
        Monterey
    };
    s.requirement = GpuRequirement::NootRx;
    s.boot_args.clear();
    s.min_native = Some(min);
    s.max_native = None;
    let mut notes = vec![format!(
        "Apple never shipped this model: it needs NootRX (macOS {} to 26), which replaces WhateverGreen.",
        min.id()
    )];
    if let Some(alt) = alternative {
        notes.push(format!(
            "Alternative that keeps WhateverGreen: spoof device-id to 0x{alt:04X} and add -radcodec \
             (plus agdpmod=pikera with iMac SMBIOS)."
        ));
    }
    notes.push(NOTE_NOOTRX_BUILDS.to_string());
    notes.push(NOTE_AVX2.to_string());
    s.notes = notes;
}

fn apply_device(s: &mut GpuSupport, id: u16, kind: Kind) {
    match kind {
        Kind::Family => {}
        // Skylake's own fakes only matter up to macOS 12; apply_skylake sets
        // the Kaby Lake spoof used from Ventura on.
        Kind::Fake(target) | Kind::FakeGuess(target) if s.family == IntelSkylake => {
            s.notes.insert(
                0,
                format!("Up to macOS 12: inject device-id 0x{target:04X}."),
            );
        }
        Kind::Fake(target) => apply_spoof(s, target, false),
        Kind::FakeGuess(target) => apply_spoof(s, target, true),
        Kind::Gt1(target) => {
            if let Some(target) = target {
                s.requirement = GpuRequirement::DeviceIdSpoof(le(target));
                s.notes
                    .insert(0, format!("Inject device-id 0x{target:04X}."));
            }
            s.notes.insert(0, NOTE_GT1.to_string());
        }
        Kind::Headless => make_headless(s, NOTE_HEADLESS),
        Kind::Compute => make_headless(s, NOTE_COMPUTE),
        Kind::NootRx(alternative) => apply_nootrx(s, alternative),
        Kind::NoDriver(reason) => {
            make_unsupported(s, reason);
            if !ids::family_is_igpu(s.family) {
                s.notes.push(NOTE_DISABLE.to_string());
            }
            return;
        }
    }
    if !s.display_capable {
        return;
    }
    match s.family {
        IntelSkylake => apply_skylake(s, id, kind),
        IntelKabyLake if matches!(id, 0x591C | 0x87C0 | 0x87CA) => {
            s.min_native = Some(Mojave);
            s.notes
                .push("Amber Lake graphics are native from macOS 10.14.1.".to_string());
        }
        IntelCoffeeLake => {
            // CFL-H/U shipped in MacBookPro15,x with 10.13.6; desktop S parts,
            // the ids faked to them (0x3E94 -> 0x3E92) and Whiskey Lake start
            // with 10.14.
            if matches!(id, 0x3E9B | 0x3EA5..=0x3EA9) {
                s.min_native = Some(HighSierra);
            }
            if id == 0x3E98 {
                s.notes.push(
                    "Coffee Lake-R 0x3E98 is native from macOS 10.14.4; older releases need device-id 0x3E92."
                        .to_string(),
                );
            }
        }
        IntelCometLake if id == 0x9BC4 => s.notes.push(
            "Comet Lake-H 0x9BC4 is native; device-id 0x3E9B is an optional alternative."
                .to_string(),
        ),
        IntelCometLake if matches!(id, 0x9B21 | 0x9BAA | 0x9BAC) => s.notes.push(
            "This Comet Lake-U id is also listed for Celeron/Pentium GT1 parts; those get no usable \
             acceleration even with the device-id."
                .to_string(),
        ),
        AmdVega10 if (0x69A0..=0x69AF).contains(&id) => s.min_native = Some(Mojave),
        // Desktop Cape Verde boards (HD 7730/7750/7770, R7 250/250X/250E, R9 255).
        AmdGcn1 if matches!(id, 0x682B | 0x6835 | 0x6837 | 0x6839 | 0x683B | 0x683D | 0x683F) => {
            s.boot_args.push("radpg=15");
            s.notes
                .push("Cape Verde cards need radpg=15 to initialise.".to_string());
        }
        _ => {}
    }
}

fn apply_skylake(s: &mut GpuSupport, id: u16, kind: Kind) {
    match ids::skylake_to_kaby(id) {
        Some(kaby) => skylake_as_kaby(s, kaby, ids::skylake_spoof_is_inferred(id)),
        None => {
            if let Kind::Fake(target) | Kind::FakeGuess(target) = kind {
                // Also right after the OCLP Skylake root patch restores the drivers.
                s.requirement = GpuRequirement::DeviceIdSpoof(le(target));
            }
            s.notes.push(NOTE_SKL_ROOT_PATCH.to_string());
        }
    }
}

/// Skylake with a Kaby Lake counterpart: native through macOS 26 with the spoof.
fn skylake_as_kaby(s: &mut GpuSupport, kaby: u16, inferred: bool) {
    s.requirement = GpuRequirement::DeviceIdSpoof(le(kaby));
    s.max_native = None;
    s.max_with_root_patch = None;
    let mut text = format!(
        "macOS 13 and newer: spoof as Kaby Lake with device-id 0x{kaby:04X} and the matching Kaby Lake \
         AAPL,ig-platform-id."
    );
    if inferred {
        text.push_str(" This Kaby Lake target is inferred, not documented upstream.");
    }
    s.notes.push(text);
}

/// Name-only refinements when no PCI id identifies the exact model.
fn apply_name_hints(s: &mut GpuSupport, gpu: &ProfileGpu) {
    let n = names::normalize(&gpu.name);
    let has = |needle: &str| n.contains(needle);
    match gpu.family {
        AmdPolaris if has("2048sp") || has("590 gme") => make_unsupported(
            s,
            "RX 580 2048SP / RX 590 GME (Polaris 20 XL) is not matched by Apple's driver; only VBIOS-flash \
             workarounds exist.",
        ),
        AmdPolaris if has("vega m") => make_unsupported(s, "Radeon RX Vega M (Kaby Lake-G) has no working macOS acceleration."),
        AmdNavi21 | AmdNavi23 => {
            if let Some(alternative) = nootrx_name_variant(gpu) {
                apply_nootrx(s, alternative);
            }
        }
        IntelSandyBridge if has("2000") => make_headless(s, NOTE_HEADLESS),
        IntelIvyBridge if has("2500") => make_headless(s, NOTE_HEADLESS),
        IntelHaswell if ["4200", "4400", "4600", "4700"].iter().any(|m| has(m)) => {
            s.requirement = GpuRequirement::DeviceIdSpoof(le(0x0412));
        }
        IntelSkylake => {
            let kaby = if has("p530") || has("535") {
                Some(0x591B)
            } else if has("530") {
                Some(0x5912)
            } else if has("520") {
                Some(0x5916)
            } else if has("515") {
                Some(0x591E)
            } else if has("540") {
                Some(0x5926)
            } else if has("550") || has("555") {
                Some(0x5927)
            } else {
                None
            };
            match kaby {
                Some(kaby) => skylake_as_kaby(s, kaby, matches!(kaby, 0x5926 | 0x5927)),
                None if has("510") || has("580") => {
                    if has("510") {
                        s.notes.insert(0, NOTE_GT1.to_string());
                    }
                    s.notes.push(NOTE_SKL_ROOT_PATCH.to_string());
                }
                None => s.notes.push(
                    "The exact Skylake model is unknown: enter its PCI device id to get the Kaby Lake spoof for \
                     macOS 13 and newer."
                        .to_string(),
                ),
            }
        }
        IntelKabyLake if has("610") => {
            s.requirement = GpuRequirement::DeviceIdSpoof(le(0x5912));
            s.notes.insert(0, NOTE_GT1.to_string());
        }
        IntelKabyLake if has("p630") => s.requirement = GpuRequirement::DeviceIdSpoof(le(0x591B)),
        IntelKabyLake if has("uhd") && has("620") => {
            s.requirement = GpuRequirement::DeviceIdSpoof(le(0x5916));
            s.notes.push(
                "UHD 620 exists on Kaby Lake-R (device-id 0x5916), Whiskey Lake and Comet Lake (0x3E9B); the PCI \
                 device id decides."
                    .to_string(),
            );
        }
        IntelKabyLake if has("617") || (has("uhd") && has("615")) => s.min_native = Some(Mojave),
        IntelCoffeeLake if has("610") => {
            s.requirement = GpuRequirement::DeviceIdSpoof(le(0x3E92));
            s.notes.insert(0, NOTE_GT1.to_string());
        }
        IntelCoffeeLake if has("620") => s.requirement = GpuRequirement::DeviceIdSpoof(le(0x3E9B)),
        IntelCoffeeLake if has("p630") => s.requirement = GpuRequirement::DeviceIdSpoof(le(0x3E92)),
        IntelCometLake if has("610") => {
            s.requirement = GpuRequirement::DeviceIdSpoof(le(0x9BC8));
            s.notes.insert(0, NOTE_GT1.to_string());
        }
        IntelCometLake if has("620") => s.requirement = GpuRequirement::DeviceIdSpoof(le(0x3E9B)),
        IntelCometLake if has("p630") => s.requirement = GpuRequirement::DeviceIdSpoof(le(0x9BC5)),
        AmdGcn1 if names::is_oland(&n) => {
            make_unsupported(s, ids::OLAND);
            s.notes.push(NOTE_DISABLE.to_string());
        }
        AmdGcn1 if names::is_cape_verde(&n) => {
            s.boot_args.push("radpg=15");
            s.notes
                .push("Cape Verde cards need radpg=15 to initialise.".to_string());
        }
        IntelIceLake if has("g1") => make_unsupported(s, "Ice Lake G1 (UHD Graphics) has no macOS driver."),
        NvidiaKepler | NvidiaFermi if has("730") || has("630") || has("640") => s.notes.push(
            "GT 630/640/730 were sold as both Fermi and Kepler; only the PCI device id tells them apart (GT 730 \
             GK208 0x1287 is Kepler, GF108 0x0F02 is Fermi)."
                .to_string(),
        ),
        _ => {}
    }
}

/// Navi 21/23 models Apple never shipped, by name: Some(WEG spoof alternative).
fn nootrx_name_variant(gpu: &ProfileGpu) -> Option<Option<u16>> {
    let n = names::normalize(&gpu.name);
    match gpu.family {
        AmdNavi21 if n.contains("6950") || n.contains("xtxh") => Some(Some(0x73BF)),
        AmdNavi23 if n.contains("6650") || n.contains("6700s") || n.contains("6800s") => {
            Some(Some(0x73FF))
        }
        AmdNavi23 if n.contains("w6600m") => Some(Some(0x73E3)),
        _ => None,
    }
}
