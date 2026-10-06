//! The Windows device inventory: one PowerShell/CIM script, its JSON output
//! and the mapping of that output (plus in-process facts) to
//! `DetectedHardware`. Pure code; the process and Win32 calls live in
//! `powershell.rs`, `native.rs` and `usb_ports.rs`.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde_json::Value;

use crate::contracts::{
    AudioDevice, ChassisInfo, CpuInfo, DetectedHardware, FirmwareInfo, GpuInfo, InputDevice,
    InputKind, MemoryInfo, MotherboardInfo, NetworkDevice, NetworkKind, PciLocation, StorageDevice,
    UsbControllerInfo, UsbPortInfo,
};
use crate::error::AppError;
use crate::platform::common::{
    acpi_hid_from_pnp_id, base_clock_from_brand, clean_dmi, clean_text, input_vendor,
    is_hdmi_codec_vendor, location_path_to_acpi_path, location_path_to_device_path,
    normalize_acpi_path, normalize_mac, parse_pnp_id, parse_processor_description,
    pci_class_from_compatible_id, resolve_hypervisor, storage_kind_from_class, usb_controller_kind,
    CpuidInfo, PciClass, PnpIds,
};

use super::raw::ProcessorTopology;

pub const BEGIN_MARKER: &str = "<<OCJSON>>";
pub const END_MARKER: &str = "<</OCJSON>>";

/// CIM queries for everything the scanner needs, in one PowerShell run.
/// Every query is guarded so one failing class only loses its own section
/// (reported in `errors`). Phantom devices (problem code 45) are skipped.
/// Device properties are read with `GetDeviceProperties`, only where the
/// mapping uses them: location paths, ACPI names and driver keys of network,
/// display, audio and USB controllers (HEDT boards have hundreds of other
/// PCI functions), and the parent of codecs (plus the Intel SST bus nodes
/// between a codec and its controller), disks and non-USB HID collections.
/// Non-ASCII characters are escaped so the output survives any console code
/// page. BIOS dates are converted back to UTC before formatting, so the
/// local time zone cannot move them by a day.
pub const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
try{[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding $false}catch{}
$r=[ordered]@{errors=@()}
function Q($n,$b){try{$r[$n]=@(& $b)}catch{$r.errors+=('{0}: {1}' -f $n,$_.Exception.Message);$r[$n]=@()}}
function D($d){if($d){try{$d.ToUniversalTime().ToString('MM/dd/yyyy',[Globalization.CultureInfo]::InvariantCulture)}catch{}}}
Q cpu {Get-CimInstance Win32_Processor|Select-Object Name,Manufacturer,Description,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed}
Q system {Get-CimInstance Win32_ComputerSystem|Select-Object Manufacturer,Model,TotalPhysicalMemory,HypervisorPresent}
Q board {Get-CimInstance Win32_BaseBoard|Select-Object Manufacturer,Product}
Q enclosure {Get-CimInstance Win32_SystemEnclosure|Select-Object Manufacturer,ChassisTypes}
Q bios {Get-CimInstance Win32_BIOS|Select-Object Manufacturer,SMBIOSBIOSVersion,@{n='ReleaseDate';e={D $_.ReleaseDate}}}
Q battery {Get-CimInstance Win32_Battery|Select-Object DeviceID,Name,Chemistry,PNPDeviceID}
Q memory {Get-CimInstance Win32_PhysicalMemory|Select-Object Capacity}
Q adapters {Get-CimInstance Win32_NetworkAdapter|Where-Object{$_.PNPDeviceID}|Select-Object Name,PNPDeviceID,MACAddress,PhysicalAdapter}
Q disks {Get-CimInstance Win32_DiskDrive|Select-Object Index,Model,Size,PNPDeviceID,InterfaceType}
Q physicalDisks {Get-CimInstance -Namespace root/Microsoft/Windows/Storage -ClassName MSFT_PhysicalDisk|Select-Object DeviceId,BusType}
$dev=@()
try{
$cls='Display','MEDIA','Net','Bluetooth','HIDClass','Keyboard','Mouse','DiskDrive'
$dev=@(Get-CimInstance Win32_PnPEntity|Where-Object{$_.Present -ne $false -and $_.ConfigManagerErrorCode -ne 45}|Where-Object{$i=[string]$_.PNPDeviceID;$i -like 'PCI\*' -or $i -like 'INTELAUDIO\*' -or $i -like 'ACPI\PNP0C0D*' -or ($cls -contains $_.PNPClass -and $i -notmatch '^(ROOT|SWD|SW|BTH|BTHENUM|BTHLE|BTHLEDEVICE|UMB|STORAGE)\\')})
}catch{$r.errors+=('devices: {0}' -f $_.Exception.Message)}
$props=@()
$t0=Get-Date
$pk=[string[]]('DEVPKEY_Device_LocationPaths','DEVPKEY_Device_BiosDeviceName','DEVPKEY_Device_Driver')
$ck=[string[]]('DEVPKEY_Device_Parent')
foreach($d in $dev){
if(((Get-Date)-$t0).TotalSeconds -gt 20){$r.errors+='properties: time limit reached';break}
$i=[string]$d.PNPDeviceID
$k=$null
if($i -like 'PCI\*'){$cc=@($d.CompatibleID|Where-Object{$_ -like 'PCI\CC_*'});if($cc.Count -eq 0 -or ($cc -match '^PCI\\CC_(02|03|04|0C03)')){$k=$pk}}elseif($d.PNPClass -eq 'DiskDrive' -or $i -match '^(HDAUDIO|INTELAUDIO)\\' -or $i -match '^HID\\(?!VID_|\{)'){$k=$ck}
if($null -eq $k){continue}
try{$o=Invoke-CimMethod -InputObject $d -MethodName GetDeviceProperties -Arguments @{devicePropertyKeys=$k}
foreach($p in @($o.deviceProperties)){if($null -ne $p -and $null -ne $p.Data){$props+=[pscustomobject]@{Id=$i;Key=[string]$p.KeyName;Data=$p.Data}}}}catch{}
}
$r.devices=@($dev|Select-Object Name,PNPClass,PNPDeviceID,Service,HardwareID,CompatibleID)
$r.properties=$props
$j=ConvertTo-Json -InputObject $r -Depth 5 -Compress
try{$j=[regex]::Replace($j,'[^\x00-\x7f]',{param($m)'\u{0:x4}' -f [int][char]$m.Value})}catch{}
'<<OCJSON>>'+$j+'<</OCJSON>>'
"#;

/// `-EncodedCommand` argument: base64 of the UTF-16LE script.
pub fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// PowerShell stdout as text (UTF-8, BOM tolerated, invalid bytes replaced).
pub fn decode_output(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// The JSON between the markers (anything a profile script or module prints
/// around it is ignored).
pub fn extract_payload(text: &str) -> Option<&str> {
    let start = text.find(BEGIN_MARKER)? + BEGIN_MARKER.len();
    let end = text[start..].find(END_MARKER)? + start;
    Some(text[start..end].trim())
}

// ─── Parsed inventory ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Processor {
    pub name: Option<String>,
    pub manufacturer: Option<String>,
    pub description: Option<String>,
    pub cores: Option<u64>,
    pub threads: Option<u64>,
    pub max_clock_mhz: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetAdapter {
    pub pnp_id: String,
    pub mac: Option<String>,
}

/// One `Win32_Battery`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Battery {
    /// `DeviceID` and `Name`.
    pub label: String,
    /// 3 = lead acid (UPS), 6 = lithium-ion, 8 = lithium polymer.
    pub chemistry: Option<u64>,
    pub pnp_id: Option<String>,
}

impl Battery {
    /// UPS units on USB / HID show up as batteries too.
    fn is_ups(&self) -> bool {
        let hid_or_usb = self.pnp_id.as_deref().is_some_and(|id| {
            let upper = id.to_ascii_uppercase();
            upper.starts_with(r"HID\") || upper.starts_with(r"USB\")
        });
        let named_ups = self
            .label
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w.eq_ignore_ascii_case("ups"));
        self.chemistry == Some(3) || hid_or_usb || named_ups
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Disk {
    pub index: Option<u64>,
    pub model: Option<String>,
    pub size_bytes: Option<u64>,
    pub pnp_id: Option<String>,
    pub interface: Option<String>,
}

/// One `Win32_PnPEntity`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PnpDevice {
    pub name: String,
    pub class: String,
    pub id: String,
    pub service: Option<String>,
    pub hardware_ids: Vec<String>,
    pub compatible_ids: Vec<String>,
}

impl PnpDevice {
    fn key(&self) -> String {
        self.id.to_ascii_uppercase()
    }

    fn enumerator(&self) -> String {
        self.id
            .split('\\')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase()
    }

    fn is_class(&self, class: &str) -> bool {
        self.class.eq_ignore_ascii_case(class)
    }

    fn ids(&self) -> PnpIds {
        parse_pnp_id(&self.id)
    }

    /// PCI class code from the compatible ids (`PCI\CC_0C0330`), preferring
    /// the form that carries the programming interface.
    fn pci_class(&self) -> Option<PciClass> {
        let classes: Vec<PciClass> = self
            .compatible_ids
            .iter()
            .chain(&self.hardware_ids)
            .filter_map(|id| pci_class_from_compatible_id(id))
            .collect();
        classes
            .iter()
            .find(|c| c.prog_if.is_some())
            .or_else(|| classes.first())
            .copied()
    }

    fn has_id_containing(&self, needle: &str) -> bool {
        self.hardware_ids
            .iter()
            .chain(&self.compatible_ids)
            .any(|id| id.to_ascii_uppercase().contains(needle))
    }

    fn service_is(&self, service: &str) -> bool {
        self.service
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case(service))
    }
}

/// `DEVPKEY_Device_*` values of one device.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceProps {
    pub location_paths: Vec<String>,
    pub bios_name: Option<String>,
    /// Driver key below the class key ("{4d36e968-…}\\0001").
    pub driver: Option<String>,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inventory {
    /// CIM sections that failed (`"cpu: <message>"`).
    pub errors: Vec<String>,
    pub processors: Vec<Processor>,
    pub system_manufacturer: Option<String>,
    pub system_model: Option<String>,
    /// `Win32_ComputerSystem.HypervisorPresent`. Kept as a raw fact only:
    /// it is also true on bare metal with Hyper-V / VBS (the Windows 11
    /// default), so it never marks a machine as virtual on its own.
    pub hypervisor_present: Option<bool>,
    pub total_physical_memory: Option<u64>,
    pub board_manufacturer: Option<String>,
    pub board_product: Option<String>,
    pub enclosure_manufacturer: Option<String>,
    pub chassis_types: Vec<u32>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
    pub batteries: Vec<Battery>,
    pub memory_modules: Vec<u64>,
    pub adapters: Vec<NetAdapter>,
    pub disks: Vec<Disk>,
    /// `MSFT_PhysicalDisk.BusType` by disk number.
    pub bus_types: HashMap<String, u64>,
    pub devices: Vec<PnpDevice>,
    /// Device properties by upper-case instance id.
    pub props: HashMap<String, DeviceProps>,
}

/// Items of a JSON list. Windows PowerShell may emit a lone object instead of
/// a one-element array, or wrap an array as `{"value": [...], "Count": n}`.
fn items(value: Option<&Value>) -> Vec<&Value> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(v @ Value::Object(o)) => match (o.get("value"), o.contains_key("Count")) {
            (Some(Value::Array(a)), true) => a.iter().collect(),
            _ => vec![v],
        },
        Some(v) => vec![v],
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => clean_text(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn number(v: &Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    items(v)
        .into_iter()
        .filter_map(|item| match item {
            Value::String(s) => clean_text(s),
            _ => None,
        })
        .collect()
}

pub fn parse(json: &str) -> Result<Inventory, AppError> {
    let root: Value = serde_json::from_str(json).map_err(|e| {
        AppError::new(
            "SCAN_PARSE",
            format!("The Windows device inventory is not valid JSON: {e}"),
        )
    })?;
    let section = |name: &str| items(root.get(name));
    let first = |name: &str| {
        section(name)
            .into_iter()
            .next()
            .cloned()
            .unwrap_or(Value::Null)
    };

    let system = first("system");
    let board = first("board");
    let bios = first("bios");
    let enclosures = section("enclosure");
    let mut inv = Inventory {
        errors: strings(root.get("errors")),
        processors: section("cpu")
            .into_iter()
            .map(|p| Processor {
                name: text(p, "Name"),
                manufacturer: text(p, "Manufacturer"),
                description: text(p, "Description"),
                cores: number(p, "NumberOfCores"),
                threads: number(p, "NumberOfLogicalProcessors"),
                max_clock_mhz: number(p, "MaxClockSpeed"),
            })
            .collect(),
        system_manufacturer: text(&system, "Manufacturer"),
        system_model: text(&system, "Model"),
        hypervisor_present: system.get("HypervisorPresent").and_then(Value::as_bool),
        total_physical_memory: number(&system, "TotalPhysicalMemory"),
        board_manufacturer: text(&board, "Manufacturer"),
        board_product: text(&board, "Product"),
        enclosure_manufacturer: enclosures.iter().find_map(|e| text(e, "Manufacturer")),
        chassis_types: Vec::new(),
        bios_vendor: text(&bios, "Manufacturer"),
        bios_version: text(&bios, "SMBIOSBIOSVersion"),
        bios_date: text(&bios, "ReleaseDate"),
        batteries: section("battery")
            .into_iter()
            .map(|b| Battery {
                label: [text(b, "DeviceID"), text(b, "Name")]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
                chemistry: number(b, "Chemistry"),
                pnp_id: text(b, "PNPDeviceID"),
            })
            .collect(),
        memory_modules: section("memory")
            .into_iter()
            .filter_map(|m| number(m, "Capacity"))
            .collect(),
        adapters: section("adapters")
            .into_iter()
            .filter_map(|a| {
                Some(NetAdapter {
                    pnp_id: text(a, "PNPDeviceID")?,
                    mac: text(a, "MACAddress").and_then(|m| normalize_mac(&m)),
                })
            })
            .collect(),
        disks: section("disks")
            .into_iter()
            .map(|d| Disk {
                index: number(d, "Index"),
                model: text(d, "Model"),
                size_bytes: number(d, "Size"),
                pnp_id: text(d, "PNPDeviceID"),
                interface: text(d, "InterfaceType"),
            })
            .collect(),
        bus_types: section("physicalDisks")
            .into_iter()
            .filter_map(|d| Some((text(d, "DeviceId")?, number(d, "BusType")?)))
            .collect(),
        devices: section("devices")
            .into_iter()
            .filter_map(|d| {
                Some(PnpDevice {
                    id: text(d, "PNPDeviceID")?,
                    name: text(d, "Name").unwrap_or_default(),
                    class: text(d, "PNPClass").unwrap_or_default(),
                    service: text(d, "Service"),
                    hardware_ids: strings(d.get("HardwareID")),
                    compatible_ids: strings(d.get("CompatibleID")),
                })
            })
            .collect(),
        props: HashMap::new(),
    };
    for enclosure in enclosures {
        for item in items(enclosure.get("ChassisTypes")) {
            if let Some(t) = item.as_u64().and_then(|t| u32::try_from(t).ok()) {
                if !inv.chassis_types.contains(&t) {
                    inv.chassis_types.push(t);
                }
            }
        }
    }
    for p in section("properties") {
        let (Some(id), Some(key)) = (text(p, "Id"), text(p, "Key")) else {
            continue;
        };
        let entry = inv.props.entry(id.to_ascii_uppercase()).or_default();
        match key.trim_start_matches("DEVPKEY_Device_") {
            "LocationPaths" => entry.location_paths = strings(p.get("Data")),
            "BiosDeviceName" => entry.bios_name = text(p, "Data"),
            "Driver" => entry.driver = text(p, "Data"),
            "Parent" => entry.parent = text(p, "Data"),
            _ => {}
        }
    }
    Ok(inv)
}

// ─── In-process facts ───────────────────────────────────────────────────────

/// Facts read without PowerShell (CPUID, Win32 and registry calls).
#[derive(Debug, Clone, Default)]
pub struct NativeFacts {
    pub cpuid: Option<CpuidInfo>,
    pub topology: Option<ProcessorTopology>,
    pub installed_memory_kb: Option<u64>,
    pub uefi: Option<bool>,
    pub secure_boot: Option<bool>,
    /// Dedicated video memory by upper-case display driver key.
    pub vram_by_driver: HashMap<String, u64>,
    /// Root hub ports by upper-case USB host controller instance id.
    pub usb_ports: HashMap<String, Vec<UsbPortInfo>>,
}

// ─── Assembly ───────────────────────────────────────────────────────────────

/// Inventory plus native facts → `DetectedHardware` (no ACPI dump, warnings
/// only for failed inventory sections).
pub fn assemble(inv: &Inventory, native: &NativeFacts) -> DetectedHardware {
    let view = View::new(inv);
    let motherboard = motherboard(inv, &view);
    let hypervisor = resolve_hypervisor(
        native.cpuid.as_ref(),
        motherboard.system_manufacturer.as_deref(),
        motherboard.system_product.as_deref(),
    );
    DetectedHardware {
        host_os: "windows".into(),
        cpu: cpu(inv, native),
        gpus: view.gpus(native),
        audio: view.audio(),
        network: view.network(),
        input: view.input(),
        memory: memory(inv, native),
        motherboard,
        storage: view.storage(),
        usb_controllers: view.usb_controllers(native),
        chassis: chassis(inv, &view),
        firmware: FirmwareInfo {
            uefi: native.uefi,
            secure_boot: native.secure_boot,
            bios_vendor: inv.bios_vendor.as_deref().and_then(clean_dmi),
            bios_version: inv.bios_version.as_deref().and_then(clean_dmi),
            bios_date: inv.bios_date.clone(),
        },
        acpi_tables_dir: None,
        hypervisor,
        warnings: inv
            .errors
            .iter()
            .map(|e| format!("Windows device query failed ({e})"))
            .collect(),
    }
}

/// Display driver keys whose `HardwareInformation.*MemorySize` should be read.
pub fn display_driver_keys(inv: &Inventory) -> Vec<String> {
    let view = View::new(inv);
    view.pci()
        .filter(|(_, class)| class.is_some_and(|c| c.base == 0x03))
        .filter_map(|(d, _)| view.props(d).and_then(|p| p.driver.clone()))
        .collect()
}

fn cpu(inv: &Inventory, native: &NativeFacts) -> CpuInfo {
    let wmi = inv.processors.first();
    let mut cpu = CpuInfo::default();
    match &native.cpuid {
        Some(id) => {
            cpu.vendor = id.vendor.clone();
            cpu.name = id.brand.clone();
            cpu.family = Some(id.family);
            cpu.model = Some(id.model);
            cpu.stepping = Some(id.stepping);
            cpu.features = id.features.clone();
        }
        None => {
            cpu.vendor = wmi.and_then(|p| p.manufacturer.clone()).unwrap_or_default();
            if let Some((family, model, stepping)) = wmi
                .and_then(|p| p.description.as_deref())
                .and_then(parse_processor_description)
            {
                cpu.family = Some(family);
                cpu.model = Some(model);
                cpu.stepping = Some(stepping);
            }
        }
    }
    if cpu.name.is_empty() {
        cpu.name = wmi.and_then(|p| p.name.clone()).unwrap_or_default();
    }
    let sum = |f: fn(&Processor) -> Option<u64>| -> u32 {
        inv.processors
            .iter()
            .filter_map(f)
            .sum::<u64>()
            .try_into()
            .unwrap_or(u32::MAX)
    };
    let topology = native.topology.filter(|t| t.cores > 0);
    cpu.cores = topology
        .map(|t| t.cores)
        .unwrap_or_else(|| sum(|p| p.cores));
    cpu.threads = topology
        .map(|t| t.threads)
        .filter(|t| *t > 0)
        .unwrap_or_else(|| sum(|p| p.threads));
    cpu.packages = topology
        .map(|t| t.packages)
        .filter(|p| *p > 0)
        .unwrap_or(inv.processors.len() as u32)
        .max(1);
    cpu.base_clock_mhz = base_clock_from_brand(&cpu.name).or_else(|| {
        wmi.and_then(|p| p.max_clock_mhz)
            .and_then(|m| u32::try_from(m).ok())
            .filter(|m| *m > 0)
    });
    cpu
}

fn memory(inv: &Inventory, native: &NativeFacts) -> MemoryInfo {
    let modules: u64 = inv.memory_modules.iter().sum();
    let bytes = native
        .installed_memory_kb
        .map(|kb| kb.saturating_mul(1024))
        .filter(|b| *b > 0)
        .or((modules > 0).then_some(modules))
        .or(inv.total_physical_memory)
        .unwrap_or(0);
    MemoryInfo {
        total_mb: bytes / (1024 * 1024),
    }
}

fn motherboard(inv: &Inventory, view: &View) -> MotherboardInfo {
    let lpc = view
        .pci()
        .find(|(_, class)| class.is_some_and(|c| c.is(0x06, 0x01)))
        .map(|(d, _)| d.ids());
    MotherboardInfo {
        manufacturer: inv.board_manufacturer.as_deref().and_then(clean_dmi),
        product: inv.board_product.as_deref().and_then(clean_dmi),
        system_manufacturer: inv.system_manufacturer.as_deref().and_then(clean_dmi),
        system_product: inv.system_model.as_deref().and_then(clean_dmi),
        lpc_vendor_id: lpc.as_ref().and_then(|i| i.vendor_id.clone()),
        lpc_device_id: lpc.and_then(|i| i.device_id),
        chipset: None,
    }
}

fn chassis(inv: &Inventory, view: &View) -> ChassisInfo {
    ChassisInfo {
        chassis_types: inv.chassis_types.clone(),
        manufacturer: inv.enclosure_manufacturer.as_deref().and_then(clean_dmi),
        has_battery: inv.batteries.iter().any(|b| !b.is_ups()),
        has_lid: view
            .devices
            .iter()
            .any(|d| d.key().starts_with(r"ACPI\PNP0C0D")),
    }
}

/// Lookups over the device list.
struct View<'a> {
    inv: &'a Inventory,
    devices: &'a [PnpDevice],
    by_id: HashMap<String, &'a PnpDevice>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Usage {
    Mouse,
    Keyboard,
    Touchscreen,
    Touchpad,
}

impl<'a> View<'a> {
    fn new(inv: &'a Inventory) -> Self {
        let by_id = inv.devices.iter().map(|d| (d.key(), d)).collect();
        Self {
            inv,
            devices: &inv.devices,
            by_id,
        }
    }

    fn props(&self, device: &PnpDevice) -> Option<&'a DeviceProps> {
        self.inv.props.get(&device.key())
    }

    /// PCI functions with their class code.
    fn pci(&self) -> impl Iterator<Item = (&'a PnpDevice, Option<PciClass>)> + '_ {
        self.devices
            .iter()
            .filter(|d| d.enumerator() == "PCI")
            .map(|d| (d, d.pci_class()))
    }

    fn location(&self, device: &PnpDevice) -> PciLocation {
        let Some(props) = self.props(device) else {
            return PciLocation::default();
        };
        PciLocation {
            pci_path: props
                .location_paths
                .iter()
                .find_map(|p| location_path_to_device_path(p)),
            acpi_path: props
                .bios_name
                .as_deref()
                .map(normalize_acpi_path)
                .or_else(|| {
                    props
                        .location_paths
                        .iter()
                        .find_map(|p| location_path_to_acpi_path(p))
                }),
        }
    }

    /// Nearest PCI ancestor (a codec's HD Audio controller, a disk's storage
    /// controller), following `DEVPKEY_Device_Parent`.
    fn pci_ancestor(&self, device: &PnpDevice) -> Option<&'a PnpDevice> {
        let mut current = self.props(device)?.parent.clone()?;
        for _ in 0..8 {
            let key = current.to_ascii_uppercase();
            let parent = self.by_id.get(&key).copied();
            if key.starts_with(r"PCI\") {
                return parent;
            }
            current = self.inv.props.get(&key)?.parent.clone()?;
        }
        None
    }

    fn mac_for(&self, device: &PnpDevice) -> Option<String> {
        self.inv
            .adapters
            .iter()
            .filter(|a| a.pnp_id.eq_ignore_ascii_case(&device.id))
            .find_map(|a| a.mac.clone())
    }

    fn gpus(&self, native: &NativeFacts) -> Vec<GpuInfo> {
        self.pci()
            .filter(|(d, class)| match class {
                Some(c) => c.base == 0x03,
                None => d.is_class("Display"),
            })
            .map(|(d, _)| {
                let ids = d.ids();
                let vram = self
                    .props(d)
                    .and_then(|p| p.driver.as_ref())
                    .and_then(|key| native.vram_by_driver.get(&key.to_ascii_uppercase()));
                GpuInfo {
                    name: display_name(d, &ids, "Display controller"),
                    vendor_id: ids.vendor_id,
                    device_id: ids.device_id,
                    subsystem_vendor_id: ids.subsystem_vendor_id,
                    subsystem_device_id: ids.subsystem_device_id,
                    revision: ids.revision,
                    vram_mb: vram.map(|b| b / (1024 * 1024)).filter(|mb| *mb > 0),
                    location: self.location(d),
                }
            })
            .collect()
    }

    fn audio(&self) -> Vec<AudioDevice> {
        let mut out = Vec::new();
        let mut used_controllers = HashSet::new();
        for d in self.devices {
            let enumerator = d.enumerator();
            let bus = match enumerator.as_str() {
                "HDAUDIO" => "hdaudio",
                "INTELAUDIO" => "sst",
                _ => continue,
            };
            // FUNC_01 is the audio function group; FUNC_02 is a modem.
            if !d.key().contains("FUNC_01") {
                continue;
            }
            let ids = d.ids();
            let (Some(vendor), Some(device)) = (ids.vendor_id.clone(), ids.device_id.clone())
            else {
                continue;
            };
            let controller = self.pci_ancestor(d);
            if let Some(c) = controller {
                used_controllers.insert(c.key());
            }
            let controller_ids = controller.map(PnpDevice::ids);
            out.push(AudioDevice {
                name: display_name(d, &ids, "HD Audio codec"),
                is_hdmi: is_hdmi_codec_vendor(&vendor),
                codec_vendor_id: Some(vendor),
                codec_device_id: Some(device),
                codec_subsystem_id: match (&ids.subsystem_vendor_id, &ids.subsystem_device_id) {
                    (Some(v), Some(d)) => Some(format!("{v}{d}")),
                    _ => None,
                },
                controller_vendor_id: controller_ids.as_ref().and_then(|i| i.vendor_id.clone()),
                controller_device_id: controller_ids.and_then(|i| i.device_id),
                location: controller.map(|c| self.location(c)).unwrap_or_default(),
                bus: bus.into(),
            });
        }
        let mut usb_seen = HashSet::new();
        for d in self
            .devices
            .iter()
            .filter(|d| d.is_class("MEDIA") && d.enumerator() == "USB")
        {
            let ids = d.ids();
            if usb_seen.insert((ids.vendor_id.clone(), ids.device_id.clone())) {
                out.push(AudioDevice {
                    name: display_name(d, &ids, "USB audio"),
                    bus: "usb".into(),
                    ..Default::default()
                });
            }
        }
        // Controllers whose codecs are not visible (no driver, HDMI audio of a
        // GPU without its driver, DSP-mode Intel controllers).
        for (d, class) in self.pci() {
            let Some(class) = class.filter(|c| c.base == 0x04) else {
                continue;
            };
            if used_controllers.contains(&d.key()) {
                continue;
            }
            let ids = d.ids();
            let bus = if class.is(0x04, 0x03) {
                "hdaudio"
            } else if ids.vendor_id.as_deref() == Some("8086")
                && (class.sub == 0x01 || class.sub == 0x80)
            {
                "sst"
            } else {
                continue;
            };
            out.push(AudioDevice {
                name: display_name(d, &ids, "Audio controller"),
                controller_vendor_id: ids.vendor_id,
                controller_device_id: ids.device_id,
                location: self.location(d),
                bus: bus.into(),
                ..Default::default()
            });
        }
        out
    }

    fn network(&self) -> Vec<NetworkDevice> {
        let mut out: Vec<NetworkDevice> = Vec::new();
        for (d, class) in self.pci() {
            let kind = match class {
                Some(c) if c.base == 0x02 => match c.sub {
                    0x00 => NetworkKind::Ethernet,
                    0x80 => NetworkKind::Wifi,
                    _ => NetworkKind::Other,
                },
                None if d.is_class("Net") => wireless_kind(&d.name),
                _ => continue,
            };
            let ids = d.ids();
            out.push(NetworkDevice {
                name: display_name(d, &ids, "Network controller"),
                kind,
                bus: "pci".into(),
                mac_address: self.mac_for(d),
                location: self.location(d),
                vendor_id: ids.vendor_id,
                device_id: ids.device_id,
                subsystem_vendor_id: ids.subsystem_vendor_id,
                subsystem_device_id: ids.subsystem_device_id,
            });
        }
        let mut seen = HashSet::new();
        for d in self
            .devices
            .iter()
            .filter(|d| d.is_class("Net") || d.is_class("Bluetooth"))
        {
            let bus = match d.enumerator().as_str() {
                "USB" => "usb",
                "SD" => "sdio",
                // Radios on a UART (ACPI-enumerated, e.g. BCM2E7C).
                "ACPI" if d.is_class("Bluetooth") => "other",
                _ => continue,
            };
            let ids = d.ids();
            let kind = if d.is_class("Bluetooth") {
                NetworkKind::Bluetooth
            } else {
                wireless_kind(&d.name)
            };
            if !seen.insert((
                format!("{kind:?}"),
                ids.vendor_id.clone(),
                ids.device_id.clone(),
            )) {
                continue;
            }
            out.push(NetworkDevice {
                name: display_name(d, &ids, "Network adapter"),
                kind,
                bus: bus.into(),
                mac_address: if kind == NetworkKind::Bluetooth {
                    None
                } else {
                    self.mac_for(d)
                },
                vendor_id: ids.vendor_id,
                device_id: ids.device_id,
                ..Default::default()
            });
        }
        out
    }

    fn input(&self) -> Vec<InputDevice> {
        // Usages reported by each HID collection, keyed by its parent.
        let mut usages: HashMap<String, Vec<Usage>> = HashMap::new();
        let mut out: Vec<InputDevice> = Vec::new();
        let mut external: Vec<(InputDevice, Option<String>, Option<String>)> = Vec::new();
        for d in self.devices.iter().filter(|d| d.enumerator() == "HID") {
            let Some(usage) = hid_usage(d) else { continue };
            let parent = self
                .props(d)
                .and_then(|p| p.parent.clone())
                .map(|p| p.to_ascii_uppercase());
            let ids = d.ids();
            let key = d.key();
            let bluetooth =
                key.contains("{00001124-") || key.contains("{00001812-") || key.contains("_VID&");
            if bluetooth || ids.vendor_id.is_some() {
                let device = InputDevice {
                    name: display_name(d, &ids, "HID device"),
                    kind: usage.kind(),
                    bus: if bluetooth { "bluetooth" } else { "usb" }.into(),
                    hardware_id: None,
                    vendor: input_vendor(None, &d.name),
                };
                external.push((device, ids.vendor_id, ids.device_id));
                continue;
            }
            let owner =
                parent.or_else(|| hid_from_collection_id(&d.id).map(|h| format!("HID:{h}")));
            if let Some(owner) = owner {
                usages.entry(owner).or_default().push(usage);
            }
        }

        for d in self.devices.iter().filter(|d| d.enumerator() == "ACPI") {
            let ps2 = d.is_class("Keyboard") || d.is_class("Mouse");
            let i2c = d.is_class("HIDClass")
                && (d.service_is("hidi2c")
                    || d.has_id_containing("PNP0C50")
                    || d.has_id_containing("ACPI0C50"));
            if !(ps2 || i2c) {
                continue;
            }
            let hid = acpi_hid_from_pnp_id(&d.id);
            let vendor = input_vendor(hid.as_deref(), &d.name);
            let kind = if ps2 {
                if d.is_class("Keyboard") {
                    InputKind::Keyboard
                } else if is_touchpad_vendor(vendor.as_deref()) || is_touchpad_name(&d.name) {
                    InputKind::Touchpad
                } else {
                    InputKind::Mouse
                }
            } else {
                let mut seen = usages.get(&d.key()).cloned().unwrap_or_default();
                if let Some(h) = &hid {
                    seen.extend(
                        usages
                            .get(&format!("HID:{h}"))
                            .into_iter()
                            .flatten()
                            .copied(),
                    );
                }
                match seen.iter().max() {
                    Some(Usage::Mouse) if is_touchpad_vendor(vendor.as_deref()) => {
                        InputKind::Touchpad
                    }
                    Some(usage) => usage.kind(),
                    None => InputKind::Other,
                }
            };
            out.push(InputDevice {
                name: display_name(
                    d,
                    &d.ids(),
                    if ps2 { "PS/2 device" } else { "I2C HID device" },
                ),
                kind,
                bus: if ps2 { "ps2" } else { "i2c" }.into(),
                hardware_id: hid,
                vendor,
            });
        }

        let mut seen = HashSet::new();
        for (device, vendor, product) in external {
            if seen.insert((
                format!("{:?}", device.kind),
                device.bus.clone(),
                vendor,
                product,
            )) {
                out.push(device);
            }
        }
        out
    }

    fn storage(&self) -> Vec<StorageDevice> {
        let mut out = Vec::new();
        let mut used = HashSet::new();
        for disk in &self.inv.disks {
            let id = disk.pnp_id.clone().unwrap_or_default().to_ascii_uppercase();
            let bus_type = disk
                .index
                .and_then(|i| self.inv.bus_types.get(&i.to_string()).copied());
            // File-backed virtual disks (VHD mounts) and Storage Spaces.
            if id.starts_with(r"ROOT\") || matches!(bus_type, Some(14..=16)) {
                continue;
            }
            let controller = self.by_id.get(&id).and_then(|d| self.pci_ancestor(d));
            if let Some(c) = controller {
                used.insert(c.key());
            }
            let controller_kind = controller
                .and_then(PnpDevice::pci_class)
                .and_then(storage_kind_from_class);
            let kind = match bus_type {
                Some(17) => "nvme",
                Some(3 | 11) => "sata",
                Some(7) => "usb",
                Some(8) => "raid",
                Some(13) => "emmc",
                _ if id.starts_with(r"USBSTOR\") || disk.interface.as_deref() == Some("USB") => {
                    "usb"
                }
                _ => controller_kind.unwrap_or("other"),
            };
            let ids = controller.map(PnpDevice::ids);
            out.push(StorageDevice {
                name: disk.model.clone().unwrap_or_else(|| "Disk".into()),
                kind: kind.into(),
                controller_vendor_id: ids.as_ref().and_then(|i| i.vendor_id.clone()),
                controller_device_id: ids.and_then(|i| i.device_id),
                size_bytes: disk.size_bytes.filter(|s| *s > 0),
            });
        }
        for (d, class) in self.pci() {
            let Some(kind) = class.and_then(storage_kind_from_class) else {
                continue;
            };
            if used.contains(&d.key()) {
                continue;
            }
            let ids = d.ids();
            out.push(StorageDevice {
                name: display_name(d, &ids, "Storage controller"),
                kind: kind.into(),
                controller_vendor_id: ids.vendor_id,
                controller_device_id: ids.device_id,
                size_bytes: None,
            });
        }
        out
    }

    fn usb_controllers(&self, native: &NativeFacts) -> Vec<UsbControllerInfo> {
        self.pci()
            .filter(|(_, class)| class.is_some_and(|c| c.is(0x0c, 0x03)))
            .map(|(d, class)| {
                let ids = d.ids();
                let name = display_name(d, &ids, "USB controller");
                UsbControllerInfo {
                    kind: usb_controller_kind(class.and_then(|c| c.prog_if), &name).into(),
                    name,
                    vendor_id: ids.vendor_id,
                    device_id: ids.device_id,
                    location: self.location(d),
                    ports: native.usb_ports.get(&d.key()).cloned().unwrap_or_default(),
                }
            })
            .collect()
    }
}

impl Usage {
    fn kind(self) -> InputKind {
        match self {
            Usage::Mouse => InputKind::Mouse,
            Usage::Keyboard => InputKind::Keyboard,
            Usage::Touchscreen => InputKind::Touchscreen,
            Usage::Touchpad => InputKind::Touchpad,
        }
    }
}

/// Top-level usage of a HID collection from its generic hardware ids.
fn hid_usage(device: &PnpDevice) -> Option<Usage> {
    if device.has_id_containing("HID_DEVICE_UP:000D_U:0005") {
        Some(Usage::Touchpad)
    } else if device.has_id_containing("HID_DEVICE_UP:000D_U:0004") {
        Some(Usage::Touchscreen)
    } else if device.has_id_containing("HID_DEVICE_SYSTEM_KEYBOARD")
        || device.has_id_containing("HID_DEVICE_UP:0001_U:0006")
    {
        Some(Usage::Keyboard)
    } else if device.has_id_containing("HID_DEVICE_SYSTEM_MOUSE")
        || device.has_id_containing("HID_DEVICE_UP:0001_U:0002")
    {
        Some(Usage::Mouse)
    } else if device.is_class("Keyboard") {
        Some(Usage::Keyboard)
    } else if device.is_class("Mouse") {
        Some(Usage::Mouse)
    } else {
        None
    }
}

/// ACPI id of the I2C device behind a HID collection:
/// "HID\\VEN_SYNA&DEV_2393&Col02\\…" or "HID\\SYNA2393&Col02\\…" → "SYNA2393".
fn hid_from_collection_id(id: &str) -> Option<String> {
    let body = id.split('\\').nth(1)?.to_ascii_uppercase();
    let mut tokens = body.split('&');
    let first = tokens.next()?;
    if let Some(vendor) = first.strip_prefix("VEN_") {
        let device = tokens.next()?.strip_prefix("DEV_")?;
        return Some(format!("{vendor}{device}"));
    }
    let plausible =
        (7..=9).contains(&first.len()) && first.chars().all(|c| c.is_ascii_alphanumeric());
    plausible.then(|| first.to_string())
}

fn is_touchpad_vendor(vendor: Option<&str>) -> bool {
    matches!(
        vendor,
        Some("synaptics" | "elan" | "alps" | "focaltech" | "cypress")
    )
}

fn is_touchpad_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "touchpad",
        "touch pad",
        "trackpad",
        "clickpad",
        "glidepoint",
    ]
    .iter()
    .any(|n| lower.contains(n))
}

fn wireless_kind(name: &str) -> NetworkKind {
    let lower = name.to_lowercase();
    let wireless = ["wi-fi", "wifi", "wireless", "wlan", "802.11"]
        .iter()
        .any(|n| lower.contains(n));
    if wireless {
        NetworkKind::Wifi
    } else {
        NetworkKind::Ethernet
    }
}

fn display_name(device: &PnpDevice, ids: &PnpIds, fallback: &str) -> String {
    clean_text(&device.name).unwrap_or_else(|| match (&ids.vendor_id, &ids.device_id) {
        (Some(v), Some(d)) => format!("{fallback} {v}:{d}"),
        _ => fallback.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed inventory of a Z390 desktop: Coffee Lake iGPU + RX 580,
    /// ALC1220 + RX 580 HDMI codec, I219-V, an Intel Bluetooth radio on USB,
    /// NVMe and SATA disks, a USB stick, a USB keyboard and mouse, and a UPS
    /// that Windows lists as a battery.
    const DESKTOP: &str = r#"{
 "errors": [],
 "cpu": [{"Name":"Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz","Manufacturer":"GenuineIntel","Description":"Intel64 Family 6 Model 158 Stepping 13","NumberOfCores":8,"NumberOfLogicalProcessors":8,"MaxClockSpeed":3600}],
 "system": [{"Manufacturer":"System manufacturer","Model":"System Product Name","TotalPhysicalMemory":34282340352,"HypervisorPresent":true}],
 "board": [{"Manufacturer":"ASUSTeK COMPUTER INC.","Product":"PRIME Z390-A"}],
 "enclosure": [{"Manufacturer":"Default string","ChassisTypes":[3]}],
 "bios": [{"Manufacturer":"American Megatrends Inc.","SMBIOSBIOSVersion":"2417","ReleaseDate":"06/03/2021"}],
 "battery": [{"DeviceID":"CPS CP1500PFCLCD","Name":"CP1500PFCLCD","Chemistry":3,"PNPDeviceID":"HID\\VID_0764&PID_0501\\6&2B7E6A1&0&0000"}],
 "memory": [{"Capacity":17179869184},{"Capacity":17179869184}],
 "adapters": [
  {"Name":"Intel(R) Ethernet Connection (7) I219-V","PNPDeviceID":"PCI\\VEN_8086&DEV_15BC&SUBSYS_86721043&REV_10\\3&11583659&0&FE","MACAddress":"A4:BB:6D:12:34:56","PhysicalAdapter":true},
  {"Name":"WAN Miniport (IP)","PNPDeviceID":"SWD\\MSRRAS\\MS_NDISWANIP","MACAddress":null,"PhysicalAdapter":false}
 ],
 "disks": [
  {"Index":0,"Model":"Samsung SSD 970 EVO Plus 1TB","Size":1000202273280,"PNPDeviceID":"SCSI\\DISK&VEN_NVME&PROD_SAMSUNG_SSD_970\\5&2C4E9BBF&0&000000","InterfaceType":"SCSI"},
  {"Index":1,"Model":"Samsung SSD 860 EVO 500GB","Size":500105249280,"PNPDeviceID":"SCSI\\DISK&VEN_&PROD_SAMSUNG_SSD_860\\4&2B3B4C5&0&010000","InterfaceType":"IDE"},
  {"Index":2,"Model":"SanDisk Cruzer Blade USB Device","Size":15997464576,"PNPDeviceID":"USBSTOR\\DISK&VEN_SANDISK&PROD_CRUZER_BLADE&REV_1.00\\4C530001","InterfaceType":"USB"}
 ],
 "physicalDisks": [{"DeviceId":"0","BusType":17},{"DeviceId":"1","BusType":11},{"DeviceId":"2","BusType":7}],
 "devices": [
  {"Name":"Intel(R) UHD Graphics 630","PNPClass":"Display","PNPDeviceID":"PCI\\VEN_8086&DEV_3E98&SUBSYS_86941043&REV_02\\3&11583659&0&10","Service":"igfx","HardwareID":["PCI\\VEN_8086&DEV_3E98&SUBSYS_86941043&REV_02"],"CompatibleID":["PCI\\VEN_8086&DEV_3E98&REV_02","PCI\\VEN_8086&CC_030000","PCI\\VEN_8086&CC_0300","PCI\\CC_030000","PCI\\CC_0300"]},
  {"Name":"Radeon RX 580 Series","PNPClass":"Display","PNPDeviceID":"PCI\\VEN_1002&DEV_67DF&SUBSYS_E3531DA2&REV_E7\\4&2B8EBBA8&0&0008","Service":"amdkmdag","HardwareID":["PCI\\VEN_1002&DEV_67DF&SUBSYS_E3531DA2&REV_E7"],"CompatibleID":["PCI\\VEN_1002&CC_030000","PCI\\CC_0300"]},
  {"Name":"High Definition Audio Controller","PNPClass":"MEDIA","PNPDeviceID":"PCI\\VEN_8086&DEV_A348&SUBSYS_86941043&REV_10\\3&11583659&0&FB","Service":"HDAudBus","HardwareID":["PCI\\VEN_8086&DEV_A348&SUBSYS_86941043&REV_10"],"CompatibleID":["PCI\\VEN_8086&CC_040300","PCI\\CC_0403"]},
  {"Name":"High Definition Audio Controller","PNPClass":"MEDIA","PNPDeviceID":"PCI\\VEN_1002&DEV_AAF0&SUBSYS_AAF01DA2&REV_00\\4&2B8EBBA8&0&0108","Service":"HDAudBus","HardwareID":["PCI\\VEN_1002&DEV_AAF0&SUBSYS_AAF01DA2&REV_00"],"CompatibleID":["PCI\\VEN_1002&CC_040300","PCI\\CC_0403"]},
  {"Name":"Realtek High Definition Audio","PNPClass":"MEDIA","PNPDeviceID":"HDAUDIO\\FUNC_01&VEN_10EC&DEV_1220&SUBSYS_10438694&REV_1001\\4&3A2D5A0D&0&0001","Service":"IntcAzAudAddService","HardwareID":["HDAUDIO\\FUNC_01&VEN_10EC&DEV_1220&SUBSYS_10438694&REV_1001"],"CompatibleID":["HDAUDIO\\FUNC_01&VEN_10EC&DEV_1220&REV_1001"]},
  {"Name":"AMD High Definition Audio Device","PNPClass":"MEDIA","PNPDeviceID":"HDAUDIO\\FUNC_01&VEN_1002&DEV_AA01&SUBSYS_00AA0100&REV_1008\\5&1E4E0E5A&0&0001","Service":"AtiHDAudioService","HardwareID":[],"CompatibleID":[]},
  {"Name":"Intel(R) Ethernet Connection (7) I219-V","PNPClass":"Net","PNPDeviceID":"PCI\\VEN_8086&DEV_15BC&SUBSYS_86721043&REV_10\\3&11583659&0&FE","Service":"e1dexpress","HardwareID":[],"CompatibleID":["PCI\\VEN_8086&CC_020000","PCI\\CC_0200"]},
  {"Name":"Intel(R) Wireless Bluetooth(R)","PNPClass":"Bluetooth","PNPDeviceID":"USB\\VID_8087&PID_0AAA\\5&2A4C1B1&0&14","Service":"BTHUSB","HardwareID":["USB\\VID_8087&PID_0AAA&REV_0002"],"CompatibleID":["USB\\Class_E0&SubClass_01&Prot_01"]},
  {"Name":"Intel(R) USB 3.1 eXtensible Host Controller - 1.10 (Microsoft)","PNPClass":"USB","PNPDeviceID":"PCI\\VEN_8086&DEV_A36D&SUBSYS_86941043&REV_10\\3&11583659&0&A0","Service":"USBXHCI","HardwareID":[],"CompatibleID":["PCI\\VEN_8086&CC_0C0330","PCI\\CC_0C03"]},
  {"Name":"Standard NVM Express Controller","PNPClass":"SCSIAdapter","PNPDeviceID":"PCI\\VEN_144D&DEV_A808&SUBSYS_A801144D&REV_00\\4&1E2A9C3&0&00E8","Service":"stornvme","HardwareID":[],"CompatibleID":["PCI\\VEN_144D&CC_010802","PCI\\CC_0108"]},
  {"Name":"Standard SATA AHCI Controller","PNPClass":"HDC","PNPDeviceID":"PCI\\VEN_8086&DEV_A352&SUBSYS_86941043&REV_10\\3&11583659&0&B8","Service":"storahci","HardwareID":[],"CompatibleID":["PCI\\VEN_8086&CC_010601","PCI\\CC_0106"]},
  {"Name":"Intel(R) LPC Controller (Z390) - A305","PNPClass":"System","PNPDeviceID":"PCI\\VEN_8086&DEV_A305&SUBSYS_86941043&REV_10\\3&11583659&0&F8","Service":"msisadrv","HardwareID":[],"CompatibleID":["PCI\\VEN_8086&CC_060100","PCI\\CC_0601"]},
  {"Name":"Samsung SSD 970 EVO Plus 1TB","PNPClass":"DiskDrive","PNPDeviceID":"SCSI\\DISK&VEN_NVME&PROD_SAMSUNG_SSD_970\\5&2C4E9BBF&0&000000","Service":"disk","HardwareID":[],"CompatibleID":[]},
  {"Name":"Samsung SSD 860 EVO 500GB","PNPClass":"DiskDrive","PNPDeviceID":"SCSI\\DISK&VEN_&PROD_SAMSUNG_SSD_860\\4&2B3B4C5&0&010000","Service":"disk","HardwareID":[],"CompatibleID":[]},
  {"Name":"HID Keyboard Device","PNPClass":"Keyboard","PNPDeviceID":"HID\\VID_046D&PID_C52B&MI_00\\8&1A2B3C4D&0&0000","Service":"kbdhid","HardwareID":["HID\\VID_046D&PID_C52B&REV_1211&MI_00","HID_DEVICE_SYSTEM_KEYBOARD","HID_DEVICE_UP:0001_U:0006","HID_DEVICE"],"CompatibleID":[]},
  {"Name":"HID-compliant mouse","PNPClass":"Mouse","PNPDeviceID":"HID\\VID_046D&PID_C52B&MI_01&COL01\\8&2B3C4D5E&0&0000","Service":"mouhid","HardwareID":["HID_DEVICE_SYSTEM_MOUSE","HID_DEVICE_UP:0001_U:0002"],"CompatibleID":[]},
  {"Name":"HID-compliant consumer control device","PNPClass":"HIDClass","PNPDeviceID":"HID\\VID_046D&PID_C52B&MI_01&COL02\\8&2B3C4D5E&0&0001","Service":null,"HardwareID":["HID_DEVICE_UP:000C_U:0001"],"CompatibleID":[]}
 ],
 "properties": [
  {"Id":"PCI\\VEN_8086&DEV_3E98&SUBSYS_86941043&REV_02\\3&11583659&0&10","Key":"DEVPKEY_Device_LocationPaths","Data":["PCIROOT(0)#PCI(0200)","ACPI(_SB_)#ACPI(PCI0)#ACPI(GFX0)"]},
  {"Id":"PCI\\VEN_8086&DEV_3E98&SUBSYS_86941043&REV_02\\3&11583659&0&10","Key":"DEVPKEY_Device_Driver","Data":"{4d36e968-e325-11ce-bfc1-08002be10318}\\0000"},
  {"Id":"PCI\\VEN_1002&DEV_67DF&SUBSYS_E3531DA2&REV_E7\\4&2B8EBBA8&0&0008","Key":"DEVPKEY_Device_LocationPaths","Data":["PCIROOT(0)#PCI(0100)#PCI(0000)","ACPI(_SB_)#ACPI(PCI0)#ACPI(PEG0)#ACPI(PEGP)"]},
  {"Id":"PCI\\VEN_1002&DEV_67DF&SUBSYS_E3531DA2&REV_E7\\4&2B8EBBA8&0&0008","Key":"DEVPKEY_Device_BiosDeviceName","Data":"\\_SB.PCI0.PEG0.PEGP"},
  {"Id":"PCI\\VEN_1002&DEV_67DF&SUBSYS_E3531DA2&REV_E7\\4&2B8EBBA8&0&0008","Key":"DEVPKEY_Device_Driver","Data":"{4d36e968-e325-11ce-bfc1-08002be10318}\\0001"},
  {"Id":"PCI\\VEN_8086&DEV_A348&SUBSYS_86941043&REV_10\\3&11583659&0&FB","Key":"DEVPKEY_Device_LocationPaths","Data":"PCIROOT(0)#PCI(1F03)"},
  {"Id":"PCI\\VEN_8086&DEV_A348&SUBSYS_86941043&REV_10\\3&11583659&0&FB","Key":"DEVPKEY_Device_BiosDeviceName","Data":"\\_SB.PCI0.HDAS"},
  {"Id":"HDAUDIO\\FUNC_01&VEN_10EC&DEV_1220&SUBSYS_10438694&REV_1001\\4&3A2D5A0D&0&0001","Key":"DEVPKEY_Device_Parent","Data":"PCI\\VEN_8086&DEV_A348&SUBSYS_86941043&REV_10\\3&11583659&0&FB"},
  {"Id":"HDAUDIO\\FUNC_01&VEN_1002&DEV_AA01&SUBSYS_00AA0100&REV_1008\\5&1E4E0E5A&0&0001","Key":"DEVPKEY_Device_Parent","Data":"pci\\ven_1002&dev_aaf0&subsys_aaf01da2&rev_00\\4&2b8ebba8&0&0108"},
  {"Id":"PCI\\VEN_8086&DEV_15BC&SUBSYS_86721043&REV_10\\3&11583659&0&FE","Key":"DEVPKEY_Device_LocationPaths","Data":["PCIROOT(0)#PCI(1F06)"]},
  {"Id":"PCI\\VEN_8086&DEV_A36D&SUBSYS_86941043&REV_10\\3&11583659&0&A0","Key":"DEVPKEY_Device_LocationPaths","Data":["PCIROOT(0)#PCI(1400)"]},
  {"Id":"SCSI\\DISK&VEN_NVME&PROD_SAMSUNG_SSD_970\\5&2C4E9BBF&0&000000","Key":"DEVPKEY_Device_Parent","Data":"PCI\\VEN_144D&DEV_A808&SUBSYS_A801144D&REV_00\\4&1E2A9C3&0&00E8"},
  {"Id":"SCSI\\DISK&VEN_&PROD_SAMSUNG_SSD_860\\4&2B3B4C5&0&010000","Key":"DEVPKEY_Device_Parent","Data":"PCI\\VEN_8086&DEV_A352&SUBSYS_86941043&REV_10\\3&11583659&0&B8"},
  {"Id":"HID\\VID_046D&PID_C52B&MI_00\\8&1A2B3C4D&0&0000","Key":"DEVPKEY_Device_Parent","Data":"USB\\VID_046D&PID_C52B&MI_00\\7&3F0C2A&0&0000"}
 ]
}"#;

    /// Trimmed inventory of a Whiskey Lake laptop: Synaptics I2C touchpad,
    /// PS/2 keyboard, Realtek codec on the Intel SST bus, CNVi Wi-Fi, a lid,
    /// a laptop battery. PowerShell emitted the single CPU as an object and
    /// wrapped the device list.
    const LAPTOP: &str = r#"{
 "errors": ["physicalDisks: Invalid namespace"],
 "cpu": {"Name":"Intel(R) Core(TM) i7-8565U CPU @ 1.80GHz","Manufacturer":"GenuineIntel","Description":"Intel64 Family 6 Model 142 Stepping 11","NumberOfCores":4,"NumberOfLogicalProcessors":8,"MaxClockSpeed":1992},
 "system": {"Manufacturer":"LENOVO","Model":"20N2CTO1WW","TotalPhysicalMemory":"16904675328"},
 "board": {"Manufacturer":"LENOVO","Product":"20N2CTO1WW"},
 "enclosure": {"Manufacturer":"LENOVO","ChassisTypes":[10]},
 "bios": {"Manufacturer":"LENOVO","SMBIOSBIOSVersion":"N2IET98W (1.76 )","ReleaseDate":"03/21/2023"},
 "battery": {"DeviceID":"5B10W13930 SMP","Name":"5B10W13930","Chemistry":6,"PNPDeviceID":"ACPI\\PNP0C0A\\1"},
 "memory": [],
 "adapters": [{"Name":"Intel(R) Wireless-AC 9560 160MHz","PNPDeviceID":"PCI\\VEN_8086&DEV_9DF0&SUBSYS_00348086&REV_30\\3&11583659&0&A3","MACAddress":"8C-C6-81-AA-BB-CC","PhysicalAdapter":true}],
 "disks": [],
 "physicalDisks": [],
 "devices": {"value": [
  {"Name":"Intel(R) Wireless-AC 9560 160MHz","PNPClass":"Net","PNPDeviceID":"PCI\\VEN_8086&DEV_9DF0&SUBSYS_00348086&REV_30\\3&11583659&0&A3","Service":"Netwtw08","HardwareID":[],"CompatibleID":["PCI\\VEN_8086&CC_028000","PCI\\CC_0280"]},
  {"Name":"Intel(R) Smart Sound Technology (Intel(R) SST) Audio Controller","PNPClass":"MEDIA","PNPDeviceID":"PCI\\VEN_8086&DEV_9DC8&SUBSYS_229217AA&REV_30\\3&11583659&0&FB","Service":"IntcAudioBus","HardwareID":[],"CompatibleID":["PCI\\VEN_8086&CC_040100","PCI\\CC_0401"]},
  {"Name":"Realtek(R) Audio","PNPClass":"MEDIA","PNPDeviceID":"INTELAUDIO\\FUNC_01&VEN_10EC&DEV_0257&SUBSYS_17AA2292&REV_1000\\4&2D5C3F&0&0001","Service":"IntcAzAudAddService","HardwareID":[],"CompatibleID":[]},
  {"Name":"Intel(R) Smart Sound Technology (Intel(R) SST) OED","PNPClass":"System","PNPDeviceID":"INTELAUDIO\\CTLR_DEV_9DC8&LINKTYPE_06&DEVTYPE_06&VEN_8086&DEV_AE20\\5&1","Service":"IntcOED","HardwareID":[],"CompatibleID":[]},
  {"Name":"Intel(R) Display Audio","PNPClass":"MEDIA","PNPDeviceID":"INTELAUDIO\\FUNC_01&VEN_8086&DEV_280B&SUBSYS_80860101&REV_1000\\4&2D5C3F&0&0201","Service":"IntcDAud","HardwareID":[],"CompatibleID":[]},
  {"Name":"Standard PS/2 Keyboard","PNPClass":"Keyboard","PNPDeviceID":"ACPI\\VEN_LEN&DEV_0071\\4&22D9B7FC&0","Service":"i8042prt","HardwareID":["ACPI\\VEN_LEN&DEV_0071","ACPI\\LEN0071","*LEN0071","*PNP0303"],"CompatibleID":["*PNP0303"]},
  {"Name":"I2C HID Device","PNPClass":"HIDClass","PNPDeviceID":"ACPI\\SYNA2B52\\4&1F3E5C6&0","Service":"hidi2c","HardwareID":["ACPI\\SYNA2B52","*SYNA2B52"],"CompatibleID":["ACPI\\PNP0C50","*PNP0C50"]},
  {"Name":"HID-compliant mouse","PNPClass":"Mouse","PNPDeviceID":"HID\\SYNA2B52&COL01\\5&3A1F&0&0000","Service":"mouhid","HardwareID":["HID\\SYNA2B52&Col01","HID_DEVICE_SYSTEM_MOUSE","HID_DEVICE_UP:0001_U:0002","HID_DEVICE"],"CompatibleID":[]},
  {"Name":"HID-compliant touch pad","PNPClass":"HIDClass","PNPDeviceID":"HID\\SYNA2B52&COL02\\5&3A1F&0&0001","Service":null,"HardwareID":["HID\\SYNA2B52&Col02","HID_DEVICE_UP:000D_U:0005","HID_DEVICE"],"CompatibleID":[]},
  {"Name":"I2C HID Device","PNPClass":"HIDClass","PNPDeviceID":"ACPI\\VEN_WCOM&DEV_5157\\4&1F3E5C6&0","Service":"hidi2c","HardwareID":[],"CompatibleID":["ACPI\\PNP0C50"]},
  {"Name":"HID-compliant touch screen","PNPClass":"HIDClass","PNPDeviceID":"HID\\VEN_WCOM&DEV_5157&COL01\\5&9B1&0&0000","Service":null,"HardwareID":["HID_DEVICE_UP:000D_U:0004"],"CompatibleID":[]},
  {"Name":"ACPI Lid","PNPClass":"System","PNPDeviceID":"ACPI\\PNP0C0D\\2&DABA3FF&1","Service":null,"HardwareID":["ACPI\\PNP0C0D"],"CompatibleID":[]},
  {"Name":"Intel(R) Wireless Bluetooth(R)","PNPClass":"Bluetooth","PNPDeviceID":"USB\\VID_8087&PID_0AAA\\5&1","Service":"BTHUSB","HardwareID":[],"CompatibleID":[]},
  {"Name":"Generic Bluetooth Radio","PNPClass":"Bluetooth","PNPDeviceID":"USB\\VID_8087&PID_0AAA&MI_00\\6&2","Service":"BTHUSB","HardwareID":[],"CompatibleID":[]},
  {"Name":"Microsoft Bluetooth Enumerator","PNPClass":"Bluetooth","PNPDeviceID":"BTH\\MS_BTHBRB\\7&1","Service":"BthEnum","HardwareID":[],"CompatibleID":[]},
  {"Name":"Logitech Pebble","PNPClass":"Mouse","PNPDeviceID":"HID\\{00001812-0000-1000-8000-00805F9B34FB}_DEV_VID&02046D_PID&B021_REV&0007_D8B2CDE4D1F7&COL01\\9&1","Service":"mouhid","HardwareID":["HID_DEVICE_UP:0001_U:0002"],"CompatibleID":[]},
  {"Name":"Sound Blaster G3","PNPClass":"MEDIA","PNPDeviceID":"USB\\VID_041E&PID_3256&MI_00\\7&1","Service":"usbaudio","HardwareID":[],"CompatibleID":[]},
  {"Name":"Intel(R) USB 3.1 eXtensible Host Controller","PNPClass":"USB","PNPDeviceID":"PCI\\VEN_8086&DEV_9DED&SUBSYS_229217AA&REV_30\\3&11583659&0&A0","Service":"USBXHCI","HardwareID":[],"CompatibleID":["PCI\\CC_0C0330"]},
  {"Name":"Intel(R) RST VMD Controller 9A0B","PNPClass":"SCSIAdapter","PNPDeviceID":"PCI\\VEN_8086&DEV_9A0B&SUBSYS_380117AA&REV_00\\3&1&0&70","Service":"iaStorVD","HardwareID":[],"CompatibleID":["PCI\\CC_010400"]},
  {"Name":"Intel(R) LPC Controller - 9D84","PNPClass":"System","PNPDeviceID":"PCI\\VEN_8086&DEV_9D84&SUBSYS_229217AA&REV_30\\3&1&0&F8","Service":null,"HardwareID":[],"CompatibleID":["PCI\\CC_060100"]},
  {"Name":"Türkçe Ad","PNPClass":"Net","PNPDeviceID":"USB\\VID_0BDA&PID_8153\\000001","Service":"rtux64w10","HardwareID":[],"CompatibleID":[]}
 ], "Count": 21},
 "properties": [
  {"Id":"INTELAUDIO\\FUNC_01&VEN_10EC&DEV_0257&SUBSYS_17AA2292&REV_1000\\4&2D5C3F&0&0001","Key":"DEVPKEY_Device_Parent","Data":"INTELAUDIO\\CTLR_DEV_9DC8&LINKTYPE_06&DEVTYPE_06&VEN_8086&DEV_AE20\\5&1"},
  {"Id":"INTELAUDIO\\CTLR_DEV_9DC8&LINKTYPE_06&DEVTYPE_06&VEN_8086&DEV_AE20\\5&1","Key":"DEVPKEY_Device_Parent","Data":"PCI\\VEN_8086&DEV_9DC8&SUBSYS_229217AA&REV_30\\3&11583659&0&FB"},
  {"Id":"PCI\\VEN_8086&DEV_9DC8&SUBSYS_229217AA&REV_30\\3&11583659&0&FB","Key":"DEVPKEY_Device_LocationPaths","Data":["PCIROOT(0)#PCI(1F03)","ACPI(_SB_)#ACPI(PCI0)#ACPI(HDAS)"]},
  {"Id":"HID\\SYNA2B52&COL01\\5&3A1F&0&0000","Key":"DEVPKEY_Device_Parent","Data":"ACPI\\SYNA2B52\\4&1F3E5C6&0"},
  {"Id":"HID\\SYNA2B52&COL02\\5&3A1F&0&0001","Key":"DEVPKEY_Device_Parent","Data":"ACPI\\SYNA2B52\\4&1F3E5C6&0"},
  {"Id":"PCI\\VEN_8086&DEV_9DED&SUBSYS_229217AA&REV_30\\3&11583659&0&A0","Key":"DEVPKEY_Device_LocationPaths","Data":["PCIROOT(0)#PCI(1400)"]},
  {"Id":"PCI\\VEN_8086&DEV_9DED&SUBSYS_229217AA&REV_30\\3&11583659&0&A0","Key":null,"Data":"ignored"}
 ]
}"#;

    fn desktop() -> DetectedHardware {
        let inv = parse(DESKTOP).unwrap();
        let mut native = NativeFacts {
            cpuid: Some(CpuidInfo {
                vendor: "GenuineIntel".into(),
                brand: "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz".into(),
                family: 6,
                model: 0x9e,
                stepping: 13,
                features: vec![
                    "sse4_2".into(),
                    "avx".into(),
                    "avx2".into(),
                    "vmx".into(),
                    "hypervisor".into(),
                ],
                hypervisor_signature: Some("Microsoft Hv".into()),
                hyperv_root_partition: true,
            }),
            topology: Some(ProcessorTopology {
                cores: 8,
                packages: 1,
                threads: 8,
                efficiency_classes: 1,
            }),
            installed_memory_kb: Some(32 * 1024 * 1024),
            uefi: Some(true),
            secure_boot: Some(false),
            ..Default::default()
        };
        native.vram_by_driver.insert(
            r"{4D36E968-E325-11CE-BFC1-08002BE10318}\0001".into(),
            8 << 30,
        );
        native.usb_ports.insert(
            r"PCI\VEN_8086&DEV_A36D&SUBSYS_86941043&REV_10\3&11583659&0&A0".into(),
            vec![UsbPortInfo {
                index: 1,
                speed_class: "usb2".into(),
                ..Default::default()
            }],
        );
        assert_eq!(display_driver_keys(&inv).len(), 2);
        assemble(&inv, &native)
    }

    #[test]
    fn desktop_cpu_board_and_firmware() {
        let hw = desktop();
        assert_eq!(hw.host_os, "windows");
        assert_eq!(
            (hw.cpu.family, hw.cpu.model, hw.cpu.stepping),
            (Some(6), Some(0x9e), Some(13))
        );
        assert_eq!((hw.cpu.cores, hw.cpu.threads, hw.cpu.packages), (8, 8, 1));
        assert_eq!(hw.cpu.base_clock_mhz, Some(3600));
        assert_eq!(hw.memory.total_mb, 32 * 1024);
        assert_eq!(
            hw.motherboard.manufacturer.as_deref(),
            Some("ASUSTeK COMPUTER INC.")
        );
        assert_eq!(hw.motherboard.product.as_deref(), Some("PRIME Z390-A"));
        assert_eq!(
            hw.motherboard.system_manufacturer, None,
            "SMBIOS placeholders are dropped"
        );
        assert_eq!(hw.motherboard.lpc_device_id.as_deref(), Some("a305"));
        assert_eq!(hw.chassis.chassis_types, [3]);
        assert_eq!(hw.chassis.manufacturer, None);
        assert!(!hw.chassis.has_battery && !hw.chassis.has_lid);
        assert_eq!(hw.firmware.uefi, Some(true));
        assert_eq!(hw.firmware.bios_date.as_deref(), Some("06/03/2021"));
        assert_eq!(hw.hypervisor, None, "Hyper-V root partition is bare metal");
        assert!(hw.warnings.is_empty());
    }

    #[test]
    fn desktop_devices() {
        let hw = desktop();
        assert_eq!(hw.gpus.len(), 2);
        let igpu = &hw.gpus[0];
        assert_eq!(
            (igpu.vendor_id.as_deref(), igpu.device_id.as_deref()),
            (Some("8086"), Some("3e98"))
        );
        assert_eq!(
            igpu.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x2,0x0)")
        );
        assert_eq!(igpu.location.acpi_path.as_deref(), Some(r"\_SB.PCI0.GFX0"));
        assert_eq!(igpu.vram_mb, None);
        let dgpu = &hw.gpus[1];
        assert_eq!(dgpu.subsystem_vendor_id.as_deref(), Some("1da2"));
        assert_eq!(dgpu.subsystem_device_id.as_deref(), Some("e353"));
        assert_eq!(dgpu.revision.as_deref(), Some("e7"));
        assert_eq!(dgpu.vram_mb, Some(8192));
        assert_eq!(
            dgpu.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)")
        );
        assert_eq!(
            dgpu.location.acpi_path.as_deref(),
            Some(r"\_SB.PCI0.PEG0.PEGP")
        );

        let codecs: Vec<_> = hw
            .audio
            .iter()
            .filter(|a| a.codec_device_id.is_some())
            .collect();
        assert_eq!(codecs.len(), 2);
        let alc = codecs[0];
        assert_eq!(
            (
                alc.codec_vendor_id.as_deref(),
                alc.codec_device_id.as_deref()
            ),
            (Some("10ec"), Some("1220"))
        );
        assert_eq!(alc.codec_subsystem_id.as_deref(), Some("10438694"));
        assert_eq!(alc.controller_device_id.as_deref(), Some("a348"));
        assert_eq!(
            alc.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x3)")
        );
        assert_eq!(alc.location.acpi_path.as_deref(), Some(r"\_SB.PCI0.HDAS"));
        assert_eq!(alc.bus, "hdaudio");
        assert!(!alc.is_hdmi);
        assert!(codecs[1].is_hdmi);
        assert_eq!(
            codecs[1].controller_device_id.as_deref(),
            Some("aaf0"),
            "parent ids match case-insensitively"
        );
        assert_eq!(hw.audio.len(), 2, "both controllers own a codec");

        assert_eq!(hw.network.len(), 2);
        let lan = &hw.network[0];
        assert_eq!(
            (lan.kind, lan.device_id.as_deref()),
            (NetworkKind::Ethernet, Some("15bc"))
        );
        assert_eq!(lan.mac_address.as_deref(), Some("a4:bb:6d:12:34:56"));
        assert_eq!(
            lan.location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x1f,0x6)")
        );
        let bt = &hw.network[1];
        assert_eq!((bt.kind, bt.bus.as_str()), (NetworkKind::Bluetooth, "usb"));
        assert_eq!(
            (bt.vendor_id.as_deref(), bt.device_id.as_deref()),
            (Some("8087"), Some("0aaa"))
        );

        let storage: Vec<(&str, &str, Option<&str>)> = hw
            .storage
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.kind.as_str(),
                    s.controller_device_id.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            storage,
            [
                ("Samsung SSD 970 EVO Plus 1TB", "nvme", Some("a808")),
                ("Samsung SSD 860 EVO 500GB", "sata", Some("a352")),
                ("SanDisk Cruzer Blade USB Device", "usb", None),
            ]
        );
        assert_eq!(hw.storage[0].size_bytes, Some(1_000_202_273_280));

        assert_eq!(hw.usb_controllers.len(), 1);
        assert_eq!(hw.usb_controllers[0].kind, "xhci");
        assert_eq!(hw.usb_controllers[0].ports.len(), 1);
        assert_eq!(
            hw.usb_controllers[0].location.pci_path.as_deref(),
            Some("PciRoot(0x0)/Pci(0x14,0x0)")
        );

        let input: Vec<(InputKind, &str)> =
            hw.input.iter().map(|i| (i.kind, i.bus.as_str())).collect();
        assert_eq!(
            input,
            [(InputKind::Keyboard, "usb"), (InputKind::Mouse, "usb")]
        );
    }

    #[test]
    fn laptop_inventory() {
        let inv = parse(LAPTOP).unwrap();
        assert_eq!(inv.devices.len(), 21);
        let native = NativeFacts {
            uefi: Some(true),
            secure_boot: Some(true),
            ..Default::default()
        };
        let hw = assemble(&inv, &native);

        // No CPUID: identity from Win32_Processor.Description.
        assert_eq!(hw.cpu.vendor, "GenuineIntel");
        assert_eq!(
            (hw.cpu.family, hw.cpu.model, hw.cpu.stepping),
            (Some(6), Some(142), Some(11))
        );
        assert_eq!((hw.cpu.cores, hw.cpu.threads, hw.cpu.packages), (4, 8, 1));
        assert_eq!(hw.cpu.base_clock_mhz, Some(1800));
        assert_eq!(
            hw.memory.total_mb, 16121,
            "falls back to TotalPhysicalMemory"
        );
        assert_eq!(hw.chassis.chassis_types, [10]);
        assert!(hw.chassis.has_battery && hw.chassis.has_lid);
        assert_eq!(
            hw.firmware.bios_version.as_deref(),
            Some("N2IET98W (1.76 )")
        );
        assert_eq!(hw.motherboard.lpc_device_id.as_deref(), Some("9d84"));
        assert_eq!(hw.warnings.len(), 1);

        let wifi = &hw.network[0];
        assert_eq!(
            (wifi.kind, wifi.device_id.as_deref()),
            (NetworkKind::Wifi, Some("9df0"))
        );
        assert_eq!(wifi.mac_address.as_deref(), Some("8c:c6:81:aa:bb:cc"));
        let bluetooth: Vec<_> = hw
            .network
            .iter()
            .filter(|n| n.kind == NetworkKind::Bluetooth)
            .collect();
        assert_eq!(
            bluetooth.len(),
            1,
            "the radio and its interface are one device"
        );
        let usb_lan = hw
            .network
            .iter()
            .find(|n| n.bus == "usb" && n.kind == NetworkKind::Ethernet)
            .unwrap();
        assert_eq!(usb_lan.name, "Türkçe Ad");
        assert_eq!(usb_lan.device_id.as_deref(), Some("8153"));

        let realtek = hw
            .audio
            .iter()
            .find(|a| a.codec_vendor_id.as_deref() == Some("10ec"))
            .unwrap();
        assert_eq!(realtek.bus, "sst");
        assert_eq!(realtek.codec_device_id.as_deref(), Some("0257"));
        assert_eq!(realtek.codec_subsystem_id.as_deref(), Some("17aa2292"));
        assert_eq!(
            realtek.controller_device_id.as_deref(),
            Some("9dc8"),
            "walks through the SST bus node"
        );
        assert_eq!(
            realtek.location.acpi_path.as_deref(),
            Some(r"\_SB.PCI0.HDAS")
        );
        let hdmi = hw
            .audio
            .iter()
            .find(|a| a.codec_vendor_id.as_deref() == Some("8086"))
            .unwrap();
        assert!(hdmi.is_hdmi);
        assert_eq!(hdmi.controller_device_id, None, "no parent known");
        assert!(hw
            .audio
            .iter()
            .any(|a| a.bus == "usb" && a.name == "Sound Blaster G3"));
        assert!(
            !hw.audio
                .iter()
                .any(|a| a.codec_vendor_id.is_none() && a.bus == "sst"),
            "the SST controller owns a codec"
        );

        let input: Vec<(InputKind, &str, Option<&str>, Option<&str>)> = hw
            .input
            .iter()
            .map(|i| {
                (
                    i.kind,
                    i.bus.as_str(),
                    i.hardware_id.as_deref(),
                    i.vendor.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            input,
            [
                (InputKind::Keyboard, "ps2", Some("LEN0071"), None),
                (
                    InputKind::Touchpad,
                    "i2c",
                    Some("SYNA2B52"),
                    Some("synaptics")
                ),
                (
                    InputKind::Touchscreen,
                    "i2c",
                    Some("WCOM5157"),
                    Some("wacom")
                ),
                (InputKind::Mouse, "bluetooth", None, None),
            ]
        );

        let storage: Vec<(&str, &str)> = hw
            .storage
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert_eq!(storage, [("Intel(R) RST VMD Controller 9A0B", "raid")]);
        assert_eq!(hw.usb_controllers[0].ports.len(), 0);
    }

    #[test]
    fn script_and_output_framing() {
        let encoded = encode_command(SCRIPT);
        // CreateProcess limits the whole command line to 32767 characters.
        assert!(
            encoded.len() < 30_000,
            "encoded script is {} characters",
            encoded.len()
        );
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .unwrap();
        assert_eq!(&decoded[..4], &[b'$', 0, b'E', 0]);
        assert!(SCRIPT.contains(BEGIN_MARKER) && SCRIPT.contains(END_MARKER));

        let noisy = "Loading personal profile...\r\n<<OCJSON>>{\"errors\":[]}<</OCJSON>>\r\n";
        assert_eq!(extract_payload(noisy), Some("{\"errors\":[]}"));
        assert_eq!(extract_payload("no markers"), None);
        assert_eq!(decode_output(b"\xef\xbb\xbfabc"), "abc");
        assert_eq!(decode_output(b"G\xf6r"), "G\u{fffd}r");
    }

    #[test]
    fn empty_and_broken_inventories() {
        assert!(parse("not json").is_err());
        let inv = parse("{}").unwrap();
        assert_eq!(inv, Inventory::default());
        let hw = assemble(&inv, &NativeFacts::default());
        assert_eq!(hw.cpu.packages, 1);
        assert!(
            hw.gpus.is_empty()
                && hw.audio.is_empty()
                && hw.network.is_empty()
                && hw.input.is_empty()
        );
        assert_eq!(hw.memory.total_mb, 0);
        assert_eq!(hw.firmware.uefi, None);
        assert_eq!(hw.hypervisor, None);

        // HypervisorPresent alone is bare metal with VBS (e.g. Windows on Arm,
        // where there is no CPUID); SMBIOS still names real VMs.
        let vbs = parse(
            r#"{"system":{"Manufacturer":"LENOVO","Model":"21BX","HypervisorPresent":true}}"#,
        )
        .unwrap();
        assert_eq!(vbs.hypervisor_present, Some(true));
        assert_eq!(assemble(&vbs, &NativeFacts::default()).hypervisor, None);
        let guest = parse(
            r#"{"system":{"Manufacturer":"Microsoft Corporation","Model":"Virtual Machine","HypervisorPresent":true}}"#,
        )
        .unwrap();
        assert_eq!(
            assemble(&guest, &NativeFacts::default())
                .hypervisor
                .as_deref(),
            Some("Microsoft Hyper-V")
        );
    }

    #[test]
    fn ups_units_are_not_batteries() {
        let battery = |label: &str, chemistry: Option<u64>, pnp: Option<&str>| Battery {
            label: label.into(),
            chemistry,
            pnp_id: pnp.map(str::to_string),
        };
        assert!(battery("Back-UPS ES 700G FW:871.O2", Some(2), None).is_ups());
        assert!(battery("CP1500PFCLCD", Some(3), None).is_ups());
        assert!(battery("CP1500PFCLCD", None, Some(r"HID\VID_0764&PID_0501\6&1")).is_ups());
        assert!(!battery("5B10W13930 SMP", Some(6), Some(r"ACPI\PNP0C0A\1")).is_ups());
        assert!(
            !battery("ASUS GROUPS 4C", Some(2), None).is_ups(),
            "\"UPS\" only as a word"
        );

        let mut inv = parse(r#"{"battery":[{"Name":"Internal Battery","Chemistry":8}]}"#).unwrap();
        assert!(assemble(&inv, &NativeFacts::default()).chassis.has_battery);
        inv.batteries[0].chemistry = Some(3);
        assert!(!assemble(&inv, &NativeFacts::default()).chassis.has_battery);
    }

    #[test]
    fn uart_bluetooth_is_reported_without_ids() {
        let inv = parse(
            r#"{"devices":[{"Name":"Broadcom Serial Bus Driver over UART Bus Enumerator","PNPClass":"Bluetooth","PNPDeviceID":"ACPI\\BCM2E7C\\0"}]}"#,
        )
        .unwrap();
        let hw = assemble(&inv, &NativeFacts::default());
        assert_eq!(hw.network.len(), 1);
        assert_eq!(
            (hw.network[0].kind, hw.network[0].bus.as_str()),
            (NetworkKind::Bluetooth, "other")
        );
    }

    #[test]
    fn collection_ids() {
        assert_eq!(
            hid_from_collection_id(r"HID\VEN_SYNA&DEV_2393&COL02\5&1").as_deref(),
            Some("SYNA2393")
        );
        assert_eq!(
            hid_from_collection_id(r"HID\ELAN0662&COL01\5&1").as_deref(),
            Some("ELAN0662")
        );
        assert_eq!(
            hid_from_collection_id(r"HID\VID_046D&PID_C52B&MI_01\8&1"),
            None
        );
        assert_eq!(
            wireless_kind("Realtek USB GbE Family Controller"),
            NetworkKind::Ethernet
        );
        assert_eq!(
            wireless_kind("TP-Link Wireless USB Adapter"),
            NetworkKind::Wifi
        );
    }
}
