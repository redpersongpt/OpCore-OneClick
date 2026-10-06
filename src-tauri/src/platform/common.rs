//! Platform-independent helpers shared by the scanners and disk code
//! (unit-testable on every host): PnP / location-path / sysfs parsing, CPUID
//! decoding, VM and vendor hints, ACPI dump files and child-process deadlines.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::tasks::cancellation::CancellationToken;

// ─── Hex ids ────────────────────────────────────────────────────────────────

/// Normalise a hex id ("0x8086", "8086", "10EC") to lowercase without prefix,
/// left-padded to `width` digits. Returns `None` for empty or non-hex input.
pub fn hex_id(value: &str, width: usize) -> Option<String> {
    let v = value.trim();
    let v = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);
    if v.is_empty() || v.len() > 16 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!(
        "{:0>width$}",
        v.to_ascii_lowercase(),
        width = width
    ))
}

/// Parse "0x10ec0897" / "10EC0897" as a number.
pub fn parse_hex_u32(value: &str) -> Option<u32> {
    let v = value.trim();
    let v = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);
    u32::from_str_radix(v, 16).ok()
}

/// Lowercase 4-digit hex of a 16-bit id.
pub fn hex16(value: u32) -> String {
    format!("{:04x}", value & 0xffff)
}

// ─── Windows PnP ids ────────────────────────────────────────────────────────

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

/// `SUBSYS_` byte order differs by bus: PCI writes the subsystem *device*
/// first (`SUBSYS_DDDDVVVV`), HD Audio / Intel SST write the *vendor* first
/// (`SUBSYS_VVVVDDDD`). Both are normalised to separate vendor/device fields.
pub fn parse_pnp_id(id: &str) -> PnpIds {
    let mut parts = id.trim().split('\\');
    let bus = parts.next().unwrap_or_default().trim().to_ascii_uppercase();
    let body = parts.next().unwrap_or_default();
    let vendor_first_subsys = matches!(bus.as_str(), "HDAUDIO" | "INTELAUDIO");
    let mut ids = PnpIds {
        bus,
        ..Default::default()
    };
    for token in body.split('&') {
        let Some((key, value)) = token.split_once('_') else {
            continue;
        };
        match key.to_ascii_uppercase().as_str() {
            "VEN" | "VID" if value.len() == 4 => ids.vendor_id = hex_id(value, 4),
            "DEV" | "PID" if value.len() == 4 => ids.device_id = hex_id(value, 4),
            "SUBSYS" if value.len() == 8 => {
                if let Some(v) = hex_id(value, 8) {
                    let (hi, lo) = v.split_at(4);
                    let (vendor, device) = if vendor_first_subsys {
                        (hi, lo)
                    } else {
                        (lo, hi)
                    };
                    ids.subsystem_vendor_id = Some(vendor.to_string());
                    ids.subsystem_device_id = Some(device.to_string());
                }
            }
            "REV" => ids.revision = hex_id(value, 2),
            _ => {}
        }
    }
    ids
}

/// ACPI hardware id of an ACPI-enumerated PnP device:
/// "ACPI\\SYNA2393\\4&2c..." → "SYNA2393", "ACPI\\VEN_ELAN&DEV_0662\\..." → "ELAN0662",
/// "*PNP0303" → "PNP0303".
pub fn acpi_hid_from_pnp_id(id: &str) -> Option<String> {
    let trimmed = id.trim();
    if let Some(star) = trimmed.strip_prefix('*') {
        return valid_acpi_hid(star);
    }
    let mut parts = trimmed.split('\\');
    if !parts.next()?.eq_ignore_ascii_case("ACPI") {
        return None;
    }
    let body = parts.next()?;
    let upper = body.to_ascii_uppercase();
    if let Some(rest) = upper.strip_prefix("VEN_") {
        let (ven, dev) = rest.split_once("&DEV_")?;
        let dev = dev.split('&').next().unwrap_or_default();
        return valid_acpi_hid(&format!("{ven}{dev}"));
    }
    valid_acpi_hid(&upper)
}

fn valid_acpi_hid(value: &str) -> Option<String> {
    let v = value.trim().to_ascii_uppercase();
    let ok = (4..=9).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(v)
}

/// Extract a PCI class from a Windows compatible id ("PCI\\VEN_8086&CC_0C0330").
pub fn pci_class_from_compatible_id(id: &str) -> Option<PciClass> {
    let upper = id.to_ascii_uppercase();
    let start = upper.find("CC_")? + 3;
    let digits: String = upper[start..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    PciClass::from_hex(&digits)
}

/// PCI class code split into base class, subclass and programming interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciClass {
    pub base: u8,
    pub sub: u8,
    pub prog_if: Option<u8>,
}

impl PciClass {
    /// From a 24-bit class value (0xCCSSPP).
    pub fn from_u32(value: u32) -> Self {
        Self {
            base: (value >> 16) as u8,
            sub: (value >> 8) as u8,
            prog_if: Some(value as u8),
        }
    }

    /// From hex text: "0x040300" / "040300" (with prog-if) or "0403" (without).
    pub fn from_hex(text: &str) -> Option<Self> {
        let t = text.trim();
        let t = t
            .strip_prefix("0x")
            .or_else(|| t.strip_prefix("0X"))
            .unwrap_or(t);
        let value = u32::from_str_radix(t, 16).ok()?;
        match t.len() {
            6 => Some(Self::from_u32(value)),
            4 => Some(Self {
                base: (value >> 8) as u8,
                sub: value as u8,
                prog_if: None,
            }),
            _ => None,
        }
    }

    pub fn is(&self, base: u8, sub: u8) -> bool {
        self.base == base && self.sub == sub
    }
}

/// USB host controller flavour from the PCI programming interface (class
/// 0x0C03), falling back to the controller name.
pub fn usb_controller_kind(prog_if: Option<u8>, name: &str) -> &'static str {
    match prog_if {
        Some(0x30) => return "xhci",
        Some(0x20) => return "ehci",
        Some(0x10) => return "ohci",
        Some(0x00) => return "uhci",
        Some(_) => return "other",
        None => {}
    }
    let n = name.to_ascii_lowercase();
    if n.contains("xhci") || n.contains("extensible") || n.contains("usb 3") {
        "xhci"
    } else if n.contains("ehci") || n.contains("enhanced") {
        "ehci"
    } else if n.contains("ohci") || n.contains("open host") {
        "ohci"
    } else if n.contains("uhci") || n.contains("universal host") {
        "uhci"
    } else {
        "other"
    }
}

/// Storage controller flavour from a PCI class (mass-storage base class 0x01).
pub fn storage_kind_from_class(class: PciClass) -> Option<&'static str> {
    if class.base != 0x01 {
        return None;
    }
    Some(match class.sub {
        0x08 => "nvme",
        0x06 | 0x01 | 0x05 => "sata",
        0x04 => "raid",
        _ => "other",
    })
}

// ─── Device paths ───────────────────────────────────────────────────────────

/// Convert a Windows `DEVPKEY_Device_LocationPaths` entry
/// ("PCIROOT(0)#PCI(1F03)" / "PCIROOT(0)#PCI(0100)#PCI(0000)") to an OpenCore
/// device path ("PciRoot(0x0)/Pci(0x1f,0x3)").
pub fn location_path_to_device_path(location: &str) -> Option<String> {
    let mut segments = location.trim().split('#');
    let root = call_argument(segments.next()?, "PCIROOT")?;
    let uid = u32::from_str_radix(root, 16).ok()?;
    let mut path = format!("PciRoot({uid:#x})");
    let mut nodes = 0;
    for segment in segments {
        let arg = call_argument(segment, "PCI")?;
        if arg.len() != 4 || !arg.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let device = u8::from_str_radix(arg.get(..2)?, 16).ok()?;
        let function = u8::from_str_radix(arg.get(2..)?, 16).ok()?;
        if device > 0x1f || function > 7 {
            return None;
        }
        path.push_str(&format!("/Pci({device:#x},{function:#x})"));
        nodes += 1;
    }
    (nodes > 0).then_some(path)
}

/// Convert an ACPI location path ("ACPI(_SB_)#ACPI(PCI0)#ACPI(HDAS)") to an
/// ACPI path ("\\_SB.PCI0.HDAS").
pub fn location_path_to_acpi_path(location: &str) -> Option<String> {
    let names = location
        .trim()
        .split('#')
        .map(|segment| call_argument(segment, "ACPI"))
        .collect::<Option<Vec<_>>>()?;
    if names.is_empty() {
        return None;
    }
    Some(normalize_acpi_path(&names.join(".")))
}

/// Canonical ACPI path: leading backslash, dot separators, trailing `_`
/// padding removed ("\\_SB_.PCI0.EC__" → "\\_SB.PCI0.EC").
pub fn normalize_acpi_path(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches('\\');
    let segments: Vec<&str> = trimmed
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let t = s.trim_end_matches('_');
            if t.is_empty() {
                s
            } else {
                t
            }
        })
        .collect();
    format!("\\{}", segments.join("."))
}

fn call_argument<'a>(segment: &'a str, name: &str) -> Option<&'a str> {
    let s = segment.trim();
    if !s.get(..name.len())?.eq_ignore_ascii_case(name) {
        return None;
    }
    s.get(name.len()..)?.strip_prefix('(')?.strip_suffix(')')
}

/// Convert a Linux sysfs PCI device path
/// ("/sys/devices/pci0000:00/0000:00:01.0/0000:01:00.0") to an OpenCore device path.
/// The PciRoot UID is taken from the domain for root bus 00; other roots need
/// the ACPI `_UID` (see [`sysfs_to_device_path_with_root_uid`]).
pub fn sysfs_to_device_path(sysfs: &str) -> Option<String> {
    sysfs_to_device_path_with_root_uid(sysfs, None)
}

/// Like [`sysfs_to_device_path`] with the host bridge `_UID` read from
/// `/sys/devices/pciDDDD:BB/firmware_node/uid`.
pub fn sysfs_to_device_path_with_root_uid(sysfs: &str, root_uid: Option<u32>) -> Option<String> {
    let mut segments = sysfs.split('/').filter(|s| !s.is_empty());
    let (domain, bus) = segments.by_ref().find_map(parse_pci_root_segment)?;
    // Intel VMD exposes its children in a synthetic domain (>= 0x10000) that
    // has no firmware device path.
    if domain >= 0x1_0000 {
        return None;
    }
    let uid = match root_uid {
        Some(uid) => uid,
        None if bus == 0 => domain,
        None => return None,
    };
    let mut path = format!("PciRoot({uid:#x})");
    let mut nodes = 0;
    for segment in segments {
        if parse_pci_root_segment(segment).is_some() {
            // A nested root (VMD) below this path: not reachable from firmware.
            return None;
        }
        let Some((_, _, device, function)) = parse_bdf(segment) else {
            break;
        };
        path.push_str(&format!("/Pci({device:#x},{function:#x})"));
        nodes += 1;
    }
    (nodes > 0).then_some(path)
}

/// "pci0000:00" → (domain, bus).
pub fn parse_pci_root_segment(segment: &str) -> Option<(u32, u32)> {
    let rest = segment.strip_prefix("pci")?;
    let (domain, bus) = rest.split_once(':')?;
    Some((
        u32::from_str_radix(domain, 16).ok()?,
        u32::from_str_radix(bus, 16).ok()?,
    ))
}

/// "0000:00:1f.3" → (domain, bus, device, function).
pub fn parse_bdf(segment: &str) -> Option<(u32, u8, u8, u8)> {
    let mut parts = segment.splitn(3, ':');
    let domain = parts.next()?;
    let bus = parts.next()?;
    let (device, function) = parts.next()?.split_once('.')?;
    let hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());
    if domain.len() < 4
        || bus.len() != 2
        || device.len() != 2
        || function.len() != 1
        || ![domain, bus, device, function].into_iter().all(hex)
    {
        return None;
    }
    let device = u8::from_str_radix(device, 16).ok()?;
    let function = u8::from_str_radix(function, 16).ok()?;
    if device > 0x1f || function > 7 {
        return None;
    }
    Some((
        u32::from_str_radix(domain, 16).ok()?,
        u8::from_str_radix(bus, 16).ok()?,
        device,
        function,
    ))
}

// ─── CPUID ──────────────────────────────────────────────────────────────────

/// Raw CPUID facts of the CPU the app runs on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuidInfo {
    /// "GenuineIntel", "AuthenticAMD", ...
    pub vendor: String,
    /// Brand string from leaves 0x80000002-4 (whitespace collapsed).
    pub brand: String,
    /// Display family / model (extended fields folded in) and stepping.
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    /// Feature flags of interest, see [`decode_features`].
    pub features: Vec<String>,
    /// Leaf 0x40000000 signature when the hypervisor bit is set ("KVMKVMKVM").
    pub hypervisor_signature: Option<String>,
    /// Hyper-V reports this OS as the root partition (bare metal with VBS),
    /// i.e. the hypervisor bit does not mean "virtual machine".
    pub hyperv_root_partition: bool,
}

/// Registers that carry the feature bits [`decode_features`] looks at.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuidFeatureRegs {
    pub leaf1_ecx: u32,
    pub leaf7_ebx: u32,
    pub leaf7_edx: u32,
    pub ext1_ecx: u32,
}

/// Leaf 1 EAX → (display family, display model, stepping).
pub fn decode_signature(eax: u32) -> (u32, u32, u32) {
    let base_family = (eax >> 8) & 0xf;
    let base_model = (eax >> 4) & 0xf;
    let family = if base_family == 0xf {
        base_family + ((eax >> 20) & 0xff)
    } else {
        base_family
    };
    let model = if base_family == 0x6 || base_family == 0xf {
        (((eax >> 16) & 0xf) << 4) | base_model
    } else {
        base_model
    };
    (family, model, eax & 0xf)
}

/// Lowercase flags: sse3, ssse3, sse4_1, sse4_2, sse4a, avx, avx2, avx512f,
/// vmx, svm, hybrid (Intel P/E cores), hypervisor.
pub fn decode_features(regs: &CpuidFeatureRegs) -> Vec<String> {
    let bit = |reg: u32, n: u32| (reg >> n) & 1 == 1;
    let table: [(&str, bool); 13] = [
        ("sse3", bit(regs.leaf1_ecx, 0)),
        ("ssse3", bit(regs.leaf1_ecx, 9)),
        ("sse4_1", bit(regs.leaf1_ecx, 19)),
        ("sse4_2", bit(regs.leaf1_ecx, 20)),
        ("sse4a", bit(regs.ext1_ecx, 6)),
        ("avx", bit(regs.leaf1_ecx, 28)),
        ("avx2", bit(regs.leaf7_ebx, 5)),
        ("rdrand", bit(regs.leaf1_ecx, 30)),
        ("avx512f", bit(regs.leaf7_ebx, 16)),
        ("vmx", bit(regs.leaf1_ecx, 5)),
        ("svm", bit(regs.ext1_ecx, 2)),
        ("hybrid", bit(regs.leaf7_edx, 15)),
        ("hypervisor", bit(regs.leaf1_ecx, 31)),
    ];
    table
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| (*name).to_string())
        .collect()
}

/// Concatenate register bytes (little endian) into text, dropping NULs and
/// collapsing whitespace.
pub fn registers_to_text(registers: &[u32]) -> String {
    let bytes: Vec<u8> = registers
        .iter()
        .flat_map(|r| r.to_le_bytes())
        .filter(|b| *b != 0)
        .collect();
    String::from_utf8_lossy(&bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod cpuid_hw {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__cpuid_count, CpuidResult};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__cpuid_count, CpuidResult};

    // `__cpuid_count` is a safe function on newer toolchains.
    #[allow(unused_unsafe)]
    pub fn leaf(leaf: u32, sub_leaf: u32) -> CpuidResult {
        // SAFETY: CPUID is available on every x86_64 CPU and on every x86 CPU
        // that can run this binary; unknown leaves return zeros or the highest
        // basic leaf, which the callers guard against with the max-leaf checks.
        unsafe { __cpuid_count(leaf, sub_leaf) }
    }
}

/// Read CPUID in-process. `None` on non-x86 hosts.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub fn read_cpuid() -> Option<CpuidInfo> {
    use cpuid_hw::leaf;
    let l0 = leaf(0, 0);
    let vendor = registers_to_text(&[l0.ebx, l0.edx, l0.ecx]);
    if l0.eax < 1 {
        return None;
    }
    let l1 = leaf(1, 0);
    let (family, model, stepping) = decode_signature(l1.eax);
    let (leaf7_ebx, leaf7_edx) = if l0.eax >= 7 {
        let l7 = leaf(7, 0);
        (l7.ebx, l7.edx)
    } else {
        (0, 0)
    };
    let max_ext = leaf(0x8000_0000, 0).eax;
    let ext1_ecx = if max_ext >= 0x8000_0001 {
        leaf(0x8000_0001, 0).ecx
    } else {
        0
    };
    let brand = if max_ext >= 0x8000_0004 {
        let regs: Vec<u32> = (0x8000_0002u32..=0x8000_0004)
            .flat_map(|l| {
                let r = leaf(l, 0);
                [r.eax, r.ebx, r.ecx, r.edx]
            })
            .collect();
        registers_to_text(&regs)
    } else {
        String::new()
    };
    let features = decode_features(&CpuidFeatureRegs {
        leaf1_ecx: l1.ecx,
        leaf7_ebx,
        leaf7_edx,
        ext1_ecx,
    });
    let (hypervisor_signature, hyperv_root_partition) = if (l1.ecx >> 31) & 1 == 1 {
        let hv = leaf(0x4000_0000, 0);
        let signature = registers_to_text(&[hv.ebx, hv.ecx, hv.edx]);
        // Hyper-V: CPUID 0x40000003 EBX bit 0 (CreatePartitions) is only
        // granted to the root partition, i.e. the bare-metal OS.
        let root = signature == "Microsoft Hv"
            && hv.eax >= 0x4000_0003
            && leaf(0x4000_0003, 0).ebx & 1 == 1;
        ((!signature.is_empty()).then_some(signature), root)
    } else {
        (None, false)
    };
    Some(CpuidInfo {
        vendor,
        brand,
        family,
        model,
        stepping,
        features,
        hypervisor_signature,
        hyperv_root_partition,
    })
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
pub fn read_cpuid() -> Option<CpuidInfo> {
    None
}

/// Base clock from a brand string ("... CPU @ 3.60GHz" → 3600).
pub fn base_clock_from_brand(brand: &str) -> Option<u32> {
    let after = brand.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    let (number, scale) = match after.strip_suffix("ghz") {
        Some(n) => (n, 1000.0),
        None => (after.strip_suffix("mhz")?, 1.0),
    };
    let value: f64 = number.trim().parse().ok()?;
    let mhz = (value * scale).round();
    (mhz > 0.0 && mhz < 100_000.0).then_some(mhz as u32)
}

/// Parse a Windows `Win32_Processor.Description` ("Intel64 Family 6 Model 158
/// Stepping 10") into display family / model / stepping.
pub fn parse_processor_description(text: &str) -> Option<(u32, u32, u32)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let find = |key: &str| {
        words
            .windows(2)
            .find(|w| w[0].eq_ignore_ascii_case(key))
            .and_then(|w| w[1].parse::<u32>().ok())
    };
    Some((
        find("Family")?,
        find("Model")?,
        find("Stepping").unwrap_or(0),
    ))
}

// ─── Virtual machines ───────────────────────────────────────────────────────

/// Friendly hypervisor name from a CPUID 0x40000000 signature.
pub fn hypervisor_from_cpuid_signature(signature: &str) -> String {
    let s = signature.trim();
    let name = match s {
        "KVMKVMKVM" | "Linux KVM Hv" => "KVM",
        "VMwareVMware" => "VMware",
        "Microsoft Hv" => "Microsoft Hyper-V",
        "VBoxVBoxVBox" => "VirtualBox",
        "XenVMMXenVMM" => "Xen",
        "prl hyperv" | "lrpepyh vr" => "Parallels",
        "TCGTCGTCGTCG" => "QEMU",
        "bhyve bhyve" => "bhyve",
        "ACRNACRNACRN" => "ACRN",
        "Apple VZ" | "Apple VZ VZ" => "Apple Virtualization",
        _ => s,
    };
    name.to_string()
}

/// Hypervisor name from SMBIOS system manufacturer / product strings.
pub fn hypervisor_from_dmi(manufacturer: Option<&str>, product: Option<&str>) -> Option<String> {
    let m = manufacturer.unwrap_or_default().to_ascii_lowercase();
    let p = product.unwrap_or_default().to_ascii_lowercase();
    let any = |needle: &str| m.contains(needle) || p.contains(needle);
    let name = if any("vmware") {
        "VMware"
    } else if any("innotek") || any("virtualbox") {
        "VirtualBox"
    } else if any("parallels") {
        "Parallels"
    } else if m.contains("microsoft") && p.contains("virtual machine") {
        "Microsoft Hyper-V"
    } else if any("kvm") {
        "KVM"
    } else if any("qemu") {
        "QEMU"
    } else if m == "xen" || p.contains("hvm domu") {
        "Xen"
    } else if any("bochs") {
        "Bochs"
    } else if any("bhyve") {
        "bhyve"
    } else if p.starts_with("virtualmac") {
        "Apple Virtualization"
    } else {
        return None;
    };
    Some(name.to_string())
}

/// Combine CPUID and SMBIOS evidence. The Hyper-V root partition (Windows with
/// VBS / Hyper-V enabled on bare metal) is not treated as a VM.
pub fn resolve_hypervisor(
    cpuid: Option<&CpuidInfo>,
    manufacturer: Option<&str>,
    product: Option<&str>,
) -> Option<String> {
    if let Some(info) = cpuid {
        if let Some(signature) = &info.hypervisor_signature {
            if !info.hyperv_root_partition {
                return Some(hypervisor_from_cpuid_signature(signature));
            }
        }
    }
    hypervisor_from_dmi(manufacturer, product)
}

// ─── Vendor hints and text cleanup ──────────────────────────────────────────

/// Touchpad / touchscreen vendor from an ACPI hardware id or device name.
pub fn input_vendor(hardware_id: Option<&str>, name: &str) -> Option<String> {
    const PREFIXES: &[(&str, &str)] = &[
        ("SYN", "synaptics"),
        ("ELAN", "elan"),
        ("ETD", "elan"),
        ("ALP", "alps"),
        ("FTCS", "focaltech"),
        ("FTE", "focaltech"),
        ("GXTP", "goodix"),
        ("GDIX", "goodix"),
        ("CYAP", "cypress"),
        ("ATML", "atmel"),
        ("WCOM", "wacom"),
    ];
    const NAMES: &[(&str, &str)] = &[
        ("synaptics", "synaptics"),
        ("synps/2", "synaptics"),
        ("elan", "elan"),
        ("etps/2", "elan"),
        ("alps", "alps"),
        ("focaltech", "focaltech"),
        ("goodix", "goodix"),
        ("cypress", "cypress"),
        ("wacom", "wacom"),
    ];
    if let Some(hid) = hardware_id {
        let upper = hid.to_ascii_uppercase();
        if let Some((_, vendor)) = PREFIXES
            .iter()
            .find(|(prefix, _)| upper.starts_with(prefix))
        {
            return Some((*vendor).to_string());
        }
    }
    let lower = name.to_ascii_lowercase();
    NAMES
        .iter()
        .find(|(needle, _)| lower.contains(needle))
        .map(|(_, vendor)| (*vendor).to_string())
}

/// HD Audio codec vendors that only make HDMI/DisplayPort codecs
/// (Intel, AMD/ATI, NVIDIA).
pub fn is_hdmi_codec_vendor(vendor_id: &str) -> bool {
    matches!(
        vendor_id.to_ascii_lowercase().as_str(),
        "8086" | "1002" | "10de"
    )
}

/// Trim and drop empty strings and NULs.
pub fn clean_text(value: &str) -> Option<String> {
    let cleaned: String = value.chars().filter(|c| *c != '\0').collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

/// Like [`clean_text`] but also drops SMBIOS placeholder strings
/// ("To be filled by O.E.M.", "System Product Name", "Default string", ...).
pub fn clean_dmi(value: &str) -> Option<String> {
    const PLACEHOLDERS: &[&str] = &[
        "to be filled by o.e.m.",
        "to be filled by oem",
        "o.e.m.",
        "oem",
        "system manufacturer",
        "system product name",
        "system version",
        "base board product name",
        "default string",
        "not applicable",
        "not specified",
        "not available",
        "n/a",
        "na",
        "none",
        "unknown",
        "type1productconfigid",
        "x.x",
        "0",
        "123456789",
    ];
    let cleaned = clean_text(value)?;
    let lower = cleaned.to_ascii_lowercase();
    (!PLACEHOLDERS.contains(&lower.as_str())).then_some(cleaned)
}

/// Normalise a MAC address ("AA-BB-CC-DD-EE-FF", "aabbccddeeff") to
/// "aa:bb:cc:dd:ee:ff". All-zero addresses are rejected.
pub fn normalize_mac(value: &str) -> Option<String> {
    let hex: String = value.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let separators = value
        .chars()
        .filter(|c| !c.is_ascii_hexdigit())
        .all(|c| matches!(c, ':' | '-' | '.' | ' '));
    if hex.len() != 12 || !separators || hex.chars().all(|c| c == '0') {
        return None;
    }
    let lower = hex.to_ascii_lowercase();
    Some(
        lower
            .as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).into_owned())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// SMBIOS chassis types that describe portable machines.
pub const PORTABLE_CHASSIS_TYPES: &[u32] = &[8, 9, 10, 14, 30, 31, 32];

// ─── ACPI dump files ────────────────────────────────────────────────────────

pub const ACPI_HEADER_LEN: usize = 36;

/// Signature and declared length of an ACPI table image.
pub fn acpi_table_header(bytes: &[u8]) -> Option<(String, usize)> {
    if bytes.len() < ACPI_HEADER_LEN {
        return None;
    }
    let signature = &bytes[..4];
    if !signature
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
    {
        return None;
    }
    let length = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if length < ACPI_HEADER_LEN || length > bytes.len() {
        return None;
    }
    Some((String::from_utf8_lossy(signature).into_owned(), length))
}

/// File name for the `index`-th (0-based) table with this signature:
/// "DSDT.aml", "SSDT-1.aml", "SSDT-2.aml", "APIC.aml", "APIC-2.aml".
pub fn acpi_dump_file_name(signature: &str, index: usize) -> String {
    if signature == "SSDT" {
        format!("SSDT-{}.aml", index + 1)
    } else if index == 0 {
        format!("{signature}.aml")
    } else {
        format!("{signature}-{}.aml", index + 1)
    }
}

/// Writes dumped ACPI tables into one directory, skipping byte-identical
/// duplicates. The `*.aml` files of an earlier dump are removed when the
/// first new table is written, so a failed dump (no root, no firmware
/// access) never destroys a good one.
pub struct AcpiDumpWriter {
    dir: PathBuf,
    counts: HashMap<String, usize>,
    seen: HashSet<[u8; 32]>,
    written: Vec<String>,
}

impl AcpiDumpWriter {
    /// Create `dir` if needed.
    pub fn create(dir: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(dir).map_err(|e| {
            AppError::new("ACPI_DUMP", format!("Cannot create {}: {e}", dir.display()))
        })?;
        Ok(Self {
            dir: dir.to_path_buf(),
            counts: HashMap::new(),
            seen: HashSet::new(),
            written: Vec::new(),
        })
    }

    fn remove_previous_dump(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_aml = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("aml"));
            if is_aml && path.is_file() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// Validate and write one table. `Ok(None)` for a duplicate.
    pub fn add(&mut self, bytes: &[u8]) -> Result<Option<String>, AppError> {
        let (signature, length) = acpi_table_header(bytes)
            .ok_or_else(|| AppError::new("ACPI_DUMP", "Firmware returned an invalid ACPI table"))?;
        let table = &bytes[..length];
        let digest: [u8; 32] = Sha256::digest(table).into();
        if !self.seen.insert(digest) {
            return Ok(None);
        }
        if self.written.is_empty() {
            self.remove_previous_dump();
        }
        let index = self.counts.entry(signature.clone()).or_insert(0);
        let name = acpi_dump_file_name(&signature, *index);
        *index += 1;
        std::fs::write(self.dir.join(&name), table)
            .map_err(|e| AppError::new("ACPI_DUMP", format!("Cannot write {name}: {e}")))?;
        self.written.push(name.clone());
        Ok(Some(name))
    }

    pub fn written(&self) -> &[String] {
        &self.written
    }

    pub fn has_dsdt(&self) -> bool {
        self.counts.contains_key("DSDT")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

// ─── Deadlines for blocking work and child processes ────────────────────────

/// Await `fut` unless it takes longer than `timeout` or the user cancels.
pub async fn with_deadline<F: Future>(
    fut: F,
    timeout: Duration,
    cancel: &CancellationToken,
    what: &str,
) -> Result<F::Output, AppError> {
    let cancelled = async {
        loop {
            if cancel.is_cancelled() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        out = fut => Ok(out),
        () = tokio::time::sleep(timeout) => Err(AppError::new(
            "SCAN_TIMEOUT",
            format!("{what} did not finish within {} s", timeout.as_secs()),
        )
        .recoverable()),
        () = cancelled => Err(AppError::new("TASK_CANCELLED", "Operation was cancelled by user")),
    }
}

/// Run a prepared command (stdin closed, output captured) with a deadline.
/// The child is killed when the deadline passes or the scan is cancelled.
pub async fn capture_output(
    mut command: tokio::process::Command,
    timeout: Duration,
    cancel: &CancellationToken,
    what: &str,
) -> Result<std::process::Output, AppError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|e| AppError::new("SCAN_PROCESS", format!("Could not start {what}: {e}")))?;
    with_deadline(child.wait_with_output(), timeout, cancel, what)
        .await?
        .map_err(|e| AppError::new("SCAN_PROCESS", format!("{what} failed: {e}")))
}

/// Run a blocking closure on the blocking pool with a deadline.
pub async fn blocking_with_deadline<T, F>(
    work: F,
    timeout: Duration,
    cancel: &CancellationToken,
    what: &str,
) -> Result<T, AppError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    with_deadline(tokio::task::spawn_blocking(work), timeout, cancel, what)
        .await?
        .map_err(|e| AppError::new("SCAN_INTERNAL", format!("{what} stopped unexpectedly: {e}")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pci_pnp_id_puts_subsystem_device_first() {
        let ids = parse_pnp_id(r"PCI\VEN_8086&DEV_3E92&SUBSYS_86941043&REV_02\3&11583659&0&10");
        assert_eq!(ids.bus, "PCI");
        assert_eq!(ids.vendor_id.as_deref(), Some("8086"));
        assert_eq!(ids.device_id.as_deref(), Some("3e92"));
        assert_eq!(ids.subsystem_vendor_id.as_deref(), Some("1043"));
        assert_eq!(ids.subsystem_device_id.as_deref(), Some("8694"));
        assert_eq!(ids.revision.as_deref(), Some("02"));
    }

    #[test]
    fn hdaudio_pnp_id_puts_subsystem_vendor_first() {
        let ids = parse_pnp_id(
            r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280798&REV_1002\4&2fc5d8a2&0&0001",
        );
        assert_eq!(ids.bus, "HDAUDIO");
        assert_eq!(ids.vendor_id.as_deref(), Some("10ec"));
        assert_eq!(ids.device_id.as_deref(), Some("0256"));
        assert_eq!(ids.subsystem_vendor_id.as_deref(), Some("1028"));
        assert_eq!(ids.subsystem_device_id.as_deref(), Some("0798"));
        assert_eq!(ids.revision.as_deref(), Some("1002"));

        let sst =
            parse_pnp_id(r"INTELAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10EC1196&REV_1000\5&1");
        assert_eq!(sst.subsystem_vendor_id.as_deref(), Some("10ec"));
        assert_eq!(sst.subsystem_device_id.as_deref(), Some("1196"));
    }

    #[test]
    fn usb_and_hid_pnp_ids() {
        let usb = parse_pnp_id(r"USB\VID_8087&PID_0029\5&2a4c1b1&0&10");
        assert_eq!(usb.bus, "USB");
        assert_eq!(usb.vendor_id.as_deref(), Some("8087"));
        assert_eq!(usb.device_id.as_deref(), Some("0029"));
        assert_eq!(usb.subsystem_vendor_id, None);

        let hid = parse_pnp_id(r"HID\VEN_SYNA&DEV_7DB5&Col02\5&3a1&0&0001");
        assert_eq!(hid.vendor_id, None, "ACPI vendor strings are not hex ids");
        let acpi = parse_pnp_id(r"ACPI\PNP0303\4&1d401fb5&0");
        assert_eq!(acpi.bus, "ACPI");
        assert_eq!(acpi.vendor_id, None);
        assert_eq!(parse_pnp_id(""), PnpIds::default());
    }

    #[test]
    fn acpi_hardware_ids() {
        assert_eq!(
            acpi_hid_from_pnp_id(r"ACPI\SYNA2393\4&2c4b&0").as_deref(),
            Some("SYNA2393")
        );
        assert_eq!(
            acpi_hid_from_pnp_id(r"ACPI\VEN_ELAN&DEV_0662\4&1").as_deref(),
            Some("ELAN0662")
        );
        assert_eq!(acpi_hid_from_pnp_id("*PNP0C50").as_deref(), Some("PNP0C50"));
        assert_eq!(acpi_hid_from_pnp_id(r"PCI\VEN_8086&DEV_A368"), None);
    }

    #[test]
    fn pci_class_from_compatible_ids() {
        let xhci = pci_class_from_compatible_id(r"PCI\VEN_8086&DEV_A36D&CC_0C0330").unwrap();
        assert_eq!(
            (xhci.base, xhci.sub, xhci.prog_if),
            (0x0c, 0x03, Some(0x30))
        );
        let hda = pci_class_from_compatible_id(r"PCI\CC_0403").unwrap();
        assert!(hda.is(0x04, 0x03));
        assert_eq!(hda.prog_if, None);
        assert_eq!(pci_class_from_compatible_id(r"PCI\VEN_8086"), None);
        assert_eq!(
            PciClass::from_hex("0x010802").and_then(storage_kind_from_class),
            Some("nvme")
        );
        assert_eq!(
            PciClass::from_hex("0x010400").and_then(storage_kind_from_class),
            Some("raid")
        );
        assert_eq!(usb_controller_kind(Some(0x30), ""), "xhci");
        assert_eq!(
            usb_controller_kind(None, "Intel(R) USB 3.1 eXtensible Host Controller"),
            "xhci"
        );
        assert_eq!(
            usb_controller_kind(None, "Standard Enhanced PCI to USB Host Controller"),
            "ehci"
        );
    }

    #[test]
    fn windows_location_paths() {
        assert_eq!(
            location_path_to_device_path("PCIROOT(0)#PCI(1F03)").as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        assert_eq!(
            location_path_to_device_path("PCIROOT(0)#PCI(0100)#PCI(0000)").as_deref(),
            Some("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)")
        );
        assert_eq!(
            location_path_to_device_path("PCIROOT(10)#PCI(0300)").as_deref(),
            Some("PciRoot(0x10)/Pci(0x3,0x0)")
        );
        assert_eq!(location_path_to_device_path("PCIROOT(0)"), None);
        assert_eq!(location_path_to_device_path("ACPI(_SB_)#ACPI(PCI0)"), None);
        assert_eq!(location_path_to_device_path("PCIROOT(0)#PCI(1F3)"), None);
        assert_eq!(location_path_to_device_path("PCIROOT(0)#USBROOT(0)"), None);
        assert_eq!(location_path_to_device_path("PCIRÖÖT(0)#PCI(1F03)"), None);
        assert_eq!(location_path_to_device_path("PCIROOT()#PCI(1F03)"), None);
        // Four bytes but not four ASCII digits: rejected, never split mid-character.
        assert_eq!(location_path_to_device_path("PCIROOT(0)#PCI(aÖb)"), None);
        assert_eq!(location_path_to_device_path("PCIROOT(0)#PCI(+103)"), None);
        assert_eq!(
            location_path_to_acpi_path("ACPI(_SB_)#ACPI(PCI0)#ACPI(HDAS)").as_deref(),
            Some(r"\_SB.PCI0.HDAS")
        );
        assert_eq!(location_path_to_acpi_path("PCIROOT(0)#PCI(1F03)"), None);
        assert_eq!(
            normalize_acpi_path(r"\_SB_.PCI0.LPCB.EC__"),
            r"\_SB.PCI0.LPCB.EC"
        );
    }

    #[test]
    fn sysfs_device_paths() {
        assert_eq!(
            sysfs_to_device_path("/sys/devices/pci0000:00/0000:00:01.0/0000:01:00.0").as_deref(),
            Some("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)")
        );
        assert_eq!(
            sysfs_to_device_path("/sys/devices/pci0000:00/0000:00:1f.3").as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        // Non-PCI tail components are ignored.
        assert_eq!(
            sysfs_to_device_path("/sys/devices/pci0000:00/0000:00:14.0/usb1/1-4").as_deref(),
            Some("PciRoot(0x0)/Pci(0x14,0x0)")
        );
        // A second host bridge needs its _UID.
        assert_eq!(
            sysfs_to_device_path("/sys/devices/pci0000:16/0000:16:00.0"),
            None
        );
        assert_eq!(
            sysfs_to_device_path_with_root_uid("/sys/devices/pci0000:16/0000:16:00.0", Some(1))
                .as_deref(),
            Some("PciRoot(0x1)/Pci(0x0,0x0)")
        );
        // Intel VMD children have no firmware path.
        assert_eq!(
            sysfs_to_device_path("/sys/devices/pci0000:00/0000:00:0e.0/pci10000:e0/10000:e1:00.0"),
            None
        );
        assert_eq!(sysfs_to_device_path("/sys/devices/platform/i8042"), None);
        assert_eq!(parse_bdf("0000:00:1f.3"), Some((0, 0, 0x1f, 3)));
        assert_eq!(parse_bdf("usb1"), None);
        assert_eq!(parse_bdf("0000:00:+1.0"), None);
    }

    #[test]
    fn cpuid_signatures() {
        // i7-9700K (Coffee Lake-S, 0x906ED)
        assert_eq!(decode_signature(0x0009_06ED), (6, 0x9e, 0xd));
        // Ryzen 7 5800X (Vermeer, 0xA20F10)
        assert_eq!(decode_signature(0x00A2_0F10), (0x19, 0x21, 0));
        // Core 2 Duo E8400 (Wolfdale, 0x1067A)
        assert_eq!(decode_signature(0x0001_067A), (6, 0x17, 0xa));
        // FX-8350 (Piledriver, 0x600F20)
        assert_eq!(decode_signature(0x0060_0F20), (0x15, 0x02, 0));
    }

    #[test]
    fn cpuid_feature_bits() {
        let haswell = CpuidFeatureRegs {
            leaf1_ecx: (1 << 0) | (1 << 9) | (1 << 19) | (1 << 20) | (1 << 28) | (1 << 5),
            leaf7_ebx: 1 << 5,
            leaf7_edx: 0,
            ext1_ecx: 0,
        };
        assert_eq!(
            decode_features(&haswell),
            ["sse3", "ssse3", "sse4_1", "sse4_2", "avx", "avx2", "vmx"]
        );
        let alder_vm = CpuidFeatureRegs {
            leaf1_ecx: 1 << 31,
            leaf7_ebx: 0,
            leaf7_edx: 1 << 15,
            ext1_ecx: 1 << 2,
        };
        assert_eq!(decode_features(&alder_vm), ["svm", "hybrid", "hypervisor"]);
        // "GenuineIntel" as EBX, EDX, ECX
        assert_eq!(
            registers_to_text(&[0x756e_6547, 0x4965_6e69, 0x6c65_746e]),
            "GenuineIntel"
        );
    }

    #[test]
    fn brand_strings_and_descriptions() {
        assert_eq!(
            base_clock_from_brand("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz"),
            Some(3600)
        );
        assert_eq!(
            base_clock_from_brand("Intel(R) Core(TM)2 Duo CPU     E8400  @ 3.00GHz"),
            Some(3000)
        );
        assert_eq!(
            base_clock_from_brand("AMD Ryzen 7 5800X 8-Core Processor"),
            None
        );
        assert_eq!(
            parse_processor_description("Intel64 Family 6 Model 158 Stepping 10"),
            Some((6, 158, 10))
        );
        assert_eq!(
            parse_processor_description("AMD64 Family 25 Model 33 Stepping 0"),
            Some((25, 33, 0))
        );
        assert_eq!(parse_processor_description("ARMv8 (64-bit) Family 8"), None);
    }

    #[test]
    fn hypervisor_detection() {
        assert_eq!(hypervisor_from_cpuid_signature("KVMKVMKVM"), "KVM");
        assert_eq!(
            hypervisor_from_cpuid_signature("Microsoft Hv"),
            "Microsoft Hyper-V"
        );
        assert_eq!(
            hypervisor_from_dmi(Some("QEMU"), Some("Standard PC (Q35 + ICH9, 2009)")).as_deref(),
            Some("QEMU")
        );
        assert_eq!(
            hypervisor_from_dmi(Some("innotek GmbH"), Some("VirtualBox")).as_deref(),
            Some("VirtualBox")
        );
        assert_eq!(
            hypervisor_from_dmi(Some("Microsoft Corporation"), Some("Virtual Machine")).as_deref(),
            Some("Microsoft Hyper-V")
        );
        assert_eq!(
            hypervisor_from_dmi(Some("Microsoft Corporation"), Some("Surface Laptop 4")),
            None
        );
        assert_eq!(
            hypervisor_from_dmi(Some("ASUS"), Some("System Product Name")),
            None
        );

        let bare_metal_vbs = CpuidInfo {
            hypervisor_signature: Some("Microsoft Hv".into()),
            hyperv_root_partition: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_hypervisor(
                Some(&bare_metal_vbs),
                Some("Dell Inc."),
                Some("XPS 13 9370")
            ),
            None
        );
        let guest = CpuidInfo {
            hypervisor_signature: Some("VMwareVMware".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_hypervisor(Some(&guest), None, None).as_deref(),
            Some("VMware")
        );
        assert_eq!(
            resolve_hypervisor(None, Some("VMware, Inc."), Some("VMware7,1")).as_deref(),
            Some("VMware")
        );
    }

    #[test]
    fn vendors_and_text() {
        assert_eq!(
            input_vendor(Some("SYNA2393"), "I2C HID Device").as_deref(),
            Some("synaptics")
        );
        assert_eq!(input_vendor(Some("ETD0108"), "").as_deref(), Some("elan"));
        assert_eq!(
            input_vendor(None, "SynPS/2 Synaptics TouchPad").as_deref(),
            Some("synaptics")
        );
        assert_eq!(
            input_vendor(Some("MSFT0001"), "HID-compliant touch pad"),
            None
        );
        assert!(is_hdmi_codec_vendor("8086"));
        assert!(!is_hdmi_codec_vendor("10ec"));
        assert_eq!(clean_dmi("  To Be Filled By O.E.M.  "), None);
        assert_eq!(clean_dmi("Default string"), None);
        assert_eq!(clean_dmi("PRIME Z390-A\0").as_deref(), Some("PRIME Z390-A"));
        assert_eq!(
            normalize_mac("A4-BB-6D-12-34-56").as_deref(),
            Some("a4:bb:6d:12:34:56")
        );
        assert_eq!(
            normalize_mac("a4bb6d123456").as_deref(),
            Some("a4:bb:6d:12:34:56")
        );
        assert_eq!(normalize_mac("00:00:00:00:00:00"), None);
        assert_eq!(normalize_mac("not a mac"), None);
        assert_eq!(hex_id("0x8086", 4).as_deref(), Some("8086"));
        assert_eq!(hex_id("2", 2).as_deref(), Some("02"));
        assert_eq!(hex_id("zz", 2), None);
        assert_eq!(parse_hex_u32("0x10ec0897"), Some(0x10ec_0897));
    }

    fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; ACPI_HEADER_LEN];
        bytes[..4].copy_from_slice(signature);
        bytes.extend_from_slice(body);
        let len = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&len.to_le_bytes());
        bytes
    }

    #[test]
    fn acpi_dump_writer_names_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("oc-acpi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SSDT-9.aml"), b"stale").unwrap();
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();

        let mut writer = AcpiDumpWriter::create(&dir).unwrap();
        assert!(writer.add(b"short").is_err());
        assert!(
            dir.join("SSDT-9.aml").exists(),
            "a failed dump keeps the previous one"
        );
        assert_eq!(
            writer.add(&table(b"DSDT", b"dsdt")).unwrap().as_deref(),
            Some("DSDT.aml")
        );
        assert!(!dir.join("SSDT-9.aml").exists());
        assert!(dir.join("notes.txt").exists());
        assert_eq!(
            writer.add(&table(b"SSDT", b"one")).unwrap().as_deref(),
            Some("SSDT-1.aml")
        );
        assert_eq!(writer.add(&table(b"SSDT", b"one")).unwrap(), None);
        assert_eq!(
            writer.add(&table(b"SSDT", b"two")).unwrap().as_deref(),
            Some("SSDT-2.aml")
        );
        assert_eq!(
            writer.add(&table(b"FACP", b"x")).unwrap().as_deref(),
            Some("FACP.aml")
        );
        assert!(writer.add(b"short").is_err());
        let mut padded = table(b"APIC", b"apic");
        padded.extend_from_slice(&[0xff; 8]);
        writer.add(&padded).unwrap();
        assert_eq!(
            std::fs::read(dir.join("APIC.aml")).unwrap().len(),
            ACPI_HEADER_LEN + 4
        );
        assert!(writer.has_dsdt());
        assert_eq!(writer.written().len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn deadlines_and_cancellation() {
        let token = CancellationToken::new();
        let ok = with_deadline(async { 7 }, Duration::from_secs(1), &token, "quick").await;
        assert_eq!(ok.unwrap(), 7);
        let slow = with_deadline(
            tokio::time::sleep(Duration::from_secs(5)),
            Duration::from_millis(50),
            &token,
            "slow",
        )
        .await;
        assert_eq!(slow.unwrap_err().code, "SCAN_TIMEOUT");
        token.cancel();
        let cancelled = with_deadline(
            tokio::time::sleep(Duration::from_secs(5)),
            Duration::from_secs(5),
            &token,
            "x",
        )
        .await;
        assert_eq!(cancelled.unwrap_err().code, "TASK_CANCELLED");
        let value = blocking_with_deadline(
            || 3,
            Duration::from_secs(1),
            &CancellationToken::new(),
            "work",
        )
        .await;
        assert_eq!(value.unwrap(), 3);
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(16_000_000_000), "16.0 GB");
    }
}
