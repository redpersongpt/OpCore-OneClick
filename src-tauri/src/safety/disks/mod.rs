//! Turning OS tool output (Get-Disk, lsblk, diskutil) into `DiskInfo` and
//! deciding which disks must never be offered as a flash target.
//!
//! The parsers are plain functions over captured output so they are tested
//! on every host; the `platform::*::disk` modules only run the tools.

pub mod linux;
pub mod macos;
pub mod windows;

use std::path::{Component, Path};

use crate::contracts::DiskInfo;

/// Why a disk is held back. `system` reasons set `is_system_disk`.
#[derive(Debug, Default, Clone)]
pub struct Verdict {
    pub system: Vec<String>,
    pub blocked: Vec<String>,
}

impl Verdict {
    pub fn system(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.system.contains(&reason) {
            self.system.push(reason);
        }
    }

    pub fn block(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.blocked.contains(&reason) {
            self.blocked.push(reason);
        }
    }

    pub fn apply(self, disk: &mut DiskInfo) {
        disk.is_system_disk = !self.system.is_empty();
        let reasons: Vec<String> = self.system.into_iter().chain(self.blocked).collect();
        disk.blocked_reason = if reasons.is_empty() { None } else { Some(reasons.join("; ")) };
    }
}

/// A disk the user must not pick (system disk or otherwise blocked).
pub fn is_blocked(disk: &DiskInfo) -> bool {
    disk.is_system_disk || disk.blocked_reason.is_some()
}

/// Decimal units, like the capacity printed on the drive ("31.9 GB").
pub fn size_display(bytes: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000_000, "TB"),
        (1_000_000_000, "GB"),
        (1_000_000, "MB"),
        (1_000, "KB"),
    ];
    for (factor, unit) in UNITS {
        if bytes >= factor {
            return format!("{:.1} {unit}", bytes as f64 / factor as f64);
        }
    }
    format!("{bytes} B")
}

/// "SanDisk Ultra · 32.0 GB · /dev/sdb"
pub fn disk_display(disk: &DiskInfo) -> String {
    let name = match (&disk.vendor, &disk.model) {
        (Some(v), Some(m)) if !m.to_lowercase().starts_with(&v.to_lowercase()) => format!("{v} {m}"),
        (_, Some(m)) => m.clone(),
        (Some(v), None) => v.clone(),
        (None, None) => "Unnamed disk".to_string(),
    };
    format!("{name} · {} · {}", disk.size_display, disk.device_path)
}

/// Selectable disks first, then by device path (numbers compared numerically).
pub fn sort_disks(disks: &mut [DiskInfo]) {
    disks.sort_by(|a, b| {
        is_blocked(a)
            .cmp(&is_blocked(b))
            .then_with(|| natural_key(&a.device_path).cmp(&natural_key(&b.device_path)))
    });
}

fn natural_key(path: &str) -> (String, u64) {
    let digits: String = path.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    let prefix = path[..path.len() - digits.len()].to_lowercase();
    (prefix, digits.parse().unwrap_or(0))
}

/// Component-wise "is `path` inside `mount`" (so `/media/usb2` is not inside
/// `/media/usb`). Windows drive letters compare case-insensitively.
pub fn path_is_under(path: &Path, mount: &str) -> bool {
    let mount = mount.trim();
    if mount.is_empty() {
        return false;
    }
    let normalize = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().to_lowercase()),
                Component::RootDir => Some("/".to_string()),
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                Component::CurDir | Component::ParentDir => None,
            })
            .collect()
    };
    let path_parts = normalize(&strip_verbatim(path));
    let mount_parts = normalize(&strip_verbatim(Path::new(mount)));
    !mount_parts.is_empty() && path_parts.len() >= mount_parts.len() && path_parts[..mount_parts.len()] == mount_parts[..]
}

/// `\\?\C:\x` → `C:\x` so verbatim paths from `canonicalize` compare equal.
fn strip_verbatim(path: &Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.len() >= 2 && rest.as_bytes()[1] == b':' => std::path::PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// The drive letter of a Windows path (`C:\…`, `\\?\C:\…`), upper-cased.
pub fn drive_letter(path: &Path) -> Option<char> {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut chars = text.chars();
    let letter = chars.next()?;
    (letter.is_ascii_alphabetic() && chars.next() == Some(':')).then(|| letter.to_ascii_uppercase())
}

/// True when `path` lives on one of the disk's mounted partitions.
pub fn path_on_disk(disk: &DiskInfo, path: &Path) -> bool {
    disk.partitions
        .iter()
        .filter_map(|p| p.mount_point.as_deref())
        .any(|mount| path_is_under(path, mount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::PartitionInfo;

    fn disk(path: &str, blocked: bool) -> DiskInfo {
        DiskInfo {
            device_path: path.into(),
            model: Some("Ultra".into()),
            vendor: Some("SanDisk".into()),
            serial_number: None,
            size_bytes: 32_000_000_000,
            size_display: size_display(32_000_000_000),
            transport: Some("usb".into()),
            removable: true,
            partition_table: None,
            partitions: vec![PartitionInfo {
                number: 1,
                label: None,
                filesystem: Some("vfat".into()),
                size_bytes: 1,
                mount_point: Some("/media/user/USB".into()),
            }],
            is_system_disk: false,
            blocked_reason: blocked.then(|| "x".to_string()),
        }
    }

    #[test]
    fn sizes_use_decimal_units() {
        assert_eq!(size_display(31_914_983_424), "31.9 GB");
        assert_eq!(size_display(2_000_398_934_016), "2.0 TB");
        assert_eq!(size_display(512_000_000), "512.0 MB");
        assert_eq!(size_display(999), "999 B");
    }

    #[test]
    fn sorting_puts_selectable_first_and_numbers_naturally() {
        let mut disks = vec![disk(r"\\.\PhysicalDrive10", false), disk(r"\\.\PhysicalDrive0", true), disk(r"\\.\PhysicalDrive2", false)];
        sort_disks(&mut disks);
        let order: Vec<_> = disks.iter().map(|d| d.device_path.as_str()).collect();
        assert_eq!(order, [r"\\.\PhysicalDrive2", r"\\.\PhysicalDrive10", r"\\.\PhysicalDrive0"]);
    }

    #[test]
    fn verdict_sets_flags() {
        let mut d = disk("/dev/sdb", false);
        let mut v = Verdict::default();
        v.block("Write-protected");
        v.apply(&mut d);
        assert!(!d.is_system_disk);
        assert_eq!(d.blocked_reason.as_deref(), Some("Write-protected"));

        let mut v = Verdict::default();
        v.system("Holds /");
        v.system("Holds /");
        v.block("Write-protected");
        v.apply(&mut d);
        assert!(d.is_system_disk);
        assert_eq!(d.blocked_reason.as_deref(), Some("Holds /; Write-protected"));
    }

    #[test]
    fn path_containment_is_component_wise() {
        assert!(path_is_under(Path::new("/media/user/USB/app"), "/media/user/USB"));
        assert!(path_is_under(Path::new("/media/user/USB"), "/media/user/USB"));
        assert!(!path_is_under(Path::new("/media/user/USB2/app"), "/media/user/USB"));
        assert!(path_is_under(Path::new("/anything"), "/"));
        assert!(!path_is_under(Path::new("/anything"), ""));
        let d = disk("/dev/sdb", false);
        assert!(path_on_disk(&d, Path::new("/media/user/USB/builds/x")));
        assert!(!path_on_disk(&d, Path::new("/home/user/builds/x")));
    }

    #[test]
    fn drive_letters() {
        assert_eq!(drive_letter(Path::new(r"C:\Users\me\app.exe")), Some('C'));
        assert_eq!(drive_letter(Path::new(r"\\?\e:\portable\app.exe")), Some('E'));
        assert_eq!(drive_letter(Path::new("/usr/bin/app")), None);
    }

    #[test]
    fn display_name() {
        let d = disk("/dev/sdb", false);
        assert_eq!(disk_display(&d), "SanDisk Ultra · 32.0 GB · /dev/sdb");
    }
}
