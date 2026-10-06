//! Hardware scanner. See `platform::scan` for the contract.
//!
//! One PowerShell/CIM run collects the device inventory (`inventory.rs`);
//! CPUID, processor topology, memory, firmware mode and Secure Boot are read
//! in-process (`native.rs`), USB root hub ports through the hub IOCTLs and
//! the ACPI tables through the firmware table API and the registry. The
//! parts run in parallel, each with its own deadline; any of them may fail
//! and only costs its own section (reported in `warnings`).

use std::path::Path;
use std::time::Duration;

use tracing::{debug, warn};

use crate::contracts::DetectedHardware;
use crate::error::AppError;
use crate::platform::common::blocking_with_deadline;
use crate::tasks::cancellation::CancellationToken;

use super::inventory::{self, Inventory, NativeFacts};
use super::{acpi_dump, native, powershell, usb_ports};

const INVENTORY_TIMEOUT: Duration = Duration::from_secs(60);
const NATIVE_TIMEOUT: Duration = Duration::from_secs(15);
const USB_TIMEOUT: Duration = Duration::from_secs(20);
const ACPI_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn scan(
    acpi_dir: &Path,
    cancel: &CancellationToken,
) -> Result<DetectedHardware, AppError> {
    cancel.check()?;
    debug!(acpi_dir = %acpi_dir.display(), "Windows scan started");
    let dir = acpi_dir.to_path_buf();
    let (inventory, native, ports, acpi) = tokio::join!(
        powershell::run_inventory(INVENTORY_TIMEOUT, cancel),
        blocking_with_deadline(
            native::collect,
            NATIVE_TIMEOUT,
            cancel,
            "System information"
        ),
        blocking_with_deadline(
            usb_ports::root_hub_ports,
            USB_TIMEOUT,
            cancel,
            "USB port scan"
        ),
        blocking_with_deadline(
            move || acpi_dump::dump(&dir),
            ACPI_TIMEOUT,
            cancel,
            "ACPI table dump"
        ),
    );
    cancel.check()?;

    let mut warnings = Vec::new();
    let inventory = inventory.unwrap_or_else(|e| {
        warn!(error = %e, "device inventory failed");
        warnings.push(format!(
            "The device inventory failed ({}); only CPU, memory and firmware facts were collected",
            e.message
        ));
        Inventory::default()
    });
    let mut native = native.unwrap_or_else(|e| {
        warnings.push(e.message);
        NativeFacts::default()
    });
    match ports {
        Ok(Ok(ports)) => native.usb_ports = ports,
        Ok(Err(e)) | Err(e) => warnings.push(format!("USB ports could not be read: {}", e.message)),
    }

    let driver_keys = inventory::display_driver_keys(&inventory);
    if !driver_keys.is_empty() {
        let sizes = blocking_with_deadline(
            move || native::vram_sizes(&driver_keys),
            NATIVE_TIMEOUT,
            cancel,
            "Video memory",
        )
        .await;
        cancel.check()?;
        native.vram_by_driver = sizes.unwrap_or_default();
    }

    let mut hw = inventory::assemble(&inventory, &native);
    match acpi {
        Ok(result) => {
            if !result.written.is_empty() {
                hw.acpi_tables_dir = Some(acpi_dir.to_string_lossy().into_owned());
            }
            warnings.extend(result.warnings);
        }
        Err(e) => warnings.push(e.message),
    }
    if hw.cpu.cores == 0 {
        warnings.push("The CPU core count could not be read".into());
    }
    hw.warnings.splice(0..0, warnings);
    Ok(hw)
}
