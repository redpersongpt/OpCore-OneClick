//! Facts read in-process without PowerShell: CPUID, processor topology,
//! installed memory, firmware mode, Secure Boot and display memory sizes.
//! These keep the CPU, memory and firmware sections working when WMI is
//! broken.

use std::collections::HashMap;

use windows_sys::Win32::System::SystemInformation::{
    FirmwareTypeBios, FirmwareTypeUefi, GetFirmwareType, GetLogicalProcessorInformationEx,
    GetPhysicallyInstalledSystemMemory, RelationAll, FIRMWARE_TYPE,
};

use crate::platform::common::read_cpuid;

use super::inventory::NativeFacts;
use super::raw::{parse_processor_records, ProcessorTopology};
use super::registry::{local_machine_number, RegKey};

const SECURE_BOOT_STATE: &str = r"SYSTEM\CurrentControlSet\Control\SecureBoot\State";
const CONTROL: &str = r"SYSTEM\CurrentControlSet\Control";
const CLASS_ROOT: &str = r"SYSTEM\CurrentControlSet\Control\Class";

pub fn collect() -> NativeFacts {
    let uefi = firmware_is_uefi();
    NativeFacts {
        cpuid: read_cpuid(),
        topology: processor_topology(),
        installed_memory_kb: installed_memory_kb(),
        uefi,
        secure_boot: if uefi == Some(true) {
            secure_boot()
        } else {
            None
        },
        ..Default::default()
    }
}

fn processor_topology() -> Option<ProcessorTopology> {
    let mut len = 0u32;
    // SAFETY: a null buffer asks for the required length (the call fails
    // with ERROR_INSUFFICIENT_BUFFER and sets `len`).
    unsafe { GetLogicalProcessorInformationEx(RelationAll, std::ptr::null_mut(), &mut len) };
    if len == 0 || len > 16 * 1024 * 1024 {
        return None;
    }
    // u64 storage keeps the records 8-byte aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let mut written = (buf.len() * 8) as u32;
    // SAFETY: `buf` is valid for `written` bytes.
    let ok = unsafe {
        GetLogicalProcessorInformationEx(RelationAll, buf.as_mut_ptr().cast(), &mut written)
    };
    if ok == 0 {
        return None;
    }
    let bytes: Vec<u8> = buf
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take(written as usize)
        .collect();
    Some(parse_processor_records(&bytes)).filter(|t| t.cores > 0)
}

fn installed_memory_kb() -> Option<u64> {
    let mut kb = 0u64;
    // SAFETY: `kb` is a valid output location.
    let ok = unsafe { GetPhysicallyInstalledSystemMemory(&mut kb) };
    (ok != 0 && kb > 0).then_some(kb)
}

fn firmware_is_uefi() -> Option<bool> {
    let mut kind: FIRMWARE_TYPE = 0;
    // SAFETY: `kind` is a valid output location.
    let ok = unsafe { GetFirmwareType(&mut kind) };
    if ok != 0 && (kind == FirmwareTypeUefi || kind == FirmwareTypeBios) {
        return Some(kind == FirmwareTypeUefi);
    }
    match local_machine_number(CONTROL, "PEFirmwareType") {
        Some(2) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

fn secure_boot() -> Option<bool> {
    local_machine_number(SECURE_BOOT_STATE, "UEFISecureBootEnabled").map(|v| v != 0)
}

/// Dedicated video memory per display driver key ("{4d36e968-…}\\0001"),
/// from `HardwareInformation.qwMemorySize` (64-bit) or `.MemorySize`.
pub fn vram_sizes(driver_keys: &[String]) -> HashMap<String, u64> {
    let mut sizes = HashMap::new();
    for key in driver_keys {
        let Some(reg) = RegKey::local_machine(&format!(r"{CLASS_ROOT}\{key}")) else {
            continue;
        };
        let bytes = reg
            .number("HardwareInformation.qwMemorySize")
            .or_else(|| reg.number("HardwareInformation.MemorySize"))
            .filter(|b| *b > 0);
        if let Some(bytes) = bytes {
            sizes.insert(key.to_ascii_uppercase(), bytes);
        }
    }
    sizes
}
