//! Hardware scanner. See `platform::scan` for the contract.

use std::path::Path;

use crate::contracts::DetectedHardware;
use crate::error::AppError;
use crate::tasks::cancellation::CancellationToken;

pub async fn scan(acpi_dir: &Path, cancel: &CancellationToken) -> Result<DetectedHardware, AppError> {
    todo!("scan {} {}", acpi_dir.display(), cancel.is_cancelled())
}
