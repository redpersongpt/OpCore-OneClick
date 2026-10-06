//! Firmware settings probe: UEFI boot, Secure Boot, VT-x, VT-d and Above 4G
//! decoding, read from the running OS.
//!
//! Every check ends up as "ok" (nothing to change), "action" (change it in
//! the firmware setup), "unknown" (the OS does not tell) or
//! "not_applicable" (macOS host: the EFI is built for another PC).
//! A missing permission or an unreadable value is "unknown", never a
//! failed requirement.

use std::time::Duration;

use tracing::info;

use crate::contracts::{FirmwareCheck, FirmwareReport};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Action,
    Unknown,
    NotApplicable,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Action => "action",
            Status::Unknown => "unknown",
            Status::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CpuMaker {
    Intel,
    Amd,
    #[default]
    Other,
}

impl CpuMaker {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn from_vendor(vendor: &str) -> Self {
        let vendor = vendor.to_lowercase();
        if vendor.contains("intel") {
            CpuMaker::Intel
        } else if vendor.contains("amd") {
            CpuMaker::Amd
        } else {
            CpuMaker::Other
        }
    }
}

/// What the host told us; `None` means it could not be determined.
#[derive(Debug, Clone, Default)]
struct FirmwareFacts {
    /// The host is not the PC the EFI is for (macOS builds for another PC).
    remote: bool,
    uefi: Option<bool>,
    uefi_source: String,
    secure_boot: Option<bool>,
    secure_boot_source: String,
    cpu: CpuMaker,
    /// VT-x / AMD-V usable (switched on in the firmware).
    virtualization: Option<bool>,
    virtualization_source: String,
    /// A hypervisor runs underneath (Hyper-V, VBS, WSL2, a VM).
    hypervisor: bool,
    /// ACPI DMAR (Intel VT-d) / IVRS (AMD-Vi) tables published.
    dmar: Option<bool>,
    ivrs: Option<bool>,
    iommu_source: String,
    /// A device is mapped above 4 GB, so Above 4G decoding is on.
    high_mmio: Option<bool>,
    high_mmio_source: String,
    bios_vendor: Option<String>,
    bios_version: Option<String>,
}

fn check(name: &str, required: bool, status: Status, evidence: impl Into<String>) -> FirmwareCheck {
    FirmwareCheck { name: name.to_string(), status: status.as_str().to_string(), evidence: evidence.into(), required }
}

fn with_source(text: &str, source: &str) -> String {
    if source.is_empty() {
        text.to_string()
    } else {
        format!("{text} ({source})")
    }
}

fn uefi_check(facts: &FirmwareFacts) -> FirmwareCheck {
    let name = "UEFI Boot Mode";
    match facts.uefi {
        Some(true) => check(name, true, Status::Ok, with_source("This OS was started in UEFI mode", &facts.uefi_source)),
        Some(false) => check(
            name,
            true,
            Status::Action,
            with_source(
                "This OS was started in legacy BIOS (CSM) mode. Set the boot mode to UEFI and disable CSM before booting the USB drive",
                &facts.uefi_source,
            ),
        ),
        None => check(name, true, Status::Unknown, "The boot mode could not be read; make sure the firmware boots in UEFI mode with CSM off"),
    }
}

fn secure_boot_check(facts: &FirmwareFacts) -> FirmwareCheck {
    let name = "Secure Boot";
    match facts.secure_boot {
        Some(false) => check(name, true, Status::Ok, with_source("Secure Boot is off", &facts.secure_boot_source)),
        Some(true) => check(
            name,
            true,
            Status::Action,
            with_source(
                "Secure Boot is on; turn it off in the firmware setup. If Windows uses BitLocker or device encryption, save the recovery key first",
                &facts.secure_boot_source,
            ),
        ),
        None if facts.uefi == Some(false) => check(name, true, Status::Ok, "Not active: the PC started in legacy BIOS mode"),
        None => check(name, true, Status::Unknown, "The Secure Boot state could not be read; make sure it is off"),
    }
}

fn virtualization_check(facts: &FirmwareFacts) -> FirmwareCheck {
    let name = "CPU Virtualisation (VT-x / AMD-V)";
    if facts.hypervisor {
        return check(name, false, Status::Ok, "Enabled: a hypervisor (Hyper-V, VBS, WSL 2 or a VM) is running");
    }
    match (facts.virtualization, facts.cpu) {
        (Some(true), _) => check(name, false, Status::Ok, with_source("Enabled", &facts.virtualization_source)),
        (Some(false), CpuMaker::Intel) => check(
            name,
            false,
            Status::Action,
            with_source("VT-x is off or unsupported; enable it in the firmware setup if the option exists", &facts.virtualization_source),
        ),
        (Some(false), _) => check(name, false, Status::Ok, with_source("SVM is off; macOS does not need it", &facts.virtualization_source)),
        (None, _) => check(name, false, Status::Unknown, "The virtualisation state could not be read"),
    }
}

fn iommu_check(facts: &FirmwareFacts) -> FirmwareCheck {
    let name = "VT-d / AMD-Vi (IOMMU)";
    let source = &facts.iommu_source;
    match facts.cpu {
        CpuMaker::Amd => match facts.ivrs {
            Some(true) => check(name, false, Status::Action, with_source("The IOMMU is on; disable IOMMU (AMD-Vi) in the firmware setup", source)),
            Some(false) => check(name, false, Status::Ok, with_source("The IOMMU is off", source)),
            None => check(name, false, Status::Unknown, "The IOMMU state could not be read; disable IOMMU in the firmware setup"),
        },
        _ => match facts.dmar {
            Some(true) => check(
                name,
                false,
                Status::Ok,
                with_source("VT-d is on; this is fine because the EFI makes macOS ignore it (DisableIoMapper)", source),
            ),
            Some(false) => check(name, false, Status::Ok, with_source("VT-d is off", source)),
            None => check(name, false, Status::Unknown, "The VT-d state could not be read"),
        },
    }
}

fn above_4g_check(facts: &FirmwareFacts) -> FirmwareCheck {
    let name = "Above 4G Decoding";
    match facts.high_mmio {
        Some(true) => check(name, false, Status::Ok, with_source("Enabled: devices are mapped above 4 GB", &facts.high_mmio_source)),
        _ => {
            let advice = if facts.cpu == CpuMaker::Amd {
                "Not detected. Enable Above 4G Decoding in the firmware setup (AMD boards need it, or the npci=0x3000 boot argument)"
            } else {
                "Not detected. Enable Above 4G Decoding in the firmware setup if the option exists"
            };
            check(name, false, Status::Unknown, with_source(advice, &facts.high_mmio_source))
        }
    }
}

fn confidence(checks: &[&FirmwareCheck]) -> &'static str {
    let known = checks.iter().filter(|c| c.status == "ok" || c.status == "action").count();
    match known {
        4.. => "high",
        2..=3 => "medium",
        _ => "low",
    }
}

fn build_report(facts: &FirmwareFacts) -> FirmwareReport {
    if facts.remote {
        return not_applicable_report(facts);
    }
    let report = FirmwareReport {
        uefi_mode: uefi_check(facts),
        secure_boot: secure_boot_check(facts),
        vt_x: virtualization_check(facts),
        vt_d: iommu_check(facts),
        above_4g: above_4g_check(facts),
        bios_vendor: facts.bios_vendor.clone(),
        bios_version: facts.bios_version.clone(),
        confidence: String::new(),
    };
    let confidence = confidence(&[&report.uefi_mode, &report.secure_boot, &report.vt_x, &report.vt_d, &report.above_4g]);
    FirmwareReport { confidence: confidence.to_string(), ..report }
}

/// The macOS host builds an EFI for another PC: nothing here applies.
fn not_applicable_report(facts: &FirmwareFacts) -> FirmwareReport {
    let na = |name: &str, required: bool| {
        check(name, required, Status::NotApplicable, "Check this setting in the firmware setup of the PC you build the EFI for")
    };
    FirmwareReport {
        uefi_mode: na("UEFI Boot Mode", true),
        secure_boot: na("Secure Boot", true),
        vt_x: na("CPU Virtualisation (VT-x / AMD-V)", false),
        vt_d: na("VT-d / AMD-Vi (IOMMU)", false),
        above_4g: na("Above 4G Decoding", false),
        bios_vendor: facts.bios_vendor.clone(),
        bios_version: facts.bios_version.clone(),
        confidence: "not_applicable".to_string(),
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", test))]
fn non_empty(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

#[tauri::command]
pub async fn probe_firmware() -> Result<FirmwareReport, AppError> {
    info!("Probing firmware settings");
    let report = build_report(&host_facts().await?);
    info!(confidence = %report.confidence, "Firmware probe finished");
    Ok(report)
}

async fn host_facts() -> Result<FirmwareFacts, AppError> {
    #[cfg(target_os = "windows")]
    return Ok(windows::facts().await);
    #[cfg(target_os = "linux")]
    return Ok(linux::facts().await);
    #[cfg(target_os = "macos")]
    return Ok(FirmwareFacts {
        remote: true,
        bios_vendor: Some("Apple".to_string()),
        bios_version: macos::firmware_version().await,
        ..Default::default()
    });
    #[allow(unreachable_code)]
    Err(AppError::new("UNSUPPORTED_PLATFORM", "Firmware probing is not supported on this OS"))
}

// ── Windows ─────────────────────────────────────────────────────────────────

/// Facts PowerShell reads without changing anything (JSON on one line).
#[cfg(any(target_os = "windows", test))]
const WINDOWS_PROBE_SCRIPT: &str = r#"
try { Remove-TypeData -TypeName System.Array -ErrorAction Stop } catch {}
$r = [ordered]@{}
try { $b = Get-CimInstance -ClassName Win32_BIOS -ErrorAction Stop; $r.BiosVendor = [string]$b.Manufacturer; $r.BiosVersion = [string]$b.SMBIOSBIOSVersion } catch {}
try { $r.SecureBoot = [int](Get-ItemPropertyValue -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\SecureBoot\State' -Name UEFISecureBootEnabled -ErrorAction Stop) } catch {}
try { $r.PEFirmwareType = [int](Get-ItemPropertyValue -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control' -Name PEFirmwareType -ErrorAction Stop) } catch {}
try { $r.HypervisorPresent = [bool](Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop).HypervisorPresent } catch {}
try {
  $cpus = @(Get-CimInstance -ClassName Win32_Processor -ErrorAction Stop)
  $r.CpuManufacturer = [string]$cpus[0].Manufacturer
  $r.VirtualizationFirmware = @($cpus | ForEach-Object { [bool]$_.VirtualizationFirmwareEnabled })
} catch {}
try { $r.HighMmio = [bool](@(Get-CimInstance -ClassName Win32_DeviceMemoryAddress -ErrorAction Stop | Where-Object { [uint64]$_.StartingAddress -gt 4294967295 }).Count -gt 0) } catch {}
[pscustomobject]$r | ConvertTo-Json -Compress
"#;

/// Parsed output of [`WINDOWS_PROBE_SCRIPT`].
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct WindowsProbe {
    bios_vendor: Option<String>,
    bios_version: Option<String>,
    secure_boot: Option<bool>,
    /// PEFirmwareType: 1 = BIOS, 2 = UEFI.
    pe_firmware_type: Option<u64>,
    hypervisor: Option<bool>,
    cpu_manufacturer: Option<String>,
    virtualization_firmware: Vec<bool>,
    high_mmio: Option<bool>,
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_probe(json: &str) -> WindowsProbe {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json.trim()) else { return WindowsProbe::default() };
    let text = |key: &str| non_empty(value.get(key).and_then(|v| v.as_str()));
    let number = |key: &str| value.get(key).and_then(|v| v.as_u64());
    let boolean = |key: &str| value.get(key).and_then(|v| v.as_bool());
    let virtualization_firmware = match value.get("VirtualizationFirmware") {
        Some(serde_json::Value::Array(list)) => list.iter().filter_map(|v| v.as_bool()).collect(),
        Some(serde_json::Value::Bool(single)) => vec![*single],
        // Windows PowerShell 5.1 may wrap arrays as {"value": [...], "Count": n}.
        Some(other) => other.get("value").and_then(|v| v.as_array()).map(|l| l.iter().filter_map(|v| v.as_bool()).collect()).unwrap_or_default(),
        None => Vec::new(),
    };
    WindowsProbe {
        bios_vendor: text("BiosVendor"),
        bios_version: text("BiosVersion"),
        secure_boot: number("SecureBoot").map(|v| v == 1),
        pe_firmware_type: number("PEFirmwareType"),
        hypervisor: boolean("HypervisorPresent"),
        cpu_manufacturer: text("CpuManufacturer"),
        virtualization_firmware,
        high_mmio: boolean("HighMmio"),
    }
}

/// ACPI table signatures as returned by `EnumSystemFirmwareTables('ACPI')`.
#[cfg(any(target_os = "windows", test))]
fn acpi_table_present(ids: &[u8], signature: &[u8; 4]) -> bool {
    let reversed = [signature[3], signature[2], signature[1], signature[0]];
    ids.as_chunks::<4>().0.iter().any(|id| id == signature || *id == reversed)
}

#[cfg(any(target_os = "windows", test))]
fn windows_facts_from(probe: &WindowsProbe, firmware_type_uefi: Option<bool>, acpi_ids: Option<&[u8]>) -> FirmwareFacts {
    let (uefi, uefi_source) = match (firmware_type_uefi, probe.pe_firmware_type) {
        (Some(uefi), _) => (Some(uefi), "GetFirmwareType"),
        (None, Some(2)) => (Some(true), "PEFirmwareType"),
        (None, Some(1)) => (Some(false), "PEFirmwareType"),
        _ => (None, ""),
    };
    let virtualization = if probe.virtualization_firmware.is_empty() {
        None
    } else {
        Some(probe.virtualization_firmware.iter().all(|enabled| *enabled))
    };
    FirmwareFacts {
        remote: false,
        uefi,
        uefi_source: uefi_source.to_string(),
        // The value Windows itself reports in System Information.
        secure_boot: probe.secure_boot,
        secure_boot_source: "UEFISecureBootEnabled registry value".to_string(),
        cpu: probe.cpu_manufacturer.as_deref().map(CpuMaker::from_vendor).unwrap_or_default(),
        virtualization,
        virtualization_source: "Win32_Processor.VirtualizationFirmwareEnabled".to_string(),
        hypervisor: probe.hypervisor.unwrap_or(false),
        dmar: acpi_ids.map(|ids| acpi_table_present(ids, b"DMAR")),
        ivrs: acpi_ids.map(|ids| acpi_table_present(ids, b"IVRS")),
        iommu_source: "ACPI tables".to_string(),
        high_mmio: probe.high_mmio.filter(|found| *found),
        high_mmio_source: "Win32_DeviceMemoryAddress".to_string(),
        bios_vendor: probe.bios_vendor.clone(),
        bios_version: probe.bios_version.clone(),
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use crate::services::process;
    use tracing::warn;
    use windows_sys::Win32::System::SystemInformation::{
        EnumSystemFirmwareTables, GetFirmwareType, FirmwareTypeBios, FirmwareTypeUefi, FIRMWARE_TYPE,
    };

    /// `GetFirmwareType`: how this Windows was booted (no admin needed).
    fn firmware_type_uefi() -> Option<bool> {
        let mut kind: FIRMWARE_TYPE = 0;
        // SAFETY: `kind` is a valid out pointer.
        if unsafe { GetFirmwareType(&mut kind) } == 0 {
            return None;
        }
        if kind == FirmwareTypeUefi {
            Some(true)
        } else if kind == FirmwareTypeBios {
            Some(false)
        } else {
            None
        }
    }

    /// Signatures of the ACPI tables the firmware published.
    fn acpi_table_ids() -> Option<Vec<u8>> {
        let provider = u32::from_be_bytes(*b"ACPI");
        // SAFETY: a null buffer of size 0 asks for the required size.
        let size = unsafe { EnumSystemFirmwareTables(provider, std::ptr::null_mut(), 0) };
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        // SAFETY: `buffer` holds `size` bytes.
        let written = unsafe { EnumSystemFirmwareTables(provider, buffer.as_mut_ptr(), size) };
        if written == 0 || written > size {
            return None;
        }
        buffer.truncate(written as usize);
        Some(buffer)
    }

    pub(super) async fn facts() -> FirmwareFacts {
        let probe = match process::powershell(WINDOWS_PROBE_SCRIPT, Duration::from_secs(30)).await {
            Ok(output) => parse_windows_probe(&output.stdout),
            Err(error) => {
                warn!("Firmware probe script failed: {}", error.message);
                WindowsProbe::default()
            }
        };
        let ids = acpi_table_ids();
        windows_facts_from(&probe, firmware_type_uefi(), ids.as_deref())
    }
}

// ── Linux ───────────────────────────────────────────────────────────────────

/// UEFI variables are 4 attribute bytes followed by the value.
#[cfg(any(target_os = "linux", test))]
fn efivar_bool(bytes: &[u8]) -> Option<bool> {
    match bytes.get(4) {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

/// `mokutil --sb-state` output.
#[cfg(any(target_os = "linux", test))]
fn mokutil_secure_boot(output: &str) -> Option<bool> {
    let lower = output.to_lowercase();
    if lower.contains("secureboot enabled") {
        Some(true)
    } else if lower.contains("secureboot disabled") || lower.contains("not supported") {
        Some(false)
    } else {
        None
    }
}

/// (vendor, flags of the first CPU) from /proc/cpuinfo.
#[cfg(any(target_os = "linux", test))]
fn parse_cpuinfo(text: &str) -> (CpuMaker, Vec<String>) {
    let mut vendor = CpuMaker::Other;
    let mut flags = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        match key.trim() {
            "vendor_id" if vendor == CpuMaker::Other => vendor = CpuMaker::from_vendor(value),
            "flags" if flags.is_empty() => flags = value.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    (vendor, flags)
}

/// Any PCI resource (`/sys/bus/pci/devices/*/resource`) mapped above 4 GB.
#[cfg(any(target_os = "linux", test))]
fn resource_above_4g(text: &str) -> bool {
    let parse = |v: &str| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok();
    text.lines().any(|line| {
        let mut fields = line.split_whitespace();
        match (fields.next().and_then(parse), fields.next().and_then(parse)) {
            (Some(start), Some(end)) => start >= 0x1_0000_0000 && end > start,
            _ => false,
        }
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::services::process;
    use std::path::Path;

    const SECURE_BOOT_VAR: &str = "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

    fn read_trimmed(path: &str) -> Option<String> {
        non_empty(std::fs::read_to_string(path).ok().as_deref())
    }

    async fn secure_boot(uefi: bool) -> (Option<bool>, String) {
        if !uefi {
            return (None, String::new());
        }
        if let Some(state) = std::fs::read(SECURE_BOOT_VAR).ok().as_deref().and_then(efivar_bool) {
            return (Some(state), "SecureBoot EFI variable".to_string());
        }
        if let Some(mokutil) = process::find_in_path("mokutil") {
            let program = mokutil.to_string_lossy().into_owned();
            if let Ok(output) = process::run(&program, &["--sb-state"], Duration::from_secs(10)).await {
                if let Some(state) = mokutil_secure_boot(&format!("{}\n{}", output.stdout, output.stderr)) {
                    return (Some(state), "mokutil --sb-state".to_string());
                }
            }
        }
        (None, String::new())
    }

    fn high_mmio() -> Option<bool> {
        let entries = std::fs::read_dir("/sys/bus/pci/devices").ok()?;
        let mut read_any = false;
        for entry in entries.flatten() {
            if let Ok(text) = std::fs::read_to_string(entry.path().join("resource")) {
                read_any = true;
                if resource_above_4g(&text) {
                    return Some(true);
                }
            }
        }
        read_any.then_some(false)
    }

    pub(super) async fn facts() -> FirmwareFacts {
        let uefi = Path::new("/sys/firmware/efi").exists();
        let (secure_boot, secure_boot_source) = secure_boot(uefi).await;
        let (cpu, flags) = std::fs::read_to_string("/proc/cpuinfo").map(|t| parse_cpuinfo(&t)).unwrap_or_default();
        let has = |flag: &str| flags.iter().any(|f| f == flag);
        // The kernel drops the vmx flag when the firmware locked VT-x off.
        let virtualization = (!flags.is_empty()).then(|| has("vmx") || has("svm"));
        let tables = Path::new("/sys/firmware/acpi/tables");
        let tables_readable = tables.is_dir();
        FirmwareFacts {
            remote: false,
            uefi: Some(uefi),
            uefi_source: "/sys/firmware/efi".to_string(),
            secure_boot,
            secure_boot_source,
            cpu,
            virtualization,
            virtualization_source: "/proc/cpuinfo".to_string(),
            hypervisor: has("hypervisor"),
            dmar: tables_readable.then(|| tables.join("DMAR").exists()),
            ivrs: tables_readable.then(|| tables.join("IVRS").exists()),
            iommu_source: "/sys/firmware/acpi/tables".to_string(),
            high_mmio: high_mmio().filter(|found| *found),
            high_mmio_source: "PCI resources".to_string(),
            bios_vendor: read_trimmed("/sys/class/dmi/id/bios_vendor"),
            bios_version: read_trimmed("/sys/class/dmi/id/bios_version"),
        }
    }
}

// ── macOS ───────────────────────────────────────────────────────────────────

/// "System Firmware Version" (macOS 11+) or "Boot ROM Version" from
/// `system_profiler SPHardwareDataType`.
#[cfg(any(target_os = "macos", test))]
fn parse_firmware_version(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        (key == "System Firmware Version" || key == "Boot ROM Version").then(|| value.trim().to_string()).filter(|v| !v.is_empty())
    })
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::services::process;

    pub(super) async fn firmware_version() -> Option<String> {
        let output = process::run("/usr/sbin/system_profiler", &["SPHardwareDataType"], Duration::from_secs(20)).await.ok()?;
        parse_firmware_version(&output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intel_desktop() -> FirmwareFacts {
        FirmwareFacts {
            uefi: Some(true),
            secure_boot: Some(false),
            cpu: CpuMaker::Intel,
            virtualization: Some(true),
            dmar: Some(true),
            ivrs: Some(false),
            high_mmio: Some(true),
            bios_vendor: Some("American Megatrends Inc.".into()),
            ..Default::default()
        }
    }

    #[test]
    fn statuses_use_the_contract_vocabulary() {
        let report = build_report(&intel_desktop());
        for c in [&report.uefi_mode, &report.secure_boot, &report.vt_x, &report.vt_d, &report.above_4g] {
            assert_eq!(c.status, "ok", "{}", c.name);
        }
        assert_eq!(report.confidence, "high");
        assert!(report.uefi_mode.required && report.secure_boot.required && !report.vt_x.required);

        let na = build_report(&FirmwareFacts { remote: true, bios_version: Some("2069.0.0".into()), ..intel_desktop() });
        for c in [&na.uefi_mode, &na.secure_boot, &na.vt_x, &na.vt_d, &na.above_4g] {
            assert_eq!(c.status, "not_applicable");
        }
        assert_eq!(na.confidence, "not_applicable");
        assert_eq!(na.bios_version.as_deref(), Some("2069.0.0"));
    }

    #[test]
    fn settings_to_change_are_actions() {
        let facts = FirmwareFacts { uefi: Some(false), secure_boot: None, ..intel_desktop() };
        let report = build_report(&facts);
        assert_eq!(report.uefi_mode.status, "action");
        // Legacy boot: Secure Boot cannot be active.
        assert_eq!(report.secure_boot.status, "ok");

        let facts = FirmwareFacts { secure_boot: Some(true), ..intel_desktop() };
        let report = build_report(&facts);
        assert_eq!(report.secure_boot.status, "action");
        assert!(report.secure_boot.evidence.contains("BitLocker"));

        let amd = FirmwareFacts { cpu: CpuMaker::Amd, ivrs: Some(true), high_mmio: None, ..intel_desktop() };
        let report = build_report(&amd);
        assert_eq!(report.vt_d.status, "action");
        assert_eq!(report.above_4g.status, "unknown");
        assert!(report.above_4g.evidence.contains("npci=0x3000"));
    }

    #[test]
    fn unreadable_values_are_unknown_not_failures() {
        let report = build_report(&FirmwareFacts::default());
        for c in [&report.uefi_mode, &report.secure_boot, &report.vt_x, &report.vt_d, &report.above_4g] {
            assert_eq!(c.status, "unknown", "{}", c.name);
        }
        assert_eq!(report.confidence, "low");
    }

    #[test]
    fn a_hypervisor_means_virtualisation_is_on() {
        let facts = FirmwareFacts { hypervisor: true, virtualization: Some(false), ..intel_desktop() };
        assert_eq!(build_report(&facts).vt_x.status, "ok");
        let facts = FirmwareFacts { virtualization: Some(false), ..intel_desktop() };
        assert_eq!(build_report(&facts).vt_x.status, "action");
        let facts = FirmwareFacts { virtualization: Some(false), cpu: CpuMaker::Amd, ..intel_desktop() };
        assert_eq!(build_report(&facts).vt_x.status, "ok");
    }

    #[test]
    fn windows_probe_parsing() {
        let json = r#"{"BiosVendor":"ASUSTeK COMPUTER INC.","BiosVersion":"2801 ","SecureBoot":1,"PEFirmwareType":2,
            "HypervisorPresent":true,"CpuManufacturer":"GenuineIntel","VirtualizationFirmware":[false,false],"HighMmio":true}"#;
        let probe = parse_windows_probe(json);
        assert_eq!(probe.bios_version.as_deref(), Some("2801"));
        assert_eq!(probe.secure_boot, Some(true));
        assert_eq!(probe.virtualization_firmware, [false, false]);
        let ids = b"FACPDMARAPICSSDT".to_vec();
        let facts = windows_facts_from(&probe, None, Some(&ids));
        assert_eq!(facts.uefi, Some(true));
        assert_eq!(facts.uefi_source, "PEFirmwareType");
        assert_eq!(facts.cpu, CpuMaker::Intel);
        assert_eq!(facts.dmar, Some(true));
        assert_eq!(facts.ivrs, Some(false));
        let report = build_report(&facts);
        // Secure Boot on is the one thing to change; Hyper-V explains VT-x.
        assert_eq!(report.secure_boot.status, "action");
        assert_eq!(report.uefi_mode.status, "ok");
        assert_eq!(report.vt_x.status, "ok");

        // Non-admin or legacy boot: the key is missing, nothing is guessed.
        let legacy = parse_windows_probe(r#"{"PEFirmwareType":1,"VirtualizationFirmware":{"value":[true],"Count":1}}"#);
        assert_eq!(legacy.virtualization_firmware, [true]);
        let facts = windows_facts_from(&legacy, Some(false), None);
        assert_eq!(facts.uefi_source, "GetFirmwareType");
        let report = build_report(&facts);
        assert_eq!(report.uefi_mode.status, "action");
        assert_eq!(report.secure_boot.status, "ok");
        assert_eq!(report.vt_d.status, "unknown");

        assert_eq!(parse_windows_probe("garbage"), WindowsProbe::default());
        assert!(acpi_table_present(b"RAMD", b"DMAR"));
        assert!(!acpi_table_present(b"FACPAPIC", b"IVRS"));
        assert!(WINDOWS_PROBE_SCRIPT.contains("UEFISecureBootEnabled"));
    }

    #[test]
    fn linux_parsers() {
        assert_eq!(efivar_bool(&[6, 0, 0, 0, 1]), Some(true));
        assert_eq!(efivar_bool(&[6, 0, 0, 0, 0]), Some(false));
        assert_eq!(efivar_bool(&[6, 0]), None);
        assert_eq!(mokutil_secure_boot("SecureBoot enabled\n"), Some(true));
        assert_eq!(mokutil_secure_boot("SecureBoot disabled\nPlatform is in Setup Mode"), Some(false));
        assert_eq!(mokutil_secure_boot("EFI variables are not supported on this system"), Some(false));
        assert_eq!(mokutil_secure_boot("Failed to read"), None);

        let cpuinfo = "processor\t: 0\nvendor_id\t: AuthenticAMD\nflags\t\t: fpu vme svm sse4_2 avx2\n\nprocessor\t: 1\nvendor_id\t: AuthenticAMD\nflags\t\t: fpu\n";
        let (vendor, flags) = parse_cpuinfo(cpuinfo);
        assert_eq!(vendor, CpuMaker::Amd);
        assert!(flags.contains(&"svm".to_string()));
        assert_eq!(flags.len(), 5);

        let low = "0x00000000f6000000 0x00000000f6ffffff 0x0000000000040200\n0x0000000000000000 0x0000000000000000 0x0000000000000000\n";
        assert!(!resource_above_4g(low));
        let high = format!("{low}0x0000006000000000 0x00000063ffffffff 0x000000000014220c\n");
        assert!(resource_above_4g(&high));
    }

    #[test]
    fn macos_firmware_version() {
        let text = "Hardware:\n\n    Hardware Overview:\n\n      Model Name: Mac mini\n      System Firmware Version: 11881.1.1\n";
        assert_eq!(parse_firmware_version(text).as_deref(), Some("11881.1.1"));
        assert_eq!(parse_firmware_version("      Boot ROM Version: 220.270.99.0.0\n").as_deref(), Some("220.270.99.0.0"));
        assert_eq!(parse_firmware_version("nothing"), None);
    }
}
