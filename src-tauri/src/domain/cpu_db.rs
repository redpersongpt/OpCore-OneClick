//! CPU identification: CPUID family/model/stepping first, brand-string parsing
//! as fallback. Covers every Intel generation from Penryn to Arrow/Lunar Lake,
//! Xeon E3/E5/E7/W/Scalable, Atom-class, and AMD K10 through Zen 5.
//!
//! CPUID decides the platform. The brand string refines it where one model
//! number spans several generations (0x8E Kaby Lake-R / Whiskey / Amber /
//! Comet Lake-U, 0x9E Kaby / Coffee Lake, Clarkdale vs Arrandale), tells
//! desktop from mobile parts and flags Pentium/Celeron SKUs without AVX2.
//! Without CPUID (manual entry, partial scans) the brand string alone is used.

use once_cell::sync::Lazy;
use regex::Regex;

use super::model::{CpuPlatform, CpuVendor, MacOsVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuIdentity {
    pub vendor: CpuVendor,
    pub platform: CpuPlatform,
    /// Display codename ("Coffee Lake-S", "Comet Lake-U", "Vermeer").
    pub codename: String,
    pub is_mobile: bool,
    pub is_hybrid: bool,
    /// Expected feature support for this platform when CPUID flags are unknown.
    pub has_avx2: bool,
}

/// Identify a CPU. `vendor` is the CPUID vendor string or a friendly name
/// ("GenuineIntel", "Intel", "AuthenticAMD", "AMD"). `family`/`model` are
/// CPUID *display* values (extended family/model already folded in).
pub fn identify(
    name: &str,
    vendor: &str,
    family: Option<u32>,
    model: Option<u32>,
    stepping: Option<u32>,
) -> CpuIdentity {
    let norm = normalize(name);
    let vendor_kind = match resolve_vendor(vendor) {
        CpuVendor::Unknown => brand_vendor(&norm),
        v => v,
    };
    // Rosetta 2 reports GenuineIntel with a "VirtualApple" brand string.
    let rosetta = norm.contains("virtualapple");
    if vendor_kind == CpuVendor::Apple || norm.starts_with("apple") || rosetta {
        let trimmed = name.trim();
        return CpuIdentity {
            vendor: CpuVendor::Apple,
            platform: CpuPlatform::AppleSilicon,
            codename: if trimmed.is_empty() || rosetta {
                "Apple silicon".to_string()
            } else {
                trimmed.to_string()
            },
            is_mobile: false,
            is_hybrid: false,
            has_avx2: false,
        };
    }
    let brand = match vendor_kind {
        CpuVendor::Intel => parse_intel(&norm),
        CpuVendor::Amd => parse_amd(&norm),
        _ => None,
    };
    let cpuid = match (vendor_kind, family) {
        (CpuVendor::Intel, Some(f)) => model.map(|m| intel_cpuid(f, m, stepping)),
        (CpuVendor::Amd, Some(f)) => Some(amd_cpuid(f, model)),
        _ => None,
    };
    merge(vendor_kind, name, brand, cpuid)
}

/// True when `target` lies on the CryptexFixup path of `identity` (a supported
/// CPU without AVX2 running macOS 13+), i.e. the build must add CryptexFixup.
/// Penryn never qualifies: its path ends at Monterey. See
/// [`ceiling_workaround_for`] for the caveats.
pub fn needs_cryptexfixup(identity: &CpuIdentity, target: MacOsVersion) -> bool {
    ceiling_workaround_for(identity).is_some_and(|w| {
        w.kext == Some(CRYPTEXFIXUP_KEXT)
            && target >= w.from
            && !matches!(w.max_macos, Some(max) if target > max)
    })
}

/// In a VM the guest sees only the CPU model the hypervisor exposes. A model
/// that is not usable on bare metal (for iGPU or chipset reasons) still runs
/// macOS there, except AMD K10, which lacks SSSE3/SSE4.1.
pub fn usable_as_vm_guest(platform: CpuPlatform) -> bool {
    !matches!(platform, CpuPlatform::AmdK10 | CpuPlatform::AppleSilicon)
}

/// True when a Penryn-class CPU (SSE4.1 without SSE4.2) runs `target`
/// (Mojave+), i.e. the build must add telemetrap.kext.
pub fn needs_telemetrap(identity: &CpuIdentity, target: MacOsVersion) -> bool {
    identity.platform == CpuPlatform::Penryn && target >= MacOsVersion::Mojave
}

#[derive(Debug, Clone, Copy)]
pub struct PlatformInfo {
    pub platform: CpuPlatform,
    pub vendor: CpuVendor,
    pub label: &'static str,
    /// Platform can run macOS through OpenCore at all.
    pub supported: bool,
    /// HEDT / workstation platform (X58/X79/X99/X299, Threadripper).
    pub hedt: bool,
    pub has_avx2: bool,
    /// Newest macOS the CPU itself can run (None = Tahoe / no CPU-imposed cap).
    /// CPU caps are independent of the GPU; e.g. pre-AVX2 → Monterey unless CryptexFixup.
    /// This is the native ceiling; [`ceiling_workaround`] describes the kext
    /// path beyond it.
    pub max_macos: Option<MacOsVersion>,
    /// Oldest macOS that recognises the platform (e.g. Comet Lake → Catalina 10.15.4).
    pub min_macos: Option<MacOsVersion>,
    pub notes: &'static [&'static str],
}

/// A path past a platform's native `max_macos`. `max_macos` stays the native
/// ceiling because these paths lose features: the planner offers them only as
/// an explicit, warned opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeilingWorkaround {
    /// Kext to inject ("CryptexFixup.kext"). None when the CPU has the
    /// instructions and the releases are merely untested (AMD Excavator).
    pub kext: Option<&'static str>,
    /// First release past the native ceiling.
    pub from: MacOsVersion,
    /// Newest release reachable this way (None = Tahoe).
    pub max_macos: Option<MacOsVersion>,
    pub caveat: &'static str,
}

const CRYPTEX_INTEL_CAVEAT: &str = "No AVX2: macOS 13+ installs only with CryptexFixup. \
     Delta updates are unavailable, Polaris/Vega need a non-AVX2 OCLP root patch and KDK for acceleration; Navi is not covered and \
     AppleIntelCPUPowerManagement has to be re-injected.";
const CRYPTEX_AMD_CAVEAT: &str = "No AVX2: macOS 13+ installs only with CryptexFixup. \
     Delta updates are unavailable and Polaris/Vega need a non-AVX2 OCLP root patch and KDK for acceleration; Navi is not covered; \
     untested on AMD.";
// Haswell to Comet Lake Pentium/Celeron: XCPM-era parts, so no CPU power
// management kext to restore.
const CRYPTEX_LOW_END_CAVEAT: &str = "No AVX2: macOS 13+ installs only with CryptexFixup. \
     Delta updates are unavailable and Polaris/Vega need a non-AVX2 OCLP root patch and KDK for acceleration; Navi is not covered.";
const EXCAVATOR_CAVEAT: &str = "Excavator has AVX2, so macOS 13+ needs no extra kext, \
     but no guide covers family 15h past Monterey: experimental.";
const CRYPTEXFIXUP_KEXT: &str = "CryptexFixup.kext";

const fn cryptexfixup(caveat: &'static str) -> CeilingWorkaround {
    CeilingWorkaround {
        kext: Some(CRYPTEXFIXUP_KEXT),
        from: MacOsVersion::Ventura,
        max_macos: None,
        caveat,
    }
}

/// The platform-wide path past its native ceiling, if one exists (manual
/// platform editor). Prefer [`ceiling_workaround_for`] for a detected CPU.
pub fn ceiling_workaround(platform: CpuPlatform) -> Option<CeilingWorkaround> {
    use CpuPlatform as P;
    match platform {
        // Dortania's Penryn page: MacPro6,1 + telemetrap.kext from Mojave on;
        // MacPro6,1 and the missing AVX2 both stop at Monterey.
        P::Penryn => Some(CeilingWorkaround {
            kext: Some("telemetrap.kext"),
            from: MacOsVersion::Mojave,
            max_macos: Some(MacOsVersion::Monterey),
            caveat: "No SSE4.2: Mojave to Monterey need telemetrap.kext and the MacPro6,1 \
                     SMBIOS; Big Sur and Monterey are community-tested only.",
        }),
        P::Lynnfield
        | P::Arrandale
        | P::SandyBridge
        | P::IvyBridge
        | P::NehalemHedt
        | P::SandyBridgeE
        | P::IvyBridgeE => Some(cryptexfixup(CRYPTEX_INTEL_CAVEAT)),
        P::AmdBulldozer | P::AmdJaguar => Some(cryptexfixup(CRYPTEX_AMD_CAVEAT)),
        _ => None,
    }
}

/// The path past the native ceiling of one detected CPU: the platform path,
/// CryptexFixup for Haswell-to-Comet Lake Pentium/Celeron parts without AVX2,
/// and no kext for AVX2-capable Excavator APUs.
pub fn ceiling_workaround_for(identity: &CpuIdentity) -> Option<CeilingWorkaround> {
    let info = platform_info(identity.platform);
    if !info.supported {
        return None;
    }
    if identity.platform == CpuPlatform::AmdBulldozer && identity.has_avx2 {
        return Some(CeilingWorkaround {
            kext: None,
            ..cryptexfixup(EXCAVATOR_CAVEAT)
        });
    }
    match ceiling_workaround(identity.platform) {
        Some(w) => Some(w),
        None if !identity.has_avx2 => Some(cryptexfixup(CRYPTEX_LOW_END_CAVEAT)),
        None => None,
    }
}

/// Newest macOS one detected CPU runs natively: the platform ceiling, or
/// Monterey for parts of an AVX2 platform that lack AVX2 (Pentium/Celeron).
/// Only meaningful when the platform is supported.
pub fn max_macos_for(identity: &CpuIdentity) -> Option<MacOsVersion> {
    match platform_info(identity.platform).max_macos {
        Some(max) => Some(max),
        None if !identity.has_avx2 => Some(MacOsVersion::Monterey),
        None => None,
    }
}

/// Oldest macOS one detected CPU boots: the platform floor, raised to Mojave
/// 10.14.1 for Whiskey Lake and Amber Lake (Dortania CPU support chart).
pub fn min_macos_for(identity: &CpuIdentity) -> Option<MacOsVersion> {
    let min = platform_info(identity.platform).min_macos?;
    let late_8th_gen = identity.codename.starts_with("Whiskey Lake")
        || identity.codename.starts_with("Amber Lake");
    Some(if late_8th_gen {
        min.max(MacOsVersion::Mojave)
    } else {
        min
    })
}

/// True for HEDT/workstation CPUs: the Intel HEDT platforms plus Threadripper
/// (which shares its AMD platform with the desktop Ryzen parts).
pub fn is_hedt(identity: &CpuIdentity) -> bool {
    const THREADRIPPER: &[&str] = &[
        "Whitehaven",
        "Colfax",
        "Castle Peak",
        "Chagall",
        "Storm Peak",
        "Shimada Peak",
    ];
    platform_info(identity.platform).hedt || THREADRIPPER.contains(&identity.codename.as_str())
}

const NO_AVX2_LOW_END: &str =
    "Pentium and Celeron models lack AVX2 and stop at Monterey without CryptexFixup.";
const CRYPTEX_NOTE: &str =
    "Ventura and newer need AVX2: possible only with CryptexFixup, with caveats.";
const XE_LAPTOP_NOTE: &str = "Laptops are not usable: the Xe iGPU has no macOS driver.";
const AMD_HV_NOTE: &str = "Hypervisor-based apps (VMware, Parallels, Docker) do not work on AMD.";

fn platform_vendor(platform: CpuPlatform) -> CpuVendor {
    use CpuPlatform as P;
    match platform {
        P::AmdK10
        | P::AmdBulldozer
        | P::AmdJaguar
        | P::AmdZen
        | P::AmdZen2
        | P::AmdZen3
        | P::AmdZen4
        | P::AmdZen5 => CpuVendor::Amd,
        P::AppleSilicon => CpuVendor::Apple,
        P::Unknown => CpuVendor::Unknown,
        _ => CpuVendor::Intel,
    }
}

pub fn platform_info(platform: CpuPlatform) -> PlatformInfo {
    use CpuPlatform as P;
    use MacOsVersion::{Catalina, HighSierra, Monterey};
    // Defaults: a supported AVX2 platform with no CPU-imposed ceiling.
    let base = PlatformInfo {
        platform,
        vendor: platform_vendor(platform),
        label: "",
        supported: true,
        hedt: false,
        has_avx2: true,
        max_macos: None,
        min_macos: Some(HighSierra),
        notes: &[],
    };
    let unsupported = PlatformInfo {
        supported: false,
        min_macos: None,
        ..base
    };
    let pre_avx2 = PlatformInfo {
        has_avx2: false,
        max_macos: Some(Monterey),
        ..base
    };
    match platform {
        P::Penryn => PlatformInfo {
            label: "Penryn (Core 2)",
            has_avx2: false,
            max_macos: Some(HighSierra),
            notes: &[
                "Officially supported up to High Sierra 10.13.6.",
                "No SSE4.2: Mojave to Monterey need telemetrap.kext and the MacPro6,1 SMBIOS.",
                "Most boards are legacy BIOS only; OpenCore boots through DuetPkg.",
            ],
            ..base
        },
        P::Lynnfield => PlatformInfo {
            label: "Lynnfield / Clarkdale (1st gen Core desktop)",
            notes: &[
                "Supported up to Monterey.",
                "Desktop Iron Lake graphics never had a macOS driver: a supported dGPU is required.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::Arrandale => PlatformInfo {
            label: "Arrandale / Clarksfield (1st gen Core mobile)",
            notes: &[
                "Supported up to Monterey; the iGPU only up to High Sierra and only on LVDS panels.",
                "Clarksfield (i7-7xxQM/8xxQM/9xxXM) has no iGPU.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::SandyBridge => PlatformInfo {
            label: "Sandy Bridge (2nd gen Core)",
            notes: &[
                "Supported up to Monterey; the HD 3000 iGPU only up to High Sierra.",
                "CPU power management needs an SSDT-PM generated after install.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::IvyBridge => PlatformInfo {
            label: "Ivy Bridge (3rd gen Core)",
            notes: &[
                "Supported up to Monterey; the HD 4000 iGPU only up to Big Sur.",
                "CPU power management needs an SSDT-PM generated after install.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::Haswell => PlatformInfo {
            label: "Haswell (4th gen Core)",
            notes: &[
                "Supported through Tahoe with a supported dGPU; the iGPU only up to Monterey.",
                NO_AVX2_LOW_END,
            ],
            ..base
        },
        P::Broadwell => PlatformInfo {
            label: "Broadwell (5th gen Core)",
            notes: &[
                "Supported through Tahoe with a supported dGPU; the iGPU only up to Monterey.",
                NO_AVX2_LOW_END,
            ],
            ..base
        },
        P::Skylake => PlatformInfo {
            label: "Skylake (6th gen Core)",
            notes: &[
                "Supported through Tahoe.",
                "Ventura and newer need the iGPU spoofed as Kaby Lake.",
                NO_AVX2_LOW_END,
            ],
            ..base
        },
        P::KabyLake => PlatformInfo {
            label: "Kaby Lake / Kaby Lake-R / Amber Lake (7th gen Core)",
            notes: &[
                "Supported through Tahoe; Amber Lake needs Mojave 10.14.1 or newer.",
                "Kaby Lake-R UHD 620 needs a device-id spoof; HD 610 has no driver.",
                NO_AVX2_LOW_END,
            ],
            ..base
        },
        P::CoffeeLake => PlatformInfo {
            label: "Coffee Lake / Whiskey Lake (8th/9th gen Core)",
            notes: &[
                "Supported through Tahoe; UHD 630 graphics and Whiskey Lake need Mojave or newer.",
                "300-series boards other than Z370 need SSDT-PMC for NVRAM.",
                NO_AVX2_LOW_END,
            ],
            ..base
        },
        P::CometLake => PlatformInfo {
            label: "Comet Lake (10th gen Core)",
            min_macos: Some(Catalina),
            notes: &[
                "Supported from Catalina 10.15.4 (10.15.5 recommended) through Tahoe.",
                "Comet Lake-U62 (model 0xA6) needs a CPUID spoof.",
                NO_AVX2_LOW_END,
            ],
            ..base
        },
        P::IceLake => PlatformInfo {
            label: "Ice Lake (10th gen Core mobile)",
            min_macos: Some(Catalina),
            notes: &[
                "Laptops with Iris Plus G4/G7 graphics work from Catalina 10.15.4 through Tahoe.",
                "UHD Graphics (G1) models have no supported iGPU; the iGPU driver has no HDMI.",
            ],
            ..base
        },
        P::RocketLake => PlatformInfo {
            label: "Rocket Lake (11th gen Core desktop)",
            min_macos: Some(Catalina),
            notes: &[
                "Needs a CPUID spoof to Comet Lake and a supported AMD dGPU (UHD 750 has no driver).",
                "Not covered by the Dortania guide; community configurations exist.",
            ],
            ..base
        },
        P::TigerLake => PlatformInfo {
            label: "Tiger Lake (11th gen Core mobile)",
            min_macos: Some(Catalina),
            notes: &[
                "Only systems whose display is driven by a supported AMD dGPU can work.",
                "Needs a CPUID spoof to Comet Lake.",
                XE_LAPTOP_NOTE,
            ],
            ..base
        },
        P::AlderLake => PlatformInfo {
            label: "Alder Lake (12th gen Core)",
            min_macos: Some(Catalina),
            notes: &[
                "Needs a CPUID spoof to Comet Lake, ProvideCurrentCpuInfo and a supported AMD dGPU.",
                "CpuTopologyRebuild.kext improves P/E-core scheduling.",
                XE_LAPTOP_NOTE,
            ],
            ..base
        },
        P::RaptorLake => PlatformInfo {
            label: "Raptor Lake (13th/14th gen Core)",
            min_macos: Some(Catalina),
            notes: &[
                "Needs a CPUID spoof to Comet Lake, ProvideCurrentCpuInfo and a supported AMD dGPU.",
                "CpuTopologyRebuild.kext improves P/E-core scheduling.",
                XE_LAPTOP_NOTE,
            ],
            ..base
        },
        P::MeteorLake => PlatformInfo {
            label: "Meteor Lake (Core Ultra 100)",
            notes: &[
                "Laptop and mini-PC platform whose Arc iGPU has no macOS driver.",
                "No working macOS configuration is known.",
            ],
            ..unsupported
        },
        P::ArrowLake => PlatformInfo {
            label: "Arrow Lake (Core Ultra 200)",
            min_macos: Some(Catalina),
            notes: &[
                "Experimental: desktops are reported on Sequoia with Alder Lake-style settings \
                 and an AMD dGPU; older releases are untested.",
                "Tahoe installs are hit-and-miss and need AppleMCEReporterDisabler.kext.",
                XE_LAPTOP_NOTE,
            ],
            ..base
        },
        P::LunarLake => PlatformInfo {
            label: "Lunar Lake / Panther Lake (Core Ultra 200V/300)",
            notes: &["Laptop-only platform whose Xe2/Xe3 iGPU has no macOS driver; not usable."],
            ..unsupported
        },
        P::NehalemHedt => PlatformInfo {
            label: "Nehalem / Westmere HEDT (X58, Xeon 55xx/56xx)",
            hedt: true,
            notes: &[
                "Supported up to Monterey with a supported dGPU.",
                "Dual-socket systems need AppleMCEReporterDisabler on Catalina and newer.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::SandyBridgeE => PlatformInfo {
            label: "Sandy Bridge-E (X79, Xeon E5 v1)",
            hedt: true,
            notes: &[
                "Supported up to Monterey with a supported dGPU.",
                "Most X79 boards need SSDT-UNC.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::IvyBridgeE => PlatformInfo {
            label: "Ivy Bridge-E (X79, Xeon E5 v2)",
            hedt: true,
            notes: &[
                "Supported up to Monterey with a supported dGPU.",
                "Most X79 boards need SSDT-UNC.",
                CRYPTEX_NOTE,
            ],
            ..pre_avx2
        },
        P::HaswellE => PlatformInfo {
            label: "Haswell-E (X99, Xeon E5 v3)",
            hedt: true,
            notes: &[
                "Supported through Tahoe with a supported dGPU.",
                "Needs a CPUID spoof to Haswell for XCPM, SSDT-UNC and SSDT-RTC0-RANGE.",
            ],
            ..base
        },
        P::BroadwellE => PlatformInfo {
            label: "Broadwell-E (X99, Xeon E5 v4)",
            hedt: true,
            notes: &[
                "Supported through Tahoe with a supported dGPU.",
                "Needs a CPUID spoof to Broadwell for XCPM, SSDT-UNC and SSDT-RTC0-RANGE.",
            ],
            ..base
        },
        P::SkylakeX => PlatformInfo {
            label: "Skylake-X / W / SP (X299, C422, Xeon W-21xx)",
            hedt: true,
            notes: &[
                "Supported through Tahoe with a supported dGPU; native iMac Pro CPU family.",
                "Tahoe requires the MacPro7,1 SMBIOS.",
            ],
            ..base
        },
        P::CascadeLakeX => PlatformInfo {
            label: "Cascade Lake-X / W / SP (X299, Xeon W-22xx/32xx)",
            hedt: true,
            notes: &["Supported through Tahoe with a supported dGPU; native Mac Pro 2019 CPU family."],
            ..base
        },
        P::XeonModernUnsupported => PlatformInfo {
            label: "Ice Lake-SP and newer Xeon",
            hedt: true,
            notes: &[
                "Ice Lake-SP, Sapphire/Emerald/Granite Rapids and Xeon 6 have no working configuration.",
            ],
            ..unsupported
        },
        P::IntelAtom => PlatformInfo {
            label: "Atom-class (Celeron/Pentium N/J, N100)",
            has_avx2: false,
            notes: &[
                "Atom, Celeron/Pentium N/J/Silver and N100-class CPUs are not supported by macOS.",
                "Their iGPUs have no macOS driver.",
            ],
            ..unsupported
        },
        P::AmdK10 => PlatformInfo {
            label: "AMD K10 and older (Phenom, Athlon II)",
            has_avx2: false,
            notes: &[
                "Family 10h-14h CPUs (Phenom, Athlon II, Llano, Bobcat) lack AMD kernel patches.",
            ],
            ..unsupported
        },
        P::AmdBulldozer => PlatformInfo {
            label: "AMD Bulldozer family (FX, A-series)",
            notes: &[
                "Supported up to Monterey with the AMD kernel patches.",
                "Bulldozer to Steamroller lack AVX2 (CryptexFixup for Ventura+); Excavator has it \
                 but is untested past Monterey.",
                "A-series iGPUs have no macOS driver.",
            ],
            ..pre_avx2
        },
        P::AmdJaguar => PlatformInfo {
            label: "AMD Jaguar / Puma (AM1, E/A-series)",
            notes: &["Supported up to Monterey with the AMD kernel patches.", CRYPTEX_NOTE],
            ..pre_avx2
        },
        P::AmdZen => PlatformInfo {
            label: "AMD Zen / Zen+ (Ryzen 1000/2000, Threadripper 1000/2000)",
            notes: &[
                "Supported with AMD kernel patches and a supported display GPU: Vega APUs use NootedRed; laptops need a supported iGPU or a MUX-wired supported dGPU.",
                "Raven/Picasso Vega iGPUs need NootedRed.",
                AMD_HV_NOTE,
            ],
            ..base
        },
        P::AmdZen2 => PlatformInfo {
            label: "AMD Zen 2 (Ryzen 3000/4000, Threadripper 3000)",
            notes: &[
                "Supported with AMD kernel patches and a supported display GPU: Vega APUs use NootedRed; laptops need a supported iGPU or a MUX-wired supported dGPU.",
                "Renoir/Lucienne Vega iGPUs need NootedRed.",
                AMD_HV_NOTE,
            ],
            ..base
        },
        P::AmdZen3 => PlatformInfo {
            label: "AMD Zen 3 (Ryzen 5000/6000, Threadripper PRO 5000)",
            notes: &[
                "Supported with AMD kernel patches and a supported display GPU: Vega APUs use NootedRed; laptops need a supported iGPU or a MUX-wired supported dGPU.",
                "Cezanne/Barcelo Vega iGPUs need NootedRed; Ryzen 6000 graphics have no driver.",
                AMD_HV_NOTE,
            ],
            ..base
        },
        P::AmdZen4 => PlatformInfo {
            label: "AMD Zen 4 (Ryzen 7000/8000, Threadripper 7000)",
            min_macos: Some(Monterey),
            notes: &[
                "Supported from Monterey through Tahoe with a supported dGPU (RDNA iGPU has no driver).",
                "AM5 needs the IOPCIFamily 10-bit tag patch, DevirtualiseMmio and an MMIO whitelist.",
                AMD_HV_NOTE,
            ],
            ..base
        },
        P::AmdZen5 => PlatformInfo {
            label: "AMD Zen 5 (Ryzen 9000, Ryzen AI 300)",
            min_macos: Some(Monterey),
            notes: &[
                "Experimental: confirmed on Sequoia and Tahoe with supported dGPUs (OpenCore 1.0.3+).",
                "Ryzen AI laptops are not usable: the RDNA 3.5 iGPU has no driver.",
            ],
            ..base
        },
        P::AppleSilicon => PlatformInfo {
            label: "Apple silicon",
            has_avx2: false,
            notes: &["Apple silicon Macs run macOS natively; OpenCore is not used."],
            ..unsupported
        },
        P::Unknown => PlatformInfo {
            label: "Unknown / unsupported CPU",
            has_avx2: false,
            notes: &[
                "The CPU does not match a platform macOS supports (Core 2 Conroe/Merom lack SSE4.1).",
                "Pick the platform manually if the detection is wrong.",
            ],
            ..unsupported
        },
    }
}

/// All platforms in UI order (for the manual editor).
pub fn all_platforms() -> &'static [CpuPlatform] {
    use CpuPlatform as P;
    &[
        P::Penryn,
        P::Lynnfield,
        P::Arrandale,
        P::SandyBridge,
        P::IvyBridge,
        P::Haswell,
        P::Broadwell,
        P::Skylake,
        P::KabyLake,
        P::CoffeeLake,
        P::CometLake,
        P::IceLake,
        P::RocketLake,
        P::TigerLake,
        P::AlderLake,
        P::RaptorLake,
        P::MeteorLake,
        P::ArrowLake,
        P::LunarLake,
        P::NehalemHedt,
        P::SandyBridgeE,
        P::IvyBridgeE,
        P::HaswellE,
        P::BroadwellE,
        P::SkylakeX,
        P::CascadeLakeX,
        P::XeonModernUnsupported,
        P::IntelAtom,
        P::AmdK10,
        P::AmdBulldozer,
        P::AmdJaguar,
        P::AmdZen,
        P::AmdZen2,
        P::AmdZen3,
        P::AmdZen4,
        P::AmdZen5,
        P::AppleSilicon,
        P::Unknown,
    ]
}

// ── Merging ─────────────────────────────────────────────────────────────────

/// Platform guess from one source (CPUID or brand string).
#[derive(Debug, Clone, Copy)]
struct Guess {
    platform: CpuPlatform,
    codename: &'static str,
    /// Some when the source tells desktop from mobile.
    mobile: Option<bool>,
    /// Some(false) for P-core-only parts of hybrid generations.
    hybrid: Option<bool>,
    /// Set when the part differs from the platform's AVX2 default.
    avx2: Option<bool>,
}

impl Guess {
    const fn new(platform: CpuPlatform, codename: &'static str) -> Self {
        Guess {
            platform,
            codename,
            mobile: None,
            hybrid: None,
            avx2: None,
        }
    }
    const fn mobile(mut self, mobile: bool) -> Self {
        self.mobile = Some(mobile);
        self
    }
    const fn hybrid(mut self, hybrid: bool) -> Self {
        self.hybrid = Some(hybrid);
        self
    }
    const fn avx2(mut self, avx2: bool) -> Self {
        self.avx2 = Some(avx2);
        self
    }
}

/// CPUID-derived guess plus what the brand string may still change.
#[derive(Debug, Clone, Copy)]
struct CpuidGuess {
    guess: Guess,
    /// Codename to show when the part turns out to be mobile.
    mobile_codename: Option<&'static str>,
    /// Brand-string platforms that may override this guess (models shared by
    /// several generations).
    refine: &'static [CpuPlatform],
    /// The model number is not in the table (newer or exotic part): any
    /// recognised brand string wins.
    unknown_model: bool,
}

impl CpuidGuess {
    const fn refined(mut self, platforms: &'static [CpuPlatform]) -> Self {
        self.refine = platforms;
        self
    }
}

const fn plain(guess: Guess) -> CpuidGuess {
    CpuidGuess {
        guess,
        mobile_codename: None,
        refine: &[],
        unknown_model: false,
    }
}

const fn split(guess: Guess, mobile_codename: &'static str) -> CpuidGuess {
    CpuidGuess {
        mobile_codename: Some(mobile_codename),
        ..plain(guess)
    }
}

const fn unlisted(guess: Guess) -> CpuidGuess {
    CpuidGuess {
        unknown_model: true,
        ..plain(guess)
    }
}

fn default_hybrid(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::AlderLake | P::RaptorLake | P::ArrowLake | P::MeteorLake | P::LunarLake
    )
}

fn combine_hybrid(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        _ => a.or(b),
    }
}

fn merge(
    vendor: CpuVendor,
    name: &str,
    brand: Option<Guess>,
    cpuid: Option<CpuidGuess>,
) -> CpuIdentity {
    let (guess, codename) = match (cpuid, brand) {
        // Model number missing from the table: trust a recognised brand string.
        (Some(c), Some(b)) if c.unknown_model && b.platform != CpuPlatform::Unknown => {
            (b, b.codename)
        }
        // The brand string agrees with CPUID or picks between generations that
        // share a model number: keep its finer codename and segment.
        (Some(c), Some(b)) if b.platform == c.guess.platform || c.refine.contains(&b.platform) => {
            let guess = Guess {
                mobile: b.mobile.or(c.guess.mobile),
                hybrid: combine_hybrid(b.hybrid, c.guess.hybrid),
                avx2: b.avx2.or(c.guess.avx2),
                ..b
            };
            (guess, b.codename)
        }
        (Some(c), brand) => {
            if let Some(b) = brand {
                tracing::debug!(
                    cpu = name,
                    cpuid = ?c.guess.platform,
                    brand = ?b.platform,
                    "brand string disagrees with CPUID; using CPUID"
                );
            }
            let mobile = c.guess.mobile.or(brand.and_then(|b| b.mobile));
            let codename = match (mobile, c.mobile_codename) {
                (Some(true), Some(m)) => m,
                _ => c.guess.codename,
            };
            (Guess { mobile, ..c.guess }, codename)
        }
        (None, Some(b)) => (b, b.codename),
        (None, None) => (Guess::new(CpuPlatform::Unknown, "Unknown"), "Unknown"),
    };
    let platform = guess.platform;
    let info = platform_info(platform);
    let vendor = if vendor == CpuVendor::Unknown {
        info.vendor
    } else {
        vendor
    };
    CpuIdentity {
        vendor,
        platform,
        codename: codename.to_string(),
        is_mobile: guess.mobile.unwrap_or(false),
        is_hybrid: guess.hybrid.unwrap_or_else(|| default_hybrid(platform)),
        has_avx2: !low_end_without_avx2(name, platform) && guess.avx2.unwrap_or(info.has_avx2),
    }
}

/// Haswell to Comet Lake Pentium and Celeron parts have no AVX/AVX2; the
/// Ice Lake / Tiger Lake and newer ones do.
fn low_end_without_avx2(name: &str, platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    let name = name.to_ascii_lowercase();
    (name.contains("pentium") || name.contains("celeron"))
        && matches!(
            platform,
            P::Haswell | P::Broadwell | P::Skylake | P::KabyLake | P::CoffeeLake | P::CometLake
        )
}

// ── CPUID tables ────────────────────────────────────────────────────────────

/// Intel family 6 model table (Linux `intel-family.h`, Hardware-Sniffer
/// `cpu_data.py`, Intel SDM).
fn intel_cpuid(family: u32, model: u32, stepping: Option<u32>) -> CpuidGuess {
    use CpuPlatform as P;
    let g = Guess::new;
    if family == 0xF {
        return plain(g(P::Unknown, "NetBurst (no SSE4.1)"));
    }
    if family != 6 {
        return unlisted(g(P::Unknown, "Unknown Intel"));
    }
    const KBL_CFL_CML: &[CpuPlatform] = &[P::KabyLake, P::CoffeeLake, P::CometLake];
    const KBL_CFL: &[CpuPlatform] = &[P::KabyLake, P::CoffeeLake];
    const ADL_RPL: &[CpuPlatform] = &[P::AlderLake, P::RaptorLake];
    const NEHALEM_CLIENT: &[CpuPlatform] = &[P::Lynnfield, P::Arrandale];
    const XEON_SP: &[CpuPlatform] = &[P::SkylakeX, P::CascadeLakeX, P::XeonModernUnsupported];
    // H0-stepping Alder/Raptor Lake dies have no E-cores (i3-12100, i5-12400, i3-13100).
    let p_core_die = |guess: Guess| {
        if stepping == Some(5) {
            guess.hybrid(false)
        } else {
            guess
        }
    };
    match model {
        0x09 | 0x0D | 0x0E => plain(g(P::Unknown, "Pentium M / Core Duo (32-bit)")),
        0x0F | 0x16 => plain(g(P::Unknown, "Conroe/Merom (no SSE4.1)")),
        0x17 => plain(g(P::Penryn, "Penryn")),
        0x1D => plain(g(P::Penryn, "Dunnington").mobile(false)),
        0x1A => plain(g(P::NehalemHedt, "Bloomfield").mobile(false)),
        0x1E | 0x1F => plain(g(P::Lynnfield, "Lynnfield")).refined(NEHALEM_CLIENT),
        0x2E => plain(g(P::NehalemHedt, "Beckton").mobile(false)),
        0x25 => plain(g(P::Lynnfield, "Clarkdale")).refined(NEHALEM_CLIENT),
        0x2C => plain(g(P::NehalemHedt, "Gulftown").mobile(false)),
        0x2F => plain(g(P::NehalemHedt, "Westmere-EX").mobile(false)),
        0x2A => plain(g(P::SandyBridge, "Sandy Bridge")),
        0x2D => plain(g(P::SandyBridgeE, "Sandy Bridge-E").mobile(false)),
        0x3A => plain(g(P::IvyBridge, "Ivy Bridge")),
        0x3E => plain(g(P::IvyBridgeE, "Ivy Bridge-E").mobile(false)),
        0x3C => split(g(P::Haswell, "Haswell"), "Haswell-H"),
        0x45 => plain(g(P::Haswell, "Haswell-ULT").mobile(true)),
        0x46 => plain(g(P::Haswell, "Crystal Well")),
        0x3F => plain(g(P::HaswellE, "Haswell-E").mobile(false)),
        0x3D => plain(g(P::Broadwell, "Broadwell-U/Y").mobile(true)),
        0x47 => split(g(P::Broadwell, "Broadwell-C"), "Broadwell-H"),
        0x4F => plain(g(P::BroadwellE, "Broadwell-E").mobile(false)),
        0x56 => plain(g(P::BroadwellE, "Broadwell-DE").mobile(false)),
        0x4E => plain(g(P::Skylake, "Skylake-U/Y").mobile(true)),
        0x5E => split(g(P::Skylake, "Skylake-S"), "Skylake-H"),
        // Stepping 0-4 Skylake-SP/X/W, 5-7 Cascade Lake, 10-11 Cooper Lake.
        0x55 => match stepping {
            None => plain(g(P::SkylakeX, "Skylake-X").mobile(false)).refined(XEON_SP),
            Some(0..=4) => plain(g(P::SkylakeX, "Skylake-X").mobile(false)),
            Some(5..=9) => plain(g(P::CascadeLakeX, "Cascade Lake-X").mobile(false)),
            Some(_) => plain(g(P::XeonModernUnsupported, "Cooper Lake").mobile(false)),
        },
        // Stepping 9 Kaby Lake / Amber Lake, 10 Kaby Lake-R / Coffee Lake-U,
        // 11 Whiskey Lake, 12 Whiskey Lake (V0) / Amber Lake / Comet Lake-U.
        // Without a brand string the newest candidate is assumed so the macOS
        // floor is never too low.
        0x8E => {
            let guess = match stepping {
                Some(10) => g(P::KabyLake, "Kaby Lake-R"),
                Some(11) => g(P::CoffeeLake, "Whiskey Lake-U"),
                Some(12) => g(P::CometLake, "Comet Lake-U"),
                _ => g(P::KabyLake, "Kaby Lake-U/Y"),
            };
            plain(guess.mobile(true)).refined(KBL_CFL_CML)
        }
        // Stepping 9 Kaby Lake, 10-13 Coffee Lake (8th/9th gen, Xeon E-21xx/22xx).
        0x9E => {
            let cpuid = match stepping {
                Some(s) if s <= 9 => split(g(P::KabyLake, "Kaby Lake-S"), "Kaby Lake-H"),
                _ => split(g(P::CoffeeLake, "Coffee Lake-S"), "Coffee Lake-H"),
            };
            cpuid.refined(KBL_CFL)
        }
        0x66 => plain(g(P::CoffeeLake, "Cannon Lake-U").mobile(true)),
        0xA5 => split(g(P::CometLake, "Comet Lake-S"), "Comet Lake-H"),
        0xA6 => plain(g(P::CometLake, "Comet Lake-U").mobile(true)),
        0x7D => plain(g(P::IceLake, "Ice Lake-Y").mobile(true)),
        0x7E => plain(g(P::IceLake, "Ice Lake-U").mobile(true)),
        0x6A => plain(g(P::XeonModernUnsupported, "Ice Lake-SP").mobile(false)),
        0x6C => plain(g(P::XeonModernUnsupported, "Ice Lake-D").mobile(false)),
        0x8C => plain(g(P::TigerLake, "Tiger Lake-UP3").mobile(true)),
        0x8D => split(g(P::TigerLake, "Tiger Lake-B"), "Tiger Lake-H"),
        0xA7 => plain(g(P::RocketLake, "Rocket Lake-S").mobile(false)),
        0x97 => {
            split(p_core_die(g(P::AlderLake, "Alder Lake-S")), "Alder Lake-HX").refined(ADL_RPL)
        }
        0x9A => plain(g(P::AlderLake, "Alder Lake-P").mobile(true)).refined(ADL_RPL),
        0xBE => plain(g(P::IntelAtom, "Alder Lake-N").avx2(true)),
        0xB7 => split(g(P::RaptorLake, "Raptor Lake-S"), "Raptor Lake-HX").refined(ADL_RPL),
        0xBF => split(
            p_core_die(g(P::RaptorLake, "Raptor Lake-S")),
            "Raptor Lake-HX",
        )
        .refined(ADL_RPL),
        0xBA => plain(g(P::RaptorLake, "Raptor Lake-P").mobile(true)).refined(ADL_RPL),
        0xD7 => plain(
            g(P::RaptorLake, "Bartlett Lake-S")
                .mobile(false)
                .hybrid(false),
        ),
        0xAA => plain(g(P::MeteorLake, "Meteor Lake").mobile(true)),
        0xAC => plain(g(P::MeteorLake, "Meteor Lake-S").mobile(false)),
        0xC6 => split(g(P::ArrowLake, "Arrow Lake-S"), "Arrow Lake-HX"),
        0xC5 => plain(g(P::ArrowLake, "Arrow Lake-H").mobile(true)),
        0xB5 => plain(g(P::ArrowLake, "Arrow Lake-U").mobile(true)),
        0xBD => plain(g(P::LunarLake, "Lunar Lake").mobile(true)),
        0xCC => plain(g(P::LunarLake, "Panther Lake").mobile(true)),
        0xD5 => plain(g(P::LunarLake, "Wildcat Lake").mobile(true)),
        0x8F => plain(g(P::XeonModernUnsupported, "Sapphire Rapids").mobile(false)),
        0xCF => plain(g(P::XeonModernUnsupported, "Emerald Rapids").mobile(false)),
        0xAD => plain(g(P::XeonModernUnsupported, "Granite Rapids").mobile(false)),
        0xAE => plain(g(P::XeonModernUnsupported, "Granite Rapids-D").mobile(false)),
        0xAF => plain(g(P::XeonModernUnsupported, "Sierra Forest").mobile(false)),
        0xDD => plain(g(P::XeonModernUnsupported, "Clearwater Forest").mobile(false)),
        0x57 => plain(g(P::XeonModernUnsupported, "Knights Landing").mobile(false)),
        0x85 => plain(g(P::XeonModernUnsupported, "Knights Mill").mobile(false)),
        0x8A => plain(g(P::IntelAtom, "Lakefield").mobile(true).hybrid(true)),
        0x1C | 0x26 => plain(g(P::IntelAtom, "Bonnell")),
        0x27 | 0x35 | 0x36 => plain(g(P::IntelAtom, "Saltwell")),
        0x37 => plain(g(P::IntelAtom, "Bay Trail")),
        0x4A | 0x5A | 0x5D => plain(g(P::IntelAtom, "Silvermont")),
        0x4D => plain(g(P::IntelAtom, "Avoton")),
        0x4C => plain(g(P::IntelAtom, "Braswell / Cherry Trail")),
        0x6E | 0x75 => plain(g(P::IntelAtom, "Airmont")),
        0x5C => plain(g(P::IntelAtom, "Apollo Lake")),
        0x5F => plain(g(P::IntelAtom, "Denverton")),
        0x7A => plain(g(P::IntelAtom, "Gemini Lake")),
        0x86 => plain(g(P::IntelAtom, "Snow Ridge")),
        0x96 => plain(g(P::IntelAtom, "Elkhart Lake")),
        0x9C => plain(g(P::IntelAtom, "Jasper Lake")),
        0xB6 => plain(g(P::IntelAtom, "Grand Ridge")),
        _ => unlisted(g(P::Unknown, "Unknown Intel")),
    }
}

/// AMD family/model table (Linux `amd_nb`/`cpu_data`, Hardware-Sniffer).
fn amd_cpuid(family: u32, model: Option<u32>) -> CpuidGuess {
    use CpuPlatform as P;
    let g = Guess::new;
    const ZEN_1_2: &[CpuPlatform] = &[P::AmdZen, P::AmdZen2];
    const ZEN_3_4: &[CpuPlatform] = &[P::AmdZen3, P::AmdZen4];
    match family {
        0..=0x0F => plain(g(P::AmdK10, "K8")),
        0x10 => plain(g(P::AmdK10, "K10")),
        0x11 => plain(g(P::AmdK10, "Griffin")),
        0x12 => plain(g(P::AmdK10, "Llano")),
        0x13 | 0x14 => plain(g(P::AmdK10, "Bobcat")),
        // Models 0x60 and up are Excavator, the only 15h core with AVX2.
        0x15 => plain(match model {
            None => g(P::AmdBulldozer, "Bulldozer family"),
            Some(0x00..=0x01) => g(P::AmdBulldozer, "Zambezi"),
            Some(0x02..=0x0F) => g(P::AmdBulldozer, "Vishera"),
            Some(0x10..=0x2F) => g(P::AmdBulldozer, "Trinity / Richland"),
            Some(0x30..=0x5F) => g(P::AmdBulldozer, "Kaveri / Godavari"),
            Some(0x60..=0x6F) => g(P::AmdBulldozer, "Carrizo / Bristol Ridge").avx2(true),
            Some(_) => g(P::AmdBulldozer, "Stoney Ridge").avx2(true),
        }),
        0x16 => plain(match model {
            Some(m) if m >= 0x30 => g(P::AmdJaguar, "Puma (Beema/Mullins)"),
            _ => g(P::AmdJaguar, "Jaguar (Kabini)"),
        }),
        0x17 => match model {
            None => plain(g(P::AmdZen, "Zen")).refined(ZEN_1_2),
            Some(m) => match m {
                0x00..=0x07 => plain(g(P::AmdZen, "Summit Ridge")),
                0x08..=0x0F => plain(g(P::AmdZen, "Pinnacle Ridge")),
                0x10..=0x17 => plain(g(P::AmdZen, "Raven Ridge")),
                0x18..=0x1F => plain(g(P::AmdZen, "Picasso")),
                0x20..=0x2F => plain(g(P::AmdZen, "Dali").mobile(true)),
                0x30..=0x3F => plain(g(P::AmdZen2, "Castle Peak").mobile(false)),
                0x60..=0x67 | 0x40..=0x4F | 0x84 => plain(g(P::AmdZen2, "Renoir")),
                0x68..=0x6F => plain(g(P::AmdZen2, "Lucienne").mobile(true)),
                0x70..=0x7F => plain(g(P::AmdZen2, "Matisse").mobile(false)),
                0x90..=0x9F => plain(g(P::AmdZen2, "Van Gogh").mobile(true)),
                0xA0..=0xAF => plain(g(P::AmdZen2, "Mendocino").mobile(true)),
                _ => unlisted(g(P::AmdZen2, "Zen 2")).refined(ZEN_1_2),
            },
        },
        0x19 => match model {
            None => plain(g(P::AmdZen3, "Zen 3")).refined(ZEN_3_4),
            Some(m) => match m {
                0x00..=0x07 => plain(g(P::AmdZen3, "Milan").mobile(false)),
                0x08..=0x0F => plain(g(P::AmdZen3, "Chagall").mobile(false)),
                0x10..=0x17 => plain(g(P::AmdZen4, "Genoa").mobile(false)),
                0x18..=0x1F => plain(g(P::AmdZen4, "Storm Peak").mobile(false)),
                0x20..=0x2F => plain(g(P::AmdZen3, "Vermeer").mobile(false)),
                0x40..=0x4F => plain(g(P::AmdZen3, "Rembrandt").mobile(true)),
                0x50..=0x5F => plain(g(P::AmdZen3, "Cezanne")),
                0x60..=0x6F => plain(g(P::AmdZen4, "Raphael")),
                0x70..=0x7B => plain(g(P::AmdZen4, "Phoenix")),
                0x7C..=0x7F => plain(g(P::AmdZen4, "Hawk Point").mobile(true)),
                0xA0..=0xAF => plain(g(P::AmdZen4, "Bergamo").mobile(false)),
                _ => unlisted(g(P::AmdZen3, "Zen 3")).refined(ZEN_3_4),
            },
        },
        0x1A => match model {
            Some(0x00..=0x07 | 0x10..=0x1F) => plain(g(P::AmdZen5, "Turin").mobile(false)),
            Some(0x08..=0x0F) => plain(g(P::AmdZen5, "Shimada Peak").mobile(false)),
            Some(0x20..=0x2F) => plain(g(P::AmdZen5, "Strix Point").mobile(true)),
            Some(0x40..=0x4F) => plain(g(P::AmdZen5, "Granite Ridge").mobile(false)),
            Some(0x60..=0x6F) => plain(g(P::AmdZen5, "Krackan Point").mobile(true)),
            Some(0x70..=0x7F) => plain(g(P::AmdZen5, "Strix Halo").mobile(true)),
            _ => plain(g(P::AmdZen5, "Zen 5")),
        },
        _ => unlisted(g(P::Unknown, "Unknown AMD")),
    }
}

// ── Brand strings ───────────────────────────────────────────────────────────

/// Lowercase, drop trademark marks, the clock suffix and filler words:
/// "Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz" → "intel core i7-8700k".
fn normalize(name: &str) -> String {
    let mut s = name.to_lowercase();
    for mark in ["(r)", "(tm)", "®", "™"] {
        s = s.replace(mark, "");
    }
    if let Some(at) = s.find('@') {
        s.truncate(at);
    }
    s.split_whitespace()
        .filter(|t| !matches!(*t, "cpu" | "processor" | "apu"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn resolve_vendor(vendor: &str) -> CpuVendor {
    let v = vendor.trim().to_lowercase();
    if v.contains("intel") {
        CpuVendor::Intel
    } else if v.contains("amd") || v.contains("advanced micro") {
        CpuVendor::Amd
    } else if v.contains("apple") {
        CpuVendor::Apple
    } else {
        CpuVendor::Unknown
    }
}

static RE_AMD_WORD: Lazy<Regex> = Lazy::new(|| {
    re(concat!(
        r"\b(amd|ryzen|athlon|phenom|opteron|sempron|turion|epyc|threadripper|fx\s*-?\s*\d",
        r"|a(?:4|6|8|9|10|12)\s*-\s*\d{4})"
    ))
});
static RE_INTEL_WORD: Lazy<Regex> = Lazy::new(|| {
    re(concat!(
        r"\b(intel|xeon|pentium|celeron|atom|core\s*(i[3579]|2|m\d?|ultra|[3579]\s)",
        r"|i[3579](?:\s*-\s*|\s+)\d|m[357]-\dy)"
    ))
});

fn brand_vendor(norm: &str) -> CpuVendor {
    if norm.starts_with("apple") {
        CpuVendor::Apple
    } else if RE_AMD_WORD.is_match(norm) {
        CpuVendor::Amd
    } else if RE_INTEL_WORD.is_match(norm) {
        CpuVendor::Intel
    } else {
        CpuVendor::Unknown
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex")
}

fn num(caps: &regex::Captures<'_>, i: usize) -> Option<u32> {
    caps.get(i)?.as_str().parse().ok()
}

fn text<'a>(caps: &regex::Captures<'a>, i: usize) -> &'a str {
    caps.get(i).map_or("", |m| m.as_str())
}

// Intel patterns
static RE_LAKEFIELD: Lazy<Regex> = Lazy::new(|| re(r"\bi[35]\s*-\s*l1\dg7\b"));
static RE_I3_N: Lazy<Regex> = Lazy::new(|| re(r"\bi3\s*-\s*n3\d{2}\b"));
static RE_INTEL_N: Lazy<Regex> =
    Lazy::new(|| re(r"(?:^|\bintel\s+|\bcore\s+[357]\s+)n(\d{2,3})\b"));
static RE_PC_NJ: Lazy<Regex> =
    Lazy::new(|| re(r"\b(?:pentium|celeron)\s+(?:silver\s+|gold\s+)?([nj])(\d)(\d)\d{2}\b"));
static RE_CORE_ULTRA: Lazy<Regex> =
    Lazy::new(|| re(r"\bcore\s+ultra\s+x?[3579]\s*-?\s*(\d)(\d{2})([a-z]*)\b"));
static RE_CORE_SERIES: Lazy<Regex> = Lazy::new(|| re(r"\bcore\s+([3579])\s+(\d)(\d{2})([a-z]*)\b"));
static RE_CORE_I: Lazy<Regex> = Lazy::new(|| {
    re(r"\bi([3579])(?:\s*-\s*|\s+)?(?:([mqxul])\s+)?(\d{3,5})([a-z]{0,3}\d?[a-z]?)\b")
});
static RE_CORE_M: Lazy<Regex> = Lazy::new(|| re(r"\bcore\s+m[357]?\s*-?\s*(\d)y\d{2}"));
static RE_M_DASH: Lazy<Regex> = Lazy::new(|| re(r"\b[mi][357]\s*-\s*(\d)y\d{2}\b"));
static RE_M_AMBER: Lazy<Regex> = Lazy::new(|| re(r"\bm3\s*-\s*8\d{3}y\b"));
static RE_CORE2: Lazy<Regex> = Lazy::new(|| {
    re(r"\bcore\s*2\s*(?:duo|quad|extreme|solo)?\s*(?:mobile\s+)?([a-z]{1,2})\s*(\d)(\d{3})\b")
});
static RE_XEON_SCALABLE: Lazy<Regex> =
    Lazy::new(|| re(r"\b(bronze|silver|gold|platinum)\s+(\d)(\d)(\d{2})([a-z]*)\b"));
static RE_XEON_W_NEW: Lazy<Regex> = Lazy::new(|| re(r"\bw[3579]\s*-\s*\d{4}"));
static RE_XEON_6: Lazy<Regex> = Lazy::new(|| re(r"\bxeon\s+(?:6\s+)?6\d{3}[a-z]*\b"));
static RE_XEON_E3: Lazy<Regex> =
    Lazy::new(|| re(r"\be3\s*-?\s*1(\d)(\d{2})([a-z]*)(?:\s*v\s*(\d))?"));
static RE_XEON_E57: Lazy<Regex> =
    Lazy::new(|| re(r"\be([57])\s*-?\s*(\d{4})([a-z]*)(?:\s*v\s*(\d))?"));
static RE_XEON_E2: Lazy<Regex> = Lazy::new(|| re(r"\be\s*-\s*2(\d)(\d{2})([a-z]*)\b"));
static RE_XEON_W: Lazy<Regex> = Lazy::new(|| re(r"\bw\s*-\s*(\d{4,5})([a-z]*)\b"));
static RE_XEON_D: Lazy<Regex> = Lazy::new(|| re(r"\bd\s*-\s*(\d)(\d)(\d{2})[a-z]*\b"));
static RE_XEON_OLD: Lazy<Regex> = Lazy::new(|| re(r"\b([xwel])(\d)(\d)(\d{2})\b"));
static RE_PC_LEGACY: Lazy<Regex> =
    Lazy::new(|| re(r"\b(?:pentium\s+(?:4|d|m|iii|ii|pro)|celeron\s+(?:d|m))\b"));
static RE_PC_G: Lazy<Regex> = Lazy::new(|| re(r"\bg(\d{3,4})[a-z]*\b"));
static RE_PC_P: Lazy<Regex> = Lazy::new(|| re(r"\bp(\d{4})\b"));
static RE_PC_B: Lazy<Regex> = Lazy::new(|| re(r"\bb(\d{3})\b"));
static RE_PC_ET: Lazy<Regex> = Lazy::new(|| re(r"\b([et])(\d)\d{3}\b"));
static RE_PC_U: Lazy<Regex> = Lazy::new(|| re(r"\b(s?u)(\d)\d{3}\b"));
static RE_PC_4DIGIT: Lazy<Regex> = Lazy::new(|| re(r"\b(\d{4})([a-z]{0,2})\b"));
static RE_PC_3DIGIT: Lazy<Regex> = Lazy::new(|| re(r"\b(\d)(\d)(\d)\b"));
static RE_YONAH: Lazy<Regex> =
    Lazy::new(|| re(r"\b(?:core\s+(?:duo|solo)|genuine\s+intel\s+[tlu][12]\d{3})\b"));
static RE_INTEL_U300: Lazy<Regex> = Lazy::new(|| re(r"\bintel\s+u300e?\b"));

fn parse_intel(s: &str) -> Option<Guess> {
    if let Some(g) = intel_atom_class(s) {
        return Some(g);
    }
    if let Some(c) = RE_CORE_ULTRA.captures(s) {
        return core_ultra(num(&c, 1)?, text(&c, 3));
    }
    if let Some(c) = RE_CORE_SERIES.captures(s) {
        return core_series(num(&c, 2)?, text(&c, 4));
    }
    if let Some(c) = RE_CORE_I.captures(s) {
        let tier = num(&c, 1)?;
        let prefix = c.get(2).and_then(|m| m.as_str().chars().next());
        let digits = text(&c, 3);
        let n: u32 = digits.parse().ok()?;
        let suffix = text(&c, 4);
        return match digits.len() {
            3 => core_first_gen(tier, prefix, n, suffix),
            4 if digits.starts_with('1') => core_mobile_10plus(n / 100, suffix),
            4 => core_classic(n / 1000, n, suffix),
            _ => core_modern(tier, n, suffix),
        };
    }
    if RE_M_AMBER.is_match(s) {
        return Some(Guess::new(CpuPlatform::KabyLake, "Amber Lake-Y").mobile(true));
    }
    if let Some(c) = RE_CORE_M.captures(s).or_else(|| RE_M_DASH.captures(s)) {
        return core_m(num(&c, 1)?);
    }
    if let Some(c) = RE_CORE2.captures(s) {
        return core2(text(&c, 1), num(&c, 2)?);
    }
    if s.contains("xeon") || RE_XEON_E57.is_match(s) || RE_XEON_E3.is_match(s) {
        if let Some(g) = xeon(s) {
            return Some(g);
        }
    }
    if s.contains("pentium") || s.contains("celeron") {
        return pentium_celeron(s);
    }
    if RE_INTEL_U300.is_match(s) {
        return Some(Guess::new(CpuPlatform::RaptorLake, "Raptor Lake-U").mobile(true));
    }
    if RE_YONAH.is_match(s) {
        return Some(Guess::new(CpuPlatform::Unknown, "Yonah (32-bit)").mobile(true));
    }
    None
}

fn intel_atom_class(s: &str) -> Option<Guess> {
    use CpuPlatform::IntelAtom;
    if s.contains("atom") {
        return Some(Guess::new(IntelAtom, "Atom"));
    }
    if RE_LAKEFIELD.is_match(s) {
        return Some(Guess::new(IntelAtom, "Lakefield").mobile(true).hybrid(true));
    }
    if RE_I3_N.is_match(s) {
        return Some(Guess::new(IntelAtom, "Alder Lake-N").avx2(true));
    }
    if let Some(c) = RE_INTEL_N.captures(s) {
        let codename = match num(&c, 1) {
            Some(150 | 250 | 355) => "Twin Lake",
            _ => "Alder Lake-N",
        };
        return Some(Guess::new(IntelAtom, codename).avx2(true));
    }
    if let Some(c) = RE_PC_NJ.captures(s) {
        let codename = match (num(&c, 2), num(&c, 3)) {
            (Some(1 | 2), _) => "Bay Trail",
            (Some(3), Some(3 | 4)) | (Some(4), Some(2)) => "Apollo Lake",
            (Some(3), Some(5)) => "Bay Trail",
            (Some(3), _) => "Braswell",
            (Some(4), Some(0 | 1)) | (Some(5), Some(0)) => "Gemini Lake",
            (Some(4..=6), _) => "Jasper Lake",
            _ => "Atom-class",
        };
        // N parts are laptop chips, J parts desktop/embedded.
        return Some(Guess::new(IntelAtom, codename).mobile(text(&c, 1) == "n"));
    }
    if s.contains("pentium silver") {
        return Some(Guess::new(IntelAtom, "Gemini Lake"));
    }
    None
}

fn core_ultra(series: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let g = match series {
        1 if suffix.starts_with('u') => Guess::new(P::MeteorLake, "Meteor Lake-U").mobile(true),
        1 => Guess::new(P::MeteorLake, "Meteor Lake-H").mobile(true),
        2 if suffix.starts_with('v') => Guess::new(P::LunarLake, "Lunar Lake").mobile(true),
        2 if suffix.starts_with("hx") => Guess::new(P::ArrowLake, "Arrow Lake-HX").mobile(true),
        2 if suffix.starts_with('h') => Guess::new(P::ArrowLake, "Arrow Lake-H").mobile(true),
        2 if suffix.starts_with('u') => Guess::new(P::ArrowLake, "Arrow Lake-U").mobile(true),
        2 => Guess::new(P::ArrowLake, "Arrow Lake-S").mobile(false),
        3 => Guess::new(P::LunarLake, "Panther Lake").mobile(true),
        _ => return None,
    };
    Some(g)
}

/// "Core 5 120U", "Core 7 240H": Raptor Lake rebrands without "Ultra".
fn core_series(series: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform::RaptorLake;
    if !(1..=2).contains(&series) {
        return None;
    }
    let g = if suffix.starts_with('u') {
        Guess::new(RaptorLake, "Raptor Lake-U").mobile(true)
    } else if suffix.starts_with('h') {
        Guess::new(RaptorLake, "Raptor Lake-H").mobile(true)
    } else if series == 1 {
        Guess::new(RaptorLake, "Raptor Lake-S")
            .mobile(false)
            .hybrid(false)
    } else {
        Guess::new(RaptorLake, "Bartlett Lake-S").mobile(false)
    };
    Some(g)
}

fn suffix_is_mobile(suffix: &str, generation: u32) -> bool {
    if suffix.contains('m') {
        return true;
    }
    match suffix.as_bytes().first() {
        Some(b'u' | b'y' | b'h' | b'g' | b'n' | b'q') => true,
        Some(b'p') => generation >= 10,
        Some(b'e') => suffix.starts_with("eq"),
        _ => false,
    }
}

/// 1st gen Core: "i7 920", "i5 m 520", "i7 x 980", "i7-720qm".
fn core_first_gen(tier: u32, prefix: Option<char>, n: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let extreme_mobile = suffix == "xm" || (prefix == Some('x') && matches!(n, 920 | 940));
    let mobile =
        matches!(prefix, Some('m' | 'q' | 'u' | 'l')) || suffix.contains('m') || extreme_mobile;
    let g = match (tier, mobile) {
        (7, true) if n >= 700 => Guess::new(P::Arrandale, "Clarksfield"),
        (3 | 5 | 7, true) => Guess::new(P::Arrandale, "Arrandale"),
        // i7-970/980/990X are Gulftown; i7-975 is still a Bloomfield.
        (7, false) if matches!(n, 970 | 980 | 990) => Guess::new(P::NehalemHedt, "Gulftown"),
        (7, false) if n >= 920 => Guess::new(P::NehalemHedt, "Bloomfield"),
        (7, false) if (800..900).contains(&n) => Guess::new(P::Lynnfield, "Lynnfield"),
        (5, false) if (700..800).contains(&n) => Guess::new(P::Lynnfield, "Lynnfield"),
        (5, false) if (600..700).contains(&n) => Guess::new(P::Lynnfield, "Clarkdale"),
        (3, false) if (500..600).contains(&n) => Guess::new(P::Lynnfield, "Clarkdale"),
        _ => return None,
    };
    Some(g.mobile(mobile))
}

/// 2nd to 9th gen Core: four-digit model numbers.
fn core_classic(generation: u32, n: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let mobile = suffix_is_mobile(suffix, generation);
    let x_suffix = suffix.starts_with('x');
    if !mobile {
        let hedt = match n {
            3820 | 3930 | 3960 | 3970 => Some(Guess::new(P::SandyBridgeE, "Sandy Bridge-E")),
            4820 | 4930 | 4960 => Some(Guess::new(P::IvyBridgeE, "Ivy Bridge-E")),
            5820 | 5930 | 5960 => Some(Guess::new(P::HaswellE, "Haswell-E")),
            6800 | 6850 | 6900 | 6950 => Some(Guess::new(P::BroadwellE, "Broadwell-E")),
            7640 | 7740 if x_suffix => Some(Guess::new(P::KabyLake, "Kaby Lake-X")),
            _ if x_suffix && (generation == 7 || generation == 9) => {
                Some(Guess::new(P::SkylakeX, "Skylake-X"))
            }
            _ => None,
        };
        if let Some(g) = hedt {
            return Some(g.mobile(false));
        }
    }
    let first = suffix.as_bytes().first().copied();
    let g = match generation {
        2 => Guess::new(P::SandyBridge, "Sandy Bridge"),
        3 => Guess::new(P::IvyBridge, "Ivy Bridge"),
        4 => match first {
            Some(b'u' | b'y') => Guess::new(P::Haswell, "Haswell-ULT"),
            Some(b'r') => Guess::new(P::Haswell, "Crystal Well"),
            _ if mobile => Guess::new(P::Haswell, "Haswell-H"),
            _ => Guess::new(P::Haswell, "Haswell"),
        },
        5 => match first {
            Some(b'u') => Guess::new(P::Broadwell, "Broadwell-U"),
            Some(b'y') => Guess::new(P::Broadwell, "Broadwell-Y"),
            _ if mobile => Guess::new(P::Broadwell, "Broadwell-H"),
            _ => Guess::new(P::Broadwell, "Broadwell-C"),
        },
        6 => match first {
            Some(b'u') => Guess::new(P::Skylake, "Skylake-U"),
            Some(b'y') => Guess::new(P::Skylake, "Skylake-Y"),
            _ if mobile => Guess::new(P::Skylake, "Skylake-H"),
            _ => Guess::new(P::Skylake, "Skylake-S"),
        },
        7 => match first {
            Some(b'u' | b'y') => Guess::new(P::KabyLake, "Kaby Lake-U/Y"),
            _ if mobile => Guess::new(P::KabyLake, "Kaby Lake-H"),
            _ => Guess::new(P::KabyLake, "Kaby Lake-S"),
        },
        8 => match first {
            Some(b'y') => Guess::new(P::KabyLake, "Amber Lake-Y"),
            Some(b'g') => Guess::new(P::KabyLake, "Kaby Lake-G"),
            Some(b'u') => match n % 100 {
                30 | 50 => Guess::new(P::KabyLake, "Kaby Lake-R"),
                45 | 65 => Guess::new(P::CoffeeLake, "Whiskey Lake-U"),
                21 => Guess::new(P::CoffeeLake, "Cannon Lake-U"),
                _ => Guess::new(P::CoffeeLake, "Coffee Lake-U"),
            },
            _ if mobile => Guess::new(P::CoffeeLake, "Coffee Lake-H"),
            _ => Guess::new(P::CoffeeLake, "Coffee Lake-S"),
        },
        9 if mobile => Guess::new(P::CoffeeLake, "Coffee Lake-H"),
        9 => Guess::new(P::CoffeeLake, "Coffee Lake-S"),
        _ => return None,
    };
    Some(g.mobile(mobile))
}

/// 10th to 14th gen mobile parts with four-digit numbers ("1065G7", "1235U").
fn core_mobile_10plus(generation: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let p_series = suffix.starts_with('p');
    let g = match generation {
        10 => Guess::new(P::IceLake, "Ice Lake-U"),
        11 => Guess::new(P::TigerLake, "Tiger Lake-UP3"),
        12 if p_series => Guess::new(P::AlderLake, "Alder Lake-P"),
        12 => Guess::new(P::AlderLake, "Alder Lake-U"),
        13 | 14 if p_series => Guess::new(P::RaptorLake, "Raptor Lake-P"),
        13 | 14 => Guess::new(P::RaptorLake, "Raptor Lake-U"),
        _ => return None,
    };
    Some(g.mobile(true))
}

/// 10th to 14th gen parts with five-digit numbers ("10900K", "11800H").
fn core_modern(tier: u32, n: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let generation = n / 1000;
    let mobile = suffix_is_mobile(suffix, generation);
    let hx = suffix.starts_with("hx");
    let first = suffix.as_bytes().first().copied();
    let p_core_only = !mobile
        && matches!(generation, 12..=14)
        && (tier == 3
            || (generation == 12
                && tier == 5
                && matches!(n, 12400 | 12490 | 12500 | 12600)
                && !suffix.contains('k')));
    let g = match generation {
        10 if !mobile && suffix.starts_with('x') => Guess::new(P::CascadeLakeX, "Cascade Lake-X"),
        10 => match first {
            Some(b'u') => Guess::new(P::CometLake, "Comet Lake-U"),
            Some(b'y') => Guess::new(P::CometLake, "Comet Lake-Y"),
            _ if mobile => Guess::new(P::CometLake, "Comet Lake-H"),
            _ => Guess::new(P::CometLake, "Comet Lake-S"),
        },
        11 if mobile => Guess::new(P::TigerLake, "Tiger Lake-H"),
        11 if suffix.contains('b') => Guess::new(P::TigerLake, "Tiger Lake-B"),
        11 => Guess::new(P::RocketLake, "Rocket Lake-S"),
        12 if hx => Guess::new(P::AlderLake, "Alder Lake-HX"),
        12 if mobile => Guess::new(P::AlderLake, "Alder Lake-H"),
        12 => Guess::new(P::AlderLake, "Alder Lake-S"),
        13 if hx => Guess::new(P::RaptorLake, "Raptor Lake-HX"),
        13 if mobile => Guess::new(P::RaptorLake, "Raptor Lake-H"),
        13 => Guess::new(P::RaptorLake, "Raptor Lake-S"),
        14 if mobile => Guess::new(P::RaptorLake, "Raptor Lake-HX Refresh"),
        14 => Guess::new(P::RaptorLake, "Raptor Lake-S Refresh"),
        _ => return None,
    };
    let g = g.mobile(mobile);
    Some(if p_core_only { g.hybrid(false) } else { g })
}

/// Core M ("M-5Y10c", "m3-6Y30", "m3-7Y30").
fn core_m(generation: u32) -> Option<Guess> {
    use CpuPlatform as P;
    let g = match generation {
        5 => Guess::new(P::Broadwell, "Broadwell-Y"),
        6 => Guess::new(P::Skylake, "Skylake-Y"),
        7 => Guess::new(P::KabyLake, "Kaby Lake-Y"),
        8 => Guess::new(P::KabyLake, "Amber Lake-Y"),
        _ => return None,
    };
    Some(g.mobile(true))
}

/// Core 2 Duo/Quad/Extreme/Solo by letter prefix and first digit.
fn core2(letters: &str, d1: u32) -> Option<Guess> {
    use CpuPlatform as P;
    let desktop = matches!(letters, "e" | "q" | "qx");
    let penryn = match letters {
        "e" => d1 >= 7,
        "q" | "qx" => d1 >= 8,
        "t" => matches!(d1, 3 | 4 | 6 | 8 | 9),
        "p" | "su" | "sl" | "sp" => true,
        "l" | "x" => d1 >= 9,
        "u" => false,
        _ => return None,
    };
    let g = match (penryn, letters) {
        (true, "e") => Guess::new(P::Penryn, "Wolfdale"),
        (true, "q" | "qx") => Guess::new(P::Penryn, "Yorkfield"),
        (true, _) => Guess::new(P::Penryn, "Penryn"),
        (false, "q" | "qx") => Guess::new(P::Unknown, "Kentsfield (no SSE4.1)"),
        (false, "e") => Guess::new(P::Unknown, "Conroe (no SSE4.1)"),
        (false, _) => Guess::new(P::Unknown, "Merom (no SSE4.1)"),
    };
    Some(g.mobile(!desktop))
}

fn xeon(s: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let modern = |codename| Some(Guess::new(P::XeonModernUnsupported, codename).mobile(false));
    if s.contains("xeon phi") {
        return modern("Xeon Phi");
    }
    if let Some(c) = RE_XEON_SCALABLE.captures(s).filter(|_| s.contains("xeon")) {
        let (d1, d2, suffix) = (num(&c, 2)?, num(&c, 3)?, text(&c, 5));
        return match (d1, d2) {
            (9, 2) => Some(Guess::new(P::CascadeLakeX, "Cascade Lake-AP").mobile(false)),
            (_, 1) => Some(Guess::new(P::SkylakeX, "Skylake-SP").mobile(false)),
            (_, 2) => Some(Guess::new(P::CascadeLakeX, "Cascade Lake-SP").mobile(false)),
            (_, 3) if suffix.starts_with('h') => modern("Cooper Lake"),
            (_, 3) => modern("Ice Lake-SP"),
            (_, 4) => modern("Sapphire Rapids"),
            (_, 5) => modern("Emerald Rapids"),
            _ => modern("Xeon Scalable"),
        };
    }
    if RE_XEON_W_NEW.is_match(s) {
        return modern("Sapphire Rapids");
    }
    if RE_XEON_6.is_match(s) {
        return modern("Xeon 6");
    }
    if let Some(c) = RE_XEON_E3.captures(s) {
        let class = num(&c, 1)?;
        let mobile = class == 5 || text(&c, 3).contains('m');
        let g = match num(&c, 4).unwrap_or(1) {
            0 | 1 => Guess::new(P::SandyBridge, "Sandy Bridge"),
            2 => Guess::new(P::IvyBridge, "Ivy Bridge"),
            3 => Guess::new(P::Haswell, "Haswell"),
            4 => Guess::new(P::Broadwell, "Broadwell"),
            5 => Guess::new(P::Skylake, if mobile { "Skylake-H" } else { "Skylake-S" }),
            6 => Guess::new(
                P::KabyLake,
                if mobile { "Kaby Lake-H" } else { "Kaby Lake-S" },
            ),
            _ => return None,
        };
        return Some(g.mobile(mobile));
    }
    if let Some(c) = RE_XEON_E57.captures(s) {
        let e7 = text(&c, 1) == "7";
        let g = match (e7, num(&c, 4).unwrap_or(1)) {
            (false, 0 | 1) => Guess::new(P::SandyBridgeE, "Sandy Bridge-EP"),
            (true, 0 | 1) => Guess::new(P::NehalemHedt, "Westmere-EX"),
            (false, 2) => Guess::new(P::IvyBridgeE, "Ivy Bridge-EP"),
            (true, 2) => Guess::new(P::IvyBridgeE, "Ivy Bridge-EX"),
            (false, 3) => Guess::new(P::HaswellE, "Haswell-EP"),
            (true, 3) => Guess::new(P::HaswellE, "Haswell-EX"),
            (false, 4) => Guess::new(P::BroadwellE, "Broadwell-EP"),
            (true, 4) => Guess::new(P::BroadwellE, "Broadwell-EX"),
            _ => return None,
        };
        return Some(g.mobile(false));
    }
    if let Some(c) = RE_XEON_E2.captures(s) {
        let mobile = text(&c, 3).contains('m');
        let g = match num(&c, 1)? {
            1 => Guess::new(
                P::CoffeeLake,
                if mobile {
                    "Coffee Lake-H"
                } else {
                    "Coffee Lake-E"
                },
            ),
            2 => Guess::new(
                P::CoffeeLake,
                if mobile {
                    "Coffee Lake-H"
                } else {
                    "Coffee Lake-E Refresh"
                },
            ),
            3 => Guess::new(P::RocketLake, "Rocket Lake-E"),
            4 => Guess::new(P::RaptorLake, "Raptor Lake-E").hybrid(false),
            _ => return None,
        };
        return Some(g.mobile(mobile));
    }
    if let Some(c) = RE_XEON_W.captures(s) {
        let digits = text(&c, 1);
        let n: u32 = digits.parse().ok()?;
        if digits.len() == 5 {
            return match n / 1000 {
                10 => Some(Guess::new(P::CometLake, "Comet Lake-H").mobile(true)),
                11 => Some(Guess::new(P::TigerLake, "Tiger Lake-H").mobile(true)),
                _ => None,
            };
        }
        let g = match (n / 1000, (n / 100) % 10) {
            (1, 2) => Guess::new(P::CometLake, "Comet Lake-W"),
            (1, 3) => Guess::new(P::RocketLake, "Rocket Lake-W"),
            (2 | 3, 1) => Guess::new(P::SkylakeX, "Skylake-W"),
            (2 | 3, 2) => Guess::new(P::CascadeLakeX, "Cascade Lake-W"),
            (3, 3) => Guess::new(P::XeonModernUnsupported, "Ice Lake-W"),
            (2 | 3, 4 | 5) => Guess::new(P::XeonModernUnsupported, "Sapphire Rapids"),
            _ => return None,
        };
        return Some(g.mobile(false));
    }
    if let Some(c) = RE_XEON_D.captures(s) {
        let g = match (num(&c, 1)?, num(&c, 2)?) {
            (1, 5 | 6) => Guess::new(P::BroadwellE, "Broadwell-DE"),
            (2, 1) => Guess::new(P::SkylakeX, "Skylake-D"),
            _ => Guess::new(P::XeonModernUnsupported, "Ice Lake-D"),
        };
        return Some(g.mobile(false));
    }
    if let Some(c) = RE_XEON_OLD.captures(s) {
        let letter = text(&c, 1);
        let g = match (num(&c, 2)?, num(&c, 3)?) {
            (5, 5) => Guess::new(P::NehalemHedt, "Gainestown"),
            (5, 6) => Guess::new(P::NehalemHedt, "Westmere-EP"),
            (5, 2 | 4) => Guess::new(P::Penryn, "Harpertown"),
            (5, 1 | 3) => Guess::new(P::Unknown, "Woodcrest/Clovertown (no SSE4.1)"),
            (3, 5) => Guess::new(P::NehalemHedt, "Bloomfield"),
            (3, 6) => Guess::new(P::NehalemHedt, "Gulftown"),
            (3, 4) => Guess::new(P::Lynnfield, "Lynnfield"),
            (3, 3) => Guess::new(P::Penryn, "Yorkfield"),
            (3, 1) if letter == "e" => Guess::new(P::Penryn, "Wolfdale"),
            (3, 0..=2) => Guess::new(P::Unknown, "Conroe/Kentsfield (no SSE4.1)"),
            (7, 5) => Guess::new(P::NehalemHedt, "Beckton"),
            (7, 4) => Guess::new(P::Penryn, "Dunnington"),
            (7, 3) => Guess::new(P::Unknown, "Tigerton (no SSE4.1)"),
            _ => return None,
        };
        return Some(g.mobile(false));
    }
    None
}

/// Pentium/Celeron (big-core based). AVX2 is cleared in [`merge`].
fn pentium_celeron(s: &str) -> Option<Guess> {
    use CpuPlatform as P;
    if RE_PC_LEGACY.is_match(s) {
        return Some(Guess::new(P::Unknown, "NetBurst / Pentium M (no SSE4.1)"));
    }
    let g = if let Some(c) = RE_PC_G.captures(s) {
        let n = num(&c, 1)?;
        let g = match n {
            100..=999 => Guess::new(P::SandyBridge, "Sandy Bridge"),
            1101 => Guess::new(P::Lynnfield, "Clarkdale"),
            1600..=1699 | 2000..=2199 => Guess::new(P::IvyBridge, "Ivy Bridge"),
            1800..=1899 | 3200..=3499 => Guess::new(P::Haswell, "Haswell"),
            3900..=3929 | 4400..=4559 => Guess::new(P::Skylake, "Skylake-S"),
            3930..=3999 | 4560..=4699 => Guess::new(P::KabyLake, "Kaby Lake-S"),
            4900..=4999 | 5400..=5699 => Guess::new(P::CoffeeLake, "Coffee Lake-S"),
            5900..=5999 | 6400..=6699 => Guess::new(P::CometLake, "Comet Lake-S"),
            6900..=6949 | 7400..=7499 => Guess::new(P::AlderLake, "Alder Lake-S").hybrid(false),
            6950..=6999 => Guess::new(P::Lynnfield, "Clarkdale"),
            _ => return None,
        };
        g.mobile(false)
    } else if RE_PC_P.is_match(s) {
        Guess::new(P::Arrandale, "Arrandale").mobile(true)
    } else if RE_PC_B.is_match(s) {
        Guess::new(P::SandyBridge, "Sandy Bridge").mobile(true)
    } else if let Some(c) = RE_PC_U.captures(s) {
        // SU2300/SU4100 are Penryn ULV, U3400/U5400 Arrandale ULV.
        match (text(&c, 1), num(&c, 2)?) {
            ("su", _) => Guess::new(P::Penryn, "Penryn"),
            (_, 3 | 5) => Guess::new(P::Arrandale, "Arrandale"),
            _ => return None,
        }
        .mobile(true)
    } else if let Some(c) = RE_PC_ET.captures(s) {
        let pentium = s.contains("pentium");
        match (text(&c, 1), num(&c, 2)?, pentium) {
            ("e", 5 | 6, true) | ("e", 3, false) => Guess::new(P::Penryn, "Wolfdale").mobile(false),
            ("e", _, _) => Guess::new(P::Unknown, "Conroe (no SSE4.1)").mobile(false),
            // Pentium T4xxx/T6xxx and Celeron T3xxx are Penryn; T1/T2/T3 Pentiums Merom.
            ("t", 4 | 6, true) | ("t", 3, false) => Guess::new(P::Penryn, "Penryn").mobile(true),
            _ => Guess::new(P::Unknown, "Merom (no SSE4.1)").mobile(true),
        }
    } else if let Some(c) = RE_PC_4DIGIT.captures(s) {
        let n = num(&c, 1)?;
        let g = match n {
            1000..=1099 | 2000..=2199 => Guess::new(P::IvyBridge, "Ivy Bridge"),
            2900..=2999 | 3500..=3599 => Guess::new(P::Haswell, "Haswell-ULT"),
            3855 | 3955 => Guess::new(P::Skylake, "Skylake-U"),
            3860..=3999 => Guess::new(P::KabyLake, "Kaby Lake-U"),
            3200..=3299 | 3700..=3849 => Guess::new(P::Broadwell, "Broadwell-U"),
            4400..=4409 => Guess::new(P::Skylake, "Skylake-U/Y"),
            4410..=4419 => Guess::new(P::KabyLake, "Kaby Lake-U/Y"),
            4425 | 6500 => Guess::new(P::KabyLake, "Amber Lake-Y"),
            4200..=4399 | 5400..=5499 => Guess::new(P::CoffeeLake, "Whiskey Lake-U"),
            5200..=5399 | 6400..=6499 => Guess::new(P::CometLake, "Comet Lake-U"),
            6300..=6399 | 6600..=6699 | 6800..=6899 | 7500..=7599 => {
                Guess::new(P::TigerLake, "Tiger Lake-U")
            }
            7300..=7399 | 8500..=8599 => Guess::new(P::AlderLake, "Alder Lake-U"),
            _ => return None,
        };
        g.mobile(true)
    } else {
        // Sandy Bridge: Celeron 787/797/8x7, Pentium 957-997. Penryn: Celeron
        // 723/743/900/925. Conroe-L: Celeron 420-450.
        let c = RE_PC_3DIGIT.captures(s)?;
        let pentium = s.contains("pentium");
        match (num(&c, 1)?, num(&c, 2)?, num(&c, 3)?) {
            (8, _, 7) | (7, 8 | 9, 7) => Guess::new(P::SandyBridge, "Sandy Bridge").mobile(true),
            (9, 5..=9, 7) if pentium => Guess::new(P::SandyBridge, "Sandy Bridge").mobile(true),
            (4, 2..=5, 0) => Guess::new(P::Unknown, "Conroe-L (no SSE4.1)").mobile(false),
            (9, _, _) | (7, 2..=4, _) => Guess::new(P::Penryn, "Penryn").mobile(true),
            _ => return None,
        }
    };
    Some(g)
}

// AMD patterns
static RE_RYZEN_AI: Lazy<Regex> = Lazy::new(|| {
    re(r"\bryzen\s+ai\s+(max\+?\s+)?(?:pro\s+)?(?:[3579]\s+)?(?:hx\s+)?(?:pro\s+)?(\d)(\d)\d\b")
});
static RE_THREADRIPPER: Lazy<Regex> =
    Lazy::new(|| re(r"\bthreadripper\s+(?:pro\s+)?(\d)\d{3}[a-z]*"));
// The tier digit is optional so manual entries such as "Ryzen 5600X" parse.
static RE_RYZEN4: Lazy<Regex> =
    Lazy::new(|| re(r"\bryzen\s+(?:pro\s+)?(?:[3579]\s+)?(?:pro\s+)?(\d)(\d)(\d)(\d)([a-z0-9]*)"));
static RE_RYZEN3: Lazy<Regex> =
    Lazy::new(|| re(r"\bryzen\s+(?:pro\s+)?[3579]\s+(?:pro\s+)?([12])\d{2}([a-z]*)\b"));
static RE_RYZEN_Z: Lazy<Regex> = Lazy::new(|| re(r"\bryzen\s+z(\d)\s*(extreme|go|a)?\b"));
static RE_EPYC: Lazy<Regex> = Lazy::new(|| re(r"\bepyc\s+(?:embedded\s+)?\d{3}(\d)[a-z]*"));
static RE_ATHLON_K8: Lazy<Regex> = Lazy::new(|| re(r"\bathlon\s+(?:ii|64|xp|mp|neo|x2\s+\d{4})"));
static RE_ATHLON_SG: Lazy<Regex> =
    Lazy::new(|| re(r"\bathlon\s+(?:silver|gold)\s+(?:pro\s+)?(\d)\d{3}([a-z]*)"));
static RE_ATHLON_ZEN: Lazy<Regex> =
    Lazy::new(|| re(r"\bathlon\s+(?:pro\s+)?(\d)\d{2,3}(ge|g|u|c)\b"));
static RE_ATHLON_X: Lazy<Regex> = Lazy::new(|| re(r"\bathlon\s+x[24]\s*(\d)(\d{2})[a-z]*"));
static RE_ATHLON_AM1: Lazy<Regex> = Lazy::new(|| re(r"\bathlon\s+5\d{3}\b"));
static RE_FX: Lazy<Regex> = Lazy::new(|| re(r"\bfx\s*-?\s*(\d)(\d)\d{2}([a-z]*)\b"));
static RE_FX_FM2: Lazy<Regex> = Lazy::new(|| re(r"\bfx\s*-?\s*([67])\d0k\b"));
static RE_A_SERIES: Lazy<Regex> =
    Lazy::new(|| re(r"\ba(?:4|6|8|9|10|12)\s*-\s*(\d)(\d{3})([a-z]*)\b"));
static RE_A_MICRO: Lazy<Regex> = Lazy::new(|| re(r"\ba(?:4|6|10)\s+micro\s*-"));
static RE_E_SERIES: Lazy<Regex> = Lazy::new(|| re(r"\be([12])?\s*-\s*(\d)\d{2,3}[a-z]*\b"));
// AM1 Semprons only; K8 Semprons ("Sempron 3400+") share the digit count.
static RE_SEMPRON_KABINI: Lazy<Regex> = Lazy::new(|| re(r"\bsempron\s+(?:2650|3850)\b"));
static RE_OPTERON: Lazy<Regex> = Lazy::new(|| re(r"\bopteron\s+(x)?(\d)(\d)\d{2}"));

fn parse_amd(s: &str) -> Option<Guess> {
    use CpuPlatform as P;
    if let Some(c) = RE_RYZEN_AI.captures(s) {
        let codename = match (c.get(1).is_some(), num(&c, 2)?, num(&c, 3)?) {
            (true, _, _) => "Strix Halo",
            (false, 4, _) => "Gorgon Point",
            (false, _, 6 | 7) => "Strix Point",
            _ => "Krackan Point",
        };
        return Some(Guess::new(P::AmdZen5, codename).mobile(true));
    }
    if let Some(c) = RE_THREADRIPPER.captures(s) {
        let g = match num(&c, 1)? {
            1 => Guess::new(P::AmdZen, "Whitehaven"),
            2 => Guess::new(P::AmdZen, "Colfax"),
            3 => Guess::new(P::AmdZen2, "Castle Peak"),
            5 => Guess::new(P::AmdZen3, "Chagall"),
            7 => Guess::new(P::AmdZen4, "Storm Peak"),
            9 => Guess::new(P::AmdZen5, "Shimada Peak"),
            _ => return None,
        };
        return Some(g.mobile(false));
    }
    if let Some(c) = RE_RYZEN4.captures(s) {
        return ryzen(
            num(&c, 1)?,
            num(&c, 2)?,
            num(&c, 3)?,
            num(&c, 4)?,
            text(&c, 5),
        );
    }
    if let Some(c) = RE_RYZEN3.captures(s) {
        let g = match num(&c, 1)? {
            2 => Guess::new(P::AmdZen4, "Hawk Point"),
            _ => Guess::new(P::AmdZen3, "Rembrandt-R"),
        };
        return Some(g.mobile(true));
    }
    if let Some(c) = RE_RYZEN_Z.captures(s) {
        let g = match (num(&c, 1)?, text(&c, 2)) {
            (1, _) => Guess::new(P::AmdZen4, "Phoenix"),
            (2, "extreme") => Guess::new(P::AmdZen5, "Strix Point"),
            (2, "go") => Guess::new(P::AmdZen3, "Rembrandt-R"),
            (2, "a") => Guess::new(P::AmdZen2, "Van Gogh"),
            (2, _) => Guess::new(P::AmdZen4, "Hawk Point"),
            _ => return None,
        };
        return Some(g.mobile(true));
    }
    if let Some(c) = RE_EPYC.captures(s) {
        let g = match num(&c, 1)? {
            1 => Guess::new(P::AmdZen, "Naples"),
            2 => Guess::new(P::AmdZen2, "Rome"),
            3 => Guess::new(P::AmdZen3, "Milan"),
            4 => Guess::new(P::AmdZen4, "Genoa"),
            5 => Guess::new(P::AmdZen5, "Turin"),
            _ => return None,
        };
        return Some(g.mobile(false));
    }
    if s.contains("athlon") {
        return athlon(s);
    }
    if let Some(c) = RE_FX.captures(s) {
        let (d1, d2, suffix) = (num(&c, 1)?, num(&c, 2)?, text(&c, 3));
        // P-suffix FX parts and the FX-7500 are laptop APUs; the rest are AM3+.
        let g = if suffix.starts_with('p') || d1 == 7 {
            match d1 {
                9 => Guess::new(P::AmdBulldozer, "Bristol Ridge").avx2(true),
                8 => Guess::new(P::AmdBulldozer, "Carrizo").avx2(true),
                _ => Guess::new(P::AmdBulldozer, "Kaveri"),
            }
            .mobile(true)
        } else if d2 == 1 {
            Guess::new(P::AmdBulldozer, "Zambezi").mobile(false)
        } else {
            Guess::new(P::AmdBulldozer, "Vishera").mobile(false)
        };
        return Some(g);
    }
    if let Some(c) = RE_FX_FM2.captures(s) {
        let codename = if text(&c, 1) == "7" {
            "Kaveri"
        } else {
            "Richland"
        };
        return Some(Guess::new(P::AmdBulldozer, codename).mobile(false));
    }
    if RE_A_MICRO.is_match(s) {
        return Some(Guess::new(P::AmdJaguar, "Mullins").mobile(true));
    }
    if let Some(c) = RE_A_SERIES.captures(s) {
        return a_series(num(&c, 1)?, num(&c, 2)?, text(&c, 3));
    }
    if let Some(c) = RE_E_SERIES.captures(s) {
        let g = match (c.get(1).is_some(), num(&c, 2)?) {
            (false, _) | (true, 1) => Guess::new(P::AmdK10, "Bobcat"),
            (true, 2 | 3) => Guess::new(P::AmdJaguar, "Kabini"),
            (true, 6 | 7) => Guess::new(P::AmdJaguar, "Beema / Carrizo-L"),
            (true, 9) => Guess::new(P::AmdBulldozer, "Stoney Ridge").avx2(true),
            _ => return None,
        };
        return Some(g.mobile(true));
    }
    if s.contains("phenom") {
        return Some(Guess::new(P::AmdK10, "Phenom"));
    }
    if s.contains("turion") {
        return Some(Guess::new(P::AmdK10, "Turion").mobile(true));
    }
    if s.contains("sempron") {
        return Some(if RE_SEMPRON_KABINI.is_match(s) {
            Guess::new(P::AmdJaguar, "Kabini")
        } else {
            Guess::new(P::AmdK10, "Sempron")
        });
    }
    if let Some(c) = RE_OPTERON.captures(s) {
        // 32xx/33xx/42xx/43xx/62xx/63xx Bulldozer/Piledriver; X1xxx/X2xxx
        // Jaguar, X3xxx Excavator; the rest (13xx/23xx/24xx/41xx/61xx/83xx) K10.
        let g = match (c.get(1).is_some(), num(&c, 2)?, num(&c, 3)?) {
            (true, 1 | 2, _) => Guess::new(P::AmdJaguar, "Opteron X (Jaguar)"),
            (true, 3, _) => Guess::new(P::AmdBulldozer, "Opteron X (Excavator)").avx2(true),
            (false, 3 | 4 | 6, 2 | 3) => Guess::new(P::AmdBulldozer, "Opteron (Bulldozer)"),
            (false, _, _) => Guess::new(P::AmdK10, "Opteron (K10)"),
            _ => return None,
        };
        return Some(g.mobile(false));
    }
    None
}

/// Ryzen with a four-digit model number.
fn ryzen(d1: u32, d2: u32, d3: u32, d4: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    // U/H/HS/HX laptops and C (Chromebook) parts.
    let mobile = matches!(suffix.as_bytes().first(), Some(b'u' | b'h' | b'c'));
    let apu = suffix.starts_with('g');
    let g = match d1 {
        1 => Guess::new(P::AmdZen, "Summit Ridge"),
        2 if apu || mobile => Guess::new(P::AmdZen, "Raven Ridge"),
        2 => Guess::new(P::AmdZen, "Pinnacle Ridge"),
        // Ryzen 3 3250U/3250C are Dali; 3200U and the other 3000 APUs Picasso.
        3 if mobile && d2 == 2 && d3 == 5 => Guess::new(P::AmdZen, "Dali"),
        3 if apu || mobile => Guess::new(P::AmdZen, "Picasso"),
        3 => Guess::new(P::AmdZen2, "Matisse"),
        4 => Guess::new(P::AmdZen2, "Renoir"),
        5 if mobile && !suffix.starts_with('h') && d3 == 2 && d4 == 5 => {
            Guess::new(P::AmdZen3, "Barcelo")
        }
        5 if mobile && suffix.starts_with('u') && d3 == 0 && matches!(d2, 3 | 5 | 7) => {
            Guess::new(P::AmdZen2, "Lucienne")
        }
        5 if mobile || apu => Guess::new(P::AmdZen3, "Cezanne"),
        5 if suffix.is_empty() && d4 == 0 && d3 == 0 && matches!(d2, 1 | 5 | 7) => {
            Guess::new(P::AmdZen3, "Cezanne")
        }
        5 => Guess::new(P::AmdZen3, "Vermeer"),
        6 => Guess::new(P::AmdZen3, "Rembrandt"),
        7 if mobile => match d3 {
            1 | 2 => Guess::new(P::AmdZen2, "Mendocino"),
            3 if d4 == 5 => Guess::new(P::AmdZen3, "Rembrandt-R"),
            3 => Guess::new(P::AmdZen3, "Barcelo-R"),
            _ if suffix.starts_with("hx") => Guess::new(P::AmdZen4, "Dragon Range"),
            _ => Guess::new(P::AmdZen4, "Phoenix"),
        },
        7 => Guess::new(P::AmdZen4, "Raphael"),
        8 if mobile => Guess::new(P::AmdZen4, "Hawk Point"),
        8 => Guess::new(P::AmdZen4, "Phoenix"),
        9 if mobile => Guess::new(P::AmdZen5, "Fire Range"),
        9 => Guess::new(P::AmdZen5, "Granite Ridge"),
        _ => return None,
    };
    Some(g.mobile(mobile))
}

fn athlon(s: &str) -> Option<Guess> {
    use CpuPlatform as P;
    if RE_ATHLON_K8.is_match(s) {
        return Some(Guess::new(P::AmdK10, "Athlon II / Athlon 64"));
    }
    if let Some(c) = RE_ATHLON_SG.captures(s) {
        let mobile = text(&c, 2).starts_with('u');
        let g = match num(&c, 1)? {
            3 => Guess::new(P::AmdZen, "Dali"),
            4 => Guess::new(P::AmdZen2, "Renoir"),
            7 => Guess::new(P::AmdZen2, "Mendocino"),
            _ => return None,
        };
        return Some(g.mobile(mobile));
    }
    if let Some(c) = RE_ATHLON_ZEN.captures(s) {
        let mobile = matches!(text(&c, 2), "u" | "c");
        let g = match num(&c, 1)? {
            2 => Guess::new(P::AmdZen, "Raven Ridge"),
            3 => Guess::new(P::AmdZen, "Picasso"),
            4 => Guess::new(P::AmdZen2, "Renoir"),
            _ => return None,
        };
        return Some(g.mobile(mobile));
    }
    if let Some(c) = RE_ATHLON_X.captures(s) {
        let (d1, rest) = (num(&c, 1)?, num(&c, 2)?);
        let g = match (d1, rest) {
            (9, _) => Guess::new(P::AmdBulldozer, "Bristol Ridge").avx2(true),
            (8, 45) => Guess::new(P::AmdBulldozer, "Carrizo").avx2(true),
            (8, _) | (4, _) => Guess::new(P::AmdBulldozer, "Kaveri / Godavari"),
            _ => Guess::new(P::AmdBulldozer, "Trinity / Richland"),
        };
        return Some(g.mobile(false));
    }
    if RE_ATHLON_AM1.is_match(s) {
        return Some(Guess::new(P::AmdJaguar, "Kabini").mobile(false));
    }
    None
}

/// A4/A6/A8/A9/A10/A12 APUs.
fn a_series(d1: u32, rest: u32, suffix: &str) -> Option<Guess> {
    use CpuPlatform as P;
    let mobile = suffix.starts_with('p') || suffix.starts_with('m');
    let g = match d1 {
        1 => Guess::new(P::AmdJaguar, "Temash").mobile(true),
        3 => Guess::new(P::AmdK10, "Llano"),
        5 if suffix.is_empty() && matches!(rest, 0 | 100 | 200) => {
            Guess::new(P::AmdJaguar, "Kabini").mobile(true)
        }
        6 | 7 if suffix.is_empty() && rest % 100 == 10 && (2..=4).contains(&(rest / 100)) => {
            Guess::new(P::AmdJaguar, "Beema / Carrizo-L").mobile(true)
        }
        4..=6 => Guess::new(P::AmdBulldozer, "Trinity / Richland"),
        7 => Guess::new(P::AmdBulldozer, "Kaveri / Godavari"),
        8 => Guess::new(P::AmdBulldozer, "Carrizo").avx2(true),
        9 => Guess::new(P::AmdBulldozer, "Bristol Ridge / Stoney Ridge").avx2(true),
        _ => return None,
    };
    Some(if mobile { g.mobile(true) } else { g })
}

#[cfg(test)]
mod tests {
    use super::*;
    use CpuPlatform as P;

    fn intel(name: &str, model: u32, stepping: u32) -> CpuIdentity {
        identify(name, "GenuineIntel", Some(6), Some(model), Some(stepping))
    }

    fn amd(name: &str, family: u32, model: u32) -> CpuIdentity {
        identify(name, "AuthenticAMD", Some(family), Some(model), Some(0))
    }

    fn brand(name: &str) -> CpuIdentity {
        identify(name, "", None, None, None)
    }

    #[track_caller]
    fn check(id: &CpuIdentity, platform: CpuPlatform, mobile: bool) {
        assert_eq!(id.platform, platform, "{id:?}");
        assert_eq!(id.is_mobile, mobile, "mobile flag of {id:?}");
    }

    // ── CPUID path ──────────────────────────────────────────────────────────

    #[test]
    fn cpuid_core2_and_nehalem() {
        let id = intel("Intel(R) Core(TM)2 Quad CPU Q6600 @ 2.40GHz", 0x0F, 11);
        assert_eq!(id.platform, P::Unknown);
        assert!(!platform_info(id.platform).supported);
        let id = intel("Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz", 0x17, 10);
        check(&id, P::Penryn, false);
        assert_eq!(id.codename, "Wolfdale");
        let id = intel("Intel(R) Core(TM)2 Duo CPU P8600 @ 2.40GHz", 0x17, 6);
        check(&id, P::Penryn, true);
        check(
            &intel("Intel(R) Core(TM) i7 CPU 920 @ 2.67GHz", 0x1A, 5),
            P::NehalemHedt,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i7 CPU 860 @ 2.80GHz", 0x1E, 5),
            P::Lynnfield,
            false,
        );
        let id = intel("Intel(R) Core(TM) i7 CPU Q 720 @ 1.60GHz", 0x1E, 5);
        check(&id, P::Arrandale, true);
        assert_eq!(id.codename, "Clarksfield");
        let id = intel("Intel(R) Core(TM) i5 CPU 650 @ 3.20GHz", 0x25, 5);
        check(&id, P::Lynnfield, false);
        assert_eq!(id.codename, "Clarkdale");
        check(
            &intel("Intel(R) Core(TM) i5 CPU M 520 @ 2.40GHz", 0x25, 5),
            P::Arrandale,
            true,
        );
        let id = intel("Intel(R) Core(TM) i7 CPU X 980 @ 3.33GHz", 0x2C, 2);
        check(&id, P::NehalemHedt, false);
        assert_eq!(id.codename, "Gulftown");
        check(
            &intel("Intel(R) Xeon(R) CPU X5675 @ 3.07GHz", 0x2C, 2),
            P::NehalemHedt,
            false,
        );
    }

    #[test]
    fn cpuid_sandy_to_broadwell() {
        let id = intel("Intel(R) Core(TM) i5-2500K CPU @ 3.30GHz", 0x2A, 7);
        check(&id, P::SandyBridge, false);
        assert!(!id.has_avx2);
        check(
            &intel("Intel(R) Core(TM) i7-2630QM CPU @ 2.00GHz", 0x2A, 7),
            P::SandyBridge,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) i7-3930K CPU @ 3.20GHz", 0x2D, 7),
            P::SandyBridgeE,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i7-3770K CPU @ 3.50GHz", 0x3A, 9),
            P::IvyBridge,
            false,
        );
        check(
            &intel("Intel(R) Xeon(R) CPU E5-2670 v2 @ 2.50GHz", 0x3E, 4),
            P::IvyBridgeE,
            false,
        );
        let id = intel("Intel(R) Core(TM) i7-4770K CPU @ 3.50GHz", 0x3C, 3);
        check(&id, P::Haswell, false);
        assert!(id.has_avx2);
        check(
            &intel("Intel(R) Core(TM) i5-4200U CPU @ 1.60GHz", 0x45, 1),
            P::Haswell,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) i7-4870HQ CPU @ 2.50GHz", 0x46, 1),
            P::Haswell,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) i7-5820K CPU @ 3.30GHz", 0x3F, 2),
            P::HaswellE,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i7-5775C CPU @ 3.30GHz", 0x47, 1),
            P::Broadwell,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i5-5257U CPU @ 2.70GHz", 0x3D, 4),
            P::Broadwell,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) i7-6850K CPU @ 3.60GHz", 0x4F, 1),
            P::BroadwellE,
            false,
        );
        check(
            &intel("Intel(R) Xeon(R) CPU D-1540 @ 2.00GHz", 0x56, 3),
            P::BroadwellE,
            false,
        );
    }

    #[test]
    fn cpuid_skylake_to_comet_lake() {
        check(
            &intel("Intel(R) Core(TM) i7-6700K CPU @ 4.00GHz", 0x5E, 3),
            P::Skylake,
            false,
        );
        let id = intel("Intel(R) Core(TM) i7-6820HQ CPU @ 2.70GHz", 0x5E, 3);
        check(&id, P::Skylake, true);
        assert_eq!(id.codename, "Skylake-H");
        check(
            &intel("Intel(R) Core(TM) i5-6200U CPU @ 2.30GHz", 0x4E, 3),
            P::Skylake,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) i7-7700K CPU @ 4.20GHz", 0x9E, 9),
            P::KabyLake,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i7-7700HQ CPU @ 2.80GHz", 0x9E, 9),
            P::KabyLake,
            true,
        );
        let id = intel("Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz", 0x8E, 10);
        check(&id, P::KabyLake, true);
        assert_eq!(id.codename, "Kaby Lake-R");
        let id = intel("Intel(R) Core(TM) i5-8259U CPU @ 2.30GHz", 0x8E, 10);
        check(&id, P::CoffeeLake, true);
        assert_eq!(id.codename, "Coffee Lake-U");
        let id = intel("Intel(R) Core(TM) i7-8565U CPU @ 1.80GHz", 0x8E, 11);
        check(&id, P::CoffeeLake, true);
        assert_eq!(id.codename, "Whiskey Lake-U");
        let id = intel("Intel(R) Core(TM) i7-8665U CPU @ 1.90GHz", 0x8E, 12);
        check(&id, P::CoffeeLake, true);
        assert_eq!(id.codename, "Whiskey Lake-U");
        let id = intel("Intel(R) Core(TM) i5-8210Y CPU @ 1.60GHz", 0x8E, 12);
        check(&id, P::KabyLake, true);
        assert_eq!(id.codename, "Amber Lake-Y");
        let id = intel("Intel(R) Core(TM) i5-10210U CPU @ 1.60GHz", 0x8E, 12);
        check(&id, P::CometLake, true);
        assert_eq!(id.codename, "Comet Lake-U");
        check(&intel("", 0x8E, 12), P::CometLake, true);
        check(&intel("", 0x8E, 10), P::KabyLake, true);
        let id = intel("Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz", 0x9E, 10);
        check(&id, P::CoffeeLake, false);
        assert_eq!(id.codename, "Coffee Lake-S");
        let id = intel("Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz", 0x9E, 13);
        check(&id, P::CoffeeLake, true);
        assert_eq!(id.codename, "Coffee Lake-H");
        check(&intel("", 0x9E, 10), P::CoffeeLake, false);
        check(
            &intel("Intel(R) Core(TM) i9-10900K CPU @ 3.70GHz", 0xA5, 5),
            P::CometLake,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i7-10750H CPU @ 2.60GHz", 0xA5, 2),
            P::CometLake,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) i5-10210U CPU @ 1.60GHz", 0xA6, 0),
            P::CometLake,
            true,
        );
        let id = intel("", 0xA5, 5);
        assert_eq!(id.codename, "Comet Lake-S");
    }

    #[test]
    fn cpuid_hedt_skylake_x_family() {
        let id = intel("Intel(R) Core(TM) i9-7900X CPU @ 3.30GHz", 0x55, 4);
        check(&id, P::SkylakeX, false);
        assert!(platform_info(id.platform).hedt);
        check(
            &intel("Intel(R) Xeon(R) W-2140B CPU @ 3.20GHz", 0x55, 4),
            P::SkylakeX,
            false,
        );
        check(
            &intel("Intel(R) Core(TM) i9-10980XE CPU @ 3.00GHz", 0x55, 7),
            P::CascadeLakeX,
            false,
        );
        check(
            &intel("Intel(R) Xeon(R) W-3275M CPU @ 2.50GHz", 0x55, 7),
            P::CascadeLakeX,
            false,
        );
        check(
            &intel("Intel(R) Xeon(R) Platinum 8380H CPU @ 2.90GHz", 0x55, 11),
            P::XeonModernUnsupported,
            false,
        );
        check(&intel("", 0x55, 5), P::CascadeLakeX, false);
        check(
            &intel("Intel(R) Xeon(R) Gold 6348 CPU @ 2.60GHz", 0x6A, 6),
            P::XeonModernUnsupported,
            false,
        );
        check(
            &intel("Intel(R) Xeon(R) w9-3495X", 0x8F, 8),
            P::XeonModernUnsupported,
            false,
        );
        check(&intel("", 0xAD, 1), P::XeonModernUnsupported, false);
        check(&intel("", 0xCF, 2), P::XeonModernUnsupported, false);
    }

    #[test]
    fn cpuid_modern_intel() {
        let id = intel("Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz", 0x7E, 5);
        check(&id, P::IceLake, true);
        assert!(platform_info(id.platform).supported);
        check(
            &intel("Intel(R) Core(TM) i7-1165G7 @ 2.80GHz", 0x8C, 1),
            P::TigerLake,
            true,
        );
        check(
            &intel("11th Gen Intel(R) Core(TM) i7-11800H @ 2.30GHz", 0x8D, 1),
            P::TigerLake,
            true,
        );
        let id = intel("11th Gen Intel(R) Core(TM) i9-11900K @ 3.50GHz", 0xA7, 1);
        check(&id, P::RocketLake, false);
        assert!(!id.is_hybrid);
        let id = intel("12th Gen Intel(R) Core(TM) i9-12900K", 0x97, 2);
        check(&id, P::AlderLake, false);
        assert!(id.is_hybrid);
        let id = intel("12th Gen Intel(R) Core(TM) i5-12400F", 0x97, 5);
        check(&id, P::AlderLake, false);
        assert!(!id.is_hybrid);
        check(
            &intel("12th Gen Intel(R) Core(TM) i7-1260P", 0x9A, 3),
            P::AlderLake,
            true,
        );
        let id = intel("13th Gen Intel(R) Core(TM) i9-13900K", 0xB7, 1);
        check(&id, P::RaptorLake, false);
        assert!(id.is_hybrid);
        check(
            &intel("13th Gen Intel(R) Core(TM) i5-13400", 0xBF, 2),
            P::RaptorLake,
            false,
        );
        check(
            &intel("14th Gen Intel(R) Core(TM) i7-14700K", 0xB7, 1),
            P::RaptorLake,
            false,
        );
        let id = intel("13th Gen Intel(R) Core(TM) i9-13980HX", 0xB7, 1);
        check(&id, P::RaptorLake, true);
        assert_eq!(id.codename, "Raptor Lake-HX");
        check(
            &intel("13th Gen Intel(R) Core(TM) i7-1360P", 0xBA, 2),
            P::RaptorLake,
            true,
        );
        let id = intel("Intel(R) Core(TM) Ultra 7 155H", 0xAA, 4);
        check(&id, P::MeteorLake, true);
        assert!(id.is_hybrid);
        assert!(!platform_info(id.platform).supported);
        let id = intel("Intel(R) Core(TM) Ultra 9 285K", 0xC6, 2);
        check(&id, P::ArrowLake, false);
        assert!(id.is_hybrid);
        check(
            &intel("Intel(R) Core(TM) Ultra 9 275HX", 0xC6, 2),
            P::ArrowLake,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) Ultra 7 255H", 0xC5, 2),
            P::ArrowLake,
            true,
        );
        check(
            &intel("Intel(R) Core(TM) Ultra 7 258V", 0xBD, 1),
            P::LunarLake,
            true,
        );
    }

    #[test]
    fn cpuid_atom_class() {
        let id = intel("Intel(R) N100", 0xBE, 0);
        assert_eq!(id.platform, P::IntelAtom);
        assert!(id.has_avx2);
        assert!(!platform_info(id.platform).supported);
        assert_eq!(
            intel("Intel(R) Celeron(R) J4125 CPU @ 2.00GHz", 0x7A, 8).platform,
            P::IntelAtom
        );
        assert_eq!(
            intel("Intel(R) Celeron(R) N4020 CPU @ 1.10GHz", 0x7A, 8).platform,
            P::IntelAtom
        );
        assert_eq!(
            intel("Intel(R) Celeron(R) N5105 @ 2.00GHz", 0x9C, 0).platform,
            P::IntelAtom
        );
        assert_eq!(
            intel("Intel(R) Celeron(R) CPU N3350 @ 1.10GHz", 0x5C, 9).platform,
            P::IntelAtom
        );
        assert_eq!(
            intel("Intel(R) Atom(TM) x5-Z8350 CPU @ 1.44GHz", 0x4C, 4).platform,
            P::IntelAtom
        );
        assert_eq!(
            intel("Intel(R) Core(TM) i3-N305", 0xBE, 0).platform,
            P::IntelAtom
        );
    }

    #[test]
    fn pentium_avx2_survives_cpuid_disagreement() {
        // Unparsed Pentium number: CPUID decides the platform, the AVX2 rule still applies.
        let id = intel("Intel(R) Pentium(R) CPU G9999 @ 3.00GHz", 0x9E, 11);
        assert_eq!(id.platform, P::CoffeeLake);
        assert!(!id.has_avx2);
        let id = intel("Intel(R) Celeron(R) CPU 5205U @ 1.90GHz", 0x8E, 12);
        assert_eq!(id.platform, P::CometLake);
        assert!(!id.has_avx2);
        let id = intel("Intel(R) Pentium(R) Gold 7505 @ 2.00GHz", 0x8C, 1);
        assert_eq!(id.platform, P::TigerLake);
        assert!(id.has_avx2);
    }

    #[test]
    fn cpuid_pentium_celeron_avx2() {
        let id = intel("Intel(R) Pentium(R) CPU G4560 @ 3.50GHz", 0x9E, 9);
        check(&id, P::KabyLake, false);
        assert!(!id.has_avx2);
        let id = intel("Intel(R) Pentium(R) Gold G5400 CPU @ 3.70GHz", 0x9E, 11);
        check(&id, P::CoffeeLake, false);
        assert!(!id.has_avx2);
        let id = intel("Intel(R) Celeron(R) CPU G1840 @ 2.80GHz", 0x3C, 3);
        check(&id, P::Haswell, false);
        assert!(!id.has_avx2);
        let id = intel("Intel(R) Pentium(R) Gold G7400", 0x97, 5);
        check(&id, P::AlderLake, false);
        assert!(id.has_avx2);
        assert!(!id.is_hybrid);
        let id = intel("Intel(R) Core(TM) i3-9100F CPU @ 3.60GHz", 0x9E, 11);
        assert!(id.has_avx2);
    }

    #[test]
    fn cpuid_amd_families() {
        let id = amd("AMD Phenom(tm) II X6 1090T Processor", 0x10, 10);
        assert_eq!(id.platform, P::AmdK10);
        assert!(!platform_info(id.platform).supported);
        let id = amd("AMD FX(tm)-8350 Eight-Core Processor", 0x15, 0x02);
        check(&id, P::AmdBulldozer, false);
        assert!(!id.has_avx2);
        assert_eq!(id.codename, "Vishera");
        let id = amd("AMD A12-9800 RADEON R7, 12 COMPUTE CORES 4C+8G", 0x15, 0x65);
        assert_eq!(id.platform, P::AmdBulldozer);
        assert!(id.has_avx2);
        assert_eq!(
            amd("AMD Athlon(tm) 5350 APU with Radeon(tm) R3", 0x16, 0x00).platform,
            P::AmdJaguar
        );
        let id = amd("AMD Ryzen 7 1700X Eight-Core Processor", 0x17, 0x01);
        check(&id, P::AmdZen, false);
        assert_eq!(id.codename, "Summit Ridge");
        check(
            &amd("AMD Ryzen 5 2600 Six-Core Processor", 0x17, 0x08),
            P::AmdZen,
            false,
        );
        check(
            &amd("AMD Ryzen 5 3400G with Radeon Vega Graphics", 0x17, 0x18),
            P::AmdZen,
            false,
        );
        check(
            &amd("AMD Ryzen 7 3700U with Radeon Vega Mobile Gfx", 0x17, 0x18),
            P::AmdZen,
            true,
        );
        check(
            &amd("AMD Ryzen 9 3900X 12-Core Processor", 0x17, 0x71),
            P::AmdZen2,
            false,
        );
        check(
            &amd("AMD Ryzen Threadripper 3970X 32-Core Processor", 0x17, 0x31),
            P::AmdZen2,
            false,
        );
        check(
            &amd("AMD Ryzen 7 4800H with Radeon Graphics", 0x17, 0x60),
            P::AmdZen2,
            true,
        );
        let id = amd("AMD Ryzen 7 5700U with Radeon Graphics", 0x17, 0x68);
        check(&id, P::AmdZen2, true);
        assert_eq!(id.codename, "Lucienne");
        check(
            &amd("AMD Ryzen 7 5800X 8-Core Processor", 0x19, 0x21),
            P::AmdZen3,
            false,
        );
        check(
            &amd("AMD Ryzen 5 5600G with Radeon Graphics", 0x19, 0x50),
            P::AmdZen3,
            false,
        );
        check(
            &amd("AMD Ryzen 7 6800H with Radeon Graphics", 0x19, 0x44),
            P::AmdZen3,
            true,
        );
        let id = amd("AMD Ryzen 7 7700X 8-Core Processor", 0x19, 0x61);
        check(&id, P::AmdZen4, false);
        assert_eq!(id.codename, "Raphael");
        check(
            &amd("AMD Ryzen 7 7840HS w/ Radeon 780M Graphics", 0x19, 0x74),
            P::AmdZen4,
            true,
        );
        check(
            &amd("AMD Ryzen 7 8700G w/ Radeon 780M Graphics", 0x19, 0x75),
            P::AmdZen4,
            false,
        );
        check(
            &amd("AMD Ryzen Threadripper 7980X 64-Cores", 0x19, 0x18),
            P::AmdZen4,
            false,
        );
        let id = amd("AMD Ryzen 9 9950X 16-Core Processor", 0x1A, 0x44);
        check(&id, P::AmdZen5, false);
        assert_eq!(id.codename, "Granite Ridge");
        check(
            &amd("AMD Ryzen AI 9 HX 370 w/ Radeon 890M", 0x1A, 0x24),
            P::AmdZen5,
            true,
        );
        for id in [
            amd("", 0x17, 0x71),
            amd("", 0x19, 0x61),
            amd("", 0x1A, 0x44),
        ] {
            assert!(id.has_avx2);
            assert!(!id.is_hybrid);
            assert_eq!(id.vendor, CpuVendor::Amd);
        }
    }

    #[test]
    fn amd_family_without_model_uses_brand() {
        let id = identify(
            "AMD Ryzen 9 3900X 12-Core Processor",
            "AuthenticAMD",
            Some(0x17),
            None,
            None,
        );
        assert_eq!(id.platform, P::AmdZen2);
        let id = identify(
            "AMD Ryzen 7 7700X 8-Core Processor",
            "AuthenticAMD",
            Some(0x19),
            None,
            None,
        );
        assert_eq!(id.platform, P::AmdZen4);
        let id = identify("", "AuthenticAMD", Some(0x15), None, None);
        assert_eq!(id.platform, P::AmdBulldozer);
    }

    #[test]
    fn cpuid_beats_conflicting_brand() {
        // Hypervisor exposing a host CPUID with an unrelated brand string.
        let id = intel("AMD Ryzen 9 5950X", 0x9E, 10);
        assert_eq!(id.platform, P::CoffeeLake);
        let id = intel("Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz", 0x3C, 3);
        assert_eq!(id.platform, P::Haswell);
        assert_eq!(id.codename, "Haswell");
    }

    // ── Brand-string fallback ───────────────────────────────────────────────

    #[test]
    fn brand_core2_and_legacy() {
        check(
            &brand("Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz"),
            P::Penryn,
            false,
        );
        check(
            &brand("Intel(R) Core(TM)2 Quad CPU Q9550 @ 2.83GHz"),
            P::Penryn,
            false,
        );
        check(
            &brand("Intel(R) Core(TM)2 Extreme CPU X9100 @ 3.06GHz"),
            P::Penryn,
            true,
        );
        check(
            &brand("Intel(R) Core(TM)2 Duo CPU T9600 @ 2.80GHz"),
            P::Penryn,
            true,
        );
        check(&brand("Core 2 Duo SU9400"), P::Penryn, true);
        assert_eq!(
            brand("Intel(R) Core(TM)2 Quad CPU Q6600 @ 2.40GHz").platform,
            P::Unknown
        );
        assert_eq!(
            brand("Intel(R) Core(TM)2 Duo CPU E6750 @ 2.66GHz").platform,
            P::Unknown
        );
        assert_eq!(
            brand("Intel(R) Core(TM)2 Duo CPU T7500 @ 2.20GHz").platform,
            P::Unknown
        );
        assert_eq!(
            brand("Intel(R) Pentium(R) 4 CPU 3.00GHz").platform,
            P::Unknown
        );
        check(
            &brand("Pentium(R) Dual-Core CPU E5400 @ 2.70GHz"),
            P::Penryn,
            false,
        );
        assert_eq!(
            brand("Intel(R) Pentium(R) Dual CPU E2180 @ 2.00GHz").platform,
            P::Unknown
        );
    }

    #[test]
    fn brand_first_gen_core() {
        check(
            &brand("Intel(R) Core(TM) i7 CPU 920 @ 2.67GHz"),
            P::NehalemHedt,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i7 CPU X 990 @ 3.47GHz"),
            P::NehalemHedt,
            false,
        );
        check(&brand("i7-980X"), P::NehalemHedt, false);
        check(
            &brand("Intel(R) Core(TM) i5 CPU 750 @ 2.67GHz"),
            P::Lynnfield,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i3 CPU 530 @ 2.93GHz"),
            P::Lynnfield,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i7 CPU Q 740 @ 1.73GHz"),
            P::Arrandale,
            true,
        );
        check(
            &brand("Intel(R) Core(TM) i7 CPU X 920 @ 2.00GHz"),
            P::Arrandale,
            true,
        );
        check(&brand("i7-640M"), P::Arrandale, true);
        check(
            &brand("Intel(R) Core(TM) i3 CPU U 380 @ 1.33GHz"),
            P::Arrandale,
            true,
        );
        check(&brand("i5-430UM"), P::Arrandale, true);
    }

    #[test]
    fn brand_core_2xxx_to_9xxx() {
        check(
            &brand("Intel(R) Core(TM) i5-2400 CPU @ 3.10GHz"),
            P::SandyBridge,
            false,
        );
        check(&brand("i7-2960XM"), P::SandyBridge, true);
        check(
            &brand("Intel(R) Core(TM) i7-3820 CPU @ 3.60GHz"),
            P::SandyBridgeE,
            false,
        );
        check(&brand("i7-3820QM"), P::IvyBridge, true);
        check(&brand("i5-3350P"), P::IvyBridge, false);
        check(
            &brand("Intel(R) Core(TM) i7-4930K CPU @ 3.40GHz"),
            P::IvyBridgeE,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i7-4790K CPU @ 4.00GHz"),
            P::Haswell,
            false,
        );
        check(&brand("i7-4700MQ"), P::Haswell, true);
        check(&brand("i7-4770R"), P::Haswell, false);
        check(
            &brand("Intel(R) Core(TM) i7-5960X CPU @ 3.00GHz"),
            P::HaswellE,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i7-6950X CPU @ 3.00GHz"),
            P::BroadwellE,
            false,
        );
        check(&brand("i5-5675C"), P::Broadwell, false);
        check(
            &brand("Intel(R) Core(TM) i5-6500 CPU @ 3.20GHz"),
            P::Skylake,
            false,
        );
        check(&brand("i7-6700HQ"), P::Skylake, true);
        check(
            &brand("Intel(R) Core(TM) i7-7820X CPU @ 3.60GHz"),
            P::SkylakeX,
            false,
        );
        check(&brand("i7-7740X"), P::KabyLake, false);
        check(
            &brand("Intel(R) Core(TM) i3-7100U CPU @ 2.40GHz"),
            P::KabyLake,
            true,
        );
        check(&brand("i7-8809G"), P::KabyLake, true);
        check(&brand("i5-8350U"), P::KabyLake, true);
        check(&brand("i7-8550U"), P::KabyLake, true);
        check(&brand("i5-8265U"), P::CoffeeLake, true);
        check(&brand("i5-8279U"), P::CoffeeLake, true);
        check(&brand("i7-8750H"), P::CoffeeLake, true);
        check(&brand("i5-8500B"), P::CoffeeLake, false);
        check(
            &brand("Intel(R) Core(TM) i9-9900K CPU @ 3.60GHz"),
            P::CoffeeLake,
            false,
        );
        check(&brand("i9-9980HK"), P::CoffeeLake, true);
        check(
            &brand("Intel(R) Core(TM) i9-9940X CPU @ 3.30GHz"),
            P::SkylakeX,
            false,
        );
        assert_eq!(brand("i3-8121U").codename, "Cannon Lake-U");
    }

    #[test]
    fn brand_core_10th_to_14th() {
        check(
            &brand("Intel(R) Core(TM) i9-10900K CPU @ 3.70GHz"),
            P::CometLake,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz"),
            P::CometLake,
            true,
        );
        check(&brand("i7-10875H"), P::CometLake, true);
        check(
            &brand("Intel(R) Core(TM) i9-10980XE CPU @ 3.00GHz"),
            P::CascadeLakeX,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz"),
            P::IceLake,
            true,
        );
        check(
            &brand("Intel(R) Core(TM) i5-1035G1 CPU @ 1.00GHz"),
            P::IceLake,
            true,
        );
        check(
            &brand("Intel(R) Core(TM) i7-1068NG7 CPU @ 2.30GHz"),
            P::IceLake,
            true,
        );
        check(
            &brand("11th Gen Intel(R) Core(TM) i7-1165G7 @ 2.80GHz"),
            P::TigerLake,
            true,
        );
        check(
            &brand("11th Gen Intel(R) Core(TM) i7-11800H @ 2.30GHz"),
            P::TigerLake,
            true,
        );
        check(
            &brand("11th Gen Intel(R) Core(TM) i9-11900KB @ 3.30GHz"),
            P::TigerLake,
            false,
        );
        check(
            &brand("11th Gen Intel(R) Core(TM) i5-11400F @ 2.60GHz"),
            P::RocketLake,
            false,
        );
        let id = brand("12th Gen Intel(R) Core(TM) i7-12700K");
        check(&id, P::AlderLake, false);
        assert!(id.is_hybrid);
        let id = brand("12th Gen Intel(R) Core(TM) i5-12400");
        assert!(!id.is_hybrid);
        assert!(!brand("12th Gen Intel(R) Core(TM) i3-12100F").is_hybrid);
        assert!(brand("12th Gen Intel(R) Core(TM) i5-12600K").is_hybrid);
        check(
            &brand("12th Gen Intel(R) Core(TM) i7-1255U"),
            P::AlderLake,
            true,
        );
        check(
            &brand("12th Gen Intel(R) Core(TM) i9-12900HX"),
            P::AlderLake,
            true,
        );
        check(
            &brand("13th Gen Intel(R) Core(TM) i5-13600K"),
            P::RaptorLake,
            false,
        );
        check(
            &brand("13th Gen Intel(R) Core(TM) i7-1355U"),
            P::RaptorLake,
            true,
        );
        check(
            &brand("14th Gen Intel(R) Core(TM) i9-14900K"),
            P::RaptorLake,
            false,
        );
        check(
            &brand("14th Gen Intel(R) Core(TM) i7-14650HX"),
            P::RaptorLake,
            true,
        );
    }

    #[test]
    fn brand_core_ultra_and_core_n() {
        check(
            &brand("Intel(R) Core(TM) Ultra 7 155H"),
            P::MeteorLake,
            true,
        );
        check(
            &brand("Intel(R) Core(TM) Ultra 5 125U"),
            P::MeteorLake,
            true,
        );
        let id = brand("Intel(R) Core(TM) Ultra 9 285K");
        check(&id, P::ArrowLake, false);
        assert!(id.is_hybrid);
        assert!(id.has_avx2);
        check(
            &brand("Intel(R) Core(TM) Ultra 5 225F"),
            P::ArrowLake,
            false,
        );
        check(
            &brand("Intel(R) Core(TM) Ultra 9 275HX"),
            P::ArrowLake,
            true,
        );
        check(&brand("Intel(R) Core(TM) Ultra 7 255H"), P::ArrowLake, true);
        check(&brand("Intel(R) Core(TM) Ultra 5 235U"), P::ArrowLake, true);
        check(&brand("Intel(R) Core(TM) Ultra 7 258V"), P::LunarLake, true);
        check(&brand("Intel(R) Core(TM) Ultra 9 288V"), P::LunarLake, true);
        check(&brand("Intel(R) Core(TM) 7 150U"), P::RaptorLake, true);
        check(&brand("Intel(R) Core(TM) 7 240H"), P::RaptorLake, true);
    }

    #[test]
    fn brand_core_m_is_not_apple() {
        let id = brand("Intel(R) Core(TM) m3-7Y30 CPU @ 1.00GHz");
        check(&id, P::KabyLake, true);
        assert_eq!(id.vendor, CpuVendor::Intel);
        check(
            &brand("Intel(R) Core(TM) M-5Y10c CPU @ 0.80GHz"),
            P::Broadwell,
            true,
        );
        check(
            &brand("Intel(R) Core(TM) m5-6Y57 CPU @ 1.10GHz"),
            P::Skylake,
            true,
        );
        let id = brand("Intel(R) Core(TM) m3-8100Y CPU @ 1.10GHz");
        check(&id, P::KabyLake, true);
        assert_eq!(id.codename, "Amber Lake-Y");
        check(&brand("m7-6Y75"), P::Skylake, true);
        let apple = brand("Apple M1");
        assert_eq!(apple.platform, P::AppleSilicon);
        assert_eq!(apple.vendor, CpuVendor::Apple);
        assert_eq!(
            identify("Apple M2 Pro", "Apple", None, None, None).platform,
            P::AppleSilicon
        );
        assert_eq!(
            identify("", "Apple", None, None, None).codename,
            "Apple silicon"
        );
    }

    #[test]
    fn brand_pentium_celeron() {
        let id = brand("Intel(R) Celeron(R) CPU G530 @ 2.40GHz");
        check(&id, P::SandyBridge, false);
        check(
            &brand("Intel(R) Pentium(R) CPU G2020 @ 2.90GHz"),
            P::IvyBridge,
            false,
        );
        let id = brand("Intel(R) Pentium(R) CPU G3258 @ 3.20GHz");
        check(&id, P::Haswell, false);
        assert!(!id.has_avx2);
        check(
            &brand("Intel(R) Celeron(R) CPU G3900 @ 2.80GHz"),
            P::Skylake,
            false,
        );
        let id = brand("Intel(R) Pentium(R) CPU G4560 @ 3.50GHz");
        check(&id, P::KabyLake, false);
        assert!(!id.has_avx2);
        check(
            &brand("Intel(R) Celeron(R) G4900 CPU @ 3.10GHz"),
            P::CoffeeLake,
            false,
        );
        let id = brand("Intel(R) Pentium(R) Gold G6400 CPU @ 4.00GHz");
        check(&id, P::CometLake, false);
        assert!(!id.has_avx2);
        check(
            &brand("Intel(R) Celeron(R) G5905 CPU @ 3.50GHz"),
            P::CometLake,
            false,
        );
        check(
            &brand("Intel(R) Pentium(R) CPU G6950 @ 2.80GHz"),
            P::Lynnfield,
            false,
        );
        let id = brand("Intel(R) Celeron(R) G6900");
        check(&id, P::AlderLake, false);
        assert!(id.has_avx2);
        check(
            &brand("Intel(R) Pentium(R) CPU 4415U @ 2.30GHz"),
            P::KabyLake,
            true,
        );
        check(
            &brand("Intel(R) Celeron(R) CPU 1007U @ 1.50GHz"),
            P::IvyBridge,
            true,
        );
        check(
            &brand("Intel(R) Celeron(R) 2957U @ 1.40GHz"),
            P::Haswell,
            true,
        );
        check(
            &brand("Intel(R) Pentium(R) Gold 7505 @ 2.00GHz"),
            P::TigerLake,
            true,
        );
        check(
            &brand("Intel(R) Pentium(R) CPU B960 @ 2.20GHz"),
            P::SandyBridge,
            true,
        );
        check(
            &brand("Intel(R) Pentium(R) CPU P6200 @ 2.13GHz"),
            P::Arrandale,
            true,
        );
        assert_eq!(
            brand("Intel(R) Celeron(R) N4020 CPU @ 1.10GHz").platform,
            P::IntelAtom
        );
        assert_eq!(
            brand("Intel(R) Celeron(R) J4125 CPU @ 2.00GHz").platform,
            P::IntelAtom
        );
        assert_eq!(
            brand("Intel(R) Pentium(R) Silver N5000 CPU @ 1.10GHz").platform,
            P::IntelAtom
        );
        assert_eq!(
            brand("Intel(R) Pentium(R) CPU J2900 @ 2.41GHz").platform,
            P::IntelAtom
        );
        let id = brand("Intel(R) N100");
        assert_eq!(id.platform, P::IntelAtom);
        assert!(id.has_avx2);
        assert_eq!(brand("Intel(R) Core(TM) i3-N305").platform, P::IntelAtom);
        assert_eq!(
            brand("Intel(R) Core(TM) i5-L16G7 CPU @ 1.40GHz").platform,
            P::IntelAtom
        );
    }

    #[test]
    fn brand_xeon() {
        let cases: &[(&str, CpuPlatform, bool)] = &[
            (
                "Intel(R) Xeon(R) CPU E3-1230 @ 3.20GHz",
                P::SandyBridge,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E3-1230 V2 @ 3.30GHz",
                P::IvyBridge,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E3-1231 v3 @ 3.40GHz",
                P::Haswell,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E3-1285 v4 @ 3.50GHz",
                P::Broadwell,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E3-1245 v5 @ 3.50GHz",
                P::Skylake,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E3-1505M v5 @ 2.80GHz",
                P::Skylake,
                true,
            ),
            (
                "Intel(R) Xeon(R) CPU E3-1275 v6 @ 3.80GHz",
                P::KabyLake,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E5-2690 0 @ 2.90GHz",
                P::SandyBridgeE,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E5-2670 v2 @ 2.50GHz",
                P::IvyBridgeE,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E5-2678 v3 @ 2.50GHz",
                P::HaswellE,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz",
                P::BroadwellE,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E7-4870 @ 2.40GHz",
                P::NehalemHedt,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E7-8890 v4 @ 2.20GHz",
                P::BroadwellE,
                false,
            ),
            (
                "Intel(R) Xeon(R) E-2176G CPU @ 3.70GHz",
                P::CoffeeLake,
                false,
            ),
            (
                "Intel(R) Xeon(R) E-2176M CPU @ 2.70GHz",
                P::CoffeeLake,
                true,
            ),
            (
                "Intel(R) Xeon(R) E-2288G CPU @ 3.70GHz",
                P::CoffeeLake,
                false,
            ),
            (
                "Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz",
                P::RocketLake,
                false,
            ),
            (
                "Intel(R) Xeon(R) W-1290P CPU @ 3.70GHz",
                P::CometLake,
                false,
            ),
            ("Intel(R) Xeon(R) W-1390P @ 3.50GHz", P::RocketLake, false),
            (
                "Intel(R) Xeon(R) W-10885M CPU @ 2.40GHz",
                P::CometLake,
                true,
            ),
            (
                "Intel(R) Xeon(R) W-11955M CPU @ 2.60GHz",
                P::TigerLake,
                true,
            ),
            ("Intel(R) Xeon(R) W-2140B CPU @ 3.20GHz", P::SkylakeX, false),
            (
                "Intel(R) Xeon(R) W-2295 CPU @ 3.00GHz",
                P::CascadeLakeX,
                false,
            ),
            ("Intel(R) Xeon(R) W-3175X CPU @ 3.10GHz", P::SkylakeX, false),
            (
                "Intel(R) Xeon(R) W-3275M CPU @ 2.50GHz",
                P::CascadeLakeX,
                false,
            ),
            (
                "Intel(R) Xeon(R) W-3375 CPU @ 2.50GHz",
                P::XeonModernUnsupported,
                false,
            ),
            ("Intel(R) Xeon(R) w7-2495X", P::XeonModernUnsupported, false),
            (
                "Intel(R) Xeon(R) Gold 6148 CPU @ 2.40GHz",
                P::SkylakeX,
                false,
            ),
            (
                "Intel(R) Xeon(R) Silver 4210 CPU @ 2.20GHz",
                P::CascadeLakeX,
                false,
            ),
            (
                "Intel(R) Xeon(R) Platinum 8380 CPU @ 2.30GHz",
                P::XeonModernUnsupported,
                false,
            ),
            (
                "Intel(R) Xeon(R) Gold 6448Y",
                P::XeonModernUnsupported,
                false,
            ),
            ("Intel(R) Xeon(R) 6980P", P::XeonModernUnsupported, false),
            (
                "Intel(R) Xeon(R) CPU X5675 @ 3.07GHz",
                P::NehalemHedt,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU W3680 @ 3.33GHz",
                P::NehalemHedt,
                false,
            ),
            (
                "Intel(R) Xeon(R) CPU E5520 @ 2.27GHz",
                P::NehalemHedt,
                false,
            ),
            ("Intel(R) Xeon(R) CPU X3450 @ 2.67GHz", P::Lynnfield, false),
            ("Intel(R) Xeon(R) CPU E5450 @ 3.00GHz", P::Penryn, false),
            (
                "Intel(R) Xeon(R) CPU D-1540 @ 2.00GHz",
                P::BroadwellE,
                false,
            ),
        ];
        for (name, platform, mobile) in cases {
            check(&brand(name), *platform, *mobile);
        }
        assert_eq!(
            brand("Intel(R) Xeon(R) CPU 5160 @ 3.00GHz").platform,
            P::Unknown
        );
    }

    #[test]
    fn brand_amd() {
        let cases: &[(&str, CpuPlatform, bool)] = &[
            ("AMD Ryzen 7 1700X Eight-Core Processor", P::AmdZen, false),
            (
                "AMD Ryzen 5 2400G with Radeon Vega Graphics",
                P::AmdZen,
                false,
            ),
            (
                "AMD Ryzen 5 2500U with Radeon Vega Mobile Gfx",
                P::AmdZen,
                true,
            ),
            ("AMD Ryzen 7 2700X Eight-Core Processor", P::AmdZen, false),
            ("AMD Ryzen 5 3600 6-Core Processor", P::AmdZen2, false),
            (
                "AMD Ryzen 3 3200G with Radeon Vega Graphics",
                P::AmdZen,
                false,
            ),
            ("AMD Ryzen 3 3250U with Radeon Graphics", P::AmdZen, true),
            (
                "AMD Ryzen 5 PRO 4650G with Radeon Graphics",
                P::AmdZen2,
                false,
            ),
            ("AMD Ryzen 9 4900HS with Radeon Graphics", P::AmdZen2, true),
            ("AMD Ryzen 5 5500U with Radeon Graphics", P::AmdZen2, true),
            ("AMD Ryzen 5 5600U with Radeon Graphics", P::AmdZen3, true),
            ("AMD Ryzen 5 5625U with Radeon Graphics", P::AmdZen3, true),
            ("AMD Ryzen 7 5800X3D 8-Core Processor", P::AmdZen3, false),
            ("AMD Ryzen 7 5700G with Radeon Graphics", P::AmdZen3, false),
            ("AMD Ryzen 9 6900HX with Radeon Graphics", P::AmdZen3, true),
            ("AMD Ryzen 5 7520U with Radeon Graphics", P::AmdZen2, true),
            ("AMD Ryzen 7 7730U with Radeon Graphics", P::AmdZen3, true),
            ("AMD Ryzen 7 7735HS with Radeon Graphics", P::AmdZen3, true),
            (
                "AMD Ryzen 7 7840U w/ Radeon 780M Graphics",
                P::AmdZen4,
                true,
            ),
            ("AMD Ryzen 9 7945HX with Radeon Graphics", P::AmdZen4, true),
            ("AMD Ryzen 9 7950X3D 16-Core Processor", P::AmdZen4, false),
            (
                "AMD Ryzen 5 8600G w/ Radeon 760M Graphics",
                P::AmdZen4,
                false,
            ),
            (
                "AMD Ryzen 7 8845HS w/ Radeon 780M Graphics",
                P::AmdZen4,
                true,
            ),
            ("AMD Ryzen 7 9800X3D 8-Core Processor", P::AmdZen5, false),
            ("AMD Ryzen 9 9955HX", P::AmdZen5, true),
            ("AMD Ryzen AI 9 HX 370 w/ Radeon 890M", P::AmdZen5, true),
            ("AMD RYZEN AI MAX+ 395 w/ Radeon 8060S", P::AmdZen5, true),
            ("AMD Ryzen AI 5 340 w/ Radeon 840M", P::AmdZen5, true),
            ("AMD Ryzen 7 260 w/ Radeon 780M Graphics", P::AmdZen4, true),
            ("AMD Ryzen Z1 Extreme", P::AmdZen4, true),
            (
                "AMD Ryzen Threadripper 1950X 16-Core Processor",
                P::AmdZen,
                false,
            ),
            (
                "AMD Ryzen Threadripper 2990WX 32-Core Processor",
                P::AmdZen,
                false,
            ),
            (
                "AMD Ryzen Threadripper PRO 3995WX 64-Cores",
                P::AmdZen2,
                false,
            ),
            (
                "AMD Ryzen Threadripper PRO 5975WX 32-Cores",
                P::AmdZen3,
                false,
            ),
            ("AMD Ryzen Threadripper 7960X 24-Cores", P::AmdZen4, false),
            ("AMD Ryzen Threadripper 9980X 64-Cores", P::AmdZen5, false),
            ("AMD EPYC 7742 64-Core Processor", P::AmdZen2, false),
            ("AMD EPYC 4564P 16-Core Processor", P::AmdZen4, false),
            (
                "AMD Athlon 3000G with Radeon Vega Graphics",
                P::AmdZen,
                false,
            ),
            (
                "AMD Athlon 200GE with Radeon Vega Graphics",
                P::AmdZen,
                false,
            ),
            (
                "AMD Athlon Silver 3050U with Radeon Graphics",
                P::AmdZen,
                true,
            ),
            ("AMD Athlon Gold 7220U with Radeon 610M", P::AmdZen2, true),
            ("AMD Athlon(tm) II X4 640 Processor", P::AmdK10, false),
            (
                "AMD Athlon(tm) 64 X2 Dual Core Processor 5000+",
                P::AmdK10,
                false,
            ),
            ("AMD Athlon(tm) X4 950", P::AmdBulldozer, false),
            (
                "AMD Athlon(tm) X4 860K Quad Core Processor",
                P::AmdBulldozer,
                false,
            ),
            (
                "AMD Athlon(tm) 5350 APU with Radeon(tm) R3",
                P::AmdJaguar,
                false,
            ),
            (
                "AMD FX(tm)-8350 Eight-Core Processor",
                P::AmdBulldozer,
                false,
            ),
            ("AMD FX(tm)-6100 Six-Core Processor", P::AmdBulldozer, false),
            (
                "AMD FX-9800P RADEON R7, 12 COMPUTE CORES 4C+8G",
                P::AmdBulldozer,
                true,
            ),
            (
                "AMD A10-7850K Radeon R7, 12 Compute Cores 4C+8G",
                P::AmdBulldozer,
                false,
            ),
            (
                "AMD A8-6500 APU with Radeon(tm) HD Graphics",
                P::AmdBulldozer,
                false,
            ),
            (
                "AMD A12-9800 RADEON R7, 12 COMPUTE CORES 4C+8G",
                P::AmdBulldozer,
                false,
            ),
            (
                "AMD A8-3870K APU with Radeon(tm) HD Graphics",
                P::AmdK10,
                false,
            ),
            (
                "AMD A6-6310 APU with AMD Radeon R4 Graphics",
                P::AmdJaguar,
                true,
            ),
            (
                "AMD A4-5000 APU with Radeon(TM) HD Graphics",
                P::AmdJaguar,
                true,
            ),
            ("AMD E-350 Processor", P::AmdK10, true),
            (
                "AMD E1-2500 APU with Radeon(TM) HD Graphics",
                P::AmdJaguar,
                true,
            ),
            ("AMD Phenom(tm) II X6 1090T Processor", P::AmdK10, false),
        ];
        for (name, platform, mobile) in cases {
            let id = brand(name);
            check(&id, *platform, *mobile);
            assert_eq!(id.vendor, CpuVendor::Amd, "{name}");
        }
        assert!(brand("AMD A12-9800 RADEON R7, 12 COMPUTE CORES 4C+8G").has_avx2);
        assert!(brand("AMD Athlon(tm) X4 950").has_avx2);
        assert!(!brand("AMD FX(tm)-8350 Eight-Core Processor").has_avx2);
        assert_eq!(
            brand("AMD Ryzen 5 5500U with Radeon Graphics").codename,
            "Lucienne"
        );
        assert_eq!(
            brand("AMD Ryzen 9 7945HX with Radeon Graphics").codename,
            "Dragon Range"
        );
    }

    #[test]
    fn manual_entry_without_vendor() {
        check(&brand("i7-8700K"), P::CoffeeLake, false);
        check(&brand("Core i5 750"), P::Lynnfield, false);
        check(&brand("Ryzen 5 3600"), P::AmdZen2, false);
        check(&brand("FX-8350"), P::AmdBulldozer, false);
        check(&brand("Xeon E5-2680 v4"), P::BroadwellE, false);
        check(&brand("Core Ultra 9 285K"), P::ArrowLake, false);
        assert_eq!(brand("i7-8700K").vendor, CpuVendor::Intel);
        assert_eq!(brand("Ryzen 5 3600").vendor, CpuVendor::Amd);
        check(&brand("Ryzen 5600X"), P::AmdZen3, false);
        check(&brand("Ryzen 7840HS"), P::AmdZen4, true);
        assert_eq!(brand("Ryzen 7 260").platform, P::AmdZen4);
        assert_eq!(
            brand("Intel(R) Pentium(R) CPU N3540 @ 2.16GHz").codename,
            "Bay Trail"
        );
    }

    #[test]
    fn unknown_inputs_never_panic() {
        for name in [
            "",
            "   ",
            "QEMU Virtual CPU version 2.5+",
            "Common KVM processor",
            "VIA Nano",
            "@@@",
            "i",
            "core",
            "ryzen",
            "xeon",
            "pentium",
            "athlon",
        ] {
            let id = identify(name, "", None, None, None);
            assert_eq!(id.platform, P::Unknown, "{name}");
            assert!(!id.is_hybrid);
        }
        let id = identify("", "GenuineIntel", Some(6), Some(0xFF), Some(0));
        assert_eq!(id.platform, P::Unknown);
        assert_eq!(id.vendor, CpuVendor::Intel);
        let id = identify("", "GenuineIntel", Some(0xF), Some(4), Some(1));
        assert_eq!(id.platform, P::Unknown);
        let id = identify("", "AuthenticAMD", Some(0x30), Some(0), None);
        assert_eq!(id.platform, P::Unknown);
        let id = identify(
            "Intel(R) Core(TM) i7-8700K",
            "GenuineIntel",
            Some(6),
            None,
            None,
        );
        assert_eq!(
            id.platform,
            P::CoffeeLake,
            "family without model falls back to the brand"
        );
        let id = identify("", "Qualcomm", None, None, None);
        assert_eq!(id.platform, P::Unknown);
        assert_eq!(id.vendor, CpuVendor::Unknown);
    }

    #[test]
    fn vendor_strings() {
        assert_eq!(resolve_vendor("GenuineIntel"), CpuVendor::Intel);
        assert_eq!(resolve_vendor("Intel"), CpuVendor::Intel);
        assert_eq!(resolve_vendor("AuthenticAMD"), CpuVendor::Amd);
        assert_eq!(
            resolve_vendor("Advanced Micro Devices, Inc."),
            CpuVendor::Amd
        );
        assert_eq!(resolve_vendor("Apple"), CpuVendor::Apple);
        assert_eq!(resolve_vendor("HygonGenuine"), CpuVendor::Unknown);
        assert_eq!(
            brand_vendor(&normalize("AMD Ryzen 7 5800X 8-Core Processor")),
            CpuVendor::Amd
        );
        assert_eq!(
            brand_vendor(&normalize("Intel(R) Core(TM)2 Duo CPU E8400")),
            CpuVendor::Intel
        );
    }

    #[test]
    fn normalization() {
        assert_eq!(
            normalize("Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz"),
            "intel core i7-8700k"
        );
        assert_eq!(
            normalize("Intel(R) Core(TM)2 Duo CPU     E8400  @ 3.00GHz"),
            "intel core2 duo e8400"
        );
        assert_eq!(
            normalize("AMD FX(tm)-8350 Eight-Core Processor"),
            "amd fx-8350 eight-core"
        );
        assert_eq!(
            normalize("Intel(R) Xeon(R) CPU E5-2690 0 @ 2.90GHz"),
            "intel xeon e5-2690 0"
        );
    }

    // ── Platform table ──────────────────────────────────────────────────────

    #[test]
    fn all_platforms_cover_every_variant_once() {
        let list = all_platforms();
        assert_eq!(list.len(), 38);
        for (i, p) in list.iter().enumerate() {
            assert!(!list[i + 1..].contains(p), "{p:?} listed twice");
            let info = platform_info(*p);
            assert_eq!(info.platform, *p);
            assert!(!info.label.is_empty());
            assert!(!info.notes.is_empty(), "{p:?} has no notes");
        }
    }

    #[test]
    fn platform_ranges() {
        use MacOsVersion::*;
        for p in all_platforms() {
            let info = platform_info(*p);
            if let (Some(min), Some(max)) = (info.min_macos, info.max_macos) {
                assert!(min <= max, "{p:?}");
            }
            if info.supported {
                assert!(info.min_macos.is_some(), "{p:?} supported without a floor");
            } else {
                assert!(info.min_macos.is_none(), "{p:?} unsupported with a floor");
            }
            if info.supported && !info.has_avx2 {
                assert!(
                    info.max_macos.is_some_and(|m| m < Ventura),
                    "{p:?} without AVX2 must stop before Ventura"
                );
            }
        }
        assert_eq!(platform_info(P::Penryn).max_macos, Some(HighSierra));
        assert_eq!(platform_info(P::IvyBridge).max_macos, Some(Monterey));
        assert_eq!(platform_info(P::Haswell).max_macos, None);
        assert_eq!(platform_info(P::CometLake).min_macos, Some(Catalina));
        assert_eq!(platform_info(P::IceLake).min_macos, Some(Catalina));
        assert_eq!(platform_info(P::AmdBulldozer).max_macos, Some(Monterey));
        assert_eq!(platform_info(P::AmdZen4).min_macos, Some(Monterey));
        assert!(platform_info(P::HaswellE).hedt);
        assert!(platform_info(P::NehalemHedt).hedt);
        assert!(!platform_info(P::CoffeeLake).hedt);
        assert!(!platform_info(P::AmdK10).supported);
        assert!(!platform_info(P::IntelAtom).supported);
        assert!(!platform_info(P::XeonModernUnsupported).supported);
        assert!(!platform_info(P::AppleSilicon).supported);
        assert!(!platform_info(P::Unknown).supported);
        assert_eq!(platform_info(P::AmdZen3).vendor, CpuVendor::Amd);
        assert_eq!(platform_info(P::Skylake).vendor, CpuVendor::Intel);
    }

    #[test]
    fn ceiling_workarounds() {
        use MacOsVersion::*;
        let w = ceiling_workaround(P::IvyBridge).unwrap();
        assert_eq!(w.kext, Some("CryptexFixup.kext"));
        assert_eq!(w.from, Ventura);
        assert_eq!(w.max_macos, None);
        let w = ceiling_workaround(P::Penryn).unwrap();
        assert_eq!(w.kext, Some("telemetrap.kext"));
        assert_eq!(w.from, Mojave);
        assert_eq!(w.max_macos, Some(Monterey));
        assert!(ceiling_workaround(P::AmdBulldozer).is_some());
        assert!(ceiling_workaround(P::Haswell).is_none());
        assert!(ceiling_workaround(P::IntelAtom).is_none());
        for p in all_platforms() {
            let info = platform_info(*p);
            if info.supported && info.max_macos.is_some() {
                let w = ceiling_workaround(*p)
                    .unwrap_or_else(|| panic!("{p:?} has a cap but no workaround"));
                assert!(
                    info.max_macos < Some(w.from),
                    "{p:?} workaround starts too early"
                );
                if let Some(max) = w.max_macos {
                    assert!(max >= w.from, "{p:?}");
                }
            }
        }
    }

    #[test]
    fn ceiling_workarounds_per_cpu() {
        use MacOsVersion::*;
        let pentium = brand("Intel(R) Pentium(R) CPU G4560 @ 3.50GHz");
        let w = ceiling_workaround_for(&pentium).unwrap();
        assert_eq!(w.kext, Some("CryptexFixup.kext"));
        assert_eq!(w.from, Ventura);
        let excavator = brand("AMD A12-9800 RADEON R7, 12 COMPUTE CORES 4C+8G");
        let w = ceiling_workaround_for(&excavator).unwrap();
        assert_eq!(w.kext, None, "Excavator has AVX2");
        assert_eq!(w.from, Ventura);
        let fx = brand("AMD FX(tm)-8350 Eight-Core Processor");
        assert_eq!(
            ceiling_workaround_for(&fx).and_then(|w| w.kext),
            Some("CryptexFixup.kext")
        );
        let penryn = brand("Intel(R) Core(TM)2 Quad CPU Q9650 @ 3.00GHz");
        assert_eq!(
            ceiling_workaround_for(&penryn).and_then(|w| w.kext),
            Some("telemetrap.kext")
        );
        assert!(ceiling_workaround_for(&brand("Intel(R) Core(TM) i7-7700K")).is_none());
        assert!(ceiling_workaround_for(&brand("Intel(R) Celeron(R) N4020")).is_none());
        assert!(ceiling_workaround_for(&brand("Apple M2")).is_none());
    }

    #[test]
    fn cryptexfixup_need() {
        use MacOsVersion::*;
        let pentium = brand("Intel(R) Pentium(R) CPU G4560 @ 3.50GHz");
        assert!(needs_cryptexfixup(&pentium, Ventura));
        assert!(!needs_cryptexfixup(&pentium, Monterey));
        let i7 = brand("Intel(R) Core(TM) i7-7700K CPU @ 4.20GHz");
        assert!(!needs_cryptexfixup(&i7, Tahoe));
        let ivy = brand("Intel(R) Core(TM) i7-3770K CPU @ 3.50GHz");
        assert!(needs_cryptexfixup(&ivy, Sonoma));
        let atom = brand("Intel(R) Celeron(R) J4125 CPU @ 2.00GHz");
        assert!(
            !needs_cryptexfixup(&atom, Sonoma),
            "unsupported platforms never get a CryptexFixup build"
        );
        let penryn = brand("Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz");
        assert!(needs_telemetrap(&penryn, Mojave));
        assert!(!needs_telemetrap(&penryn, HighSierra));
        assert!(!needs_telemetrap(&ivy, Monterey));
    }

    #[test]
    fn per_cpu_release_range() {
        use MacOsVersion::*;
        let cases: &[(&str, Option<MacOsVersion>, Option<MacOsVersion>)] = &[
            (
                "Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz",
                Some(HighSierra),
                None,
            ),
            (
                "Intel(R) Core(TM) i7-8565U CPU @ 1.80GHz",
                Some(Mojave),
                None,
            ),
            (
                "Intel(R) Core(TM) i5-8200Y CPU @ 1.30GHz",
                Some(Mojave),
                None,
            ),
            (
                "Intel(R) Core(TM) m3-8100Y CPU @ 1.10GHz",
                Some(Mojave),
                None,
            ),
            (
                "Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz",
                Some(HighSierra),
                None,
            ),
            (
                "Intel(R) Core(TM) i5-10210U CPU @ 1.60GHz",
                Some(Catalina),
                None,
            ),
            (
                "Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz",
                Some(Catalina),
                None,
            ),
            (
                "Intel(R) Pentium(R) CPU G4560 @ 3.50GHz",
                Some(HighSierra),
                Some(Monterey),
            ),
            ("Intel(R) Pentium(R) Gold G7400", Some(Catalina), None),
            (
                "Intel(R) Core(TM) i7-3770K CPU @ 3.50GHz",
                Some(HighSierra),
                Some(Monterey),
            ),
            (
                "Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz",
                Some(HighSierra),
                Some(HighSierra),
            ),
            (
                "AMD FX(tm)-8350 Eight-Core Processor",
                Some(HighSierra),
                Some(Monterey),
            ),
            (
                "AMD A12-9800 RADEON R7, 12 COMPUTE CORES 4C+8G",
                Some(HighSierra),
                Some(Monterey),
            ),
            ("AMD Ryzen 7 7700X 8-Core Processor", Some(Monterey), None),
            (
                "Intel(R) Celeron(R) N4020 CPU @ 1.10GHz",
                None,
                Some(Monterey),
            ),
        ];
        for (name, min, max) in cases {
            let id = brand(name);
            assert_eq!(min_macos_for(&id), *min, "floor of {name}");
            assert_eq!(max_macos_for(&id), *max, "ceiling of {name}");
        }
        // CPUID-only Whiskey Lake (0x8E stepping 11) keeps the Mojave floor.
        assert_eq!(min_macos_for(&intel("", 0x8E, 11)), Some(Mojave));
    }

    #[test]
    fn hedt_detection() {
        assert!(is_hedt(&brand(
            "AMD Ryzen Threadripper 3970X 32-Core Processor"
        )));
        assert!(is_hedt(&amd(
            "AMD Ryzen Threadripper 7980X 64-Cores",
            0x19,
            0x18
        )));
        assert!(is_hedt(&amd("", 0x17, 0x31)));
        assert!(!is_hedt(&brand("AMD Ryzen 9 3950X 16-Core Processor")));
        assert!(is_hedt(&brand(
            "Intel(R) Core(TM) i9-10980XE CPU @ 3.00GHz"
        )));
        assert!(is_hedt(&intel("", 0x3F, 2)));
        assert!(!is_hedt(&brand("Intel(R) Core(TM) i9-9900K CPU @ 3.60GHz")));
    }

    /// One representative CPU per platform, checked through CPUID alone (when
    /// the model number is unambiguous), the brand string alone and both.
    #[test]
    fn every_platform_from_cpuid_and_brand() {
        // (vendor, family, model, stepping, brand, platform, CPUID alone decides)
        #[rustfmt::skip]
        let cases: &[(&str, u32, u32, u32, &str, CpuPlatform, bool)] = &[
            ("GenuineIntel", 6, 0x17, 10, "Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz", P::Penryn, true),
            ("GenuineIntel", 6, 0x1E, 5, "Intel(R) Core(TM) i7 CPU 870 @ 2.93GHz", P::Lynnfield, true),
            ("GenuineIntel", 6, 0x25, 5, "Intel(R) Core(TM) i5 CPU M 540 @ 2.53GHz", P::Arrandale, false),
            ("GenuineIntel", 6, 0x2A, 7, "Intel(R) Core(TM) i7-2600K CPU @ 3.40GHz", P::SandyBridge, true),
            ("GenuineIntel", 6, 0x3A, 9, "Intel(R) Core(TM) i5-3570K CPU @ 3.40GHz", P::IvyBridge, true),
            ("GenuineIntel", 6, 0x3C, 3, "Intel(R) Core(TM) i7-4770K CPU @ 3.50GHz", P::Haswell, true),
            ("GenuineIntel", 6, 0x47, 1, "Intel(R) Core(TM) i7-5775C CPU @ 3.30GHz", P::Broadwell, true),
            ("GenuineIntel", 6, 0x5E, 3, "Intel(R) Core(TM) i7-6700K CPU @ 4.00GHz", P::Skylake, true),
            ("GenuineIntel", 6, 0x9E, 9, "Intel(R) Core(TM) i7-7700K CPU @ 4.20GHz", P::KabyLake, true),
            ("GenuineIntel", 6, 0x9E, 12, "Intel(R) Core(TM) i9-9900K CPU @ 3.60GHz", P::CoffeeLake, true),
            ("GenuineIntel", 6, 0xA5, 5, "Intel(R) Core(TM) i9-10900K CPU @ 3.70GHz", P::CometLake, true),
            ("GenuineIntel", 6, 0x7E, 5, "Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz", P::IceLake, true),
            ("GenuineIntel", 6, 0xA7, 1, "11th Gen Intel(R) Core(TM) i7-11700K @ 3.60GHz", P::RocketLake, true),
            ("GenuineIntel", 6, 0x8C, 1, "11th Gen Intel(R) Core(TM) i5-1135G7 @ 2.40GHz", P::TigerLake, true),
            ("GenuineIntel", 6, 0x97, 2, "12th Gen Intel(R) Core(TM) i7-12700K", P::AlderLake, true),
            ("GenuineIntel", 6, 0xB7, 1, "13th Gen Intel(R) Core(TM) i9-13900K", P::RaptorLake, true),
            ("GenuineIntel", 6, 0xAA, 4, "Intel(R) Core(TM) Ultra 5 125H", P::MeteorLake, true),
            ("GenuineIntel", 6, 0xC6, 2, "Intel(R) Core(TM) Ultra 7 265K", P::ArrowLake, true),
            ("GenuineIntel", 6, 0xBD, 1, "Intel(R) Core(TM) Ultra 5 226V", P::LunarLake, true),
            ("GenuineIntel", 6, 0x1A, 5, "Intel(R) Core(TM) i7 CPU 950 @ 3.07GHz", P::NehalemHedt, true),
            ("GenuineIntel", 6, 0x2D, 7, "Intel(R) Core(TM) i7-3930K CPU @ 3.20GHz", P::SandyBridgeE, true),
            ("GenuineIntel", 6, 0x3E, 4, "Intel(R) Core(TM) i7-4960X CPU @ 3.60GHz", P::IvyBridgeE, true),
            ("GenuineIntel", 6, 0x3F, 2, "Intel(R) Core(TM) i7-5960X CPU @ 3.00GHz", P::HaswellE, true),
            ("GenuineIntel", 6, 0x4F, 1, "Intel(R) Xeon(R) CPU E5-2650 v4 @ 2.20GHz", P::BroadwellE, true),
            ("GenuineIntel", 6, 0x55, 4, "Intel(R) Core(TM) i9-7980XE CPU @ 2.60GHz", P::SkylakeX, true),
            ("GenuineIntel", 6, 0x55, 7, "Intel(R) Xeon(R) W-2245 CPU @ 3.90GHz", P::CascadeLakeX, true),
            ("GenuineIntel", 6, 0x8F, 8, "Intel(R) Xeon(R) Gold 6430", P::XeonModernUnsupported, true),
            ("GenuineIntel", 6, 0x9C, 0, "Intel(R) Celeron(R) N5105 @ 2.00GHz", P::IntelAtom, true),
            ("AuthenticAMD", 0x10, 4, 2, "AMD Phenom(tm) II X4 965 Processor", P::AmdK10, true),
            ("AuthenticAMD", 0x15, 2, 0, "AMD FX(tm)-6300 Six-Core Processor", P::AmdBulldozer, true),
            ("AuthenticAMD", 0x16, 0, 1, "AMD Athlon(tm) 5350 APU with Radeon(tm) R3", P::AmdJaguar, true),
            ("AuthenticAMD", 0x17, 0x08, 2, "AMD Ryzen 7 2700X Eight-Core Processor", P::AmdZen, true),
            ("AuthenticAMD", 0x17, 0x71, 0, "AMD Ryzen 7 3700X 8-Core Processor", P::AmdZen2, true),
            ("AuthenticAMD", 0x19, 0x21, 2, "AMD Ryzen 9 5950X 16-Core Processor", P::AmdZen3, true),
            ("AuthenticAMD", 0x19, 0x61, 2, "AMD Ryzen 5 7600X 6-Core Processor", P::AmdZen4, true),
            ("AuthenticAMD", 0x1A, 0x44, 0, "AMD Ryzen 5 9600X 6-Core Processor", P::AmdZen5, true),
            ("GenuineIntel", 6, 0x0F, 11, "Intel(R) Core(TM)2 CPU 6600 @ 2.40GHz", P::Unknown, true),
        ];
        let mut covered = vec![P::AppleSilicon];
        for (vendor, family, model, stepping, name, platform, cpuid_alone) in cases {
            let full = identify(name, vendor, Some(*family), Some(*model), Some(*stepping));
            assert_eq!(full.platform, *platform, "CPUID + brand: {name}");
            assert_eq!(brand(name).platform, *platform, "brand only: {name}");
            if *cpuid_alone {
                let bare = identify("", vendor, Some(*family), Some(*model), Some(*stepping));
                assert_eq!(bare.platform, *platform, "CPUID only: {name}");
            }
            covered.push(*platform);
        }
        let apple = identify("Apple M3 Max", "Apple", None, None, None);
        assert_eq!(apple.platform, P::AppleSilicon);
        for p in all_platforms() {
            assert!(covered.contains(p), "{p:?} has no identification case");
        }
    }

    #[test]
    fn unlisted_cpuid_model_defers_to_brand() {
        let id = intel("Intel(R) Core(TM) Ultra 9 285K", 0xE7, 0);
        assert_eq!(id.platform, P::ArrowLake);
        let id = intel("", 0xE7, 0);
        assert_eq!(id.platform, P::Unknown);
        assert_eq!(id.codename, "Unknown Intel");
        let id = identify(
            "Intel(R) Core(TM) i9-9900K",
            "GenuineIntel",
            Some(0x13),
            Some(1),
            Some(0),
        );
        assert_eq!(id.platform, P::CoffeeLake);
        let id = amd("AMD Ryzen 5 3600 6-Core Processor", 0x17, 0xF0);
        assert_eq!(id.platform, P::AmdZen2);
        let id = amd("AMD Ryzen 5 2600 Six-Core Processor", 0x17, 0xF0);
        assert_eq!(id.platform, P::AmdZen);
        // A listed but unsupported model is not overridden.
        let id = intel("Intel(R) Core(TM)2 Duo CPU E8400", 0x0F, 11);
        assert_eq!(id.platform, P::Unknown);
    }

    #[test]
    fn brand_edge_cases() {
        #[rustfmt::skip]
        let cases: &[(&str, CpuPlatform, bool)] = &[
            ("Intel(R) Core(TM) i7-7Y75 CPU @ 1.30GHz", P::KabyLake, true),
            ("Intel(R) Core(TM) i5-7Y54 CPU @ 1.20GHz", P::KabyLake, true),
            ("Intel(R) Core(TM) Ultra X7 358H", P::LunarLake, true),
            ("Intel(R) Pentium(R) CPU 4410Y @ 1.50GHz", P::KabyLake, true),
            ("Intel(R) Pentium(R) CPU 4405U @ 2.10GHz", P::Skylake, true),
            ("Intel(R) Pentium(R) CPU U5400 @ 1.20GHz", P::Arrandale, true),
            ("Intel(R) Celeron(R) CPU SU2300 @ 1.20GHz", P::Penryn, true),
            ("Intel(R) Xeon(R) CPU D-1653N @ 2.80GHz", P::BroadwellE, false),
            ("Intel(R) Xeon(R) D-2146NT CPU @ 2.30GHz", P::SkylakeX, false),
            ("Intel(R) Xeon(R) D-1736NT CPU @ 2.70GHz", P::XeonModernUnsupported, false),
            ("Intel(R) Core(TM) i5-11320H @ 3.20GHz", P::TigerLake, true),
            ("Intel(R) Core(TM) i9-12900HK", P::AlderLake, true),
            ("Intel(R) Core(TM) i7-1185G7 @ 3.00GHz", P::TigerLake, true),
            ("Intel(R) Core(TM) i5-9300H CPU @ 2.40GHz", P::CoffeeLake, true),
            ("Intel(R) Core(TM) i7-10710U CPU @ 1.10GHz", P::CometLake, true),
            ("Intel(R) Xeon(R) CPU E5-2697A v4 @ 2.60GHz", P::BroadwellE, false),
            ("Intel(R) Xeon(R) CPU E3-1535M v6 @ 3.10GHz", P::KabyLake, true),
            ("AMD FX(tm)-770K Quad Core Processor", P::AmdBulldozer, false),
            ("AMD A10-6800K APU with Radeon(tm) HD Graphics", P::AmdBulldozer, false),
            ("AMD Opteron(tm) Processor 6380", P::AmdBulldozer, false),
            ("AMD Opteron(tm) Processor 2435", P::AmdK10, false),
            ("AMD Opteron(tm) X2150 APU", P::AmdJaguar, false),
            ("AMD Ryzen 5 7520C with Radeon Graphics", P::AmdZen2, true),
            ("AMD Ryzen 5 PRO 4650GE with Radeon Graphics", P::AmdZen2, false),
            ("AMD Ryzen 7 PRO 7840U w/ Radeon 780M Graphics", P::AmdZen4, true),
            ("AMD Ryzen 5 8500G w/ Radeon 740M Graphics", P::AmdZen4, false),
            ("AMD Ryzen 9 9955HX3D 16-Core Processor", P::AmdZen5, true),
            ("AMD Ryzen Threadripper PRO 9995WX 96-Cores", P::AmdZen5, false),
            ("AMD Athlon PRO 300GE w/ Radeon Vega Graphics", P::AmdZen, false),
            ("AMD Athlon Gold 3150U with Radeon Graphics", P::AmdZen, true),
        ];
        for (name, platform, mobile) in cases {
            check(&brand(name), *platform, *mobile);
        }
        assert!(brand("AMD Opteron(tm) X3421 APU").has_avx2);
        assert_eq!(brand("AMD Opteron(tm) X3421 APU").platform, P::AmdBulldozer);
        assert_eq!(brand("Intel(R) Core(TM) 3 N355").platform, P::IntelAtom);
        assert_eq!(brand("AMD Ryzen 5 5500").codename, "Cezanne");
        assert_eq!(brand("AMD Ryzen 7 5700").codename, "Cezanne");
        assert_eq!(brand("AMD Ryzen 7 5700X").codename, "Vermeer");
        assert_eq!(brand("AMD Ryzen 5 5600").codename, "Vermeer");
        assert_eq!(amd("AMD Ryzen 5 5500", 0x19, 0x50).codename, "Cezanne");
        // Manual entry without any vendor word.
        check(&brand("i7 920"), P::NehalemHedt, false);
        let id = brand("A10-7850K");
        assert_eq!(id.platform, P::AmdBulldozer);
        assert_eq!(id.vendor, CpuVendor::Amd);
        assert_eq!(brand("FX 8350").platform, P::AmdBulldozer);
    }

    #[test]
    fn hybrid_flags() {
        assert!(intel("12th Gen Intel(R) Core(TM) i9-12900K", 0x97, 2).is_hybrid);
        assert!(!intel("", 0x97, 5).is_hybrid);
        assert!(!intel("13th Gen Intel(R) Core(TM) i3-13100", 0xBF, 5).is_hybrid);
        assert!(intel("13th Gen Intel(R) Core(TM) i5-13400", 0xBF, 2).is_hybrid);
        assert!(intel("Intel(R) Core(TM) Ultra 7 155H", 0xAA, 4).is_hybrid);
        assert!(intel("Intel(R) Core(TM) Ultra 7 258V", 0xBD, 1).is_hybrid);
        assert!(!intel("Intel(R) Core(TM) i9-10900K", 0xA5, 5).is_hybrid);
        assert!(!intel("Intel(R) N100", 0xBE, 0).is_hybrid);
        assert!(brand("Intel(R) Core(TM) i5-14400F").is_hybrid);
        assert!(!brand("AMD Ryzen 5 8500G w/ Radeon 740M Graphics").is_hybrid);
    }

    #[test]
    fn legacy_model_numbers() {
        #[rustfmt::skip]
        let cases: &[(&str, CpuPlatform, bool, &str)] = &[
            ("Intel(R) Core(TM) i7 CPU 975 @ 3.33GHz", P::NehalemHedt, false, "Bloomfield"),
            ("Intel(R) Core(TM) i7 CPU 970 @ 3.20GHz", P::NehalemHedt, false, "Gulftown"),
            ("Intel(R) Pentium(R) CPU 997 @ 1.60GHz", P::SandyBridge, true, "Sandy Bridge"),
            ("Intel(R) Pentium(R) CPU 957 @ 1.20GHz", P::SandyBridge, true, "Sandy Bridge"),
            ("Intel(R) Celeron(R) CPU 797 @ 1.40GHz", P::SandyBridge, true, "Sandy Bridge"),
            ("Intel(R) Celeron(R) CPU 807 @ 1.50GHz", P::SandyBridge, true, "Sandy Bridge"),
            ("Intel(R) Celeron(R) CPU 847 @ 1.10GHz", P::SandyBridge, true, "Sandy Bridge"),
            ("Intel(R) Celeron(R) CPU 900 @ 2.20GHz", P::Penryn, true, "Penryn"),
            ("Intel(R) Celeron(R) CPU 743 @ 1.30GHz", P::Penryn, true, "Penryn"),
            ("Pentium(R) Dual-Core CPU T6600 @ 2.20GHz", P::Penryn, true, "Penryn"),
            ("Pentium(R) Dual-Core CPU T4500 @ 2.30GHz", P::Penryn, true, "Penryn"),
            ("Intel(R) Celeron(R) CPU T3100 @ 1.90GHz", P::Penryn, true, "Penryn"),
            ("Intel(R) Celeron(R) CPU 3955U @ 2.00GHz", P::Skylake, true, "Skylake-U"),
            ("Intel(R) Celeron(R) CPU 3865U @ 1.80GHz", P::KabyLake, true, "Kaby Lake-U"),
            ("Intel(R) Pentium(R) Gold 6805 @ 1.10GHz", P::TigerLake, true, "Tiger Lake-U"),
            ("AMD Ryzen 3 3200U with Radeon Vega Mobile Gfx", P::AmdZen, true, "Picasso"),
            ("AMD Ryzen 3 3250U with Radeon Graphics", P::AmdZen, true, "Dali"),
            ("AMD FX-7500 Radeon R7, 10 Compute Cores 4C+6G", P::AmdBulldozer, true, "Kaveri"),
            ("AMD FX(tm)-9590 Eight-Core Processor", P::AmdBulldozer, false, "Vishera"),
            ("AMD Sempron(tm) 2650 APU with Radeon(tm) R3", P::AmdJaguar, false, "Kabini"),
            ("AMD Ryzen 3 5125C with Radeon Graphics", P::AmdZen3, true, "Barcelo"),
            ("AMD Ryzen 7 5825U with Radeon Graphics", P::AmdZen3, true, "Barcelo"),
            ("Intel(R) Celeron(R) N4500 @ 1.10GHz", P::IntelAtom, true, "Jasper Lake"),
            ("Intel(R) Celeron(R) J4125 CPU @ 2.00GHz", P::IntelAtom, false, "Gemini Lake"),
        ];
        for (name, platform, mobile, codename) in cases {
            let id = brand(name);
            check(&id, *platform, *mobile);
            assert_eq!(id.codename, *codename, "{name}");
        }
        for name in [
            "Intel(R) Pentium(R) Dual CPU T2390 @ 1.86GHz",
            "Intel(R) Celeron(R) CPU 220 @ 1.20GHz",
        ] {
            assert_eq!(brand(name).platform, P::Unknown, "{name}");
        }
        // K8 Semprons share the AM1 digit count but are family 0Fh.
        assert_eq!(brand("AMD Sempron(tm) Processor 3400+").platform, P::AmdK10);
        assert!(brand("Intel(R) Pentium(R) Gold 6805 @ 1.10GHz").has_avx2);
        assert!(!brand("Intel(R) Celeron(R) CPU 3955U @ 2.00GHz").has_avx2);
    }

    #[test]
    fn rosetta_host_is_apple_silicon() {
        let id = identify(
            "VirtualApple @ 2.50GHz processor",
            "GenuineIntel",
            Some(6),
            Some(0x2C),
            Some(0),
        );
        assert_eq!(id.platform, P::AppleSilicon);
        assert_eq!(id.vendor, CpuVendor::Apple);
        assert_eq!(id.codename, "Apple silicon");
    }

    #[test]
    fn cryptexfixup_follows_the_workaround_path() {
        use MacOsVersion::*;
        let penryn = brand("Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz");
        for target in [Ventura, Sonoma, Tahoe] {
            assert!(!needs_cryptexfixup(&penryn, target), "{target:?}");
        }
        let celeron = brand("Intel(R) Celeron(R) CPU G1840 @ 2.80GHz");
        assert!(needs_cryptexfixup(&celeron, Tahoe));
        let w = ceiling_workaround_for(&celeron).unwrap();
        assert!(
            !w.caveat.contains("AppleIntelCPUPowerManagement"),
            "XCPM-era parts keep their power management"
        );
        let sandy = brand("Intel(R) Core(TM) i7-2600K CPU @ 3.40GHz");
        assert!(needs_cryptexfixup(&sandy, Ventura));
        assert!(ceiling_workaround_for(&sandy)
            .unwrap()
            .caveat
            .contains("AppleIntelCPUPowerManagement"));
        let excavator = brand("AMD A12-9800 RADEON R7, 12 COMPUTE CORES 4C+8G");
        assert!(!needs_cryptexfixup(&excavator, Sonoma));
        let jaguar = brand("AMD Athlon(tm) 5350 APU with Radeon(tm) R3");
        assert!(needs_cryptexfixup(&jaguar, Ventura));
        assert!(!needs_cryptexfixup(&jaguar, Monterey));
    }
}
