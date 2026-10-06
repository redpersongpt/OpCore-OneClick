//! `system_profiler -json` parsing. Keys differ between macOS releases and
//! between Intel and Apple silicon, so every lookup is optional.

use serde_json::Value;

use crate::error::AppError;
use crate::platform::common::{clean_text, hex_id, normalize_mac};

/// Data types requested in one `system_profiler` run.
pub const DATA_TYPES: &[&str] = &[
    "SPHardwareDataType",
    "SPDisplaysDataType",
    "SPPCIDataType",
    "SPAudioDataType",
    "SPNetworkDataType",
    "SPBluetoothDataType",
    "SPNVMeDataType",
    "SPSerialATADataType",
    "SPUSBDataType",
    "SPUSBHostDataType",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerReport {
    pub machine_model: Option<String>,
    pub machine_name: Option<String>,
    /// "Apple M2 Pro" (Apple silicon) or "Quad-Core Intel Core i7".
    pub chip: Option<String>,
    pub boot_rom: Option<String>,
    pub memory_bytes: Option<u64>,
    pub gpus: Vec<ProfilerGpu>,
    pub network: Vec<ProfilerNetwork>,
    pub bluetooth: Vec<ProfilerBluetooth>,
    pub drives: Vec<ProfilerDrive>,
    pub usb_buses: Vec<ProfilerUsbBus>,
    /// Built-in CoreAudio devices (speakers, microphones).
    pub builtin_audio: Vec<String>,
    pub pci_cards: Vec<ProfilerPci>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerGpu {
    pub name: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub revision: Option<String>,
    pub vram_mb: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerNetwork {
    pub name: String,
    /// "Ethernet" | "AirPort" | ...
    pub kind: String,
    pub interface: Option<String>,
    pub mac: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerBluetooth {
    pub chipset: Option<String>,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    /// "USB" | "PCIe" | "UART"
    pub transport: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerDrive {
    pub name: String,
    /// "nvme" | "sata"
    pub kind: String,
    pub controller: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerUsbBus {
    pub name: String,
    pub driver: Option<String>,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilerPci {
    pub name: String,
    pub device_type: Option<String>,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub revision: Option<String>,
}

pub fn parse(json: &str) -> Result<ProfilerReport, AppError> {
    let root: Value = serde_json::from_str(json).map_err(|e| {
        AppError::new(
            "SCAN_PARSE",
            format!("system_profiler output is not JSON: {e}"),
        )
    })?;
    let section = |name: &str| -> Vec<&Value> {
        root.get(name)
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default()
    };

    let mut report = ProfilerReport::default();
    if let Some(hw) = section("SPHardwareDataType").first() {
        report.machine_model = text(hw, "machine_model");
        report.machine_name = text(hw, "machine_name");
        report.chip = text(hw, "chip_type").or_else(|| text(hw, "cpu_type"));
        report.boot_rom = text(hw, "boot_rom_version");
        report.memory_bytes = text(hw, "physical_memory").and_then(|m| parse_size_bytes(&m));
    }
    report.gpus = section("SPDisplaysDataType")
        .into_iter()
        .filter_map(parse_gpu)
        .collect();
    report.network = section("SPNetworkDataType")
        .into_iter()
        .filter_map(parse_network)
        .collect();
    report.bluetooth = section("SPBluetoothDataType")
        .into_iter()
        .filter_map(parse_bluetooth)
        .collect();
    for (data_type, kind) in [("SPNVMeDataType", "nvme"), ("SPSerialATADataType", "sata")] {
        for controller in section(data_type) {
            report.drives.extend(parse_drives(controller, kind));
        }
    }
    for bus in section("SPUSBDataType")
        .into_iter()
        .chain(section("SPUSBHostDataType"))
    {
        if let Some(parsed) = parse_usb_bus(bus) {
            report.usb_buses.push(parsed);
        }
    }
    for group in section("SPAudioDataType") {
        for item in items(group) {
            let builtin =
                text(item, "coreaudio_device_transport").is_some_and(|t| t.contains("builtin"));
            if let (true, Some(name)) = (builtin, text(item, "_name")) {
                report.builtin_audio.push(name);
            }
        }
    }
    report.pci_cards = section("SPPCIDataType")
        .into_iter()
        .filter_map(parse_pci)
        .collect();
    Ok(report)
}

fn parse_gpu(item: &Value) -> Option<ProfilerGpu> {
    let name = text(item, "sppci_model").or_else(|| text(item, "_name"))?;
    let vendor_text = text(item, "spdisplays_vendor").unwrap_or_default();
    let vendor_id = text(item, "spdisplays_vendor-id")
        .or_else(|| text(item, "spdisplays_vendor_id"))
        .and_then(|v| hex_id(&v, 4))
        .or_else(|| hex_in_parentheses(&vendor_text))
        .or_else(|| vendor_from_keyword(&vendor_text));
    let vram_mb = item
        .as_object()
        .and_then(|o| {
            o.iter()
                .find(|(k, _)| k.contains("vram"))
                .and_then(|(_, v)| v.as_str())
        })
        .and_then(parse_size_bytes)
        .map(|b| b / (1024 * 1024));
    Some(ProfilerGpu {
        name,
        vendor_id,
        device_id: text(item, "spdisplays_device-id").and_then(|v| hex_id(&v, 4)),
        revision: text(item, "spdisplays_revision-id").and_then(|v| hex_id(&v, 2)),
        vram_mb,
    })
}

fn parse_network(item: &Value) -> Option<ProfilerNetwork> {
    let name = text(item, "_name")?;
    let kind = text(item, "type")
        .or_else(|| text(item, "hardware"))
        .unwrap_or_default();
    let mac = item
        .get("Ethernet")
        .and_then(|e| text(e, "MAC Address"))
        .and_then(|m| normalize_mac(&m));
    Some(ProfilerNetwork {
        name,
        kind,
        interface: text(item, "interface"),
        mac,
    })
}

fn parse_bluetooth(item: &Value) -> Option<ProfilerBluetooth> {
    // macOS 12+: "controller_properties"; older: "local_device_title".
    let controller = item
        .get("controller_properties")
        .or_else(|| item.get("local_device_title"))
        .unwrap_or(item);
    let object = controller.as_object()?;
    let find = |suffix: &str| {
        object
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase().ends_with(suffix))
            .and_then(|(_, v)| v.as_str())
            .and_then(clean_text)
    };
    let id = |suffix: &str| {
        find(suffix).and_then(|v| hex_id(v.split_whitespace().next().unwrap_or_default(), 4))
    };
    let parsed = ProfilerBluetooth {
        chipset: find("chipset"),
        vendor_id: id("vendorid"),
        product_id: id("productid"),
        transport: find("transport"),
    };
    (parsed.vendor_id.is_some() || parsed.chipset.is_some()).then_some(parsed)
}

fn parse_drives(controller: &Value, kind: &str) -> Vec<ProfilerDrive> {
    let controller_name = text(controller, "_name");
    items(controller)
        .into_iter()
        .filter_map(|drive| {
            let name = text(drive, "device_model").or_else(|| text(drive, "_name"))?;
            let size_bytes = drive.get("size_in_bytes").and_then(Value::as_u64);
            Some(ProfilerDrive {
                name,
                kind: kind.to_string(),
                controller: controller_name.clone(),
                size_bytes,
            })
        })
        .collect()
}

fn parse_usb_bus(bus: &Value) -> Option<ProfilerUsbBus> {
    let name = text(bus, "_name")?;
    Some(ProfilerUsbBus {
        name,
        driver: text(bus, "host_controller").or_else(|| text(bus, "Driver")),
        vendor_id: text(bus, "pci_vendor").and_then(|v| hex_id(&v, 4)),
        device_id: text(bus, "pci_device").and_then(|v| hex_id(&v, 4)),
    })
}

fn parse_pci(item: &Value) -> Option<ProfilerPci> {
    let name = text(item, "_name").or_else(|| text(item, "sppci_name"))?;
    let id = |key: &str, width: usize| text(item, key).and_then(|v| hex_id(&v, width));
    Some(ProfilerPci {
        name,
        device_type: text(item, "sppci_device_type"),
        vendor_id: id("sppci_vendor-id", 4),
        device_id: id("sppci_device-id", 4),
        subsystem_vendor_id: id("sppci_subsystem-vendor-id", 4),
        subsystem_device_id: id("sppci_subsystem-id", 4),
        revision: id("sppci_revision-id", 2),
    })
}

fn items(value: &Value) -> Vec<&Value> {
    value
        .get("_items")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(s) => clean_text(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// "(0x1002)" inside "AMD (0x1002)".
fn hex_in_parentheses(text: &str) -> Option<String> {
    let start = text.find("(0x")? + 1;
    let end = text[start..].find(')')? + start;
    hex_id(&text[start..end], 4)
}

fn vendor_from_keyword(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let id = if words.contains(&"nvidia") {
        "10de"
    } else if words.contains(&"intel") {
        "8086"
    } else if words.contains(&"amd") || words.contains(&"ati") {
        "1002"
    } else {
        return None;
    };
    Some(id.to_string())
}

/// "16 GB", "1536 MB", "500,28 GB" → bytes (binary units, as macOS reports memory).
pub fn parse_size_bytes(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let number: f64 = parts.next()?.replace(',', ".").parse().ok()?;
    let unit = parts.next().unwrap_or("B").to_ascii_uppercase();
    let scale: u64 = match unit.as_str() {
        "TB" => 1 << 40,
        "GB" => 1 << 30,
        "MB" => 1 << 20,
        "KB" => 1 << 10,
        "B" => 1,
        _ => return None,
    };
    (number >= 0.0).then(|| (number * scale as f64).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE_SILICON: &str = r#"{
      "SPAudioDataType" : [ { "_items" : [
          { "_name" : "MacBook Pro Microphone", "coreaudio_device_transport" : "coreaudio_device_type_builtin" },
          { "_name" : "MacBook Pro Speakers", "coreaudio_device_transport" : "coreaudio_device_type_builtin" },
          { "_name" : "Phone Microphone", "coreaudio_device_transport" : "coreaudio_device_type_unknown" } ],
        "_name" : "coreaudio_device" } ],
      "SPBluetoothDataType" : [ { "controller_properties" : {
          "controller_chipset" : "BCM_4388", "controller_productID" : "0x4A13",
          "controller_transport" : "PCIe", "controller_vendorID" : "0x004C (Apple)" } } ],
      "SPDisplaysDataType" : [ { "_name" : "Apple M2 Pro", "spdisplays_vendor" : "sppci_vendor_Apple",
          "sppci_bus" : "spdisplays_builtin", "sppci_cores" : "19", "sppci_model" : "Apple M2 Pro" } ],
      "SPHardwareDataType" : [ { "_name" : "hardware_overview", "boot_rom_version" : "20457.1.29",
          "chip_type" : "Apple M2 Pro", "machine_model" : "Mac14,10", "machine_name" : "MacBook Pro",
          "number_processors" : "proc 12:0:8:4", "physical_memory" : "16 GB" } ],
      "SPNetworkDataType" : [
          { "_name" : "Wi-Fi", "hardware" : "AirPort", "interface" : "en0", "type" : "AirPort" },
          { "_name" : "USB 10/100/1000 LAN", "Ethernet" : { "MAC Address" : "5e:47:1f:70:6a:80" },
            "hardware" : "Ethernet", "interface" : "en7", "type" : "Ethernet" } ],
      "SPNVMeDataType" : [ { "_items" : [ { "_name" : "APPLE SSD AP0512Z", "device_model" : "APPLE SSD AP0512Z",
          "size_in_bytes" : 500277792768 } ], "_name" : "Apple SSD Controller" } ],
      "SPPCIDataType" : [ ],
      "SPSerialATADataType" : [ ],
      "SPUSBDataType" : [ ],
      "SPUSBHostDataType" : [ { "_name" : "USB 3.1 Bus", "Driver" : "AppleT8112USBXHCI",
          "USBKeyHardwareType" : "Built-in", "USBKeyLocationID" : "0x02000000" } ]
    }"#;

    const INTEL_HACKINTOSH: &str = r#"{
      "SPHardwareDataType" : [ { "machine_model" : "iMac19,1", "cpu_type" : "8-Core Intel Core i7",
          "boot_rom_version" : "1916.0.3.0.0", "physical_memory" : "32 GB" } ],
      "SPDisplaysDataType" : [
          { "_name" : "kHW_IntelUHDGraphics630Item", "spdisplays_device-id" : "0x3e98",
            "spdisplays_revision-id" : "0x0002", "spdisplays_vendor" : "sppci_vendor_intel",
            "spdisplays_vram_shared" : "1536 MB", "sppci_model" : "Intel UHD Graphics 630" },
          { "_name" : "Radeon RX 580", "spdisplays_device-id" : "0x67df", "spdisplays_vendor" : "sppci_vendor_amd",
            "spdisplays_vram" : "8 GB", "sppci_model" : "Radeon RX 580" } ],
      "SPSerialATADataType" : [ { "_name" : "Intel 300 Series Chipset", "_items" : [
          { "_name" : "Samsung SSD 860 EVO 500GB", "device_model" : "Samsung SSD 860 EVO 500GB",
            "size_in_bytes" : 500107862016 } ] } ],
      "SPUSBDataType" : [ { "_name" : "USB30Bus", "host_controller" : "AppleUSBXHCISPT",
          "pci_device" : "0xa36d ", "pci_vendor" : "0x8086 " } ],
      "SPBluetoothDataType" : [ { "local_device_title" : { "general_vendorID" : "0x05AC",
          "general_productID" : "0x828D", "general_chipset" : "20702B0" } } ],
      "SPPCIDataType" : [ { "_name" : "pci14e4,43a0", "sppci_device_type" : "sppci_network",
          "sppci_vendor-id" : "0x14e4", "sppci_device-id" : "0x43a0", "sppci_subsystem-vendor-id" : "0x106b",
          "sppci_subsystem-id" : "0x0117", "sppci_revision-id" : "0x0003" } ]
    }"#;

    #[test]
    fn apple_silicon_report() {
        let r = parse(APPLE_SILICON).unwrap();
        assert_eq!(r.machine_model.as_deref(), Some("Mac14,10"));
        assert_eq!(r.chip.as_deref(), Some("Apple M2 Pro"));
        assert_eq!(r.memory_bytes, Some(16 << 30));
        assert_eq!(r.gpus.len(), 1);
        assert_eq!(r.gpus[0].vendor_id, None);
        assert_eq!(r.network.len(), 2);
        assert_eq!(r.network[1].mac.as_deref(), Some("5e:47:1f:70:6a:80"));
        assert_eq!(r.bluetooth[0].vendor_id.as_deref(), Some("004c"));
        assert_eq!(r.bluetooth[0].product_id.as_deref(), Some("4a13"));
        assert_eq!(r.drives[0].size_bytes, Some(500_277_792_768));
        assert_eq!(r.drives[0].kind, "nvme");
        assert_eq!(r.usb_buses[0].driver.as_deref(), Some("AppleT8112USBXHCI"));
        assert_eq!(r.builtin_audio.len(), 2);
    }

    #[test]
    fn intel_report() {
        let r = parse(INTEL_HACKINTOSH).unwrap();
        assert_eq!(r.chip.as_deref(), Some("8-Core Intel Core i7"));
        assert_eq!(r.gpus[0].vendor_id.as_deref(), Some("8086"));
        assert_eq!(r.gpus[0].device_id.as_deref(), Some("3e98"));
        assert_eq!(r.gpus[0].vram_mb, Some(1536));
        assert_eq!(r.gpus[1].vendor_id.as_deref(), Some("1002"));
        assert_eq!(r.gpus[1].vram_mb, Some(8192));
        assert_eq!(r.drives[0].kind, "sata");
        assert_eq!(
            r.drives[0].controller.as_deref(),
            Some("Intel 300 Series Chipset")
        );
        assert_eq!(r.usb_buses[0].device_id.as_deref(), Some("a36d"));
        assert_eq!(r.bluetooth[0].vendor_id.as_deref(), Some("05ac"));
        assert_eq!(r.pci_cards[0].subsystem_vendor_id.as_deref(), Some("106b"));
    }

    #[test]
    fn tolerates_missing_and_garbage() {
        assert_eq!(parse("{}").unwrap(), ProfilerReport::default());
        assert!(parse("not json").is_err());
        assert_eq!(parse_size_bytes("1,5 GB"), Some(1_610_612_736));
        assert_eq!(
            vendor_from_keyword("NVIDIA Corporation").as_deref(),
            Some("10de")
        );
        assert_eq!(
            vendor_from_keyword("sppci_vendor_ati").as_deref(),
            Some("1002")
        );
        assert_eq!(parse_size_bytes("lots"), None);
    }
}
