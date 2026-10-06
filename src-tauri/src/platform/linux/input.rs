//! Keyboards, touchpads and other input devices: `/proc/bus/input/devices`
//! for bound devices, serio `firmware_id` for PS/2 ids and ACPI-enumerated
//! I2C clients for HID-over-I2C devices that have no driver yet.

use crate::contracts::{InputDevice, InputKind};
use crate::platform::common::input_vendor;

use super::parse::{
    hid_from_i2c_name, parse_input_devices, serio_firmware_ids, ProcInputDevice, BUS_BLUETOOTH,
    BUS_I2C, BUS_I8042, BUS_RMI, BUS_USB,
};
use super::sysfs::SysRoot;

const PROP_DIRECT: u64 = 1 << 1;
const PROP_BUTTONPAD: u64 = 1 << 2;
const EV_REP: u64 = 1 << 20;

pub fn collect(sys: &SysRoot) -> Vec<InputDevice> {
    let text = sys
        .read_bytes("/proc/bus/input/devices")
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    let mut out: Vec<InputDevice> = Vec::new();
    for device in parse_input_devices(&text.unwrap_or_default()) {
        let bus = match device.bus {
            BUS_I8042 => "ps2",
            BUS_I2C => "i2c",
            BUS_RMI => "smbus",
            BUS_USB => "usb",
            BUS_BLUETOOTH => "bluetooth",
            _ => continue,
        };
        let kind = classify(&device);
        if kind == InputKind::Other && matches!(bus, "usb" | "bluetooth") {
            continue;
        }
        let hardware_id = match bus {
            "i2c" => hid_from_i2c_name(&device.sysfs).or_else(|| hid_from_i2c_name(&device.phys)),
            "ps2" => serio_hardware_id(sys, &device.phys),
            _ => None,
        };
        if out.iter().any(|d| d.name == device.name && d.kind == kind) {
            continue;
        }
        // An I2C HID device registers one input device per collection
        // ("… Mouse" and "… Touchpad"): keep one entry with the most
        // specific kind.
        if let Some(existing) = out.iter_mut().find(|d| {
            bus == "i2c" && d.bus == "i2c" && hardware_id.is_some() && d.hardware_id == hardware_id
        }) {
            if rank(kind) > rank(existing.kind) {
                existing.kind = kind;
                existing.name = device.name;
            }
            continue;
        }
        out.push(InputDevice {
            vendor: input_vendor(hardware_id.as_deref(), &device.name),
            name: device.name,
            kind,
            bus: bus.into(),
            hardware_id,
        });
    }

    // HID-over-I2C clients enumerated from ACPI but without an input device.
    for client in sys.list("/sys/bus/i2c/devices") {
        let base = format!("/sys/bus/i2c/devices/{client}");
        let Some(hid) = sys.read(&format!("{base}/firmware_node/hid")) else {
            continue;
        };
        let modalias = sys
            .read(&format!("{base}/modalias"))
            .unwrap_or_default()
            .to_ascii_uppercase();
        let hid_over_i2c = modalias.contains("PNP0C50") || modalias.contains("ACPI0C50");
        let vendor = input_vendor(Some(&hid), "");
        if !(hid_over_i2c || vendor.is_some())
            || out
                .iter()
                .any(|d| d.hardware_id.as_deref() == Some(hid.as_str()))
        {
            continue;
        }
        out.push(InputDevice {
            name: client,
            kind: InputKind::Other,
            bus: "i2c".into(),
            hardware_id: Some(hid),
            vendor,
        });
    }
    out
}

fn classify(device: &ProcInputDevice) -> InputKind {
    let name = device.name.to_ascii_lowercase();
    let has = |needle: &str| name.contains(needle);
    if has("touchpad")
        || has("trackpad")
        || has("touch pad")
        || has("glidepoint")
        || device.props & PROP_BUTTONPAD != 0
    {
        InputKind::Touchpad
    } else if has("touchscreen") || has("touch screen") || device.props & PROP_DIRECT != 0 {
        InputKind::Touchscreen
    } else if has("mouse")
        || has("trackpoint")
        || device.handlers.iter().any(|h| h.starts_with("mouse"))
    {
        InputKind::Mouse
    } else if device.handlers.iter().any(|h| h == "kbd") && device.ev & EV_REP != 0 {
        InputKind::Keyboard
    } else {
        InputKind::Other
    }
}

fn rank(kind: InputKind) -> u8 {
    match kind {
        InputKind::Touchpad => 4,
        InputKind::Touchscreen => 3,
        InputKind::Keyboard => 2,
        InputKind::Mouse => 1,
        InputKind::Other => 0,
    }
}

/// "isa0060/serio1/input0" → first PnP id of `/sys/bus/serio/devices/serio1/firmware_id`.
/// Pass-through ports ("synaptics-pt/serio0/input0", a TrackPoint behind
/// the touchpad) number their ports locally and carry no PnP id.
fn serio_hardware_id(sys: &SysRoot, phys: &str) -> Option<String> {
    let serio = phys.strip_prefix("isa0060/")?.split('/').next()?;
    if !serio.starts_with("serio") {
        return None;
    }
    let ids =
        serio_firmware_ids(&sys.read(&format!("/sys/bus/serio/devices/{serio}/firmware_id"))?);
    ids.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::super::sysfs::fixture::Tree;
    use super::*;

    const DEVICES: &str = r#"I: Bus=0011 Vendor=0001 Product=0001 Version=ab41
N: Name="AT Translated Set 2 keyboard"
P: Phys=isa0060/serio0/input0
S: Sysfs=/devices/platform/i8042/serio0/input/input0
H: Handlers=sysrq kbd event0 leds
B: PROP=0
B: EV=120013

I: Bus=0011 Vendor=0002 Product=0007 Version=01b1
N: Name="SynPS/2 Synaptics TouchPad"
P: Phys=isa0060/serio1/input0
S: Sysfs=/devices/platform/i8042/serio1/input/input5
H: Handlers=mouse0 event5
B: PROP=5
B: EV=b

I: Bus=0018 Vendor=04f3 Product=3195 Version=0100
N: Name="ELAN0662:00 04F3:3195 Mouse"
P: Phys=i2c-ELAN0662:00
S: Sysfs=/devices/pci0000:00/0000:00:15.0/i2c_designware.0/i2c-0/i2c-ELAN0662:00/0018:04F3:3195.0001/input/input8
H: Handlers=mouse2 event8
B: PROP=0
B: EV=17

I: Bus=0011 Vendor=0002 Product=000a Version=0000
N: Name="TPPS/2 IBM TrackPoint"
P: Phys=synaptics-pt/serio0/input0
S: Sysfs=/devices/platform/i8042/serio1/serio2/input/input6
H: Handlers=mouse3 event6
B: PROP=21
B: EV=7

I: Bus=0018 Vendor=04f3 Product=3195 Version=0100
N: Name="ELAN0662:00 04F3:3195 Touchpad"
P: Phys=i2c-ELAN0662:00
S: Sysfs=/devices/pci0000:00/0000:00:15.0/i2c_designware.0/i2c-0/i2c-ELAN0662:00/0018:04F3:3195.0001/input/input9
H: Handlers=mouse1 event9
B: PROP=5
B: EV=1b

I: Bus=0003 Vendor=046d Product=c52b Version=0111
N: Name="Logitech USB Receiver Consumer Control"
P: Phys=usb-0000:00:14.0-2/input2
H: Handlers=kbd event11
B: PROP=0
B: EV=13

I: Bus=0003 Vendor=046d Product=c52b Version=0111
N: Name="Logitech USB Receiver Mouse"
P: Phys=usb-0000:00:14.0-2/input2
H: Handlers=mouse2 event12
B: PROP=0
B: EV=17

I: Bus=0019 Vendor=0000 Product=0001 Version=0000
N: Name="Power Button"
P: Phys=LNXPWRBN/button/input0
H: Handlers=kbd event3
B: PROP=0
B: EV=3
"#;

    #[test]
    fn classifies_laptop_input() {
        let t = Tree::new("input");
        t.file("/proc/bus/input/devices", DEVICES)
            .file(
                "/sys/bus/serio/devices/serio0/firmware_id",
                "PNP: PNP0303\n",
            )
            .file(
                "/sys/bus/serio/devices/serio1/firmware_id",
                "PNP: SYN3286 PNP0f13\n",
            )
            .file(
                "/sys/bus/i2c/devices/i2c-ELAN0662:00/firmware_node/hid",
                "ELAN0662",
            )
            .file(
                "/sys/bus/i2c/devices/i2c-ELAN0662:00/modalias",
                "acpi:ELAN0662:PNP0C50:",
            )
            .file(
                "/sys/bus/i2c/devices/i2c-SYNA7DB5:00/firmware_node/hid",
                "SYNA7DB5",
            )
            .file(
                "/sys/bus/i2c/devices/i2c-SYNA7DB5:00/modalias",
                "acpi:SYNA7DB5:PNP0C50:",
            )
            .file(
                "/sys/bus/i2c/devices/i2c-10EC5682:00/firmware_node/hid",
                "10EC5682",
            );
        let devices = collect(&SysRoot::new(&t.root));
        assert_eq!(devices[2].name, "ELAN0662:00 04F3:3195 Touchpad");
        let summary: Vec<(InputKind, &str, Option<&str>, Option<&str>)> = devices
            .iter()
            .map(|d| {
                (
                    d.kind,
                    d.bus.as_str(),
                    d.hardware_id.as_deref(),
                    d.vendor.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (InputKind::Keyboard, "ps2", Some("PNP0303"), None),
                (
                    InputKind::Touchpad,
                    "ps2",
                    Some("SYN3286"),
                    Some("synaptics")
                ),
                (InputKind::Touchpad, "i2c", Some("ELAN0662"), Some("elan")),
                (InputKind::Mouse, "ps2", None, None),
                (InputKind::Mouse, "usb", None, None),
                (InputKind::Other, "i2c", Some("SYNA7DB5"), Some("synaptics")),
            ]
        );
    }

    #[test]
    fn missing_procfs_gives_empty_list() {
        let t = Tree::new("input-empty");
        assert!(collect(&SysRoot::new(&t.root)).is_empty());
    }
}
