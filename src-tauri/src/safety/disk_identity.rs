//! Disk identity fingerprinting and collision detection.
//!
//! The fingerprint taken when the user confirms a flash must still match
//! the disk right before it is erased; a field that disappears counts as a
//! change, not as "unknown".

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::contracts::DiskInfo;
use crate::error::AppError;

/// A fingerprint capturing the stable identity fields of a disk device.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiskIdentityFingerprint {
    pub serial_number: Option<String>,
    pub device_path: Option<String>,
    pub vendor: Option<String>,
    pub transport: Option<String>,
    pub partition_table: Option<String>,
    pub size_bytes: Option<u64>,
    pub model: Option<String>,
    pub removable: Option<bool>,
}

/// Field-level confidence for a fingerprint comparison.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldConfidence {
    /// The serial number, or path + model + size, matched.
    Strong,
    Weak,
}

/// Result of comparing two disk identity fingerprints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintComparison {
    pub matches: bool,
    pub confidence: FieldConfidence,
    pub matched_fields: Vec<String>,
    pub mismatched_fields: Vec<String>,
    /// Absent on both sides.
    pub missing_fields: Vec<String>,
}

/// Trim + lowercase; empty becomes None.
fn normalize(value: Option<&str>) -> Option<String> {
    value.map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty())
}

/// Build a fingerprint from a DiskInfo, normalizing all string fields.
pub fn build_fingerprint(info: &DiskInfo) -> DiskIdentityFingerprint {
    DiskIdentityFingerprint {
        serial_number: normalize(info.serial_number.as_deref()),
        device_path: normalize(Some(&info.device_path)),
        vendor: normalize(info.vendor.as_deref()),
        transport: normalize(info.transport.as_deref()),
        partition_table: normalize(info.partition_table.as_deref()),
        size_bytes: (info.size_bytes > 0).then_some(info.size_bytes),
        model: normalize(info.model.as_deref()),
        removable: Some(info.removable),
    }
}

fn compare_field<T: PartialEq>(
    name: &str,
    expected: &Option<T>,
    actual: &Option<T>,
    matched: &mut Vec<String>,
    mismatched: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    match (expected, actual) {
        (Some(e), Some(a)) if e == a => matched.push(name.to_string()),
        (None, None) => missing.push(name.to_string()),
        // Different, appeared or disappeared: the disk is not provably the same.
        _ => mismatched.push(name.to_string()),
    }
}

/// Compare two fingerprints field by field.
pub fn compare_fingerprints(expected: &DiskIdentityFingerprint, actual: &DiskIdentityFingerprint) -> FingerprintComparison {
    let mut matched = Vec::new();
    let mut mismatched = Vec::new();
    let mut missing = Vec::new();
    let (m, x, s) = (&mut matched, &mut mismatched, &mut missing);
    compare_field("serial_number", &expected.serial_number, &actual.serial_number, m, x, s);
    compare_field("device_path", &expected.device_path, &actual.device_path, m, x, s);
    compare_field("vendor", &expected.vendor, &actual.vendor, m, x, s);
    compare_field("transport", &expected.transport, &actual.transport, m, x, s);
    compare_field("partition_table", &expected.partition_table, &actual.partition_table, m, x, s);
    compare_field("model", &expected.model, &actual.model, m, x, s);
    compare_field("size_bytes", &expected.size_bytes, &actual.size_bytes, m, x, s);
    compare_field("removable", &expected.removable, &actual.removable, m, x, s);

    let has = |name: &str| matched.iter().any(|f| f == name);
    let confidence = if has("serial_number") || (has("device_path") && has("model") && has("size_bytes")) {
        FieldConfidence::Strong
    } else {
        FieldConfidence::Weak
    };
    let comparison = FingerprintComparison {
        matches: mismatched.is_empty(),
        confidence,
        matched_fields: matched,
        mismatched_fields: mismatched,
        missing_fields: missing,
    };
    info!(
        matches = comparison.matches,
        mismatched = ?comparison.mismatched_fields,
        confidence = ?comparison.confidence,
        "Disk fingerprint comparison"
    );
    comparison
}

/// The disk the user confirmed for the flash that is running (one at a time).
static CONFIRMED: Mutex<Option<(String, DiskIdentityFingerprint)>> = Mutex::new(None);

/// Keeps the confirmed target registered; dropping it ends the flash.
pub struct ConfirmedTarget(());

impl Drop for ConfirmedTarget {
    fn drop(&mut self) {
        let mut slot = CONFIRMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = None;
    }
}

/// Register the confirmed disk before handing the flash to the platform code,
/// which re-lists the disks itself and checks them with [`ensure_confirmed`].
pub fn confirm_target(device: &str, fingerprint: DiskIdentityFingerprint) -> ConfirmedTarget {
    let mut slot = CONFIRMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = Some((device.trim().to_lowercase(), fingerprint));
    ConfirmedTarget(())
}

/// The disk about to be erased must be the one the user confirmed; without
/// a confirmation nothing may be erased.
pub fn ensure_confirmed(disk: &DiskInfo) -> Result<(), AppError> {
    let slot = CONFIRMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some((device, expected)) = slot.as_ref() else {
        return Err(AppError::new("FLASH_NOT_CONFIRMED", "The flash was not confirmed"));
    };
    if *device != disk.device_path.trim().to_lowercase() {
        return Err(AppError::new("CONFIRMATION_DEVICE_MISMATCH", "The confirmation was issued for a different disk"));
    }
    let comparison = compare_fingerprints(expected, &build_fingerprint(disk));
    if !comparison.matches {
        return Err(AppError::new(
            "DISK_IDENTITY_CHANGED",
            format!(
                "The disk at {} is not the one you confirmed (changed: {})",
                disk.device_path,
                comparison.mismatched_fields.join(", ")
            ),
        )
        .with_suggestion("Re-select the USB drive and confirm again."));
    }
    Ok(())
}

/// Other connected disks that cannot be told apart from the target: same
/// serial number, or (both without serial) same vendor, model and size.
pub fn find_collisions(target: &DiskIdentityFingerprint, all_devices: &[DiskInfo], target_device_path: &str) -> Vec<String> {
    let target_path = target_device_path.trim().to_lowercase();
    let mut collisions = Vec::new();
    for device in all_devices {
        if device.device_path.trim().to_lowercase() == target_path {
            continue;
        }
        let other = build_fingerprint(device);
        let ambiguous = match (&target.serial_number, &other.serial_number) {
            (Some(a), Some(b)) => a == b,
            (None, None) => {
                target.model == other.model && target.vendor == other.vendor && target.size_bytes == other.size_bytes
            }
            _ => false,
        };
        if ambiguous {
            warn!(device = %device.device_path, "Disk cannot be told apart from the flash target");
            collisions.push(device.device_path.clone());
        }
    }
    collisions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(path: &str, serial: Option<&str>, size: u64) -> DiskInfo {
        DiskInfo {
            device_path: path.into(),
            model: Some("Ultra USB 3.0".into()),
            vendor: Some("SanDisk".into()),
            serial_number: serial.map(String::from),
            size_bytes: size,
            size_display: String::new(),
            transport: Some("usb".into()),
            removable: true,
            partition_table: Some("gpt".into()),
            partitions: vec![],
            is_system_disk: false,
            blocked_reason: None,
        }
    }

    #[test]
    fn identical_disks_match_strongly() {
        let a = build_fingerprint(&disk("/dev/sdb", Some(" 4C53 "), 1000));
        let b = build_fingerprint(&disk("/dev/sdb", Some("4c53"), 1000));
        let cmp = compare_fingerprints(&a, &b);
        assert!(cmp.matches);
        assert_eq!(cmp.confidence, FieldConfidence::Strong);
        assert!(cmp.mismatched_fields.is_empty());
    }

    #[test]
    fn a_vanished_serial_is_a_mismatch() {
        let a = build_fingerprint(&disk("/dev/sdb", Some("4C53"), 1000));
        let b = build_fingerprint(&disk("/dev/sdb", None, 1000));
        let cmp = compare_fingerprints(&a, &b);
        assert!(!cmp.matches);
        assert_eq!(cmp.mismatched_fields, ["serial_number"]);
    }

    #[test]
    fn size_or_path_change_is_a_mismatch() {
        let a = build_fingerprint(&disk("/dev/sdb", Some("1"), 1000));
        assert!(!compare_fingerprints(&a, &build_fingerprint(&disk("/dev/sdb", Some("1"), 2000))).matches);
        assert!(!compare_fingerprints(&a, &build_fingerprint(&disk("/dev/sdc", Some("1"), 1000))).matches);
    }

    #[test]
    fn missing_on_both_sides_is_not_a_mismatch() {
        let a = build_fingerprint(&disk("/dev/sdb", None, 1000));
        let cmp = compare_fingerprints(&a, &a.clone());
        assert!(cmp.matches);
        assert!(cmp.missing_fields.contains(&"serial_number".to_string()));
        assert_eq!(cmp.confidence, FieldConfidence::Strong);
    }

    #[test]
    fn platform_check_needs_the_confirmed_disk() {
        // One test owns the process-wide slot, so nothing races on it.
        let stick = disk("/dev/sdb", Some("4C53"), 1000);
        assert_eq!(ensure_confirmed(&stick).unwrap_err().code, "FLASH_NOT_CONFIRMED");
        {
            let _target = confirm_target("/dev/SDB", build_fingerprint(&stick));
            ensure_confirmed(&stick).unwrap();
            // Same path, another stick plugged in since the confirmation.
            let swapped = disk("/dev/sdb", Some("9999"), 1000);
            assert_eq!(ensure_confirmed(&swapped).unwrap_err().code, "DISK_IDENTITY_CHANGED");
            let other = disk("/dev/sdc", Some("4C53"), 1000);
            assert_eq!(ensure_confirmed(&other).unwrap_err().code, "CONFIRMATION_DEVICE_MISMATCH");
        }
        // The confirmation ends with the flash.
        assert_eq!(ensure_confirmed(&stick).unwrap_err().code, "FLASH_NOT_CONFIRMED");
    }

    #[test]
    fn collisions_ignore_the_path() {
        let target_disk = disk("/dev/sdb", None, 1000);
        let target = build_fingerprint(&target_disk);
        let others = vec![
            target_disk.clone(),
            disk("/dev/sdc", None, 1000),        // same model/size, no serial: ambiguous
            disk("/dev/sdd", None, 2000),        // different size
            disk("/dev/sde", Some("ABC"), 1000), // has a serial
        ];
        assert_eq!(find_collisions(&target, &others, "/dev/sdb"), ["/dev/sdc"]);

        let target = build_fingerprint(&disk("/dev/sdb", Some("ABC"), 1000));
        let others = vec![disk("/dev/sdc", Some("abc"), 1000), disk("/dev/sdd", Some("XYZ"), 1000)];
        assert_eq!(find_collisions(&target, &others, "/dev/sdb"), ["/dev/sdc"]);
    }
}
