//! Copy the firmware ACPI tables from `/sys/firmware/acpi/tables` (static
//! tables plus `dynamic/` ones loaded at runtime). The files are readable by
//! root only.

use std::io::ErrorKind;
use std::path::Path;

use crate::platform::common::AcpiDumpWriter;

use super::sysfs::SysRoot;

const TABLES: &str = "/sys/firmware/acpi/tables";

#[derive(Debug, Clone, Default)]
pub struct AcpiDumpResult {
    pub written: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn dump(sys: &SysRoot, dir: &Path) -> AcpiDumpResult {
    let mut result = AcpiDumpResult::default();
    let mut writer = match AcpiDumpWriter::create(dir) {
        Ok(w) => w,
        Err(e) => {
            result.warnings.push(e.message);
            return result;
        }
    };
    let names = sys.list(TABLES);
    if names.is_empty() {
        result
            .warnings
            .push("No ACPI tables found in /sys/firmware/acpi/tables".into());
        return result;
    }
    // DSDT first, then SSDTs in numeric order, then everything else.
    let rank = |n: &str| match n {
        "DSDT" => 0,
        n if n.starts_with("SSDT") => 1,
        _ => 2,
    };
    let mut ordered: Vec<String> = names
        .iter()
        .filter(|n| is_table_name(n))
        .map(|n| format!("{TABLES}/{n}"))
        .collect();
    ordered.sort_by_key(|p| rank(p.rsplit('/').next().unwrap_or_default()));
    ordered.extend(
        sys.list(&format!("{TABLES}/dynamic"))
            .into_iter()
            .filter(|n| is_table_name(n))
            .map(|n| format!("{TABLES}/dynamic/{n}")),
    );

    let mut denied = false;
    for path in ordered {
        match sys.read_bytes(&path) {
            Ok(bytes) => {
                if let Err(e) = writer.add(&bytes) {
                    result.warnings.push(format!("{path}: {}", e.message));
                }
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => denied = true,
            Err(e) => result.warnings.push(format!("Cannot read {path}: {e}")),
        }
    }
    if denied {
        result.warnings.push(
            "ACPI tables are readable by root only: run the scan with administrator rights (sudo / pkexec) \
             to include DSDT/SSDT-based fixes"
                .into(),
        );
    } else if !writer.has_dsdt() {
        result
            .warnings
            .push("The firmware DSDT could not be dumped".into());
    }
    result.written = writer.written().to_vec();
    result
}

/// Table files are 4-character signatures with an optional instance number
/// ("DSDT", "SSDT12", "FACP"); skips `data/`, `dynamic/` and other entries.
fn is_table_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 4
        && bytes[..4]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
        && bytes[4..].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::super::sysfs::fixture::Tree;
    use super::*;

    fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 36];
        bytes[..4].copy_from_slice(signature);
        bytes.extend_from_slice(body);
        let len = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&len.to_le_bytes());
        bytes
    }

    #[test]
    fn dumps_static_and_dynamic_tables() {
        let t = Tree::new("acpi");
        t.bytes("/sys/firmware/acpi/tables/DSDT", &table(b"DSDT", b"main"))
            .bytes("/sys/firmware/acpi/tables/SSDT10", &table(b"SSDT", b"ten"))
            .bytes("/sys/firmware/acpi/tables/SSDT2", &table(b"SSDT", b"two"))
            .bytes("/sys/firmware/acpi/tables/FACP", &table(b"FACP", b"fadt"))
            .bytes(
                "/sys/firmware/acpi/tables/dynamic/SSDT20",
                &table(b"SSDT", b"cpu pm"),
            )
            .dir("/sys/firmware/acpi/tables/data");
        let out = t.root.join("out");
        let result = dump(&SysRoot::new(&t.root), &out);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(
            result.written,
            [
                "DSDT.aml",
                "SSDT-1.aml",
                "SSDT-2.aml",
                "FACP.aml",
                "SSDT-3.aml"
            ]
        );
        assert_eq!(
            std::fs::read(out.join("SSDT-1.aml")).unwrap(),
            table(b"SSDT", b"two")
        );
    }

    #[test]
    fn missing_tables_warn() {
        let t = Tree::new("acpi-none");
        let result = dump(&SysRoot::new(&t.root), &t.root.join("out"));
        assert!(result.written.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(
            is_table_name("SSDT12")
                && is_table_name("FACP")
                && !is_table_name("data")
                && !is_table_name("dynamic")
        );
        assert!(!is_table_name("DSDÜ") && !is_table_name("SSD"));
    }
}
