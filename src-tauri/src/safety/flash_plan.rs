//! Destructive scripts and progress mapping for writing the USB drive:
//! diskpart scripts (Windows), the single elevated shell script (Linux),
//! partition sizing and the overall progress of each flash phase.

use std::path::Path;

use crate::error::AppError;
use crate::safety::device_path::{validate_fat_label, validate_linux_disk};
use crate::services::process::{ps_quote, sh_quote};

/// Volume label of the USB partition.
pub const VOLUME_LABEL: &str = "OPENCORE";

/// `format fs=fat32` refuses volumes over 32 GB, so the partition is capped.
pub const WINDOWS_FAT32_MAX_MB: u64 = 32_000;
/// Room for the GPT, the MSR partition on fixed disks and rounding.
pub const PARTITION_HEADROOM_MB: u64 = 256;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashPhase {
    Prepare,
    Partition,
    Format,
    CopyEfi,
    CopyRecovery,
    Verify,
    Complete,
}

impl FlashPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            FlashPhase::Prepare => "prepare",
            FlashPhase::Partition => "partition",
            FlashPhase::Format => "format",
            FlashPhase::CopyEfi => "copy-efi",
            FlashPhase::CopyRecovery => "copy-recovery",
            FlashPhase::Verify => "verify",
            FlashPhase::Complete => "complete",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim() {
            "prepare" => FlashPhase::Prepare,
            "partition" => FlashPhase::Partition,
            "format" => FlashPhase::Format,
            "copy-efi" => FlashPhase::CopyEfi,
            "copy-recovery" => FlashPhase::CopyRecovery,
            "verify" => FlashPhase::Verify,
            "complete" => FlashPhase::Complete,
            _ => return None,
        })
    }

    /// Share of the overall progress bar: (start, end).
    fn range(self, with_recovery: bool) -> (f64, f64) {
        match (self, with_recovery) {
            (FlashPhase::Prepare, true) => (0.0, 0.03),
            (FlashPhase::Partition, true) => (0.03, 0.08),
            (FlashPhase::Format, true) => (0.08, 0.12),
            (FlashPhase::CopyEfi, true) => (0.12, 0.20),
            (FlashPhase::CopyRecovery, true) => (0.20, 0.92),
            (FlashPhase::Verify, true) => (0.92, 0.99),
            (FlashPhase::Prepare, false) => (0.0, 0.05),
            (FlashPhase::Partition, false) => (0.05, 0.20),
            (FlashPhase::Format, false) => (0.20, 0.35),
            (FlashPhase::CopyEfi, false) => (0.35, 0.85),
            (FlashPhase::CopyRecovery, false) => (0.85, 0.85),
            (FlashPhase::Verify, false) => (0.85, 0.99),
            (FlashPhase::Complete, _) => (1.0, 1.0),
        }
    }

    /// Overall progress for `fraction` (0..1) of this phase.
    pub fn overall(self, fraction: f64, with_recovery: bool) -> f64 {
        let (start, end) = self.range(with_recovery);
        start + (end - start) * fraction.clamp(0.0, 1.0)
    }
}

/// Partition sizes (MiB) to try on Windows, largest first: the disk minus
/// headroom, capped at [`WINDOWS_FAT32_MAX_MB`]; smaller fallbacks are used
/// when Windows still refuses to format the volume as FAT32.
pub fn windows_partition_sizes(disk_bytes: u64) -> Vec<u64> {
    let usable = (disk_bytes / MIB).saturating_sub(PARTITION_HEADROOM_MB);
    if usable == 0 {
        return Vec::new();
    }
    let first = usable.min(WINDOWS_FAT32_MAX_MB);
    let mut sizes = vec![first];
    sizes.extend([16_000u64, 8_000].into_iter().filter(|fallback| *fallback < first));
    sizes
}

/// Bytes available on a partition of `size_mb` MiB.
pub fn partition_capacity(size_mb: u64) -> u64 {
    size_mb * MIB
}

/// One diskpart run: wipe the disk, convert it to GPT, create one Basic Data
/// partition of `size_mb` MiB, quick-format it FAT32 and give it a drive
/// letter (when none is free the volume is still reachable by its GUID path).
pub fn diskpart_flash_script(disk: u32, size_mb: u64, label: &str) -> Result<String, AppError> {
    let label = validate_fat_label(label)?;
    Ok([
        format!("select disk {disk}"),
        "attributes disk clear readonly noerr".to_string(),
        "online disk noerr".to_string(),
        "clean".to_string(),
        "convert gpt".to_string(),
        format!("create partition primary size={size_mb}"),
        format!("format fs=fat32 quick label={label}"),
        "assign noerr".to_string(),
        "exit".to_string(),
        String::new(),
    ]
    .join("\r\n"))
}

/// Ask the Virtual Disk Service to re-read the disks after a failure.
pub fn diskpart_rescan_script() -> String {
    "rescan\r\nexit\r\n".to_string()
}

/// "Virtual Disk Service" and device-busy failures are often transient.
pub fn diskpart_error_is_transient(output: &str, exit_code: i32) -> bool {
    let lower = output.to_lowercase();
    exit_code == 4
        || lower.contains("virtual disk service")
        || lower.contains("device is not ready")
        || lower.contains("not up to date")
        || lower.contains("access is denied")
}

/// The FAT32 volume was too large for Windows' formatter.
pub fn diskpart_error_is_size(output: &str) -> bool {
    output.to_lowercase().contains("too big")
}

/// What the disk must still look like right before diskpart runs.
#[derive(Debug, Clone)]
pub struct WindowsIdentity<'a> {
    pub size_bytes: u64,
    pub serial: Option<&'a str>,
}

/// PowerShell that (optionally) re-checks the disk identity and then runs a
/// diskpart script, decoding diskpart's OEM-code-page output correctly.
/// Prints `IDENTITY:changed` or the diskpart output followed by
/// `DISKPART-EXIT:<code>`.
pub fn windows_diskpart_ps(disk: u32, script: &str, identity: Option<&WindowsIdentity<'_>>) -> String {
    let lines: Vec<String> = script.lines().filter(|l| !l.trim().is_empty()).map(ps_quote).collect();
    let check = match identity {
        Some(identity) => {
            let serial = identity.serial.map(|s| s.trim()).unwrap_or("");
            format!(
                "$d = Get-Disk -Number {disk} -ErrorAction SilentlyContinue\n\
                 if ($null -eq $d) {{ Write-Output 'IDENTITY:changed'; exit 0 }}\n\
                 $serial = ([string]$d.SerialNumber).Trim()\n\
                 $expectedSerial = {serial}\n\
                 if ([uint64]$d.Size -ne {size} -or $d.IsBoot -or $d.IsSystem -or ($expectedSerial -ne '' -and $serial -ne $expectedSerial)) {{ Write-Output 'IDENTITY:changed'; exit 0 }}\n",
                serial = ps_quote(serial),
                size = identity.size_bytes,
            )
        }
        None => String::new(),
    };
    format!(
        "{check}$file = Join-Path ([IO.Path]::GetTempPath()) ('opcore-' + [guid]::NewGuid().ToString('N') + '.txt')\n\
         [IO.File]::WriteAllLines($file, [string[]]@({lines}))\n\
         $previous = [Console]::OutputEncoding\n\
         try {{ [Console]::OutputEncoding = [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.OEMCodePage) }} catch {{}}\n\
         $ErrorActionPreference = 'Continue'\n\
         $out = & \"$env:SystemRoot\\System32\\diskpart.exe\" /s $file 2>&1 | Out-String\n\
         $code = $LASTEXITCODE\n\
         try {{ [Console]::OutputEncoding = $previous }} catch {{}}\n\
         Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue\n\
         Write-Output $out\n\
         Write-Output ('DISKPART-EXIT:' + $code)\n",
        lines = lines.join(","),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskpartResult {
    IdentityChanged,
    Finished { exit_code: i32, output: String },
}

pub fn parse_diskpart_result(stdout: &str) -> DiskpartResult {
    if stdout.lines().any(|l| l.trim() == "IDENTITY:changed") {
        return DiskpartResult::IdentityChanged;
    }
    let exit_code = stdout
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("DISKPART-EXIT:"))
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(-1);
    let output = stdout.lines().filter(|l| !l.trim().starts_with("DISKPART-EXIT:")).collect::<Vec<_>>().join("\n");
    DiskpartResult::Finished { exit_code, output: output.trim().to_string() }
}

/// PowerShell printing `PARTITION:<n>` for the first Basic Data partition.
pub fn windows_partition_query_ps(disk: u32) -> String {
    format!(
        "Update-Disk -Number {disk} -ErrorAction SilentlyContinue\n\
         $p = @(Get-Partition -DiskNumber {disk} -ErrorAction SilentlyContinue | Where-Object {{ ([string]$_.GptType).ToLower() -eq '{basic}' }} | Sort-Object Offset)\n\
         if ($p.Count -gt 0) {{ Write-Output ('PARTITION:' + $p[0].PartitionNumber) }} else {{ Write-Output 'PARTITION:none' }}\n",
        basic = crate::safety::disks::windows::BASIC_DATA_GPT_TYPE,
    )
}

pub fn parse_partition_number(stdout: &str) -> Option<u32> {
    stdout.lines().find_map(|l| l.trim().strip_prefix("PARTITION:")).and_then(|n| n.trim().parse().ok())
}

/// PowerShell printing the drive letter and access paths of a partition as JSON.
pub fn windows_volume_query_ps(disk: u32, partition: u32) -> String {
    format!(
        "Update-Disk -Number {disk} -ErrorAction SilentlyContinue\n\
         $part = Get-Partition -DiskNumber {disk} -PartitionNumber {partition} -ErrorAction Stop\n\
         $letter = [string]$part.DriveLetter\n\
         if ($letter -notmatch '^[A-Za-z]$') {{ $letter = '' }}\n\
         [pscustomobject]@{{ Letter = $letter; Paths = @($part.AccessPaths | Where-Object {{ $_ }}) }} | ConvertTo-Json -Compress\n"
    )
}

/// Root of the new volume: `E:\`, or the `\\?\Volume{…}\` path when no
/// letter was assigned.
pub fn parse_volume_root(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    if let Some(letter) = value.get("Letter").and_then(|l| l.as_str()).and_then(|l| l.chars().next()) {
        if letter.is_ascii_alphabetic() {
            return Some(format!("{}:\\", letter.to_ascii_uppercase()));
        }
    }
    let paths: Vec<String> = match value.get("Paths") {
        Some(serde_json::Value::Array(list)) => list.iter().filter_map(|p| p.as_str().map(str::to_string)).collect(),
        Some(serde_json::Value::String(single)) => vec![single.clone()],
        _ => Vec::new(),
    };
    paths.into_iter().find(|p| p.starts_with(r"\\?\Volume{")).map(|p| if p.ends_with('\\') { p } else { format!("{p}\\") })
}

/// Inputs of the Linux flash script.
#[derive(Debug, Clone)]
pub struct LinuxFlashScript<'a> {
    pub device: &'a str,
    pub expected_size: u64,
    pub expected_serial: Option<&'a str>,
    /// The `EFI` folder to copy (its contents end up in `<volume>/EFI`).
    pub efi_dir: &'a Path,
    /// Recovery files: (source, file name), chunklist first.
    pub recovery_files: Vec<(&'a Path, &'a str)>,
    pub mount_dir: &'a Path,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub label: &'a str,
    /// Relative paths whose SHA-256 is printed after the copy.
    pub hash_files: Vec<&'a str>,
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl LinuxFlashScript<'_> {
    /// Build the POSIX sh script run as root. Every step re-checks its
    /// preconditions; nothing destructive happens before the identity check.
    /// Progress lines `STEP <phase>` are appended to `$SCRATCH_DIR/progress`.
    pub fn render(&self) -> Result<String, AppError> {
        let device = validate_linux_disk(self.device)?;
        let label = validate_fat_label(self.label)?;
        for (_, name) in &self.recovery_files {
            if !safe_file_name(name) {
                return Err(AppError::new("INVALID_PAYLOAD", format!("Unexpected recovery file name {name}")));
            }
        }
        let q = |p: &Path| sh_quote(&p.to_string_lossy());
        let serial_check = match self.expected_serial.map(|s| s.split_whitespace().collect::<String>()) {
            Some(serial) if !serial.is_empty() => format!(
                "SERIAL=$(lsblk -dno SERIAL \"$DEV\" 2>/dev/null | tr -d ' \\t')\n[ \"$SERIAL\" = {} ] || fail IDENTITY_CHANGED \"serial $SERIAL\"\n",
                sh_quote(&serial)
            ),
            _ => String::new(),
        };
        let mut sources = format!("[ -d {} ] || fail SOURCE_MISSING EFI\n", q(self.efi_dir));
        for (path, _) in &self.recovery_files {
            sources.push_str(&format!("[ -r {p} ] || fail SOURCE_MISSING {p}\n", p = q(path)));
        }
        let recovery_copy = if self.recovery_files.is_empty() {
            String::new()
        } else {
            let mut copy = String::from(
                "step copy-recovery\nmkdir -p \"$MNT/com.apple.recovery.boot\" || fail COPY_FAILED com.apple.recovery.boot\n",
            );
            for (path, name) in &self.recovery_files {
                copy.push_str(&format!(
                    "cp {src} \"$MNT/com.apple.recovery.boot/{name}\" || fail COPY_FAILED {name}\n",
                    src = q(path)
                ));
            }
            copy
        };
        let hash_lines: String = self
            .hash_files
            .iter()
            .map(|rel| {
                let rel_q = sh_quote(rel);
                format!(
                    "if [ -f \"$MNT\"/{rel_q} ]; then echo \"HASH $(sha256sum < \"$MNT\"/{rel_q} | cut -d' ' -f1) \"{rel_q}; fi\n"
                )
            })
            .collect();

        Ok(format!(
            r#"set -u
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export PATH
LC_ALL=C
export LC_ALL
DEV={device}
MNT={mnt}
PROGRESS="${{SCRATCH_DIR:-/nonexistent}}/progress"
MOUNTED=
step() {{ echo "STEP $1"; echo "STEP $1" >>"$PROGRESS" 2>/dev/null || true; }}
fail() {{ echo "FAIL $*" >&2; exit 3; }}
cleanup() {{ if [ -n "$MOUNTED" ]; then umount "$MNT" 2>/dev/null || true; fi; }}
trap cleanup EXIT
release() {{
  n=0
  while findmnt -rn -S "$1" >/dev/null 2>&1; do
    n=$((n + 1))
    [ "$n" -le 16 ] || return 1
    umount "$1" 2>/dev/null || udisksctl unmount --no-user-interaction -b "$1" >/dev/null 2>&1 || return 1
  done
  return 0
}}

step prepare
for tool in lsblk findmnt wipefs blockdev mount umount sha256sum stat find; do
  command -v "$tool" >/dev/null 2>&1 || fail TOOL_MISSING "$tool"
done
MKFS=
for tool in mkfs.vfat mkfs.fat; do
  if command -v "$tool" >/dev/null 2>&1; then MKFS=$tool; break; fi
done
[ -n "$MKFS" ] || fail TOOL_MISSING mkfs.vfat
PARTTOOL=
for tool in sgdisk sfdisk parted; do
  if command -v "$tool" >/dev/null 2>&1; then PARTTOOL=$tool; break; fi
done
[ -n "$PARTTOOL" ] || fail TOOL_MISSING sgdisk
[ -b "$DEV" ] || fail DEVICE_MISSING
[ "$(lsblk -dno TYPE "$DEV" 2>/dev/null)" = disk ] || fail NOT_A_DISK
SIZE=$(blockdev --getsize64 "$DEV") || fail DEVICE_MISSING
[ "$SIZE" = "{size}" ] || fail IDENTITY_CHANGED "size $SIZE"
{serial_check}[ "$(blockdev --getro "$DEV")" = 0 ] || fail READ_ONLY
{sources}[ -d "$MNT" ] || fail MOUNT_FAILED "$MNT"
for NODE in $(lsblk -lnpo NAME "$DEV"); do
  for TARGET in $(findmnt -rn -o TARGET -S "$NODE" 2>/dev/null); do
    case "$TARGET" in
      /|/boot|/boot/efi|/efi|/usr|/var|/home) fail SYSTEM_DISK "$NODE" ;;
    esac
  done
done
HOLDERS=$(lsblk -lnpo NAME,TYPE "$DEV" | awk '$2 != "disk" && $2 != "part" {{ print $1 }}')
[ -z "$HOLDERS" ] || fail HOLDERS_ACTIVE "$HOLDERS"
# Only now touch the user's mounts; wipefs below opens the disk with O_EXCL
# and fails if anything (ZFS, btrfs, md) still holds a partition.
for NODE in $(lsblk -lnpo NAME "$DEV"); do
  release "$NODE" || fail UNMOUNT_FAILED "$NODE"
  if grep -q "^$NODE[[:space:]]" /proc/swaps 2>/dev/null; then swapoff "$NODE" || fail SWAP_ACTIVE "$NODE"; fi
done

step partition
for NODE in $(lsblk -lnpo NAME,TYPE "$DEV" | awk '$2 == "part" {{ print $1 }}'); do
  wipefs -a "$NODE" >/dev/null 2>&1 || true
done
wipefs -a "$DEV" >/dev/null 2>&1 || fail WIPE_FAILED
case "$PARTTOOL" in
  sgdisk)
    sgdisk -Z "$DEV" >/dev/null 2>&1 || true
    sgdisk -o -n 1:0:0 -t 1:EF00 -c 1:{label} "$DEV" >/dev/null || fail PARTITION_FAILED ;;
  sfdisk)
    printf 'label: gpt\n,,U\n' | sfdisk --wipe always "$DEV" >/dev/null || fail PARTITION_FAILED ;;
  parted)
    parted -s "$DEV" mklabel gpt mkpart {label} fat32 1MiB 100% set 1 esp on >/dev/null || fail PARTITION_FAILED ;;
esac
blockdev --rereadpt "$DEV" 2>/dev/null || partprobe "$DEV" 2>/dev/null || partx -u "$DEV" 2>/dev/null || true
if command -v udevadm >/dev/null 2>&1; then udevadm settle --timeout=30 2>/dev/null || true; fi
PART=
i=0
while [ "$i" -lt 15 ]; do
  PART=$(lsblk -lnpo NAME,TYPE "$DEV" | awk '$2 == "part" {{ print $1; exit }}')
  if [ -n "$PART" ] && [ -b "$PART" ]; then break; fi
  PART=
  i=$((i + 1))
  sleep 1
done
[ -n "$PART" ] || fail PARTITION_MISSING
release "$PART" || fail UNMOUNT_FAILED "$PART"

step format
"$MKFS" -F 32 -n {label} "$PART" >/dev/null || fail FORMAT_FAILED
release "$PART" || fail UNMOUNT_FAILED "$PART"
mount -t vfat -o "uid={uid},gid={gid},umask=022,flush" "$PART" "$MNT" || fail MOUNT_FAILED
MOUNTED=1

step copy-efi
cp -R {efi} "$MNT/EFI" || fail COPY_FAILED EFI
{recovery_copy}
step verify
sync
umount "$MNT" || fail UNMOUNT_FAILED "$MNT"
MOUNTED=
# A desktop automounter may have mounted the new volume elsewhere.
release "$PART" || fail UNMOUNT_FAILED "$PART"
blockdev --flushbufs "$PART" 2>/dev/null || true
mount -t vfat -o ro "$PART" "$MNT" || fail MOUNT_FAILED
MOUNTED=1
{hash_lines}(cd "$MNT" && find . -type f -exec stat -c 'SIZE %s %n' {{}} + | sed 's#^\(SIZE [0-9]*\) \./#\1 #')
if [ -d "$MNT/com.apple.recovery.boot" ]; then
  echo "DMGCOUNT $(ls "$MNT/com.apple.recovery.boot" | grep -ci '\.dmg$')"
fi
umount "$MNT" || fail UNMOUNT_FAILED "$MNT"
MOUNTED=
release "$PART" || true
sync
step complete
"#,
            device = sh_quote(device),
            mnt = q(self.mount_dir),
            size = self.expected_size,
            serial_check = serial_check,
            sources = sources,
            label = label,
            uid = self.owner_uid,
            gid = self.owner_gid,
            efi = q(self.efi_dir),
            recovery_copy = recovery_copy,
            hash_lines = hash_lines,
        ))
    }
}

/// Map a `FAIL <CODE> [detail]` line of the Linux script to an error.
pub fn linux_script_error(stderr: &str, status: i32) -> AppError {
    let line = stderr.lines().rev().find(|l| l.starts_with("FAIL ")).unwrap_or("");
    let mut parts = line.trim_start_matches("FAIL ").splitn(2, ' ');
    let code = parts.next().unwrap_or("").trim();
    let detail = parts.next().unwrap_or("").trim();
    let (code, message, suggestion): (&str, String, Option<&str>) = match code {
        "TOOL_MISSING" => (
            "TOOL_MISSING",
            format!("The system tool \"{detail}\" is not installed"),
            Some("Install dosfstools, gdisk (or util-linux) and coreutils, then try again."),
        ),
        "DEVICE_MISSING" | "NOT_A_DISK" => ("DISK_NOT_FOUND", "The USB drive disappeared".into(), Some("Reconnect it and select it again.")),
        "IDENTITY_CHANGED" => (
            "DISK_IDENTITY_CHANGED",
            format!("The disk no longer matches the one you confirmed ({detail})"),
            Some("Select the USB drive again."),
        ),
        "READ_ONLY" => ("DISK_READ_ONLY", "The USB drive is write-protected".into(), Some("Check the lock switch on the drive or SD adapter.")),
        "SOURCE_MISSING" => ("SOURCE_MISSING", format!("A file to copy is missing: {detail}"), None),
        "SYSTEM_DISK" => ("SYSTEM_DISK", format!("{detail} holds the running system"), None),
        "UNMOUNT_FAILED" => (
            "DISK_BUSY",
            format!("{detail} could not be unmounted"),
            Some("Close any window or program that uses the USB drive and try again."),
        ),
        "SWAP_ACTIVE" => ("DISK_BUSY", format!("Swap is active on {detail}"), None),
        "HOLDERS_ACTIVE" => (
            "DISK_BUSY",
            format!("The disk is in use by {detail}"),
            Some("Close encrypted or LVM volumes on the drive first."),
        ),
        "WIPE_FAILED" | "PARTITION_FAILED" | "PARTITION_MISSING" => (
            "PARTITION_FAILED",
            "Creating the partition table failed".into(),
            Some("Reconnect the drive and try again, or try another drive."),
        ),
        "FORMAT_FAILED" => ("FORMAT_FAILED", "Formatting the USB drive as FAT32 failed".into(), None),
        "MOUNT_FAILED" => ("MOUNT_FAILED", "The new FAT32 volume could not be mounted".into(), None),
        "COPY_FAILED" => ("COPY_FAILED", format!("Copying {detail} to the USB drive failed"), Some("The drive may be full or faulty.")),
        _ => ("FLASH_FAILED", format!("The disk script stopped with exit code {status}: {}", stderr.trim()), None),
    };
    let mut error = AppError::new(code, message);
    if let Some(suggestion) = suggestion {
        error = error.with_suggestion(suggestion);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_sizes_respect_the_fat32_cap() {
        // 8 GB stick: the whole disk minus headroom.
        assert_eq!(windows_partition_sizes(8_000_000_000), vec![7_373]);
        assert_eq!(windows_partition_sizes(30_752_000_000), vec![29_071, 16_000, 8_000]);
        assert_eq!(windows_partition_sizes(64_000_000_000), vec![32_000, 16_000, 8_000]);
        assert_eq!(windows_partition_sizes(2_000_000_000_000), vec![32_000, 16_000, 8_000]);
        assert_eq!(windows_partition_sizes(32_000 * MIB + PARTITION_HEADROOM_MB * MIB)[0], 32_000);
        assert_eq!(windows_partition_sizes(40_000 * MIB)[0], 32_000);
        assert!(windows_partition_sizes(100 * MIB).is_empty());
        assert_eq!(partition_capacity(32_000), 32_000 * MIB);
    }

    #[test]
    fn diskpart_scripts() {
        let script = diskpart_flash_script(2, 32_000, "OPENCORE").unwrap();
        assert_eq!(
            script,
            "select disk 2\r\nattributes disk clear readonly noerr\r\nonline disk noerr\r\nclean\r\nconvert gpt\r\n\
             create partition primary size=32000\r\nformat fs=fat32 quick label=OPENCORE\r\nassign noerr\r\nexit\r\n"
        );
        assert!(diskpart_flash_script(2, 1000, "bad label").is_err());
        assert!(diskpart_error_is_transient("Virtual Disk Service error:\nThe device is not ready.", 0));
        assert!(diskpart_error_is_transient("", 4));
        assert!(!diskpart_error_is_transient("DiskPart succeeded", 0));
        assert!(diskpart_error_is_size("Virtual Disk Service error:\nThe volume size is too big."));
    }

    #[test]
    fn diskpart_powershell_wrapper() {
        let identity = WindowsIdentity { size_bytes: 30_752_000_000, serial: Some(" 4C53'01 ") };
        let ps = windows_diskpart_ps(2, &diskpart_flash_script(2, 32_000, "OPENCORE").unwrap(), Some(&identity));
        assert!(ps.starts_with("$d = Get-Disk -Number 2 "));
        assert!(ps.contains("$expectedSerial = '4C53''01'"));
        assert!(ps.contains("[uint64]$d.Size -ne 30752000000"));
        assert!(ps.contains(
            "'select disk 2','attributes disk clear readonly noerr','online disk noerr','clean','convert gpt',\
             'create partition primary size=32000','format fs=fat32 quick label=OPENCORE','assign noerr','exit'"
        ));
        // The identity check comes before diskpart runs.
        assert!(ps.find("IDENTITY:changed").unwrap() < ps.find("diskpart.exe").unwrap());
        let rescan = windows_diskpart_ps(2, &diskpart_rescan_script(), None);
        assert!(!rescan.contains("Get-Disk"));
        assert!(rescan.contains("@('rescan','exit')"));
    }

    #[test]
    fn windows_scripts_fit_on_the_command_line() {
        // `process::powershell` refuses encoded scripts over 30 000 characters.
        let identity = WindowsIdentity { size_bytes: u64::MAX, serial: Some("WD-WCC4N0123456789012345678901234") };
        let flash = windows_diskpart_ps(127, &diskpart_flash_script(127, 32_000, "OPENCORE").unwrap(), Some(&identity));
        for script in [
            flash.as_str(),
            crate::safety::disks::windows::INVENTORY_SCRIPT,
            &windows_partition_query_ps(127),
            &windows_volume_query_ps(127, 12),
        ] {
            assert!(crate::services::process::encode_powershell(script).len() < 30_000);
        }
    }

    #[test]
    fn diskpart_results() {
        assert_eq!(parse_diskpart_result("IDENTITY:changed\n"), DiskpartResult::IdentityChanged);
        let ok = "\nMicrosoft DiskPart version 10.0\n\nDiskPart succeeded in creating the specified partition.\n\nDISKPART-EXIT:0\n";
        match parse_diskpart_result(ok) {
            DiskpartResult::Finished { exit_code, output } => {
                assert_eq!(exit_code, 0);
                assert!(output.ends_with("specified partition."));
            }
            other => panic!("{other:?}"),
        }
        match parse_diskpart_result("Virtual Disk Service error:\nThe volume size is too big.\nDISKPART-EXIT:4") {
            DiskpartResult::Finished { exit_code, output } => {
                assert_eq!(exit_code, 4);
                assert!(diskpart_error_is_size(&output));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_diskpart_result("garbage"), DiskpartResult::Finished { exit_code: -1, .. }));
    }

    #[test]
    fn partition_and_volume_queries() {
        assert!(windows_partition_query_ps(3).contains("{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}"));
        assert_eq!(parse_partition_number("PARTITION:2\n"), Some(2));
        assert_eq!(parse_partition_number("PARTITION:none\n"), None);
        assert!(windows_volume_query_ps(3, 2).contains("-PartitionNumber 2"));
        assert_eq!(parse_volume_root(r#"{"Letter":"e","Paths":["E:\\","\\\\?\\Volume{abc}\\"]}"#).as_deref(), Some("E:\\"));
        assert_eq!(
            parse_volume_root(r#"{"Letter":"","Paths":"\\\\?\\Volume{abc}\\"}"#).as_deref(),
            Some(r"\\?\Volume{abc}\")
        );
        assert_eq!(parse_volume_root(r#"{"Letter":"","Paths":[]}"#), None);
        assert_eq!(parse_volume_root("nope"), None);
    }

    #[test]
    fn progress_ranges_are_monotonic() {
        for with_recovery in [true, false] {
            let mut last = 0.0;
            for phase in [
                FlashPhase::Prepare,
                FlashPhase::Partition,
                FlashPhase::Format,
                FlashPhase::CopyEfi,
                FlashPhase::CopyRecovery,
                FlashPhase::Verify,
                FlashPhase::Complete,
            ] {
                let start = phase.overall(0.0, with_recovery);
                let end = phase.overall(1.0, with_recovery);
                assert!(start >= last && end >= start, "{phase:?}");
                last = end;
                assert_eq!(FlashPhase::parse(phase.as_str()), Some(phase));
            }
            assert!((last - 1.0).abs() < 1e-9);
        }
        assert!((FlashPhase::CopyRecovery.overall(2.0, true) - 0.92).abs() < 1e-9);
    }

    fn script() -> String {
        LinuxFlashScript {
            device: "/dev/sdb",
            expected_size: 30_752_000_000,
            expected_serial: Some("4C53 0001"),
            efi_dir: Path::new("/home/me/.local/share/app/builds/b1/EFI"),
            recovery_files: vec![
                (Path::new("/home/me/rec's/BaseSystem.chunklist"), "BaseSystem.chunklist"),
                (Path::new("/home/me/rec's/BaseSystem.dmg"), "BaseSystem.dmg"),
            ],
            mount_dir: Path::new("/tmp/opcore-x/mnt"),
            owner_uid: 1000,
            owner_gid: 1000,
            label: "OPENCORE",
            hash_files: vec!["EFI/BOOT/BOOTx64.efi", "EFI/OC/config.plist"],
        }
        .render()
        .unwrap()
    }

    #[test]
    fn linux_script_checks_identity_before_any_write() {
        let s = script();
        let identity = s.find("fail IDENTITY_CHANGED \"size $SIZE\"").unwrap();
        let serial = s.find("[ \"$SERIAL\" = '4C530001' ]").unwrap();
        let first_wipe = s.find("wipefs -a").unwrap();
        assert!(identity < first_wipe && serial < first_wipe);
        assert!(s.find("command -v \"$tool\"").unwrap() < first_wipe);
        // Refusals (system mount, open crypt/LVM holders) come before any unmount.
        let first_release = s.find("release \"$NODE\"").unwrap();
        assert!(s.find("fail SYSTEM_DISK").unwrap() < first_release);
        assert!(s.find("fail HOLDERS_ACTIVE").unwrap() < first_release);
        assert!(first_release < first_wipe);
        assert!(s.contains("DEV='/dev/sdb'"));
        assert!(s.contains("[ \"$SIZE\" = \"30752000000\" ]"));
    }

    #[test]
    fn linux_script_partitions_formats_and_copies() {
        let s = script();
        assert!(s.contains("sgdisk -o -n 1:0:0 -t 1:EF00 -c 1:OPENCORE \"$DEV\""));
        assert!(s.contains("printf 'label: gpt\\n,,U\\n' | sfdisk --wipe always \"$DEV\""));
        assert!(s.contains("\"$MKFS\" -F 32 -n OPENCORE \"$PART\""));
        assert!(s.contains("mount -t vfat -o \"uid=1000,gid=1000,umask=022,flush\" \"$PART\" \"$MNT\""));
        assert!(s.contains("cp -R '/home/me/.local/share/app/builds/b1/EFI' \"$MNT/EFI\""));
        // Paths with quotes are shell-quoted, chunklist copied before the DMG.
        let chunklist = s.find(r"cp '/home/me/rec'\''s/BaseSystem.chunklist'").unwrap();
        let dmg = s.find(r"cp '/home/me/rec'\''s/BaseSystem.dmg'").unwrap();
        assert!(chunklist < dmg);
        assert!(s.contains("echo \"HASH $(sha256sum < \"$MNT\"/'EFI/BOOT/BOOTx64.efi' | cut -d' ' -f1) \"'EFI/BOOT/BOOTx64.efi'"));
        assert!(s.contains("mount -t vfat -o ro \"$PART\" \"$MNT\""));
        // Read-back happens on a fresh mount with every other mount released.
        let remount = s.find("mount -t vfat -o ro").unwrap();
        assert!(s[..remount].rfind("release \"$PART\"").unwrap() > s.find("step verify").unwrap());
        let steps: Vec<_> = s.lines().filter(|l| l.starts_with("step ")).collect();
        assert_eq!(steps, ["step prepare", "step partition", "step format", "step copy-efi", "step copy-recovery", "step verify", "step complete"]);
    }

    #[cfg(unix)]
    #[test]
    fn linux_script_is_valid_sh() {
        let path = std::env::temp_dir().join(format!("opcore-flash-{}.sh", uuid::Uuid::new_v4().simple()));
        std::fs::write(&path, script()).unwrap();
        let status = std::process::Command::new("/bin/sh").arg("-n").arg(&path).status().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(status.success());
    }

    #[test]
    fn linux_script_refuses_partitions_and_odd_names() {
        let mut params = LinuxFlashScript {
            device: "/dev/sdb1",
            expected_size: 1,
            expected_serial: None,
            efi_dir: Path::new("/x/EFI"),
            recovery_files: vec![],
            mount_dir: Path::new("/tmp/m"),
            owner_uid: 0,
            owner_gid: 0,
            label: "OPENCORE",
            hash_files: vec![],
        };
        assert_eq!(params.render().unwrap_err().code, "INVALID_DEVICE");
        params.device = "/dev/sdb";
        let s = params.render().unwrap();
        assert!(!s.contains("SERIAL="));
        assert!(!s.contains("step copy-recovery"));
        params.recovery_files = vec![(Path::new("/x/a.dmg"), "a b.dmg")];
        assert_eq!(params.render().unwrap_err().code, "INVALID_PAYLOAD");
    }

    #[test]
    fn linux_errors_are_mapped() {
        let err = linux_script_error("some noise\nFAIL TOOL_MISSING mkfs.vfat\n", 3);
        assert_eq!(err.code, "TOOL_MISSING");
        assert!(err.message.contains("mkfs.vfat"));
        assert_eq!(linux_script_error("FAIL IDENTITY_CHANGED size 1\n", 3).code, "DISK_IDENTITY_CHANGED");
        assert_eq!(linux_script_error("FAIL UNMOUNT_FAILED /dev/sdb1\n", 3).code, "DISK_BUSY");
        assert_eq!(linux_script_error("Segmentation fault\n", 139).code, "FLASH_FAILED");
    }
}
