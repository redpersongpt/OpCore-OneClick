//! The planner turns a hardware profile + build options into a complete,
//! declarative `BuildPlan` following the Dortania OpenCore Install Guide (and
//! its laptop / AMD / HEDT variants), updated for OpenCore 1.0.8 and macOS 26.
//! Pure function: no I/O except reading ACPI tables referenced by the profile
//! (through `domain::acpi`).

use crate::error::AppError;

use super::model::{BuildOptions, BuildPlan, HardwareProfile};

/// Produce the full plan. Returns an error only when the target cannot work
/// at all (e.g. Apple silicon, no display path, CPU unsupported); soft
/// problems become `PlanNote`s.
pub fn plan(profile: &HardwareProfile, options: &BuildOptions) -> Result<BuildPlan, AppError> {
    todo!("plan {} {:?}", profile.cpu.name, options.target)
}
