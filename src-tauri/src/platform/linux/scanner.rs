//! Linux hardware detection using lspci, /proc/cpuinfo, /sys/class/dmi, etc.
//! Ported from electron/hardwareDetect.ts detectLinuxHardware().

use once_cell::sync::Lazy;
use regex::Regex;
use tracing::{debug, warn};

use crate::contracts::*;
use crate::error::AppError;

// Re-use the shared lookup functions from the windows scanner.
// On Linux builds, the windows module isn't compiled, so we duplicate the
// resolution logic here to keep each platform self-contained.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Lookup tables (identical to windows/scanner.rs)
// ---------------------------------------------------------------------------

static GPU_VENDOR_MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("10de", "NVIDIA"), ("1002", "AMD"), ("8086", "Intel"),
        ("1414", "Microsoft"), ("15ad", "VMware"), ("1234", "QEMU"),
    ])
});

static HDA_VENDOR_MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("10ec", "Realtek"), ("14f1", "Conexant"), ("111d", "IDT"),
        ("8384", "SigmaTel"), ("1013", "Cirrus Logic"), ("8086", "Intel HDMI"),
    ])
});

static REALTEK_DEVICE_TO_CODEC: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("0215", "ALC215"), ("0221", "ALC221"), ("0222", "ALC222"), ("0225", "ALC225"),
        ("0230", "ALC230"), ("0233", "ALC233"), ("0235", "ALC235"), ("0236", "ALC236"),
        ("0245", "ALC245"), ("0255", "ALC255"), ("0256", "ALC256"), ("0257", "ALC257"),
        ("0260", "ALC260"), ("0262", "ALC262"), ("0268", "ALC268"), ("0269", "ALC269"),
        ("0270", "ALC270"), ("0272", "ALC272"), ("0274", "ALC274"), ("0275", "ALC275"),
        ("0280", "ALC280"), ("0282", "ALC282"), ("0283", "ALC283"), ("0284", "ALC284"),
        ("0285", "ALC285"), ("0286", "ALC286"), ("0288", "ALC288"), ("0289", "ALC289"),
        ("0290", "ALC290"), ("0292", "ALC292"), ("0293", "ALC293"), ("0294", "ALC294"),
        ("0295", "ALC295"), ("0298", "ALC298"), ("0299", "ALC299"),
        ("0662", "ALC662"), ("0663", "ALC663"), ("0668", "ALC668"),
        ("0670", "ALC670"), ("0671", "ALC671"), ("0700", "ALC700"),
        ("0882", "ALC882"), ("0883", "ALC883"), ("0885", "ALC885"), ("0887", "ALC887"),
        ("0888", "ALC888"), ("0889", "ALC889"), ("0891", "ALC891"), ("0892", "ALC892"),
        ("0897", "ALC897"), ("0898", "ALC898"), ("0899", "ALC899"),
        ("0b00", "ALC1200"), ("0b50", "ALC1220"),
    ])
});

static NETWORK_VENDOR_MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("8086", "Intel"), ("10ec", "Realtek"), ("14e4", "Broadcom"),
        ("1969", "Atheros"), ("168c", "Atheros"), ("1b4b", "Marvell"),
        ("1186", "D-Link"), ("15b7", "Killer"),
    ])
});

static INTEL_ETHERNET_DEVICES: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("153a", "Intel I217-LM"), ("153b", "Intel I217-V"),
        ("155a", "Intel I218-LM"), ("1559", "Intel I218-V"),
        ("15a0", "Intel I218-LM"), ("15a1", "Intel I218-V"),
        ("15a2", "Intel I218-LM"), ("15a3", "Intel I218-V"),
        ("156f", "Intel I219-LM"), ("1570", "Intel I219-V"),
        ("15b7", "Intel I219-LM"), ("15b8", "Intel I219-V"),
        ("15bb", "Intel I219-LM"), ("15bc", "Intel I219-V"),
        ("15bd", "Intel I219-LM"), ("15be", "Intel I219-V"),
        ("15d7", "Intel I219-LM"), ("15d8", "Intel I219-V"),
        ("15e3", "Intel I219-LM"), ("15e4", "Intel I219-V"),
        ("0d4e", "Intel I219-LM"), ("0d4f", "Intel I219-V"),
        ("0d4c", "Intel I219-LM"), ("0d4d", "Intel I219-V"),
        ("0d53", "Intel I219-LM"), ("0d55", "Intel I219-V"),
        ("15f9", "Intel I219-LM"), ("15fa", "Intel I219-V"),
        ("15fb", "Intel I219-LM"), ("15fc", "Intel I219-V"),
        ("1539", "Intel I211-AT"),
        ("1533", "Intel I210-AT"), ("1536", "Intel I210-IT"),
        ("15f2", "Intel I225-V"), ("15f3", "Intel I225-LM"),
        ("125b", "Intel I226-V"), ("125c", "Intel I226-LM"),
        ("10d3", "Intel 82574L"), ("10ea", "Intel 82577LM"),
        ("10eb", "Intel 82577LC"), ("10ef", "Intel 82578DC"),
        ("10f0", "Intel 82578DM"), ("1502", "Intel 82579LM"),
        ("1503", "Intel 82579V"),
    ])
});

static REALTEK_ETHERNET_DEVICES: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("8136", "Realtek RTL8101/8102"), ("8168", "Realtek RTL8111"),
        ("8169", "Realtek RTL8169"), ("8125", "Realtek RTL8125"),
        ("8126", "Realtek RTL8126"), ("2502", "Realtek RTL8125"),
        ("2600", "Realtek RTL8125"),
    ])
});

static INTEL_WIFI_DEVICES: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("08b1", "Intel Wireless 7260"), ("08b2", "Intel Wireless 7260"),
        ("095a", "Intel Wireless 7265"), ("095b", "Intel Wireless 7265"),
        ("08b3", "Intel Wireless 3160"), ("08b4", "Intel Wireless 3160"),
        ("3165", "Intel Wireless 3165"), ("3166", "Intel Wireless 3165"),
        ("3168", "Intel Wireless 3168"),
        ("24f3", "Intel Wireless 8260"), ("24f4", "Intel Wireless 8260"),
        ("24fd", "Intel Wireless 8265"),
        ("2526", "Intel Wireless 9260"),
        ("9df0", "Intel Wireless 9560"), ("9df4", "Intel Wireless 9560"),
        ("30dc", "Intel Wireless 9560"), ("31dc", "Intel Wireless 9560"),
        ("9461", "Intel Wireless 9461"), ("9462", "Intel Wireless 9462"),
        ("2723", "Intel Wi-Fi 6 AX200"),
        ("02f0", "Intel Wi-Fi 6 AX201"), ("06f0", "Intel Wi-Fi 6 AX201"),
        ("a0f0", "Intel Wi-Fi 6 AX201"), ("34f0", "Intel Wi-Fi 6 AX201"),
        ("2725", "Intel Wi-Fi 6E AX210"),
        ("7a70", "Intel Wi-Fi 6E AX211"), ("51f0", "Intel Wi-Fi 6E AX211"),
        ("51f1", "Intel Wi-Fi 6E AX211"), ("54f0", "Intel Wi-Fi 6E AX211"),
    ])
});

static BROADCOM_WIFI_DEVICES: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("4331", "Broadcom BCM4331"), ("4353", "Broadcom BCM43224"),
        ("43a0", "Broadcom BCM4360"), ("43a3", "Broadcom BCM4350"),
        ("43b1", "Broadcom BCM4352"), ("43b2", "Broadcom BCM4352"),
        ("43ba", "Broadcom BCM43602"), ("43dc", "Broadcom BCM4355"),
        ("4464", "Broadcom BCM4364"), ("4488", "Broadcom BCM4377"),
    ])
});

static ATHEROS_ETHERNET_DEVICES: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    HashMap::from([
        ("e091", "Killer E2200"), ("e0a1", "Killer E2400"),
        ("e0b1", "Killer E2500"), ("10a1", "Killer E2600"),
        ("1091", "Atheros AR8161"), ("1083", "Atheros AR8151"),
        ("e062", "Qualcomm Atheros Killer E2200"),
    ])
});

/// Pattern for I2C HID devices on Linux (from /sys/bus/i2c/devices names).
static I2C_HID_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)i2c-hid|hid-over-i2c|ACPI0C50|PNP0C50|ELAN|SYNA|ALPS|ATML|WCOM").expect("regex")
});

/// lspci -nn ID extraction: `[xxxx:yyyy]`
static LSPCI_ID_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[([0-9a-fA-F]{4}):([0-9a-fA-F]{4})\]").expect("regex"));

/// lspci device name extraction: `]: <name> [xxxx:yyyy]`
static LSPCI_NAME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\]:\s*(.+?)(?:\s*\[[0-9a-f]{4}:[0-9a-f]{4}\])?(?:\s*\(rev|$)").expect("regex")
});

// ---------------------------------------------------------------------------
// Shared resolution functions (duplicated from windows to keep cfg-clean)
// ---------------------------------------------------------------------------

fn resolve_gpu_vendor(vendor_id: Option<&str>, raw_name: &str) -> String {
    if let Some(vid) = vendor_id {
        if let Some(&name) = GPU_VENDOR_MAP.get(vid) {
            return name.to_string();
        }
    }
    let n = raw_name.to_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("quadro")
        || n.contains("rtx") || n.contains("gtx")
    {
        return "NVIDIA".to_string();
    }
    if n.contains("amd") || n.contains("radeon") || n.contains("rx ")
        || n.contains("vega") || n.contains("navi")
    {
        return "AMD".to_string();
    }
    if n.contains("intel") || n.contains("iris") || n.contains("uhd")
        || n.contains("hd graphics")
    {
        return "Intel".to_string();
    }
    "Unknown".to_string()
}

fn resolve_audio_codec(vendor_id: Option<&str>, device_id: Option<&str>) -> Option<String> {
    let vid = vendor_id?;
    let did = device_id?;
    if vid == "10ec" {
        return Some(
            REALTEK_DEVICE_TO_CODEC
                .get(did)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Realtek (DEV_{})", did.to_uppercase())),
        );
    }
    HDA_VENDOR_MAP
        .get(vid)
        .map(|name| format!("{} (DEV_{})", name, did.to_uppercase()))
}

fn classify_network_type(name: &str) -> NetworkDeviceType {
    let lower = name.to_lowercase();
    if lower.contains("wi-fi") || lower.contains("wifi") || lower.contains("wireless")
        || lower.contains("wlan") || lower.contains("802.11") || lower.contains("centrino")
        || lower.contains("airport") || lower.contains("dual band")
    {
        return NetworkDeviceType::Wifi;
    }
    if lower.contains("ethernet") || lower.contains("gigabit")
        || lower.contains("network connection") || lower.contains("lan")
        || lower.contains("nic") || lower.contains("gbe")
        || lower.contains("i217") || lower.contains("i218") || lower.contains("i219")
        || lower.contains("i211") || lower.contains("i225") || lower.contains("i226")
        || lower.contains("rtl8111") || lower.contains("rtl8168") || lower.contains("rtl8125")
        || lower.contains("killer e") || lower.contains("82579") || lower.contains("82574")
    {
        return NetworkDeviceType::Ethernet;
    }
    NetworkDeviceType::Ethernet
}

fn resolve_network_adapter(
    vendor_id: Option<&str>,
    device_id: Option<&str>,
    name: &str,
) -> (String, Option<String>, NetworkDeviceType) {
    let vid = vendor_id.unwrap_or("");
    let did = device_id.unwrap_or("");
    let vendor_name = NETWORK_VENDOR_MAP.get(vid).copied().unwrap_or("Unknown").to_string();
    let mut net_type = classify_network_type(name);
    let mut adapter_family: Option<String> = None;

    if vid == "8086" {
        if let Some(&fam) = INTEL_ETHERNET_DEVICES.get(did) {
            adapter_family = Some(fam.to_string());
            net_type = NetworkDeviceType::Ethernet;
        } else if let Some(&fam) = INTEL_WIFI_DEVICES.get(did) {
            adapter_family = Some(fam.to_string());
            net_type = NetworkDeviceType::Wifi;
        } else {
            adapter_family = Some(match net_type {
                NetworkDeviceType::Wifi => "Intel Wi-Fi (unknown model)",
                _ => "Intel Ethernet (unknown model)",
            }.to_string());
        }
    } else if vid == "10ec" {
        if let Some(&fam) = REALTEK_ETHERNET_DEVICES.get(did) {
            adapter_family = Some(fam.to_string());
            net_type = NetworkDeviceType::Ethernet;
        } else {
            adapter_family = Some("Realtek (unknown model)".to_string());
        }
    } else if vid == "14e4" {
        if let Some(&fam) = BROADCOM_WIFI_DEVICES.get(did) {
            adapter_family = Some(fam.to_string());
            net_type = NetworkDeviceType::Wifi;
        } else {
            adapter_family = Some(match net_type {
                NetworkDeviceType::Wifi => "Broadcom Wi-Fi (unknown model)",
                NetworkDeviceType::Ethernet => "Broadcom Ethernet (unknown model)",
                _ => "Broadcom (unknown model)",
            }.to_string());
        }
    } else if vid == "1969" || vid == "168c" || vid == "15b7" {
        if let Some(&fam) = ATHEROS_ETHERNET_DEVICES.get(did) {
            adapter_family = Some(fam.to_string());
            net_type = NetworkDeviceType::Ethernet;
        } else {
            adapter_family = Some(match net_type {
                NetworkDeviceType::Wifi => "Atheros Wi-Fi (unknown model)",
                _ => "Atheros/Killer (unknown model)",
            }.to_string());
        }
    }

    (vendor_name, adapter_family, net_type)
}

fn resolve_cpu_vendor(vendor_raw: &str, cpu_name: &str) -> (String, String) {
    let v = vendor_raw.to_lowercase();
    if v.contains("genuineintel") || v.contains("intel") {
        return ("GenuineIntel".to_string(), "Intel".to_string());
    }
    if v.contains("authenticamd") || v.contains("amd") {
        return ("AuthenticAMD".to_string(), "AMD".to_string());
    }
    let n = cpu_name.to_lowercase();
    if n.contains("intel") {
        return ("GenuineIntel".to_string(), "Intel".to_string());
    }
    if n.contains("amd") || n.contains("ryzen") || n.contains("threadripper") {
        return ("AuthenticAMD".to_string(), "AMD".to_string());
    }
    (
        if vendor_raw.is_empty() { "Unknown".to_string() } else { vendor_raw.to_string() },
        "Unknown".to_string(),
    )
}

// ---------------------------------------------------------------------------
// Shell command helpers
// ---------------------------------------------------------------------------

fn run_cmd(cmd: &str, args: &[&str]) -> String {
    debug!(cmd, ?args, "Running shell command");
    match std::process::Command::new(cmd).args(args).output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
        Err(e) => {
            warn!(cmd, error = %e, "Shell command failed");
            String::new()
        }
    }
}

fn read_sysfs(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

/// Parse lspci -nn line for vendor:device IDs.
fn parse_lspci_ids(line: &str) -> (Option<String>, Option<String>) {
    LSPCI_ID_RE.captures(line).map_or((None, None), |caps| {
        (Some(caps[1].to_lowercase()), Some(caps[2].to_lowercase()))
    })
}

/// Extract device name from lspci -nn line.
fn parse_lspci_name(line: &str) -> String {
    LSPCI_NAME_RE
        .captures(line)
        .map(|caps| caps[1].trim().to_string())
        .unwrap_or_else(|| {
            line.split(':')
                .skip(2)
                .collect::<Vec<_>>()
                .join(":")
                .trim()
                .to_string()
        })
}

// ---------------------------------------------------------------------------
// Public scanner entry point
// ---------------------------------------------------------------------------

/// Full Linux hardware scan. Runs shell commands in parallel via spawn_blocking.
pub async fn scan() -> Result<DetectedHardware, AppError> {
    use crate::domain::form_factor::{infer_laptop_form_factor, FormFactorEvidence};

    tracing::info!("Starting Linux hardware detection");

    // Run all commands in parallel via spawn_blocking
    let (cpu_res, gpu_res, board_vendor_res, board_model_res, chassis_res,
         sys_vendor_res, battery_res, mem_res, audio_res, network_res, i2c_res) =
        tokio::try_join!(
            tokio::task::spawn_blocking(|| std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default()),
            tokio::task::spawn_blocking(|| run_cmd("lspci", &["-nn"])),
            tokio::task::spawn_blocking(|| read_sysfs("/sys/class/dmi/id/board_vendor")),
            tokio::task::spawn_blocking(|| {
                let name = read_sysfs("/sys/class/dmi/id/board_name");
                if name.is_empty() { read_sysfs("/sys/class/dmi/id/product_name") } else { name }
            }),
            tokio::task::spawn_blocking(|| read_sysfs("/sys/class/dmi/id/chassis_type")),
            tokio::task::spawn_blocking(|| read_sysfs("/sys/class/dmi/id/sys_vendor")),
            tokio::task::spawn_blocking(|| run_cmd("ls", &["/sys/class/power_supply"])),
            tokio::task::spawn_blocking(|| std::fs::read_to_string("/proc/meminfo").unwrap_or_default()),
            tokio::task::spawn_blocking(|| run_cmd("lspci", &["-nn"])), // filtered below for audio
            tokio::task::spawn_blocking(|| run_cmd("lspci", &["-nn"])), // filtered below for network
            tokio::task::spawn_blocking(|| {
                // List I2C devices + read their names
                let ls = run_cmd("ls", &["/sys/bus/i2c/devices"]);
                let mut names = Vec::new();
                for dev in ls.lines() {
                    let dev = dev.trim();
                    if dev.is_empty() { continue; }
                    let path = format!("/sys/bus/i2c/devices/{}/name", dev);
                    let name = read_sysfs(&path);
                    if !name.is_empty() {
                        names.push(name);
                    }
                }
                names
            }),
        )
        .map_err(|e| AppError::new("LINUX_SCAN_JOIN", format!("Task join error: {e}")))?;

    // ── CPU ──
    let cpu_lines: Vec<&str> = cpu_res.lines().collect();
    let cpu_name = cpu_lines
        .iter()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unknown CPU".to_string());
    let cpu_vendor_raw = cpu_lines
        .iter()
        .find(|l| l.starts_with("vendor_id"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (_vendor_id, vendor_name) = resolve_cpu_vendor(&cpu_vendor_raw, &cpu_name);

    // ── GPU ── lspci lines containing VGA/3D/Display
    static GPU_LINE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)VGA|3D|Display").expect("regex"));

    let gpu_lines: Vec<&str> = gpu_res.lines().filter(|l| GPU_LINE_RE.is_match(l)).collect();
    let mut gpus: Vec<GpuInfo> = gpu_lines
        .iter()
        .map(|line| {
            let (vid, did) = parse_lspci_ids(line);
            let name = parse_lspci_name(line);
            let vendor_str = resolve_gpu_vendor(vid.as_deref(), &name);
            let is_igpu = vendor_str == "Intel";
            let is_discrete = vendor_str == "NVIDIA" || vendor_str == "AMD";
            GpuInfo {
                name,
                vendor: vendor_str,
                vendor_id: vid,
                device_id: did,
                vram_mb: None,
                is_discrete,
                is_igpu,
            }
        })
        .collect();
    if gpus.is_empty() {
        gpus.push(GpuInfo::default());
    }

    // ── RAM ──
    static MEMTOTAL_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"MemTotal:\s*(\d+)").expect("regex"));
    let mem_kb: u64 = MEMTOTAL_RE
        .captures(&mem_res)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0);
    let total_mb = mem_kb / 1024;

    // ── Chassis / laptop ──
    let chassis_type: u32 = chassis_res.parse().unwrap_or(0);
    let battery_lines: Vec<&str> = battery_res
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("BAT")
        })
        .collect();
    let battery_present = !battery_lines.is_empty();

    let gpu_name_str: String = gpus.iter().map(|g| g.name.as_str()).collect::<Vec<_>>().join(" / ");
    let chassis_types: Vec<u32> = if chassis_type > 0 {
        vec![chassis_type]
    } else {
        Vec::new()
    };
    let is_laptop = infer_laptop_form_factor(&FormFactorEvidence {
        cpu_name: &cpu_name,
        chassis_types: &chassis_types,
        model_name: &board_model_res,
        battery_present,
        manufacturer: &sys_vendor_res,
        gpu_name: &gpu_name_str,
    });

    // ── Audio ── lspci lines containing audio/HDA
    static AUDIO_LINE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)audio|HDA").expect("regex"));

    let audio_devices: Vec<AudioDevice> = audio_res
        .lines()
        .filter(|l| AUDIO_LINE_RE.is_match(l))
        .map(|line| {
            let (vid, did) = parse_lspci_ids(line);
            let name = parse_lspci_name(line);
            let codec = resolve_audio_codec(vid.as_deref(), did.as_deref());
            AudioDevice {
                name,
                codec,
                vendor_id: vid,
                device_id: did,
            }
        })
        .collect();

    // ── Network ── lspci lines containing Ethernet/Network/Wireless/Wi-Fi
    static NET_LINE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)Ethernet|Network|Wireless|Wi-Fi").expect("regex"));

    let network_devices: Vec<NetworkDevice> = network_res
        .lines()
        .filter(|l| NET_LINE_RE.is_match(l))
        .map(|line| {
            let (vid, did) = parse_lspci_ids(line);
            let name = parse_lspci_name(line);
            let (_vendor_name, adapter_family, net_type) =
                resolve_network_adapter(vid.as_deref(), did.as_deref(), &name);
            NetworkDevice {
                name,
                device_type: net_type,
                vendor_id: vid,
                device_id: did,
                chipset: adapter_family,
            }
        })
        .collect();

    // ── Input devices ── I2C HID from /sys/bus/i2c/devices
    // Only count devices with HID-compatible names (not backlight controllers, VRMs, etc.)
    let input_devices: Vec<InputDevice> = i2c_res
        .iter()
        .filter(|name| I2C_HID_PATTERN.is_match(name))
        .map(|name| InputDevice {
            name: name.clone(),
            device_type: InputDeviceType::I2c,
            instance_id: Some(format!("/sys/bus/i2c/devices/{}", name)),
        })
        .collect();

    tracing::info!(
        cpu = %cpu_name,
        gpu_count = gpus.len(),
        is_laptop,
        "Linux hardware detection complete"
    );

    Ok(DetectedHardware {
        cpu: CpuInfo {
            name: cpu_name,
            vendor: vendor_name,
            cores: 0, // Could parse from cpuinfo but not critical
            threads: 0,
            base_clock: None,
            generation: None,
            architecture: None,
            codename: None,
            family: None,
            model: None,
            stepping: None,
        },
        gpu: gpus,
        audio: audio_devices,
        network: network_devices,
        input: input_devices,
        memory: MemoryInfo {
            total_mb,
            slots: vec![],
        },
        motherboard: MotherboardInfo {
            manufacturer: Some(if board_vendor_res.is_empty() {
                "Unknown".to_string()
            } else {
                board_vendor_res
            }),
            product: Some(if board_model_res.is_empty() {
                "Unknown".to_string()
            } else {
                board_model_res
            }),
            chipset: None,
        },
        storage: vec![],
        chassis: ChassisInfo {
            chassis_type: if chassis_type > 0 {
                Some(chassis_type.to_string())
            } else {
                None
            },
            manufacturer: Some(sys_vendor_res),
            has_battery: battery_present,
        },
        platform: "linux".to_string(),
        is_laptop,
    })
}
