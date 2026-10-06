//! Hardware scanner. See `platform::scan` for the contract.
//!
//! Everything comes from world-readable sysfs/procfs (no lspci, no
//! dmidecode) plus in-process CPUID; only the ACPI table dump needs root.
//! Sections run in parallel on the blocking pool, each with its own deadline,
//! so one hung sysfs read only costs that section.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{debug, warn};

use crate::contracts::DetectedHardware;
use crate::error::AppError;
use crate::platform::common::{blocking_with_deadline, read_cpuid, resolve_hypervisor};
use crate::tasks::cancellation::CancellationToken;

use super::sysfs::SysRoot;
use super::{acpi_dump, devices, input, system};

const SECTION_TIMEOUT: Duration = Duration::from_secs(20);
const ACPI_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn scan(
    acpi_dir: &Path,
    cancel: &CancellationToken,
) -> Result<DetectedHardware, AppError> {
    cancel.check()?;
    scan_root(SysRoot::system(), acpi_dir.to_path_buf(), cancel).await
}

async fn scan_root(
    sys: SysRoot,
    acpi_dir: PathBuf,
    cancel: &CancellationToken,
) -> Result<DetectedHardware, AppError> {
    debug!(acpi_dir = %acpi_dir.display(), "Linux scan started");
    let cpuid = read_cpuid();
    let (s1, s2, s3, s4, s5, s6) = (
        sys.clone(),
        sys.clone(),
        sys.clone(),
        sys.clone(),
        sys.clone(),
        sys,
    );
    let cpuid_for_cpu = cpuid.clone();
    let (cpu, memory, platform, devices, input, acpi) = tokio::join!(
        blocking_with_deadline(
            move || system::cpu(&s1, cpuid_for_cpu.as_ref()),
            SECTION_TIMEOUT,
            cancel,
            "CPU scan"
        ),
        blocking_with_deadline(
            move || system::memory(&s2),
            SECTION_TIMEOUT,
            cancel,
            "Memory scan"
        ),
        blocking_with_deadline(
            move || system::platform(&s3),
            SECTION_TIMEOUT,
            cancel,
            "Firmware scan"
        ),
        blocking_with_deadline(
            move || devices::collect(&s4),
            SECTION_TIMEOUT,
            cancel,
            "Device scan"
        ),
        blocking_with_deadline(
            move || input::collect(&s5),
            SECTION_TIMEOUT,
            cancel,
            "Input scan"
        ),
        {
            let dir = acpi_dir.clone();
            blocking_with_deadline(
                move || acpi_dump::dump(&s6, &dir),
                ACPI_TIMEOUT,
                cancel,
                "ACPI table dump",
            )
        },
    );
    cancel.check()?;

    let mut warnings = Vec::new();
    let cpu = section(cpu, &mut warnings);
    let memory = section(memory, &mut warnings);
    let platform = section(platform, &mut warnings);
    let devices = section(devices, &mut warnings);
    let input = section(input, &mut warnings);
    let acpi = section(acpi, &mut warnings);

    let mut hw = DetectedHardware {
        host_os: "linux".into(),
        ..Default::default()
    };
    let mut hypervisor_flag = false;
    if let Some((cpu, flag)) = cpu {
        hw.cpu = cpu;
        hypervisor_flag = flag;
    }
    if let Some(memory) = memory {
        hw.memory = memory;
    }
    let mut hypervisor_type = None;
    if let Some(p) = platform {
        hw.motherboard = p.motherboard;
        hw.chassis = p.chassis;
        hw.firmware = p.firmware;
        hypervisor_type = p.hypervisor_type;
    }
    if let Some(d) = devices {
        hw.gpus = d.gpus;
        hw.audio = d.audio;
        hw.network = d.network;
        hw.storage = d.storage;
        hw.usb_controllers = d.usb_controllers;
        if let Some((vendor, device)) = d.lpc {
            hw.motherboard.lpc_vendor_id = Some(vendor);
            hw.motherboard.lpc_device_id = Some(device);
        }
        warnings.extend(d.warnings);
    }
    if let Some(input) = input {
        hw.input = input;
    }
    if let Some(acpi) = acpi {
        if !acpi.written.is_empty() {
            hw.acpi_tables_dir = Some(acpi_dir.to_string_lossy().into_owned());
        }
        warnings.extend(acpi.warnings);
    }
    hw.hypervisor = resolve_hypervisor(
        cpuid.as_ref(),
        hw.motherboard.system_manufacturer.as_deref(),
        hw.motherboard.system_product.as_deref(),
    )
    .or_else(|| {
        hypervisor_type.map(|t| {
            if t.eq_ignore_ascii_case("xen") {
                "Xen".to_string()
            } else {
                t
            }
        })
    })
    .or_else(|| (hypervisor_flag && cpuid.is_none()).then(|| "Unknown hypervisor".to_string()));
    if hw.cpu.cores == 0 {
        warnings.push("The CPU core count could not be read".into());
    }
    hw.warnings = warnings;
    Ok(hw)
}

fn section<T>(result: Result<T, AppError>, warnings: &mut Vec<String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            warn!(error = %e, "scan section failed");
            warnings.push(e.message);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sysfs::fixture::Tree;
    use super::*;

    #[tokio::test]
    async fn scans_a_fixture_tree() {
        let t = Tree::new("scan");
        t.file("/proc/cpuinfo", "processor : 0\nvendor_id : GenuineIntel\nmodel name : Test CPU\nphysical id : 0\ncore id : 0\n")
            .file("/proc/meminfo", "MemTotal: 8388608 kB\n")
            .file("/sys/class/dmi/id/sys_vendor", "QEMU")
            .file("/sys/class/dmi/id/product_name", "Standard PC (Q35 + ICH9, 2009)")
            .file("/sys/class/dmi/id/chassis_type", "1")
            .dir("/sys/firmware/efi")
            .file("/sys/firmware/acpi/tables/DSDT", "too short");
        let lpc = t.pci("pci0000:00", "0000:00:1f.0", "8086", "2918", "060100");
        let _ = lpc;
        let acpi_dir = t.root.join("acpi-out");
        let hw = scan_root(SysRoot::new(&t.root), acpi_dir, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(hw.host_os, "linux");
        assert_eq!(hw.cpu.threads, 1);
        assert_eq!(hw.cpu.cores, 1);
        assert_eq!(hw.memory.total_mb, 8192);
        assert_eq!(hw.firmware.uefi, Some(true));
        assert_eq!(hw.motherboard.lpc_device_id.as_deref(), Some("2918"));
        assert!(hw.hypervisor.is_some());
        assert!(hw.acpi_tables_dir.is_none());
        assert!(hw.warnings.iter().any(|w| w.contains("DSDT")));
    }

    #[tokio::test]
    async fn cancelled_scan_returns_error() {
        let t = Tree::new("scan-cancel");
        let token = CancellationToken::new();
        token.cancel();
        let err = scan_root(SysRoot::new(&t.root), t.root.join("out"), &token)
            .await
            .unwrap_err();
        assert_eq!(err.code, "TASK_CANCELLED");
    }
}
