//! CPU, memory, SMBIOS (DMI), firmware, battery / lid and hypervisor facts.

use crate::contracts::{ChassisInfo, CpuInfo, FirmwareInfo, MemoryInfo, MotherboardInfo};
use crate::platform::common::{base_clock_from_brand, clean_dmi, parse_hex_u32, CpuidInfo};

use super::parse::{count_topology, features_from_flags, parse_cpuinfo, secure_boot_from_efivar};
use super::sysfs::SysRoot;

const SECURE_BOOT_VAR: &str =
    "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

/// CPU facts; `cpuid` (x86 hosts) wins over `/proc/cpuinfo` for identity.
/// The bool is the `hypervisor` flag from cpuinfo.
pub fn cpu(sys: &SysRoot, cpuid: Option<&CpuidInfo>) -> (CpuInfo, bool) {
    let info = sys
        .read_bytes("/proc/cpuinfo")
        .map(|b| parse_cpuinfo(&String::from_utf8_lossy(&b)))
        .unwrap_or_default();
    let topology: Vec<(u32, u32)> = sys
        .list("/sys/devices/system/cpu")
        .into_iter()
        .filter(|n| {
            n.strip_prefix("cpu")
                .is_some_and(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()))
        })
        .filter_map(|n| {
            let dir = format!("/sys/devices/system/cpu/{n}/topology");
            let package = sys
                .read(&format!("{dir}/physical_package_id"))?
                .parse()
                .ok()?;
            let core = sys.read(&format!("{dir}/core_id"))?.parse().ok()?;
            Some((package, core))
        })
        .collect();
    let (topo_cores, topo_packages) = count_topology(&topology);
    let threads = if info.logical > 0 {
        info.logical
    } else {
        topology.len() as u32
    };
    let cores = info
        .physical_cores
        .or((topo_cores > 0).then_some(topo_cores))
        .unwrap_or(threads);
    let packages = info
        .packages
        .or((topo_packages > 0).then_some(topo_packages))
        .unwrap_or(1);

    let mut cpu = CpuInfo {
        cores,
        threads,
        packages,
        ..Default::default()
    };
    match cpuid {
        Some(id) => {
            cpu.vendor = id.vendor.clone();
            cpu.name = if id.brand.is_empty() {
                info.model_name.clone().unwrap_or_default()
            } else {
                id.brand.clone()
            };
            cpu.family = Some(id.family);
            cpu.model = Some(id.model);
            cpu.stepping = Some(id.stepping);
            cpu.features = id.features.clone();
        }
        None => {
            cpu.vendor = info.vendor.clone().unwrap_or_default();
            cpu.name = info.model_name.clone().unwrap_or_default();
            cpu.family = info.family;
            cpu.model = info.model;
            cpu.stepping = info.stepping;
            cpu.features = features_from_flags(&info.flags);
            if sys.exists("/sys/devices/cpu_core") && sys.exists("/sys/devices/cpu_atom") {
                cpu.features.push("hybrid".into());
            }
        }
    }
    // intel_pstate exposes the base (non-turbo) frequency in kHz.
    cpu.base_clock_mhz = sys
        .read("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")
        .and_then(|k| k.parse::<u32>().ok())
        .map(|khz| khz / 1000)
        .filter(|mhz| *mhz > 0)
        .or_else(|| base_clock_from_brand(&cpu.name));
    let hypervisor_flag = info.flags.iter().any(|f| f == "hypervisor");
    (cpu, hypervisor_flag)
}

/// Installed memory from the online memory blocks (close to the DIMM total),
/// else `MemTotal`.
pub fn memory(sys: &SysRoot) -> MemoryInfo {
    let block_size = sys
        .read("/sys/devices/system/memory/block_size_bytes")
        .and_then(|v| parse_hex_u32(&v));
    let online = sys
        .list("/sys/devices/system/memory")
        .into_iter()
        .filter(|n| n.starts_with("memory"))
        .filter(|n| {
            sys.read(&format!("/sys/devices/system/memory/{n}/online"))
                .as_deref()
                == Some("1")
        })
        .count() as u64;
    if let Some(size) = block_size.filter(|_| online > 0) {
        return MemoryInfo {
            total_mb: online * u64::from(size) / (1024 * 1024),
        };
    }
    let total_kb = sys
        .read_bytes("/proc/meminfo")
        .ok()
        .and_then(|b| {
            String::from_utf8_lossy(&b).lines().find_map(|l| {
                l.strip_prefix("MemTotal:")
                    .map(|v| v.trim().trim_end_matches("kB").trim().to_string())
            })
        })
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    MemoryInfo {
        total_mb: total_kb / 1024,
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlatformFacts {
    pub motherboard: MotherboardInfo,
    pub chassis: ChassisInfo,
    pub firmware: FirmwareInfo,
    /// `/sys/hypervisor/type` ("xen").
    pub hypervisor_type: Option<String>,
}

pub fn platform(sys: &SysRoot) -> PlatformFacts {
    let dmi = |name: &str| {
        sys.read(&format!("/sys/class/dmi/id/{name}"))
            .and_then(|v| clean_dmi(&v))
    };
    let chassis_types = sys
        .read("/sys/class/dmi/id/chassis_type")
        .and_then(|v| v.parse::<u32>().ok())
        .into_iter()
        .collect();
    let has_battery = sys
        .list("/sys/class/power_supply")
        .into_iter()
        .any(|supply| {
            let base = format!("/sys/class/power_supply/{supply}");
            sys.read(&format!("{base}/type")).as_deref() == Some("Battery")
                && sys.read(&format!("{base}/scope")).as_deref() != Some("Device")
        });
    // Generic desktop DSDTs often declare a lid with _STA = 0.
    let has_lid = !sys.list("/proc/acpi/button/lid").is_empty()
        || sys
            .list("/sys/bus/acpi/devices")
            .iter()
            .filter(|d| d.starts_with("PNP0C0D:"))
            .any(|d| {
                sys.read(&format!("/sys/bus/acpi/devices/{d}/status"))
                    .and_then(|s| s.parse::<u32>().ok())
                    .is_none_or(|s| s & 1 == 1)
            });
    let uefi = sys.exists("/sys/firmware/efi");
    let secure_boot = if uefi {
        sys.read_bytes(SECURE_BOOT_VAR)
            .ok()
            .and_then(|b| secure_boot_from_efivar(&b))
    } else {
        None
    };
    PlatformFacts {
        motherboard: MotherboardInfo {
            manufacturer: dmi("board_vendor"),
            product: dmi("board_name"),
            system_manufacturer: dmi("sys_vendor"),
            system_product: dmi("product_name"),
            ..Default::default()
        },
        chassis: ChassisInfo {
            chassis_types,
            manufacturer: dmi("chassis_vendor"),
            has_battery,
            has_lid,
        },
        firmware: FirmwareInfo {
            uefi: Some(uefi),
            secure_boot,
            bios_vendor: dmi("bios_vendor"),
            bios_version: dmi("bios_version"),
            bios_date: dmi("bios_date"),
        },
        hypervisor_type: sys.read("/sys/hypervisor/type"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::sysfs::fixture::Tree;
    use super::*;

    #[test]
    fn cpu_from_cpuinfo_and_topology() {
        let t = Tree::new("cpu");
        t.file(
            "/proc/cpuinfo",
            "processor\t: 0\nvendor_id\t: GenuineIntel\ncpu family\t: 6\nmodel\t\t: 158\n\
             model name\t: Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz\nstepping\t: 13\n\
             flags\t\t: fpu sse4_2 avx avx2 vmx hypervisor\n\nprocessor\t: 1\n\nprocessor\t: 2\n",
        );
        for (cpu, core) in [(0, 0), (1, 1), (2, 1)] {
            t.file(
                &format!("/sys/devices/system/cpu/cpu{cpu}/topology/physical_package_id"),
                "0",
            )
            .file(
                &format!("/sys/devices/system/cpu/cpu{cpu}/topology/core_id"),
                &core.to_string(),
            );
        }
        t.file(
            "/sys/devices/system/cpu/cpu0/cpufreq/base_frequency",
            "3600000",
        );
        let (cpu, hv) = cpu(&SysRoot::new(&t.root), None);
        assert_eq!(cpu.vendor, "GenuineIntel");
        assert_eq!(
            (cpu.family, cpu.model, cpu.stepping),
            (Some(6), Some(158), Some(13))
        );
        assert_eq!((cpu.cores, cpu.threads, cpu.packages), (2, 3, 1));
        assert_eq!(cpu.base_clock_mhz, Some(3600));
        assert_eq!(cpu.features, ["sse4_2", "avx", "avx2", "vmx", "hypervisor"]);
        assert!(hv);

        let id = CpuidInfo {
            vendor: "AuthenticAMD".into(),
            brand: "AMD Ryzen 7 5800X 8-Core Processor".into(),
            family: 0x19,
            model: 0x21,
            ..Default::default()
        };
        let (cpu, _) = super::cpu(&SysRoot::new(&t.root), Some(&id));
        assert_eq!(cpu.vendor, "AuthenticAMD");
        assert_eq!(cpu.family, Some(0x19));
    }

    #[test]
    fn memory_blocks_and_meminfo() {
        let t = Tree::new("mem");
        t.file(
            "/proc/meminfo",
            "MemTotal:       16281236 kB\nMemFree: 1 kB\n",
        );
        assert_eq!(memory(&SysRoot::new(&t.root)).total_mb, 15899);
        t.file("/sys/devices/system/memory/block_size_bytes", "8000000\n");
        for i in 0..128 {
            t.file(&format!("/sys/devices/system/memory/memory{i}/online"), "1");
        }
        assert_eq!(memory(&SysRoot::new(&t.root)).total_mb, 16384);
    }

    #[test]
    fn dmi_firmware_and_power() {
        let t = Tree::new("dmi");
        t.file("/sys/class/dmi/id/sys_vendor", "Dell Inc.\n")
            .file("/sys/class/dmi/id/product_name", "XPS 13 9370\n")
            .file("/sys/class/dmi/id/board_vendor", "Dell Inc.\n")
            .file("/sys/class/dmi/id/board_name", "0F6P3V\n")
            .file("/sys/class/dmi/id/chassis_type", "10\n")
            .file("/sys/class/dmi/id/chassis_vendor", "Dell Inc.\n")
            .file("/sys/class/dmi/id/bios_vendor", "Dell Inc.\n")
            .file("/sys/class/dmi/id/bios_version", "1.21.0\n")
            .file("/sys/class/dmi/id/bios_date", "07/06/2022\n")
            .file("/sys/class/power_supply/BAT0/type", "Battery")
            .file("/sys/class/power_supply/hidpp_battery_0/type", "Battery")
            .file("/sys/class/power_supply/hidpp_battery_0/scope", "Device")
            .file("/proc/acpi/button/lid/LID0/state", "state:      open")
            .bytes(SECURE_BOOT_VAR, &[6, 0, 0, 0, 1]);
        let p = platform(&SysRoot::new(&t.root));
        assert_eq!(p.motherboard.system_product.as_deref(), Some("XPS 13 9370"));
        assert_eq!(p.motherboard.product.as_deref(), Some("0F6P3V"));
        assert_eq!(p.chassis.chassis_types, [10]);
        assert!(p.chassis.has_battery && p.chassis.has_lid);
        assert_eq!(p.firmware.uefi, Some(true));
        assert_eq!(p.firmware.secure_boot, Some(true));
        assert_eq!(p.firmware.bios_date.as_deref(), Some("07/06/2022"));

        let desktop = Tree::new("dmi-desktop");
        desktop
            .file("/sys/class/dmi/id/board_name", "To be filled by O.E.M.")
            .file("/sys/class/power_supply/hidpp_battery_0/type", "Battery")
            .file("/sys/class/power_supply/hidpp_battery_0/scope", "Device");
        desktop.file("/sys/bus/acpi/devices/PNP0C0D:00/status", "0\n");
        let p = platform(&SysRoot::new(&desktop.root));
        assert_eq!(p.motherboard.product, None);
        assert!(!p.chassis.has_battery && !p.chassis.has_lid);
        desktop.file("/sys/bus/acpi/devices/PNP0C0D:00/status", "15\n");
        assert!(platform(&SysRoot::new(&desktop.root)).chassis.has_lid);
        assert_eq!(p.firmware.uefi, Some(false));
        assert_eq!(p.firmware.secure_boot, None);
    }
}
