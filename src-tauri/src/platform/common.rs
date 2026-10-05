//! Platform-independent parsing helpers shared by the scanners and disk code
//! (unit-testable on every host).

/// Parse a Windows PnP device id ("PCI\\VEN_8086&DEV_3E92&SUBSYS_86941043&REV_02",
/// "HDAUDIO\\FUNC_01&VEN_10EC&DEV_0897&SUBSYS_10438698&REV_1003",
/// "USB\\VID_8087&PID_0029") into lowercase hex fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PnpIds {
    pub bus: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub revision: Option<String>,
}

pub fn parse_pnp_id(id: &str) -> PnpIds {
    todo!("parse_pnp_id {id}")
}

/// Convert a Windows `DEVPKEY_Device_LocationPaths` entry
/// ("PCIROOT(0)#PCI(1F03)" / "PCIROOT(0)#PCI(0100)#PCI(0000)") to an OpenCore
/// device path ("PciRoot(0x0)/Pci(0x1f,0x3)").
pub fn location_path_to_device_path(location: &str) -> Option<String> {
    todo!("location_path_to_device_path {location}")
}

/// Convert a Linux sysfs PCI device path
/// ("/sys/devices/pci0000:00/0000:00:01.0/0000:01:00.0") to an OpenCore device path.
pub fn sysfs_to_device_path(sysfs: &str) -> Option<String> {
    todo!("sysfs_to_device_path {sysfs}")
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
