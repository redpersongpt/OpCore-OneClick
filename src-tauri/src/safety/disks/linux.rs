//! Linux: parse `lsblk -J -b -O` and decide which disks hold the running
//! system (root, /boot, /boot/efi, swap, live media, the app itself).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{path_is_under, size_display, Verdict};
use crate::contracts::{DiskInfo, PartitionInfo};
use crate::error::AppError;
use crate::safety::device_path::validate_linux_disk;

/// Columns used when `-O` is not supported (util-linux < 2.37 has no MOUNTPOINTS).
pub const LSBLK_FALLBACK_COLUMNS: &str =
    "NAME,KNAME,PATH,TYPE,TRAN,RM,HOTPLUG,RO,SIZE,MODEL,VENDOR,SERIAL,PTTYPE,FSTYPE,LABEL,PARTLABEL,MOUNTPOINT";

/// Mount points whose disk must never be erased.
const SYSTEM_MOUNTS: [&str; 12] = [
    "/", "/boot", "/boot/efi", "/efi", "/usr", "/var", "/home", "/opt", "/srv", "/nix", "/gnu/store", "/snap",
];

/// Where live systems keep their boot medium.
const LIVE_MEDIA_MOUNTS: [&str; 7] = [
    "/cdrom",
    "/run/initramfs/live",
    "/run/initramfs/isoscan",
    "/run/archiso/bootmnt",
    "/lib/live/mount/medium",
    "/run/live/medium",
    "/isodevice",
];

#[derive(Debug, Default, Clone)]
pub struct LinuxHost {
    /// Contents of /proc/swaps.
    pub swaps: String,
    /// Paths whose disk must be protected (the app binary, $APPIMAGE, app data).
    pub guarded_paths: Vec<PathBuf>,
    /// /sys/block/<mmcblkN>/device/type: "SD" or "MMC" (eMMC).
    pub mmc_types: HashMap<String, String>,
}

fn str_field(node: &Value, key: &str) -> Option<String> {
    node.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// lsblk prints booleans as true/false, 0/1 or "0"/"1" depending on version.
fn bool_field(node: &Value, key: &str) -> Option<bool> {
    match node.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_u64().map(|n| n != 0),
        Value::String(s) => match s.trim() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn u64_field(node: &Value, key: &str) -> Option<u64> {
    match node.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn children(node: &Value) -> &[Value] {
    node.get("children").and_then(|c| c.as_array()).map(Vec::as_slice).unwrap_or(&[])
}

fn node_path(node: &Value) -> Option<String> {
    str_field(node, "path").or_else(|| str_field(node, "name").map(|n| if n.starts_with('/') { n } else { format!("/dev/{n}") }))
}

fn kname(node: &Value) -> Option<String> {
    str_field(node, "kname").or_else(|| str_field(node, "name")).map(|n| n.rsplit('/').next().unwrap_or(&n).to_string())
}

fn mountpoints(node: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(list) = node.get("mountpoints").and_then(|v| v.as_array()) {
        out.extend(list.iter().filter_map(|v| v.as_str()).map(str::to_string));
    }
    if let Some(single) = str_field(node, "mountpoint") {
        if !out.contains(&single) {
            out.push(single);
        }
    }
    out
}

/// Every node of a subtree (the disk, its partitions, crypt/LVM holders).
fn walk<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    out.push(node);
    for child in children(node) {
        walk(child, out);
    }
}

struct Subtree<'a> {
    nodes: Vec<&'a Value>,
    mounts: Vec<String>,
}

fn subtree(disk: &Value) -> Subtree<'_> {
    let mut nodes = Vec::new();
    walk(disk, &mut nodes);
    let mounts = nodes.iter().flat_map(|n| mountpoints(n)).collect();
    Subtree { nodes, mounts }
}

/// Swap devices and swap files listed in /proc/swaps.
fn parse_swaps(swaps: &str) -> (Vec<String>, Vec<PathBuf>) {
    let mut devices = Vec::new();
    let mut files = Vec::new();
    for line in swaps.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(kind)) = (fields.next(), fields.next()) else { continue };
        // /proc/swaps escapes spaces as \040.
        let name = name.replace("\\040", " ");
        if kind == "partition" {
            devices.push(name);
        } else {
            files.push(PathBuf::from(name));
        }
    }
    (devices, files)
}

/// Index of the disk whose mounted filesystem contains `path` (longest mount wins).
fn owner_of(path: &Path, trees: &[Subtree<'_>]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (index, tree) in trees.iter().enumerate() {
        for mount in &tree.mounts {
            if mount.starts_with('[') || !path_is_under(path, mount) {
                continue;
            }
            let depth = Path::new(mount).components().count();
            let deeper = match best {
                None => true,
                Some((_, best_depth)) => depth > best_depth,
            };
            if deeper {
                best = Some((index, depth));
            }
        }
    }
    best.map(|(index, _)| index)
}

fn is_external(disk: &Value, kname: &str, host: &LinuxHost) -> bool {
    let tran = str_field(disk, "tran").map(|t| t.to_lowercase());
    if kname.starts_with("mmcblk") {
        // Only SD cards; eMMC (type "MMC") is the soldered system storage of many laptops.
        return host.mmc_types.get(kname).is_some_and(|t| t.trim().eq_ignore_ascii_case("SD"));
    }
    match tran.as_deref() {
        Some("usb") => true,
        Some("nvme" | "sata" | "ata" | "sas" | "pcie" | "virtio" | "fc" | "iscsi") => false,
        _ => bool_field(disk, "rm").unwrap_or(false) || bool_field(disk, "hotplug").unwrap_or(false),
    }
}

fn partition_number(node: &Value) -> u32 {
    if let Some(n) = u64_field(node, "partn") {
        return u32::try_from(n).unwrap_or(0);
    }
    let name = kname(node).unwrap_or_default();
    let digits: String = name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().unwrap_or(0)
}

/// Parse `lsblk -J -b -O` (or the fallback columns). Only external disks are
/// returned; those holding the running system are flagged.
pub fn parse_lsblk(json: &str, host: &LinuxHost) -> Result<Vec<DiskInfo>, AppError> {
    let root: Value = serde_json::from_str(json.trim())
        .map_err(|e| AppError::new("DISK_LIST_PARSE", format!("Unexpected lsblk output: {e}")))?;
    let devices = root.get("blockdevices").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let disks: Vec<&Value> = devices.iter().filter(|d| str_field(d, "type").as_deref() == Some("disk")).collect();
    let trees: Vec<Subtree<'_>> = disks.iter().map(|d| subtree(d)).collect();

    let (swap_devices, swap_files) = parse_swaps(&host.swaps);
    let mut flagged: HashMap<usize, Vec<String>> = HashMap::new();
    for file in &swap_files {
        if let Some(index) = owner_of(file, &trees) {
            flagged.entry(index).or_default().push(format!("Holds an active swap file ({})", file.display()));
        }
    }
    for path in &host.guarded_paths {
        if let Some(index) = owner_of(path, &trees) {
            flagged.entry(index).or_default().push("OpCore-OneClick runs from or stores its data on this disk".to_string());
        }
    }

    let mut result = Vec::new();
    for (index, disk) in disks.iter().enumerate() {
        let Some(path) = node_path(disk) else { continue };
        let Some(disk_kname) = kname(disk) else { continue };
        if validate_linux_disk(&path).is_err() || !is_external(disk, &disk_kname, host) {
            continue;
        }
        let size = u64_field(disk, "size").unwrap_or(0);
        if size == 0 {
            continue;
        }
        let tree = &trees[index];
        let mut verdict = Verdict::default();
        for mount in &tree.mounts {
            if mount == "[SWAP]" {
                verdict.system("Holds an active swap partition");
            } else if SYSTEM_MOUNTS.contains(&mount.as_str()) {
                verdict.system(format!("Holds the running system ({mount})"));
            } else if LIVE_MEDIA_MOUNTS.iter().any(|live| path_is_under(Path::new(mount), live)) {
                verdict.system(format!("Holds the live system the PC was started from ({mount})"));
            }
        }
        for node in &tree.nodes {
            let node_dev = node_path(node).unwrap_or_default();
            let node_k = kname(node).map(|k| format!("/dev/{k}")).unwrap_or_default();
            if swap_devices.iter().any(|s| *s == node_dev || *s == node_k) {
                verdict.system("Holds an active swap partition");
            }
            let kind = str_field(node, "type").unwrap_or_default();
            if !matches!(kind.as_str(), "disk" | "part") {
                verdict.block(format!("In use by {kind} ({node_dev}); close it before erasing the disk"));
            }
        }
        for reason in flagged.remove(&index).unwrap_or_default() {
            verdict.system(reason);
        }
        if bool_field(disk, "ro").unwrap_or(false) {
            verdict.block("The disk is write-protected");
        }

        let partitions = children(disk)
            .iter()
            .filter(|c| str_field(c, "type").as_deref() == Some("part"))
            .map(|c| PartitionInfo {
                number: partition_number(c),
                label: str_field(c, "label").or_else(|| str_field(c, "partlabel")),
                filesystem: str_field(c, "fstype"),
                size_bytes: u64_field(c, "size").unwrap_or(0),
                mount_point: mountpoints(c).into_iter().find(|m| !m.starts_with('[')),
            })
            .collect();
        let partition_table = str_field(disk, "pttype").map(|t| match t.as_str() {
            "dos" => "mbr".to_string(),
            other => other.to_string(),
        });
        let transport = str_field(disk, "tran")
            .map(|t| t.to_lowercase())
            .or_else(|| disk_kname.starts_with("mmcblk").then(|| "sd".to_string()));
        let mut info = DiskInfo {
            device_path: path,
            model: str_field(disk, "model"),
            vendor: str_field(disk, "vendor"),
            serial_number: str_field(disk, "serial"),
            size_bytes: size,
            size_display: size_display(size),
            transport,
            removable: bool_field(disk, "rm").unwrap_or(false),
            partition_table,
            partitions,
            is_system_disk: false,
            blocked_reason: None,
        };
        verdict.apply(&mut info);
        result.push(info);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
   "blockdevices": [
      {"name":"sda","kname":"sda","path":"/dev/sda","maj:min":"8:0","type":"disk","tran":"sata","rm":false,"hotplug":false,"ro":false,"size":500107862016,"model":"Samsung SSD 870","serial":"S6P","pttype":"gpt","mountpoints":[null],
       "children":[
          {"name":"sda1","kname":"sda1","path":"/dev/sda1","type":"part","size":536870912,"fstype":"vfat","mountpoints":["/boot/efi"]},
          {"name":"sda2","kname":"sda2","path":"/dev/sda2","type":"part","size":499570991104,"fstype":"btrfs","mountpoints":["/home","/"]}
       ]},
      {"name":"sdb","kname":"sdb","path":"/dev/sdb","maj:min":"8:16","type":"disk","tran":"usb","rm":true,"hotplug":true,"ro":false,"size":30752000000,"model":"Ultra","vendor":"SanDisk ","serial":"4C530001231115117453","pttype":"dos","mountpoints":[null],
       "children":[
          {"name":"sdb1","kname":"sdb1","path":"/dev/sdb1","type":"part","partn":1,"size":30750000000,"fstype":"exfat","label":"STICK","mountpoints":["/media/user/STICK"]}
       ]},
      {"name":"sdc","kname":"sdc","path":"/dev/sdc","type":"disk","tran":"usb","rm":"1","hotplug":"1","ro":"0","size":"7743995904","model":"Live USB","mountpoint":null,
       "children":[
          {"name":"sdc1","kname":"sdc1","path":"/dev/sdc1","type":"part","size":"3000000000","fstype":"iso9660","mountpoint":"/run/initramfs/live"}
       ]},
      {"name":"sdd","kname":"sdd","path":"/dev/sdd","type":"disk","tran":"usb","rm":0,"hotplug":1,"ro":0,"size":1000204886016,"model":"Elements","serial":"WX1","pttype":"gpt",
       "children":[
          {"name":"sdd1","kname":"sdd1","path":"/dev/sdd1","type":"part","size":1000000000000,"fstype":"crypto_LUKS",
           "children":[{"name":"luks-1","kname":"dm-3","path":"/dev/mapper/luks-1","type":"crypt","size":999000000000,"fstype":"ext4","mountpoints":["/media/user/backup"]}]}
       ]},
      {"name":"sde","kname":"sde","path":"/dev/sde","type":"disk","tran":"usb","rm":true,"ro":true,"size":15500000000,"model":"Locked SD adapter"},
      {"name":"sdf","kname":"sdf","path":"/dev/sdf","type":"disk","tran":"usb","rm":true,"ro":false,"size":0,"model":"Card reader"},
      {"name":"sdg","kname":"sdg","path":"/dev/sdg","type":"disk","tran":"usb","rm":true,"ro":false,"size":16000000000,"model":"Swap stick",
       "children":[{"name":"sdg1","kname":"sdg1","path":"/dev/sdg1","type":"part","size":16000000000,"fstype":"swap","mountpoints":["[SWAP]"]}]},
      {"name":"sdh","kname":"sdh","path":"/dev/sdh","type":"disk","tran":"usb","rm":true,"ro":false,"size":64000000000,"model":"App stick",
       "children":[{"name":"sdh1","kname":"sdh1","path":"/dev/sdh1","type":"part","size":64000000000,"fstype":"ext4","mountpoints":["/media/user/APPS"]}]},
      {"name":"nvme0n1","kname":"nvme0n1","path":"/dev/nvme0n1","type":"disk","tran":"nvme","rm":false,"size":1000204886016},
      {"name":"mmcblk0","kname":"mmcblk0","path":"/dev/mmcblk0","type":"disk","tran":null,"rm":false,"size":31914983424,"children":[]},
      {"name":"mmcblk1","kname":"mmcblk1","path":"/dev/mmcblk1","type":"disk","tran":null,"rm":false,"size":63864569856,"children":[]},
      {"name":"mmcblk1boot0","kname":"mmcblk1boot0","path":"/dev/mmcblk1boot0","type":"disk","rm":false,"size":4194304},
      {"name":"loop0","kname":"loop0","path":"/dev/loop0","type":"loop","size":4096,"mountpoints":["/snap/core/1"]},
      {"name":"sr0","kname":"sr0","path":"/dev/sr0","type":"rom","tran":"sata","rm":true,"size":1073741312}
   ]
}"#;

    fn host() -> LinuxHost {
        let mut mmc_types = HashMap::new();
        mmc_types.insert("mmcblk0".to_string(), "SD".to_string());
        mmc_types.insert("mmcblk1".to_string(), "MMC".to_string());
        LinuxHost {
            swaps: "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n/swapfile                               file\t\t8388604\t\t0\t\t-2\n".into(),
            guarded_paths: vec![PathBuf::from("/media/user/APPS/OpCore-OneClick.AppImage")],
            mmc_types,
        }
    }

    fn find<'a>(disks: &'a [DiskInfo], path: &str) -> &'a DiskInfo {
        disks.iter().find(|d| d.device_path == path).unwrap_or_else(|| panic!("{path} missing"))
    }

    #[test]
    fn lists_external_disks_only() {
        let disks = parse_lsblk(FIXTURE, &host()).unwrap();
        let paths: Vec<_> = disks.iter().map(|d| d.device_path.as_str()).collect();
        assert_eq!(paths, ["/dev/sdb", "/dev/sdc", "/dev/sdd", "/dev/sde", "/dev/sdg", "/dev/sdh", "/dev/mmcblk0"]);
    }

    #[test]
    fn plain_usb_stick_is_selectable() {
        let disks = parse_lsblk(FIXTURE, &host()).unwrap();
        let stick = find(&disks, "/dev/sdb");
        assert!(!stick.is_system_disk);
        assert!(stick.blocked_reason.is_none());
        assert_eq!(stick.vendor.as_deref(), Some("SanDisk"));
        assert_eq!(stick.partition_table.as_deref(), Some("mbr"));
        assert_eq!(stick.partitions[0].number, 1);
        assert_eq!(stick.partitions[0].label.as_deref(), Some("STICK"));
        assert_eq!(stick.partitions[0].mount_point.as_deref(), Some("/media/user/STICK"));
        assert!(stick.removable);
    }

    #[test]
    fn live_medium_is_system() {
        let disks = parse_lsblk(FIXTURE, &host()).unwrap();
        let live = find(&disks, "/dev/sdc");
        assert!(live.is_system_disk);
        assert!(live.blocked_reason.as_deref().unwrap().contains("live system"));
        assert_eq!(live.size_bytes, 7_743_995_904);
        assert_eq!(live.partitions[0].number, 1);
    }

    #[test]
    fn open_crypt_holder_blocks_the_disk() {
        let disks = parse_lsblk(FIXTURE, &host()).unwrap();
        let hdd = find(&disks, "/dev/sdd");
        assert!(!hdd.is_system_disk);
        assert!(hdd.blocked_reason.as_deref().unwrap().contains("crypt"));
        assert!(!hdd.removable);
    }

    #[test]
    fn swap_read_only_and_app_medium() {
        let disks = parse_lsblk(FIXTURE, &host()).unwrap();
        assert!(find(&disks, "/dev/sde").blocked_reason.as_deref().unwrap().contains("write-protected"));
        assert!(!find(&disks, "/dev/sde").is_system_disk);
        assert!(find(&disks, "/dev/sdg").is_system_disk);
        let app = find(&disks, "/dev/sdh");
        assert!(app.is_system_disk);
        assert!(app.blocked_reason.as_deref().unwrap().contains("OpCore-OneClick"));
    }

    #[test]
    fn sd_card_is_listed_but_emmc_is_not() {
        let disks = parse_lsblk(FIXTURE, &host()).unwrap();
        let sd = find(&disks, "/dev/mmcblk0");
        assert_eq!(sd.transport.as_deref(), Some("sd"));
        assert!(disks.iter().all(|d| d.device_path != "/dev/mmcblk1"));
    }

    #[test]
    fn root_on_a_usb_disk_is_system() {
        let json = r#"{"blockdevices":[{"name":"sdb","path":"/dev/sdb","type":"disk","tran":"usb","rm":false,"size":256000000000,
           "children":[{"name":"sdb1","path":"/dev/sdb1","type":"part","size":1000000,"mountpoint":"/boot/efi"},
                       {"name":"sdb2","path":"/dev/sdb2","type":"part","size":255000000000,"fstype":"crypto_LUKS",
                        "children":[{"name":"root","kname":"dm-0","path":"/dev/mapper/root","type":"crypt","mountpoints":["/"]}]}]}]}"#;
        let disks = parse_lsblk(json, &LinuxHost::default()).unwrap();
        assert!(disks[0].is_system_disk);
        let reason = disks[0].blocked_reason.clone().unwrap();
        assert!(reason.contains("(/boot/efi)"));
        assert!(reason.contains("(/)"));
    }

    #[test]
    fn swap_partition_from_proc_swaps() {
        let json = r#"{"blockdevices":[{"name":"sdb","path":"/dev/sdb","type":"disk","tran":"usb","rm":true,"size":8000000000,
           "children":[{"name":"sdb2","kname":"sdb2","path":"/dev/sdb2","type":"part","size":8000000000}]}]}"#;
        let host = LinuxHost { swaps: "Filename Type Size Used Priority\n/dev/sdb2 partition 1000 0 -2\n".into(), ..Default::default() };
        let disks = parse_lsblk(json, &host).unwrap();
        assert!(disks[0].is_system_disk);
    }

    #[test]
    fn swap_file_on_usb_disk() {
        let json = r#"{"blockdevices":[
           {"name":"sda","path":"/dev/sda","type":"disk","tran":"sata","size":500000000000,"children":[{"name":"sda1","type":"part","mountpoints":["/"]}]},
           {"name":"sdb","path":"/dev/sdb","type":"disk","tran":"usb","rm":true,"size":8000000000,"children":[{"name":"sdb1","type":"part","mountpoints":["/mnt/extra"]}]}]}"#;
        let host = LinuxHost { swaps: "Filename Type Size Used Priority\n/mnt/extra/swap\\040file file 1000 0 -2\n".into(), ..Default::default() };
        let disks = parse_lsblk(json, &host).unwrap();
        assert_eq!(disks.len(), 1);
        assert!(disks[0].is_system_disk);
        assert!(disks[0].blocked_reason.as_deref().unwrap().contains("/mnt/extra/swap file"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_lsblk("{", &LinuxHost::default()).is_err());
        assert!(parse_lsblk(r#"{"blockdevices":[]}"#, &LinuxHost::default()).unwrap().is_empty());
    }
}
