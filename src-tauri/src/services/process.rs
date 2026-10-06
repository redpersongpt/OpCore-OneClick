//! Child-process helpers: timeouts, no console window on Windows, UTF-16/OEM
//! output decoding, and privilege elevation for disk operations.

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tracing::{debug, warn};

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == 0
    }

    /// stdout and stderr joined and shortened, for error messages.
    pub fn summary(&self) -> String {
        let mut text = String::new();
        for part in [self.stderr.trim(), self.stdout.trim()] {
            if part.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(part);
        }
        shorten(&text, 2000)
    }

    /// Turn a non-zero exit status into a `COMMAND_FAILED` error that carries
    /// the tool output.
    pub fn ensure_success(self, what: &str) -> Result<Self, AppError> {
        if self.success() {
            return Ok(self);
        }
        let summary = self.summary();
        let message = if summary.is_empty() {
            format!("{what} failed with exit code {}", self.status)
        } else {
            format!("{what} failed with exit code {}: {summary}", self.status)
        };
        Err(AppError::new("COMMAND_FAILED", message)
            .with_context(serde_json::json!({ "exitCode": self.status })))
    }
}

/// Keep the last `max` characters (tool errors are usually at the end).
fn shorten(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max).collect();
    format!("...{tail}")
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn command(program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn spawn_error(program: &str, err: std::io::Error) -> AppError {
    if err.kind() == std::io::ErrorKind::NotFound {
        AppError::new("TOOL_NOT_FOUND", format!("{program} is not installed or not on PATH"))
            .with_context(serde_json::json!({ "program": program }))
    } else if err.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::new("PERMISSION_DENIED", format!("{program} could not be started: {err}"))
    } else {
        AppError::new("COMMAND_ERROR", format!("{program} could not be started: {err}"))
    }
}

/// Run a program with arguments (never through a shell), with timeout.
///
/// A non-zero exit status is returned as `Ok` with `status` set; only spawn
/// failures and timeouts are errors. A timed-out child is killed.
pub async fn run(program: &str, args: &[&str], timeout: Duration) -> Result<CommandOutput, AppError> {
    debug!(program, ?args, "running");
    let child = command(program, args).spawn().map_err(|e| spawn_error(program, e))?;
    // Dropping the future on timeout drops the child, and kill_on_drop ends it.
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: decode_output(&output.stdout),
            stderr: decode_output(&output.stderr),
        }),
        Ok(Err(e)) => Err(AppError::new("COMMAND_ERROR", format!("{program}: {e}"))),
        Err(_) => {
            warn!(program, "timed out after {}s", timeout.as_secs());
            Err(AppError::new(
                "COMMAND_TIMEOUT",
                format!("{program} did not finish within {} seconds", timeout.as_secs()),
            )
            .recoverable())
        }
    }
}

/// Decode tool output: UTF-8 (with or without BOM), UTF-16LE (BOM or
/// detected), otherwise lossy UTF-8. Line endings are normalised to `\n`.
pub fn decode_output(bytes: &[u8]) -> String {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        decode_utf16le(rest)
    } else if looks_like_utf16le(bytes) {
        decode_utf16le(bytes)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.replace("\r\n", "\n")
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// ASCII text encoded as UTF-16LE has a zero in every odd byte.
fn looks_like_utf16le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let sample = &bytes[..bytes.len().min(512)];
    let pairs = sample.len() / 2;
    let zero_high = sample.as_chunks::<2>().0.iter().filter(|c| c[1] == 0 && c[0] != 0).count();
    zero_high * 10 >= pairs * 9
}

/// Absolute path of Windows PowerShell 5.1 (present on every Windows 10/11).
#[cfg(windows)]
fn powershell_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// Prefix for every PowerShell script: stop on errors, no progress records on
/// stderr, UTF-8 output.
const POWERSHELL_PRELUDE: &str = "$ErrorActionPreference = 'Stop'\n\
$ProgressPreference = 'SilentlyContinue'\n\
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}\n\
$OutputEncoding = [System.Text.Encoding]::UTF8\n";

/// `-EncodedCommand` payload: base64 of the UTF-16LE script.
pub fn encode_powershell(script: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let full = format!("{POWERSHELL_PRELUDE}{script}");
    let bytes: Vec<u8> = full.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    STANDARD.encode(bytes)
}

/// Run a PowerShell script (Windows only) with -NoProfile -NonInteractive
/// -ExecutionPolicy Bypass, UTF-8 output.
pub async fn powershell(script: &str, timeout: Duration) -> Result<CommandOutput, AppError> {
    #[cfg(windows)]
    {
        let encoded = encode_powershell(script);
        // CreateProcess limits the whole command line to 32767 characters.
        if encoded.len() > 30_000 {
            return Err(AppError::new("SCRIPT_TOO_LONG", "PowerShell script is too long to pass on the command line"));
        }
        let exe = powershell_exe();
        return run(
            &exe,
            &["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded],
            timeout,
        )
        .await;
    }
    #[allow(unreachable_code)]
    {
        let _ = (script, timeout);
        Err(AppError::new("UNSUPPORTED_PLATFORM", "PowerShell is only available on Windows"))
    }
}

/// Quote a string for PowerShell single-quoted literals.
pub fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Quote a string for POSIX sh.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// True when the current process has administrator/root rights.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        return windows_token_elevated();
    }
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        return unsafe { libc::geteuid() } == 0;
    }
    #[allow(unreachable_code)]
    false
}

#[cfg(windows)]
fn windows_token_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo handle; `token` is a valid out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: `token` was opened above with TOKEN_QUERY; the buffer is a
    // TOKEN_ELEVATION of the size passed.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    // SAFETY: closing the handle we opened.
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

/// How the app can obtain administrator rights for disk operations, if at all.
pub fn elevation_method() -> Option<&'static str> {
    if is_elevated() {
        return Some(if cfg!(windows) { "administrator" } else { "root" });
    }
    #[cfg(target_os = "linux")]
    {
        if find_in_path("pkexec").is_some() {
            return Some("pkexec");
        }
        if find_in_path("sudo").is_some() {
            return Some("sudo");
        }
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        return Some("osascript");
    }
    #[allow(unreachable_code)]
    None
}

/// Locate an executable in PATH plus the usual sbin directories (which a
/// desktop session's PATH often lacks).
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for extra in ["/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin"] {
        dirs.push(PathBuf::from(extra));
    }
    dirs.into_iter().map(|d| d.join(program)).find(|p| p.is_file())
}

/// Run a shell script with elevated privileges: as-is when already elevated,
/// via `pkexec` on Linux, `osascript ... with administrator privileges` on
/// macOS. On Windows the app manifest already requires administrator and the
/// script is PowerShell.
pub async fn run_elevated_script(script: &str, timeout: Duration) -> Result<CommandOutput, AppError> {
    #[cfg(windows)]
    {
        if !is_elevated() {
            return Err(admin_required());
        }
        return powershell(script, timeout).await;
    }
    #[cfg(unix)]
    {
        let scratch = ElevatedScratch::new()?;
        return run_elevated_script_in(&scratch, script, timeout).await;
    }
    #[allow(unreachable_code)]
    {
        let _ = (script, timeout);
        Err(AppError::new("UNSUPPORTED_PLATFORM", "Elevation is not supported on this OS"))
    }
}

pub fn admin_required() -> AppError {
    AppError::new("ADMIN_REQUIRED", "Writing a USB drive needs administrator rights")
        .with_suggestion("Close OpCore-OneClick and start it again with \"Run as administrator\".")
}

/// Private (0700) scratch directory for one elevated run. The script sees it
/// as `$SCRATCH_DIR`; anything appended to `$SCRATCH_DIR/progress` can be
/// polled by the caller while the script runs.
#[cfg(unix)]
pub struct ElevatedScratch {
    dir: PathBuf,
}

#[cfg(unix)]
const SCRATCH_FILES: [&str; 6] = ["script.sh", "wrapper.sh", "out", "err", "status", "progress"];

#[cfg(unix)]
impl ElevatedScratch {
    pub fn new() -> Result<Self, AppError> {
        use std::os::unix::fs::DirBuilderExt;
        let dir = std::env::temp_dir().join(format!("opcore-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| {
            AppError::new("IO_ERROR", format!("Cannot create a temporary folder in {}: {e}", dir.display()))
        })?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Create an empty sub-directory (for example a mount point).
    pub fn subdir(&self, name: &str) -> Result<PathBuf, AppError> {
        use std::os::unix::fs::DirBuilderExt;
        let path = self.dir.join(name);
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(path)
    }
}

#[cfg(unix)]
impl Drop for ElevatedScratch {
    fn drop(&mut self) {
        // Never delete recursively: a sub-directory may still be a mount point.
        for name in SCRATCH_FILES {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_dir(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// Run `script` (POSIX sh) as root inside `scratch`. The script's exit status,
/// stdout and stderr are returned; failing to obtain root is an error
/// (`ELEVATION_CANCELLED` or `ELEVATION_UNAVAILABLE`).
#[cfg(unix)]
pub async fn run_elevated_script_in(
    scratch: &ElevatedScratch,
    script: &str,
    timeout: Duration,
) -> Result<CommandOutput, AppError> {
    use std::os::unix::fs::PermissionsExt;

    let script_path = scratch.file("script.sh");
    let wrapper_path = scratch.file("wrapper.sh");
    let out = scratch.file("out");
    let err = scratch.file("err");
    let status = scratch.file("status");
    std::fs::write(&script_path, script)?;
    std::fs::write(scratch.file("progress"), b"")?;
    let wrapper = elevated_wrapper(scratch.path(), &script_path, &out, &err, &status);
    std::fs::write(&wrapper_path, wrapper)?;
    for path in [&script_path, &wrapper_path] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let wrapper_str = wrapper_path.to_string_lossy().into_owned();

    let launch = if is_elevated() {
        run("/bin/sh", &[&wrapper_str], timeout).await?
    } else {
        launch_elevated(&wrapper_str, timeout).await?
    };

    match std::fs::read_to_string(&status) {
        Ok(code) => Ok(CommandOutput {
            status: code.trim().parse().unwrap_or(-1),
            stdout: std::fs::read(&out).map(|b| decode_output(&b)).unwrap_or_default(),
            stderr: std::fs::read(&err).map(|b| decode_output(&b)).unwrap_or_default(),
        }),
        // The wrapper never ran as root: the prompt was dismissed or refused.
        Err(_) => Err(elevation_failure(&launch)),
    }
}

/// The files root creates must stay readable by the user (pkexec and
/// osascript inherit whatever umask the session has, 077 on hardened setups).
#[cfg(unix)]
fn elevated_wrapper(dir: &Path, script: &Path, out: &Path, err: &Path, status: &Path) -> String {
    let q = |p: &Path| sh_quote(&p.to_string_lossy());
    format!(
        "umask 022\nSCRATCH_DIR={dir}\nexport SCRATCH_DIR\n/bin/sh {script} >{out} 2>{err} </dev/null\necho $? >{status}\n",
        dir = q(dir),
        script = q(script),
        out = q(out),
        err = q(err),
        status = q(status),
    )
}

#[cfg(target_os = "linux")]
async fn launch_elevated(wrapper: &str, timeout: Duration) -> Result<CommandOutput, AppError> {
    if let Some(pkexec) = find_in_path("pkexec") {
        let pkexec = pkexec.to_string_lossy().into_owned();
        let output = run(&pkexec, &["/bin/sh", wrapper], timeout).await?;
        // 126: the user dismissed the dialog. 127: no agent / not authorised.
        if output.status != 127 {
            return Ok(output);
        }
        warn!("pkexec could not authenticate, trying sudo -n: {}", output.summary());
    }
    if let Some(sudo) = find_in_path("sudo") {
        let sudo = sudo.to_string_lossy().into_owned();
        return run(&sudo, &["-n", "/bin/sh", wrapper], timeout).await;
    }
    Err(AppError::new(
        "ELEVATION_UNAVAILABLE",
        "Neither pkexec nor sudo is available to run the disk operation as root",
    )
    .with_suggestion("Install polkit (pkexec) or run OpCore-OneClick from a desktop session with an authentication agent."))
}

#[cfg(target_os = "macos")]
async fn launch_elevated(wrapper: &str, timeout: Duration) -> Result<CommandOutput, AppError> {
    let script = macos_admin_applescript(wrapper);
    run("/usr/bin/osascript", &["-e", &script], timeout).await
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
async fn launch_elevated(_wrapper: &str, _timeout: Duration) -> Result<CommandOutput, AppError> {
    Err(AppError::new("ELEVATION_UNAVAILABLE", "Elevation is not supported on this OS"))
}

/// AppleScript that runs `/bin/sh <wrapper>` with an administrator prompt.
/// The path is escaped for an AppleScript string literal and then shell-quoted
/// by `quoted form of`.
pub fn macos_admin_applescript(wrapper: &str) -> String {
    let literal = wrapper.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "with timeout of 7200 seconds\n\
         do shell script \"/bin/sh \" & quoted form of \"{literal}\" with prompt \"OpCore-OneClick needs your password to erase and write the selected USB drive.\" with administrator privileges without altering line endings\n\
         end timeout"
    )
}

#[cfg(unix)]
fn elevation_failure(launch: &CommandOutput) -> AppError {
    let text = launch.summary();
    let lower = text.to_lowercase();
    let cancelled = launch.status == 126
        || lower.contains("user canceled")
        || lower.contains("user cancelled")
        || lower.contains("(-128)")
        || lower.contains("dismissed");
    if cancelled {
        AppError::new("ELEVATION_CANCELLED", "The administrator password prompt was cancelled").recoverable()
    } else {
        AppError::new(
            "ELEVATION_UNAVAILABLE",
            if text.is_empty() {
                "Could not obtain administrator rights".to_string()
            } else {
                format!("Could not obtain administrator rights: {text}")
            },
        )
        .with_suggestion("Make sure your account is an administrator and a password prompt can be shown.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_with_bom_and_crlf() {
        let bytes = b"\xEF\xBB\xBFhello\r\nworld\r\n";
        assert_eq!(decode_output(bytes), "hello\nworld\n");
    }

    #[test]
    fn decodes_utf16le_with_and_without_bom() {
        let text = "Disk 2 ‑ USB";
        let mut with_bom = vec![0xFF, 0xFE];
        with_bom.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(decode_output(&with_bom), text);

        let ascii: Vec<u8> = "DISKPART> list disk".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_output(&ascii), "DISKPART> list disk");
    }

    #[test]
    fn invalid_utf8_is_decoded_lossily() {
        let out = decode_output(b"Datentr\x84ger");
        assert!(out.starts_with("Datentr"));
        assert!(out.ends_with("ger"));
    }

    #[test]
    fn summary_prefers_stderr_and_is_bounded() {
        let output = CommandOutput { status: 1, stdout: "x".repeat(5000), stderr: "boom".into() };
        let summary = output.summary();
        assert!(summary.starts_with("..."));
        assert!(summary.chars().count() <= 2003);
        let small = CommandOutput { status: 1, stdout: "out".into(), stderr: "err".into() };
        assert_eq!(small.summary(), "err\nout");
    }

    #[test]
    fn ensure_success_reports_exit_code() {
        let ok = CommandOutput { status: 0, stdout: String::new(), stderr: String::new() };
        assert!(ok.ensure_success("tool").is_ok());
        let bad = CommandOutput { status: 4, stdout: "Virtual Disk Service error".into(), stderr: String::new() };
        let err = bad.ensure_success("diskpart").unwrap_err();
        assert_eq!(err.code, "COMMAND_FAILED");
        assert!(err.message.contains("exit code 4"));
        assert!(err.message.contains("Virtual Disk Service"));
    }

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(ps_quote("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn applescript_escapes_path() {
        let script = macos_admin_applescript("/tmp/a\"b\\c/wrapper.sh");
        assert!(script.contains("quoted form of \"/tmp/a\\\"b\\\\c/wrapper.sh\""));
        assert!(script.contains("with administrator privileges"));
    }

    #[test]
    fn powershell_payload_is_utf16_base64() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let encoded = encode_powershell("Write-Output 1");
        let bytes = STANDARD.decode(encoded).unwrap();
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let text = String::from_utf16(&units).unwrap();
        assert!(text.starts_with("$ErrorActionPreference = 'Stop'"));
        assert!(text.ends_with("Write-Output 1"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_captures_output_and_status() {
        let out = run("/bin/sh", &["-c", "echo out; echo err >&2; exit 3"], Duration::from_secs(10)).await.unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_times_out_and_kills() {
        let started = std::time::Instant::now();
        let err = run("/bin/sh", &["-c", "sleep 30"], Duration::from_millis(300)).await.unwrap_err();
        assert_eq!(err.code, "COMMAND_TIMEOUT");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn missing_program_is_reported() {
        let err = run("opcore-no-such-tool", &[], Duration::from_secs(5)).await.unwrap_err();
        assert_eq!(err.code, "TOOL_NOT_FOUND");
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_quotes_paths() {
        let dir = Path::new("/tmp/x y");
        let w = elevated_wrapper(dir, &dir.join("script.sh"), &dir.join("out"), &dir.join("err"), &dir.join("status"));
        assert!(w.starts_with("umask 022\n"));
        assert!(w.contains("SCRATCH_DIR='/tmp/x y'"));
        assert!(w.contains("/bin/sh '/tmp/x y/script.sh' >'/tmp/x y/out' 2>'/tmp/x y/err' </dev/null"));
        assert!(w.ends_with("echo $? >'/tmp/x y/status'\n"));
    }

    #[cfg(unix)]
    #[test]
    fn scratch_cleanup_does_not_recurse() {
        let scratch = ElevatedScratch::new().unwrap();
        let dir = scratch.path().to_path_buf();
        let sub = scratch.subdir("mnt").unwrap();
        std::fs::write(sub.join("keep.txt"), b"data").unwrap();
        std::fs::write(scratch.file("out"), b"x").unwrap();
        drop(scratch);
        // The non-empty sub-directory (a stand-in for a mount point) survives.
        assert!(sub.join("keep.txt").exists());
        assert!(!dir.join("out").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
