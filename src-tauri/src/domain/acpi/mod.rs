//! ACPI support: parse the machine's DSDT/SSDTs (`dsdt`), emit AML bytecode
//! (`aml`) and generate path-correct SSDTs (`ssdt`) like SSDTTime does, with
//! no external iasl dependency.

pub mod aml;
pub mod dsdt;
pub mod ssdt;

pub use dsdt::parse_tables;
pub use ssdt::{fallback_prebuilt, generate, GeneratedSsdt, SsdtKind};
