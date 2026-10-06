//! IORegistry parsing (`ioreg -a` XML plists): PCI devices with their device
//! paths, HD Audio codecs, NIC MAC addresses, drives and XHCI ports found in
//! each device's subtree, HID / PS2 input devices and NVRAM values.

use plist::{Dictionary, Value};

use crate::contracts::{InputDevice, InputKind, PciLocation, UsbPortInfo};
use crate::error::AppError;
use crate::platform::common::{
    clean_text, hex16, input_vendor, normalize_acpi_path, normalize_mac, PciClass,
};

/// OpenCore's vendor GUID; `oem-*` variables are exposed with ExposeSensitiveData.
pub const OPENCORE_GUID: &str = "4D1FDA02-38C7-4A6A-9CC6-4BCCA8B30102";

#[derive(Debug, Clone, Default)]
pub struct IoPciDevice {
    pub vendor_id: String,
    pub device_id: String,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub revision: Option<String>,
    pub class: Option<PciClass>,
    /// `model` property, else `IOName`, else the registry entry name.
    pub name: String,
    /// Registry entry name ("GFX0", "HDEF", "wlan").
    pub entry_name: String,
    pub location: PciLocation,
    pub mac_addresses: Vec<String>,
    pub codecs: Vec<IoCodec>,
    pub drives: Vec<IoDrive>,
    pub usb_ports: Vec<UsbPortInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoCodec {
    pub vendor_id: String,
    pub device_id: String,
    pub revision: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoDrive {
    pub name: String,
    pub size_bytes: Option<u64>,
}

fn parse_array(xml: &[u8], what: &str) -> Result<Vec<Value>, AppError> {
    if xml.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let value = Value::from_reader_xml(xml)
        .map_err(|e| AppError::new("SCAN_PARSE", format!("Cannot parse {what}: {e}")))?;
    Ok(value.into_array().unwrap_or_default())
}

/// True when an `ioreg -a -r ...` query matched at least one entry.
pub fn has_entries(xml: &[u8]) -> bool {
    parse_array(xml, "ioreg output")
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

/// Parse `ioreg -a -r -c IOPCIDevice` (full subtrees).
pub fn parse_pci_tree(xml: &[u8]) -> Result<Vec<IoPciDevice>, AppError> {
    let roots = parse_array(xml, "PCI registry")?;
    let mut devices = Vec::new();
    for root in &roots {
        if let Some(dict) = root.as_dictionary() {
            let uid = text(dict, "acpi-path")
                .map(|p| acpi_root_uid(&p))
                .unwrap_or(0);
            walk(dict, &format!("PciRoot({uid:#x})"), None, &mut devices);
        }
    }
    Ok(devices)
}

fn walk(node: &Dictionary, parent_path: &str, owner: Option<usize>, out: &mut Vec<IoPciDevice>) {
    let (path, owner) = match pci_device(node, parent_path) {
        Some(device) => {
            let path = device.location.pci_path.clone().unwrap_or_default();
            out.push(device);
            (path, Some(out.len() - 1))
        }
        None => {
            if let Some(device) = owner.and_then(|i| out.get_mut(i)) {
                collect_facts(node, device);
            }
            (parent_path.to_string(), owner)
        }
    };
    for child in children(node) {
        walk(child, &path, owner, out);
    }
}

fn children(node: &Dictionary) -> impl Iterator<Item = &Dictionary> {
    node.get("IORegistryEntryChildren")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_dictionary)
}

fn pci_device(node: &Dictionary, parent_path: &str) -> Option<IoPciDevice> {
    let class_name = text(node, "IOObjectClass").unwrap_or_default();
    if !class_name.ends_with("PCIDevice") {
        return None;
    }
    let vendor = data_u32(node, "vendor-id")?;
    let device = data_u32(node, "device-id")?;
    let entry_name = text(node, "IORegistryEntryName").unwrap_or_default();
    let name = text(node, "model")
        .or_else(|| text(node, "IOName"))
        .unwrap_or_else(|| entry_name.clone());
    let pci_path = text(node, "IORegistryEntryLocation")
        .and_then(|loc| parse_location(&loc))
        .map(|(dev, func)| format!("{parent_path}/Pci({dev:#x},{func:#x})"));
    let acpi_path = text(node, "acpi-path").and_then(|p| acpi_plane_to_path(&p));
    Some(IoPciDevice {
        vendor_id: hex16(vendor),
        device_id: hex16(device),
        subsystem_vendor_id: data_u32(node, "subsystem-vendor-id").map(hex16),
        subsystem_device_id: data_u32(node, "subsystem-id").map(hex16),
        revision: data_u32(node, "revision-id").map(|r| format!("{:02x}", r & 0xff)),
        class: data_u32(node, "class-code").map(PciClass::from_u32),
        name,
        entry_name,
        location: PciLocation {
            pci_path,
            acpi_path,
        },
        ..Default::default()
    })
}

fn collect_facts(node: &Dictionary, device: &mut IoPciDevice) {
    if let Some(mac) = node
        .get("IOMACAddress")
        .and_then(Value::as_data)
        .and_then(mac_from_bytes)
    {
        if !device.mac_addresses.contains(&mac) {
            device.mac_addresses.push(mac);
        }
    }
    if let Some(id) = node.get("IOHDACodecVendorID").and_then(integer) {
        let id = id as u32;
        device.codecs.push(IoCodec {
            vendor_id: hex16(id >> 16),
            device_id: hex16(id),
            revision: node
                .get("IOHDACodecRevisionID")
                .and_then(integer)
                .map(|r| r as u32),
        });
    }
    if let Some(characteristics) = node
        .get("Device Characteristics")
        .and_then(Value::as_dictionary)
    {
        let name =
            text(characteristics, "Product Name").or_else(|| text(characteristics, "Model Number"));
        if let Some(name) = name {
            device.drives.push(IoDrive {
                name,
                size_bytes: whole_media_size(node),
            });
        }
    }
    let class_name = text(node, "IOObjectClass").unwrap_or_default();
    if class_name.starts_with("AppleUSB")
        && class_name.ends_with("Port")
        && !class_name.contains("Hub")
    {
        device.usb_ports.push(usb_port(node, &class_name));
    }
}

fn usb_port(node: &Dictionary, class_name: &str) -> UsbPortInfo {
    let name = text(node, "IORegistryEntryName");
    let index = data_u32(node, "port").or_else(|| {
        name.as_deref()
            .map(|n| n.chars().filter(char::is_ascii_digit).collect::<String>())
            .and_then(|d| d.parse().ok())
    });
    let usb3 = class_name.contains("30") || name.as_deref().is_some_and(|n| n.starts_with("SS"));
    let connector = node.get("UsbConnector").and_then(integer).map(|c| c as u32);
    UsbPortInfo {
        index: index.unwrap_or(0),
        name,
        speed_class: if usb3 { "usb3" } else { "usb2" }.to_string(),
        connector,
        user_connectable: connector.map(|c| c != 255),
        companion: None,
    }
}

fn whole_media_size(node: &Dictionary) -> Option<u64> {
    if node.get("Whole").and_then(Value::as_boolean) == Some(true) {
        if let Some(size) = node.get("Size").and_then(integer) {
            return Some(size);
        }
    }
    children(node).find_map(whole_media_size)
}

/// "1f,3" → (0x1f, 3); "2" → (2, 0).
fn parse_location(location: &str) -> Option<(u32, u32)> {
    let (dev, func) = location.split_once(',').unwrap_or((location, "0"));
    let dev = u32::from_str_radix(dev.trim(), 16).ok()?;
    let func = u32::from_str_radix(func.trim(), 16).ok()?;
    (dev <= 0x1f && func <= 7).then_some((dev, func))
}

/// "IOACPIPlane:/_SB/PCI0@0/HDEF@1f0003" → "\\_SB.PCI0.HDEF".
pub fn acpi_plane_to_path(path: &str) -> Option<String> {
    let body = path.split_once(":/").map(|(_, b)| b).unwrap_or(path);
    let names: Vec<&str> = body
        .split('/')
        .map(|segment| segment.split('@').next().unwrap_or_default())
        .filter(|s| !s.is_empty())
        .collect();
    (!names.is_empty()).then(|| normalize_acpi_path(&names.join(".")))
}

/// Host bridge `_UID` guess from its ACPI name: PCI0/PC00 → 0, PC01 → 1.
fn acpi_root_uid(path: &str) -> u32 {
    let body = path.split_once(":/").map(|(_, b)| b).unwrap_or(path);
    let root = body
        .split('/')
        .nth(1)
        .and_then(|s| s.split('@').next())
        .unwrap_or_default();
    let digits = root
        .strip_prefix("PCI")
        .or_else(|| root.strip_prefix("PC"))
        .unwrap_or_default();
    if digits.is_empty() || digits.len() > 2 {
        return 0;
    }
    u32::from_str_radix(digits, 16).unwrap_or(0)
}

fn mac_from_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 6 {
        return None;
    }
    normalize_mac(&bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn integer(value: &Value) -> Option<u64> {
    value
        .as_unsigned_integer()
        .or_else(|| value.as_signed_integer().map(|v| v as u64))
}

/// Little-endian `<data>` (or integer) value as u32.
fn data_u32(dict: &Dictionary, key: &str) -> Option<u32> {
    let value = dict.get(key)?;
    if let Some(bytes) = value.as_data() {
        let mut buf = [0u8; 4];
        let n = bytes.len().min(4);
        if n == 0 {
            return None;
        }
        buf[..n].copy_from_slice(&bytes[..n]);
        return Some(u32::from_le_bytes(buf));
    }
    integer(value).map(|v| v as u32)
}

/// String or NUL-terminated data value.
fn text(dict: &Dictionary, key: &str) -> Option<String> {
    match dict.get(key)? {
        Value::String(s) => clean_text(s),
        Value::Data(bytes) => {
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            let s = std::str::from_utf8(&bytes[..end]).ok()?;
            s.chars()
                .all(|c| !c.is_control())
                .then(|| clean_text(s))
                .flatten()
        }
        _ => None,
    }
}

/// Keyboards, pointing devices and touch surfaces from `ioreg -a -r -c IOHIDDevice -d 1`.
pub fn parse_hid_devices(xml: &[u8]) -> Result<Vec<InputDevice>, AppError> {
    let mut devices: Vec<InputDevice> = Vec::new();
    for entry in parse_array(xml, "HID registry")? {
        let Some(dict) = entry.as_dictionary() else {
            continue;
        };
        let page = dict.get("PrimaryUsagePage").and_then(integer);
        let usage = dict.get("PrimaryUsage").and_then(integer);
        let name = text(dict, "Product").unwrap_or_else(|| "HID device".to_string());
        let lower = name.to_ascii_lowercase();
        let kind = match (page, usage) {
            (Some(1), Some(6)) => InputKind::Keyboard,
            (Some(1), Some(2)) if lower.contains("trackpad") || lower.contains("touchpad") => {
                InputKind::Touchpad
            }
            (Some(1), Some(2)) => InputKind::Mouse,
            (Some(0x0d), Some(5)) => InputKind::Touchpad,
            (Some(0x0d), Some(4)) => InputKind::Touchscreen,
            _ => continue,
        };
        let transport = text(dict, "Transport")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let bus = if transport == "usb" {
            "usb"
        } else if transport.contains("bluetooth") {
            "bluetooth"
        } else if transport == "i2c" {
            "i2c"
        } else {
            "unknown"
        };
        if devices.iter().any(|d| d.name == name && d.kind == kind) {
            continue;
        }
        devices.push(InputDevice {
            vendor: input_vendor(None, &name),
            name,
            kind,
            bus: bus.to_string(),
            hardware_id: None,
        });
    }
    Ok(devices)
}

/// PS/2 devices attached to VoodooPS2's `ApplePS2Controller` (Hackintosh).
pub fn parse_ps2_devices(xml: &[u8]) -> Result<Vec<InputDevice>, AppError> {
    fn visit(node: &Dictionary, out: &mut Vec<InputDevice>) {
        let class_name = text(node, "IOObjectClass").unwrap_or_default();
        let lower = class_name.to_ascii_lowercase();
        let kind = if lower.contains("keyboard") {
            Some(InputKind::Keyboard)
        } else if lower.contains("touchpad")
            || lower.contains("glidepoint")
            || lower.contains("elan")
        {
            Some(InputKind::Touchpad)
        } else if lower.contains("mouse") {
            Some(InputKind::Mouse)
        } else {
            None
        };
        // `ApplePS2KeyboardDevice` / `ApplePS2MouseDevice` are the port nubs;
        // the driver objects attached below them name the actual device.
        let is_driver = lower.starts_with("appleps2")
            && !lower.ends_with("device")
            && class_name != "ApplePS2Controller";
        if let Some(kind) = kind.filter(|_| is_driver) {
            let vendor = if lower.contains("synaptics") {
                Some("synaptics".to_string())
            } else if lower.contains("alps") || lower.contains("glidepoint") {
                Some("alps".to_string())
            } else if lower.contains("elan") {
                Some("elan".to_string())
            } else {
                None
            };
            out.push(InputDevice {
                name: class_name.clone(),
                kind,
                bus: "ps2".into(),
                hardware_id: None,
                vendor,
            });
        }
        for child in children(node) {
            visit(child, out);
        }
    }
    let mut out = Vec::new();
    for entry in parse_array(xml, "PS2 registry")? {
        if let Some(dict) = entry.as_dictionary() {
            visit(dict, &mut out);
        }
    }
    Ok(out)
}

/// String value of an NVRAM variable from `nvram -x -p`.
pub fn nvram_text(xml: &[u8], name: &str) -> Option<String> {
    let value = Value::from_reader_xml(xml).ok()?;
    let dict = value.as_dictionary()?;
    match dict.get(name)? {
        Value::String(s) => clean_text(s),
        Value::Data(bytes) => clean_text(&String::from_utf8_lossy(bytes)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plist(body: &str) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n{body}\n</plist>"
        )
        .into_bytes()
    }

    // Trimmed Coffee Lake Hackintosh tree: iGPU, HDEF with an ALC1220 codec,
    // a root port with an RX 580 behind it, an I219-V and the PCH XHCI.
    fn hackintosh_tree() -> Vec<u8> {
        plist(
            r#"<array>
  <dict>
    <key>IOObjectClass</key><string>IOPCIDevice</string>
    <key>IORegistryEntryName</key><string>IGPU</string>
    <key>IORegistryEntryLocation</key><string>2</string>
    <key>acpi-path</key><string>IOACPIPlane:/_SB/PCI0@0/IGPU@20000</string>
    <key>vendor-id</key><data>hoAAAA==</data>
    <key>device-id</key><data>mD4AAA==</data>
    <key>class-code</key><data>AAADAA==</data>
    <key>revision-id</key><data>AgAAAA==</data>
    <key>model</key><data>SW50ZWwgVUhEIEdyYXBoaWNzIDYzMAA=</data>
  </dict>
  <dict>
    <key>IOObjectClass</key><string>IOPCIDevice</string>
    <key>IORegistryEntryName</key><string>HDEF</string>
    <key>IORegistryEntryLocation</key><string>1f,3</string>
    <key>acpi-path</key><string>IOACPIPlane:/_SB/PCI0@0/HDEF@1f0003</string>
    <key>vendor-id</key><data>hoAAAA==</data>
    <key>device-id</key><data>SKMAAA==</data>
    <key>class-code</key><data>AAMEAA==</data>
    <key>IORegistryEntryChildren</key>
    <array>
      <dict>
        <key>IOObjectClass</key><string>AppleHDAController</string>
        <key>IORegistryEntryChildren</key>
        <array>
          <dict>
            <key>IOObjectClass</key><string>IOHDACodecDevice</string>
            <key>IOHDACodecVendorID</key><integer>283906592</integer>
            <key>IOHDACodecRevisionID</key><integer>1049089</integer>
          </dict>
        </array>
      </dict>
    </array>
  </dict>
  <dict>
    <key>IOObjectClass</key><string>IOPCIDevice</string>
    <key>IORegistryEntryName</key><string>PEG0</string>
    <key>IORegistryEntryLocation</key><string>1</string>
    <key>acpi-path</key><string>IOACPIPlane:/_SB/PCI0@0/PEG0@10000</string>
    <key>vendor-id</key><data>hoAAAA==</data>
    <key>device-id</key><data>ARkAAA==</data>
    <key>class-code</key><data>AAQGAA==</data>
    <key>IORegistryEntryChildren</key>
    <array>
      <dict>
        <key>IOObjectClass</key><string>IOPCI2PCIBridge</string>
        <key>IORegistryEntryChildren</key>
        <array>
          <dict>
            <key>IOObjectClass</key><string>IOPCIDevice</string>
            <key>IORegistryEntryName</key><string>GFX0</string>
            <key>IORegistryEntryLocation</key><string>0</string>
            <key>acpi-path</key><string>IOACPIPlane:/_SB/PCI0@0/PEG0@10000/PEGP@0</string>
            <key>vendor-id</key><data>AhAAAA==</data>
            <key>device-id</key><data>32cAAA==</data>
            <key>subsystem-vendor-id</key><data>ghQAAA==</data>
            <key>subsystem-id</key><data>QJkAAA==</data>
            <key>class-code</key><data>AAADAA==</data>
            <key>revision-id</key><data>5wAAAA==</data>
          </dict>
        </array>
      </dict>
    </array>
  </dict>
  <dict>
    <key>IOObjectClass</key><string>IOPCIDevice</string>
    <key>IORegistryEntryName</key><string>GLAN</string>
    <key>IORegistryEntryLocation</key><string>1f,6</string>
    <key>vendor-id</key><data>hoAAAA==</data>
    <key>device-id</key><data>vBUAAA==</data>
    <key>class-code</key><data>AAACAA==</data>
    <key>IORegistryEntryChildren</key>
    <array>
      <dict>
        <key>IOObjectClass</key><string>IntelMausi</string>
        <key>IOMACAddress</key><data>pLttEjRW</data>
      </dict>
    </array>
  </dict>
  <dict>
    <key>IOObjectClass</key><string>IOPCIDevice</string>
    <key>IORegistryEntryName</key><string>XHC</string>
    <key>IORegistryEntryLocation</key><string>14</string>
    <key>vendor-id</key><data>hoAAAA==</data>
    <key>device-id</key><data>baMAAA==</data>
    <key>class-code</key><data>MAMMAA==</data>
    <key>IORegistryEntryChildren</key>
    <array>
      <dict>
        <key>IOObjectClass</key><string>AppleUSBXHCISPT300</string>
        <key>IORegistryEntryChildren</key>
        <array>
          <dict>
            <key>IOObjectClass</key><string>AppleUSB20XHCIPort</string>
            <key>IORegistryEntryName</key><string>HS01</string>
            <key>port</key><data>AQAAAA==</data>
            <key>UsbConnector</key><integer>3</integer>
          </dict>
          <dict>
            <key>IOObjectClass</key><string>AppleUSB30XHCIPort</string>
            <key>IORegistryEntryName</key><string>SS01</string>
            <key>port</key><data>EQAAAA==</data>
            <key>UsbConnector</key><integer>3</integer>
            <key>IORegistryEntryChildren</key>
            <array>
              <dict>
                <key>IOObjectClass</key><string>AppleUSB30HubPort</string>
                <key>IORegistryEntryName</key><string>Port1</string>
              </dict>
            </array>
          </dict>
          <dict>
            <key>IOObjectClass</key><string>AppleUSB20XHCIPort</string>
            <key>IORegistryEntryName</key><string>HS14</string>
            <key>port</key><data>DgAAAA==</data>
            <key>UsbConnector</key><integer>255</integer>
          </dict>
        </array>
      </dict>
    </array>
  </dict>
</array>"#,
        )
    }

    #[test]
    fn parses_pci_tree_with_paths_and_children() {
        let devices = parse_pci_tree(&hackintosh_tree()).unwrap();
        let ids: Vec<String> = devices
            .iter()
            .map(|d| format!("{}:{}", d.vendor_id, d.device_id))
            .collect();
        assert_eq!(
            ids,
            [
                "8086:3e98",
                "8086:a348",
                "8086:1901",
                "1002:67df",
                "8086:15bc",
                "8086:a36d"
            ]
        );

        let igpu = &devices[0];
        assert_eq!(igpu.name, "Intel UHD Graphics 630");
        assert_eq!(
            igpu.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x2,0x0)")
        );
        assert_eq!(igpu.location.acpi_path.as_deref(), Some(r"\_SB.PCI0.IGPU"));
        assert_eq!(igpu.revision.as_deref(), Some("02"));
        assert!(igpu.class.unwrap().is(0x03, 0x00));

        let hdef = &devices[1];
        assert_eq!(
            hdef.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        assert_eq!(
            hdef.codecs,
            [IoCodec {
                vendor_id: "10ec".into(),
                device_id: "1220".into(),
                revision: Some(0x100201)
            }]
        );

        let gpu = &devices[3];
        assert_eq!(
            gpu.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)")
        );
        assert_eq!(gpu.subsystem_vendor_id.as_deref(), Some("1482"));
        assert_eq!(gpu.subsystem_device_id.as_deref(), Some("9940"));
        assert_eq!(gpu.revision.as_deref(), Some("e7"));

        assert_eq!(devices[4].mac_addresses, ["a4:bb:6d:12:34:56"]);

        let xhci = &devices[5];
        assert_eq!(xhci.class.unwrap().prog_if, Some(0x30));
        assert_eq!(
            xhci.usb_ports.len(),
            3,
            "hub ports below a root port are not root ports"
        );
        assert_eq!(xhci.usb_ports[1].index, 17);
        assert_eq!(xhci.usb_ports[1].speed_class, "usb3");
        assert_eq!(xhci.usb_ports[2].user_connectable, Some(false));
    }

    #[test]
    fn apple_silicon_wlan_without_acpi() {
        let xml = plist(
            r#"<array><dict>
  <key>IOObjectClass</key><string>IOPCIDevice</string>
  <key>IORegistryEntryName</key><string>wlan</string>
  <key>IORegistryEntryLocation</key><string>0</string>
  <key>IOName</key><string>pci14e4,4434</string>
  <key>vendor-id</key><data>5BQAAA==</data>
  <key>device-id</key><data>NEQAAA==</data>
  <key>class-code</key><data>AIACAA==</data>
</dict></array>"#,
        );
        let devices = parse_pci_tree(&xml).unwrap();
        assert_eq!(devices[0].vendor_id, "14e4");
        assert_eq!(devices[0].name, "pci14e4,4434");
        assert!(devices[0].class.unwrap().is(0x02, 0x80));
        assert_eq!(devices[0].location.acpi_path, None);
        assert!(parse_pci_tree(b"").unwrap().is_empty());
        assert!(parse_pci_tree(b"<not a plist").is_err());
    }

    #[test]
    fn acpi_names() {
        assert_eq!(
            acpi_plane_to_path("IOACPIPlane:/_SB/PCI0@0/RP01@1c0000/PXSX@0").as_deref(),
            Some(r"\_SB.PCI0.RP01.PXSX")
        );
        assert_eq!(acpi_root_uid("IOACPIPlane:/_SB/PC01@0/BR1A@0"), 1);
        assert_eq!(acpi_root_uid("IOACPIPlane:/_SB/PCI0@0/HDEF@1f0003"), 0);
        assert_eq!(parse_location("1c,4"), Some((0x1c, 4)));
        assert_eq!(parse_location("zz"), None);
    }

    #[test]
    fn hid_and_ps2_devices() {
        let hid = plist(
            r#"<array>
  <dict><key>Product</key><string>Apple Internal Keyboard / Trackpad</string><key>Transport</key><string>FIFO</string>
    <key>PrimaryUsagePage</key><integer>1</integer><key>PrimaryUsage</key><integer>2</integer></dict>
  <dict><key>Product</key><string>Apple Internal Keyboard / Trackpad</string><key>Transport</key><string>FIFO</string>
    <key>PrimaryUsagePage</key><integer>1</integer><key>PrimaryUsage</key><integer>6</integer></dict>
  <dict><key>Product</key><string>Apple Internal Keyboard / Trackpad</string><key>Transport</key><string>FIFO</string>
    <key>PrimaryUsagePage</key><integer>1</integer><key>PrimaryUsage</key><integer>6</integer></dict>
  <dict><key>Product</key><string>accel</string><key>Transport</key><string>SPU</string>
    <key>PrimaryUsagePage</key><integer>65280</integer><key>PrimaryUsage</key><integer>3</integer></dict>
  <dict><key>Product</key><string>SYNA2393</string><key>Transport</key><string>I2C</string>
    <key>PrimaryUsagePage</key><integer>13</integer><key>PrimaryUsage</key><integer>5</integer></dict>
  <dict><key>Product</key><string>USB Receiver</string><key>Transport</key><string>USB</string>
    <key>PrimaryUsagePage</key><integer>1</integer><key>PrimaryUsage</key><integer>2</integer></dict>
</array>"#,
        );
        let devices = parse_hid_devices(&hid).unwrap();
        let kinds: Vec<(InputKind, &str)> =
            devices.iter().map(|d| (d.kind, d.bus.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (InputKind::Touchpad, "unknown"),
                (InputKind::Keyboard, "unknown"),
                (InputKind::Touchpad, "i2c"),
                (InputKind::Mouse, "usb")
            ]
        );
        assert_eq!(devices[2].vendor, None);

        let ps2 = plist(
            r#"<array><dict><key>IOObjectClass</key><string>ApplePS2Controller</string>
  <key>IORegistryEntryChildren</key><array>
    <dict><key>IOObjectClass</key><string>ApplePS2KeyboardDevice</string>
      <key>IORegistryEntryChildren</key><array><dict><key>IOObjectClass</key><string>ApplePS2Keyboard</string></dict></array></dict>
    <dict><key>IOObjectClass</key><string>ApplePS2MouseDevice</string>
      <key>IORegistryEntryChildren</key><array><dict><key>IOObjectClass</key><string>ApplePS2SynapticsTouchPad</string></dict></array></dict>
  </array></dict></array>"#,
        );
        let ps2 = parse_ps2_devices(&ps2).unwrap();
        let summary: Vec<(InputKind, Option<&str>)> =
            ps2.iter().map(|d| (d.kind, d.vendor.as_deref())).collect();
        assert_eq!(
            summary,
            [
                (InputKind::Keyboard, None),
                (InputKind::Touchpad, Some("synaptics"))
            ]
        );
    }

    #[test]
    fn nvram_and_presence() {
        let nvram = plist(&format!(
            "<dict><key>{OPENCORE_GUID}:oem-product</key><data>UFJJTUUgWjM5MC1BAA==</data>\
             <key>{OPENCORE_GUID}:opencore-version</key><string>REL-108-2026-09-27</string></dict>"
        ));
        assert_eq!(
            nvram_text(&nvram, &format!("{OPENCORE_GUID}:oem-product")).as_deref(),
            Some("PRIME Z390-A")
        );
        assert_eq!(nvram_text(&nvram, "missing"), None);
        assert!(has_entries(&plist("<array><dict/></array>")));
        assert!(!has_entries(b""));
    }
}
