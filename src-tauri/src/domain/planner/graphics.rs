//! Display path selection, iGPU framebuffer properties, dGPU handling and GPU
//! kexts (WhateverGreen / NootRX / NootedRed).

use crate::domain::model::BuildPlan;
use crate::error::AppError;

use super::{DisplayPlan, PlanContext};

/// Decide which GPU drives the displays on the target and which GPUs are
/// disabled. Error when no GPU can show a picture on the target (except VMs).
pub fn choose_display(ctx: &PlanContext) -> Result<DisplayPlan, AppError> {
    let _ = ctx.target;
    todo!("graphics::choose_display")
}

/// Add GPU device properties (AAPL,ig-platform-id / AAPL,snb-platform-id,
/// device-id spoofs, framebuffer patches, headless ids, disable-gpu), GPU
/// boot-args (-wegnoegpu, agdpmod=..., -igfxblr, ...), and the GPU kext
/// (exactly one of WhateverGreen / NootRX / NootedRed, plus SMCRadeonSensors
/// where useful).
pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    let _ = (ctx.target, display, &plan.kexts);
    todo!("graphics::apply")
}
