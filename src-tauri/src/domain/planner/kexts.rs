//! Every non-GPU kext: Lilu/VirtualSMC + sensors, audio (AppleALC + layout-id),
//! Ethernet, Wi-Fi, Bluetooth, input, USB, storage, CPU helpers, OTA helpers.

use crate::domain::model::BuildPlan;

use super::{DisplayPlan, PlanContext};

pub fn apply(ctx: &PlanContext, display: &DisplayPlan, plan: &mut BuildPlan) {
    let _ = (ctx.target, display, &plan.kexts);
    todo!("kexts::apply")
}
