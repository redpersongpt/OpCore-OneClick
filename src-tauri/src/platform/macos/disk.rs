//! Removable disk enumeration and flashing. See `platform::flash`.

use crate::contracts::DiskInfo;
use crate::error::AppError;
use crate::platform::{FlashJob, FlashProgressFn};
use crate::tasks::cancellation::CancellationToken;

pub async fn list_disks() -> Result<Vec<DiskInfo>, AppError> {
    todo!()
}

pub async fn flash(job: &FlashJob, progress: FlashProgressFn<'_>, cancel: &CancellationToken) -> Result<(), AppError> {
    let _ = progress;
    todo!("flash {} {}", job.device, cancel.is_cancelled())
}
