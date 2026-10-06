//! User-facing notes and post-install steps for the plan.

use crate::domain::model::BuildPlan;

use super::{DisplayPlan, PlanContext};

pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    let _ = (ctx.target, display, &plan.notes);
    todo!("notes::apply")
}
