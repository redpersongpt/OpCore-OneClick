//! Booter / Kernel / UEFI quirks, Kernel->Emulate and AMD kernel patches.

use crate::domain::model::BuildPlan;

use super::PlanContext;

pub fn apply(ctx: &PlanContext, plan: &mut BuildPlan) {
    let _ = (ctx.target, &plan.booter_quirks);
    todo!("quirks::apply")
}
