//! Booter / Kernel / UEFI quirks, Kernel->Emulate, the AMD core count and
//! the Booter MMIO whitelist.
//!
//! Values follow the Dortania per-platform config pages (desktop, laptop,
//! HEDT, AMD) as collected in research-intel-desktop §1-4,
//! research-intel-laptop §2.3-2.5 and research-amd §5. Every key written
//! here exists in OpenCore 1.0.8's Sample.plist; keys a platform does not
//! change are written with the Sample value so the plan is self-describing.

use crate::domain::device_db;
use crate::domain::model::{
    BuildPlan, CpuPlatform as P, MacOsVersion, MmioEntry, NoteLevel, PlistScalar, ProfileStorage,
    SettingMap, StorageKind,
};

use super::{note, PlanContext};

/// Booter quirk set, one per firmware family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooterProfile {
    /// Legacy BIOS through OpenDuet (Dortania penryn/clarkdale/nehalem
    /// "Legacy Settings").
    LegacyBios,
    /// Core 2, 1st gen Core and X58 boards with UEFI: RebuildAppleMemoryMap.
    LegacyUefi,
    /// Sample.plist defaults: Sandy Bridge..Kaby Lake, X79, X99, AMD 15h/16h.
    Default,
    /// QEMU/KVM as tested by OSX-KVM.
    Kvm,
    /// Coffee Lake desktops (300-series); ProtectUefiServices on Z390.
    CoffeeDesktop { z390: bool },
    /// Comet Lake and newer desktops (400-800 series).
    CometDesktop,
    /// X299 / C422; newer ASUS firmware does not boot with SetupVirtualMap.
    SkylakeX { setup_virtual_map: bool },
    /// 8th gen Coffee Lake and Whiskey Lake laptops.
    CoffeeLaptop,
    /// 9th gen Coffee Lake-H and Comet Lake laptops.
    CoffeePlusLaptop,
    /// Ice Lake and newer laptops.
    IceLakeLaptop,
    /// AMD family 17h-1Ah by chipset (research-amd §5.2).
    AmdZen {
        setup_virtual_map: bool,
        devirtualise_mmio: bool,
        rebar_zero: bool,
    },
}

/// AM5 MMIO region every AM5 configuration whitelists (research-amd §5.2,
/// §11: "MMIO devirt 0xFD000000 (0x1E00 pages, 0x800000000000100D)").
pub const AM5_MMIO: u64 = 0xFD00_0000;

pub fn apply(ctx: &PlanContext, plan: &mut BuildPlan) {
    let profile = booter_profile(ctx);
    booter(ctx, profile, plan);
    kernel(ctx, plan);
    emulate(ctx, plan);
    uefi(ctx, plan);
    amd_core_count(ctx, plan);
}

/// Pick the Booter quirk family for this machine.
pub fn booter_profile(ctx: &PlanContext) -> BooterProfile {
    use BooterProfile as B;
    if ctx.is_vm {
        return match ctx.profile.vm {
            Some(crate::domain::model::VmKind::Kvm) => B::Kvm,
            _ => B::Default,
        };
    }
    if ctx.legacy_bios {
        return B::LegacyBios;
    }
    if ctx.is_amd() {
        return amd_profile(ctx);
    }
    let platform = ctx.platform();
    let mobile = ctx.mobile_cpu();
    match platform {
        // Dortania penryn/clarkdale/nehalem UEFI tables; the arrandale laptop
        // page keeps Sample defaults (RebuildAppleMemoryMap only for 10.4-10.6).
        P::Penryn | P::Lynnfield | P::NehalemHedt => B::LegacyUefi,
        P::SkylakeX | P::CascadeLakeX => B::SkylakeX {
            setup_virtual_map: !ctx.is_asus,
        },
        P::CoffeeLake if mobile => {
            if ctx.intel_generation() == Some(9) {
                B::CoffeePlusLaptop
            } else {
                B::CoffeeLaptop
            }
        }
        P::CoffeeLake => B::CoffeeDesktop {
            z390: ctx.chipset.as_ref().is_some_and(|c| c.name == "Z390"),
        },
        P::CometLake if mobile => B::CoffeePlusLaptop,
        P::IceLake => B::IceLakeLaptop,
        P::TigerLake | P::AlderLake | P::RaptorLake | P::ArrowLake if mobile => B::IceLakeLaptop,
        P::CometLake
        | P::RocketLake
        | P::TigerLake
        | P::AlderLake
        | P::RaptorLake
        | P::ArrowLake => B::CometDesktop,
        _ => B::Default,
    }
}

fn amd_profile(ctx: &PlanContext) -> BooterProfile {
    let platform = ctx.platform();
    if matches!(platform, P::AmdBulldozer | P::AmdJaguar) {
        return BooterProfile::Default;
    }
    let zen = |svm: bool, devirt: bool, rebar_zero: bool| BooterProfile::AmdZen {
        setup_virtual_map: svm,
        devirtualise_mmio: devirt,
        rebar_zero,
    };
    if ctx.mobile_cpu() {
        // research-amd §5.2 "AMD laptops" row.
        return zen(true, false, false);
    }
    let chipset = ctx
        .chipset
        .as_ref()
        .filter(|c| c.vendor == crate::domain::model::CpuVendor::Amd);
    let threadripper = match ctx.profile.cpu.codename.as_str() {
        "Castle Peak" => Some("TRX40"),
        "Chagall" => Some("WRX80"),
        "Storm Peak" | "Shimada Peak" => Some("TRX50"),
        "Whitehaven" | "Colfax" => Some("X399"),
        _ => None,
    };
    // A Threadripper CPU names its socket better than a desktop chipset id
    // (TRX40/WRX80 boards expose X570 Promontory silicon).
    let name = match (chipset, threadripper) {
        (Some(c), _) if c.is_hedt => c.name.as_str(),
        (_, Some(socket)) => socket,
        (Some(c), None) => c.name.as_str(),
        (None, None) => "",
    };
    match name {
        // Dortania zen: DevirtualiseMmio for TRx40, SetupVirtualMap off.
        "TRX40" | "WRX80" => zen(false, true, false),
        // sTR5 follows the AM5 recipe; its whitelist is board specific.
        "TRX50" | "WRX90" => zen(true, true, false),
        "X399" => zen(true, false, false),
        _ => {
            let am5 = chipset.map_or(matches!(platform, P::AmdZen4 | P::AmdZen5), |c| c.is_am5());
            if am5 {
                // Every published AM5 config: SetupVirtualMap on,
                // DevirtualiseMmio + 0xFD000000, ResizeAppleGpuBars 0.
                zen(true, true, true)
            } else {
                // Dortania zen: "X570, B550, A520 and TRx40 boards might need
                // this disabled; X470 and B450 with late 2020 BIOS updates
                // might also require this disabled" (every board shipping a
                // Ryzen 5000-capable BIOS has one).
                let series = chipset.map_or(0, |c| c.series);
                zen(!matches!(series, 400 | 500), false, false)
            }
        }
    }
}

fn set(map: &mut SettingMap, key: &str, value: PlistScalar) {
    map.insert(key.to_string(), value);
}

fn flag(map: &mut SettingMap, key: &str, value: bool) {
    set(map, key, PlistScalar::Bool(value));
}

/// Booter quirk values; `Default` is Sample.plist 1.0.8
/// (research-opencore-macos §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Booter {
    avoid_runtime_defrag: bool,
    devirtualise_mmio: bool,
    safe_mode_slide: bool,
    write_unprotector: bool,
    protect_uefi_services: bool,
    custom_slide: bool,
    rebuild_memory_map: bool,
    resize_gpu_bars: i64,
    setup_virtual_map: bool,
    sync_runtime_permissions: bool,
}

impl Default for Booter {
    fn default() -> Self {
        Self {
            avoid_runtime_defrag: true,
            devirtualise_mmio: false,
            safe_mode_slide: true,
            write_unprotector: true,
            protect_uefi_services: false,
            custom_slide: true,
            rebuild_memory_map: false,
            resize_gpu_bars: -1,
            setup_virtual_map: true,
            sync_runtime_permissions: false,
        }
    }
}

impl Booter {
    /// RebuildAppleMemoryMap + SyncRuntimePermissions replace
    /// EnableWriteUnprotector on firmware with a MEMORY_ATTRIBUTE_TABLE.
    fn memory_map(devirt: bool, protect_uefi: bool, setup_virtual_map: bool) -> Self {
        Self {
            devirtualise_mmio: devirt,
            write_unprotector: false,
            protect_uefi_services: protect_uefi,
            rebuild_memory_map: true,
            setup_virtual_map,
            sync_runtime_permissions: true,
            ..Self::default()
        }
    }

    fn for_profile(profile: BooterProfile, target: MacOsVersion) -> Self {
        use BooterProfile as B;
        match profile {
            B::LegacyBios => Self {
                // "Big Sur may require this quirk enabled" (APIC table).
                avoid_runtime_defrag: target >= MacOsVersion::BigSur,
                safe_mode_slide: false,
                write_unprotector: false,
                custom_slide: false,
                rebuild_memory_map: true,
                setup_virtual_map: false,
                ..Self::default()
            },
            B::LegacyUefi => Self {
                rebuild_memory_map: true,
                ..Self::default()
            },
            B::Default => Self::default(),
            B::Kvm => Self {
                safe_mode_slide: false,
                custom_slide: false,
                setup_virtual_map: false,
                ..Self::default()
            },
            B::CoffeeDesktop { z390 } => Self::memory_map(true, z390, true),
            B::CometDesktop => Self::memory_map(true, true, false),
            B::SkylakeX { setup_virtual_map } => Self::memory_map(true, false, setup_virtual_map),
            B::CoffeeLaptop => Self::memory_map(false, false, true),
            B::CoffeePlusLaptop => Self::memory_map(true, true, true),
            B::IceLakeLaptop => Self::memory_map(true, true, false),
            B::AmdZen {
                setup_virtual_map,
                devirtualise_mmio,
                rebar_zero,
            } => Self {
                resize_gpu_bars: if rebar_zero { 0 } else { -1 },
                ..Self::memory_map(devirtualise_mmio, false, setup_virtual_map)
            },
        }
    }
}

fn booter(ctx: &PlanContext, profile: BooterProfile, plan: &mut BuildPlan) {
    use BooterProfile as B;
    let b = Booter::for_profile(profile, ctx.target);
    let q = &mut plan.booter_quirks;
    flag(q, "AvoidRuntimeDefrag", b.avoid_runtime_defrag);
    flag(q, "DevirtualiseMmio", b.devirtualise_mmio);
    flag(q, "EnableSafeModeSlide", b.safe_mode_slide);
    flag(q, "EnableWriteUnprotector", b.write_unprotector);
    // Needed by strict PE loaders when SecureBootModel is Disabled (and by
    // OpenDuet); Sample default since 1.0.2.
    flag(q, "FixupAppleEfiImages", true);
    // Chromebook firmware only (Dortania laptop pages).
    flag(q, "ProtectMemoryRegions", false);
    flag(q, "ProtectUefiServices", b.protect_uefi_services);
    flag(q, "ProvideCustomSlide", b.custom_slide);
    flag(q, "RebuildAppleMemoryMap", b.rebuild_memory_map);
    set(q, "ResizeAppleGpuBars", PlistScalar::Int(b.resize_gpu_bars));
    flag(q, "SetupVirtualMap", b.setup_virtual_map);
    flag(q, "SyncRuntimePermissions", b.sync_runtime_permissions);

    if let B::AmdZen {
        devirtualise_mmio: true,
        rebar_zero,
        ..
    } = profile
    {
        if rebar_zero {
            plan.mmio_whitelist.push(MmioEntry {
                address: AM5_MMIO,
                comment: "MMIO devirt 0xFD000000 (0x1E00 pages)".into(),
                enabled: true,
            });
        } else {
            plan.post_install.push(note(
                NoteLevel::Info,
                "firmware",
                "Threadripper MMIO whitelist",
                "DevirtualiseMmio is on. If a device or the boot hangs, boot the DEBUG OpenCore once, copy the \
                 \"MMIO devirt\" addresses from the log into Booter/MmioWhitelist and enable them one at a time.",
            ));
        }
    }
    if b.resize_gpu_bars == 0 {
        plan.notes.push(note(
            NoteLevel::Info,
            "firmware",
            "Resizable BAR",
            "ResizeAppleGpuBars is 0 so macOS sees GPU BARs it can handle with Resizable BAR enabled, the AM5 \
             firmware default.",
        ));
    }
    if matches!(
        profile,
        B::SkylakeX {
            setup_virtual_map: false
        }
    ) {
        plan.notes.push(note(
            NoteLevel::Info,
            "firmware",
            "SetupVirtualMap off for ASUS X299",
            "Newer ASUS X299 firmware (3006 and later) does not boot with SetupVirtualMap. Turn it on if the \
             board runs an older BIOS and boot stops early.",
        ));
    }
    if matches!(
        profile,
        B::AmdZen {
            setup_virtual_map: false,
            ..
        }
    ) {
        plan.notes.push(note(
            NoteLevel::Info,
            "firmware",
            "SetupVirtualMap off",
            "AMD 400/500-series and TRX40 boards with current BIOS versions can hang early with \
             SetupVirtualMap on. If boot stops at the first kernel messages on an older BIOS, turn it on.",
        ));
    }
    if b.rebuild_memory_map && !b.write_unprotector && ctx.mobile_cpu() {
        plan.notes.push(note(
            NoteLevel::Info,
            "firmware",
            "Early boot hang fallback",
            "Some laptop firmware lacks a MEMORY_ATTRIBUTE_TABLE. If boot stops at EB|LOG:EXITBS:START, set \
             RebuildAppleMemoryMap to false and EnableWriteUnprotector to true.",
        ));
    }
}

/// XhciPortLimit policy. Dortania: "Disable if running macOS 11.3+" (the
/// patch broke there and recovery images for Big Sur are 11.5+); OpenCore
/// 1.0.7 re-implemented it for Darwin 25, so on macOS 26 it is turned on as
/// an install-time aid until a USB map exists (research-opencore-macos §1.2,
/// §3.5; critic-gaps §3).
pub fn xhci_port_limit(ctx: &PlanContext) -> bool {
    !ctx.is_vm && (ctx.target <= MacOsVersion::Catalina || ctx.target == MacOsVersion::Tahoe)
}

fn kernel(ctx: &PlanContext, plan: &mut BuildPlan) {
    let bare_intel = ctx.is_intel() && !ctx.is_vm;
    let platform = ctx.platform();
    let target = ctx.target;

    // CFG Lock state is unknown before install, so the quirks stay on
    // (Dortania: "not needed if CFG-Lock is disabled"). Pre-Haswell CPUs use
    // AppleIntelCPUPowerManagement, Haswell and newer XCPM; Penryn has no
    // CFG Lock.
    let acpm = bare_intel
        && matches!(
            platform,
            P::Lynnfield
                | P::Arrandale
                | P::SandyBridge
                | P::IvyBridge
                | P::NehalemHedt
                | P::SandyBridgeE
                | P::IvyBridgeE
        );
    let xcpm = bare_intel && !acpm && platform != P::Penryn;
    let extra_msrs = bare_intel && matches!(platform, P::HaswellE | P::BroadwellE);

    // Aquantia needs VT-d enabled and DisableIoMapper off (Configuration.tex
    // ForceAquantiaEthernet, 10.15.4+).
    let aquantia = ctx.has_aquantia() && target >= MacOsVersion::Catalina;
    let keep_vtd = bare_intel && aquantia;
    let dmar_replaced = plan
        .acpi_deletes
        .iter()
        .any(|d| d.table_signature.eq_ignore_ascii_case("DMAR"))
        || plan
            .ssdts
            .iter()
            .any(|s| s.file_name.to_ascii_uppercase().contains("DMAR"));
    // DisableIoMapperMapping (13.3+) fixes NICs under AppleVTD with more than
    // 16 GB of RAM, but only with a DMAR table without Reserved Memory Regions
    // (Configuration.tex; research-opencore-macos §3.5 item 3).
    let vtd_heavy = keep_vtd && target >= MacOsVersion::Ventura && ctx.profile.ram_gb > 16;
    let io_mapper_mapping = vtd_heavy && dmar_replaced;

    // Configuration.tex SetApfsTrimTimeout: Samsung controllers deallocate so
    // slowly that the boot-time TRIM times out and stops being effective.
    // macOS 12+ only accepts 0 (no boot-time TRIM), which then just saves the
    // ~10 s wait; older releases keep the default timeout.
    let slow_trim =
        target >= MacOsVersion::Monterey && ctx.profile.storage.iter().any(slow_trim_nvme);
    let provide_cpu_info = ctx.is_amd()
        || ctx.is_vm
        || ctx.identity.is_hybrid
        || matches!(platform, P::AlderLake | P::RaptorLake | P::ArrowLake);

    let q = &mut plan.kernel_quirks;
    flag(q, "AppleCpuPmCfgLock", acpm);
    flag(q, "AppleXcpmCfgLock", xcpm);
    flag(q, "AppleXcpmExtraMsrs", extra_msrs);
    flag(q, "AppleXcpmForceBoost", false);
    flag(q, "CustomSMBIOSGuid", ctx.needs_custom_smbios());
    // Dortania: DisableIoMapper YES on Intel; AMD-Vi is not supported by macOS.
    flag(q, "DisableIoMapper", bare_intel && !keep_vtd);
    flag(q, "DisableIoMapperMapping", io_mapper_mapping);
    flag(q, "DisableLinkeditJettison", true);
    flag(q, "DisableRtcChecksum", false);
    flag(q, "ExtendBTFeatureFlags", false);
    flag(q, "ForceAquantiaEthernet", aquantia);
    // Configuration.tex: required on VMs with Apple Secure Boot (OSX-KVM sets it).
    flag(q, "ForceSecureBootScheme", ctx.is_vm);
    flag(q, "IncreasePciBarSize", false);
    flag(q, "LapicKernelPanic", ctx.is_hp && !ctx.is_vm);
    flag(q, "PanicNoKextDump", true);
    flag(q, "PowerTimeoutKernelPanic", true);
    flag(q, "ProvideCurrentCpuInfo", provide_cpu_info);
    set(
        q,
        "SetApfsTrimTimeout",
        PlistScalar::Int(if slow_trim { 0 } else { -1 }),
    );
    flag(q, "ThirdPartyDrives", false);
    flag(q, "XhciPortLimit", xhci_port_limit(ctx));

    if acpm || xcpm {
        plan.notes.push(note(
            NoteLevel::Info,
            "cpu",
            "CFG Lock",
            format!(
                "{} is on because the CFG Lock state of the firmware is unknown. Disable CFG Lock in the BIOS \
                 (or check it with the ControlMsrE2 tool in the OpenCore picker) and the quirk can be turned \
                 off.",
                if acpm { "AppleCpuPmCfgLock" } else { "AppleXcpmCfgLock" }
            ),
        ));
    }
    if slow_trim {
        plan.notes.push(note(
            NoteLevel::Info,
            "storage",
            "Boot-time TRIM off for Samsung NVMe",
            "SetApfsTrimTimeout is 0: Samsung NVMe controllers do not finish the boot-time TRIM of APFS in \
             time, so it only delays every boot by about 10 seconds. Set it back to -1 to try TRIM again.",
        ));
    }
    if xhci_port_limit(ctx) {
        plan.post_install.push(note(
            NoteLevel::Info,
            "usb",
            "Map the USB ports",
            "XhciPortLimit is on so every USB port works during the install. Map the ports (at most 15 per \
             controller) after installing and turn XhciPortLimit off.",
        ));
    }
    if aquantia {
        let detail = if ctx.is_amd() {
            "ForceAquantiaEthernet is on. macOS has no AMD-Vi support, so from macOS 12 the Aquantia driver \
             also needs CaseySJ's Aquantia kernel patches; use another NIC for the install if the link stays \
             down."
                .to_string()
        } else if vtd_heavy && !dmar_replaced {
            "ForceAquantiaEthernet is on and DisableIoMapper is off: enable VT-d in the BIOS. With more than \
             16 GB of RAM on macOS 13.3+, a link that drops needs a DMAR table without Reserved Memory Regions \
             plus the DisableIoMapperMapping quirk."
                .to_string()
        } else {
            "ForceAquantiaEthernet is on and DisableIoMapper is off: enable VT-d in the BIOS and keep the DMAR \
             table."
                .to_string()
        };
        plan.notes.push(note(
            NoteLevel::Warning,
            "ethernet",
            "Aquantia 10GbE",
            detail,
        ));
    }
}

fn slow_trim_nvme(drive: &ProfileStorage) -> bool {
    let samsung_id = drive
        .vendor_id
        .as_deref()
        .and_then(device_db::parse_id16)
        .is_some_and(|v| v == 0x144D);
    drive.kind == StorageKind::Nvme
        && (samsung_id || drive.name.to_ascii_uppercase().contains("SAMSUNG"))
}

/// Kernel->Emulate CPUID spoof, as (Cpuid1Data, Cpuid1Mask, what).
///
/// Configuration.tex "Cpuid1Data" recommendations: Haswell-E → Haswell
/// (0x0306C3), Broadwell-E → Broadwell (0x0306D4), Comet Lake U62
/// (0x0A0660) → U42 (0x0806EC), Rocket Lake / Alder Lake and newer →
/// Comet Lake (0x0A0655). Ice Lake and every other supported platform are
/// native (Dortania leaves Emulate blank, Pentium/Celeron included).
pub fn cpuid_spoof(ctx: &PlanContext) -> Option<([u8; 16], [u8; 16], &'static str)> {
    const MASK: [u8; 16] = [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let eax = |a: u8, b: u8, c: u8| [a, b, c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    if !ctx.is_intel() {
        return None;
    }
    match ctx.platform() {
        P::HaswellE => Some((eax(0xC3, 0x06, 0x03), MASK, "Haswell-E as Haswell (XCPM)")),
        P::BroadwellE => Some((
            eax(0xD4, 0x06, 0x03),
            MASK,
            "Broadwell-E as Broadwell (XCPM)",
        )),
        P::CometLake if ctx.profile.cpu.model == Some(0xA6) => Some((
            eax(0xEC, 0x06, 0x08),
            MASK,
            "Comet Lake U62 as Comet Lake U42",
        )),
        P::RocketLake | P::TigerLake | P::AlderLake | P::RaptorLake | P::ArrowLake => Some((
            eax(0x55, 0x06, 0x0A),
            MASK,
            "unsupported Intel generation as Comet Lake",
        )),
        _ => None,
    }
}

fn emulate(ctx: &PlanContext, plan: &mut BuildPlan) {
    let e = &mut plan.kernel_emulate;
    match cpuid_spoof(ctx) {
        Some((data, mask, _)) => {
            set(e, "Cpuid1Data", PlistScalar::data(&data));
            set(e, "Cpuid1Mask", PlistScalar::data(&mask));
        }
        None => {
            set(e, "Cpuid1Data", PlistScalar::data(&[]));
            set(e, "Cpuid1Mask", PlistScalar::data(&[]));
        }
    }
    // AMD has no native power management (Dortania zen/fx); VMs have none
    // either (OSX-KVM).
    flag(e, "DummyPowerManagement", ctx.is_amd() || ctx.is_vm);
}

fn uefi(ctx: &PlanContext, plan: &mut BuildPlan) {
    let platform = ctx.platform();
    let bare_intel = ctx.is_intel() && !ctx.is_vm;
    // Dortania: "required for all pre-Skylake" (Penryn only with UEFI).
    let flex_ratio = bare_intel
        && match platform {
            P::Penryn => !ctx.legacy_bios,
            P::Lynnfield
            | P::Arrandale
            | P::SandyBridge
            | P::IvyBridge
            | P::Haswell
            | P::Broadwell
            | P::NehalemHedt
            | P::SandyBridgeE
            | P::IvyBridgeE
            | P::HaswellE
            | P::BroadwellE => true,
            _ => false,
        };
    let model = ctx.profile.motherboard_model.to_ascii_uppercase();
    // Configuration.tex ForceOcWriteFlash: "Boot issues ... on e.g. Lenovo
    // ThinkPad T430 and T530 without this quirk".
    let thinkpad_flash = ctx.is_lenovo && (model.contains("T430") || model.contains("T530"));

    let q = &mut plan.uefi_quirks;
    // Disables firmware security features; only coreboot (MrChromebox)
    // firmware is documented to need it, and it cannot be detected here.
    flag(q, "DisableSecurityPolicy", false);
    // "May cause issues on certain laptop firmwares, including Lenovo."
    flag(
        q,
        "EnableVectorAcceleration",
        !(ctx.is_lenovo && ctx.is_laptop),
    );
    flag(q, "ForceOcWriteFlash", thinkpad_flash);
    flag(q, "IgnoreInvalidFlexRatio", flex_ratio);
    // Dortania laptop guides: ReleaseUsbOwnership YES.
    flag(q, "ReleaseUsbOwnership", ctx.is_laptop && !ctx.is_vm);
    // Never needed on OpenDuet (no firmware boot entries to protect).
    flag(q, "RequestBootVarRouting", !ctx.legacy_bios);
    set(q, "ResizeGpuBars", PlistScalar::Int(-1));
    // HP firmware blocks partition handles (Dortania: "Needed mainly by HP").
    flag(q, "UnblockFsConnect", ctx.is_hp && !ctx.is_vm);
}

/// Physical cores per package for the AMD_Vanilla core-count patches.
fn amd_core_count(ctx: &PlanContext, plan: &mut BuildPlan) {
    if !ctx.is_amd() {
        return;
    }
    let cores = ctx.physical_cores();
    if (1..=64).contains(&cores) {
        plan.amd_core_count = Some(cores);
        if ctx.profile.cpu.threads > 64 {
            plan.notes.push(note(
                NoteLevel::Warning,
                "cpu",
                "Disable SMT",
                format!(
                    "macOS handles at most 64 logical CPUs and this CPU has {} threads. Disable SMT \
                     (simultaneous multithreading) in the BIOS before booting macOS.",
                    ctx.profile.cpu.threads
                ),
            ));
        }
    } else {
        plan.amd_core_count = None;
        plan.notes.push(note(
            NoteLevel::Blocking,
            "cpu",
            "AMD core count",
            format!(
                "The AMD kernel patches need the physical core count (1-64); the profile says {cores}. \
                 Correct it in the hardware editor."
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::empty_plan;
    use super::super::test_support::*;
    use super::*;
    use crate::domain::cpu_db;
    use crate::domain::model::{
        AcpiDelete, BuildOptions, FormFactor, GpuFamily, HardwareProfile, VmKind,
    };
    use MacOsVersion::*;

    fn run_with(p: &HardwareProfile, o: &BuildOptions) -> BuildPlan {
        let ctx = PlanContext::new(p, o);
        let mut plan = empty_plan(o.target);
        apply(&ctx, &mut plan);
        plan
    }

    fn run(p: &HardwareProfile, target: MacOsVersion) -> BuildPlan {
        run_with(p, &options(target))
    }

    fn desktop(platform: P, name: &str, codename: &str, cores: u32) -> HardwareProfile {
        profile(
            cpu(platform, name, codename, cores),
            FormFactor::Desktop,
            vec![gpu(GpuFamily::AmdPolaris, false)],
        )
    }

    fn laptop(platform: P, name: &str, codename: &str, cores: u32) -> HardwareProfile {
        profile(
            cpu(platform, name, codename, cores),
            FormFactor::Laptop,
            vec![],
        )
    }

    fn booter_set(plan: &BuildPlan) -> [bool; 7] {
        let b = &plan.booter_quirks;
        [
            on(b, "DevirtualiseMmio"),
            on(b, "EnableWriteUnprotector"),
            on(b, "ProtectUefiServices"),
            on(b, "RebuildAppleMemoryMap"),
            on(b, "SetupVirtualMap"),
            on(b, "SyncRuntimePermissions"),
            on(b, "AvoidRuntimeDefrag"),
        ]
    }

    #[test]
    fn intel_desktop_booter_tables() {
        // [DevirtMmio, EWU, PUS, RAMM, SVM, SRP, ARD]
        let defaults = [false, true, false, false, true, false, true];
        for (platform, name) in [
            (P::SandyBridge, "i7-2600K"),
            (P::IvyBridge, "i7-3770K"),
            (P::Haswell, "i7-4790K"),
            (P::Broadwell, "i7-5775C"),
            (P::Skylake, "i7-6700K"),
            (P::KabyLake, "i7-7700K"),
            (P::SandyBridgeE, "i7-3930K"),
            (P::HaswellE, "i7-5960X"),
        ] {
            let plan = run(&desktop(platform, name, "x", 4), Monterey);
            assert_eq!(booter_set(&plan), defaults, "{platform:?}");
        }
        let mut coffee = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i9-9900K",
            "Coffee Lake-S",
            8,
        );
        assert_eq!(
            booter_set(&run(&coffee, Sequoia)),
            [true, false, false, true, true, true, true]
        );
        coffee.chipset = Some("Z390".into());
        assert_eq!(
            booter_set(&run(&coffee, Sequoia)),
            [true, false, true, true, true, true, true]
        );
        for platform in [
            P::CometLake,
            P::RocketLake,
            P::AlderLake,
            P::RaptorLake,
            P::ArrowLake,
        ] {
            let plan = run(&desktop(platform, "x", "x", 8), Sequoia);
            assert_eq!(
                booter_set(&plan),
                [true, false, true, true, false, true, true],
                "{platform:?}"
            );
        }
        let mut x299 = desktop(P::SkylakeX, "Intel(R) Core(TM) i9-7900X", "Skylake-X", 10);
        assert_eq!(
            booter_set(&run(&x299, Sequoia)),
            [true, false, false, true, true, true, true]
        );
        x299.motherboard_vendor = "ASUSTeK COMPUTER INC.".into();
        let plan = run(&x299, Sequoia);
        assert!(!on(&plan.booter_quirks, "SetupVirtualMap"));
        for platform in [P::Penryn, P::Lynnfield, P::NehalemHedt] {
            let plan = run(&desktop(platform, "x", "x", 4), HighSierra);
            assert_eq!(
                booter_set(&plan),
                [false, true, false, true, true, false, true],
                "{platform:?}"
            );
        }
    }

    #[test]
    fn legacy_bios_booter() {
        let mut p = desktop(P::Penryn, "Intel(R) Core(TM)2 Quad Q9550", "Penryn", 4);
        p.firmware_uefi = Some(false);
        let plan = run(&p, HighSierra);
        let b = &plan.booter_quirks;
        assert!(!on(b, "AvoidRuntimeDefrag"));
        assert!(!on(b, "EnableSafeModeSlide"));
        assert!(!on(b, "EnableWriteUnprotector"));
        assert!(on(b, "FixupAppleEfiImages"));
        assert!(!on(b, "ProvideCustomSlide"));
        assert!(on(b, "RebuildAppleMemoryMap"));
        assert!(!on(b, "SetupVirtualMap"));
        assert!(!on(&plan.uefi_quirks, "RequestBootVarRouting"));
        assert!(!on(&plan.uefi_quirks, "IgnoreInvalidFlexRatio"));
        assert!(on(&run(&p, BigSur).booter_quirks, "AvoidRuntimeDefrag"));
    }

    #[test]
    fn intel_laptop_booter_tables() {
        let defaults = [false, true, false, false, true, false, true];
        for (platform, name) in [
            (P::Arrandale, "Intel(R) Core(TM) i5 CPU M 520"),
            (P::SandyBridge, "Intel(R) Core(TM) i5-2520M"),
            (P::IvyBridge, "Intel(R) Core(TM) i5-3320M"),
            (P::Broadwell, "Intel(R) Core(TM) i5-5200U"),
            (P::Haswell, "Intel(R) Core(TM) i5-4200U"),
            (P::Skylake, "Intel(R) Core(TM) i5-6200U"),
            (P::KabyLake, "Intel(R) Core(TM) i5-8250U"),
        ] {
            assert_eq!(
                booter_set(&run(&laptop(platform, name, "x", 4), Ventura)),
                defaults,
                "{platform:?}"
            );
        }
        let cfl8 = laptop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8750H",
            "Coffee Lake-H",
            6,
        );
        assert_eq!(
            booter_set(&run(&cfl8, Sequoia)),
            [false, false, false, true, true, true, true]
        );
        let whl = laptop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8565U",
            "Whiskey Lake-U",
            4,
        );
        assert_eq!(
            booter_set(&run(&whl, Sequoia)),
            [false, false, false, true, true, true, true]
        );
        let cfl9 = laptop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-9750H",
            "Coffee Lake-H",
            6,
        );
        assert_eq!(
            booter_set(&run(&cfl9, Sequoia)),
            [true, false, true, true, true, true, true]
        );
        let cml = laptop(
            P::CometLake,
            "Intel(R) Core(TM) i5-10210U",
            "Comet Lake-U",
            4,
        );
        assert_eq!(
            booter_set(&run(&cml, Sequoia)),
            [true, false, true, true, true, true, true]
        );
        let icl = laptop(P::IceLake, "Intel(R) Core(TM) i5-1035G7", "Ice Lake-U", 4);
        let plan = run(&icl, Tahoe);
        assert_eq!(
            booter_set(&plan),
            [true, false, true, true, false, true, true]
        );
        assert!(!on(&plan.booter_quirks, "ProtectMemoryRegions"));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.title == "Early boot hang fallback"));
    }

    #[test]
    fn amd_booter_and_mmio() {
        let mut am5 = desktop(
            P::AmdZen4,
            "AMD Ryzen 5 7600X 6-Core Processor",
            "Raphael",
            6,
        );
        am5.chipset = Some("B650".into());
        let plan = run(&am5, Sequoia);
        assert_eq!(
            booter_set(&plan),
            [true, false, false, true, true, true, true]
        );
        assert_eq!(int_of(&plan.booter_quirks, "ResizeAppleGpuBars"), Some(0));
        assert_eq!(plan.mmio_whitelist.len(), 1);
        assert_eq!(plan.mmio_whitelist[0].address, 0xFD00_0000);
        assert!(plan.mmio_whitelist[0].enabled);
        // Zen 5 without a detected chipset is AM5 too.
        let zen5 = desktop(
            P::AmdZen5,
            "AMD Ryzen 9 9950X 16-Core Processor",
            "Granite Ridge",
            16,
        );
        assert_eq!(run(&zen5, Tahoe).mmio_whitelist.len(), 1);

        let mut b550 = desktop(
            P::AmdZen3,
            "AMD Ryzen 7 5800X 8-Core Processor",
            "Vermeer",
            8,
        );
        b550.chipset = Some("B550".into());
        let plan = run(&b550, Sequoia);
        assert_eq!(
            booter_set(&plan),
            [false, false, false, true, false, true, true]
        );
        assert!(plan.mmio_whitelist.is_empty());
        assert_eq!(int_of(&plan.booter_quirks, "ResizeAppleGpuBars"), Some(-1));
        let mut b350 = b550.clone();
        b350.chipset = Some("B350".into());
        assert!(on(&run(&b350, Sequoia).booter_quirks, "SetupVirtualMap"));

        let trx40 = desktop(
            P::AmdZen2,
            "AMD Ryzen Threadripper 3970X 32-Core Processor",
            "Castle Peak",
            32,
        );
        let plan = run(&trx40, Sequoia);
        assert_eq!(
            booter_set(&plan),
            [true, false, false, true, false, true, true]
        );
        assert!(plan.mmio_whitelist.is_empty());
        assert!(plan.post_install.iter().any(|n| n.title.contains("MMIO")));
        // TRX40 boards report the X570 Promontory id; the CPU decides.
        let mut trx40_x570 = trx40.clone();
        trx40_x570.chipset = Some("X570".into());
        assert_eq!(
            booter_set(&run(&trx40_x570, Sequoia)),
            [true, false, false, true, false, true, true]
        );
        let mut x399 = desktop(
            P::AmdZen,
            "AMD Ryzen Threadripper 1950X 16-Core Processor",
            "Whitehaven",
            16,
        );
        x399.chipset = Some("X399".into());
        assert_eq!(
            booter_set(&run(&x399, Monterey)),
            [false, false, false, true, true, true, true]
        );
        let trx50 = desktop(
            P::AmdZen4,
            "AMD Ryzen Threadripper 7960X 24-Cores",
            "Storm Peak",
            24,
        );
        let plan = run(&trx50, Tahoe);
        assert!(on(&plan.booter_quirks, "DevirtualiseMmio"));
        assert!(plan.mmio_whitelist.is_empty());

        let fx = desktop(
            P::AmdBulldozer,
            "AMD FX(tm)-8350 Eight-Core Processor",
            "Vishera",
            4,
        );
        let plan = run(&fx, Monterey);
        assert_eq!(
            booter_set(&plan),
            [false, true, false, false, true, false, true]
        );
        let apu_laptop = laptop(
            P::AmdZen2,
            "AMD Ryzen 7 4800U with Radeon Graphics",
            "Renoir",
            8,
        );
        assert_eq!(
            booter_set(&run(&apu_laptop, Sequoia)),
            [false, false, false, true, true, true, true]
        );
    }

    #[test]
    fn amd_kernel_and_core_count() {
        let mut p = desktop(
            P::AmdZen3,
            "AMD Ryzen 9 5950X 16-Core Processor",
            "Vermeer",
            16,
        );
        let plan = run(&p, Tahoe);
        let k = &plan.kernel_quirks;
        assert!(on(k, "ProvideCurrentCpuInfo"));
        assert!(!on(k, "DisableIoMapper"));
        assert!(!on(k, "AppleXcpmCfgLock") && !on(k, "AppleCpuPmCfgLock"));
        assert!(on(&plan.kernel_emulate, "DummyPowerManagement"));
        assert_eq!(
            data_of(&plan.kernel_emulate, "Cpuid1Data"),
            Some(String::new())
        );
        assert_eq!(plan.amd_core_count, Some(16));
        assert!(on(k, "XhciPortLimit"), "Tahoe install aid");
        assert!(!on(&run(&p, Sequoia).kernel_quirks, "XhciPortLimit"));

        let fx = desktop(
            P::AmdBulldozer,
            "AMD FX(tm)-8350 Eight-Core Processor",
            "Vishera",
            4,
        );
        let mut fx = fx;
        fx.cpu.threads = 8;
        assert_eq!(
            run(&fx, Monterey).amd_core_count,
            Some(8),
            "15h counts modules as two cores"
        );

        p.cpu.cores = 64;
        p.cpu.threads = 128;
        let plan = run(&p, Sequoia);
        assert_eq!(plan.amd_core_count, Some(64));
        assert!(plan.notes.iter().any(|n| n.title == "Disable SMT"));
        p.cpu.cores = 0;
        p.cpu.threads = 0;
        let plan = run(&p, Sequoia);
        assert_eq!(plan.amd_core_count, None);
        assert!(plan.notes.iter().any(|n| n.level == NoteLevel::Blocking));

        let intel = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i7-8700K",
            "Coffee Lake-S",
            6,
        );
        assert_eq!(run(&intel, Sequoia).amd_core_count, None);
    }

    #[test]
    fn intel_kernel_quirks() {
        let ivy = desktop(P::IvyBridge, "Intel(R) Core(TM) i7-3770", "Ivy Bridge", 4);
        let k = run(&ivy, Monterey).kernel_quirks;
        assert!(on(&k, "AppleCpuPmCfgLock") && !on(&k, "AppleXcpmCfgLock"));
        assert!(on(&k, "DisableIoMapper"));
        let k = run(&desktop(P::Penryn, "x", "Penryn", 4), HighSierra).kernel_quirks;
        assert!(!on(&k, "AppleCpuPmCfgLock") && !on(&k, "AppleXcpmCfgLock"));
        assert!(on(&k, "XhciPortLimit"));
        let plan = run(&desktop(P::Haswell, "x", "Haswell", 4), Catalina);
        let k = &plan.kernel_quirks;
        assert!(
            on(k, "AppleXcpmCfgLock")
                && !on(k, "AppleCpuPmCfgLock")
                && !on(k, "AppleXcpmExtraMsrs")
        );
        assert!(on(k, "XhciPortLimit"));
        assert!(plan.post_install.iter().any(|n| n.component == "usb"));
        assert!(plan.notes.iter().any(|n| n.title == "CFG Lock"));
        assert!(!on(
            &run(&desktop(P::Haswell, "x", "Haswell", 4), BigSur).kernel_quirks,
            "XhciPortLimit"
        ));
        let k = run(&desktop(P::HaswellE, "x", "Haswell-E", 8), Sequoia).kernel_quirks;
        assert!(on(&k, "AppleXcpmExtraMsrs") && on(&k, "AppleXcpmCfgLock"));
        let k = run(&desktop(P::SkylakeX, "x", "Skylake-X", 10), Sequoia).kernel_quirks;
        assert!(!on(&k, "AppleXcpmExtraMsrs"));
        for platform in [P::AlderLake, P::RaptorLake, P::ArrowLake] {
            let k = run(&desktop(platform, "x", "x", 8), Sequoia).kernel_quirks;
            assert!(on(&k, "ProvideCurrentCpuInfo"), "{platform:?}");
        }
        let k = run(&desktop(P::RocketLake, "x", "x", 8), Sequoia).kernel_quirks;
        assert!(!on(&k, "ProvideCurrentCpuInfo"));
        for k in [
            "PanicNoKextDump",
            "PowerTimeoutKernelPanic",
            "DisableLinkeditJettison",
        ] {
            assert!(on(&run(&ivy, Monterey).kernel_quirks, k), "{k}");
        }
    }

    #[test]
    fn vendor_quirks() {
        let mut hp = laptop(P::KabyLake, "Intel(R) Core(TM) i5-8250U", "Kaby Lake-R", 4);
        hp.motherboard_vendor = "HP".into();
        let plan = run(&hp, Ventura);
        assert!(on(&plan.kernel_quirks, "LapicKernelPanic"));
        assert!(on(&plan.uefi_quirks, "UnblockFsConnect"));
        assert!(on(&plan.uefi_quirks, "ReleaseUsbOwnership"));
        let mut dell = hp.clone();
        dell.motherboard_vendor = "Dell Inc.".into();
        let plan = run(&dell, Ventura);
        assert!(on(&plan.kernel_quirks, "CustomSMBIOSGuid"));
        assert!(!on(&plan.kernel_quirks, "LapicKernelPanic"));
        let mut thinkpad = hp.clone();
        thinkpad.motherboard_vendor = "LENOVO".into();
        thinkpad.motherboard_model = "ThinkPad T530 2429BQ1".into();
        thinkpad.cpu.platform = P::IvyBridge;
        let plan = run(&thinkpad, Catalina);
        assert!(on(&plan.uefi_quirks, "ForceOcWriteFlash"));
        assert!(!on(&plan.uefi_quirks, "EnableVectorAcceleration"));
        let mut surface = hp.clone();
        surface.motherboard_vendor = "Microsoft Corporation".into();
        surface.motherboard_model = "Surface Laptop 3".into();
        let plan = run(&surface, Ventura);
        assert!(!on(&plan.uefi_quirks, "DisableSecurityPolicy"));
        assert!(!on(&plan.kernel_quirks, "LapicKernelPanic"));
        let desk = desktop(P::KabyLake, "x", "Kaby Lake-S", 4);
        let plan = run(&desk, Ventura);
        assert!(!on(&plan.uefi_quirks, "ReleaseUsbOwnership"));
        assert!(!on(&plan.kernel_quirks, "CustomSMBIOSGuid"));
        assert!(on(&plan.uefi_quirks, "EnableVectorAcceleration"));
    }

    #[test]
    fn cpuid_spoofs() {
        let spoof = |p: &HardwareProfile| data_of(&run(p, Sequoia).kernel_emulate, "Cpuid1Data");
        let mask = "FFFFFFFF000000000000000000000000".to_string();
        let mut hsw_e = desktop(P::HaswellE, "x", "Haswell-E", 8);
        assert_eq!(
            spoof(&hsw_e).as_deref(),
            Some("C3060300000000000000000000000000")
        );
        assert_eq!(
            data_of(&run(&hsw_e, Sequoia).kernel_emulate, "Cpuid1Mask"),
            Some(mask.clone())
        );
        hsw_e.cpu.platform = P::BroadwellE;
        assert_eq!(
            spoof(&hsw_e).as_deref(),
            Some("D4060300000000000000000000000000")
        );
        for platform in [P::RocketLake, P::AlderLake, P::RaptorLake, P::ArrowLake] {
            let p = desktop(platform, "x", "x", 8);
            assert_eq!(
                spoof(&p).as_deref(),
                Some("55060A00000000000000000000000000"),
                "{platform:?}"
            );
        }
        let mut u62 = laptop(
            P::CometLake,
            "Intel(R) Core(TM) i5-10210U",
            "Comet Lake-U",
            4,
        );
        assert_eq!(spoof(&u62).as_deref(), Some(""));
        u62.cpu.model = Some(0xA6);
        assert_eq!(
            spoof(&u62).as_deref(),
            Some("EC060800000000000000000000000000")
        );
        let icl = laptop(P::IceLake, "Intel(R) Core(TM) i5-1035G7", "Ice Lake-U", 4);
        assert_eq!(spoof(&icl).as_deref(), Some(""));
        let pentium = desktop(
            P::CoffeeLake,
            "Intel(R) Pentium(R) Gold G5400",
            "Coffee Lake-S",
            2,
        );
        assert_eq!(spoof(&pentium).as_deref(), Some(""));
    }

    #[test]
    fn aquantia_keeps_vtd() {
        let mut p = desktop(
            P::CoffeeLake,
            "Intel(R) Core(TM) i9-9900K",
            "Coffee Lake-S",
            8,
        );
        p.ethernet = vec![nic("1d6a", "07b1")];
        p.ram_gb = 32;
        let plan = run(&p, Sequoia);
        let k = &plan.kernel_quirks;
        assert!(on(k, "ForceAquantiaEthernet"));
        assert!(!on(k, "DisableIoMapper"));
        assert!(!on(k, "DisableIoMapperMapping"), "no replacement DMAR");
        assert!(plan
            .notes
            .iter()
            .any(|n| n.component == "ethernet" && n.detail.contains("Reserved Memory")));
        let ctx_o = options(Sequoia);
        let ctx = PlanContext::new(&p, &ctx_o);
        let mut plan = empty_plan(Sequoia);
        plan.acpi_deletes.push(AcpiDelete {
            comment: "Drop DMAR".into(),
            table_signature: "DMAR".into(),
            oem_table_id: String::new(),
            all: false,
        });
        apply(&ctx, &mut plan);
        assert!(on(&plan.kernel_quirks, "DisableIoMapperMapping"));
        assert!(
            !on(&run(&p, Mojave).kernel_quirks, "ForceAquantiaEthernet"),
            "needs 10.15.4"
        );
    }

    #[test]
    fn samsung_nvme_trim() {
        use crate::domain::model::{ProfileStorage, StorageKind};
        let mut p = desktop(P::CoffeeLake, "x", "Coffee Lake-S", 6);
        assert_eq!(
            int_of(&run(&p, Sequoia).kernel_quirks, "SetApfsTrimTimeout"),
            Some(-1)
        );
        p.storage = vec![ProfileStorage {
            name: "Samsung SSD 970 EVO Plus 1TB".into(),
            kind: StorageKind::Nvme,
            vendor_id: Some("144d".into()),
            device_id: Some("a808".into()),
            size_bytes: None,
        }];
        assert_eq!(
            int_of(&run(&p, Sequoia).kernel_quirks, "SetApfsTrimTimeout"),
            Some(0)
        );
        assert_eq!(
            int_of(&run(&p, BigSur).kernel_quirks, "SetApfsTrimTimeout"),
            Some(-1)
        );
    }

    #[test]
    fn virtual_machine_quirks() {
        let p = vm_profile(P::Unknown, VmKind::Kvm);
        let plan = run(&p, Sequoia);
        let k = &plan.kernel_quirks;
        assert!(on(k, "ProvideCurrentCpuInfo") && on(k, "ForceSecureBootScheme"));
        assert!(!on(k, "DisableIoMapper") && !on(k, "XhciPortLimit"));
        assert!(!on(k, "AppleXcpmCfgLock"));
        assert!(on(&plan.kernel_emulate, "DummyPowerManagement"));
        let b = &plan.booter_quirks;
        assert!(
            !on(b, "SetupVirtualMap")
                && !on(b, "ProvideCustomSlide")
                && !on(b, "EnableSafeModeSlide")
        );
        let p = vm_profile(P::Unknown, VmKind::Vmware);
        assert!(on(&run(&p, Sequoia).booter_quirks, "SetupVirtualMap"));
    }

    /// Every key exists in Sample.plist with the same type, for every
    /// supported platform, form factor and release.
    #[test]
    fn keys_match_sample_plist() {
        for &platform in cpu_db::all_platforms() {
            if !cpu_db::platform_info(platform).supported {
                continue;
            }
            for form in [FormFactor::Desktop, FormFactor::Laptop] {
                let p = profile(cpu(platform, "x", "x", 4), form, vec![]);
                for target in [HighSierra, Monterey, Tahoe] {
                    let plan = run(&p, target);
                    assert_schema("Booter/Quirks", &plan.booter_quirks);
                    assert_schema("Kernel/Quirks", &plan.kernel_quirks);
                    assert_schema("Kernel/Emulate", &plan.kernel_emulate);
                    assert_schema("UEFI/Quirks", &plan.uefi_quirks);
                    assert!(!plan.booter_quirks.is_empty() && !plan.kernel_quirks.is_empty());
                }
            }
        }
    }
}
