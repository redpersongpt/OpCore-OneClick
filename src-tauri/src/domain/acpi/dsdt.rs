//! DSDT/SSDT analysis without iasl: walk AML scopes (Scope/Device/Processor/
//! Method/ThermalZone/PowerResource with PkgLength) to recover the namespace,
//! then collect the facts SSDT generation needs.

use std::path::Path;

use crate::domain::model::AcpiFacts;
use crate::error::AppError;

/// Parse `DSDT.aml` (+ `SSDT*.aml` for processor objects defined there) in
/// `dir` and return the extracted facts.
pub fn parse_tables(dir: &Path) -> Result<AcpiFacts, AppError> {
    todo!("parse_tables {}", dir.display())
}

/// Parse a single DSDT image (header + AML).
pub fn parse_dsdt(bytes: &[u8]) -> Result<AcpiFacts, AppError> {
    todo!("parse_dsdt {}", bytes.len())
}
