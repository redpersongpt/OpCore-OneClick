//! Rooted access to `/sys` and `/proc` (tests point the root at a fixture
//! tree) and PCI function enumeration.

use std::path::{Path, PathBuf};

use crate::contracts::PciLocation;
use crate::platform::common::{
    clean_text, hex_id, normalize_acpi_path, parse_bdf, parse_pci_root_segment,
    sysfs_to_device_path_with_root_uid, PciClass,
};

use super::parse::PciIds;

#[derive(Debug, Clone)]
pub struct SysRoot {
    root: PathBuf,
    canonical_root: PathBuf,
}

impl SysRoot {
    pub fn system() -> Self {
        Self::new("/")
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let canonical_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        Self {
            root,
            canonical_root,
        }
    }

    pub fn path(&self, abs: &str) -> PathBuf {
        self.root.join(abs.trim_start_matches('/'))
    }

    /// Trimmed text of a small attribute file.
    pub fn read(&self, abs: &str) -> Option<String> {
        std::fs::read_to_string(self.path(abs))
            .ok()
            .and_then(|s| clean_text(&s))
    }

    pub fn read_bytes(&self, abs: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.path(abs))
    }

    /// Hex attribute ("0x8086") as a lowercase id of `width` digits.
    pub fn read_hex(&self, abs: &str, width: usize) -> Option<String> {
        self.read(abs).and_then(|v| hex_id(&v, width))
    }

    pub fn exists(&self, abs: &str) -> bool {
        self.path(abs).exists()
    }

    /// Sorted entry names of a directory (empty when unreadable).
    pub fn list(&self, abs: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path(abs))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by(|a, b| natural_cmp(a, b));
        names
    }

    /// File name of a symlink target (`driver` → "e1000e").
    pub fn link_name(&self, abs: &str) -> Option<String> {
        std::fs::read_link(self.path(abs))
            .ok()
            .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
    }

    /// Resolved path below the root, as an absolute sysfs path
    /// ("/sys/devices/pci0000:00/0000:00:1f.3").
    pub fn real(&self, abs: &str) -> Option<String> {
        let resolved = std::fs::canonicalize(self.path(abs)).ok()?;
        let relative = resolved.strip_prefix(&self.canonical_root).ok()?;
        Some(format!("/{}", relative.to_string_lossy()))
    }
}

/// Compare names so that "SSDT2" sorts before "SSDT10" and "usb2-port2" before "usb2-port10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let digits_at = s.len() - s.chars().rev().take_while(char::is_ascii_digit).count();
        let (head, tail) = s.split_at(digits_at);
        (
            head.to_string(),
            tail.parse::<u64>().unwrap_or(0),
            s.to_string(),
        )
    };
    key(a).cmp(&key(b))
}

/// One PCI function from `/sys/bus/pci/devices`.
#[derive(Debug, Clone)]
pub struct PciFunction {
    /// "0000:00:1f.3"
    pub address: String,
    /// "/sys/bus/pci/devices/0000:00:1f.3"
    pub dir: String,
    pub vendor_id: String,
    pub device_id: String,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub revision: Option<String>,
    pub class: PciClass,
    pub driver: Option<String>,
    pub location: PciLocation,
    /// lspci-style name from pci.ids, if available.
    pub db_name: Option<String>,
}

impl PciFunction {
    pub fn is(&self, base: u8, sub: u8) -> bool {
        self.class.is(base, sub)
    }

    /// pci.ids name, else "<class label> vvvv:dddd".
    pub fn display_name(&self, class_label: &str) -> String {
        self.db_name
            .clone()
            .unwrap_or_else(|| format!("{class_label} {}:{}", self.vendor_id, self.device_id))
    }
}

pub fn pci_functions(sys: &SysRoot, ids: &PciIds) -> Vec<PciFunction> {
    sys.list("/sys/bus/pci/devices")
        .into_iter()
        .filter_map(|address| {
            let dir = format!("/sys/bus/pci/devices/{address}");
            let vendor_id = sys.read_hex(&format!("{dir}/vendor"), 4)?;
            let device_id = sys.read_hex(&format!("{dir}/device"), 4)?;
            let class = sys
                .read(&format!("{dir}/class"))
                .and_then(|c| PciClass::from_hex(&c))?;
            let location = pci_location(sys, &dir);
            Some(PciFunction {
                db_name: ids.name(&vendor_id, &device_id),
                subsystem_vendor_id: sys.read_hex(&format!("{dir}/subsystem_vendor"), 4),
                subsystem_device_id: sys.read_hex(&format!("{dir}/subsystem_device"), 4),
                revision: sys.read_hex(&format!("{dir}/revision"), 2),
                driver: sys.link_name(&format!("{dir}/driver")),
                address,
                dir,
                vendor_id,
                device_id,
                class,
                location,
            })
        })
        .collect()
}

fn pci_location(sys: &SysRoot, dir: &str) -> PciLocation {
    let pci_path = sys.real(dir).and_then(|real| {
        let root = real
            .split('/')
            .find(|s| parse_pci_root_segment(s).is_some())?
            .to_string();
        let uid = sys
            .read(&format!("/sys/devices/{root}/firmware_node/uid"))
            .and_then(|u| u.parse::<u32>().ok());
        sysfs_to_device_path_with_root_uid(&real, uid)
    });
    let acpi_path = sys
        .read(&format!("{dir}/firmware_node/path"))
        .map(|p| normalize_acpi_path(&p));
    PciLocation {
        pci_path,
        acpi_path,
    }
}

/// PCI address of the deepest PCI function in a resolved sysfs path.
pub fn owning_pci_address(real: &str) -> Option<String> {
    real.split('/')
        .rev()
        .find(|s| parse_bdf(s).is_some())
        .map(str::to_string)
}

/// Directory of the USB device (the one with `idVendor`) that contains a
/// resolved sysfs path, e.g. the parent of a USB interface.
pub fn usb_device_dir(sys: &SysRoot, real: &str) -> Option<String> {
    let mut path = Path::new(real);
    loop {
        let candidate = path.to_string_lossy();
        if sys.exists(&format!("{candidate}/idVendor")) {
            return Some(candidate.into_owned());
        }
        path = path.parent()?;
        if path.as_os_str().len() <= "/sys/devices".len() {
            return None;
        }
    }
}

#[cfg(test)]
pub mod fixture {
    //! Builds fake `/sys` trees for tests.

    use std::path::{Path, PathBuf};

    pub struct Tree {
        pub root: PathBuf,
    }

    impl Tree {
        pub fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("oc-sysfs-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn abs(&self, path: &str) -> PathBuf {
            self.root.join(path.trim_start_matches('/'))
        }

        pub fn file(&self, path: &str, contents: &str) -> &Self {
            self.bytes(path, contents.as_bytes())
        }

        pub fn bytes(&self, path: &str, contents: &[u8]) -> &Self {
            let p = self.abs(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, contents).unwrap();
            self
        }

        pub fn dir(&self, path: &str) -> &Self {
            std::fs::create_dir_all(self.abs(path)).unwrap();
            self
        }

        /// Symlink `link` → `target` (both absolute inside the tree).
        pub fn link(&self, link: &str, target: &str) -> &Self {
            let l = self.abs(link);
            std::fs::create_dir_all(l.parent().unwrap()).unwrap();
            let t = self.abs(target);
            let relative = relative_path(l.parent().unwrap(), &t);
            #[cfg(unix)]
            std::os::unix::fs::symlink(relative, &l).unwrap();
            self
        }

        /// A PCI function under `/sys/devices/<parent>/<address>` with the
        /// usual attribute files and a `/sys/bus/pci/devices` link.
        pub fn pci(
            &self,
            parent: &str,
            address: &str,
            vendor: &str,
            device: &str,
            class: &str,
        ) -> String {
            let dir = format!("/sys/devices/{parent}/{address}");
            self.file(&format!("{dir}/vendor"), &format!("0x{vendor}\n"))
                .file(&format!("{dir}/device"), &format!("0x{device}\n"))
                .file(&format!("{dir}/class"), &format!("0x{class}\n"))
                .file(&format!("{dir}/subsystem_vendor"), "0x1043\n")
                .file(&format!("{dir}/subsystem_device"), "0x8694\n")
                .file(&format!("{dir}/revision"), "0x02\n")
                .link(&format!("/sys/bus/pci/devices/{address}"), &dir);
            dir
        }
    }

    fn relative_path(from_dir: &Path, to: &Path) -> PathBuf {
        let from: Vec<_> = from_dir.components().collect();
        let to_parts: Vec<_> = to.components().collect();
        let common = from
            .iter()
            .zip(&to_parts)
            .take_while(|(a, b)| a == b)
            .count();
        let mut out = PathBuf::new();
        for _ in common..from.len() {
            out.push("..");
        }
        for part in &to_parts[common..] {
            out.push(part);
        }
        out
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Tree;
    use super::*;

    #[test]
    fn enumerates_pci_functions_with_paths() {
        let tree = Tree::new("pci");
        let hda = tree.pci("pci0000:00", "0000:00:1f.3", "8086", "a348", "040300");
        tree.file(&format!("{hda}/firmware_node/path"), "\\_SB_.PCI0.HDAS\n");
        tree.link(
            &format!("{hda}/driver"),
            "/sys/bus/pci/drivers/snd_hda_intel",
        );
        let gpu = tree.pci(
            "pci0000:00/0000:00:01.0",
            "0000:01:00.0",
            "1002",
            "67df",
            "030000",
        );
        let _ = gpu;
        tree.pci("pci0000:16", "0000:16:00.0", "8086", "2030", "060400");
        tree.file("/sys/devices/pci0000:16/firmware_node/uid", "1\n");
        tree.dir("/sys/bus/pci/drivers/snd_hda_intel");

        let sys = SysRoot::new(&tree.root);
        let ids = PciIds::parse("8086  Intel Corporation\n\ta348  Cannon Lake PCH cAVS\n");
        let functions = pci_functions(&sys, &ids);
        let find = |a: &str| functions.iter().find(|f| f.address == a).unwrap();

        let hda = find("0000:00:1f.3");
        assert!(hda.is(0x04, 0x03));
        assert_eq!(hda.driver.as_deref(), Some("snd_hda_intel"));
        assert_eq!(
            hda.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        assert_eq!(hda.location.acpi_path.as_deref(), Some(r"\_SB.PCI0.HDAS"));
        assert_eq!(
            hda.db_name.as_deref(),
            Some("Intel Corporation Cannon Lake PCH cAVS")
        );
        assert_eq!(hda.subsystem_vendor_id.as_deref(), Some("1043"));

        let gpu = find("0000:01:00.0");
        assert_eq!(
            gpu.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)")
        );
        assert_eq!(gpu.display_name("VGA"), "VGA 1002:67df");

        let second_root = find("0000:16:00.0");
        assert_eq!(
            second_root.location.pci_path.as_deref(),
            Some("PciRoot(0x1)/Pci(0x0,0x0)")
        );
        assert_eq!(
            owning_pci_address("/sys/devices/pci0000:00/0000:00:1d.0/0000:3d:00.0/nvme/nvme0")
                .as_deref(),
            Some("0000:3d:00.0")
        );
    }

    #[test]
    fn natural_order() {
        let mut names = vec![
            "SSDT10".to_string(),
            "SSDT2".into(),
            "SSDT1".into(),
            "DSDT".into(),
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["DSDT", "SSDT1", "SSDT2", "SSDT10"]);
    }
}
