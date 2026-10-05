//! Pure domain logic: hardware knowledge, interpretation, planning and
//! config generation. Nothing in here performs network or disk-write I/O
//! except `config_writer`/`kernel_add`, which only read local files.

pub mod model;

// Knowledge base (static data)
pub mod chipset_db;
pub mod codec_db;
pub mod cpu_db;
pub mod device_db;
pub mod gpu_db;
pub mod kext_catalog;
pub mod macos_db;
pub mod smbios_db;

// Interpretation and planning
pub mod bios;
pub mod compatibility;
pub mod planner;
pub mod profile;

// Generation
pub mod acpi;
pub mod amd_patches;
pub mod config_writer;
pub mod kernel_add;
pub mod smbios_gen;
