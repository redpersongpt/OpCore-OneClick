//! Firmware (BIOS/UEFI) settings the user must apply before booting OpenCore.
//!
//! The lists follow Dortania's "Intel BIOS settings" and "AMD BIOS settings"
//! (research-intel-desktop §1.2, research-intel-laptop §2.7, research-amd
//! §14). `required` marks settings without an OpenCore-side workaround; the
//! rest are recommendations whose reason names the fallback. Menu locations
//! are only given for vendors whose setup layout is stable across boards.

use super::compatibility::{self, DisplayPath, ModernStandby};
use super::model::{
    BiosSetting, CpuPlatform, CpuVendor, FormFactor, GpuFamily, HardwareProfile, MacOsVersion,
    ProfileGpu, StorageKind, VmKind,
};
use super::{chipset_db, cpu_db, device_db, gpu_db};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    BootMode,
    Csm,
    SecureBoot,
    FastBoot,
    OsType,
    SataMode,
    Vmd,
    XhciHandoff,
    Above4g,
    ReBar,
    CfgLock,
    VtD,
    VtX,
    Svm,
    Iommu,
    HyperThreading,
    ExecuteDisable,
    EfficientCores,
    Sgx,
    Ptt,
    Dvmt,
    UmaFrameBuffer,
    IgpuMultiMonitor,
    PrimaryDisplay,
    InternalGraphics,
    DiscreteGpu,
    SerialPort,
    Thunderbolt,
    SleepState,
    MmioHigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoardVendor {
    Asus,
    Gigabyte,
    Msi,
    Asrock,
    Dell,
    Hp,
    Lenovo,
    Other,
}

/// ThinkPad machine types are 20xx/21xx ("20HRCTO1WW"); IdeaPad, Yoga and
/// Legion boards report "LNVNB..." and use a different (Insyde) setup.
fn is_thinkpad(model: &str) -> bool {
    let m = model.trim();
    m.to_ascii_lowercase().contains("thinkpad")
        || ((m.starts_with("20") || m.starts_with("21"))
            && (4..=10).contains(&m.len())
            && m.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn board_vendor(profile: &HardwareProfile) -> BoardVendor {
    let v = profile.motherboard_vendor.trim().to_ascii_lowercase();
    if v.contains("asus") {
        BoardVendor::Asus
    } else if v.contains("gigabyte") {
        BoardVendor::Gigabyte
    } else if v.contains("micro-star") || v.split_whitespace().next() == Some("msi") {
        BoardVendor::Msi
    } else if v.contains("asrock") {
        BoardVendor::Asrock
    } else if v.contains("dell") {
        // Alienware keeps its own AMI-style setup, not Dell's.
        BoardVendor::Dell
    } else if v.contains("hewlett") || v == "hp" || v.starts_with("hp ") {
        BoardVendor::Hp
    } else if v.contains("lenovo") && is_thinkpad(&profile.motherboard_model) {
        BoardVendor::Lenovo
    } else {
        BoardVendor::Other
    }
}

/// Usual menu path. Retail boards (ASUS, Gigabyte, MSI, ASRock) only for
/// desktops (their laptops use reduced setups), Lenovo only for ThinkPads,
/// HP with the business (EliteBook/ProBook/Z) and consumer layouts named.
fn hint(vendor: BoardVendor, key: Key, amd: bool, laptop: bool) -> Option<&'static str> {
    use BoardVendor as V;
    use Key as K;
    let retail = matches!(vendor, V::Asus | V::Gigabyte | V::Msi | V::Asrock);
    if (retail && laptop) || (vendor == V::Lenovo && !laptop) {
        return None;
    }
    Some(match (vendor, key) {
        (V::Asus, K::FastBoot) => "Boot > Boot Configuration > Fast Boot",
        (V::Asus, K::SecureBoot | K::OsType) => "Boot > Secure Boot > OS Type",
        (V::Asus, K::Csm) => "Boot > CSM (Compatibility Support Module) > Launch CSM",
        (V::Asus, K::Above4g | K::ReBar) => "Advanced > PCI Subsystem Settings",
        (V::Asus, K::XhciHandoff) => "Advanced > USB Configuration",
        (V::Asus, K::SataMode) if amd => "Advanced > SATA Configuration",
        (V::Asus, K::SataMode) => "Advanced > PCH Storage Configuration > SATA Mode Selection",
        (V::Asus, K::VtD) => "Advanced > System Agent (SA) Configuration > VT-d",
        (V::Asus, K::Dvmt | K::IgpuMultiMonitor | K::PrimaryDisplay) if !amd => {
            "Advanced > System Agent (SA) Configuration > Graphics Configuration"
        }
        (V::Asus, K::VtX | K::Sgx | K::HyperThreading) => "Advanced > CPU Configuration",
        (V::Asus, K::Svm) => "Advanced > CPU Configuration > SVM Mode",
        (V::Asus, K::Iommu) => "Advanced > AMD CBS > NBIO Common Options > IOMMU",
        (V::Asus, K::Ptt) => "Advanced > PCH-FW Configuration > PTT",
        (V::Asus, K::SerialPort) => "Advanced > Onboard Devices Configuration",
        (V::Gigabyte, K::FastBoot) => "Boot > Fast Boot",
        (V::Gigabyte, K::SecureBoot) => "Boot > Secure Boot",
        (V::Gigabyte, K::Csm) => "Boot > CSM Support",
        (V::Gigabyte, K::Above4g | K::ReBar) => "Settings > IO Ports",
        (V::Gigabyte, K::XhciHandoff) => "Settings > IO Ports > USB Configuration",
        (V::Gigabyte, K::SataMode) => "Settings > IO Ports > SATA Configuration",
        (V::Gigabyte, K::Dvmt | K::IgpuMultiMonitor | K::PrimaryDisplay | K::InternalGraphics) => {
            "Settings > IO Ports > Internal Graphics"
        }
        (V::Gigabyte, K::VtD | K::Iommu | K::Ptt | K::Sgx) => "Settings > Miscellaneous",
        (V::Gigabyte, K::VtX | K::Svm | K::HyperThreading) => "Tweaker > Advanced CPU Settings",
        (V::Msi, K::FastBoot) => "Settings > Boot > Fast Boot",
        (V::Msi, K::Csm | K::OsType) => "Settings > Advanced > Windows OS Configuration",
        (V::Msi, K::SecureBoot) => "Settings > Security > Secure Boot",
        (V::Msi, K::Above4g | K::ReBar) => "Settings > Advanced > PCI Subsystem Settings",
        (V::Msi, K::XhciHandoff) => "Settings > Advanced > USB Configuration",
        (V::Msi, K::SataMode) => "Settings > Advanced > Integrated Peripherals",
        (V::Msi, K::Dvmt | K::IgpuMultiMonitor | K::PrimaryDisplay | K::InternalGraphics) => {
            "Settings > Advanced > Integrated Graphics Configuration"
        }
        (V::Msi, K::Svm | K::HyperThreading) if amd => "OC > Advanced CPU Configuration",
        (V::Msi, K::Iommu) => "Settings > Advanced > AMD CBS > NBIO Common Options",
        (V::Msi, K::VtX | K::VtD | K::HyperThreading) => "OC > CPU Features",
        (V::Msi, K::Ptt) => "Settings > Security > Trusted Computing",
        (V::Msi, K::Sgx) => "OC > CPU Features",
        (V::Msi, K::SerialPort) => "Settings > Advanced > Super IO Configuration",
        (V::Asrock, K::FastBoot) => "Boot > Fast Boot",
        (V::Asrock, K::Csm) => "Boot > CSM (Compatibility Support Module)",
        (V::Asrock, K::SecureBoot) => "Security > Secure Boot",
        (
            V::Asrock,
            K::Above4g
            | K::ReBar
            | K::VtD
            | K::Dvmt
            | K::IgpuMultiMonitor
            | K::PrimaryDisplay
            | K::InternalGraphics,
        ) => "Advanced > Chipset Configuration",
        (V::Asrock, K::SataMode) => "Advanced > Storage Configuration",
        (V::Asrock, K::XhciHandoff) => "Advanced > USB Configuration",
        (V::Asrock, K::VtX | K::Svm | K::HyperThreading | K::Sgx) => "Advanced > CPU Configuration",
        (V::Asrock, K::Iommu) => "Advanced > AMD CBS > NBIO Common Options",
        (V::Dell, K::SecureBoot) => "Secure Boot > Secure Boot Enable",
        (V::Dell, K::SataMode | K::Vmd) => "System Configuration > SATA Operation",
        (V::Dell, K::VtD) => "Virtualization Support > VT for Direct I/O",
        (V::Dell, K::VtX) => "Virtualization Support > Virtualization",
        (V::Dell, K::FastBoot) => "POST Behavior > Fastboot",
        (V::Dell, K::Csm) => "General > Advanced Boot Options",
        (V::Dell, K::Sgx) => "Intel Software Guard Extensions",
        (V::Dell, K::Thunderbolt) => "System Configuration > Thunderbolt Adapter Configuration",
        (V::Hp, K::SecureBoot | K::Csm) => {
            "Advanced > Secure Boot Configuration (business models) or System Configuration > Boot Options \
             (consumer models)"
        }
        (V::Hp, K::VtD | K::VtX) => {
            "Advanced > System Options (business models) or System Configuration (consumer models)"
        }
        (V::Lenovo, K::SecureBoot) => "Security > Secure Boot",
        (V::Lenovo, K::VtD | K::VtX) => "Security > Virtualization",
        (V::Lenovo, K::Csm) => "Startup > UEFI/Legacy Boot",
        (V::Lenovo, K::Sgx) => "Security > Intel SGX",
        (V::Lenovo, K::Ptt) => "Security > Security Chip",
        (V::Lenovo, K::SleepState) => "Config > Power > Sleep State",
        (V::Lenovo, K::Thunderbolt) => "Config > Thunderbolt(TM) 3",
        (V::Lenovo, K::Dvmt) => "Config > Display > Total Graphics Memory",
        (V::Lenovo, K::SataMode | K::Vmd) => "Config > Storage > Controller Mode",
        _ => return None,
    })
}

struct Checklist {
    vendor: BoardVendor,
    amd: bool,
    laptop: bool,
    items: Vec<BiosSetting>,
}

impl Checklist {
    fn add(
        &mut self,
        key: Key,
        name: &str,
        value: &str,
        required: bool,
        reason: impl Into<String>,
    ) {
        self.items.push(BiosSetting {
            name: name.into(),
            value: value.into(),
            required,
            reason: reason.into(),
            location_hint: hint(self.vendor, key, self.amd, self.laptop).map(Into::into),
        });
    }
}

fn is_legacy_era(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::Penryn
            | CpuPlatform::Lynnfield
            | CpuPlatform::Arrandale
            | CpuPlatform::NehalemHedt
    )
}

/// Platforms before Haswell use AppleIntelCPUPowerManagement (AppleCpuPmCfgLock)
/// instead of XCPM (AppleXcpmCfgLock).
fn pre_xcpm(platform: CpuPlatform) -> bool {
    matches!(
        platform,
        CpuPlatform::Lynnfield
            | CpuPlatform::Arrandale
            | CpuPlatform::SandyBridge
            | CpuPlatform::IvyBridge
            | CpuPlatform::NehalemHedt
            | CpuPlatform::SandyBridgeE
            | CpuPlatform::IvyBridgeE
    )
}

fn at_least_skylake(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::Skylake
            | P::KabyLake
            | P::CoffeeLake
            | P::CometLake
            | P::IceLake
            | P::RocketLake
            | P::TigerLake
            | P::AlderLake
            | P::RaptorLake
            | P::MeteorLake
            | P::ArrowLake
            | P::LunarLake
            | P::SkylakeX
            | P::CascadeLakeX
    )
}

fn has_sgx(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::Skylake
            | P::KabyLake
            | P::CoffeeLake
            | P::CometLake
            | P::IceLake
            | P::RocketLake
            | P::TigerLake
            | P::SkylakeX
            | P::CascadeLakeX
    )
}

/// Platforms whose boards may expose VMD (11th gen and newer, Ice Lake).
fn may_have_vmd(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::IceLake
            | P::TigerLake
            | P::RocketLake
            | P::AlderLake
            | P::RaptorLake
            | P::MeteorLake
            | P::ArrowLake
    )
}

/// Platforms whose boards commonly offer Resizable BAR.
fn may_have_rebar(platform: CpuPlatform) -> bool {
    use CpuPlatform as P;
    matches!(
        platform,
        P::CometLake
            | P::RocketLake
            | P::AlderLake
            | P::RaptorLake
            | P::ArrowLake
            | P::CascadeLakeX
            | P::AmdZen2
            | P::AmdZen3
            | P::AmdZen4
            | P::AmdZen5
    )
}

/// DVMT pre-allocated minimum for an Intel iGPU that drives a display
/// (research-intel-laptop §2.7).
fn dvmt(family: GpuFamily) -> &'static str {
    match family {
        GpuFamily::IntelIronLake | GpuFamily::IntelSandyBridge | GpuFamily::IntelIvyBridge => {
            "32MB"
        }
        GpuFamily::IntelIceLake => "256MB",
        _ => "64MB",
    }
}

fn is_intel_igpu_family(family: GpuFamily) -> bool {
    use GpuFamily::*;
    matches!(
        family,
        IntelIronLake
            | IntelSandyBridge
            | IntelIvyBridge
            | IntelHaswell
            | IntelBroadwell
            | IntelSkylake
            | IntelKabyLake
            | IntelCoffeeLake
            | IntelCometLake
            | IntelIceLake
    )
}

fn vmd_detected(profile: &HardwareProfile) -> bool {
    profile.storage.iter().any(|d| {
        let ids = d
            .vendor_id
            .as_deref()
            .and_then(device_db::parse_id16)
            .zip(d.device_id.as_deref().and_then(device_db::parse_id16));
        ids.is_some_and(|(v, dev)| device_db::is_intel_vmd(v, dev))
    })
}

fn vm_settings(kind: VmKind) -> Vec<BiosSetting> {
    let setting = |name: &str, value: &str, reason: &str| BiosSetting {
        name: name.into(),
        value: value.into(),
        required: true,
        reason: reason.into(),
        location_hint: None,
    };
    let mut out = vec![
        setting(
            "VM firmware",
            "UEFI",
            "OpenCore boots as a UEFI application (OVMF on QEMU/KVM).",
        ),
        setting(
            "Secure Boot (VM firmware)",
            "disable",
            "OpenCore is not signed with Microsoft's keys.",
        ),
    ];
    match kind {
        VmKind::Kvm => out.push(setting(
            "CPU model",
            "host (Intel) or a Haswell/Skylake-class model",
            "macOS needs a CPU model it knows; AMD hosts must present an Intel model with AVX2 for macOS 13+.",
        )),
        VmKind::HyperV => out.push(setting(
            "VM generation",
            "Generation 2",
            "MacHyperVSupport targets Generation 2 (UEFI) VMs.",
        )),
        _ => {}
    }
    out
}

/// Platform-specific BIOS checklist (Dortania "Intel BIOS settings" / "AMD BIOS
/// settings"): Fast Boot, Secure Boot, CSM, VT-d (or DisableIoMapper), CFG Lock,
/// Above 4G Decoding, Resizable BAR, XHCI hand-off, SATA AHCI, DVMT
/// pre-allocated, Intel SGX, Platform Trust, Serial port, SVM/IOMMU on AMD,
/// VMD off, iGPU multi-monitor for headless setups, Thunderbolt, etc.
pub fn recommended_settings(profile: &HardwareProfile, target: MacOsVersion) -> Vec<BiosSetting> {
    let platform = profile.cpu.platform;
    if platform == CpuPlatform::AppleSilicon || profile.cpu.vendor == CpuVendor::Apple {
        return Vec::new();
    }
    if let Some(kind) = profile.vm {
        return vm_settings(kind);
    }
    let info = cpu_db::platform_info(platform);
    let vendor = match profile.cpu.vendor {
        CpuVendor::Unknown => info.vendor,
        v => v,
    };
    let amd = vendor == CpuVendor::Amd;
    let intel = vendor == CpuVendor::Intel;
    let laptop = profile.form_factor == FormFactor::Laptop;
    let desktop = !laptop;
    let chipset = profile
        .chipset
        .as_deref()
        .and_then(|c| chipset_db::from_name(c).or_else(|| chipset_db::from_board_name(c)))
        .or_else(|| chipset_db::from_board_name(&profile.motherboard_model));
    let am5 = chipset.as_ref().is_some_and(|c| c.is_am5())
        || (matches!(platform, CpuPlatform::AmdZen4 | CpuPlatform::AmdZen5)
            && desktop
            && !profile.cpu.is_mobile);
    let threadripper = amd
        && (cpu_db::is_hedt(&compatibility::cpu_identity(profile))
            || chipset
                .as_ref()
                .is_some_and(|c| c.is_hedt && c.vendor == CpuVendor::Amd));
    let legacy_only = compatibility::legacy_boot_only(profile);
    let display = compatibility::display_path(profile, target);
    let display_gpu = match display {
        DisplayPath::Native(i) | DisplayPath::RootPatch(i) => profile.gpus.get(i),
        _ => None,
    };
    let mut list = Checklist {
        vendor: board_vendor(profile),
        amd,
        laptop,
        items: Vec::new(),
    };

    // Boot
    if legacy_only {
        list.add(
            Key::BootMode,
            "Boot mode",
            "Legacy BIOS",
            true,
            "This board appears to have no UEFI. OpenCore then starts through OpenDuet: the legacy boot sector from \
             OpenCore's Utilities/LegacyBoot must be written to the USB drive. If the setup does offer UEFI boot, \
             use it and turn CSM off instead.",
        );
    } else {
        let mut reason = String::from(
            "OpenCore needs pure UEFI boot with a GOP; with CSM on, GPU stalls (gIO) are common (Dortania).",
        );
        if laptop {
            reason
                .push_str(" Only if a laptop shows a scrambled screen with CSM off, leave it on.");
        }
        list.add(
            Key::Csm,
            "CSM (Compatibility Support Module)",
            "disable",
            true,
            reason,
        );
        list.add(
            Key::SecureBoot,
            "Secure Boot",
            "disable",
            true,
            "OpenCore is not signed with Microsoft's keys. Turning it off makes Windows BitLocker / Device \
             Encryption ask for its recovery key: save the key and suspend BitLocker first (manage-bde -protectors \
             -disable C: -RebootCount 0).",
        );
    }
    list.add(
        Key::FastBoot,
        "Fast Boot",
        "disable",
        true,
        "Fast Boot can skip USB initialisation, so the installer drive may not appear in the boot menu.",
    );
    if desktop && !legacy_only {
        list.add(
            Key::OsType,
            "OS Type",
            "Windows 8.1/10 UEFI Mode (or Other OS)",
            false,
            "Some boards tie Secure Boot and the UEFI CSM policy to this option.",
        );
    }

    // Storage and USB
    let raid = profile.storage.iter().any(|d| d.kind == StorageKind::Raid);
    let mut sata_reason = String::from(
        "macOS has no driver for RAID / Intel RST mode. Windows installed in RAID mode must be switched to the AHCI \
         driver first (one boot in Safe Mode) or it will not start.",
    );
    if raid {
        sata_reason.insert_str(0, "The storage controller currently runs in RAID mode. ");
    }
    list.add(Key::SataMode, "SATA Mode", "AHCI", true, sata_reason);
    let vmd = vmd_detected(profile);
    if vmd || (intel && may_have_vmd(platform)) {
        let mut reason = String::from(
            "Intel VMD hides NVMe drives from macOS. Windows installed behind VMD needs its storage driver switched \
             before VMD is turned off.",
        );
        if vmd {
            reason.insert_str(0, "VMD is enabled on this machine. ");
        }
        list.add(Key::Vmd, "Intel VMD controller", "disable", vmd, reason);
    }
    list.add(
        Key::XhciHandoff,
        "EHCI/XHCI Hand-off",
        "enable",
        true,
        "Hands the USB controllers over to the OS; without it USB ports can stop working in macOS.",
    );

    // PCI resources
    if !is_legacy_era(platform) {
        let mut reason = String::from(
            "Lets the firmware map large GPU memory windows. If the option does not exist, add npci=0x2000 (or \
             0x3000) to boot-args instead; never both.",
        );
        if amd && matches!(list.vendor, BoardVendor::Gigabyte | BoardVendor::Asrock) {
            reason.push_str(
                " On some Gigabyte and ASRock AMD boards it breaks onboard Ethernet: test it.",
            );
        }
        list.add(
            Key::Above4g,
            "Above 4G Decoding",
            "enable",
            am5 || threadripper,
            reason,
        );
    }
    if may_have_rebar(platform) {
        let mut reason = String::from(
            "macOS does not use Resizable BAR. If Windows needs it, leave it on and set Booter > Quirks > \
             ResizeAppleGpuBars to 0 in config.plist.",
        );
        if am5 && list.vendor == BoardVendor::Asus {
            reason.push_str(" ASUS AM5 boards enable it by default.");
        }
        list.add(
            Key::ReBar,
            "Resizable BAR (Re-Size BAR Support)",
            "disable",
            false,
            reason,
        );
    }

    // CPU
    if intel {
        if platform != CpuPlatform::Penryn {
            let quirk = if pre_xcpm(platform) {
                "AppleCpuPmCfgLock"
            } else {
                "AppleXcpmCfgLock"
            };
            list.add(
                Key::CfgLock,
                "CFG Lock (MSR 0xE2 write protection)",
                "disable",
                false,
                format!(
                    "macOS writes MSR 0xE2 for power management. If the option is hidden or locked, OpenCore's \
                     {quirk} quirk works around it; the ControlMsrE2 tool shows the current state."
                ),
            );
        }
        list.add(
            Key::VtD,
            "VT-d (Intel Virtualization for Directed I/O)",
            "disable",
            false,
            "Or leave it enabled and rely on OpenCore's DisableIoMapper quirk.",
        );
        list.add(
            Key::VtX,
            "VT-x (Intel Virtualization Technology)",
            "enable",
            false,
            "Needed by virtualisation apps; harmless for macOS.",
        );
        list.add(
            Key::ExecuteDisable,
            "Execute Disable Bit",
            "enable",
            true,
            "macOS requires the NX/XD bit.",
        );
        if profile.cpu.is_hybrid {
            list.add(
                Key::EfficientCores,
                "Efficient cores (E-cores)",
                "enable",
                false,
                "Both work: with E-cores on, macOS gets more threads but does not tell P- and E-cores apart; turning \
                 them off raises the ring clock and simplifies scheduling.",
            );
        }
        if has_sgx(platform) {
            list.add(
                Key::Sgx,
                "Intel SGX (Software Guard Extensions)",
                "disable",
                false,
                "Listed as off in the Dortania guide; macOS does not use it.",
            );
        }
        if at_least_skylake(platform)
            || matches!(platform, CpuPlatform::Haswell | CpuPlatform::Broadwell)
        {
            list.add(
                Key::Ptt,
                "Intel Platform Trust Technology (PTT)",
                "disable",
                false,
                "Listed as off in the Dortania guide. Windows 11 needs the TPM, and with BitLocker on, disabling it \
                 triggers BitLocker recovery: suspend BitLocker first, or leave it on if macOS boots fine.",
            );
        }
    }
    if amd {
        list.add(
            Key::Svm,
            "SVM Mode (AMD-V)",
            "enable",
            false,
            "Only needed for virtual machines in other systems; macOS hypervisor apps do not support AMD. Harmless.",
        );
        list.add(
            Key::Iommu,
            "IOMMU (AMD-Vi)",
            "disable",
            true,
            "Listed as off in the Dortania AMD guide; macOS has no AMD IOMMU support.",
        );
    }
    if profile.cpu.threads > 64 {
        list.add(
            Key::HyperThreading,
            if amd { "SMT" } else { "Hyper-Threading" },
            "disable",
            true,
            format!(
                "macOS handles at most 64 threads; this CPU has {}.",
                profile.cpu.threads
            ),
        );
    } else if intel {
        list.add(
            Key::HyperThreading,
            "Hyper-Threading",
            "enable",
            false,
            "macOS uses all threads.",
        );
    }

    // Graphics
    let display_is_igpu = display_gpu.is_some_and(|g| g.is_igpu);
    if let Some(g) = display_gpu.filter(|g| display_is_igpu && is_intel_igpu_family(g.family)) {
        let mut reason = format!(
            "The iGPU drives the display; its framebuffer needs at least {}. If the option is missing or locked \
             lower, the framebuffer memory patches in the config take over.",
            dvmt(g.family)
        );
        if list.vendor == BoardVendor::Dell {
            reason.push_str(" Some Dell firmware reports 64MB but allocates only 32MB.");
        }
        list.add(
            Key::Dvmt,
            "DVMT Pre-Allocated",
            dvmt(g.family),
            g.family == GpuFamily::IntelIceLake,
            reason,
        );
    }
    if display_gpu.is_some_and(|g| g.family == GpuFamily::AmdApuVega) {
        list.add(
            Key::UmaFrameBuffer,
            "UMA Frame Buffer Size",
            "512MB (1GB recommended)",
            true,
            "NootedRed needs at least 512MB of carved-out video memory. Some laptops hide the option; then the \
             default has to do.",
        );
    }
    if desktop && !display_is_igpu && display_gpu.is_some() {
        // An Intel iGPU inside its native range stays on headless (Quick
        // Sync); one listed nowhere is usually switched off in the firmware
        // (not on F-series CPUs).
        let in_native_range = |g: &ProfileGpu| {
            let s = gpu_db::support(g);
            s.min_native.is_some_and(|min| target >= min)
                && s.max_native.is_none_or(|max| target <= max)
        };
        let headless_igpu = profile.gpus.iter().any(|g| {
            g.is_igpu && !g.disabled && is_intel_igpu_family(g.family) && in_native_range(g)
        });
        let amd_apu = profile.gpus.iter().any(|g| {
            g.is_igpu
                && matches!(
                    g.family,
                    GpuFamily::AmdApuVega | GpuFamily::AmdApuRdna | GpuFamily::AmdApuLegacy
                )
        });
        let unsupported_igpu = profile.gpus.iter().any(|g| {
            g.is_igpu
                && !gpu_db::support(g).display_capable
                && gpu_db::support(g).min_native.is_none()
        });
        if headless_igpu || (intel && compatibility::igpu_may_be_disabled(profile)) {
            list.add(
                Key::IgpuMultiMonitor,
                "iGPU Multi-Monitor",
                "enable",
                false,
                "Keeps the iGPU enabled next to the dGPU so macOS can use it headless for Quick Sync and hardware \
                 video decoding. Not available on F-series CPUs.",
            );
            list.add(
                Key::PrimaryDisplay,
                "Primary Display / Initiate Graphic Adapter",
                "PEG (PCIe)",
                false,
                "The dGPU drives the displays, so the firmware should initialise it first.",
            );
        } else if amd_apu {
            // research-amd §11/§13: NootedRed cannot share the system with the
            // dGPU's driver, so the build disables the APU graphics.
            list.add(
                Key::InternalGraphics,
                "Integrated Graphics",
                "disable",
                false,
                "The dGPU drives the displays and the APU graphics cannot run next to it under macOS; the build \
                 disables them, and turning them off in the firmware also frees their memory.",
            );
        } else if unsupported_igpu {
            list.add(
                Key::InternalGraphics,
                "Internal Graphics",
                "disable",
                false,
                "The iGPU has no macOS driver; with the dGPU driving the displays it is not needed.",
            );
        }
    }
    if laptop {
        let hidden_dgpu = (0..profile.gpus.len())
            .any(|i| !profile.gpus[i].is_igpu && !compatibility::can_drive_display(profile, i));
        if hidden_dgpu {
            list.add(
                Key::DiscreteGpu,
                "Discrete graphics / Graphics mode",
                "disable (Integrated only)",
                false,
                "macOS cannot use the switchable dGPU; turning it off in the firmware saves power. Without the option \
                 it stays powered unless ACPI turns it off.",
            );
        }
    }

    // Devices
    if desktop {
        list.add(
            Key::SerialPort,
            "Serial / Parallel port",
            "disable",
            false,
            "Listed as off in the Dortania guide; legacy ports can stall early boot.",
        );
    }
    if (intel && at_least_skylake(platform)) || am5 {
        list.add(
            Key::Thunderbolt,
            "Thunderbolt / USB4",
            "disable for the install",
            false,
            "Thunderbolt can cause problems until it is set up (Dortania); enable it again after installing.",
        );
    }
    let standby = compatibility::modern_standby(profile);
    if standby != ModernStandby::Unlikely {
        let lenovo = list.vendor == BoardVendor::Lenovo;
        list.add(
            Key::SleepState,
            "Sleep State",
            if lenovo { "Linux (S3)" } else { "S3 if available" },
            false,
            "macOS sleep needs ACPI S3; laptops limited to Modern Standby (S0ix) cannot sleep under macOS.",
        );
    }
    // Dortania X99 and X299 pages (research-intel-desktop §4.3, §4.5).
    if matches!(
        platform,
        CpuPlatform::HaswellE
            | CpuPlatform::BroadwellE
            | CpuPlatform::SkylakeX
            | CpuPlatform::CascadeLakeX
    ) {
        list.add(
            Key::MmioHigh,
            "MMIOH Base / MMIO High Base",
            "12T or lower",
            false,
            "Only if the boot hangs at PCI configuration (Dortania X99/X299 notes).",
        );
    }
    list.items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::ProfileStorage;
    use crate::domain::profile::{build_profile, fixtures};

    fn find<'a>(list: &'a [BiosSetting], name: &str) -> Option<&'a BiosSetting> {
        list.iter().find(|s| s.name.starts_with(name))
    }

    #[test]
    fn asus_z390_with_dgpu() {
        let p = build_profile(&fixtures::windows_z390());
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        let secure = find(&list, "Secure Boot").expect("secure boot");
        assert!(secure.required && secure.value == "disable");
        assert!(secure.reason.contains("BitLocker"));
        assert_eq!(
            secure.location_hint.as_deref(),
            Some("Boot > Secure Boot > OS Type")
        );
        assert!(find(&list, "CSM").is_some_and(|s| s.required));
        assert!(find(&list, "CFG Lock").is_some_and(|s| s.reason.contains("AppleXcpmCfgLock")));
        assert!(find(&list, "VT-d").is_some_and(|s| s.reason.contains("DisableIoMapper")));
        assert!(find(&list, "SATA Mode").is_some_and(|s| s.value == "AHCI" && s.required));
        assert!(find(&list, "EHCI/XHCI Hand-off").is_some());
        assert!(find(&list, "Above 4G").is_some_and(|s| !s.required));
        assert!(find(&list, "Intel SGX").is_some());
        assert!(find(&list, "Intel Platform Trust").is_some());
        // RX 580 drives the displays, UHD 630 stays on headless.
        assert!(find(&list, "iGPU Multi-Monitor").is_some_and(|s| s.value == "enable"));
        assert!(find(&list, "DVMT").is_none());
        assert!(find(&list, "SVM").is_none() && find(&list, "IOMMU").is_none());
        assert!(find(&list, "Thunderbolt").is_some());
        assert!(find(&list, "Intel VMD").is_none());
        assert!(
            find(&list, "Resizable BAR").is_none(),
            "Coffee Lake boards have no ReBAR"
        );
    }

    #[test]
    fn whiskey_lake_laptop() {
        let p = build_profile(&fixtures::linux_laptop_i2c());
        let list = recommended_settings(&p, MacOsVersion::Sonoma);
        let dvmt = find(&list, "DVMT").expect("dvmt");
        assert_eq!(dvmt.value, "64MB");
        assert!(dvmt.reason.contains("Dell"));
        assert_eq!(
            find(&list, "SATA Mode").and_then(|s| s.location_hint.as_deref()),
            Some("System Configuration > SATA Operation")
        );
        assert!(find(&list, "Serial").is_none());
        assert!(find(&list, "OS Type").is_none());
        assert!(find(&list, "CSM").is_some_and(|s| s.reason.contains("scrambled")));
        assert!(find(&list, "Sleep State").is_some());
    }

    #[test]
    fn amd_b550() {
        let p = build_profile(&fixtures::amd_b550());
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert!(find(&list, "SVM Mode").is_some_and(|s| s.value == "enable" && !s.required));
        assert!(find(&list, "IOMMU").is_some_and(|s| s.value == "disable"));
        assert!(find(&list, "CFG Lock").is_none() && find(&list, "VT-d").is_none());
        assert!(
            find(&list, "Resizable BAR").is_some_and(|s| s.reason.contains("ResizeAppleGpuBars"))
        );
        let above = find(&list, "Above 4G").expect("above 4g");
        assert!(!above.required, "B550 is not AM5");
        assert_eq!(
            above.location_hint.as_deref(),
            Some("Settings > Advanced > PCI Subsystem Settings")
        );
        assert!(find(&list, "Thunderbolt").is_none());
        assert!(find(&list, "Hyper-Threading").is_none());
    }

    #[test]
    fn am5_needs_above_4g() {
        let mut p = build_profile(&fixtures::amd_b550());
        p.cpu.platform = CpuPlatform::AmdZen4;
        p.motherboard_vendor = "ASUSTeK COMPUTER INC.".into();
        p.motherboard_model = "ROG STRIX X670E-E GAMING WIFI".into();
        p.chipset = Some("X670E".into());
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert!(find(&list, "Above 4G").is_some_and(|s| s.required));
        assert!(find(&list, "Resizable BAR").is_some_and(|s| s.reason.contains("ASUS AM5")));
        assert!(find(&list, "Thunderbolt").is_some());
    }

    #[test]
    fn vm_gets_firmware_items_only() {
        let p = build_profile(&fixtures::kvm_guest());
        let list = recommended_settings(&p, MacOsVersion::Tahoe);
        assert!(list.iter().all(|s| s.required));
        assert!(find(&list, "CPU model").is_some());
    }

    #[test]
    fn vega_apu_uma_and_alder_lake_igpu() {
        let mut p = build_profile(&fixtures::amd_b550());
        p.form_factor = FormFactor::Laptop;
        p.gpus = vec![ProfileGpu {
            name: "AMD Radeon Vega 8 Graphics".into(),
            family: GpuFamily::AmdApuVega,
            vendor_id: Some("1002".into()),
            device_id: Some("15d8".into()),
            is_igpu: true,
            ..Default::default()
        }];
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert!(find(&list, "UMA Frame Buffer Size").is_some_and(|s| s.required));

        let mut p = build_profile(&fixtures::windows_z390());
        p.cpu.platform = CpuPlatform::AlderLake;
        p.cpu.is_hybrid = true;
        p.gpus[0] = ProfileGpu {
            name: "Intel UHD Graphics 770".into(),
            family: GpuFamily::IntelXe,
            vendor_id: Some("8086".into()),
            device_id: Some("4680".into()),
            is_igpu: true,
            ..Default::default()
        };
        p.storage.push(ProfileStorage {
            name: "VMD".into(),
            kind: StorageKind::Raid,
            vendor_id: Some("8086".into()),
            device_id: Some("467f".into()),
            size_bytes: None,
        });
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert!(find(&list, "Internal Graphics").is_some());
        assert!(find(&list, "iGPU Multi-Monitor").is_none());
        assert!(find(&list, "Efficient cores").is_some());
        assert!(find(&list, "Intel VMD").is_some_and(|s| s.required));
        assert!(find(&list, "Resizable BAR").is_some());
        assert!(find(&list, "SATA Mode")
            .is_some_and(|s| s.reason.starts_with("The storage controller")));
    }

    #[test]
    fn legacy_board_and_many_threads() {
        let mut p = build_profile(&fixtures::windows_z390());
        p.cpu.platform = CpuPlatform::Penryn;
        p.firmware_uefi = Some(false);
        let list = recommended_settings(&p, MacOsVersion::HighSierra);
        assert!(find(&list, "Boot mode").is_some_and(|s| s.required));
        assert!(find(&list, "CSM").is_none() && find(&list, "Secure Boot").is_none());
        assert!(find(&list, "CFG Lock").is_none());

        let mut p = build_profile(&fixtures::amd_b550());
        p.cpu.threads = 128;
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert!(find(&list, "SMT").is_some_and(|s| s.required && s.value == "disable"));
    }

    #[test]
    fn desktop_igpu_items_follow_the_target_and_cpu() {
        // Haswell iGPU next to an RX 580: headless on Monterey, gone from Ventura.
        let mut p = build_profile(&fixtures::windows_z390());
        p.cpu.platform = CpuPlatform::Haswell;
        p.gpus[0] = ProfileGpu {
            name: "Intel HD Graphics 4600".into(),
            family: GpuFamily::IntelHaswell,
            vendor_id: Some("8086".into()),
            device_id: Some("0412".into()),
            is_igpu: true,
            ..Default::default()
        };
        assert!(find(
            &recommended_settings(&p, MacOsVersion::Monterey),
            "iGPU Multi-Monitor"
        )
        .is_some());
        assert!(find(
            &recommended_settings(&p, MacOsVersion::Ventura),
            "iGPU Multi-Monitor"
        )
        .is_none());
        // No iGPU listed: switched off in the firmware unless it is an F model.
        let mut p = build_profile(&fixtures::windows_z390());
        p.gpus.remove(0);
        assert!(find(
            &recommended_settings(&p, MacOsVersion::Sequoia),
            "iGPU Multi-Monitor"
        )
        .is_some());
        p.cpu.name = "Intel(R) Core(TM) i9-9900KF CPU @ 3.60GHz".into();
        assert!(find(
            &recommended_settings(&p, MacOsVersion::Sequoia),
            "iGPU Multi-Monitor"
        )
        .is_none());
        // Ryzen APU next to a supported dGPU: the APU graphics are turned off.
        let mut p = build_profile(&fixtures::amd_b550());
        p.gpus.push(ProfileGpu {
            name: "AMD Radeon Vega 8 Graphics".into(),
            family: GpuFamily::AmdApuVega,
            vendor_id: Some("1002".into()),
            device_id: Some("1638".into()),
            is_igpu: true,
            ..Default::default()
        });
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert!(find(&list, "Integrated Graphics").is_some_and(|s| s.value == "disable"));
        assert!(find(&list, "UMA Frame Buffer").is_none());
    }

    #[test]
    fn hedt_items_and_vendor_hints() {
        let mut p = build_profile(&fixtures::windows_z390());
        p.cpu.platform = CpuPlatform::SkylakeX;
        assert!(find(&recommended_settings(&p, MacOsVersion::Sonoma), "MMIOH").is_some());
        // TRX40 by chipset: Above 4G is required.
        let mut p = build_profile(&fixtures::amd_b550());
        p.cpu.platform = CpuPlatform::AmdZen2;
        p.chipset = Some("TRX40".into());
        p.motherboard_model = "TRX40 AORUS MASTER".into();
        let list = recommended_settings(&p, MacOsVersion::Sonoma);
        assert!(find(&list, "Above 4G").is_some_and(|s| s.required));
        // ThinkPad hints only for ThinkPads; Alienware is not Dell's setup.
        let mut p = build_profile(&fixtures::linux_laptop_i2c());
        p.motherboard_vendor = "LENOVO".into();
        p.motherboard_model = "20HRCTO1WW".into();
        let list = recommended_settings(&p, MacOsVersion::Sonoma);
        assert_eq!(
            find(&list, "Secure Boot").and_then(|s| s.location_hint.as_deref()),
            Some("Security > Secure Boot")
        );
        p.motherboard_model = "LNVNB161216".into();
        let list = recommended_settings(&p, MacOsVersion::Sonoma);
        assert!(list.iter().all(|s| s.location_hint.is_none()));
        p.motherboard_vendor = "Alienware".into();
        p.motherboard_model = "0XYZ12".into();
        let list = recommended_settings(&p, MacOsVersion::Sonoma);
        assert!(list.iter().all(|s| s.location_hint.is_none()));
        // MSI keeps SGX with the CPU features, PTT under Trusted Computing.
        let mut p = build_profile(&fixtures::windows_z390());
        p.motherboard_vendor = "Micro-Star International Co., Ltd.".into();
        let list = recommended_settings(&p, MacOsVersion::Sequoia);
        assert_eq!(
            find(&list, "Intel SGX").and_then(|s| s.location_hint.as_deref()),
            Some("OC > CPU Features")
        );
        assert_eq!(
            find(&list, "Intel Platform Trust").and_then(|s| s.location_hint.as_deref()),
            Some("Settings > Security > Trusted Computing")
        );
    }

    #[test]
    fn every_supported_platform_gets_a_checklist() {
        for &platform in cpu_db::all_platforms() {
            for form in [
                FormFactor::Desktop,
                FormFactor::Laptop,
                FormFactor::MiniPc,
                FormFactor::AllInOne,
            ] {
                let mut p = build_profile(&fixtures::windows_z390());
                p.cpu.platform = platform;
                p.cpu.vendor = cpu_db::platform_info(platform).vendor;
                p.form_factor = form;
                for target in MacOsVersion::ALL {
                    let list = recommended_settings(&p, target);
                    if platform == CpuPlatform::AppleSilicon {
                        assert!(list.is_empty());
                    } else {
                        assert!(
                            list.iter().any(|s| s.name.starts_with("SATA Mode")),
                            "{platform:?}"
                        );
                        assert!(list
                            .iter()
                            .all(|s| !s.name.is_empty() && !s.reason.is_empty()));
                    }
                }
            }
        }
    }
}
