//! ACPI support: parse the machine's DSDT/SSDTs (`dsdt`), emit AML bytecode
//! (`aml`) and generate path-correct SSDTs (`ssdt`) like SSDTTime does, with
//! no external iasl dependency.

pub mod aml;
pub mod dsdt;
mod namespace;
mod parse;
pub mod ssdt;

#[cfg(test)]
mod tests;

pub use dsdt::{load_tables, parse_dsdt, parse_tables, AcpiTable, AcpiTables};
pub use ssdt::{
    fallback_patches, fallback_prebuilt, fallback_prebuilt_acpi0007, generate,
    generate_with_tables, is_not_needed, GeneratedSsdt, SsdtKind, ERR_INSUFFICIENT, ERR_NOT_NEEDED,
};
