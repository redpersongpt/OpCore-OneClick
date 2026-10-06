//! Misc (boot picker, debug, security), NVRAM, PlatformInfo, UEFI drivers,
//! APFS/Output/Input settings, tools and final boot-args / csr-active-config.

use crate::domain::model::BuildPlan;

use super::PlanContext;

pub fn apply(ctx: &PlanContext, plan: &mut BuildPlan) {
    let _ = (ctx.target, &plan.misc_boot);
    todo!("settings::apply")
}
