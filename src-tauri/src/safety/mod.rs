//! Everything that stands between the user's data and a disk erase: device
//! path validation, system-disk detection, disk identity, confirmation
//! tokens, flash scripts and payload verification.

pub mod chunklist;
pub mod device_path;
pub mod disk_identity;
pub mod disks;
pub mod flash_auth;
pub mod flash_plan;
pub mod payload;
