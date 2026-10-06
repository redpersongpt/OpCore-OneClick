//! SSDT materialisation helpers: generated AML from the plan, OpenCore's
//! sample binaries from the package, Dortania prebuilts (fetched by the
//! pipeline), and the clean-up when an optional table cannot be provided.

use std::path::Path;

use crate::domain::acpi;
use crate::domain::model::{AcpiPatch, SsdtSource};
use crate::error::AppError;
use crate::services::ocvalidate::check_aml;

use super::is_plain_file_name;
use super::staging::find_ci;

/// `SsdtResult.source` value.
pub fn source_id(source: &SsdtSource) -> &'static str {
    match source {
        SsdtSource::Generated { .. } => "generated",
        SsdtSource::OcSample { .. } => "oc_sample",
        SsdtSource::Dortania { .. } => "dortania",
    }
}

/// Bytes of a generated table (hex in the plan), checked to be an ACPI table.
pub fn generated_bytes(aml_hex: &str) -> Result<Vec<u8>, AppError> {
    let bytes =
        decode_hex(aml_hex).ok_or_else(|| AppError::new("SSDT_INVALID", "the generated table is not valid hex"))?;
    check_table(&bytes)?;
    Ok(bytes)
}

/// A prebuilt from OpenCore's `Docs/AcpiSamples/Binaries`.
pub fn oc_sample_bytes(samples_dir: &Path, file: &str) -> Result<Vec<u8>, AppError> {
    if !is_plain_file_name(file, ".aml") {
        return Err(AppError::new("SSDT_INVALID", format!("'{file}' is not an ACPI table file name")));
    }
    let path = find_ci(samples_dir, file).filter(|p| p.is_file()).ok_or_else(|| {
        AppError::new("SSDT_NOT_AVAILABLE", format!("OpenCore does not ship {file} in Docs/AcpiSamples/Binaries"))
    })?;
    let bytes = std::fs::read(&path)?;
    check_table(&bytes)?;
    Ok(bytes)
}

pub fn check_table(bytes: &[u8]) -> Result<(), AppError> {
    check_aml(bytes).map_err(|why| AppError::new("SSDT_INVALID", why))
}

/// Disable the ACPI patches that only make sense with `file_name` loaded:
/// their comment names it ("EC0 _STA to XSTA rename (SSDT-EC.aml)", "_OSI to
/// XOSI rename - requires SSDT-XOSI.aml") and none of the `present` tables
/// that may need the same rename. Returns how many were disabled.
pub fn disable_dependent_patches(patches: &mut [AcpiPatch], file_name: &str, present: &[String]) -> usize {
    let needle = file_name.to_ascii_lowercase();
    let stem = needle.trim_end_matches(".aml");
    let mut count = 0;
    for patch in patches.iter_mut().filter(|p| p.enabled) {
        let comment = patch.comment.to_ascii_lowercase();
        let names_it = acpi::tables_named_in(&comment).contains(&needle.as_str())
            || comment.split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')')).any(|w| w == stem);
        let still_needed = acpi::tables_named_in(&patch.comment)
            .iter()
            .any(|t| present.iter().any(|p| p.eq_ignore_ascii_case(t)));
        if names_it && !still_needed {
            patch.enabled = false;
            count += 1;
        }
    }
    count
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(2) {
        return None;
    }
    clean
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn test_table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut t = Vec::with_capacity(36 + body.len());
    t.extend_from_slice(signature);
    t.extend_from_slice(&((36 + body.len()) as u32).to_le_bytes());
    t.push(2);
    t.push(0);
    t.extend_from_slice(b"OCLICK");
    t.extend_from_slice(b"TESTTABL");
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(b"INTL");
    t.extend_from_slice(&0x2023_0628u32.to_le_bytes());
    t.extend_from_slice(body);
    let sum = t.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    t[9] = 0u8.wrapping_sub(sum);
    t
}

#[cfg(test)]
mod tests {
    use super::super::staging::test_dir::TempDir;
    use super::*;
    use crate::domain::model::hex_upper;

    fn patch(comment: &str) -> AcpiPatch {
        AcpiPatch {
            comment: comment.into(),
            find: "5F4F5349".into(),
            replace: "584F5349".into(),
            table_signature: None,
            oem_table_id: None,
            count: 0,
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn generated_tables_are_decoded_and_checked() {
        let table = test_table(b"SSDT", &[0x10, 0x05]);
        assert_eq!(generated_bytes(&hex_upper(&table)).unwrap(), table);
        assert_eq!(generated_bytes(&hex_upper(&table).to_lowercase()).unwrap(), table);
        assert_eq!(generated_bytes("ABC").unwrap_err().code, "SSDT_INVALID");
        assert_eq!(generated_bytes("ZZ").unwrap_err().code, "SSDT_INVALID");
        assert_eq!(generated_bytes(&hex_upper(&table[..30])).unwrap_err().code, "SSDT_INVALID");
    }

    #[test]
    fn oc_samples_come_from_the_package() {
        let tmp = TempDir::new("ssdt");
        std::fs::write(tmp.path().join("SSDT-PLUG.aml"), test_table(b"SSDT", b"x")).unwrap();
        std::fs::write(tmp.path().join("SSDT-BAD.aml"), b"<html>not found</html>").unwrap();
        assert!(oc_sample_bytes(tmp.path(), "ssdt-plug.aml").is_ok());
        assert_eq!(oc_sample_bytes(tmp.path(), "SSDT-NONE.aml").unwrap_err().code, "SSDT_NOT_AVAILABLE");
        assert_eq!(oc_sample_bytes(tmp.path(), "SSDT-BAD.aml").unwrap_err().code, "SSDT_INVALID");
        assert_eq!(oc_sample_bytes(tmp.path(), "../SSDT-PLUG.aml").unwrap_err().code, "SSDT_INVALID");
    }

    #[test]
    fn patches_tied_to_a_missing_table_are_disabled() {
        let mut patches = vec![
            patch("_OSI to XOSI rename - requires SSDT-XOSI.aml"),
            patch("EC0 to EC rename"),
            patch("GPI0 _STA to XSTA rename (SSDT-GPI0.aml)"),
            patch("RTC _STA to XSTA rename (SSDT-AWAC.aml, SSDT-RTC0.aml)"),
        ];
        assert_eq!(disable_dependent_patches(&mut patches, "SSDT-XOSI.aml", &[]), 1);
        assert!(!patches[0].enabled);
        assert!(patches[1].enabled && patches[2].enabled);
        assert_eq!(disable_dependent_patches(&mut patches, "SSDT-EC.aml", &[]), 0);
        // A rename another loaded table also needs stays on.
        let present = ["SSDT-RTC0.aml".to_string()];
        assert_eq!(disable_dependent_patches(&mut patches, "SSDT-AWAC.aml", &present), 0);
        assert!(patches[3].enabled);
        assert_eq!(disable_dependent_patches(&mut patches, "ssdt-awac.aml", &[]), 1);
        assert_eq!(disable_dependent_patches(&mut patches, "SSDT-GPI0.aml", &[]), 1);
        assert!(patches[1].enabled);
        assert_eq!(source_id(&SsdtSource::Dortania { file: "x".into() }), "dortania");
    }
}
