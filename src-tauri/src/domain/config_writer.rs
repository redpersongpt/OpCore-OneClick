//! config.plist writer. Starts from the `Docs/Sample.plist` shipped in the
//! exact OpenCore release being installed (so the schema always matches), then
//! applies the build plan. Every override key must already exist in the
//! template; an unknown key is an error (catches schema drift).

use crate::domain::kernel_add::KernelAddEntry;
use crate::domain::model::{BuildPlan, PlatformIdentity};
use crate::error::AppError;

pub struct ConfigInputs<'a> {
    pub plan: &'a BuildPlan,
    pub kernel_add: &'a [KernelAddEntry],
    pub identity: &'a PlatformIdentity,
    /// SSDT file names actually present in EFI/OC/ACPI, in load order.
    pub ssdt_files: &'a [String],
    /// Driver file names actually present in EFI/OC/Drivers.
    pub driver_files: &'a [String],
    /// Tool file names actually present in EFI/OC/Tools.
    pub tool_files: &'a [String],
}

/// Produce the final config.plist (XML) from the Sample.plist bytes.
pub fn write_config(sample_plist: &[u8], inputs: &ConfigInputs) -> Result<Vec<u8>, AppError> {
    todo!("write_config {} {}", sample_plist.len(), inputs.plan.smbios.model)
}
