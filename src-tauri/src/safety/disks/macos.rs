//! macOS: parse `diskutil list -plist` / `diskutil info -plist` and keep the
//! startup disk (including APFS containers on external drives) protected.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{path_is_under, size_display, Verdict};
use crate::contracts::{DiskInfo, PartitionInfo};
use crate::error::AppError;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct DiskList {
    #[serde(rename = "AllDisksAndPartitions")]
    pub entries: Vec<ListEntry>,
    #[serde(rename = "WholeDisks")]
    pub whole_disks: Vec<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ListEntry {
    #[serde(rename = "DeviceIdentifier")]
    pub device_identifier: String,
    #[serde(rename = "Content")]
    pub content: Option<String>,
    #[serde(rename = "Partitions")]
    pub partitions: Vec<ListPartition>,
    #[serde(rename = "APFSPhysicalStores")]
    pub physical_stores: Vec<StoreRef>,
    #[serde(rename = "APFSVolumes")]
    pub apfs_volumes: Vec<ApfsVolume>,
    #[serde(rename = "MountPoint")]
    pub mount_point: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ListPartition {
    #[serde(rename = "DeviceIdentifier")]
    pub device_identifier: String,
    #[serde(rename = "Size")]
    pub size: u64,
    #[serde(rename = "Content")]
    pub content: Option<String>,
    #[serde(rename = "VolumeName")]
    pub volume_name: Option<String>,
    #[serde(rename = "MountPoint")]
    pub mount_point: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct StoreRef {
    #[serde(rename = "DeviceIdentifier")]
    pub device_identifier: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ApfsVolume {
    #[serde(rename = "MountPoint")]
    pub mount_point: Option<String>,
    #[serde(rename = "MountedSnapshots")]
    pub snapshots: Vec<MountedSnapshot>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct MountedSnapshot {
    #[serde(rename = "SnapshotMountPoint")]
    pub mount_point: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct DiskInfoPlist {
    #[serde(rename = "DeviceIdentifier")]
    pub device_identifier: String,
    #[serde(rename = "DeviceNode")]
    pub device_node: Option<String>,
    #[serde(rename = "Internal")]
    pub internal: Option<bool>,
    #[serde(rename = "RemovableMedia")]
    pub removable_media: Option<bool>,
    #[serde(rename = "Removable")]
    pub removable: Option<bool>,
    #[serde(rename = "BusProtocol")]
    pub bus_protocol: Option<String>,
    #[serde(rename = "MediaName")]
    pub media_name: Option<String>,
    #[serde(rename = "IORegistryEntryName")]
    pub io_registry_name: Option<String>,
    #[serde(rename = "TotalSize")]
    pub total_size: Option<u64>,
    #[serde(rename = "Size")]
    pub size: Option<u64>,
    #[serde(rename = "WritableMedia")]
    pub writable_media: Option<bool>,
    #[serde(rename = "Content")]
    pub content: Option<String>,
    #[serde(rename = "VirtualOrPhysical")]
    pub virtual_or_physical: Option<String>,
    #[serde(rename = "MountPoint")]
    pub mount_point: Option<String>,
    #[serde(rename = "ParentWholeDisk")]
    pub parent_whole_disk: Option<String>,
    #[serde(rename = "APFSPhysicalStores")]
    pub physical_stores: Vec<PhysicalStore>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct PhysicalStore {
    #[serde(rename = "APFSPhysicalStore")]
    pub store: String,
}

#[derive(Debug, Default, Clone)]
pub struct MacHost {
    /// Whole disks that back the running system ("disk0").
    pub boot_disks: Vec<String>,
    /// Paths whose disk must be protected (the app bundle, app data).
    pub guarded_paths: Vec<PathBuf>,
}

fn parse_plist<T: for<'de> Deserialize<'de>>(bytes: &[u8], what: &str) -> Result<T, AppError> {
    plist::from_bytes(bytes).map_err(|e| AppError::new("DISK_LIST_PARSE", format!("Unexpected {what} output: {e}")))
}

pub fn parse_list(bytes: &[u8]) -> Result<DiskList, AppError> {
    parse_plist(bytes, "diskutil list")
}

pub fn parse_info(bytes: &[u8]) -> Result<DiskInfoPlist, AppError> {
    parse_plist(bytes, "diskutil info")
}

/// "disk4s2" → "disk4"; "disk4" → "disk4".
pub fn whole_disk_of(identifier: &str) -> String {
    let rest = identifier.strip_prefix("disk").unwrap_or(identifier);
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    format!("disk{digits}")
}

/// Whole-disk identifiers from `diskutil list -plist external physical`.
pub fn whole_disks(list: &DiskList) -> Vec<String> {
    if !list.whole_disks.is_empty() {
        return list.whole_disks.clone();
    }
    list.entries.iter().map(|e| e.device_identifier.clone()).filter(|id| whole_disk_of(id) == *id).collect()
}

/// Mount points per physical whole disk, following APFS containers back to
/// their physical stores. Input: `diskutil list -plist` (all disks).
pub fn mounts_by_disk(all: &DiskList) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for entry in &all.entries {
        let mut mounts: Vec<String> = entry.partitions.iter().filter_map(|p| p.mount_point.clone()).collect();
        mounts.extend(entry.mount_point.clone());
        for volume in &entry.apfs_volumes {
            mounts.extend(volume.mount_point.clone());
            mounts.extend(volume.snapshots.iter().filter_map(|s| s.mount_point.clone()));
        }
        mounts.retain(|m| !m.is_empty());
        let owners: Vec<String> = if entry.physical_stores.is_empty() {
            vec![whole_disk_of(&entry.device_identifier)]
        } else {
            entry.physical_stores.iter().map(|s| whole_disk_of(&s.device_identifier)).collect()
        };
        for owner in owners {
            map.entry(owner).or_default().extend(mounts.iter().cloned());
        }
    }
    map
}

/// Physical whole disks behind `diskutil info -plist /`.
pub fn boot_disks(root: &DiskInfoPlist) -> Vec<String> {
    let mut disks: Vec<String> = root.physical_stores.iter().map(|s| whole_disk_of(&s.store)).collect();
    if disks.is_empty() {
        if let Some(parent) = &root.parent_whole_disk {
            disks.push(whole_disk_of(parent));
        }
    }
    disks
}

fn transport(bus: Option<&str>) -> Option<String> {
    let bus = bus?.trim().to_lowercase();
    Some(match bus.as_str() {
        "secure digital" => "sd".to_string(),
        _ => bus,
    })
}

fn partition_table(content: Option<&str>) -> Option<String> {
    match content? {
        "GUID_partition_scheme" => Some("gpt".into()),
        "FDisk_partition_scheme" => Some("mbr".into()),
        "Apple_partition_scheme" => Some("apm".into()),
        _ => None,
    }
}

fn filesystem(content: Option<&str>) -> Option<String> {
    let content = content?;
    Some(match content {
        "EFI" => "efi".to_string(),
        "Microsoft Basic Data" | "DOS_FAT_32" | "DOS_FAT_16" | "Windows_FAT_32" => "fat/exfat/ntfs".to_string(),
        "Apple_HFS" => "hfs".to_string(),
        "Apple_APFS" => "apfs".to_string(),
        "Linux" | "Linux Filesystem" => "linux".to_string(),
        other => other.to_lowercase(),
    })
}

/// Build the `DiskInfo` for one external whole disk.
pub fn build_disk(info: &DiskInfoPlist, partitions: &[ListPartition], mounts: &[String], host: &MacHost) -> Option<DiskInfo> {
    let id = if info.device_identifier.is_empty() { return None } else { info.device_identifier.clone() };
    if info.virtual_or_physical.as_deref() == Some("Virtual") {
        return None;
    }
    let size = info.total_size.or(info.size).unwrap_or(0);
    if size == 0 {
        return None;
    }
    let mut verdict = Verdict::default();
    if host.boot_disks.contains(&id) {
        verdict.system("macOS is running from this disk");
    }
    for mount in mounts {
        if mount == "/" || path_is_under(Path::new(mount), "/System/Volumes") {
            verdict.system(format!("Holds the running system ({mount})"));
        }
    }
    if host.guarded_paths.iter().any(|p| mounts.iter().any(|m| m != "/" && path_is_under(p, m))) {
        verdict.system("OpCore-OneClick runs from or stores its data on this disk");
    }
    if info.internal.unwrap_or(false) {
        verdict.block("Internal disk");
    }
    if info.writable_media == Some(false) {
        verdict.block("The disk is write-protected");
    }

    let model = info
        .media_name
        .clone()
        .filter(|m| !m.trim().is_empty())
        .or_else(|| info.io_registry_name.as_ref().map(|n| n.trim_end_matches(" Media").to_string()))
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let mut disk = DiskInfo {
        device_path: info.device_node.clone().unwrap_or_else(|| format!("/dev/{id}")),
        model,
        vendor: None,
        serial_number: None,
        size_bytes: size,
        size_display: size_display(size),
        transport: transport(info.bus_protocol.as_deref()),
        removable: info.removable_media.or(info.removable).unwrap_or(false),
        partition_table: partition_table(info.content.as_deref()),
        partitions: partitions
            .iter()
            .map(|p| PartitionInfo {
                number: p
                    .device_identifier
                    .rsplit('s')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0),
                label: p.volume_name.clone().filter(|v| !v.is_empty()),
                filesystem: filesystem(p.content.as_deref()),
                size_bytes: p.size,
                mount_point: p.mount_point.clone().filter(|m| !m.is_empty()),
            })
            .collect(),
        is_system_disk: false,
        blocked_reason: None,
    };
    verdict.apply(&mut disk);
    Some(disk)
}

/// FAT volume partition created by `diskutil eraseDisk FAT32 … GPT`: s2
/// behind a 200 MB EFI partition, or s1 on disks too small for one.
pub fn fat_partition(list: &DiskList, whole: &str) -> Option<String> {
    list.entries
        .iter()
        .filter(|e| e.device_identifier == whole)
        .flat_map(|e| e.partitions.iter())
        .find(|p| p.content.as_deref().is_some_and(|c| c == "Microsoft Basic Data" || c.starts_with("DOS_FAT")))
        .map(|p| p.device_identifier.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTERNAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>AllDisks</key><array><string>disk4</string><string>disk4s1</string><string>disk4s2</string><string>disk6</string><string>disk6s1</string><string>disk6s2</string></array>
<key>AllDisksAndPartitions</key><array>
 <dict><key>Content</key><string>GUID_partition_scheme</string><key>DeviceIdentifier</key><string>disk4</string><key>OSInternal</key><false/>
  <key>Partitions</key><array>
   <dict><key>Content</key><string>EFI</string><key>DeviceIdentifier</key><string>disk4s1</string><key>Size</key><integer>209715200</integer><key>VolumeName</key><string>EFI</string></dict>
   <dict><key>Content</key><string>Microsoft Basic Data</string><key>DeviceIdentifier</key><string>disk4s2</string><key>MountPoint</key><string>/Volumes/STICK</string><key>Size</key><integer>30541000000</integer><key>VolumeName</key><string>STICK</string></dict>
  </array><key>Size</key><integer>30752000000</integer></dict>
 <dict><key>Content</key><string>GUID_partition_scheme</string><key>DeviceIdentifier</key><string>disk6</string><key>OSInternal</key><false/>
  <key>Partitions</key><array>
   <dict><key>Content</key><string>EFI</string><key>DeviceIdentifier</key><string>disk6s1</string><key>Size</key><integer>209715200</integer></dict>
   <dict><key>Content</key><string>Apple_APFS</string><key>DeviceIdentifier</key><string>disk6s2</string><key>Size</key><integer>499000000000</integer></dict>
  </array><key>Size</key><integer>500107862016</integer></dict>
</array>
<key>VolumesFromDisks</key><array><string>STICK</string></array>
<key>WholeDisks</key><array><string>disk4</string><string>disk6</string></array>
</dict></plist>"#;

    const ALL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>AllDisksAndPartitions</key><array>
 <dict><key>Content</key><string>GUID_partition_scheme</string><key>DeviceIdentifier</key><string>disk0</string>
  <key>Partitions</key><array><dict><key>Content</key><string>Apple_APFS</string><key>DeviceIdentifier</key><string>disk0s2</string><key>Size</key><integer>494384795648</integer></dict></array></dict>
 <dict><key>APFSPhysicalStores</key><array><dict><key>DeviceIdentifier</key><string>disk0s2</string></dict></array>
  <key>APFSVolumes</key><array>
   <dict><key>DeviceIdentifier</key><string>disk3s1</string><key>MountPoint</key><string>/System/Volumes/Data</string></dict>
   <dict><key>DeviceIdentifier</key><string>disk3s3</string><key>MountPoint</key><string>/System/Volumes/Update/mnt1</string>
    <key>MountedSnapshots</key><array><dict><key>SnapshotMountPoint</key><string>/</string></dict></array></dict>
  </array><key>Content</key><string>Apple_APFS_Container</string><key>DeviceIdentifier</key><string>disk3</string><key>Partitions</key><array/></dict>
 <dict><key>Content</key><string>GUID_partition_scheme</string><key>DeviceIdentifier</key><string>disk4</string>
  <key>Partitions</key><array><dict><key>Content</key><string>Microsoft Basic Data</string><key>DeviceIdentifier</key><string>disk4s2</string><key>MountPoint</key><string>/Volumes/STICK</string><key>Size</key><integer>1</integer></dict></array></dict>
 <dict><key>APFSPhysicalStores</key><array><dict><key>DeviceIdentifier</key><string>disk6s2</string></dict></array>
  <key>APFSVolumes</key><array><dict><key>DeviceIdentifier</key><string>disk7s1</string><key>MountPoint</key><string>/Volumes/Work</string></dict></array>
  <key>Content</key><string>Apple_APFS_Container</string><key>DeviceIdentifier</key><string>disk7</string><key>Partitions</key><array/></dict>
</array></dict></plist>"#;

    fn info(id: &str, name: &str, size: u64, internal: bool) -> DiskInfoPlist {
        DiskInfoPlist {
            device_identifier: id.into(),
            device_node: Some(format!("/dev/{id}")),
            internal: Some(internal),
            removable_media: Some(true),
            bus_protocol: Some("USB".into()),
            media_name: Some(name.into()),
            total_size: Some(size),
            writable_media: Some(true),
            content: Some("GUID_partition_scheme".into()),
            virtual_or_physical: Some("Physical".into()),
            ..Default::default()
        }
    }

    #[test]
    fn parses_external_list_and_mount_map() {
        let list = parse_list(EXTERNAL.as_bytes()).unwrap();
        assert_eq!(whole_disks(&list), ["disk4", "disk6"]);
        let all = parse_list(ALL.as_bytes()).unwrap();
        let mounts = mounts_by_disk(&all);
        assert!(mounts["disk0"].contains(&"/".to_string()));
        assert!(mounts["disk0"].contains(&"/System/Volumes/Data".to_string()));
        assert_eq!(mounts["disk4"], ["/Volumes/STICK"]);
        assert_eq!(mounts["disk6"], ["/Volumes/Work"]);
        assert_eq!(fat_partition(&list, "disk4").as_deref(), Some("disk4s2"));
        assert_eq!(fat_partition(&list, "disk6"), None);
    }

    #[test]
    fn fat_partition_without_an_efi_partition() {
        // `diskutil eraseDisk FAT32 OPENCORE GPT` on a 512 MB disk (macOS 27):
        // no EFI partition, the volume is s1.
        let small = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>AllDisks</key><array><string>disk6</string><string>disk6s1</string></array>
<key>AllDisksAndPartitions</key><array><dict><key>Content</key><string>GUID_partition_scheme</string><key>DeviceIdentifier</key><string>disk6</string><key>OSInternal</key><false/>
<key>Partitions</key><array><dict><key>Content</key><string>Microsoft Basic Data</string><key>DeviceIdentifier</key><string>disk6s1</string>
<key>DiskUUID</key><string>79491F6F-73A3-4B46-9471-5B98453C547B</string><key>MountPoint</key><string>/Volumes/OPENCORE</string>
<key>Size</key><integer>534773760</integer><key>VolumeName</key><string>OPENCORE</string></dict></array>
<key>Size</key><integer>536870912</integer></dict></array><key>WholeDisks</key><array><string>disk6</string></array></dict></plist>"#;
        let list = parse_list(small.as_bytes()).unwrap();
        assert_eq!(fat_partition(&list, "disk6").as_deref(), Some("disk6s1"));
        assert_eq!(fat_partition(&list, "disk4"), None);
    }

    #[test]
    fn usb_stick_is_selectable() {
        let list = parse_list(EXTERNAL.as_bytes()).unwrap();
        let host = MacHost { boot_disks: vec!["disk0".into()], guarded_paths: vec![PathBuf::from("/Applications/OpCore-OneClick.app")] };
        let disk = build_disk(&info("disk4", "SanDisk Ultra", 30_752_000_000, false), &list.entries[0].partitions, &["/Volumes/STICK".into()], &host).unwrap();
        assert_eq!(disk.device_path, "/dev/disk4");
        assert_eq!(disk.transport.as_deref(), Some("usb"));
        assert_eq!(disk.partition_table.as_deref(), Some("gpt"));
        assert_eq!(disk.partitions.len(), 2);
        assert_eq!(disk.partitions[1].number, 2);
        assert_eq!(disk.partitions[1].mount_point.as_deref(), Some("/Volumes/STICK"));
        assert!(!disk.is_system_disk);
        assert!(disk.blocked_reason.is_none());
    }

    #[test]
    fn external_startup_disk_is_system() {
        let root = DiskInfoPlist {
            physical_stores: vec![PhysicalStore { store: "disk6s2".into() }],
            parent_whole_disk: Some("disk7".into()),
            ..Default::default()
        };
        let host = MacHost { boot_disks: boot_disks(&root), guarded_paths: vec![] };
        assert_eq!(host.boot_disks, ["disk6"]);
        let disk = build_disk(&info("disk6", "Samsung T7", 500_107_862_016, false), &[], &[], &host).unwrap();
        assert!(disk.is_system_disk);
    }

    #[test]
    fn app_volume_internal_and_read_only() {
        let host = MacHost { boot_disks: vec![], guarded_paths: vec![PathBuf::from("/Volumes/Work/OpCore-OneClick.app/Contents/MacOS/app")] };
        let disk = build_disk(&info("disk6", "T7", 500_107_862_016, false), &[], &["/Volumes/Work".into()], &host).unwrap();
        assert!(disk.is_system_disk);

        let disk = build_disk(&info("disk2", "SD", 1_000_000_000, true), &[], &[], &MacHost::default()).unwrap();
        assert!(!disk.is_system_disk);
        assert_eq!(disk.blocked_reason.as_deref(), Some("Internal disk"));

        let mut ro = info("disk5", "Locked SD", 1_000_000_000, false);
        ro.writable_media = Some(false);
        ro.bus_protocol = Some("Secure Digital".into());
        let disk = build_disk(&ro, &[], &[], &MacHost::default()).unwrap();
        assert_eq!(disk.transport.as_deref(), Some("sd"));
        assert!(disk.blocked_reason.as_deref().unwrap().contains("write-protected"));
    }

    #[test]
    fn helpers() {
        assert_eq!(whole_disk_of("disk4s2"), "disk4");
        assert_eq!(whole_disk_of("disk12"), "disk12");
        assert_eq!(whole_disk_of("disk3s3s1"), "disk3");
        let mut virtual_disk = info("disk9", "Disk Image", 1_000, false);
        virtual_disk.virtual_or_physical = Some("Virtual".into());
        assert!(build_disk(&virtual_disk, &[], &[], &MacHost::default()).is_none());
        assert!(parse_info(b"garbage").is_err());
    }
}
