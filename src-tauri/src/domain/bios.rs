//! Firmware (BIOS/UEFI) settings the user must apply before booting OpenCore.

use super::model::{BiosSetting, HardwareProfile, MacOsVersion};

/// Platform-specific BIOS checklist (Dortania "Intel BIOS settings" / "AMD BIOS
/// settings"): Fast Boot, Secure Boot, CSM, VT-d (or DisableIoMapper), CFG Lock,
/// Above 4G Decoding, Resizable BAR, XHCI hand-off, SATA AHCI, DVMT
/// pre-allocated, Intel SGX, Platform Trust, Serial port, SVM/IOMMU on AMD,
/// VMD off, iGPU multi-monitor for headless setups, Thunderbolt, etc.
pub fn recommended_settings(profile: &HardwareProfile, target: MacOsVersion) -> Vec<BiosSetting> {
    todo!("recommended_settings {} {target:?}", profile.cpu.name)
}
