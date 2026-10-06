//! Validation of a generated EFI: OpenCore's own `ocvalidate` (schema and
//! semantic checks) plus layout checks (every referenced file exists, every
//! kext has its executable/Info.plist, required binaries present).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use once_cell::sync::Lazy;
use plist::{Dictionary, Value};
use regex::Regex;

use crate::contracts::{ValidationIssue, ValidationResult};
use crate::domain::model::NoteLevel;
use crate::error::AppError;

const OCVALIDATE_TIMEOUT: Duration = Duration::from_secs(30);
const LILU_ID: &str = "as.vit9696.Lilu";

/// Validate the EFI rooted at `efi_dir` (the directory that contains `OC/`
/// and `BOOT/`). `ocvalidate` is the host binary from the matching OpenCore
/// package, when available.
pub async fn validate_efi(efi_dir: &Path, ocvalidate: Option<&Path>) -> ValidationResult {
    let efi = efi_dir.to_path_buf();
    let mut issues = match tokio::task::spawn_blocking(move || layout_issues(&efi)).await {
        Ok(v) => v,
        Err(e) => vec![issue(NoteLevel::Blocking, "layout", format!("Layout check failed to run: {e}"), None)],
    };

    let config = resolve_ci(efi_dir, "OC/config.plist");
    let mut ocvalidate_ran = false;
    let mut ocvalidate_output = None;
    match (ocvalidate, &config) {
        (None, _) => issues.push(issue(
            NoteLevel::Info,
            "ocvalidate",
            "ocvalidate is not available for this system; OpenCore's schema check was skipped".into(),
            None,
        )),
        (Some(_), None) => {}
        (Some(bin), Some(config)) => match run_ocvalidate(bin, config).await {
            Ok(run) => {
                ocvalidate_ran = true;
                issues.extend(ocvalidate_issues(&run));
                ocvalidate_output = Some(run.output);
            }
            Err(e) => {
                tracing::warn!(error = %e, "ocvalidate could not run");
                issues.push(issue(
                    NoteLevel::Warning,
                    "ocvalidate",
                    format!("ocvalidate could not run: {}", e.message),
                    None,
                ));
            }
        },
    }

    let valid = !issues.iter().any(|i| i.level == NoteLevel::Blocking);
    ValidationResult { valid, ocvalidate_ran, ocvalidate_output, issues }
}

/// Raw result of one ocvalidate run.
#[derive(Debug, Clone)]
pub struct OcValidateRun {
    /// Exit code (None when killed by a signal).
    pub status: Option<i32>,
    /// stdout followed by stderr.
    pub output: String,
}

/// Run `ocvalidate <config>` with a 30 s timeout. No console window on Windows.
pub async fn run_ocvalidate(bin: &Path, config: &Path) -> Result<OcValidateRun, AppError> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg(config).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child =
        cmd.spawn().map_err(|e| AppError::new("OCVALIDATE_FAILED", format!("Cannot start {}: {e}", bin.display())))?;
    let out = tokio::time::timeout(OCVALIDATE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| AppError::new("OCVALIDATE_TIMEOUT", "ocvalidate did not finish within 30 seconds").recoverable())?
        .map_err(|e| AppError::new("OCVALIDATE_FAILED", format!("ocvalidate failed: {e}")))?;
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        if !output.ends_with('\n') && !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&stderr);
    }
    Ok(OcValidateRun { status: out.status.code(), output })
}

/// ocvalidate output split into findings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedOcValidate {
    /// One entry per reported problem, in output order.
    pub issues: Vec<String>,
    /// "Found N issues requiring attention." when printed.
    pub reported_count: Option<usize>,
    /// "No issues found." was printed.
    pub no_issues: bool,
}

static SUMMARY_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(Serialisation|Check[A-Za-z]*) returns \d+ errors?!?$").expect("static regex"));
static FOUND_COUNT: Lazy<Regex> = Lazy::new(|| Regex::new(r"Found (\d+) issues?").expect("static regex"));
static OCS_CONTEXT: Lazy<Regex> = Lazy::new(|| Regex::new(r"context <([^>]+)>").expect("static regex"));

/// Parse ocvalidate's text output. Problem lines look like
/// `OCS: Missing key X, context <Quirks>!` or `Kernel->Add[1] discovers ...!`;
/// per-section totals (`CheckKernel returns 7 errors!`), the version NOTE and
/// the final summary are bookkeeping, not findings.
pub fn parse_output(output: &str) -> ParsedOcValidate {
    let mut parsed = ParsedOcValidate::default();
    for raw in output.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("NOTE:") || SUMMARY_LINE.is_match(line) {
            continue;
        }
        if line.starts_with("Completed validating") {
            if line.contains("No issues found") {
                parsed.no_issues = true;
            } else if let Some(c) = FOUND_COUNT.captures(line) {
                parsed.reported_count = c.get(1).and_then(|m| m.as_str().parse().ok());
            }
            continue;
        }
        parsed.issues.push(line.to_string());
    }
    parsed
}

/// Config location an ocvalidate line refers to ("Kernel->Add[1]", "Quirks").
fn ocvalidate_path(line: &str) -> Option<String> {
    if line.starts_with("OCS:") {
        return OCS_CONTEXT.captures(line).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
    }
    let first = line.split_whitespace().next()?;
    first.contains("->").then(|| first.trim_end_matches([':', '!']).to_string())
}

fn ocvalidate_issues(run: &OcValidateRun) -> Vec<ValidationIssue> {
    let parsed = parse_output(&run.output);
    let mut out: Vec<ValidationIssue> =
        parsed.issues.iter().map(|l| issue(NoteLevel::Blocking, "ocvalidate", l.clone(), ocvalidate_path(l))).collect();
    if let Some(n) = parsed.reported_count {
        if n != out.len() {
            tracing::debug!(reported = n, parsed = out.len(), "ocvalidate issue count differs from parsed lines");
        }
    }
    match run.status {
        Some(0) => {}
        Some(code) if out.is_empty() => out.push(issue(
            NoteLevel::Blocking,
            "ocvalidate",
            format!("ocvalidate exited with status {code} without listing a problem"),
            None,
        )),
        Some(_) => {}
        None => out.push(issue(NoteLevel::Warning, "ocvalidate", "ocvalidate was terminated".into(), None)),
    }
    out
}

fn issue(level: NoteLevel, source: &str, message: String, path: Option<String>) -> ValidationIssue {
    ValidationIssue { level, source: source.to_string(), message, path }
}

/// Resolve `rel` ('/'-separated) under `base` the way FAT32 does: exact
/// match first, then a case-insensitive one. Rejects `..` and absolute paths.
fn resolve_ci(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.starts_with('/') || rel.starts_with('\\') || rel.contains(':') {
        return None;
    }
    let mut cur = base.to_path_buf();
    for part in rel.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            return None;
        }
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur)
            .ok()?
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        cur = found.path();
    }
    cur.exists().then_some(cur)
}

/// Parse "A.B.C" into OpenCore's `A*10000 + B*100 + C`; empty means unbounded.
fn darwin_version(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut it = s.split('.').map(|p| p.parse::<u32>().ok());
    let a = it.next().flatten()?;
    let b = it.next().flatten().unwrap_or(0);
    let c = it.next().flatten().unwrap_or(0);
    Some(a * 10_000 + b.min(99) * 100 + c.min(99))
}

fn get_str<'a>(d: &'a Dictionary, key: &str) -> &'a str {
    d.get(key).and_then(Value::as_string).unwrap_or_default()
}

fn get_bool(d: &Dictionary, key: &str) -> bool {
    d.get(key).and_then(Value::as_boolean).unwrap_or(false)
}

fn section<'a>(root: &'a Dictionary, path: &[&str]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut d = root;
    for p in parents {
        d = d.get(p)?.as_dictionary()?;
    }
    d.get(last)
}

fn array_dicts<'a>(root: &'a Dictionary, path: &[&str]) -> Vec<&'a Dictionary> {
    section(root, path)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_dictionary).collect())
        .unwrap_or_default()
}

/// One Kernel->Add entry as found on disk.
struct KextRow {
    index: usize,
    bundle_path: String,
    id: Option<String>,
    libraries: Vec<String>,
    min: u32,
    max: u32,
}

fn layout_issues(efi: &Path) -> Vec<ValidationIssue> {
    let mut out = Vec::new();
    let blocking = |out: &mut Vec<ValidationIssue>, msg: String, path: Option<String>| {
        out.push(issue(NoteLevel::Blocking, "layout", msg, path));
    };
    if !efi.is_dir() {
        blocking(&mut out, format!("EFI folder {} does not exist", efi.display()), None);
        return out;
    }
    for (rel, what) in [("BOOT/BOOTx64.efi", "boot loader"), ("OC/OpenCore.efi", "OpenCore")] {
        if resolve_ci(efi, rel).is_none_or_dir() {
            blocking(&mut out, format!("EFI/{rel} ({what}) is missing"), Some(format!("EFI/{rel}")));
        }
    }
    let Some(oc) = resolve_ci(efi, "OC").filter(|p| p.is_dir()) else {
        blocking(&mut out, "EFI/OC folder is missing".into(), Some("EFI/OC".into()));
        return out;
    };
    let Some(config_path) = resolve_ci(&oc, "config.plist") else {
        blocking(&mut out, "EFI/OC/config.plist is missing".into(), Some("EFI/OC/config.plist".into()));
        return out;
    };
    let root = match Value::from_file(&config_path) {
        Ok(Value::Dictionary(d)) => d,
        Ok(_) => {
            blocking(&mut out, "config.plist is not a dictionary".into(), Some("EFI/OC/config.plist".into()));
            return out;
        }
        Err(e) => {
            blocking(&mut out, format!("config.plist cannot be parsed: {e}"), Some("EFI/OC/config.plist".into()));
            return out;
        }
    };

    check_files(&mut out, &root, &oc, &["ACPI", "Add"], "ACPI", "ACPI->Add");
    check_acpi_tables(&mut out, &root, &oc);
    check_files(&mut out, &root, &oc, &["UEFI", "Drivers"], "Drivers", "UEFI->Drivers");
    check_files(&mut out, &root, &oc, &["Misc", "Tools"], "Tools", "Misc->Tools");
    check_kexts(&mut out, &root, &oc);
    check_resources(&mut out, &root, &oc);
    out
}

trait MissingFile {
    fn is_none_or_dir(&self) -> bool;
}

impl MissingFile for Option<PathBuf> {
    fn is_none_or_dir(&self) -> bool {
        !matches!(self, Some(p) if p.is_file())
    }
}

/// Every enabled `Path` of an ACPI/Drivers/Tools array must exist in its folder.
fn check_files(out: &mut Vec<ValidationIssue>, root: &Dictionary, oc: &Path, key: &[&str], folder: &str, label: &str) {
    for (i, d) in array_dicts(root, key).into_iter().enumerate() {
        let path = get_str(d, "Path");
        if !get_bool(d, "Enabled") || path.is_empty() || path.starts_with('#') {
            continue;
        }
        if resolve_ci(oc, &format!("{folder}/{path}")).is_none_or_dir() {
            out.push(issue(
                NoteLevel::Blocking,
                "layout",
                format!("{label}[{i}] references {folder}/{path}, which is not in EFI/OC/{folder}"),
                Some(format!("{label}[{i}]")),
            ));
        }
    }
}

/// Structural check of an ACPI table file: header present, sane signature,
/// and a length field equal to the file size (a download error page saved as
/// `.aml` fails here).
pub fn check_aml(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 36 {
        return Err(format!("{} bytes is too short for an ACPI table", bytes.len()));
    }
    if !bytes[..4].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_') {
        return Err("no ACPI table signature".into());
    }
    let declared = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if declared != bytes.len() {
        return Err(format!("table header says {declared} bytes but the file has {}", bytes.len()));
    }
    Ok(())
}

/// ACPI tables sum to zero over their whole length.
fn aml_checksum_ok(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)) == 0
}

/// Every enabled ACPI->Add file must be a well-formed table.
fn check_acpi_tables(out: &mut Vec<ValidationIssue>, root: &Dictionary, oc: &Path) {
    for (i, d) in array_dicts(root, &["ACPI", "Add"]).into_iter().enumerate() {
        let path = get_str(d, "Path");
        if !get_bool(d, "Enabled") || path.is_empty() || path.starts_with('#') {
            continue;
        }
        let Some(file) = resolve_ci(oc, &format!("ACPI/{path}")).filter(|p| p.is_file()) else { continue };
        let at = Some(format!("ACPI->Add[{i}]"));
        match std::fs::read(&file) {
            Ok(bytes) => match check_aml(&bytes) {
                Err(why) => out.push(issue(
                    NoteLevel::Blocking,
                    "acpi",
                    format!("ACPI/{path} is not a valid ACPI table: {why}"),
                    at,
                )),
                Ok(()) if !aml_checksum_ok(&bytes) => {
                    out.push(issue(NoteLevel::Warning, "acpi", format!("ACPI/{path} has a wrong table checksum"), at))
                }
                Ok(()) => {}
            },
            Err(e) => out.push(issue(NoteLevel::Blocking, "acpi", format!("ACPI/{path} cannot be read: {e}"), at)),
        }
    }
}

fn check_kexts(out: &mut Vec<ValidationIssue>, root: &Dictionary, oc: &Path) {
    let mut rows: Vec<KextRow> = Vec::new();
    let mut seen_paths: HashMap<String, usize> = HashMap::new();
    for (i, d) in array_dicts(root, &["Kernel", "Add"]).into_iter().enumerate() {
        if !get_bool(d, "Enabled") {
            continue;
        }
        let at = format!("Kernel->Add[{i}]");
        let bundle_path = get_str(d, "BundlePath").to_string();
        let plist_rel = get_str(d, "PlistPath");
        let exe_rel = get_str(d, "ExecutablePath");
        let mut fail = |msg: String| out.push(issue(NoteLevel::Blocking, "kext", msg, Some(at.clone())));

        if let Some(prev) = seen_paths.insert(bundle_path.to_ascii_lowercase(), i) {
            fail(format!("{bundle_path} is enabled twice (Kernel->Add[{prev}] and [{i}])"));
            continue;
        }
        let Some(bundle) = resolve_ci(&oc.join("Kexts"), &bundle_path).filter(|p| p.is_dir()) else {
            fail(format!("{bundle_path} is not in EFI/OC/Kexts"));
            continue;
        };
        let info = match resolve_ci(&bundle, plist_rel).filter(|p| p.is_file()) {
            Some(p) => Value::from_file(&p).ok().and_then(Value::into_dictionary),
            None => {
                fail(format!("{bundle_path}: PlistPath '{plist_rel}' does not exist"));
                continue;
            }
        };
        let Some(info) = info else {
            fail(format!("{bundle_path}: {plist_rel} cannot be parsed"));
            continue;
        };
        let declared_exe = get_str(&info, "CFBundleExecutable");
        if exe_rel.is_empty() {
            // An empty executable file counts as codeless, as in kernel_add.
            let has_code = !declared_exe.is_empty()
                && resolve_ci(&bundle, &format!("Contents/MacOS/{declared_exe}"))
                    .and_then(|p| std::fs::metadata(p).ok())
                    .is_some_and(|m| m.is_file() && m.len() > 0);
            if has_code {
                out.push(issue(
                    NoteLevel::Warning,
                    "kext",
                    format!("{bundle_path} has an executable (Contents/MacOS/{declared_exe}) but ExecutablePath is empty, so its code is not loaded"),
                    Some(at.clone()),
                ));
            }
        } else if resolve_ci(&bundle, exe_rel).is_none_or_dir() {
            fail(format!("{bundle_path}: ExecutablePath '{exe_rel}' does not exist"));
            continue;
        }
        let id = get_str(&info, "CFBundleIdentifier").trim().to_string();
        let libraries = info
            .get("OSBundleLibraries")
            .and_then(Value::as_dictionary)
            .map(|l| l.keys().map(|k| k.to_string()).collect())
            .unwrap_or_default();
        rows.push(KextRow {
            index: i,
            bundle_path,
            id: (!id.is_empty()).then_some(id),
            libraries,
            min: darwin_version(get_str(d, "MinKernel")).unwrap_or(0),
            max: darwin_version(get_str(d, "MaxKernel")).unwrap_or(u32::MAX),
        });
    }

    // The same bundle id must not load twice for any kernel version.
    for (a_pos, a) in rows.iter().enumerate() {
        for b in rows.iter().skip(a_pos + 1) {
            let same = matches!((&a.id, &b.id), (Some(x), Some(y)) if x.eq_ignore_ascii_case(y));
            if same && a.min <= b.max && b.min <= a.max {
                out.push(issue(
                    NoteLevel::Blocking,
                    "kext",
                    format!(
                        "{} and {} are both enabled with bundle id {} for overlapping kernel ranges",
                        a.bundle_path,
                        b.bundle_path,
                        a.id.as_deref().unwrap_or_default()
                    ),
                    Some(format!("Kernel->Add[{}]", b.index)),
                ));
            }
        }
    }

    // Dependencies must be present and load first.
    for (pos, row) in rows.iter().enumerate() {
        for lib in &row.libraries {
            let providers: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(p, r)| *p != pos && r.id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(lib)))
                .map(|(p, _)| p)
                .collect();
            let at = Some(format!("Kernel->Add[{}]", row.index));
            if providers.is_empty() {
                if !lib.to_ascii_lowercase().starts_with("com.apple.") {
                    out.push(issue(
                        NoteLevel::Blocking,
                        "kext",
                        format!("{} depends on {lib}, which is not an enabled Kernel->Add entry", row.bundle_path),
                        at,
                    ));
                }
            } else if providers.iter().all(|p| *p > pos) {
                let provider = &rows[providers[0]];
                out.push(issue(
                    NoteLevel::Blocking,
                    "kext",
                    format!("{} loads before its dependency {}", row.bundle_path, provider.bundle_path),
                    at,
                ));
            }
        }
    }

    if let Some(pos) = rows.iter().position(|r| r.id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(LILU_ID))) {
        if pos != 0 {
            out.push(issue(
                NoteLevel::Warning,
                "kext",
                format!("Lilu.kext should be the first Kernel->Add entry (it is entry {})", rows[pos].index),
                Some(format!("Kernel->Add[{}]", rows[pos].index)),
            ));
        }
    }
}

/// OpenCanopy (PickerMode External) needs the OcBinaryData resources.
fn check_resources(out: &mut Vec<ValidationIssue>, root: &Dictionary, oc: &Path) {
    let external = section(root, &["Misc", "Boot", "PickerMode"]).and_then(Value::as_string) == Some("External");
    if !external {
        return;
    }
    let canopy = array_dicts(root, &["UEFI", "Drivers"])
        .iter()
        .any(|d| get_bool(d, "Enabled") && get_str(d, "Path").eq_ignore_ascii_case("OpenCanopy.efi"));
    if !canopy {
        out.push(issue(
            NoteLevel::Warning,
            "layout",
            "PickerMode is External but OpenCanopy.efi is not an enabled driver; the text picker will be used".into(),
            Some("Misc->Boot->PickerMode".into()),
        ));
        return;
    }
    for folder in ["Font", "Image", "Label"] {
        let populated = resolve_ci(oc, &format!("Resources/{folder}"))
            .and_then(|p| std::fs::read_dir(p).ok())
            .is_some_and(|mut rd| rd.next().is_some());
        if !populated {
            out.push(issue(
                NoteLevel::Warning,
                "layout",
                format!("EFI/OC/Resources/{folder} is empty; OpenCanopy will fall back to the text picker"),
                Some(format!("EFI/OC/Resources/{folder}")),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("oneclick-ocv-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SAMPLE_ERRORS: &str = "
NOTE: This version of ocvalidate is only compatible with OpenCore version 1.0.8!

OCS: Missing key ClearTaskSwitchBit, context <Quirks>!
OCS: No schema for AllowNvramReset at 0 index, context <Security>!
Serialisation returns 2 errors!

Kernel->Add[1] discovers VirtualSMC.kext, but its Parent (Lilu.kext) is either placed after it or is missing!
Kernel->Add: WhateverGreen.kext is duplicated at Index 2 and 18!
Kernel->Patch[13] has different Find and Replace size (2 vs 3)!
CheckKernel returns 3 errors!

UEFI->Output->InitialMode is illegal (Can only be Auto, Text, or Graphics)!
CheckUefi returns 1 error!

Completed validating /tmp/config.plist in 7 ms. Found 6 issues requiring attention.
";

    #[test]
    fn parses_issue_lines() {
        let p = parse_output(SAMPLE_ERRORS);
        assert_eq!(p.issues.len(), 6);
        assert_eq!(p.reported_count, Some(6));
        assert!(!p.no_issues);
        assert_eq!(p.issues[0], "OCS: Missing key ClearTaskSwitchBit, context <Quirks>!");
        assert_eq!(ocvalidate_path(&p.issues[0]).as_deref(), Some("Quirks"));
        assert_eq!(ocvalidate_path(&p.issues[2]).as_deref(), Some("Kernel->Add[1]"));
        assert_eq!(ocvalidate_path(&p.issues[3]).as_deref(), Some("Kernel->Add"));
        assert_eq!(ocvalidate_path(&p.issues[5]).as_deref(), Some("UEFI->Output->InitialMode"));
    }

    #[test]
    fn parses_clean_run_and_crlf() {
        let out = "\r\nNOTE: This version of ocvalidate is only compatible with OpenCore version 1.0.8!\r\n\r\n\r\nCompleted validating EFI\\OC\\config.plist in 4 ms. No issues found.\r\n";
        let p = parse_output(out);
        assert!(p.issues.is_empty());
        assert!(p.no_issues);
        let run = OcValidateRun { status: Some(0), output: out.into() };
        assert!(ocvalidate_issues(&run).is_empty());
    }

    #[test]
    fn fatal_output_becomes_blocking() {
        let run = OcValidateRun {
            status: Some(255),
            output: "\nNOTE: x\n\nOCS: Couldn't parse serialized file!\nInvalid config\n".into(),
        };
        let issues = ocvalidate_issues(&run);
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|i| i.level == NoteLevel::Blocking && i.source == "ocvalidate"));

        let silent = OcValidateRun { status: Some(1), output: String::new() };
        assert_eq!(ocvalidate_issues(&silent).len(), 1);

        // Real 1.0.8 output for a config it cannot open.
        let unreadable = OcValidateRun {
            status: Some(255),
            output: "\nNOTE: This version of ocvalidate is only compatible with OpenCore version 1.0.8!\n\nFailed to read EFI/OC/config.plist\n".into(),
        };
        let issues = ocvalidate_issues(&unreadable);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Failed to read EFI/OC/config.plist");
        assert_eq!(issues[0].level, NoteLevel::Blocking);
    }

    #[test]
    fn parses_real_kernel_and_uefi_findings() {
        // Captured from ocvalidate 1.0.8 on a damaged Sample.plist.
        let out = "\nNOTE: This version of ocvalidate is only compatible with OpenCore version 1.0.8!\n\n\
OCS: No schema for Foo at 10 index, context <Quirks>!\n\
Serialisation returns 1 error!\n\n\
Kernel->Add[1]->MinKernel (currently set to abc) is borked!\n\
Kernel->Add: Lilu.kext is duplicated at Index 1 and 20!\n\
CheckKernel returns 2 errors!\n\n\
Misc->Security->SecureBootModel is borked!\n\
CheckMisc returns 1 error!\n\n\
UEFI->Drivers[0].Path contains illegal character!\n\
CheckUefi returns 1 error!\n\n\
Completed validating bad.plist in 2 ms. Found 5 issues requiring attention.\n";
        let p = parse_output(out);
        assert_eq!(p.issues.len(), 5);
        assert_eq!(p.reported_count, Some(5));
        let paths: Vec<Option<String>> = p.issues.iter().map(|l| ocvalidate_path(l)).collect();
        assert_eq!(
            paths,
            vec![
                Some("Quirks".to_string()),
                Some("Kernel->Add[1]->MinKernel".to_string()),
                Some("Kernel->Add".to_string()),
                Some("Misc->Security->SecureBootModel".to_string()),
                Some("UEFI->Drivers[0].Path".to_string()),
            ]
        );
    }

    #[test]
    fn darwin_versions() {
        assert_eq!(darwin_version("25.0.0"), Some(250_000));
        assert_eq!(darwin_version("20.99.99"), Some(209_999));
        assert_eq!(darwin_version("21"), Some(210_000));
        assert_eq!(darwin_version(""), None);
        assert_eq!(darwin_version("x"), None);
    }

    fn write_plist(path: &Path, value: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        value.to_file_xml(path).unwrap();
    }

    fn kext(kexts: &Path, rel: &str, id: &str, exe: Option<&str>, libs: &[&str]) {
        let dir = kexts.join(rel);
        let mut d = Dictionary::new();
        d.insert("CFBundleIdentifier".into(), id.into());
        if let Some(e) = exe {
            d.insert("CFBundleExecutable".into(), e.into());
            std::fs::create_dir_all(dir.join("Contents/MacOS")).unwrap();
            std::fs::write(dir.join("Contents/MacOS").join(e), b"\xcf\xfa\xed\xfe").unwrap();
        }
        let mut l = Dictionary::new();
        for lib in libs {
            l.insert((*lib).into(), "1.0".into());
        }
        d.insert("OSBundleLibraries".into(), Value::Dictionary(l));
        write_plist(&dir.join("Contents/Info.plist"), Value::Dictionary(d));
    }

    fn add_entry(bundle: &str, exe: &str, enabled: bool, min: &str, max: &str) -> Value {
        let mut d = Dictionary::new();
        d.insert("Arch".into(), "Any".into());
        d.insert("BundlePath".into(), bundle.into());
        d.insert("Comment".into(), "".into());
        d.insert("Enabled".into(), enabled.into());
        d.insert("ExecutablePath".into(), exe.into());
        d.insert("MaxKernel".into(), max.into());
        d.insert("MinKernel".into(), min.into());
        d.insert("PlistPath".into(), "Contents/Info.plist".into());
        Value::Dictionary(d)
    }

    fn path_entry(path: &str, enabled: bool) -> Value {
        let mut d = Dictionary::new();
        d.insert("Path".into(), path.into());
        d.insert("Enabled".into(), enabled.into());
        Value::Dictionary(d)
    }

    fn config(kernel_add: Vec<Value>, acpi: Vec<Value>, drivers: Vec<Value>, tools: Vec<Value>) -> Value {
        let mut root = Dictionary::new();
        let mut acpi_d = Dictionary::new();
        acpi_d.insert("Add".into(), Value::Array(acpi));
        root.insert("ACPI".into(), Value::Dictionary(acpi_d));
        let mut kernel = Dictionary::new();
        kernel.insert("Add".into(), Value::Array(kernel_add));
        root.insert("Kernel".into(), Value::Dictionary(kernel));
        let mut uefi = Dictionary::new();
        uefi.insert("Drivers".into(), Value::Array(drivers));
        root.insert("UEFI".into(), Value::Dictionary(uefi));
        let mut misc = Dictionary::new();
        misc.insert("Tools".into(), Value::Array(tools));
        let mut boot = Dictionary::new();
        boot.insert("PickerMode".into(), "Builtin".into());
        misc.insert("Boot".into(), Value::Dictionary(boot));
        root.insert("Misc".into(), Value::Dictionary(misc));
        Value::Dictionary(root)
    }

    /// A minimal well-formed ACPI table with a correct checksum.
    fn aml(sig: &[u8; 4]) -> Vec<u8> {
        let mut t = vec![0u8; 40];
        t[..4].copy_from_slice(sig);
        t[4..8].copy_from_slice(&40u32.to_le_bytes());
        t[8] = 2;
        t[10..16].copy_from_slice(b"ACDT  ");
        let sum = t.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        t[9] = 0u8.wrapping_sub(sum);
        t
    }

    #[test]
    fn aml_checks() {
        let good = aml(b"SSDT");
        assert!(check_aml(&good).is_ok());
        assert!(aml_checksum_ok(&good));
        let mut bad_sum = good.clone();
        bad_sum[39] = 1;
        assert!(check_aml(&bad_sum).is_ok());
        assert!(!aml_checksum_ok(&bad_sum));
        assert!(check_aml(b"<html>").is_err());
        let mut html = b"<!DOCTYPE html><html><body>Not Found</body></html>".to_vec();
        html.resize(64, b' ');
        assert!(check_aml(&html).unwrap_err().contains("signature"));
        let mut short = good.clone();
        short.truncate(38);
        assert!(check_aml(&short).unwrap_err().contains("header says 40"));
    }

    fn skeleton(efi: &Path) {
        std::fs::create_dir_all(efi.join("BOOT")).unwrap();
        std::fs::create_dir_all(efi.join("OC/Kexts")).unwrap();
        std::fs::write(efi.join("BOOT/BOOTx64.efi"), b"MZ").unwrap();
        std::fs::write(efi.join("OC/OpenCore.efi"), b"MZ").unwrap();
        std::fs::create_dir_all(efi.join("OC/ACPI")).unwrap();
        std::fs::create_dir_all(efi.join("OC/Drivers")).unwrap();
        std::fs::create_dir_all(efi.join("OC/Tools")).unwrap();
    }

    #[tokio::test]
    async fn valid_layout_passes() {
        let tmp = TempDir::new();
        let efi = tmp.0.join("EFI");
        skeleton(&efi);
        let kexts = efi.join("OC/Kexts");
        kext(&kexts, "Lilu.kext", "as.vit9696.Lilu", Some("Lilu"), &["com.apple.kpi.bsd"]);
        kext(&kexts, "VirtualSMC.kext", "as.vit9696.VirtualSMC", Some("VirtualSMC"), &["as.vit9696.Lilu"]);
        kext(&kexts, "USBToolBox.kext", "com.dhinakg.USBToolBox.kext", Some("USBToolBox"), &[]);
        kext(&kexts, "UTBDefault.kext", "com.dhinakg.USBToolBox.injector", None, &["com.dhinakg.USBToolBox.kext"]);
        std::fs::write(efi.join("OC/ACPI/SSDT-EC.aml"), aml(b"SSDT")).unwrap();
        std::fs::write(efi.join("OC/Drivers/OpenRuntime.efi"), b"MZ").unwrap();
        std::fs::write(efi.join("OC/Tools/OpenShell.efi"), b"MZ").unwrap();
        let cfg = config(
            vec![
                add_entry("Lilu.kext", "Contents/MacOS/Lilu", true, "", ""),
                add_entry("VirtualSMC.kext", "Contents/MacOS/VirtualSMC", true, "", ""),
                add_entry("USBToolBox.kext", "Contents/MacOS/USBToolBox", true, "", ""),
                add_entry("UTBDefault.kext", "", true, "", ""),
                add_entry("Missing.kext", "Contents/MacOS/Missing", false, "", ""),
            ],
            vec![path_entry("SSDT-EC.aml", true), path_entry("DSDT.aml", false)],
            vec![path_entry("OpenRuntime.efi", true), path_entry("HfsPlus.efi", false)],
            vec![path_entry("OpenShell.efi", true)],
        );
        write_plist(&efi.join("OC/config.plist"), cfg);
        let r = validate_efi(&efi, None).await;
        assert!(r.valid, "{:#?}", r.issues);
        assert!(!r.ocvalidate_ran);
        assert!(r.issues.iter().all(|i| i.level == NoteLevel::Info));
    }

    #[tokio::test]
    async fn reports_missing_files_order_and_duplicates() {
        let tmp = TempDir::new();
        let efi = tmp.0.join("EFI");
        skeleton(&efi);
        std::fs::remove_file(efi.join("BOOT/BOOTx64.efi")).unwrap();
        let kexts = efi.join("OC/Kexts");
        kext(&kexts, "Lilu.kext", "as.vit9696.Lilu", Some("Lilu"), &[]);
        kext(&kexts, "WhateverGreen.kext", "as.vit9696.WhateverGreen", Some("WhateverGreen"), &["as.vit9696.Lilu"]);
        kext(&kexts, "SMCProcessor.kext", "as.vit9696.SMCProcessor", Some("SMCProcessor"), &["as.vit9696.VirtualSMC"]);
        kext(
            &kexts,
            "A.kext/Contents/PlugIns/VoodooInput.kext",
            "me.kishorprins.VoodooInput",
            Some("VoodooInput"),
            &[],
        );
        kext(
            &kexts,
            "B.kext/Contents/PlugIns/VoodooInput.kext",
            "me.kishorprins.VoodooInput",
            Some("VoodooInput"),
            &[],
        );
        kext(&kexts, "Codeful.kext", "org.example.Codeful", Some("Codeful"), &[]);
        let cfg = config(
            vec![
                add_entry("WhateverGreen.kext", "Contents/MacOS/WhateverGreen", true, "", ""),
                add_entry("Lilu.kext", "Contents/MacOS/Lilu", true, "", ""),
                add_entry("SMCProcessor.kext", "Contents/MacOS/SMCProcessor", true, "", ""),
                add_entry("A.kext/Contents/PlugIns/VoodooInput.kext", "Contents/MacOS/VoodooInput", true, "", ""),
                add_entry("B.kext/Contents/PlugIns/VoodooInput.kext", "Contents/MacOS/VoodooInput", true, "21.0.0", ""),
                add_entry("Ghost.kext", "Contents/MacOS/Ghost", true, "", ""),
                add_entry("Codeful.kext", "", true, "", ""),
                add_entry("Lilu.kext", "Contents/MacOS/Lilu", true, "", ""),
            ],
            vec![
                path_entry("SSDT-PLUG.aml", true),
                path_entry("SSDT-HTML.aml", true),
                path_entry("SSDT-SUM.aml", true),
            ],
            vec![path_entry("OpenRuntime.efi", true)],
            vec![],
        );
        write_plist(&efi.join("OC/config.plist"), cfg);
        std::fs::write(efi.join("OC/ACPI/SSDT-HTML.aml"), b"<html><body>rate limited</body></html>").unwrap();
        let mut bad_sum = aml(b"SSDT");
        bad_sum[20] ^= 0xff;
        std::fs::write(efi.join("OC/ACPI/SSDT-SUM.aml"), bad_sum).unwrap();
        let r = validate_efi(&efi, None).await;
        assert!(!r.valid);
        let msgs: Vec<&str> = r.issues.iter().map(|i| i.message.as_str()).collect();
        let has = |needle: &str| msgs.iter().any(|m| m.contains(needle));
        assert!(has("BOOT/BOOTx64.efi"), "{msgs:#?}");
        assert!(has("ACPI/SSDT-PLUG.aml"));
        assert!(has("Drivers/OpenRuntime.efi"));
        assert!(has("WhateverGreen.kext loads before its dependency Lilu.kext"));
        assert!(has("SMCProcessor.kext depends on as.vit9696.VirtualSMC"));
        assert!(has("bundle id me.kishorprins.VoodooInput"));
        assert!(has("Ghost.kext is not in EFI/OC/Kexts"));
        assert!(has("Lilu.kext is enabled twice"));
        assert!(has("Lilu.kext should be the first"));
        assert!(has("ACPI/SSDT-HTML.aml is not a valid ACPI table"));
        let sum = r.issues.iter().find(|i| i.message.contains("SSDT-SUM.aml has a wrong table checksum")).unwrap();
        assert_eq!(sum.level, NoteLevel::Warning);
        assert_eq!(sum.path.as_deref(), Some("ACPI->Add[2]"));
        let codeless_warning = r.issues.iter().find(|i| i.message.contains("Codeful.kext has an executable")).unwrap();
        assert_eq!(codeless_warning.level, NoteLevel::Warning);
    }

    #[tokio::test]
    async fn disjoint_kernel_ranges_allow_same_bundle_id() {
        let tmp = TempDir::new();
        let efi = tmp.0.join("EFI");
        skeleton(&efi);
        let kexts = efi.join("OC/Kexts");
        kext(&kexts, "Old.kext", "com.example.Same", Some("Old"), &[]);
        kext(&kexts, "New.kext", "com.example.Same", Some("New"), &[]);
        let cfg = config(
            vec![
                add_entry("Old.kext", "Contents/MacOS/Old", true, "", "20.99.99"),
                add_entry("New.kext", "Contents/MacOS/New", true, "21.0.0", ""),
            ],
            vec![],
            vec![],
            vec![],
        );
        write_plist(&efi.join("OC/config.plist"), cfg);
        let r = validate_efi(&efi, None).await;
        assert!(r.valid, "{:#?}", r.issues);
    }

    #[tokio::test]
    async fn missing_or_broken_config_is_blocking() {
        let tmp = TempDir::new();
        let efi = tmp.0.join("EFI");
        skeleton(&efi);
        let r = validate_efi(&efi, None).await;
        assert!(!r.valid);
        assert!(r.issues.iter().any(|i| i.message.contains("config.plist is missing")));

        std::fs::write(efi.join("OC/config.plist"), b"not a plist").unwrap();
        let r = validate_efi(&efi, None).await;
        assert!(r.issues.iter().any(|i| i.message.contains("cannot be parsed")));

        let r = validate_efi(&tmp.0.join("nope"), None).await;
        assert!(!r.valid);
    }

    #[test]
    fn case_insensitive_resolution() {
        let tmp = TempDir::new();
        std::fs::create_dir_all(tmp.0.join("OC/Drivers")).unwrap();
        std::fs::write(tmp.0.join("OC/Drivers/OpenRuntime.efi"), b"x").unwrap();
        assert!(resolve_ci(&tmp.0, "oc/drivers/openruntime.EFI").is_some());
        assert!(resolve_ci(&tmp.0, "OC/../OC").is_none());
        assert!(resolve_ci(&tmp.0, "/etc").is_none());
        assert!(resolve_ci(&tmp.0, "OC/Drivers/Missing.efi").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_a_validator_binary() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new();
        let efi = tmp.0.join("EFI");
        skeleton(&efi);
        write_plist(&efi.join("OC/config.plist"), config(vec![], vec![], vec![], vec![]));
        let script = tmp.0.join("fake-ocvalidate");
        std::fs::write(
            &script,
            "#!/bin/sh\necho\necho 'NOTE: This version of ocvalidate is only compatible with OpenCore version 1.0.8!'\necho\necho 'OCS: Missing key Foo, context <Bar>!'\necho 'Serialisation returns 1 error!'\necho \"Completed validating $1 in 1 ms. Found 1 issue requiring attention.\"\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = validate_efi(&efi, Some(&script)).await;
        assert!(r.ocvalidate_ran);
        assert!(!r.valid);
        let ocv: Vec<_> = r.issues.iter().filter(|i| i.source == "ocvalidate").collect();
        assert_eq!(ocv.len(), 1, "{:#?}", r.issues);
        assert_eq!(ocv[0].path.as_deref(), Some("Bar"));
        assert!(r.ocvalidate_output.unwrap_or_default().contains("Found 1 issue"));

        let r = validate_efi(&efi, Some(&tmp.0.join("does-not-exist"))).await;
        assert!(!r.ocvalidate_ran);
        assert!(r.valid);
        assert!(r.issues.iter().any(|i| i.level == NoteLevel::Warning && i.source == "ocvalidate"));
    }
}
