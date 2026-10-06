//! Strict validation of whole-disk device paths before they reach a
//! destructive tool. Partitions, loop devices, device-mapper nodes and
//! anything with shell metacharacters are rejected.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::error::AppError;

static LINUX_DISK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^/dev/(sd[a-z]{1,3}|nvme[0-9]{1,3}n[0-9]{1,3}|mmcblk[0-9]{1,3})$").expect("static regex"));
static MACOS_DISK: Lazy<Regex> = Lazy::new(|| Regex::new(r"^/dev/disk([0-9]{1,3})$").expect("static regex"));
static WINDOWS_DISK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\\\\\.\\PhysicalDrive([0-9]{1,3})$").expect("static regex"));

fn invalid(device: &str, expected: &str) -> AppError {
    AppError::new("INVALID_DEVICE", format!("\"{device}\" is not a whole-disk device path ({expected})"))
}

/// `/dev/sdX`, `/dev/nvmeXnY` or `/dev/mmcblkN` — never a partition.
pub fn validate_linux_disk(device: &str) -> Result<&str, AppError> {
    if LINUX_DISK.is_match(device) {
        Ok(device)
    } else {
        Err(invalid(device, "/dev/sdX, /dev/nvmeXnY or /dev/mmcblkN"))
    }
}

/// `/dev/diskN` (whole disk, not `diskNsM`). Returns N.
pub fn validate_macos_disk(device: &str) -> Result<u32, AppError> {
    MACOS_DISK
        .captures(device)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .ok_or_else(|| invalid(device, "/dev/diskN"))
}

/// `\\.\PhysicalDriveN`. Returns the disk number N.
pub fn validate_windows_disk(device: &str) -> Result<u32, AppError> {
    WINDOWS_DISK
        .captures(device)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .ok_or_else(|| invalid(device, r"\\.\PhysicalDriveN"))
}

pub fn windows_device_path(number: u32) -> String {
    format!(r"\\.\PhysicalDrive{number}")
}

/// Validate a device path for the OS the app is running on.
pub fn validate_host_disk(device: &str) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    return validate_windows_disk(device).map(|_| ());
    #[cfg(target_os = "linux")]
    return validate_linux_disk(device).map(|_| ());
    #[cfg(target_os = "macos")]
    return validate_macos_disk(device).map(|_| ());
    #[allow(unreachable_code)]
    Err(invalid(device, "unsupported platform"))
}

/// A FAT32 volume label: 1–11 characters, upper-case letters, digits, `_`.
pub fn validate_fat_label(label: &str) -> Result<&str, AppError> {
    let ok = !label.is_empty()
        && label.len() <= 11
        && label.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(label)
    } else {
        Err(AppError::new("INVALID_LABEL", format!("\"{label}\" is not a valid FAT32 volume label")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_accepts_whole_disks_only() {
        for ok in ["/dev/sda", "/dev/sdb", "/dev/sdaa", "/dev/nvme0n1", "/dev/nvme12n3", "/dev/mmcblk0"] {
            assert!(validate_linux_disk(ok).is_ok(), "{ok}");
        }
        for bad in [
            "/dev/sdb1",
            "/dev/nvme0n1p1",
            "/dev/mmcblk0p1",
            "/dev/mmcblk0boot0",
            "/dev/loop0",
            "/dev/dm-0",
            "/dev/mapper/root",
            "/dev/sr0",
            "/dev/vda",
            "/dev/sdb; rm -rf /",
            "/dev/sdb\n",
            "dev/sdb",
            "/dev/../dev/sdb",
            "/dev/SDB",
            "",
        ] {
            assert!(validate_linux_disk(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn macos_accepts_whole_disks_only() {
        assert_eq!(validate_macos_disk("/dev/disk4").unwrap(), 4);
        assert_eq!(validate_macos_disk("/dev/disk12").unwrap(), 12);
        for bad in ["/dev/disk4s1", "/dev/rdisk4", "disk4", "/dev/disk", "/dev/disk4 ", "/dev/disk-1"] {
            assert!(validate_macos_disk(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn windows_parses_disk_number() {
        assert_eq!(validate_windows_disk(r"\\.\PhysicalDrive2").unwrap(), 2);
        assert_eq!(validate_windows_disk(r"\\.\physicaldrive10").unwrap(), 10);
        for bad in [r"\\.\PhysicalDrive", r"\\.\C:", r"C:\", r"\\.\PhysicalDrive1 & del", r"\\?\PhysicalDrive1", "PhysicalDrive1"] {
            assert!(validate_windows_disk(bad).is_err(), "{bad:?}");
        }
        assert_eq!(windows_device_path(3), r"\\.\PhysicalDrive3");
    }

    #[test]
    fn fat_labels() {
        assert!(validate_fat_label("OPENCORE").is_ok());
        assert!(validate_fat_label("OC_1").is_ok());
        for bad in ["", "opencore", "OPENCORE1234", "OPEN CORE", "OC'", "OC\""] {
            assert!(validate_fat_label(bad).is_err(), "{bad:?}");
        }
    }
}
