//! config.plist from a plan without downloads: Kernel->Add entries and EFI
//! file lists are derived from the plan the way the build pipeline would
//! find them on disk, and every override is checked against OpenCore 1.0.8's
//! Sample.plist.

use std::io::Cursor;

use app_lib::domain::config_writer::{write_config, ConfigInputs};
use app_lib::domain::kernel_add::KernelAddEntry;
use app_lib::domain::model::{BuildPlan, PickerStyle, PlatformIdentity, PlistScalar, SettingMap};
use app_lib::error::AppError;
use plist::Value;

pub const SAMPLE: &[u8] = include_bytes!("../fixtures/Sample-1.0.8.plist");

/// `Docs/AcpiSamples/Binaries` of OpenCore-1.0.8-RELEASE.zip.
pub const OC_ACPI_SAMPLES: &[&str] = &[
    "SSDT-ALS0.aml",
    "SSDT-AWAC-DISABLE.aml",
    "SSDT-BRG0.aml",
    "SSDT-EC-USBX.aml",
    "SSDT-EC.aml",
    "SSDT-EHCx-DISABLE.aml",
    "SSDT-HV-DEV.aml",
    "SSDT-HV-PLUG.aml",
    "SSDT-HV-VMBUS.aml",
    "SSDT-IMEI.aml",
    "SSDT-PLUG-ALT.aml",
    "SSDT-PLUG.aml",
    "SSDT-PMC.aml",
    "SSDT-PNLF.aml",
    "SSDT-RTC0-RANGE.aml",
    "SSDT-RTC0.aml",
    "SSDT-SBUS-MCHC.aml",
    "SSDT-UNC.aml",
];

/// `X64/EFI/OC/Drivers` of OpenCore-1.0.8-RELEASE.zip.
pub const OC_DRIVERS: &[&str] = &[
    "ArpDxe.efi",
    "AudioDxe.efi",
    "BiosVideo.efi",
    "CrScreenshotDxe.efi",
    "Dhcp4Dxe.efi",
    "Dhcp6Dxe.efi",
    "DnsDxe.efi",
    "DpcDxe.efi",
    "Ext4Dxe.efi",
    "FirmwareSettingsEntry.efi",
    "Hash2DxeCrypto.efi",
    "HiiDatabase.efi",
    "HttpBootDxe.efi",
    "HttpDxe.efi",
    "HttpUtilitiesDxe.efi",
    "Ip4Dxe.efi",
    "Ip6Dxe.efi",
    "MnpDxe.efi",
    "Mtftp4Dxe.efi",
    "Mtftp6Dxe.efi",
    "NvmExpressDxe.efi",
    "OpenCanopy.efi",
    "OpenHfsPlus.efi",
    "OpenLegacyBoot.efi",
    "OpenLinuxBoot.efi",
    "OpenNetworkBoot.efi",
    "OpenNtfsDxe.efi",
    "OpenPartitionDxe.efi",
    "OpenRuntime.efi",
    "OpenUsbKbDxe.efi",
    "OpenVariableRuntimeDxe.efi",
    "Ps2KeyboardDxe.efi",
    "Ps2MouseDxe.efi",
    "RamDiskDxe.efi",
    "ResetNvramEntry.efi",
    "RngDxe.efi",
    "SnpDxe.efi",
    "TcpDxe.efi",
    "TlsDxe.efi",
    "ToggleSipEntry.efi",
    "Udp4Dxe.efi",
    "Udp6Dxe.efi",
    "UefiPxeBcDxe.efi",
    "UsbMouseDxe.efi",
    "Virtio10.efi",
    "VirtioBlkDxe.efi",
    "VirtioGpuDxe.efi",
    "VirtioNetDxe.efi",
    "VirtioPciDeviceDxe.efi",
    "VirtioScsiDxe.efi",
    "VirtioSerialDxe.efi",
    "XhciDxe.efi",
];

/// Drivers in acidanthera/OcBinaryData `Drivers/`.
pub const OCBINARYDATA_DRIVERS: &[&str] = &[
    "ExFatDxe.efi",
    "HfsPlus.efi",
    "HfsPlus32.efi",
    "HfsPlusLegacy.efi",
];

/// `X64/EFI/OC/Tools` of OpenCore-1.0.8-RELEASE.zip.
pub const OC_TOOLS: &[&str] = &[
    "BootKicker.efi",
    "ChipTune.efi",
    "CleanNvram.efi",
    "ControlMsrE2.efi",
    "CsrUtil.efi",
    "FontTester.efi",
    "GopStop.efi",
    "KeyTester.efi",
    "ListPartitions.efi",
    "MmapDump.efi",
    "OpenControl.efi",
    "OpenShell.efi",
    "ResetSystem.efi",
    "RtcRw.efi",
    "TpmInfo.efi",
];

/// Bundles that ship without an executable (Info.plist-only injectors).
const CODELESS: &[&str] = &[
    "AppleMCEReporterDisabler.kext",
    "ASPP-Override.kext",
    "XLNCUSBFix.kext",
    "BrcmBluetoothInjector.kext",
    "BrcmBluetoothInjectorLegacy.kext",
    "IntelBluetoothInjector.kext",
    "UTBDefault.kext",
    "XHCI-unsupported.kext",
    "AirPortBrcm4360_Injector.kext",
    "AirPortBrcmNIC_Injector.kext",
];

const LILU: &str = "Lilu.kext";
const VIRTUALSMC: &str = "VirtualSMC.kext";

fn stem(bundle: &str) -> &str {
    bundle.strip_suffix(".kext").unwrap_or(bundle)
}

/// "21.0.0" → (21, 0, 0) for comparing kernel bounds.
fn kernel_bound(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// A plugin loads only inside its parent's range: the later MinKernel and
/// the earlier MaxKernel of the two win (as in `kernel_add`).
fn plugin_bound<'a>(own: Option<&'a str>, parent: Option<&'a str>, later: bool) -> Option<&'a str> {
    let own = own.filter(|s| !s.trim().is_empty());
    let parent = parent.filter(|s| !s.trim().is_empty());
    match (own, parent) {
        (Some(o), Some(p)) => match (kernel_bound(o), kernel_bound(p)) {
            (Some(vo), Some(vp)) if (vp > vo) == later => Some(p),
            _ => Some(o),
        },
        (o, p) => o.or(p),
    }
}

fn add_entry(
    bundle_path: &str,
    name: &str,
    enabled: bool,
    min: Option<&str>,
    max: Option<&str>,
) -> KernelAddEntry {
    let executable = if CODELESS.iter().any(|c| c.eq_ignore_ascii_case(name)) {
        String::new()
    } else if name == "AAAMouSSE.kext" {
        "Contents/MacOS/MouSSE".into()
    } else if name == "AirPortBrcmNIC-Tahoe.kext" {
        "Contents/MacOS/AirPortBrcmNIC".into()
    } else {
        format!("Contents/MacOS/{}", stem(name))
    };
    KernelAddEntry {
        arch: "Any".into(),
        bundle_path: bundle_path.into(),
        comment: name.into(),
        enabled,
        executable_path: executable,
        max_kernel: max.unwrap_or_default().into(),
        min_kernel: min.unwrap_or_default().into(),
        plist_path: "Contents/Info.plist".into(),
        bundle_id: String::new(),
    }
}

/// Kernel->Add as `kernel_add::build_kernel_add` would produce it from the
/// installed bundles: Lilu first, VirtualSMC second, then selection order,
/// each bundle followed by its plugins.
pub fn kernel_add(plan: &BuildPlan) -> Vec<KernelAddEntry> {
    let rank = |bundle: &str| match bundle {
        b if b.eq_ignore_ascii_case(LILU) => 0,
        b if b.eq_ignore_ascii_case(VIRTUALSMC) => 1,
        _ => 2,
    };
    let mut selections: Vec<_> = plan.kexts.iter().collect();
    selections.sort_by_key(|k| rank(&k.bundle));
    let mut entries = Vec::new();
    for k in selections {
        let (min, max) = (k.min_kernel.as_deref(), k.max_kernel.as_deref());
        entries.push(add_entry(&k.bundle, &k.bundle, k.enabled, min, max));
        for p in &k.plugins {
            let path = format!("{}/Contents/PlugIns/{}", k.bundle, p.bundle);
            entries.push(add_entry(
                &path,
                &p.bundle,
                k.enabled && p.enabled,
                plugin_bound(p.min_kernel.as_deref(), min, true),
                plugin_bound(p.max_kernel.as_deref(), max, false),
            ));
        }
    }
    entries
}

pub fn identity(plan: &BuildPlan) -> PlatformIdentity {
    PlatformIdentity {
        model: plan.smbios.model.clone(),
        serial: "C02XG0FDH7JY".into(),
        mlb: "C02839303QXH69FJA".into(),
        system_uuid: "DBB364D6-44B2-4A02-B922-AB4396F16DA8".into(),
        rom: "112233445566".into(),
    }
}

/// Driver files the EFI would contain (the text picker never ships OpenCanopy).
pub fn driver_files(plan: &BuildPlan, picker: PickerStyle) -> Vec<String> {
    plan.drivers
        .iter()
        .filter(|d| {
            picker == PickerStyle::Graphical || !d.path.eq_ignore_ascii_case("OpenCanopy.efi")
        })
        .map(|d| d.path.clone())
        .collect()
}

pub fn write(
    sample: &[u8],
    plan: &BuildPlan,
    kernel_add: &[KernelAddEntry],
    identity: &PlatformIdentity,
    picker: PickerStyle,
) -> Result<Vec<u8>, AppError> {
    let ssdt_files: Vec<String> = plan.ssdts.iter().map(|s| s.file_name.clone()).collect();
    let driver_files = driver_files(plan, picker);
    write_config(
        sample,
        &ConfigInputs {
            plan,
            kernel_add,
            identity,
            ssdt_files: &ssdt_files,
            driver_files: &driver_files,
            tool_files: &plan.tools,
        },
    )
}

/// The Sample.plist dictionary each `SettingMap` of the plan overrides.
pub fn setting_maps(plan: &BuildPlan) -> [(&'static str, &SettingMap); 13] {
    [
        ("ACPI/Quirks", &plan.acpi_quirks),
        ("Booter/Quirks", &plan.booter_quirks),
        ("Kernel/Quirks", &plan.kernel_quirks),
        ("Kernel/Emulate", &plan.kernel_emulate),
        ("Misc/Boot", &plan.misc_boot),
        ("Misc/Debug", &plan.misc_debug),
        ("Misc/Security", &plan.misc_security),
        ("NVRAM", &plan.nvram_settings),
        ("PlatformInfo", &plan.platform_info),
        ("UEFI/Quirks", &plan.uefi_quirks),
        ("UEFI/APFS", &plan.uefi_apfs),
        ("UEFI/Output", &plan.uefi_output),
        ("UEFI/Input", &plan.uefi_input),
    ]
}

pub fn sample_root() -> plist::Dictionary {
    match Value::from_reader(Cursor::new(SAMPLE)).expect("Sample.plist parses") {
        Value::Dictionary(d) => d,
        _ => panic!("Sample.plist root is not a dictionary"),
    }
}

fn lookup<'a>(root: &'a plist::Dictionary, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('/');
    let mut value = root.get(parts.next()?)?;
    for part in parts {
        value = value.as_dictionary()?.get(part)?;
    }
    Some(value)
}

/// Every override key must exist in the Sample section with the same type.
pub fn schema_errors(sample: &plist::Dictionary, plan: &BuildPlan) -> Vec<String> {
    let mut errors = Vec::new();
    for (section, map) in setting_maps(plan) {
        for (key, scalar) in map {
            let path = format!("{section}/{key}");
            let Some(value) = lookup(sample, &path) else {
                errors.push(format!("{path} is not in Sample.plist"));
                continue;
            };
            let fits = matches!(
                (scalar, value),
                (PlistScalar::Bool(_), Value::Boolean(_))
                    | (PlistScalar::Int(_), Value::Integer(_))
                    | (PlistScalar::Str(_), Value::String(_))
                    | (PlistScalar::Data(_), Value::Data(_))
            );
            if !fits {
                errors.push(format!(
                    "{path}: plan has {scalar:?}, Sample.plist has {value:?}"
                ));
            }
        }
    }
    errors
}
