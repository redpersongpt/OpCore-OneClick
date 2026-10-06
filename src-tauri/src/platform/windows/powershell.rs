//! Runs the inventory script with Windows PowerShell: absolute path, no
//! profile, no console window, killed on timeout or cancellation.

use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::error::AppError;
use crate::platform::common::capture_output;
use crate::tasks::cancellation::CancellationToken;

use super::inventory::{decode_output, encode_command, extract_payload, parse, Inventory, SCRIPT};

/// `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`, else the
/// first `powershell.exe` on PATH.
fn powershell_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let full = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    if full.is_file() {
        full
    } else {
        PathBuf::from("powershell.exe")
    }
}

pub async fn run_inventory(
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Inventory, AppError> {
    let mut command = Command::new(powershell_exe());
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
        ])
        .arg(encode_command(SCRIPT))
        .creation_flags(CREATE_NO_WINDOW);
    let output = capture_output(command, timeout, cancel, "PowerShell").await?;
    let stdout = decode_output(&output.stdout);
    let Some(payload) = extract_payload(&stdout) else {
        let stderr = decode_output(&output.stderr);
        let detail = stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("no output");
        return Err(AppError::new(
            "SCAN_PROCESS",
            format!(
                "PowerShell returned no device inventory (exit code {:?}): {detail}",
                output.status.code()
            ),
        ));
    };
    parse(payload)
}
