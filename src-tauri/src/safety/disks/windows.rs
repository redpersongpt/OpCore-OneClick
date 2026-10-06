//! Windows: parse the JSON inventory printed by [`INVENTORY_SCRIPT`]
//! (Get-Disk + Get-Partition + Get-Volume, page files, BitLocker).

use serde::Deserialize;
use serde_json::Value;

use super::{size_display, Verdict};
use crate::contracts::{DiskInfo, PartitionInfo};
use crate::error::AppError;
use crate::safety::device_path::windows_device_path;

/// One PowerShell run that returns every disk with nested partitions and
/// volumes plus the host facts needed to recognise the system disk.
pub const INVENTORY_SCRIPT: &str = r#"
try { Remove-TypeData -TypeName System.Array -ErrorAction Stop } catch {}
$sys = ([string]$env:SystemDrive).TrimEnd(':').ToUpper()
$pageDrives = @()
try { $pageDrives = @(Get-CimInstance -ClassName Win32_PageFileUsage -ErrorAction Stop | ForEach-Object { ([string]$_.Name).Substring(0,1).ToUpper() }) } catch {}
$protected = @()
try {
  $protected = @(Get-CimInstance -Namespace 'root/cimv2/Security/MicrosoftVolumeEncryption' -ClassName Win32_EncryptableVolume -ErrorAction Stop |
    Where-Object { $_.ProtectionStatus -ne 0 -or $_.ConversionStatus -ne 0 } |
    ForEach-Object { ([string]$_.DriveLetter).TrimEnd(':').ToUpper() } | Where-Object { $_ -ne '' })
} catch {}
$media = @{}
try { foreach ($dd in @(Get-CimInstance -ClassName Win32_DiskDrive -ErrorAction Stop)) { $media[[string]$dd.Index] = [string]$dd.MediaType } } catch {}
$result = @(foreach ($d in @(Get-Disk)) {
  $parts = @()
  try { $parts = @(Get-Partition -DiskNumber $d.Number -ErrorAction Stop) } catch {}
  [pscustomobject]@{
    Number = [int]$d.Number
    UniqueId = [string]$d.UniqueId
    FriendlyName = [string]$d.FriendlyName
    Manufacturer = [string]$d.Manufacturer
    Model = [string]$d.Model
    SerialNumber = [string]$d.SerialNumber
    Size = [uint64]$d.Size
    BusType = [string]$d.BusType
    PartitionStyle = [string]$d.PartitionStyle
    IsBoot = [bool]$d.IsBoot
    IsSystem = [bool]$d.IsSystem
    IsOffline = [bool]$d.IsOffline
    IsReadOnly = [bool]$d.IsReadOnly
    OperationalStatus = [string](@($d.OperationalStatus) -join ',')
    MediaType = [string]$media[[string]$d.Number]
    Partitions = @(foreach ($p in $parts) {
      $vol = $null
      try { $vol = Get-Volume -Partition $p -ErrorAction Stop } catch {}
      $letter = [string]$p.DriveLetter
      if ($letter -notmatch '^[A-Za-z]$') { $letter = '' }
      [pscustomobject]@{
        Number = [int]$p.PartitionNumber
        DriveLetter = $letter
        Size = [uint64]$p.Size
        GptType = [string]$p.GptType
        IsBoot = [bool]$p.IsBoot
        IsSystem = [bool]$p.IsSystem
        FileSystem = $(if ($vol) { [string]$vol.FileSystem } else { '' })
        Label = $(if ($vol) { [string]$vol.FileSystemLabel } else { '' })
      }
    })
  }
})
[pscustomobject]@{ SystemDrive = $sys; PageFileDrives = @($pageDrives); ProtectedDrives = @($protected); Disks = $result } | ConvertTo-Json -Depth 6 -Compress
"#;

/// Facts about the running process that PowerShell cannot know.
#[derive(Debug, Default, Clone)]
pub struct WindowsHost {
    /// Drive letters the app itself runs from or keeps data on.
    pub app_drives: Vec<char>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Inventory {
    #[serde(default)]
    system_drive: Option<String>,
    #[serde(default)]
    page_file_drives: Value,
    #[serde(default)]
    protected_drives: Value,
    #[serde(default)]
    disks: Value,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct RawDisk {
    number: Option<i64>,
    friendly_name: Option<String>,
    manufacturer: Option<String>,
    model: Option<String>,
    serial_number: Option<String>,
    size: Option<u64>,
    bus_type: Value,
    partition_style: Value,
    is_boot: Option<bool>,
    is_system: Option<bool>,
    is_read_only: Option<bool>,
    operational_status: Option<String>,
    media_type: Option<String>,
    partitions: Value,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct RawPartition {
    number: Option<u32>,
    drive_letter: Option<String>,
    size: Option<u64>,
    gpt_type: Option<String>,
    is_boot: Option<bool>,
    is_system: Option<bool>,
    file_system: Option<String>,
    label: Option<String>,
}

/// ConvertTo-Json writes a one-element array as a bare object, and Windows
/// PowerShell 5.1 sometimes wraps arrays as `{"value": [...], "Count": n}`;
/// accept all three.
fn items<T: for<'de> Deserialize<'de>>(value: &Value) -> Vec<T> {
    match value {
        Value::Array(list) => list.iter().filter(|v| !v.is_null()).filter_map(|v| serde_json::from_value(v.clone()).ok()).collect(),
        Value::Null => Vec::new(),
        Value::Object(map) if map.len() == 2 && map.contains_key("Count") => {
            map.get("value").map(items).unwrap_or_default()
        }
        other => serde_json::from_value(other.clone()).map(|v| vec![v]).unwrap_or_default(),
    }
}

/// Number of entries `items` would see, parsed or not.
fn item_count(value: &Value) -> usize {
    match value {
        Value::Array(list) => list.iter().filter(|v| !v.is_null()).count(),
        Value::Null => 0,
        Value::Object(map) if map.len() == 2 && map.contains_key("Count") => map.get("value").map(item_count).unwrap_or(0),
        _ => 1,
    }
}

fn letters(value: &Value) -> Vec<char> {
    items::<String>(value).iter().filter_map(|s| s.trim().chars().next()).map(|c| c.to_ascii_uppercase()).collect()
}

fn clean(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// MSFT_Disk.BusType as a name, whether PowerShell printed the name or the code.
fn bus_name(value: &Value) -> String {
    let code = match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    };
    if let Some(code) = code {
        return match code {
            1 => "scsi",
            2 => "atapi",
            3 => "ata",
            4 => "1394",
            5 => "ssa",
            6 => "fibre channel",
            7 => "usb",
            8 => "raid",
            9 => "iscsi",
            10 => "sas",
            11 => "sata",
            12 => "sd",
            13 => "mmc",
            14 => "virtual",
            15 => "file backed virtual",
            16 => "storage spaces",
            17 => "nvme",
            _ => "unknown",
        }
        .to_string();
    }
    value.as_str().map(|s| s.trim().to_lowercase()).unwrap_or_else(|| "unknown".to_string())
}

fn partition_style(value: &Value) -> Option<String> {
    let text = match value {
        Value::Number(n) => match n.as_u64() {
            Some(1) => "mbr".to_string(),
            Some(2) => "gpt".to_string(),
            _ => return None,
        },
        Value::String(s) => s.trim().to_lowercase(),
        _ => return None,
    };
    match text.as_str() {
        "gpt" | "2" => Some("gpt".into()),
        "mbr" | "1" => Some("mbr".into()),
        _ => None,
    }
}

const EXTERNAL_MEDIA: [&str; 2] = ["removable media", "external hard disk media"];

/// Parse the inventory JSON. Only disks on an external bus are returned;
/// system disks among them are flagged with the reason.
pub fn parse_inventory(json: &str, host: &WindowsHost) -> Result<Vec<DiskInfo>, AppError> {
    let inventory: Inventory = serde_json::from_str(json.trim())
        .map_err(|e| AppError::new("DISK_LIST_PARSE", format!("Unexpected Get-Disk output: {e}")))?;
    let system_drive = inventory
        .system_drive
        .as_deref()
        .and_then(|s| s.trim().chars().next())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('C');
    let page_drives = letters(&inventory.page_file_drives);
    let protected = letters(&inventory.protected_drives);

    let mut disks = Vec::new();
    for raw in items::<RawDisk>(&inventory.disks) {
        // A missing number must never fall back to disk 0.
        let Some(number) = raw.number.and_then(|n| u32::try_from(n).ok()) else { continue };
        let size = raw.size.unwrap_or(0);
        let status = raw.operational_status.clone().unwrap_or_default().to_lowercase();
        if size == 0 || status.contains("no media") {
            continue;
        }
        let bus = bus_name(&raw.bus_type);
        let media = raw.media_type.clone().unwrap_or_default().trim().to_lowercase();
        let external = matches!(bus.as_str(), "usb" | "sd" | "mmc")
            || (bus == "scsi" && EXTERNAL_MEDIA.contains(&media.as_str()));
        if !external {
            continue;
        }

        let parts = items::<RawPartition>(&raw.partitions);
        let mut verdict = Verdict::default();
        // A partition we cannot read could be the system or a page-file drive.
        if parts.len() != item_count(&raw.partitions) {
            verdict.block("Some partitions of this disk could not be read");
        }
        if raw.is_boot.unwrap_or(false) {
            verdict.system("Windows is running from this disk");
        }
        if raw.is_system.unwrap_or(false) {
            verdict.system("Holds the EFI system partition this PC boots from");
        }
        for part in &parts {
            let letter = part.drive_letter.as_deref().and_then(|l| l.trim().chars().next()).map(|c| c.to_ascii_uppercase());
            if part.is_boot.unwrap_or(false) {
                verdict.system("Windows is running from this disk");
            }
            if part.is_system.unwrap_or(false) {
                verdict.system("Holds the EFI system partition this PC boots from");
            }
            let Some(letter) = letter else { continue };
            if letter == system_drive {
                verdict.system(format!("Holds the Windows system drive ({letter}:)"));
            }
            if page_drives.contains(&letter) {
                verdict.system(format!("Holds a page file ({letter}:)"));
            }
            if host.app_drives.contains(&letter) {
                verdict.system(format!("OpCore-OneClick runs from or stores its data on this disk ({letter}:)"));
            }
            if protected.contains(&letter) {
                verdict.block(format!("Contains a BitLocker-encrypted volume ({letter}:); decrypt or format it in Windows first"));
            }
        }
        if raw.is_read_only.unwrap_or(false) {
            verdict.block("The disk is write-protected");
        }

        let removable = media == "removable media" || matches!(bus.as_str(), "sd" | "mmc");
        let model = clean(&raw.friendly_name).or_else(|| clean(&raw.model));
        let mut disk = DiskInfo {
            device_path: windows_device_path(number),
            model,
            vendor: clean(&raw.manufacturer),
            serial_number: clean(&raw.serial_number),
            size_bytes: size,
            size_display: size_display(size),
            transport: Some(bus),
            removable,
            partition_table: partition_style(&raw.partition_style),
            partitions: parts
                .iter()
                .map(|p| PartitionInfo {
                    number: p.number.unwrap_or(0),
                    label: clean(&p.label),
                    filesystem: clean(&p.file_system).map(|f| f.to_lowercase()),
                    size_bytes: p.size.unwrap_or(0),
                    mount_point: p
                        .drive_letter
                        .as_deref()
                        .and_then(|l| l.trim().chars().next())
                        .filter(|c| c.is_ascii_alphabetic())
                        .map(|c| format!("{}:\\", c.to_ascii_uppercase())),
                })
                .collect(),
            is_system_disk: false,
            blocked_reason: None,
        };
        verdict.apply(&mut disk);
        disks.push(disk);
    }
    Ok(disks)
}

/// GPT type of a Basic Data partition (what `create partition primary` makes).
pub const BASIC_DATA_GPT_TYPE: &str = "{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}";

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"SystemDrive":"C","PageFileDrives":"C","ProtectedDrives":["F"],"Disks":[
      {"Number":0,"UniqueId":"eui.0025","FriendlyName":"Samsung SSD 980 PRO 1TB","Manufacturer":"","Model":"Samsung SSD 980 PRO 1TB","SerialNumber":"S5GXNX0R","Size":1000204886016,"BusType":"NVMe","PartitionStyle":"GPT","IsBoot":true,"IsSystem":true,"IsOffline":false,"IsReadOnly":false,"OperationalStatus":"Online","MediaType":"Fixed hard disk media",
       "Partitions":[{"Number":1,"DriveLetter":"","Size":104857600,"GptType":"{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}","IsBoot":false,"IsSystem":true,"FileSystem":"FAT32","Label":""},
                     {"Number":3,"DriveLetter":"C","Size":999000000000,"GptType":"{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}","IsBoot":true,"IsSystem":false,"FileSystem":"NTFS","Label":"Windows"}]},
      {"Number":2,"UniqueId":"USBSTOR\\DISK&VEN_SANDISK","FriendlyName":"SanDisk Ultra USB 3.0","Manufacturer":"SanDisk","Model":"Ultra USB 3.0","SerialNumber":"4C530001231115117453  ","Size":30752000000,"BusType":"USB","PartitionStyle":"MBR","IsBoot":false,"IsSystem":false,"IsOffline":false,"IsReadOnly":false,"OperationalStatus":"Online","MediaType":"Removable Media",
       "Partitions":{"Number":1,"DriveLetter":"E","Size":30750000000,"GptType":"","IsBoot":false,"IsSystem":false,"FileSystem":"exFAT","Label":"STICK"}},
      {"Number":3,"FriendlyName":"WD Elements","Manufacturer":"WD","Model":"Elements 25A3","SerialNumber":"","Size":2000365289472,"BusType":7,"PartitionStyle":2,"IsBoot":false,"IsSystem":false,"IsReadOnly":false,"OperationalStatus":"Online","MediaType":"External hard disk media",
       "Partitions":[{"Number":1,"DriveLetter":"F","Size":2000000000000,"FileSystem":"NTFS","Label":"Backup"}]},
      {"Number":4,"FriendlyName":"Generic SD Reader","BusType":"SD","Size":0,"OperationalStatus":"No Media","Partitions":[]},
      {"Number":5,"FriendlyName":"Kingston DataTraveler","Manufacturer":"Kingston","BusType":"USB","PartitionStyle":"RAW","Size":15500000000,"IsBoot":false,"IsSystem":false,"IsReadOnly":true,"MediaType":"Removable Media","Partitions":[]},
      {"Number":6,"FriendlyName":"Windows To Go","BusType":"USB","PartitionStyle":"GPT","Size":64000000000,"IsBoot":false,"IsSystem":false,"MediaType":"External hard disk media",
       "Partitions":[{"Number":2,"DriveLetter":"G","Size":63000000000,"FileSystem":"NTFS","Label":"Portable"}]},
      {"Number":7,"FriendlyName":"Msft Virtual Disk","BusType":"SCSI","Size":127000000000,"MediaType":"Fixed hard disk media","Partitions":[]},
      {"FriendlyName":"Broken entry without number","BusType":"USB","Size":8000000000}
    ]}"#;

    fn parse() -> Vec<DiskInfo> {
        parse_inventory(FIXTURE, &WindowsHost { app_drives: vec!['G'] }).unwrap()
    }

    #[test]
    fn only_external_disks_with_media_are_listed() {
        let disks = parse();
        let paths: Vec<_> = disks.iter().map(|d| d.device_path.as_str()).collect();
        assert_eq!(paths, [r"\\.\PhysicalDrive2", r"\\.\PhysicalDrive3", r"\\.\PhysicalDrive5", r"\\.\PhysicalDrive6"]);
    }

    #[test]
    fn usb_stick_fields() {
        let disks = parse();
        let stick = &disks[0];
        assert_eq!(stick.model.as_deref(), Some("SanDisk Ultra USB 3.0"));
        assert_eq!(stick.vendor.as_deref(), Some("SanDisk"));
        assert_eq!(stick.serial_number.as_deref(), Some("4C530001231115117453"));
        assert_eq!(stick.transport.as_deref(), Some("usb"));
        assert!(stick.removable);
        assert_eq!(stick.partition_table.as_deref(), Some("mbr"));
        assert_eq!(stick.size_display, "30.8 GB");
        assert_eq!(stick.partitions.len(), 1);
        assert_eq!(stick.partitions[0].mount_point.as_deref(), Some("E:\\"));
        assert_eq!(stick.partitions[0].filesystem.as_deref(), Some("exfat"));
        assert!(!stick.is_system_disk);
        assert!(stick.blocked_reason.is_none());
    }

    #[test]
    fn numeric_bus_and_style_codes_are_understood() {
        let disks = parse();
        let hdd = &disks[1];
        assert_eq!(hdd.transport.as_deref(), Some("usb"));
        assert_eq!(hdd.partition_table.as_deref(), Some("gpt"));
        assert!(!hdd.removable);
        assert!(hdd.serial_number.is_none());
    }

    #[test]
    fn bitlocker_and_read_only_disks_are_blocked_but_not_system() {
        let disks = parse();
        let hdd = &disks[1];
        assert!(!hdd.is_system_disk);
        assert!(hdd.blocked_reason.as_deref().unwrap().contains("BitLocker"));
        let ro = &disks[2];
        assert!(!ro.is_system_disk);
        assert!(ro.blocked_reason.as_deref().unwrap().contains("write-protected"));
        assert!(ro.partition_table.is_none());
    }

    #[test]
    fn disk_hosting_the_app_is_a_system_disk() {
        let disks = parse();
        let wtg = &disks[3];
        assert!(wtg.is_system_disk);
        assert!(wtg.blocked_reason.as_deref().unwrap().contains("(G:)"));
    }

    #[test]
    fn boot_disk_on_usb_is_flagged() {
        let json = r#"{"SystemDrive":"C","PageFileDrives":[],"ProtectedDrives":[],"Disks":[
          {"Number":1,"FriendlyName":"USB SSD","BusType":"USB","Size":256000000000,"IsBoot":true,"IsSystem":false,"MediaType":"External hard disk media",
           "Partitions":[{"Number":1,"DriveLetter":"","Size":100000000,"IsSystem":true},{"Number":2,"DriveLetter":"C","Size":255000000000,"IsBoot":true}]},
          {"Number":2,"FriendlyName":"Pagefile stick","BusType":"USB","Size":16000000000,"MediaType":"Removable Media",
           "Partitions":[{"Number":1,"DriveLetter":"P","Size":16000000000}]}]}"#;
        let json = json.replace(r#""PageFileDrives":[]"#, r#""PageFileDrives":["C","P"]"#);
        let disks = parse_inventory(&json, &WindowsHost::default()).unwrap();
        assert!(disks[0].is_system_disk);
        let reason = disks[0].blocked_reason.clone().unwrap();
        assert!(reason.contains("Windows is running"));
        assert!(reason.contains("EFI system partition"));
        assert!(reason.contains("system drive (C:)"));
        assert!(disks[1].is_system_disk);
        assert!(disks[1].blocked_reason.as_deref().unwrap().contains("page file (P:)"));
    }

    #[test]
    fn single_disk_object_and_empty_inventory() {
        let single = r#"{"SystemDrive":"C","Disks":{"Number":9,"FriendlyName":"Stick","BusType":"USB","Size":8000000000,"MediaType":"Removable Media","Partitions":null}}"#;
        let disks = parse_inventory(single, &WindowsHost::default()).unwrap();
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].device_path, r"\\.\PhysicalDrive9");
        assert!(disks[0].partitions.is_empty());
        let empty = r#"{"SystemDrive":"C","PageFileDrives":[],"ProtectedDrives":[],"Disks":[]}"#;
        assert!(parse_inventory(empty, &WindowsHost::default()).unwrap().is_empty());
        assert!(parse_inventory("not json", &WindowsHost::default()).is_err());
    }

    #[test]
    fn unreadable_partitions_block_the_disk() {
        let json = r#"{"SystemDrive":"C","Disks":[{"Number":4,"FriendlyName":"Stick","BusType":"USB","Size":16000000000,"MediaType":"Removable Media",
           "Partitions":[{"Number":1,"DriveLetter":"E","Size":8000000000},{"Number":"two","DriveLetter":"C","Size":8000000000}]}]}"#;
        let disks = parse_inventory(json, &WindowsHost::default()).unwrap();
        assert_eq!(disks[0].partitions.len(), 1);
        assert!(disks[0].blocked_reason.as_deref().unwrap().contains("could not be read"));
    }

    #[test]
    fn wrapped_arrays_from_windows_powershell_are_unwrapped() {
        let json = r#"{"SystemDrive":"C","PageFileDrives":{"value":["P"],"Count":1},"ProtectedDrives":[],"Disks":{"value":[
          {"Number":4,"FriendlyName":"Stick","BusType":"USB","Size":16000000000,"MediaType":"Removable Media",
           "Partitions":{"value":[{"Number":1,"DriveLetter":"P","Size":16000000000}],"Count":1}}],"Count":1}}"#;
        let disks = parse_inventory(json, &WindowsHost::default()).unwrap();
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].partitions.len(), 1);
        assert!(disks[0].is_system_disk);
        assert!(disks[0].blocked_reason.as_deref().unwrap().contains("page file (P:)"));
    }
}
