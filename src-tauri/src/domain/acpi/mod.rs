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

/// ACPI table files (`*.aml`) an ACPI patch comment names: the tables the
/// patch only makes sense with ("EC0 _STA to XSTA rename (SSDT-EC.aml)",
/// "(SSDT-A.aml, SSDT-B.aml)" when several need it).
pub fn tables_named_in(comment: &str) -> Vec<&str> {
    comment
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ','))
        .filter(|w| w.len() > 4 && w.to_ascii_lowercase().ends_with(".aml"))
        .collect()
}

/// Name `file` in an ACPI patch comment as a table the patch needs: a
/// trailing " (SSDT-X.aml)", or added to that list when one is there.
pub fn name_required_table(comment: &mut String, file: &str) {
    if tables_named_in(comment).iter().any(|t| t.eq_ignore_ascii_case(file)) {
        return;
    }
    if let Some(open) = comment.ends_with(".aml)").then(|| comment.rfind('(')).flatten() {
        let listed = &comment[open + 1..comment.len() - 1];
        if listed.split(", ").all(|t| !t.contains(' ') && t.to_ascii_lowercase().ends_with(".aml")) {
            comment.truncate(comment.len() - 1);
            comment.push_str(", ");
            comment.push_str(file);
            comment.push(')');
            return;
        }
    }
    if !comment.is_empty() {
        comment.push(' ');
    }
    comment.push('(');
    comment.push_str(file);
    comment.push(')');
}
