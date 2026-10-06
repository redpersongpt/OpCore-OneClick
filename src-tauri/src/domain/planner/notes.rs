//! User-facing notes and post-install steps for the plan.
//!
//! Runs last: earlier stages push notes and steps about their own decisions;
//! this stage adds the steps that follow an install (Dortania post-install,
//! OCLP, USBToolBox, itlwm/HeliPort, ssdtPRGen, CPUFriend, iServices) unless
//! an earlier stage already covers the topic, then orders everything by topic
//! and merges repeats of (component, title).

use crate::contracts::note;
use crate::domain::compatibility::{self, ModernStandby};
use crate::domain::model::{BuildPlan, CpuPlatform, GpuFamily, MacOsVersion, NoteLevel, PlanNote};
use crate::domain::{codec_db, device_db};

use super::{
    root_patching_planned, DisplayPlan, PlanContext, ROOT_PATCH_COMPONENT, VOODOOHDA_COMPONENT,
};

const COPY_EFI: &str = "Copy the EFI to the internal drive";
const USB_MAP: &str = "Map the USB ports";
const GPU_ROOT_PATCH: &str = "Apply the OCLP graphics root patch";
const WIFI_ROOT_PATCH: &str = "Apply the OCLP Wi-Fi root patch";
const TAHOE_AUDIO: &str = "Restore analog audio on macOS 26";
const WEB_DRIVER: &str = "Install the NVIDIA Web Driver";
const HELIPORT: &str = "Install HeliPort for Wi-Fi";
const SSDT_PM: &str = "Generate SSDT-PM for CPU power management";
const CPUFRIEND: &str = "Create a CPUFriend data provider";
const NVRAM_SCRIPT: &str = "Install the emulated NVRAM script";
const SLEEP: &str = "Tune sleep settings";
const LAYOUT: &str = "Find the best audio layout";
const ISERVICES: &str = "Check the serial number before using iServices";
const VERBOSE: &str = "Turn off verbose boot";
const FILEVAULT: &str = "Leave FileVault off";
const AMD_APPS: &str = "Apps that expect an Intel CPU";
const DUAL_BOOT: &str = "Dual boot with Windows";

/// Sentence added to a USB map step for macOS 26 (critic-gaps §3).
const USB_TAHOE: &str =
    "On macOS 26 the map works through USBToolBox.kext 1.2.0 or newer; do not convert it to a \
                         native USBMap.kext without the new Tahoe port keys.";

fn text(n: &PlanNote) -> String {
    format!("{} {}", n.title, n.detail).to_ascii_lowercase()
}

/// Position of a post-install step, whichever stage wrote it: copy the EFI,
/// USB map, root patches, companion apps, other steps, tuning, audio layout,
/// iServices, clean-up, AMD notes, dual boot.
fn step_order(n: &PlanNote) -> u8 {
    let title = n.title.to_ascii_lowercase();
    let component = n.component.to_ascii_lowercase();
    let t = text(n);
    let has = |needles: &[&str]| needles.iter().any(|k| t.contains(k));
    if title.contains("copy") && title.contains("efi") {
        0
    } else if component == "usb" {
        10
    } else if component == ROOT_PATCH_COMPONENT
        || component == VOODOOHDA_COMPONENT
        || has(&["root patch", "legacy patcher"])
        || (component == "audio" && t.contains("applehda"))
    {
        20
    } else if has(&["web driver", "heliport"]) {
        30
    } else if has(&["ssdt-pm", "ssdtprgen", "cpufriend", "launchd.command"])
        || (component == "power" && t.contains("pmset"))
    {
        50
    } else if component == "audio" && t.contains("layout") {
        60
    } else if has(&["iservices", "imessage"]) {
        70
    } else if has(&["verbose", "filevault"]) {
        80
    } else if n.title == AMD_APPS {
        85
    } else if t.contains("bitlocker") {
        90
    } else {
        40
    }
}

/// An earlier stage already wrote a step about this topic: `component`
/// (any when None) and one of `needles` in its title or detail.
fn covered(existing: &[PlanNote], component: Option<&str>, needles: &[&str]) -> bool {
    existing.iter().any(|n| {
        component.is_none_or(|c| n.component.eq_ignore_ascii_case(c)) && {
            let t = text(n);
            needles.iter().any(|k| t.contains(k))
        }
    })
}

fn level_order(level: NoteLevel) -> u8 {
    match level {
        NoteLevel::Blocking => 0,
        NoteLevel::Warning => 1,
        NoteLevel::Info => 2,
    }
}

/// Merge repeats of (component, title): the first position, the most severe
/// level and every distinct detail are kept, so two stages that used the same
/// title lose nothing.
fn dedupe(notes: Vec<PlanNote>) -> Vec<PlanNote> {
    let mut out: Vec<PlanNote> = Vec::with_capacity(notes.len());
    let key = |x: &PlanNote| {
        (
            x.component.to_ascii_lowercase(),
            x.title.to_ascii_lowercase(),
        )
    };
    for n in notes {
        match out.iter_mut().find(|o| key(o) == key(&n)) {
            Some(existing) => {
                if level_order(n.level) < level_order(existing.level) {
                    existing.level = n.level;
                }
                let detail = n.detail.trim();
                if !detail.is_empty() && !existing.detail.contains(detail) {
                    if existing.detail.trim().is_empty() {
                        existing.detail = detail.to_string();
                    } else {
                        existing.detail = format!("{} {detail}", existing.detail.trim_end());
                    }
                }
            }
            None => out.push(n),
        }
    }
    out
}

fn has_kext(plan: &BuildPlan, catalog_id: &str) -> bool {
    plan.kexts
        .iter()
        .any(|k| k.enabled && k.catalog_id.eq_ignore_ascii_case(catalog_id))
}

fn has_boot_arg(plan: &BuildPlan, arg: &str) -> bool {
    plan.boot_args
        .iter()
        .flat_map(|a| a.split_whitespace())
        .any(|a| a == arg)
}

/// OCLP's modern-wireless kext set (research-opencore-macos §3.5) or the
/// IOSkywalkFamily block that goes with it, as the root-patch decision sees it.
fn legacy_wireless(ctx: &PlanContext, plan: &BuildPlan) -> bool {
    root_patching_planned(ctx, &ctx.display.clone().unwrap_or_default(), plan).wireless
}

/// Add the plan-wide notes and the post-install steps no earlier stage
/// covered, then merge repeats and order both lists (notes by severity,
/// steps by topic).
pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    add_plan_notes(ctx, plan);
    if ctx.target == MacOsVersion::Tahoe
        && plan.csr_active_config == 0x803
        && plan.smbios.secure_boot_model == "Disabled"
    {
        for step in &mut plan.post_install {
            if step.component == "audio" && step.detail.contains("rebuild") {
                step.detail = "The EFI already has SIP lowered (03080000) and SecureBootModel Disabled for root patching. Restore AppleHDA and keep AppleALC, or install VoodooHDA to /Library/Extensions and disable AppleALC. No EFI rebuild is needed; repeat the root patch after macOS updates.".into();
            }
        }
    }
    let steps = post_install_steps(ctx, display, plan);
    plan.post_install.extend(steps);

    let mut notes = dedupe(std::mem::take(&mut plan.notes));
    notes.sort_by_key(|n| level_order(n.level));
    plan.notes = notes;

    let mut steps = dedupe(std::mem::take(&mut plan.post_install));
    // After merging, so a step written by two stages gets the sentence once.
    if ctx.target == MacOsVersion::Tahoe && has_kext(plan, "USBToolBox") {
        for step in steps
            .iter_mut()
            .filter(|n| n.component.eq_ignore_ascii_case("usb"))
            .filter(|n| n.title.to_ascii_lowercase().contains("map") && !n.detail.contains("1.2.0"))
        {
            step.detail = format!("{} {USB_TAHOE}", step.detail.trim_end());
        }
    }
    steps.sort_by_key(step_order);
    plan.post_install = steps;
}

/// Wi-Fi that works inside macOS Recovery with this plan: AirportItlwm
/// without the legacy wireless stack, Broadcom within its native range, or
/// AirPortAtheros40 up to Big Sur. itlwm needs HeliPort, and root-patched
/// cards only work after the patch.
fn wifi_in_recovery(ctx: &PlanContext, plan: &BuildPlan) -> bool {
    let Some(nic) = ctx.profile.wifi.as_ref() else {
        return false;
    };
    let airport_itlwm = plan
        .kexts
        .iter()
        .any(|k| k.enabled && k.catalog_id.starts_with("AirportItlwm"));
    match device_db::wifi_driver(nic) {
        device_db::WifiDriver::IntelItlwm => airport_itlwm && !legacy_wireless(ctx, plan),
        device_db::WifiDriver::Broadcom { native_max, .. } => ctx.target <= native_max,
        device_db::WifiDriver::AtherosLegacy => ctx.target <= MacOsVersion::BigSur,
        _ => false,
    }
}

/// Plan-wide warnings that no single stage owns.
fn add_plan_notes(ctx: &PlanContext, plan: &mut BuildPlan) {
    if ctx.is_vm {
        return;
    }
    let profile = ctx.profile;
    let board = format!(
        "{} {}",
        profile.motherboard_vendor, profile.motherboard_model
    )
    .to_ascii_uppercase();
    let chipset = &ctx.chipset;
    let mut warn = |component: &str, title: &str, detail: &str| {
        plan.notes
            .push(note(NoteLevel::Warning, component, title, detail));
    };
    if board.contains("ASUS")
        && chipset
            .as_ref()
            .is_some_and(|c| matches!(c.name.as_str(), "Z97" | "H97"))
        && ctx.target >= MacOsVersion::BigSur
    {
        warn("nvram", "ASUS 9-series NVRAM restriction",
            "ASUS Z97/H97 firmware from October 2014 onward can whitelist NVRAM variables. Big Sur and newer installers may fail near 20% with 'device is write locked' or loop at stage 2. Verify native NVRAM first. Options are correctly configured emulated NVRAM, installing on another machine and moving the disk, or an expert firmware repair using an older NvramSmi module.");
    }
    if ctx.platform() == CpuPlatform::NehalemHedt && ctx.target >= MacOsVersion::Ventura {
        warn("usb", "X58/ICH10 USB 1.1 devices need a hub",
            "Ventura removed UHCI/OHCI USB 1.1 drivers. Connect the keyboard and mouse through a USB 2.0 hub or supported USB 3.0 card during installation; apply OCLP's USB 1.1 root patch afterwards.");
    }
    if chipset.as_ref().is_some_and(|c| c.is_am5()) {
        warn("acpi", "Check AM5 BIOS compatibility",
            "Some late-2023 and newer AGESA BIOS releases, especially ASUS/MSI X670E/B650E firmware preparing for Ryzen 8000, introduce CPVS and conditional ACPI scopes that prevent macOS boot. Use board-specific ACPI patches (CorpNewt or validated etorix am5_patches), or a compatible older BIOS if it supports this CPU. Do not apply another board's DSDT blindly.");
    }
    if matches!(
        ctx.platform(),
        CpuPlatform::AmdBulldozer | CpuPlatform::AmdJaguar
    ) {
        warn("cpu", "AMD 15h/16h firmware exceptions",
            "For an 'X64 Exception Type' boot failure, first disable CSM. Some firmware is incompatible with ProvideCurrentCpuInfo; the fallback on Big Sur or older is the legacy 15h_16h AMD_Vanilla patch set at commit 06a9a7f3.");
        if ctx.target == MacOsVersion::Mojave {
            warn("cpu", "Mojave on AMD 15h/16h",
                "The first boot can restart after Data & Privacy, and some web pages can crash. Consult the AMD_Vanilla README's InsanelyMac UPDATE-2 and UPDATE-5 fixes.");
        }
    }
    if ctx.is_amd() && ctx.target <= MacOsVersion::Mojave {
        warn("cpu", "32-bit apps do not work with AMD_Vanilla",
            "On macOS 10.13 and 10.14 these kernel patches do not support 32-bit applications. A custom kernel is a separate workaround and loses iMessage support.");
    }
    if chipset
        .as_ref()
        .is_some_and(|c| c.name.eq_ignore_ascii_case("TRX40"))
    {
        warn("cpu", "TRX40 PAT patches are disabled",
            "AMD_Vanilla recommends disabling PAT patches for GPU performance on TRX40. Test this EFI on a USB drive first; re-enable the Algrey PAT patches if boot or graphics fail.");
    }
    if crate::domain::chipset_db::needs_amd_hotplug_fix(profile, chipset.as_ref()) {
        warn("pci", "AM5 PCI hotplug fix is enabled",
            "The IOPCIIsHotplugPort patch is enabled for this AM5 board's onboard Thunderbolt/USB4 and Wi-Fi combination.");
    }
    if plan
        .kexts
        .iter()
        .any(|k| k.catalog_id == "CpuTopologyRebuild")
    {
        plan.post_install.push(note(NoteLevel::Info, "cpu", "Enable hybrid CPU topology after installation",
            "CpuTopologyRebuild is disabled in the installer EFI. ProvideCurrentCpuInfo handles initial boot. After removing -v, enable CpuTopologyRebuild in Kernel → Add; verbose boot with this kext can cause random hangs."));
    }
    if matches!(plan.smbios.model.as_str(), "iMac20,1" | "MacPro7,1") && ctx.is_intel() {
        plan.post_install.push(note(NoteLevel::Info, "cpu", "Check CPU frequency vectors",
            "These SMBIOS frequency vectors can reduce CPU performance. Compare benchmarks with SSDT-PLUG disabled to diagnose it, then restore it. If affected, add CPUFriend and a CPUFriendDataProvider matched to this CPU; Dortania bugtracker issue 190 describes the Haswell-QoS provider used on Comet/Rocket/Alder Lake."));
    }
    let ethernet_ok = profile.ethernet.iter().any(|nic| {
        let info = device_db::ethernet_info(nic);
        info.driver != device_db::EthernetDriver::Unsupported
            && info.min_macos.is_none_or(|m| ctx.target >= m)
    });
    if !ethernet_ok && !wifi_in_recovery(ctx, plan) {
        let why = if has_kext(plan, "itlwm") {
            "itlwm needs the HeliPort app, which is not available in macOS Recovery."
        } else if legacy_wireless(ctx, plan) {
            "The Wi-Fi card only works after the post-install root patch."
        } else if profile.wifi.is_some() {
            "The Wi-Fi card has no driver in this build."
        } else {
            "No supported Ethernet or Wi-Fi adapter was found."
        };
        plan.notes.push(note(
            NoteLevel::Warning,
            "network",
            "No network in macOS Recovery",
            &format!(
                "{why} The installer downloads macOS from Apple, so connect a supported Ethernet adapter (a USB \
                 CDC-ECM/NCM class or supported Realtek RTL8153/RTL8156 adapter works) for the install."
            ),
        ));
    }
    if compatibility::modern_standby(profile) == ModernStandby::Likely {
        plan.notes.push(note(
            NoteLevel::Warning,
            "power",
            "Sleep may not work",
            "This laptop generation usually only offers Modern Standby (S0ix); macOS needs S3 sleep. Look for an \
             S3 / \"Linux\" sleep option in the BIOS, otherwise disable sleep after installing.",
        ));
    }
    if ctx.is_laptop
        && profile
            .motherboard_vendor
            .to_ascii_lowercase()
            .contains("lenovo")
    {
        plan.notes.push(note(NoteLevel::Warning, "nvram", "Avoid Reset NVRAM on Lenovo laptops",
            "Some Lenovo firmware becomes unbootable after an NVRAM reset (OpenCore bugtracker issue 995). ResetNvramEntry is omitted from this EFI; do not use other NVRAM reset tools without checking the exact model."));
    }
    if ctx.is_hedt
        && matches!(plan.smbios.model.as_str(), "iMacPro1,1" | "MacPro7,1")
        && profile.gpus.iter().any(|g| {
            !g.disabled
                && matches!(
                    g.family,
                    GpuFamily::AmdGcn1
                        | GpuFamily::AmdGcn2
                        | GpuFamily::AmdGcn3
                        | GpuFamily::NvidiaKepler
                )
        })
    {
        plan.notes.push(note(NoteLevel::Warning, "gpu", "Workstation graphics and DRM",
            "iMacPro1,1/MacPro7,1 expect Polaris, Vega or Navi for hardware video decoding and DRM. With an older card, consider MacPro6,1 on releases that support it, or upgrade the display GPU; root patches do not guarantee DRM."));
    }
    if ctx.platform() == CpuPlatform::CoffeeLake && ctx.target == MacOsVersion::HighSierra {
        plan.notes.push(note(NoteLevel::Warning, "gpu", "Coffee Lake graphics on High Sierra",
            "Coffee Lake iGPU support requires the MacBookPro15,x-specific macOS 10.13.6 build (17G2208 or newer), not a generic High Sierra recovery image. Mojave or newer is the safer installer target."));
    }
    if has_boot_arg(plan, "-v") {
        plan.notes.push(note(
            NoteLevel::Info,
            "boot",
            "Verbose boot is on",
            "The first boots print kernel messages instead of the Apple logo, which shows where a boot stops.",
        ));
    }
}

fn post_install_steps(ctx: &PlanContext, display: &DisplayPlan, plan: &BuildPlan) -> Vec<PlanNote> {
    let profile = ctx.profile;
    let target = ctx.target;
    let existing = plan.post_install.as_slice();
    let mut steps = Vec::new();
    let mut add = |level: NoteLevel, component: &str, title: &str, detail: &str| {
        steps.push(note(level, component, title, detail));
    };
    if let Some(vm) = profile.vm {
        let detail = match vm {
            crate::domain::model::VmKind::Kvm => "Attach the OpenCore EFI as a virtual boot disk and the converted BaseSystem recovery image on an AHCI bus (raw or qcow2). Keep the scanned CPU vendor and vCPU topology; rebuild after changing them. On the KVM host enable ignore_msrs=1 and report_ignored_msrs=0. Expose invtsc and vmware-cpuid-freq=on; an Intel guest model must use vendor=GenuineIntel. For input use usb-tablet or PS/2, not virtio-tablet-pci. Use a VGA-capable adapter such as vmware-svga; non-VGA virtio-gpu-pci may expose only a Blt-only GOP that macOS cannot draw to.",
            crate::domain::model::VmKind::HyperV => "Attach OpenCore and recovery as VHDX disks to a Generation 2 VM (qemu-img convert -O vhdx). MacHyperVFramebuffer adds synthetic display/resolution support; on macOS 11+ install it to /Library/Extensions with kext signing disabled in SIP. It does not provide Metal acceleration. DDA GPU passthrough needs a supported Windows Server host.",
            crate::domain::model::VmKind::Vmware => "Attach OpenCore and recovery as virtual disks (VMDK) or suitable bootable ISO media. With the OC4VM approach use firmware=efi, smc.present=FALSE and guestOS=darwin*-64 in the VMX, and disable virtual firmware Secure Boot; VirtualSMC supplies the SMC. Install VMware Tools from darwin.iso for the SVGA display driver. Workstation/Player does not offer PCI GPU passthrough or Metal acceleration.",
            _ => "Attach the OpenCore EFI and recovery image using virtual disks supported by the hypervisor, and select OpenCore first in its UEFI boot order.",
        };
        add(
            NoteLevel::Info,
            "vm",
            "Prepare the virtual installer disks",
            detail,
        );
    }
    if !ctx.is_vm && ctx.platform() == CpuPlatform::Penryn {
        add(NoteLevel::Info, "acpi", "Check HPET if CPU power management panics",
            "For an AppleIntelCPUPowerManagement panic, check that HPET is enabled in firmware. Run SSDTTime FixHPET on this PC and include both its SSDT and patches if an IRQ conflict is found. Do not apply another board's HPET patch; DummyPowerManagement is only a diagnostic fallback and disables CPU power management.");
    }
    if !ctx.is_vm && ctx.platform() == CpuPlatform::ArrowLake && target == MacOsVersion::Tahoe {
        add(NoteLevel::Warning, "acpi", "Check the 800-series ACPI patch if Tahoe stalls",
            "Board-specific reports differ. If Tahoe hangs, test disabling 'Remove conditional ACPI scope declaration (Intel 800-series)' on a spare boot EFI. A successful OpenCore MOD report did not need it; this does not establish a universal setting for every 800-series BIOS.");
    }
    let oclp = if target == MacOsVersion::Tahoe {
        "OpenCore Legacy Patcher 3.0 or newer"
    } else {
        "OpenCore Legacy Patcher"
    };

    if !existing.iter().any(|n| step_order(n) == 0) {
        let detail = if ctx.is_vm {
            "After installing, copy the EFI folder to the EFI partition of the VM's system disk (sudo diskutil \
             mount disk0s1, check the disk with diskutil list) so the VM boots without the installer image."
        } else {
            "After installing, mount the internal drive's EFI partition (diskutil list, then sudo diskutil mount \
             diskXs1) and copy the EFI folder from the USB drive. Keep an existing EFI/Microsoft folder for \
             Windows, put the drive first in the BIOS boot order and keep the USB drive as a backup."
        };
        add(NoteLevel::Info, "boot", COPY_EFI, detail);
    }

    if has_kext(plan, "USBToolBox")
        && !existing.iter().any(|n| {
            n.component.eq_ignore_ascii_case("usb") && n.title.to_ascii_lowercase().contains("map")
        })
    {
        let mut detail = String::from(
            "The build enables every USB port with UTBDefault.kext. Map the ports with the USBToolBox tool (on \
             Windows before installing, or in macOS): keep at most 15 ports per controller, mark internal ones, \
             build UTBMap.kext, then replace UTBDefault.kext with UTBMap.kext in EFI/OC/Kexts and config.plist. \
             Keep USBToolBox.kext.",
        );
        if target == MacOsVersion::Tahoe {
            detail.push(' ');
            detail.push_str(USB_TAHOE);
        }
        add(NoteLevel::Info, "usb", USB_MAP, &detail);
    }

    let root_patch = root_patching_planned(ctx, display, plan);
    if let Some(gpu) = display.primary.and_then(|i| profile.gpus.get(i)) {
        // The graphics stage's verdict, which includes Polaris/Vega on CPUs
        // without AVX2 (research-gpu §8).
        if root_patch.graphics
            && !covered(existing, Some("gpu"), &["root patch", "legacy patcher"])
            && !covered(
                existing,
                Some(ROOT_PATCH_COMPONENT),
                &["graphic", "gpu", "acceleration"],
            )
        {
            add(
                NoteLevel::Warning,
                "gpu",
                GPU_ROOT_PATCH,
                &format!(
                    "{} has no native driver on {}. After installing, run {oclp} → Post-Install Root Patch; \
                     until then the display runs without acceleration. Repeat it after every macOS update.",
                    gpu.name,
                    target.display_name()
                ),
            );
        }
        if target == MacOsVersion::HighSierra
            && matches!(
                gpu.family,
                GpuFamily::NvidiaMaxwell | GpuFamily::NvidiaPascal
            )
            && !covered(existing, None, &["web driver"])
        {
            add(
                NoteLevel::Warning,
                "gpu",
                WEB_DRIVER,
                "Install NVIDIA Web Driver 387.10.10.10.40.140 for macOS 10.13.6 (17G14042) after the install; \
                 without it the card has no acceleration.",
            );
        }
    }

    if root_patch.wireless && !covered(existing, Some("wifi"), &["root patch", "legacy patcher"])
    {
        add(
            NoteLevel::Warning,
            "wifi",
            WIFI_ROOT_PATCH,
            &format!(
                "Wi-Fi uses the legacy wireless stack. After installing, run {oclp} → Post-Install Root Patch \
                 (Modern Wireless) and reboot; repeat it after every macOS update."
            ),
        );
    }

    if target == MacOsVersion::Tahoe
        && compatibility::has_analog_audio(profile)
        && !covered(existing, None, &["applehda", "voodoohda"])
    {
        // research-opencore-macos §3.5: VoodooHDA with SIP 03000000, or
        // AppleHDA put back with SIP 03080000 and SecureBootModel Disabled.
        add(
            NoteLevel::Warning,
            "audio",
            TAHOE_AUDIO,
            "macOS 26 has no AppleHDA, so AppleALC alone gives no analog sound. Options: VoodooHDA (set \
             csr-active-config to 03000000, install the kext in /Library/Extensions and allow it in Privacy & \
             Security; lower quality), or re-install AppleHDA with an OCLP-based patcher (SIP 03080000, \
             SecureBootModel Disabled, repeated after every update). USB audio and HDMI/DP audio from AMD GPUs \
             work without either.",
        );
    }

    if has_kext(plan, "itlwm") && !covered(existing, None, &["heliport"]) {
        add(
            NoteLevel::Info,
            "wifi",
            HELIPORT,
            "itlwm shows Wi-Fi as an Ethernet port. Install the HeliPort app (OpenIntelWireless) to scan and join \
             networks, and add it to the login items.",
        );
    }

    if !ctx.is_vm
        && matches!(
            ctx.platform(),
            CpuPlatform::SandyBridge
                | CpuPlatform::IvyBridge
                | CpuPlatform::SandyBridgeE
                | CpuPlatform::IvyBridgeE
        )
        && !covered(existing, None, &["ssdt-pm", "ssdtprgen"])
    {
        // Dortania sandy-bridge/ivy-bridge pages and pm.md: the CpuPm and
        // Cpu0Ist tables are dropped until this SSDT exists.
        add(
            NoteLevel::Info,
            "cpu",
            SSDT_PM,
            "Sandy and Ivy Bridge need a CPU power management table made for the exact CPU. Run ssdtPRGen.sh in \
             macOS, copy the generated ssdt.aml to EFI/OC/ACPI as SSDT-PM.aml and add it to ACPI → Add in \
             config.plist.",
        );
    }

    if has_kext(plan, "CPUFriend") && !covered(existing, None, &["cpufriend"]) {
        add(
            NoteLevel::Info,
            "cpu",
            CPUFRIEND,
            "CPUFriend does nothing on its own. Generate CPUFriendDataProvider.kext for this CPU with \
             CPUFriendFriend in macOS, add it to EFI/OC/Kexts and to Kernel → Add after CPUFriend.kext.",
        );
    }

    // critic-gaps §2: without the logout hook no NVRAM change made in macOS
    // persists on emulated NVRAM (OpenVariableRuntimeDxe or OpenDuet).
    let emulated_nvram = plan
        .drivers
        .iter()
        .any(|d| d.enabled && d.path.eq_ignore_ascii_case("OpenVariableRuntimeDxe.efi"))
        || compatibility::legacy_boot_only(profile);
    if emulated_nvram && !covered(existing, None, &["launchd.command", "logouthook"]) {
        add(
            NoteLevel::Info,
            "nvram",
            NVRAM_SCRIPT,
            "This machine uses emulated NVRAM. Install OpenCore's Utilities/LogoutHook/Launchd.command (run it \
             with the install argument) so NVRAM changes are saved to nvram.plist at shutdown. During installation select macOS Installer manually on each restart, then select the target system disk when installation completes.",
        );
    }

    if ctx.is_laptop && !ctx.is_vm && !covered(existing, Some("power"), &["pmset"]) {
        let mut detail = String::from(
            "Hibernation is not supported. Run: sudo pmset -a hibernatemode 0; sudo pmset -a standby 0; sudo pmset \
             -a autopoweroff 0; then test sleep and wake on battery and on power.",
        );
        if compatibility::modern_standby(profile) != ModernStandby::Unlikely {
            detail.push_str(
                " If the laptop only supports Modern Standby, disable sleep entirely (sudo pmset -a disablesleep 1).",
            );
        }
        add(NoteLevel::Info, "power", SLEEP, &detail);
    }

    if has_kext(plan, "AppleALC")
        && target != MacOsVersion::Tahoe
        && !covered(existing, Some("audio"), &["layout"])
    {
        if let Some(id) = profile.audio.as_ref().and_then(|a| a.codec_id) {
            let layouts: Vec<String> = codec_db::ranked_layouts(id, None, ctx.is_laptop)
                .iter()
                .take(8)
                .map(u32::to_string)
                .collect();
            if layouts.len() > 1 {
                add(
                    NoteLevel::Info,
                    "audio",
                    LAYOUT,
                    &format!(
                        "If there is no sound or a jack does not work, try the other AppleALC layouts with the \
                         boot argument alcid=<id> (best first: {}), then make the working one permanent as \
                         layout-id.",
                        layouts.join(", ")
                    ),
                );
            }
        }
    }

    if !ctx.is_vm && !covered(existing, None, &["iservices", "imessage"]) {
        add(
            NoteLevel::Info,
            "smbios",
            ISERVICES,
            "Before signing in to iMessage, FaceTime or iCloud, check the generated serial on Apple's Check \
             Coverage page: it must be reported as invalid. Keep the same serial, MLB, UUID and ROM in every \
             rebuild (export the identity) so Apple services stay signed in.",
        );
    }

    if has_boot_arg(plan, "-v") && !covered(existing, None, &["verbose"]) {
        add(
            NoteLevel::Info,
            "boot",
            VERBOSE,
            "Once macOS boots reliably, remove -v (and keepsyms=1 debug=0x100 if present) from boot-args in \
             NVRAM → Add → 7C436110-AB2A-4BBB-A880-FE41995C9F82.",
        );
    }

    // research-opencore-macos §3.5: the Tahoe APFS driver cannot unlock
    // FileVault volumes under OpenCore (bugtracker #2499).
    if target == MacOsVersion::Tahoe && !covered(existing, None, &["filevault"]) {
        add(
            NoteLevel::Info,
            "storage",
            FILEVAULT,
            "OpenCore cannot unlock FileVault volumes with the macOS 26 APFS driver; do not turn FileVault on.",
        );
    }

    if ctx.is_amd() && !ctx.is_vm && !covered(existing, None, &["docker", "hypervisor-based"]) {
        // research-amd §15.
        add(
            NoteLevel::Info,
            "cpu",
            AMD_APPS,
            "Hypervisor-based apps (Docker Desktop, VMware Fusion, Parallels, Android emulators) do not run on AMD, \
             and some apps built with Intel MKL (parts of Adobe's suite) need community patches.",
        );
    }

    if !ctx.is_vm && !covered(existing, None, &["bitlocker"]) {
        // critic-gaps §4: BitLocker recovery triggers and the 200 MB ESP rule.
        add(
            NoteLevel::Warning,
            "platform",
            DUAL_BOOT,
            "With Windows on this PC: save the BitLocker recovery key (aka.ms/myrecoverykey) and suspend BitLocker \
             (manage-bde -protectors -disable C: -RebootCount 0) before changing BIOS settings or partitions. \
             Prefer a separate drive for macOS; a shared drive needs an EFI partition of at least 200 MB as the \
             first partition. Windows started through OpenCore may ask for reactivation; starting it from the BIOS \
             boot menu avoids that.",
        );
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        BuildOptions, DriverPlan, FormFactor, HardwareProfile, KernelBlock, KextSelection,
    };
    use crate::domain::planner::empty_plan;
    use crate::domain::profile::{build_profile, fixtures};

    fn kext(id: &str, bundle: &str) -> KextSelection {
        KextSelection {
            catalog_id: id.into(),
            bundle: bundle.into(),
            plugins: vec![],
            enabled: true,
            min_kernel: None,
            max_kernel: None,
            required: false,
            reason: String::new(),
        }
    }

    fn display(primary: Option<usize>) -> DisplayPlan {
        DisplayPlan {
            primary,
            igpu: None,
            igpu_headless: false,
            disabled: vec![],
        }
    }

    fn titles(notes: &[PlanNote]) -> Vec<&str> {
        notes.iter().map(|n| n.title.as_str()).collect()
    }

    fn run(
        profile: &HardwareProfile,
        target: MacOsVersion,
        display_plan: DisplayPlan,
        fill: impl FnOnce(&mut BuildPlan),
    ) -> BuildPlan {
        let options = BuildOptions {
            target,
            ..Default::default()
        };
        let ctx = PlanContext::new(profile, &options);
        let mut plan = empty_plan(target);
        fill(&mut plan);
        apply(&ctx, &display_plan, &mut plan);
        plan
    }

    #[test]
    fn desktop_post_install_order() {
        let p = build_profile(&fixtures::windows_z390());
        let plan = run(&p, MacOsVersion::Sequoia, display(Some(1)), |plan| {
            plan.kexts.push(kext("USBToolBox", "USBToolBox.kext"));
            plan.kexts.push(kext("USBToolBox", "UTBDefault.kext"));
            plan.kexts.push(kext("AppleALC", "AppleALC.kext"));
            plan.boot_args = vec!["-v".into(), "keepsyms=1".into(), "debug=0x100".into()];
            plan.post_install.push(note(
                NoteLevel::Info,
                "gpu",
                "Stage step",
                "From the graphics stage.",
            ));
            plan.notes.push(note(NoteLevel::Info, "gpu", "A", "first"));
            plan.notes.push(note(
                NoteLevel::Warning,
                "gpu",
                "a",
                "duplicate with a higher level",
            ));
            plan.notes
                .push(note(NoteLevel::Blocking, "cpu", "B", "blocking"));
        });
        let t = titles(&plan.post_install);
        assert_eq!(t.first(), Some(&COPY_EFI));
        assert_eq!(t.last(), Some(&DUAL_BOOT));
        let pos = |title: &str| t.iter().position(|x| *x == title).unwrap_or(usize::MAX);
        assert!(pos(USB_MAP) < pos("Stage step"));
        assert!(pos("Stage step") < pos(LAYOUT));
        assert!(pos(LAYOUT) < pos(ISERVICES) && pos(ISERVICES) < pos(VERBOSE));
        assert!(!t.contains(&TAHOE_AUDIO) && !t.contains(&HELIPORT) && !t.contains(&SLEEP));
        assert!(plan
            .post_install
            .iter()
            .find(|n| n.title == USB_MAP)
            .is_some_and(|n| n.detail.contains("UTBMap.kext")));
        // Notes: merged by (component, title), most severe level and every
        // distinct detail kept, sorted by level.
        assert_eq!(plan.notes.len(), 3);
        assert_eq!(plan.notes[0].level, NoteLevel::Blocking);
        let a = plan.notes.iter().find(|n| n.title == "A").expect("kept");
        assert_eq!(
            (a.level, a.detail.as_str()),
            (NoteLevel::Warning, "first duplicate with a higher level")
        );
    }

    #[test]
    fn tahoe_laptop_with_itlwm() {
        let p = build_profile(&fixtures::linux_laptop_i2c());
        let plan = run(&p, MacOsVersion::Tahoe, display(Some(0)), |plan| {
            plan.kexts.push(kext("itlwm", "itlwm.kext"));
            plan.kexts.push(kext("AppleALC", "AppleALC.kext"));
            plan.kexts.push(kext("USBToolBox", "USBToolBox.kext"));
        });
        let t = titles(&plan.post_install);
        assert!(t.contains(&TAHOE_AUDIO));
        assert!(t.contains(&HELIPORT));
        assert!(t.contains(&SLEEP));
        assert!(t.contains(&FILEVAULT));
        assert!(!t.contains(&LAYOUT), "no AppleHDA on Tahoe");
        assert!(plan
            .post_install
            .iter()
            .any(|n| n.title == USB_MAP && n.detail.contains("1.2.0")));
        assert!(plan
            .notes
            .iter()
            .any(|n| n.title == "No network in macOS Recovery"));
    }

    #[test]
    fn root_patch_paths() {
        let mut p = build_profile(&fixtures::windows_z390());
        // Haswell iGPU as the display on Ventura: OCLP root patch.
        p.cpu.platform = CpuPlatform::Haswell;
        p.gpus = vec![crate::domain::model::ProfileGpu {
            name: "Intel HD Graphics 4600".into(),
            family: GpuFamily::IntelHaswell,
            vendor_id: Some("8086".into()),
            device_id: Some("0412".into()),
            is_igpu: true,
            ..Default::default()
        }];
        let plan = run(&p, MacOsVersion::Ventura, display(Some(0)), |plan| {
            plan.kexts
                .push(kext("IOSkywalkFamily", "IOSkywalkFamily.kext"));
            plan.kernel_blocks.push(KernelBlock {
                comment: String::new(),
                identifier: "com.apple.iokit.IOSkywalkFamily".into(),
                strategy: "Exclude".into(),
                min_kernel: "23.0.0".into(),
                max_kernel: String::new(),
                enabled: true,
            });
        });
        let t = titles(&plan.post_install);
        assert!(t.contains(&GPU_ROOT_PATCH) && t.contains(&WIFI_ROOT_PATCH));
        let pos = |title: &str| t.iter().position(|x| *x == title).unwrap_or(usize::MAX);
        assert!(pos(USB_MAP) == usize::MAX && pos(GPU_ROOT_PATCH) < pos(ISERVICES));
        // Ivy Bridge: SSDT-PM.
        p.cpu.platform = CpuPlatform::IvyBridge;
        let plan = run(&p, MacOsVersion::Monterey, display(Some(0)), |_| {});
        assert!(titles(&plan.post_install).contains(&SSDT_PM));
    }

    #[test]
    fn nvidia_web_driver_and_nvram() {
        let mut p = build_profile(&fixtures::windows_z390());
        p.gpus[1] = crate::domain::model::ProfileGpu {
            name: "NVIDIA GeForce GTX 1070".into(),
            family: GpuFamily::NvidiaPascal,
            vendor_id: Some("10de".into()),
            device_id: Some("1b81".into()),
            ..Default::default()
        };
        let plan = run(&p, MacOsVersion::HighSierra, display(Some(1)), |plan| {
            plan.drivers.push(DriverPlan {
                path: "OpenVariableRuntimeDxe.efi".into(),
                load_early: true,
                enabled: true,
                comment: String::new(),
                source: "opencore".into(),
            });
        });
        let t = titles(&plan.post_install);
        assert!(t.contains(&WEB_DRIVER) && t.contains(&NVRAM_SCRIPT));
    }

    #[test]
    fn amd_and_vm_steps() {
        let p = build_profile(&fixtures::amd_b550());
        let plan = run(&p, MacOsVersion::Sequoia, display(Some(0)), |_| {});
        assert!(titles(&plan.post_install).contains(&AMD_APPS));

        let p = build_profile(&fixtures::kvm_guest());
        let plan = run(&p, MacOsVersion::Tahoe, display(Some(0)), |_| {});
        let t = titles(&plan.post_install);
        assert!(t.contains(&COPY_EFI));
        assert!(!t.contains(&DUAL_BOOT) && !t.contains(&ISERVICES) && !t.contains(&TAHOE_AUDIO));
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn steps_from_other_stages_are_not_repeated() {
        let p = build_profile(&fixtures::linux_laptop_i2c());
        let plan = run(&p, MacOsVersion::Tahoe, display(Some(0)), |plan| {
            plan.kexts.push(kext("itlwm", "itlwm.kext"));
            plan.kexts.push(kext("AppleALC", "AppleALC.kext"));
            plan.kexts.push(kext("USBToolBox", "USBToolBox.kext"));
            let step = |component: &str, title: &str, detail: &str| PlanNote {
                level: NoteLevel::Info,
                component: component.into(),
                title: title.into(),
                detail: detail.into(),
            };
            // What the kexts and quirks stages write for the same topics.
            plan.post_install.push(step(
                "audio",
                "Restore AppleHDA on macOS 26",
                "Restore AppleHDA from macOS 15 with a root-patch tool. Alternative: VoodooHDA.",
            ));
            plan.post_install.push(step(
                "wifi",
                "Install HeliPort for Intel Wi-Fi",
                "Install HeliPort after macOS is installed.",
            ));
            plan.post_install.push(step(
                "usb",
                "Map the USB ports",
                "Run the USBToolBox tool and replace UTBDefault.kext with UTBMap.kext.",
            ));
            plan.post_install.push(step(
                "usb",
                "Map the USB ports",
                "XhciPortLimit is on; turn it off after mapping.",
            ));
        });
        let t = titles(&plan.post_install);
        assert!(!t.contains(&TAHOE_AUDIO) && !t.contains(&HELIPORT));
        let usb: Vec<_> = plan
            .post_install
            .iter()
            .filter(|n| n.component == "usb")
            .collect();
        assert_eq!(usb.len(), 1, "one USB step with both details");
        assert!(usb[0].detail.contains("XhciPortLimit") && usb[0].detail.contains("1.2.0"));
        // Topic order holds for the other stages' steps too.
        let pos = |title: &str| t.iter().position(|x| *x == title).unwrap_or(usize::MAX);
        assert!(pos(COPY_EFI) < pos(USB_MAP));
        assert!(pos(USB_MAP) < pos("Restore AppleHDA on macOS 26"));
        assert!(pos("Restore AppleHDA on macOS 26") < pos("Install HeliPort for Intel Wi-Fi"));
        assert!(pos("Install HeliPort for Intel Wi-Fi") < pos(SLEEP));
        assert!(pos(SLEEP) < pos(ISERVICES) && pos(ISERVICES) < pos(FILEVAULT));
        assert_eq!(t.last(), Some(&DUAL_BOOT));
    }

    #[test]
    fn non_avx2_polaris_and_open_duet() {
        let mut p = build_profile(&fixtures::windows_z390());
        p.cpu.platform = CpuPlatform::SandyBridge;
        p.cpu.has_avx2 = Some(false);
        p.firmware_uefi = Some(false);
        // The RX 580 drives the displays on Ventura through CryptexFixup.
        let plan = run(&p, MacOsVersion::Ventura, display(Some(1)), |_| {});
        let t = titles(&plan.post_install);
        assert!(t.contains(&GPU_ROOT_PATCH), "{t:?}");
        assert!(t.contains(&NVRAM_SCRIPT), "OpenDuet has emulated NVRAM");
        assert!(t.contains(&SSDT_PM));
        // A graphics root-patch step from the graphics stage is not repeated.
        let plan = run(&p, MacOsVersion::Ventura, display(Some(1)), |plan| {
            plan.post_install.push(note(
                NoteLevel::Warning,
                "gpu",
                "Apply OpenCore Legacy Patcher root patches",
                "Install OpenCore Legacy Patcher in macOS and run Post-Install Root Patch.",
            ));
        });
        assert!(!titles(&plan.post_install).contains(&GPU_ROOT_PATCH));
        // A legacy (CSM) boot on a UEFI-era board is not OpenDuet.
        p.cpu.platform = CpuPlatform::IvyBridge;
        let plan = run(&p, MacOsVersion::Monterey, display(Some(1)), |_| {});
        assert!(!titles(&plan.post_install).contains(&NVRAM_SCRIPT));
    }

    #[test]
    fn recovery_network_warning() {
        let recovery = |plan: &BuildPlan| {
            plan.notes
                .iter()
                .any(|n| n.title == "No network in macOS Recovery")
        };
        let mut p = build_profile(&fixtures::linux_laptop_i2c());
        // AirportItlwm on Sonoma works in Recovery.
        let plan = run(&p, MacOsVersion::Sonoma, display(Some(0)), |plan| {
            plan.kexts
                .push(kext("AirportItlwm-Sonoma14.4", "AirportItlwm.kext"));
        });
        assert!(!recovery(&plan));
        // A MediaTek card has no driver at all.
        if let Some(w) = p.wifi.as_mut() {
            w.vendor_id = Some("14c3".into());
            w.device_id = Some("0616".into());
        }
        let plan = run(&p, MacOsVersion::Sonoma, display(Some(0)), |_| {});
        assert!(recovery(&plan));
        // A Broadcom BCM4360 within its native range does.
        if let Some(w) = p.wifi.as_mut() {
            w.vendor_id = Some("14e4".into());
            w.device_id = Some("43a0".into());
        }
        assert!(!recovery(&run(
            &p,
            MacOsVersion::Ventura,
            display(Some(0)),
            |_| {}
        )));
        assert!(recovery(&run(
            &p,
            MacOsVersion::Sonoma,
            display(Some(0)),
            |_| {}
        )));
        // Working Ethernet removes the warning.
        let p = build_profile(&fixtures::amd_b550());
        assert!(!recovery(&run(
            &p,
            MacOsVersion::Tahoe,
            display(Some(0)),
            |plan| {
                plan.kexts.push(kext("itlwm", "itlwm.kext"));
            }
        )));
    }

    #[test]
    fn every_platform_and_target_runs() {
        let base = build_profile(&fixtures::linux_laptop_i2c());
        for &platform in crate::domain::cpu_db::all_platforms() {
            for form in [
                FormFactor::Desktop,
                FormFactor::Laptop,
                FormFactor::AllInOne,
                FormFactor::MiniPc,
            ] {
                for vm in [None, Some(crate::domain::model::VmKind::Kvm)] {
                    let mut p = base.clone();
                    p.cpu.platform = platform;
                    p.cpu.vendor = crate::domain::cpu_db::platform_info(platform).vendor;
                    p.form_factor = form;
                    p.vm = vm;
                    for target in MacOsVersion::ALL {
                        let plan = run(&p, target, display(Some(0)), |plan| {
                            plan.boot_args.push("-v".into());
                            plan.kexts.push(kext("USBToolBox", "USBToolBox.kext"));
                        });
                        assert!(!plan.post_install.is_empty());
                        let mut seen = std::collections::HashSet::new();
                        assert!(plan
                            .post_install
                            .iter()
                            .all(|n| seen.insert((n.component.clone(), n.title.clone()))));
                        let order: Vec<u8> = plan.post_install.iter().map(step_order).collect();
                        assert!(order.windows(2).all(|w| w[0] <= w[1]), "{order:?}");
                        let levels: Vec<u8> =
                            plan.notes.iter().map(|n| level_order(n.level)).collect();
                        assert!(levels.windows(2).all(|w| w[0] <= w[1]));
                    }
                }
            }
        }
    }
}
