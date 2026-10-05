//! Network, Bluetooth, input and storage device knowledge: which kext (if any)
//! drives a given PCI/USB id under which macOS release.

use super::model::{InputBus, MacOsVersion, ProfileNic, ProfileStorage, StorageKind, TouchpadVendor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EthernetDriver {
    /// Intel 82578..I219 — IntelMausi (or Mieze IntelMausiEthernet with AppleVTD).
    IntelMausi,
    /// Intel I211 — SmallTreeIntel82576 (10.15-11) / AppleIGB (12+).
    IntelI211,
    /// Intel I225/I226 — AppleIGC kext, or native AppleIntelI210 with a device-id spoof.
    IntelI225,
    /// Intel X520/X540/X550/82598 10GbE — IntelLucy.
    IntelLucy,
    /// Intel I210/I350/X540 native (AppleIntelI210Ethernet / Intel10GbE).
    NativeIntel,
    /// Killer E220x/E2400/E2500, Atheros AR816x/AR817x — AtherosE2200Ethernet.
    AtherosE2200,
    /// Realtek RTL8111/8168 — RealtekRTL8111 (2.4.2 on AMD: no AppleVTD).
    RealtekRtl8111,
    /// Realtek RTL8125/8126 — RTL812xLucy (LucyRTL8125Ethernet fallback).
    RealtekRtl8125,
    /// Realtek RTL8100/8101 — RealtekRTL8100.
    RealtekRtl8100,
    /// Aquantia AQC107/113 — native AppleEthernetAquantiaAqtion.
    NativeAquantia,
    /// Broadcom BCM57xx — native AppleBCM5701Ethernet (sometimes needs spoof).
    NativeBroadcom,
    Unsupported,
}

/// Choose the Ethernet driver for a NIC by PCI id.
pub fn ethernet_driver(nic: &ProfileNic) -> EthernetDriver {
    todo!("ethernet_driver {:?}:{:?}", nic.vendor_id, nic.device_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiDriver {
    /// Intel Wi-Fi supported by OpenIntelWireless (itlwm / AirportItlwm).
    IntelItlwm,
    /// Broadcom that works natively (+AirportBrcmFixup) up to `native_max`,
    /// and with OCLP root patches after that.
    Broadcom { native_max: MacOsVersion, fixup: bool },
    /// Atheros AR9xxx — native up to Big Sur, OCLP root patch after.
    AtherosLegacy,
    /// Realtek rtw88 PCIe (RTL8822BE/CE, 8821CE) — experimental rtw88.kext.
    RealtekRtw88,
    /// USB dongles and everything else — no working driver.
    Unsupported,
}

pub fn wifi_driver(nic: &ProfileNic) -> WifiDriver {
    todo!("wifi_driver {:?}:{:?}", nic.vendor_id, nic.device_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BluetoothDriver {
    /// Intel — IntelBluetoothFirmware + IntelBTPatcher (+ BlueToolFixup 12+).
    IntelBluetooth,
    /// Broadcom needing firmware upload — BrcmPatchRAM3 + BrcmFirmwareData (+ BlueToolFixup 12+).
    BroadcomPatchRam,
    /// Apple/Broadcom modules with native firmware (BCM94360/20702 Apple ids) — BlueToolFixup only on 12+.
    BroadcomNative,
    /// Realtek — RealtekBluetoothFirmware (experimental).
    Realtek,
    Unsupported,
}

pub fn bluetooth_driver(nic: &ProfileNic) -> BluetoothDriver {
    todo!("bluetooth_driver {:?}:{:?}", nic.vendor_id, nic.device_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchpadDriver {
    /// VoodooPS2Trackpad (+ VoodooInput).
    Ps2,
    /// VoodooI2C + VoodooI2CHID (precision touchpads, most ELAN/Synaptics HID).
    I2cHid,
    /// VoodooRMI over I2C (Synaptics RMI4).
    RmiI2c,
    /// VoodooRMI + VoodooSMBus (Synaptics SMBus).
    RmiSmbus,
    /// VoodooSMBus (ELAN SMBus) via VoodooPS2/ELAN.
    ElanSmbus,
    /// AlpsHID on VoodooI2C.
    AlpsHid,
    None,
}

pub fn touchpad_driver(bus: Option<InputBus>, vendor: Option<TouchpadVendor>, hid: Option<&str>) -> TouchpadDriver {
    todo!("touchpad_driver {bus:?} {vendor:?} {hid:?}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAdvice {
    /// The drive cannot be used by macOS (Samsung PM981/PM991, Micron 2200S,
    /// Intel 600p early firmware, some Hynix/Kioxia) or the controller is RAID/VMD.
    pub problematic: bool,
    /// NVMeFix.kext recommended (power management on non-Apple NVMe).
    pub nvmefix: bool,
    /// SATA controller needs CtlnaAHCIPort.kext.
    pub ctlna_ahci: bool,
    pub notes: Vec<String>,
}

pub fn storage_advice(drive: &ProfileStorage) -> StorageAdvice {
    let _ = StorageKind::Nvme;
    todo!("storage_advice {}", drive.name)
}
