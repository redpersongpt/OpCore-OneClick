//! SMBIOS model, Secure Boot model and board-id handling.

use crate::domain::model::BuildPlan;
use crate::error::AppError;

use super::{DisplayPlan, PlanContext};

/// Choose `plan.smbios` (model, reason, alternatives, secure_boot_model,
/// board_id_skip). Honour `options.smbios_override` (warn when it does not
/// support the target). The chosen model must support the target per
/// `smbios_db` whenever a reasonable one exists; otherwise use the closest
/// model plus the board-id skip booter patches (and RestrictEvents
/// `revpatch=sbvmm` for updates) and explain it in a note.
pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) -> Result<(), AppError> {
    let _ = (ctx.target, display, &plan.smbios);
    todo!("smbios::apply")
}
