//! Child-process helpers: timeouts, no console window on Windows, UTF-16/OEM
//! output decoding, and privilege elevation for disk operations.

use std::time::Duration;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run a program with arguments (never through a shell), with timeout.
pub async fn run(program: &str, args: &[&str], timeout: Duration) -> Result<CommandOutput, AppError> {
    todo!("run {program} {args:?} {timeout:?}")
}

/// Run a PowerShell script (Windows only) with -NoProfile -NonInteractive
/// -ExecutionPolicy Bypass, UTF-8 output.
pub async fn powershell(script: &str, timeout: Duration) -> Result<CommandOutput, AppError> {
    todo!("powershell {} {timeout:?}", script.len())
}

/// True when the current process has administrator/root rights.
pub fn is_elevated() -> bool {
    todo!()
}

/// Run a shell script with elevated privileges: as-is when already elevated,
/// via `pkexec` on Linux, `osascript ... with administrator privileges` on
/// macOS. On Windows the app manifest already requires administrator.
pub async fn run_elevated_script(script: &str, timeout: Duration) -> Result<CommandOutput, AppError> {
    todo!("run_elevated_script {} {timeout:?}", script.len())
}
