//! ACPI table dump: DSDT and the other single-instance tables through
//! `GetSystemFirmwareTable`, every SSDT from `HKLM\HARDWARE\ACPI` (the
//! firmware table API only returns the first table of each signature).

use std::path::Path;

use windows_sys::Win32::System::Registry::REG_BINARY;
use windows_sys::Win32::System::SystemInformation::{
    EnumSystemFirmwareTables, GetSystemFirmwareTable, ACPI,
};

use crate::platform::common::AcpiDumpWriter;

use super::raw::{acpi_table_id, ssdt_registry_key, table_signatures};
use super::registry::RegKey;

const MAX_TABLE_LEN: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct AcpiDumpResult {
    pub written: Vec<String>,
    pub warnings: Vec<String>,
}

fn firmware_table(signature: &[u8; 4]) -> Option<Vec<u8>> {
    let id = acpi_table_id(signature);
    // SAFETY: a null buffer with size 0 asks for the table size.
    let size = unsafe { GetSystemFirmwareTable(ACPI, id, std::ptr::null_mut(), 0) };
    if size == 0 || size > MAX_TABLE_LEN {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` is valid for `size` bytes.
    let written = unsafe { GetSystemFirmwareTable(ACPI, id, buf.as_mut_ptr(), size) };
    if written == 0 || written > size {
        return None;
    }
    buf.truncate(written as usize);
    Some(buf)
}

fn firmware_signatures() -> Vec<[u8; 4]> {
    // SAFETY: a null buffer with size 0 asks for the list size.
    let size = unsafe { EnumSystemFirmwareTables(ACPI, std::ptr::null_mut(), 0) };
    if size == 0 || size > 64 * 1024 {
        return Vec::new();
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` is valid for `size` bytes.
    let written = unsafe { EnumSystemFirmwareTables(ACPI, buf.as_mut_ptr(), size) };
    buf.truncate((written as usize).min(buf.len()));
    table_signatures(&buf)
}

/// `HKLM\HARDWARE\ACPI\<key>\<OEM id>\<table id>\<revision>`: the table is
/// the first binary value of the innermost key.
fn registry_table(key: &str) -> Option<Vec<u8>> {
    let mut current = RegKey::local_machine(&format!(r"HARDWARE\ACPI\{key}"))?;
    for _ in 0..8 {
        let Some(first) = current.subkey_names().into_iter().next() else {
            break;
        };
        current = current.subkey(&first)?;
    }
    let (name, _) = current
        .value_names()
        .into_iter()
        .find(|(_, kind)| *kind == REG_BINARY)?;
    current.value(&name).map(|(_, data)| data)
}

pub fn dump(dir: &Path) -> AcpiDumpResult {
    let mut result = AcpiDumpResult::default();
    let mut writer = match AcpiDumpWriter::create(dir) {
        Ok(writer) => writer,
        Err(e) => {
            result.warnings.push(e.message);
            return result;
        }
    };
    let mut add = |bytes: Vec<u8>, what: &str, result: &mut AcpiDumpResult| {
        if let Err(e) = writer.add(&bytes) {
            result.warnings.push(format!("{what}: {}", e.message));
        }
    };

    match firmware_table(b"DSDT").or_else(|| registry_table("DSDT")) {
        Some(dsdt) => add(dsdt, "DSDT", &mut result),
        None => result
            .warnings
            .push("The firmware DSDT could not be read".into()),
    }
    let mut ssdts = 0;
    for key in (0..29).filter_map(ssdt_registry_key) {
        if let Some(table) = registry_table(&key) {
            add(table, &key, &mut result);
            ssdts += 1;
        }
    }
    if ssdts == 0 {
        if let Some(table) = firmware_table(b"SSDT") {
            add(table, "SSDT", &mut result);
        }
    }
    for signature in firmware_signatures() {
        if &signature == b"DSDT" || &signature == b"SSDT" {
            continue;
        }
        if let Some(table) = firmware_table(&signature) {
            add(table, &String::from_utf8_lossy(&signature), &mut result);
        }
    }
    result.written = writer.written().to_vec();
    if result.written.is_empty() {
        result
            .warnings
            .push("No ACPI tables could be read from the firmware".into());
    }
    result
}
