//! SSDT selection per platform (Dortania ssdt-platform matrix), generated from
//! the machine's DSDT when available, prebuilt fallback otherwise; ACPI
//! renames and deletes.

use crate::domain::model::BuildPlan;

use super::PlanContext;

pub fn apply(ctx: &PlanContext, plan: &mut BuildPlan) {
    let _ = (ctx.target, &plan.ssdts);
    todo!("acpi::apply")
}
