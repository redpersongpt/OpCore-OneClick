//! PlatformInfo identity generation: serial + MLB valid for the chosen model
//! (macserial format and checksum rules), SystemUUID, ROM.

use std::path::Path;

use crate::domain::model::PlatformIdentity;
use crate::error::AppError;

/// Generate a fresh identity for `model`. Uses the `macserial` binary from the
/// OpenCore package when it can run on this host, otherwise the native
/// generator. ROM = primary NIC MAC when given, else random bytes with an
/// Apple OUI.
pub fn generate_identity(model: &str, macserial: Option<&Path>, mac_address: Option<&str>) -> Result<PlatformIdentity, AppError> {
    todo!("generate_identity {model} {macserial:?} {mac_address:?}")
}

/// Native serial/MLB generator (port of macserial's rules for the models in
/// `smbios_db`). Returns (serial, mlb).
pub fn native_serial_and_mlb(model: &str) -> Option<(String, String)> {
    todo!("native_serial_and_mlb {model}")
}

/// 17-character MLB base-34 checksum validation.
pub fn mlb_checksum_valid(mlb: &str) -> bool {
    todo!("mlb_checksum_valid {mlb}")
}
