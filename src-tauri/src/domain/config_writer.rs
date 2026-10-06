//! config.plist writer. Starts from the `Docs/Sample.plist` shipped in the
//! exact OpenCore release being installed (so the schema always matches), then
//! applies the build plan. Every override key must already exist in the
//! template; an unknown key is an error (catches schema drift).
//!
//! Array entries (ACPI/Add, Kernel/Patch, UEFI/Drivers, ...) are cloned from
//! the first Sample entry of the same array with every value reset, so new
//! keys of a future Sample.plist are always present with failsafe values.
//!
//! Sample.plist values replaced on every build, besides what the plan sets:
//! - root `#WARNING` comments and NVRAM `#INFO (prev-lang:kbd)` are removed;
//! - the `ForceDisplayRotationInEFI` example variable is removed from
//!   NVRAM Add/Delete; the other Sample variables stay (`DefaultBackgroundColor`
//!   and `rtc-blacklist`, as in Dortania's guide, and `SystemAudioVolume`,
//!   which only sets the boot chime volume);
//! - `prev-lang:kbd` becomes `en-US:0` (data) and is deleted before being
//!   written, like `boot-args` and every other variable the build adds;
//! - Misc/Security `Vault` = `Optional` and `ScanPolicy` = 0 (the Sample's
//!   `Secure` vault needs a signed vault.plist and would not boot);
//! - example entries are dropped: Booter/MmioWhitelist, Kernel/Force,
//!   Misc/Entries, UEFI/ReservedMemory, DeviceProperties.
//!
//! After the plan is applied, `Kernel/Quirks/CustomSMBIOSGuid` is made to
//! match `UpdateSMBIOSMode = Custom`, `PickerMode = External` falls back to
//! `Builtin` when OpenCanopy.efi is not among the loaded drivers, and
//! OpenCanopy.efi is not loaded for any other picker.
//!
//! Plan values are checked against the rules `ocvalidate` enforces (paths,
//! identifiers, kernel versions, patch masks, printable boot-args and
//! property names) and rejected with `CONFIG_VALUE_INVALID`; comments are
//! reduced to printable ASCII instead.

use std::collections::HashSet;
use std::io::Cursor;

use base64::Engine;
use plist::{Dictionary, Value};
use serde_json::json;

use crate::domain::kernel_add::KernelAddEntry;
use crate::domain::model::{BinaryPatch, BuildPlan, PlatformIdentity, PlistScalar, SettingMap};
use crate::error::AppError;

pub struct ConfigInputs<'a> {
    pub plan: &'a BuildPlan,
    pub kernel_add: &'a [KernelAddEntry],
    pub identity: &'a PlatformIdentity,
    /// SSDT file names actually present in EFI/OC/ACPI, in load order.
    pub ssdt_files: &'a [String],
    /// Driver file names actually present in EFI/OC/Drivers.
    pub driver_files: &'a [String],
    /// Tool file names actually present in EFI/OC/Tools.
    pub tool_files: &'a [String],
}

/// NVRAM GUID of Apple's boot variables (boot-args, csr-active-config, ...).
pub const APPLE_BOOT_VARIABLE_GUID: &str = "7C436110-AB2A-4BBB-A880-FE41995C9F82";

/// `prev-lang:kbd` value: US English, US keyboard layout.
const PREV_LANG_KBD: &[u8] = b"en-US:0";
/// Sample.plist example variables that are not written to real configs.
const DROPPED_NVRAM_VARIABLES: &[&str] = &["ForceDisplayRotationInEFI"];

const XML_HEADER: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    "<plist version=\"1.0\">\n",
);

/// Produce the final config.plist (XML) from the Sample.plist bytes.
pub fn write_config(sample_plist: &[u8], inputs: &ConfigInputs) -> Result<Vec<u8>, AppError> {
    let template = parse_sample(sample_plist)?;
    let mut root = template.clone();
    root.retain(|key, _| !key.starts_with('#'));

    write_acpi(&mut root, &template, inputs)?;
    write_booter(&mut root, &template, inputs.plan)?;
    write_device_properties(&mut root, inputs.plan)?;
    write_kernel(&mut root, &template, inputs)?;
    write_misc(&mut root, &template, inputs)?;
    write_nvram(&mut root, &template, inputs.plan)?;
    write_platform_info(&mut root, inputs)?;
    write_uefi(&mut root, &template, inputs)?;
    reconcile_custom_smbios(&mut root)?;
    reconcile_picker(&mut root)?;

    to_xml(&root)
}

/// Parse a plist (XML or binary) whose root is a dictionary.
fn parse_sample(bytes: &[u8]) -> Result<Dictionary, AppError> {
    let value = Value::from_reader(Cursor::new(bytes)).map_err(|e| {
        AppError::new(
            "SAMPLE_PLIST_INVALID",
            format!("Sample.plist cannot be parsed: {e}"),
        )
    })?;
    match value {
        Value::Dictionary(dict) => Ok(dict),
        _ => Err(AppError::new(
            "SAMPLE_PLIST_INVALID",
            "Sample.plist root is not a dictionary",
        )),
    }
}

// ── ACPI ────────────────────────────────────────────────────────────────────

fn write_acpi(
    root: &mut Dictionary,
    template: &Dictionary,
    inputs: &ConfigInputs,
) -> Result<(), AppError> {
    let plan = inputs.plan;

    let add_template = entry_template(template, "ACPI/Add")?;
    let planned = plan.ssdts.iter().map(|s| s.file_name.as_str());
    let mut add = Vec::new();
    for (i, file) in present_in_order(planned, inputs.ssdt_files, &[".aml", ".bin"])
        .into_iter()
        .enumerate()
    {
        let path = oc_path(file, &format!("ACPI/Add[{i}]/Path"), None)?;
        let mut entry = Entry::new(&add_template, "ACPI/Add");
        entry
            .set("Comment", comment(file))?
            .set("Enabled", Value::Boolean(true))?
            .set("Path", string(path))?;
        add.push(entry.finish());
    }
    *array_mut(root, "ACPI/Add")? = add;

    let delete_template = entry_template(template, "ACPI/Delete")?;
    let mut delete = Vec::new();
    for (i, d) in plan.acpi_deletes.iter().enumerate() {
        let path = format!("ACPI/Delete[{i}]");
        let signature = ascii_id(&d.table_signature, 4, &format!("{path}/TableSignature"))?;
        let oem_table_id = ascii_id(&d.oem_table_id, 8, &format!("{path}/OemTableId"))?;
        if signature.iter().all(|&b| b == 0) && oem_table_id.iter().all(|&b| b == 0) {
            return Err(invalid(&path, "needs a table signature or an OEM table id"));
        }
        let mut entry = Entry::new(&delete_template, "ACPI/Delete");
        entry
            .set("All", Value::Boolean(d.all))?
            .set("Comment", comment(&d.comment))?
            .set("Enabled", Value::Boolean(true))?
            .set("OemTableId", Value::Data(oem_table_id))?
            .set("TableLength", int(0))?
            .set("TableSignature", Value::Data(signature))?;
        delete.push(entry.finish());
    }
    *array_mut(root, "ACPI/Delete")? = delete;

    let patch_template = entry_template(template, "ACPI/Patch")?;
    let mut patches = Vec::new();
    for (i, p) in plan.acpi_patches.iter().enumerate() {
        let path = format!("ACPI/Patch[{i}]");
        let find = decode_hex(&p.find, &format!("{path}/Find"))?;
        let replace = decode_hex(&p.replace, &format!("{path}/Replace"))?;
        if find.is_empty() || find.len() != replace.len() {
            return Err(invalid(
                &path,
                "Find and Replace must be non-empty and of equal size",
            ));
        }
        let signature = match p.table_signature.as_deref() {
            Some(sig) if !sig.is_empty() => ascii_id(sig, 4, &format!("{path}/TableSignature"))?,
            _ => Vec::new(),
        };
        let oem_table_id = match p.oem_table_id.as_deref() {
            Some(id) if !id.is_empty() => ascii_id(id, 8, &format!("{path}/OemTableId"))?,
            _ => Vec::new(),
        };
        let mut entry = Entry::new(&patch_template, "ACPI/Patch");
        entry
            .set("Base", string(""))?
            .set("BaseSkip", int(0))?
            .set("Comment", comment(&p.comment))?
            .set("Count", int(p.count))?
            .set("Enabled", Value::Boolean(p.enabled))?
            .set("Find", Value::Data(find))?
            .set("Limit", int(0))?
            .set("Mask", Value::Data(Vec::new()))?
            .set("OemTableId", Value::Data(oem_table_id))?
            .set("Replace", Value::Data(replace))?
            .set("ReplaceMask", Value::Data(Vec::new()))?
            .set("Skip", int(0))?
            .set("TableLength", int(0))?
            .set("TableSignature", Value::Data(signature))?;
        patches.push(entry.finish());
    }
    *array_mut(root, "ACPI/Patch")? = patches;

    apply_overrides(root, "ACPI/Quirks", &plan.acpi_quirks)
}

// ── Booter ──────────────────────────────────────────────────────────────────

fn write_booter(
    root: &mut Dictionary,
    template: &Dictionary,
    plan: &BuildPlan,
) -> Result<(), AppError> {
    let mmio_template = entry_template(template, "Booter/MmioWhitelist")?;
    let mut mmio = Vec::new();
    for entry in &plan.mmio_whitelist {
        let address = i64::try_from(entry.address)
            .map_err(|_| invalid("Booter/MmioWhitelist", "address does not fit a plist integer"))?;
        let mut item = Entry::new(&mmio_template, "Booter/MmioWhitelist");
        item.set("Address", Value::Integer(address.into()))?
            .set("Comment", comment(&entry.comment))?
            .set("Enabled", Value::Boolean(entry.enabled))?;
        mmio.push(item.finish());
    }
    *array_mut(root, "Booter/MmioWhitelist")? = mmio;

    let patch_template = entry_template(template, "Booter/Patch")?;
    let mut patches = Vec::new();
    for (i, p) in plan.booter_patches.iter().enumerate() {
        let path = format!("Booter/Patch[{i}]");
        let bytes = PatchBytes::decode(p, &path)?;
        if bytes.find.is_empty() {
            return Err(invalid(&path, "booter patches need Find data"));
        }
        if !p.base.is_empty() || !p.min_kernel.is_empty() || !p.max_kernel.is_empty() {
            tracing::warn!(patch = %p.comment, "Base/MinKernel/MaxKernel do not apply to booter patches, ignored");
        }
        let ident = identifier(&p.identifier, &format!("{path}/Identifier"), true)?;
        let mut entry = Entry::new(&patch_template, "Booter/Patch");
        entry
            .set("Arch", string(arch(&p.arch, &path)?))?
            .set("Comment", comment(&p.comment))?
            .set("Count", int(p.count))?
            .set("Enabled", Value::Boolean(p.enabled))?
            .set("Find", Value::Data(bytes.find))?
            .set("Identifier", string(ident))?
            .set("Limit", int(p.limit))?
            .set("Mask", Value::Data(bytes.mask))?
            .set("Replace", Value::Data(bytes.replace))?
            .set("ReplaceMask", Value::Data(bytes.replace_mask))?
            .set("Skip", int(p.skip))?;
        patches.push(entry.finish());
    }
    *array_mut(root, "Booter/Patch")? = patches;

    apply_overrides(root, "Booter/Quirks", &plan.booter_quirks)
}

// ── DeviceProperties ───────────────────────────────────────────────────────

fn write_device_properties(root: &mut Dictionary, plan: &BuildPlan) -> Result<(), AppError> {
    let mut add = Dictionary::new();
    for entry in &plan.device_properties {
        let path = entry.path.trim();
        if path.is_empty() || path.chars().any(char::is_whitespace) {
            return Err(invalid(
                "DeviceProperties/Add",
                &format!("invalid device path '{}'", entry.path),
            ));
        }
        let slot = slot_or_insert(
            &mut add,
            path,
            Value::Dictionary(Dictionary::new()),
            "DeviceProperties/Add",
        )?;
        let Some(props) = slot.as_dictionary_mut() else {
            return Err(type_mismatch("DeviceProperties/Add", "dict", slot));
        };
        for prop in &entry.properties {
            if prop.key.is_empty() {
                return Err(invalid(
                    &format!("DeviceProperties/Add/{path}"),
                    "empty property name",
                ));
            }
            printable(&prop.key, &format!("DeviceProperties/Add/{path}"))?;
            let value = scalar_value(
                &prop.value,
                &format!("DeviceProperties/Add/{path}/{}", prop.key),
            )?;
            if props.insert(prop.key.clone(), value).is_some() {
                tracing::debug!(path, key = %prop.key, "device property set twice, last value kept");
            }
        }
    }
    *dict_mut(root, "DeviceProperties/Add")? = add;
    dict_mut(root, "DeviceProperties/Delete")?.clear();
    Ok(())
}

// ── Kernel ──────────────────────────────────────────────────────────────────

fn write_kernel(
    root: &mut Dictionary,
    template: &Dictionary,
    inputs: &ConfigInputs,
) -> Result<(), AppError> {
    let plan = inputs.plan;

    let add_template = entry_template(template, "Kernel/Add")?;
    let mut add = Vec::new();
    for (i, k) in inputs.kernel_add.iter().enumerate() {
        let path = format!("Kernel/Add[{i}]");
        let bundle = oc_path(&k.bundle_path, &format!("{path}/BundlePath"), Some(".kext"))?;
        let plist = oc_path(&k.plist_path, &format!("{path}/PlistPath"), Some(".plist"))?;
        if !k.executable_path.is_empty() {
            oc_path(&k.executable_path, &format!("{path}/ExecutablePath"), None)?;
        }
        let mut entry = Entry::new(&add_template, "Kernel/Add");
        entry
            .set("Arch", string(arch(&k.arch, &path)?))?
            .set("BundlePath", string(bundle))?
            .set("Comment", comment(&k.comment))?
            .set("Enabled", Value::Boolean(k.enabled))?
            .set("ExecutablePath", string(&k.executable_path))?
            .set(
                "MaxKernel",
                string(kernel_version(&k.max_kernel, &format!("{path}/MaxKernel"))?),
            )?
            .set(
                "MinKernel",
                string(kernel_version(&k.min_kernel, &format!("{path}/MinKernel"))?),
            )?
            .set("PlistPath", string(plist))?;
        add.push(entry.finish());
    }
    *array_mut(root, "Kernel/Add")? = add;

    let block_template = entry_template(template, "Kernel/Block")?;
    let mut blocks = Vec::new();
    for (i, b) in plan.kernel_blocks.iter().enumerate() {
        let path = format!("Kernel/Block[{i}]");
        if !matches!(b.strategy.as_str(), "Disable" | "Exclude") {
            return Err(invalid(
                &path,
                &format!("unknown Strategy '{}'", b.strategy),
            ));
        }
        let ident = identifier(&b.identifier, &format!("{path}/Identifier"), false)?;
        let mut entry = Entry::new(&block_template, "Kernel/Block");
        entry
            .set("Arch", string("Any"))?
            .set("Comment", comment(&b.comment))?
            .set("Enabled", Value::Boolean(b.enabled))?
            .set("Identifier", string(ident))?
            .set(
                "MaxKernel",
                string(kernel_version(&b.max_kernel, &format!("{path}/MaxKernel"))?),
            )?
            .set(
                "MinKernel",
                string(kernel_version(&b.min_kernel, &format!("{path}/MinKernel"))?),
            )?
            .set("Strategy", string(&b.strategy))?;
        blocks.push(entry.finish());
    }
    *array_mut(root, "Kernel/Block")? = blocks;

    array_mut(root, "Kernel/Force")?.clear();

    let patch_template = entry_template(template, "Kernel/Patch")?;
    let mut patches = Vec::new();
    for (i, p) in plan.kernel_patches.iter().enumerate() {
        let path = format!("Kernel/Patch[{i}]");
        let bytes = PatchBytes::decode(p, &path)?;
        if bytes.find.is_empty() && p.base.trim().is_empty() {
            return Err(invalid(&path, "needs Find data or a Base symbol"));
        }
        let ident = identifier(&p.identifier, &format!("{path}/Identifier"), false)?;
        let mut entry = Entry::new(&patch_template, "Kernel/Patch");
        entry
            .set("Arch", string(arch(&p.arch, &path)?))?
            .set("Base", string(p.base.trim()))?
            .set("Comment", comment(&p.comment))?
            .set("Count", int(p.count))?
            .set("Enabled", Value::Boolean(p.enabled))?
            .set("Find", Value::Data(bytes.find))?
            .set("Identifier", string(ident))?
            .set("Limit", int(p.limit))?
            .set("Mask", Value::Data(bytes.mask))?
            .set(
                "MaxKernel",
                string(kernel_version(&p.max_kernel, &format!("{path}/MaxKernel"))?),
            )?
            .set(
                "MinKernel",
                string(kernel_version(&p.min_kernel, &format!("{path}/MinKernel"))?),
            )?
            .set("Replace", Value::Data(bytes.replace))?
            .set("ReplaceMask", Value::Data(bytes.replace_mask))?
            .set("Skip", int(p.skip))?;
        patches.push(entry.finish());
    }
    *array_mut(root, "Kernel/Patch")? = patches;

    apply_overrides(root, "Kernel/Emulate", &plan.kernel_emulate)?;
    check_cpuid_spoof(root)?;
    apply_overrides(root, "Kernel/Quirks", &plan.kernel_quirks)
}

/// Cpuid1Data / Cpuid1Mask are 16 bytes each (or empty), and every bit set in
/// the data must be selected by the mask.
fn check_cpuid_spoof(root: &Dictionary) -> Result<(), AppError> {
    const DATA: &str = "Kernel/Emulate/Cpuid1Data";
    const MASK: &str = "Kernel/Emulate/Cpuid1Mask";
    let empty: &[u8] = &[];
    let data = lookup(root, DATA)?.as_data().unwrap_or(empty);
    let mask = lookup(root, MASK)?.as_data().unwrap_or(empty);
    for (bytes, path) in [(data, DATA), (mask, MASK)] {
        if !bytes.is_empty() && bytes.len() != 16 {
            return Err(invalid(path, "must be empty or 16 bytes"));
        }
    }
    if !data.iter().all(|&b| b == 0) && (mask.is_empty() || !properly_masked(data, mask)) {
        return Err(invalid(DATA, "has bits set outside Cpuid1Mask"));
    }
    Ok(())
}

// ── Misc ────────────────────────────────────────────────────────────────────

fn write_misc(
    root: &mut Dictionary,
    template: &Dictionary,
    inputs: &ConfigInputs,
) -> Result<(), AppError> {
    let plan = inputs.plan;

    set_path(root, "Misc/Security/Vault", string("Optional"))?;
    set_path(root, "Misc/Security/ScanPolicy", int(0))?;
    if !plan.smbios.secure_boot_model.is_empty() {
        set_path(
            root,
            "Misc/Security/SecureBootModel",
            string(&plan.smbios.secure_boot_model),
        )?;
    }
    apply_overrides(root, "Misc/Boot", &plan.misc_boot)?;
    apply_overrides(root, "Misc/Debug", &plan.misc_debug)?;
    apply_overrides(root, "Misc/Security", &plan.misc_security)?;

    array_mut(root, "Misc/Entries")?.clear();

    let sample_tools = array_at(template, "Misc/Tools")?;
    let tool_template = entry_template(template, "Misc/Tools")?;
    let mut tools = Vec::new();
    let planned = plan.tools.iter().map(String::as_str);
    for (i, file) in present_in_order(planned, inputs.tool_files, &[".efi"])
        .into_iter()
        .enumerate()
    {
        oc_path(file, &format!("Misc/Tools[{i}]/Path"), None)?;
        let known = sample_tools
            .iter()
            .filter_map(Value::as_dictionary)
            .find(|t| {
                t.get("Path")
                    .and_then(Value::as_string)
                    .is_some_and(|p| p.eq_ignore_ascii_case(file))
            });
        let mut entry = match known {
            Some(sample) => Entry {
                path: "Misc/Tools",
                dict: sample.clone(),
            },
            None => {
                let flavour = if file.to_ascii_lowercase().ends_with("shell.efi") {
                    "OpenShell:UEFIShell:Shell"
                } else {
                    "Auto"
                };
                let mut entry = Entry::new(&tool_template, "Misc/Tools");
                entry
                    .set("Auxiliary", Value::Boolean(true))?
                    .set("Flavour", string(flavour))?
                    .set("Name", string(file))?;
                entry
            }
        };
        entry
            .set("Comment", comment(file))?
            .set("Enabled", Value::Boolean(true))?
            .set("Path", string(file))?;
        tools.push(entry.finish());
    }
    *array_mut(root, "Misc/Tools")? = tools;
    Ok(())
}

// ── NVRAM ───────────────────────────────────────────────────────────────────

fn write_nvram(
    root: &mut Dictionary,
    template: &Dictionary,
    plan: &BuildPlan,
) -> Result<(), AppError> {
    let mut boot_args: Vec<&str> = Vec::new();
    for arg in plan.boot_args.iter().flat_map(|a| a.split_whitespace()) {
        if !boot_args.contains(&arg) {
            boot_args.push(arg);
        }
    }

    let boot_args = boot_args.join(" ");
    printable(
        &boot_args,
        &format!("NVRAM/Add/{APPLE_BOOT_VARIABLE_GUID}/boot-args"),
    )?;
    let builtin = [
        ("boot-args", string(boot_args)),
        (
            "csr-active-config",
            Value::Data(plan.csr_active_config.to_le_bytes().to_vec()),
        ),
        ("prev-lang:kbd", Value::Data(PREV_LANG_KBD.to_vec())),
        ("run-efi-updater", string("No")),
    ];
    let mut variables: Vec<(String, String, Value)> = Vec::new();
    for (key, value) in builtin {
        let path = format!("NVRAM/Add/{APPLE_BOOT_VARIABLE_GUID}/{key}");
        if let Ok(existing) = lookup(template, &path) {
            check_kind(existing, &value, &path)?;
        }
        variables.push((APPLE_BOOT_VARIABLE_GUID.to_string(), key.to_string(), value));
    }
    for (i, var) in plan.nvram_add.iter().enumerate() {
        let path = format!("NVRAM/Add[{i}]");
        let guid = canonical_guid(&var.guid, &path)?;
        let key = printable(non_empty(&var.key, &path, "key")?, &path)?.to_string();
        let value = scalar_value(&var.value, &format!("NVRAM/Add/{guid}/{key}"))?;
        variables.push((guid, key, value));
    }

    let add = dict_mut(root, "NVRAM/Add")?;
    if let Some(Value::Dictionary(apple)) = add.get_mut(APPLE_BOOT_VARIABLE_GUID) {
        apple.retain(|key, _| {
            !key.starts_with('#') && !DROPPED_NVRAM_VARIABLES.contains(&key.as_str())
        });
    }
    for (guid, key, value) in &variables {
        let path = format!("NVRAM/Add/{guid}");
        let slot = slot_or_insert(add, guid, Value::Dictionary(Dictionary::new()), &path)?;
        let Some(vars) = slot.as_dictionary_mut() else {
            return Err(type_mismatch(&path, "dict", slot));
        };
        vars.insert(key.clone(), value.clone());
    }

    // Add never overwrites an existing variable, so everything written is
    // deleted first.
    let mut deletes: Vec<(String, String)> = variables
        .into_iter()
        .map(|(guid, key, _)| (guid, key))
        .collect();
    for (i, var) in plan.nvram_delete.iter().enumerate() {
        let path = format!("NVRAM/Delete[{i}]");
        deletes.push((
            canonical_guid(&var.guid, &path)?,
            printable(non_empty(&var.key, &path, "key")?, &path)?.to_string(),
        ));
    }
    let delete = dict_mut(root, "NVRAM/Delete")?;
    if let Some(Value::Array(apple)) = delete.get_mut(APPLE_BOOT_VARIABLE_GUID) {
        apple.retain(|v| {
            !v.as_string()
                .is_some_and(|k| DROPPED_NVRAM_VARIABLES.contains(&k))
        });
    }
    for (guid, key) in deletes {
        let path = format!("NVRAM/Delete/{guid}");
        let slot = slot_or_insert(delete, &guid, Value::Array(Vec::new()), &path)?;
        let Some(list) = slot.as_array_mut() else {
            return Err(type_mismatch(&path, "array", slot));
        };
        if !list.iter().any(|v| v.as_string() == Some(key.as_str())) {
            list.push(Value::String(key));
        }
    }

    apply_overrides(root, "NVRAM", &plan.nvram_settings)
}

// ── PlatformInfo ────────────────────────────────────────────────────────────

fn write_platform_info(root: &mut Dictionary, inputs: &ConfigInputs) -> Result<(), AppError> {
    let plan = inputs.plan;
    let id = inputs.identity;

    if !id.model.eq_ignore_ascii_case(&plan.smbios.model) {
        return Err(AppError::new(
            "IDENTITY_MODEL_MISMATCH",
            format!(
                "The serial numbers were generated for {} but the build uses {}",
                id.model, plan.smbios.model
            ),
        )
        .with_suggestion("Generate a new identity for the selected SMBIOS model."));
    }
    let identity_error = |what: &str| {
        AppError::new(
            "IDENTITY_INVALID",
            format!("PlatformInfo {what} is not valid"),
        )
    };
    let rom = decode_hex(&id.rom, "PlatformInfo/Generic/ROM").map_err(|_| identity_error("ROM"))?;
    if rom.len() != 6 {
        return Err(identity_error("ROM"));
    }
    let uuid = uuid::Uuid::parse_str(&id.system_uuid).map_err(|_| identity_error("SystemUUID"))?;
    let alnum = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    if !alnum(&id.serial) {
        return Err(identity_error("SystemSerialNumber"));
    }
    if !alnum(&id.mlb) {
        return Err(identity_error("MLB"));
    }

    set_path(root, "PlatformInfo/Automatic", Value::Boolean(true))?;
    set_path(
        root,
        "PlatformInfo/Generic/AdviseFeatures",
        Value::Boolean(false),
    )?;
    set_path(root, "PlatformInfo/Generic/MLB", string(&id.mlb))?;
    set_path(
        root,
        "PlatformInfo/Generic/MaxBIOSVersion",
        Value::Boolean(false),
    )?;
    set_path(root, "PlatformInfo/Generic/ProcessorType", int(0))?;
    set_path(root, "PlatformInfo/Generic/ROM", Value::Data(rom))?;
    set_path(
        root,
        "PlatformInfo/Generic/SpoofVendor",
        Value::Boolean(true),
    )?;
    set_path(
        root,
        "PlatformInfo/Generic/SystemMemoryStatus",
        string("Auto"),
    )?;
    set_path(
        root,
        "PlatformInfo/Generic/SystemProductName",
        string(&plan.smbios.model),
    )?;
    set_path(
        root,
        "PlatformInfo/Generic/SystemSerialNumber",
        string(&id.serial),
    )?;
    set_path(
        root,
        "PlatformInfo/Generic/SystemUUID",
        string(uuid.hyphenated().to_string().to_uppercase()),
    )?;
    set_path(root, "PlatformInfo/UpdateSMBIOSMode", string("Create"))?;

    apply_overrides(root, "PlatformInfo", &plan.platform_info)
}

/// `UpdateSMBIOSMode = Custom` only reaches macOS with the CustomSMBIOSGuid
/// quirk, and the quirk is wrong for any other mode.
fn reconcile_custom_smbios(root: &mut Dictionary) -> Result<(), AppError> {
    let custom = lookup(root, "PlatformInfo/UpdateSMBIOSMode")?.as_string() == Some("Custom");
    let quirk = lookup(root, "Kernel/Quirks/CustomSMBIOSGuid")?.as_boolean();
    if quirk != Some(custom) {
        tracing::info!(custom, "CustomSMBIOSGuid set to match UpdateSMBIOSMode");
        set_path(
            root,
            "Kernel/Quirks/CustomSMBIOSGuid",
            Value::Boolean(custom),
        )?;
    }
    Ok(())
}

/// `PickerMode = External` and an enabled OpenCanopy.efi go together. When
/// the driver is not in the EFI (e.g. its resources failed to download) the
/// built-in picker is used; when another picker was chosen, OpenCanopy.efi
/// stays on disk but is not loaded.
fn reconcile_picker(root: &mut Dictionary) -> Result<(), AppError> {
    let external = lookup(root, "Misc/Boot/PickerMode")?.as_string() == Some("External");
    let mut canopy = false;
    for driver in array_mut(root, "UEFI/Drivers")?
        .iter_mut()
        .filter_map(Value::as_dictionary_mut)
    {
        let is_canopy = driver
            .get("Path")
            .and_then(Value::as_string)
            .is_some_and(|p| p.eq_ignore_ascii_case("OpenCanopy.efi"));
        if !is_canopy || driver.get("Enabled").and_then(Value::as_boolean) != Some(true) {
            continue;
        }
        if external {
            canopy = true;
        } else {
            tracing::info!("OpenCanopy.efi is not used by the chosen picker, not loading it");
            driver.insert("Enabled".to_string(), Value::Boolean(false));
        }
    }
    if external && !canopy {
        tracing::warn!("OpenCanopy.efi is not loaded, using the built-in picker");
        set_path(root, "Misc/Boot/PickerMode", string("Builtin"))?;
    }
    Ok(())
}

// ── UEFI ────────────────────────────────────────────────────────────────────

fn write_uefi(
    root: &mut Dictionary,
    template: &Dictionary,
    inputs: &ConfigInputs,
) -> Result<(), AppError> {
    let plan = inputs.plan;

    let sample_drivers = array_at(template, "UEFI/Drivers")?;
    let driver_template = entry_template(template, "UEFI/Drivers")?;
    let mut drivers = Vec::new();
    let mut seen = HashSet::new();
    for d in &plan.drivers {
        let key = d.path.to_ascii_lowercase();
        if !seen.insert(key.clone()) {
            tracing::warn!(driver = %d.path, "driver listed twice in the plan, keeping the first entry");
            continue;
        }
        if !inputs
            .driver_files
            .iter()
            .any(|f| f.to_ascii_lowercase() == key)
        {
            tracing::warn!(driver = %d.path, "driver file missing from EFI/OC/Drivers, not added to config.plist");
            continue;
        }
        let hide_verbose = sample_drivers
            .iter()
            .filter_map(Value::as_dictionary)
            .find(|s| {
                s.get("Path")
                    .and_then(Value::as_string)
                    .is_some_and(|p| p.eq_ignore_ascii_case(&d.path))
            })
            .and_then(|s| s.get("HideVerbose"))
            .and_then(Value::as_boolean)
            .unwrap_or(false);
        let path = format!("UEFI/Drivers[{}]/Path", drivers.len());
        if oc_path(&d.path, &path, Some(".efi"))?.contains('\\') {
            return Err(invalid(&path, "use '/' as the path separator"));
        }
        let mut entry = Entry::new(&driver_template, "UEFI/Drivers");
        entry
            .set("Arguments", string(""))?
            .set("Comment", comment(&d.comment))?
            .set("Enabled", Value::Boolean(d.enabled))?
            .set("LoadEarly", Value::Boolean(d.load_early))?
            .set("Path", string(&d.path))?;
        // Per-driver HideVerbose exists from OpenCore 1.0.8 on.
        if entry.dict.contains_key("HideVerbose") {
            entry.set("HideVerbose", Value::Boolean(hide_verbose))?;
        }
        drivers.push(entry.finish());
    }
    for file in inputs.driver_files {
        if !seen.contains(&file.to_ascii_lowercase()) {
            tracing::debug!(driver = %file, "driver file present but not planned, left out of config.plist");
        }
    }
    *array_mut(root, "UEFI/Drivers")? = drivers;

    apply_overrides(root, "UEFI/APFS", &plan.uefi_apfs)?;
    apply_overrides(root, "UEFI/Output", &plan.uefi_output)?;
    apply_overrides(root, "UEFI/Input", &plan.uefi_input)?;
    apply_overrides(root, "UEFI/Quirks", &plan.uefi_quirks)?;
    array_mut(root, "UEFI/ReservedMemory")?.clear();
    array_mut(root, "UEFI/Unload")?.clear();
    Ok(())
}

// ── Entries built from Sample templates ────────────────────────────────────

/// One array entry under construction; only keys of the Sample template can
/// be set, with the template's value type.
struct Entry<'a> {
    path: &'a str,
    dict: Dictionary,
}

impl<'a> Entry<'a> {
    fn new(template: &Dictionary, path: &'a str) -> Self {
        Self {
            path,
            dict: template.clone(),
        }
    }

    fn set(&mut self, key: &str, value: Value) -> Result<&mut Self, AppError> {
        let path = format!("{}/{key}", self.path);
        let slot = self.dict.get_mut(key).ok_or_else(|| key_unknown(&path))?;
        check_kind(slot, &value, &path)?;
        *slot = value;
        Ok(self)
    }

    fn finish(self) -> Value {
        Value::Dictionary(self.dict)
    }
}

/// First entry of a Sample array with every value reset to its failsafe
/// (false / 0 / empty).
fn entry_template(template: &Dictionary, path: &str) -> Result<Dictionary, AppError> {
    let first = array_at(template, path)?.first().ok_or_else(|| {
        AppError::new(
            "SAMPLE_PLIST_INVALID",
            format!("Sample.plist {path} has no example entry"),
        )
    })?;
    let Some(dict) = first.as_dictionary() else {
        return Err(type_mismatch(&format!("{path}[0]"), "dict", first));
    };
    let mut entry = dict.clone();
    for (_, value) in entry.iter_mut() {
        *value = match &*value {
            Value::Boolean(_) => Value::Boolean(false),
            Value::Integer(_) => int(0),
            Value::String(_) => string(""),
            Value::Data(_) => Value::Data(Vec::new()),
            Value::Array(_) => Value::Array(Vec::new()),
            Value::Dictionary(_) => Value::Dictionary(Dictionary::new()),
            other => other.clone(),
        };
    }
    Ok(entry)
}

/// Files of `present`, ordered as in `planned` first, then the remaining ones
/// in their given order; duplicates removed. Hidden files (`.DS_Store`,
/// AppleDouble `._*`) and names without one of `extensions` are skipped.
fn present_in_order<'a>(
    planned: impl Iterator<Item = &'a str>,
    present: &'a [String],
    extensions: &[&str],
) -> Vec<&'a str> {
    let usable = |name: &str| {
        !name.starts_with('.') && extensions.iter().any(|ext| has_extension(name, ext))
    };
    let mut out: Vec<&'a str> = Vec::new();
    let mut push = |name: &'a str| {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(name)) {
            out.push(name);
        }
    };
    for name in planned {
        if let Some(file) = present.iter().find(|f| f.eq_ignore_ascii_case(name)) {
            push(file);
        }
    }
    for file in present {
        push(file);
    }
    out.retain(|name| {
        let keep = usable(name);
        if !keep {
            tracing::warn!(file = %name, "not a loadable file, left out of config.plist");
        }
        keep
    });
    out
}

struct PatchBytes {
    find: Vec<u8>,
    mask: Vec<u8>,
    replace: Vec<u8>,
    replace_mask: Vec<u8>,
}

impl PatchBytes {
    fn decode(p: &BinaryPatch, path: &str) -> Result<Self, AppError> {
        let bytes = Self {
            find: decode_hex(&p.find, &format!("{path}/Find"))?,
            mask: decode_hex(&p.mask, &format!("{path}/Mask"))?,
            replace: decode_hex(&p.replace, &format!("{path}/Replace"))?,
            replace_mask: decode_hex(&p.replace_mask, &format!("{path}/ReplaceMask"))?,
        };
        if bytes.replace.is_empty() {
            return Err(invalid(path, "Replace is empty"));
        }
        if !bytes.find.is_empty() && bytes.find.len() != bytes.replace.len() {
            return Err(invalid(path, "Find and Replace differ in size"));
        }
        if !bytes.mask.is_empty() && bytes.mask.len() != bytes.find.len() {
            return Err(invalid(path, "Mask and Find differ in size"));
        }
        if !bytes.replace_mask.is_empty() && bytes.replace_mask.len() != bytes.replace.len() {
            return Err(invalid(path, "ReplaceMask and Replace differ in size"));
        }
        if !properly_masked(&bytes.find, &bytes.mask) {
            return Err(invalid(path, "Find has bits set outside Mask"));
        }
        if !properly_masked(&bytes.replace, &bytes.replace_mask) {
            return Err(invalid(path, "Replace has bits set outside ReplaceMask"));
        }
        Ok(bytes)
    }
}

/// OpenCore rejects masked data with bits set where the mask is clear.
fn properly_masked(data: &[u8], mask: &[u8]) -> bool {
    mask.is_empty() || data.iter().zip(mask).all(|(d, m)| d & !m == 0)
}

// ── Value helpers ───────────────────────────────────────────────────────────

fn string(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

/// OpenCore only accepts printable ASCII in comments: common typographic
/// characters are spelled out, anything else becomes `?`.
fn comment(text: &str) -> Value {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ' '..='~' => out.push(c),
            '\t' | '\n' | '\r' => out.push(' '),
            '\u{2010}'..='\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201C}' | '\u{201D}' => out.push('"'),
            '\u{2026}' => out.push_str("..."),
            '\u{2192}' => out.push_str("->"),
            '\u{2264}' => out.push_str("<="),
            '\u{2265}' => out.push_str(">="),
            _ => out.push('?'),
        }
    }
    Value::String(out)
}

fn printable_ascii(value: &str) -> bool {
    value.bytes().all(|b| (0x20..0x7F).contains(&b))
}

/// Printable ASCII, as OpenCore requires for property names and boot-args.
fn printable<'s>(value: &'s str, path: &str) -> Result<&'s str, AppError> {
    if printable_ascii(value) {
        Ok(value)
    } else {
        Err(invalid(
            path,
            &format!("'{value}' contains non-printable or non-ASCII characters"),
        ))
    }
}

/// File path below EFI/OC as OpenCore accepts it: `0-9 A-Z a-z _ - . / \`,
/// optionally with a required extension.
fn oc_path<'s>(value: &'s str, path: &str, extension: Option<&str>) -> Result<&'s str, AppError> {
    let legal = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b'\\'));
    if value.is_empty() || !legal {
        return Err(invalid(
            path,
            &format!("'{value}' is not a valid OpenCore path"),
        ));
    }
    if let Some(ext) = extension {
        if !has_extension(value, ext) {
            return Err(invalid(path, &format!("'{value}' does not end with {ext}")));
        }
    }
    Ok(value)
}

fn has_extension(name: &str, ext: &str) -> bool {
    name.len() > ext.len()
        && name
            .get(name.len() - ext.len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(ext))
}

/// Patch / block identifiers: `kernel` or a bundle id for the kernel
/// sections; `Any`, `Apple` or an `.efi` file name for booter patches.
fn identifier<'s>(value: &'s str, path: &str, booter: bool) -> Result<&'s str, AppError> {
    let fixed = if booter {
        matches!(value, "Any" | "Apple")
    } else {
        value == "kernel"
    };
    let bundle_like = value.contains('.')
        && (!booter || has_extension(value, ".efi"))
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if fixed || bundle_like {
        Ok(value)
    } else {
        Err(invalid(
            path,
            &format!("'{value}' is not a valid Identifier"),
        ))
    }
}

/// Darwin version for MinKernel / MaxKernel: empty, or up to three numbers
/// ("25", "25.4", "25.99.99") starting at Darwin 8.
fn kernel_version<'s>(value: &'s str, path: &str) -> Result<&'s str, AppError> {
    if value.is_empty() {
        return Ok(value);
    }
    let parts: Vec<&str> = value.split('.').collect();
    let numbers: Option<Vec<u32>> = parts
        .iter()
        .map(|p| {
            (!p.is_empty() && p.len() <= 2 && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse().ok())
                .flatten()
        })
        .collect();
    match numbers {
        Some(n) if (1..=3).contains(&n.len()) && n[0] >= 8 => Ok(value),
        _ => Err(invalid(
            path,
            &format!("'{value}' is not a Darwin kernel version"),
        )),
    }
}

fn int(v: impl Into<i64>) -> Value {
    let v: i64 = v.into();
    Value::Integer(v.into())
}

fn arch<'s>(value: &'s str, path: &str) -> Result<&'s str, AppError> {
    match value {
        "" => Ok("Any"),
        "Any" | "i386" | "x86_64" => Ok(value),
        other => Err(invalid(path, &format!("unknown Arch '{other}'"))),
    }
}

fn non_empty<'s>(value: &'s str, path: &str, what: &str) -> Result<&'s str, AppError> {
    if value.trim().is_empty() {
        Err(invalid(path, &format!("{what} is empty")))
    } else {
        Ok(value)
    }
}

fn decode_hex(hex: &str, path: &str) -> Result<Vec<u8>, AppError> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(path, &format!("'{hex}' is not hex encoded data")));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid(path, "bad hex digit")))
        .collect()
}

/// ACPI signatures / OEM table ids: up to `len` ASCII characters padded with
/// NUL bytes (trailing spaces are kept as given, so pass the exact id read
/// from the table when it is space padded), or exactly `2 * len` hex digits
/// for raw bytes.
fn ascii_id(value: &str, len: usize, path: &str) -> Result<Vec<u8>, AppError> {
    if value.len() == len * 2 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return decode_hex(value, path);
    }
    if value.len() > len || !value.bytes().all(|b| (0x20..0x7F).contains(&b)) {
        return Err(invalid(
            path,
            &format!("'{value}' is not a {len}-character ACPI identifier"),
        ));
    }
    let mut bytes = value.as_bytes().to_vec();
    bytes.resize(len, 0);
    Ok(bytes)
}

fn canonical_guid(guid: &str, path: &str) -> Result<String, AppError> {
    let groups: Vec<&str> = guid.trim().split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    let valid = groups.len() == lengths.len()
        && groups
            .iter()
            .zip(lengths)
            .all(|(g, l)| g.len() == l && g.bytes().all(|b| b.is_ascii_hexdigit()));
    if !valid {
        return Err(invalid(path, &format!("'{guid}' is not a GUID")));
    }
    Ok(guid.trim().to_ascii_uppercase())
}

fn scalar_value(scalar: &PlistScalar, path: &str) -> Result<Value, AppError> {
    Ok(match scalar {
        PlistScalar::Bool(b) => Value::Boolean(*b),
        PlistScalar::Int(i) => int(*i),
        PlistScalar::Str(s) => string(s.as_str()),
        PlistScalar::Data(hex) => Value::Data(decode_hex(hex, path)?),
    })
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "array",
        Value::Dictionary(_) => "dict",
        Value::Boolean(_) => "bool",
        Value::Data(_) => "data",
        Value::Date(_) => "date",
        Value::Real(_) => "real",
        Value::Integer(_) => "int",
        Value::String(_) => "string",
        _ => "other",
    }
}

fn check_kind(existing: &Value, new: &Value, path: &str) -> Result<(), AppError> {
    if kind(existing) == kind(new) {
        Ok(())
    } else {
        Err(type_mismatch(path, kind(existing), new))
    }
}

// ── Path navigation ─────────────────────────────────────────────────────────

fn lookup<'a>(root: &'a Dictionary, path: &str) -> Result<&'a Value, AppError> {
    let mut segments = path.split('/');
    let first = segments.next().unwrap_or_default();
    let mut current = root.get(first).ok_or_else(|| key_unknown(path))?;
    for segment in segments {
        let Some(dict) = current.as_dictionary() else {
            return Err(type_mismatch(path, "dict", current));
        };
        current = dict.get(segment).ok_or_else(|| key_unknown(path))?;
    }
    Ok(current)
}

fn lookup_mut<'a>(root: &'a mut Dictionary, path: &str) -> Result<&'a mut Value, AppError> {
    let mut segments = path.split('/');
    let first = segments.next().unwrap_or_default();
    let mut current = root.get_mut(first).ok_or_else(|| key_unknown(path))?;
    for segment in segments {
        current = match current {
            Value::Dictionary(dict) => dict.get_mut(segment).ok_or_else(|| key_unknown(path))?,
            other => return Err(type_mismatch(path, "dict", other)),
        };
    }
    Ok(current)
}

fn dict_mut<'a>(root: &'a mut Dictionary, path: &str) -> Result<&'a mut Dictionary, AppError> {
    match lookup_mut(root, path)? {
        Value::Dictionary(dict) => Ok(dict),
        other => Err(type_mismatch(path, "dict", other)),
    }
}

fn array_mut<'a>(root: &'a mut Dictionary, path: &str) -> Result<&'a mut Vec<Value>, AppError> {
    match lookup_mut(root, path)? {
        Value::Array(array) => Ok(array),
        other => Err(type_mismatch(path, "array", other)),
    }
}

fn array_at<'a>(root: &'a Dictionary, path: &str) -> Result<&'a [Value], AppError> {
    match lookup(root, path)? {
        Value::Array(array) => Ok(array),
        other => Err(type_mismatch(path, "array", other)),
    }
}

/// Existing value under `key`, or `default` inserted at the end.
fn slot_or_insert<'a>(
    dict: &'a mut Dictionary,
    key: &str,
    default: Value,
    path: &str,
) -> Result<&'a mut Value, AppError> {
    if !dict.contains_key(key) {
        dict.insert(key.to_string(), default);
    }
    dict.get_mut(key).ok_or_else(|| key_unknown(path))
}

/// Replace an existing value, keeping the template's type.
fn set_path(root: &mut Dictionary, path: &str, value: Value) -> Result<(), AppError> {
    let slot = lookup_mut(root, path)?;
    check_kind(slot, &value, path)?;
    *slot = value;
    Ok(())
}

/// Apply plan overrides below `base`. Keys may name nested values with `/`
/// ("Generic/ProcessorType"); each must exist with the same type.
fn apply_overrides(
    root: &mut Dictionary,
    base: &str,
    overrides: &SettingMap,
) -> Result<(), AppError> {
    for (key, scalar) in overrides {
        let path = format!("{base}/{key}");
        let value = scalar_value(scalar, &path)?;
        set_path(root, &path, value)?;
    }
    Ok(())
}

// ── Errors ──────────────────────────────────────────────────────────────────

fn key_unknown(path: &str) -> AppError {
    AppError::new(
        "SCHEMA_KEY_UNKNOWN",
        format!("{path} does not exist in this OpenCore version's Sample.plist"),
    )
    .with_context(json!({ "path": path }))
}

fn type_mismatch(path: &str, expected: &str, found: &Value) -> AppError {
    AppError::new(
        "SCHEMA_TYPE_MISMATCH",
        format!("{path} must be of type {expected}, got {}", kind(found)),
    )
    .with_context(json!({ "path": path, "expected": expected, "found": kind(found) }))
}

fn invalid(path: &str, message: &str) -> AppError {
    AppError::new("CONFIG_VALUE_INVALID", format!("{path}: {message}"))
        .with_context(json!({ "path": path }))
}

// ── XML output ──────────────────────────────────────────────────────────────

/// Apple-style XML plist with tab indentation and single-line data, the
/// same layout as OpenCore's Sample.plist.
fn to_xml(root: &Dictionary) -> Result<Vec<u8>, AppError> {
    let mut out = String::with_capacity(64 * 1024);
    out.push_str(XML_HEADER);
    write_dict(&mut out, root, 0)?;
    out.push_str("</plist>\n");
    Ok(out.into_bytes())
}

fn write_dict(out: &mut String, dict: &Dictionary, depth: usize) -> Result<(), AppError> {
    indent(out, depth);
    if dict.is_empty() {
        out.push_str("<dict/>\n");
        return Ok(());
    }
    out.push_str("<dict>\n");
    for (key, item) in dict {
        indent(out, depth + 1);
        out.push_str("<key>");
        escape_into(out, key);
        out.push_str("</key>\n");
        write_value(out, item, depth + 1)?;
    }
    indent(out, depth);
    out.push_str("</dict>\n");
    Ok(())
}

fn write_value(out: &mut String, value: &Value, depth: usize) -> Result<(), AppError> {
    if let Value::Dictionary(dict) = value {
        return write_dict(out, dict, depth);
    }
    indent(out, depth);
    match value {
        Value::Array(items) if items.is_empty() => out.push_str("<array/>\n"),
        Value::Array(items) => {
            out.push_str("<array>\n");
            for item in items {
                write_value(out, item, depth + 1)?;
            }
            indent(out, depth);
            out.push_str("</array>\n");
        }
        Value::Boolean(true) => out.push_str("<true/>\n"),
        Value::Boolean(false) => out.push_str("<false/>\n"),
        Value::Data(bytes) => {
            out.push_str("<data>");
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(bytes));
            out.push_str("</data>\n");
        }
        Value::Integer(i) => out.push_str(&format!("<integer>{i}</integer>\n")),
        Value::Real(r) => out.push_str(&format!("<real>{r}</real>\n")),
        Value::String(s) => {
            out.push_str("<string>");
            escape_into(out, s);
            out.push_str("</string>\n");
        }
        Value::Date(date) => out.push_str(&format!("<date>{}</date>\n", date.to_xml_format())),
        other => {
            return Err(AppError::new(
                "CONFIG_SERIALIZE_FAILED",
                format!("config.plist cannot contain a {} value", kind(other)),
            ))
        }
    }
    Ok(())
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push('\t');
    }
}

/// XML text escaping; control characters XML 1.0 cannot carry are dropped.
fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        AcpiDelete, AcpiPatch, DeviceProperty, DevicePropertyEntry, DriverPlan, KernelBlock,
        MacOsVersion, NvramVariable, SmbiosPlan, SsdtPlan, SsdtSource,
    };

    const SAMPLE: &[u8] = include_bytes!("../../tests/fixtures/Sample-1.0.8.plist");

    fn plan() -> BuildPlan {
        let ssdt = |name: &str| SsdtPlan {
            file_name: name.to_string(),
            source: SsdtSource::OcSample {
                file: name.to_string(),
            },
            required: true,
            reason: String::new(),
        };
        let driver = |path: &str, load_early: bool| DriverPlan {
            path: path.to_string(),
            load_early,
            enabled: true,
            comment: String::new(),
            source: "opencore".to_string(),
        };
        BuildPlan {
            target: MacOsVersion::Sequoia,
            smbios: SmbiosPlan {
                model: "iMac19,1".to_string(),
                reason: String::new(),
                secure_boot_model: "Disabled".to_string(),
                board_id_skip: false,
                alternatives: Vec::new(),
            },
            ssdts: vec![
                ssdt("SSDT-PLUG.aml"),
                ssdt("SSDT-EC-USBX.aml"),
                ssdt("SSDT-AWAC-DISABLE.aml"),
            ],
            acpi_patches: Vec::new(),
            acpi_deletes: Vec::new(),
            acpi_quirks: SettingMap::new(),
            booter_quirks: SettingMap::from([
                ("DevirtualiseMmio".to_string(), PlistScalar::Bool(true)),
                ("ResizeAppleGpuBars".to_string(), PlistScalar::Int(-1)),
            ]),
            booter_patches: Vec::new(),
            mmio_whitelist: Vec::new(),
            device_properties: Vec::new(),
            kexts: Vec::new(),
            kernel_patches: Vec::new(),
            amd_core_count: None,
            kernel_blocks: Vec::new(),
            kernel_quirks: SettingMap::from([(
                "AppleXcpmCfgLock".to_string(),
                PlistScalar::Bool(true),
            )]),
            kernel_emulate: SettingMap::new(),
            misc_boot: SettingMap::from([(
                "PickerMode".to_string(),
                PlistScalar::Str("External".to_string()),
            )]),
            misc_debug: SettingMap::from([("Target".to_string(), PlistScalar::Int(67))]),
            misc_security: SettingMap::new(),
            tools: vec!["OpenShell.efi".to_string()],
            boot_args: vec![
                "-v".to_string(),
                "keepsyms=1 debug=0x100".to_string(),
                "-v".to_string(),
            ],
            csr_active_config: 0x0000_0803,
            nvram_add: Vec::new(),
            nvram_delete: Vec::new(),
            nvram_settings: SettingMap::from([("WriteFlash".to_string(), PlistScalar::Bool(true))]),
            platform_info: SettingMap::new(),
            drivers: vec![
                driver("OpenRuntime.efi", false),
                driver("HfsPlus.efi", false),
                driver("OpenCanopy.efi", false),
            ],
            uefi_quirks: SettingMap::new(),
            uefi_apfs: SettingMap::new(),
            uefi_output: SettingMap::new(),
            uefi_input: SettingMap::new(),
            bios_settings: Vec::new(),
            notes: Vec::new(),
            post_install: Vec::new(),
        }
    }

    fn identity() -> PlatformIdentity {
        PlatformIdentity {
            model: "iMac19,1".to_string(),
            serial: "C02XG0FDH7JY".to_string(),
            mlb: "C02839303QXH69FJA".to_string(),
            system_uuid: "dbb364d6-44b2-4a02-b922-ab4396f16da8".to_string(),
            rom: "112233445566".to_string(),
        }
    }

    fn kext(bundle: &str, exe: &str) -> KernelAddEntry {
        KernelAddEntry {
            arch: "Any".to_string(),
            bundle_path: bundle.to_string(),
            comment: bundle.to_string(),
            enabled: true,
            executable_path: exe.to_string(),
            max_kernel: String::new(),
            min_kernel: String::new(),
            plist_path: "Contents/Info.plist".to_string(),
            bundle_id: String::new(),
        }
    }

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    struct Fixture {
        plan: BuildPlan,
        identity: PlatformIdentity,
        kexts: Vec<KernelAddEntry>,
        ssdts: Vec<String>,
        drivers: Vec<String>,
        tools: Vec<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                plan: plan(),
                identity: identity(),
                kexts: vec![
                    kext("Lilu.kext", "Contents/MacOS/Lilu"),
                    kext("VirtualSMC.kext", "Contents/MacOS/VirtualSMC"),
                    kext("USBMap.kext", ""),
                ],
                ssdts: files(&[
                    "SSDT-AWAC-DISABLE.aml",
                    "SSDT-EC-USBX.aml",
                    "SSDT-PLUG.aml",
                    "SSDT-EXTRA.aml",
                ]),
                drivers: files(&[
                    "OpenRuntime.efi",
                    "HfsPlus.efi",
                    "OpenCanopy.efi",
                    "ResetNvramEntry.efi",
                ]),
                tools: files(&["OpenShell.efi"]),
            }
        }

        fn write(&self) -> Result<Vec<u8>, AppError> {
            write_config(
                SAMPLE,
                &ConfigInputs {
                    plan: &self.plan,
                    kernel_add: &self.kexts,
                    identity: &self.identity,
                    ssdt_files: &self.ssdts,
                    driver_files: &self.drivers,
                    tool_files: &self.tools,
                },
            )
        }

        fn config(&self) -> Dictionary {
            parse_sample(&self.write().unwrap()).unwrap()
        }
    }

    fn get<'a>(root: &'a Dictionary, path: &str) -> &'a Value {
        lookup(root, path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn entries<'a>(root: &'a Dictionary, path: &str) -> Vec<&'a Dictionary> {
        get(root, path)
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_dictionary().unwrap())
            .collect()
    }

    fn str_at<'a>(d: &'a Dictionary, key: &str) -> &'a str {
        d.get(key)
            .and_then(Value::as_string)
            .unwrap_or_else(|| panic!("{key} missing"))
    }

    fn data_at<'a>(d: &'a Dictionary, key: &str) -> &'a [u8] {
        d.get(key)
            .and_then(Value::as_data)
            .unwrap_or_else(|| panic!("{key} missing"))
    }

    fn keys(d: &Dictionary) -> Vec<&str> {
        d.keys().map(String::as_str).collect()
    }

    /// Same keys, same order, same types as the Sample, except free-form maps.
    fn assert_schema(sample: &Value, out: &Value, path: &str) {
        const FREE_FORM: &[&str] = &[
            "DeviceProperties/Add",
            "DeviceProperties/Delete",
            "NVRAM/Add",
            "NVRAM/Delete",
        ];
        assert_eq!(kind(sample), kind(out), "{path}");
        match (sample, out) {
            (Value::Dictionary(s), Value::Dictionary(o)) if !FREE_FORM.contains(&path) => {
                let s_keys: Vec<_> = s.keys().filter(|k| !k.starts_with('#')).collect();
                let o_keys: Vec<_> = o.keys().collect();
                assert_eq!(s_keys, o_keys, "{path}");
                for (k, v) in o {
                    let child = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}/{k}")
                    };
                    assert_schema(&s[k.as_str()], v, &child);
                }
            }
            (Value::Array(s), Value::Array(o)) => {
                if let Some(first) = s.first() {
                    for (i, item) in o.iter().enumerate() {
                        assert!(kind(item) == kind(first), "{path}[{i}]");
                        assert_schema(first, item, path);
                    }
                }
            }
            _ => {}
        }
    }

    #[test]
    fn sample_round_trips_byte_for_byte() {
        let parsed = parse_sample(SAMPLE).unwrap();
        assert_eq!(
            String::from_utf8(to_xml(&parsed).unwrap()).unwrap(),
            String::from_utf8(SAMPLE.to_vec()).unwrap()
        );
    }

    #[test]
    fn output_keeps_the_sample_schema() {
        let out = Fixture::new().config();
        let sample = parse_sample(SAMPLE).unwrap();
        assert_schema(
            &Value::Dictionary(sample),
            &Value::Dictionary(out.clone()),
            "",
        );
        assert!(!out.keys().any(|k| k.starts_with('#')));
    }

    #[test]
    fn output_is_apple_xml_with_tabs() {
        let xml = String::from_utf8(Fixture::new().write().unwrap()).unwrap();
        assert!(xml.starts_with(XML_HEADER));
        assert!(xml.ends_with("</dict>\n</plist>\n"));
        assert!(xml.contains("\n\t<key>ACPI</key>\n\t<dict>\n\t\t<key>Add</key>\n"));
        assert!(
            xml.contains("<data>AwgAAA==</data>"),
            "csr-active-config single-line data"
        );
    }

    #[test]
    fn acpi_add_follows_plan_order_then_extra_files() {
        let out = Fixture::new().config();
        let add = entries(&out, "ACPI/Add");
        let paths: Vec<_> = add.iter().map(|e| str_at(e, "Path")).collect();
        assert_eq!(
            paths,
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX.aml",
                "SSDT-AWAC-DISABLE.aml",
                "SSDT-EXTRA.aml"
            ]
        );
        assert!(add
            .iter()
            .all(|e| e.get("Enabled") == Some(&Value::Boolean(true))));
        assert_eq!(keys(add[0]), ["Comment", "Enabled", "Path"]);
    }

    #[test]
    fn missing_planned_ssdt_is_skipped() {
        let mut f = Fixture::new();
        f.ssdts = files(&["SSDT-EC-USBX.aml"]);
        let out = f.config();
        let paths: Vec<_> = entries(&out, "ACPI/Add")
            .iter()
            .map(|e| str_at(e, "Path"))
            .collect();
        assert_eq!(paths, ["SSDT-EC-USBX.aml"]);
    }

    #[test]
    fn acpi_delete_and_patch_entries() {
        let mut f = Fixture::new();
        f.plan.acpi_deletes = vec![
            AcpiDelete {
                comment: "Delete CpuPm".into(),
                table_signature: "SSDT".into(),
                oem_table_id: "CpuPm".into(),
                all: true,
            },
            AcpiDelete {
                comment: "Delete by raw id".into(),
                table_signature: "53534454".into(),
                oem_table_id: "4370753049737400".into(),
                all: false,
            },
            AcpiDelete {
                comment: "Space padded".into(),
                table_signature: "SSDT".into(),
                oem_table_id: "SataTabl".into(),
                all: false,
            },
        ];
        f.plan.acpi_patches = vec![AcpiPatch {
            comment: "_OSI to XOSI".into(),
            find: "5F4F5349".into(),
            replace: "584f5349".into(),
            table_signature: Some("DSDT".into()),
            oem_table_id: None,
            count: 0,
            enabled: true,
        }];
        let out = f.config();

        let delete = entries(&out, "ACPI/Delete");
        assert_eq!(delete.len(), 3);
        assert_eq!(
            keys(delete[0]),
            [
                "All",
                "Comment",
                "Enabled",
                "OemTableId",
                "TableLength",
                "TableSignature"
            ]
        );
        assert_eq!(data_at(delete[0], "OemTableId"), b"CpuPm\0\0\0");
        assert_eq!(data_at(delete[0], "TableSignature"), b"SSDT");
        assert_eq!(delete[0].get("All"), Some(&Value::Boolean(true)));
        assert_eq!(delete[0].get("Enabled"), Some(&Value::Boolean(true)));
        assert_eq!(data_at(delete[1], "OemTableId"), b"Cpu0Ist\0");
        assert_eq!(data_at(delete[1], "TableSignature"), b"SSDT");
        assert_eq!(data_at(delete[2], "OemTableId"), b"SataTabl");

        let patch = entries(&out, "ACPI/Patch");
        assert_eq!(patch.len(), 1);
        let p = patch[0];
        assert_eq!(
            keys(p),
            [
                "Base",
                "BaseSkip",
                "Comment",
                "Count",
                "Enabled",
                "Find",
                "Limit",
                "Mask",
                "OemTableId",
                "Replace",
                "ReplaceMask",
                "Skip",
                "TableLength",
                "TableSignature"
            ]
        );
        assert_eq!(data_at(p, "Find"), b"_OSI");
        assert_eq!(data_at(p, "Replace"), b"XOSI");
        assert_eq!(data_at(p, "TableSignature"), b"DSDT");
        assert_eq!(data_at(p, "OemTableId"), b"");
        assert_eq!(data_at(p, "Mask"), b"");
        assert_eq!(str_at(p, "Base"), "");
        assert_eq!(p.get("Count").and_then(Value::as_signed_integer), Some(0));
    }

    #[test]
    fn acpi_entries_are_validated() {
        let mut f = Fixture::new();
        f.plan.acpi_patches = vec![AcpiPatch {
            comment: "uneven".into(),
            find: "5F4F5349".into(),
            replace: "584F".into(),
            table_signature: None,
            oem_table_id: None,
            count: 0,
            enabled: true,
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.acpi_deletes = vec![AcpiDelete {
            comment: "matches everything".into(),
            table_signature: String::new(),
            oem_table_id: String::new(),
            all: true,
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.acpi_deletes = vec![AcpiDelete {
            comment: "too long".into(),
            table_signature: "SSDT".into(),
            oem_table_id: "NineChars".into(),
            all: false,
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn booter_section() {
        let mut f = Fixture::new();
        f.plan.booter_patches = vec![BinaryPatch {
            comment: "Skip Board ID check".into(),
            arch: "x86_64".into(),
            identifier: "Apple".into(),
            base: String::new(),
            find: "0050006C006100740066006F0072006D0053007500700070006F00720074002E0070006C006900730074".into(),
            mask: String::new(),
            replace: "002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E002E".into(),
            replace_mask: String::new(),
            count: 0,
            limit: 0,
            skip: 0,
            min_kernel: String::new(),
            max_kernel: String::new(),
            enabled: true,
        }];
        let out = f.config();
        assert!(get(&out, "Booter/MmioWhitelist")
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            get(&out, "Booter/Quirks/DevirtualiseMmio"),
            &Value::Boolean(true)
        );
        assert_eq!(
            get(&out, "Booter/Quirks/ResizeAppleGpuBars").as_signed_integer(),
            Some(-1)
        );
        let patch = entries(&out, "Booter/Patch");
        assert_eq!(patch.len(), 1);
        assert_eq!(
            keys(patch[0]),
            [
                "Arch",
                "Comment",
                "Count",
                "Enabled",
                "Find",
                "Identifier",
                "Limit",
                "Mask",
                "Replace",
                "ReplaceMask",
                "Skip"
            ]
        );
        assert_eq!(str_at(patch[0], "Arch"), "x86_64");
        assert_eq!(str_at(patch[0], "Identifier"), "Apple");
        assert_eq!(data_at(patch[0], "Find").len(), 42);
        assert_eq!(
            patch[0].get("Count").and_then(Value::as_signed_integer),
            Some(0)
        );
    }

    #[test]
    fn device_properties_use_plist_types() {
        let mut f = Fixture::new();
        f.plan.device_properties = vec![
            DevicePropertyEntry {
                path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
                properties: vec![
                    DeviceProperty {
                        key: "AAPL,ig-platform-id".into(),
                        value: PlistScalar::Data("07009B3E".into()),
                    },
                    DeviceProperty {
                        key: "framebuffer-patch-enable".into(),
                        value: PlistScalar::Int(1),
                    },
                    DeviceProperty {
                        key: "model".into(),
                        value: PlistScalar::Str("UHD 630".into()),
                    },
                    DeviceProperty {
                        key: "built-in".into(),
                        value: PlistScalar::Bool(true),
                    },
                ],
                reason: String::new(),
            },
            DevicePropertyEntry {
                path: "PciRoot(0x0)/Pci(0x1f,0x3)".into(),
                properties: vec![DeviceProperty {
                    key: "layout-id".into(),
                    value: PlistScalar::Data("0B000000".into()),
                }],
                reason: String::new(),
            },
        ];
        let out = f.config();
        let add = get(&out, "DeviceProperties/Add").as_dictionary().unwrap();
        assert_eq!(
            keys(add),
            ["PciRoot(0x0)/Pci(0x2,0x0)", "PciRoot(0x0)/Pci(0x1f,0x3)"]
        );
        let igpu = add["PciRoot(0x0)/Pci(0x2,0x0)"].as_dictionary().unwrap();
        assert_eq!(
            data_at(igpu, "AAPL,ig-platform-id"),
            [0x07, 0x00, 0x9B, 0x3E]
        );
        assert_eq!(
            igpu["framebuffer-patch-enable"].as_signed_integer(),
            Some(1)
        );
        assert_eq!(str_at(igpu, "model"), "UHD 630");
        assert_eq!(igpu["built-in"], Value::Boolean(true));
        assert!(get(&out, "DeviceProperties/Delete")
            .as_dictionary()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn kernel_section() {
        let mut f = Fixture::new();
        f.plan.kernel_blocks = vec![KernelBlock {
            comment: "Allow IOSkywalk Downgrade".into(),
            identifier: "com.apple.iokit.IOSkywalkFamily".into(),
            strategy: "Exclude".into(),
            min_kernel: "23.0.0".into(),
            max_kernel: String::new(),
            enabled: true,
        }];
        f.plan.kernel_patches = vec![BinaryPatch {
            comment: "algrey | Force cpuid_cores_per_package to constant (user-specified) | 13.3+"
                .into(),
            arch: "x86_64".into(),
            identifier: "kernel".into(),
            base: "_cpuid_set_info ".into(),
            find: "C1E81A0000".into(),
            mask: "FFFDFF0000".into(),
            replace: "BA08000000".into(),
            replace_mask: "FFFFFFFFFF".into(),
            count: 1,
            limit: 0,
            skip: 0,
            min_kernel: "22.4.0".into(),
            max_kernel: "25.99.99".into(),
            enabled: true,
        }];
        f.plan.kernel_emulate = SettingMap::from([
            (
                "Cpuid1Data".to_string(),
                PlistScalar::Data("55060A00000000000000000000000000".into()),
            ),
            (
                "Cpuid1Mask".to_string(),
                PlistScalar::Data("FFFFFFFF000000000000000000000000".into()),
            ),
        ]);
        let out = f.config();

        let add = entries(&out, "Kernel/Add");
        assert_eq!(add.len(), 3);
        assert_eq!(
            keys(add[0]),
            [
                "Arch",
                "BundlePath",
                "Comment",
                "Enabled",
                "ExecutablePath",
                "MaxKernel",
                "MinKernel",
                "PlistPath"
            ]
        );
        assert_eq!(str_at(add[0], "BundlePath"), "Lilu.kext");
        assert_eq!(str_at(add[2], "ExecutablePath"), "");
        assert_eq!(str_at(add[2], "PlistPath"), "Contents/Info.plist");

        let block = entries(&out, "Kernel/Block");
        assert_eq!(
            keys(block[0]),
            [
                "Arch",
                "Comment",
                "Enabled",
                "Identifier",
                "MaxKernel",
                "MinKernel",
                "Strategy"
            ]
        );
        assert_eq!(str_at(block[0], "Strategy"), "Exclude");
        assert_eq!(str_at(block[0], "Arch"), "Any");

        assert!(get(&out, "Kernel/Force").as_array().unwrap().is_empty());

        let patch = entries(&out, "Kernel/Patch");
        assert_eq!(patch.len(), 1);
        let p = patch[0];
        assert_eq!(
            keys(p),
            [
                "Arch",
                "Base",
                "Comment",
                "Count",
                "Enabled",
                "Find",
                "Identifier",
                "Limit",
                "Mask",
                "MaxKernel",
                "MinKernel",
                "Replace",
                "ReplaceMask",
                "Skip"
            ]
        );
        assert_eq!(str_at(p, "Base"), "_cpuid_set_info");
        assert_eq!(data_at(p, "Replace"), [0xBA, 0x08, 0, 0, 0]);
        assert_eq!(data_at(p, "Mask"), [0xFF, 0xFD, 0xFF, 0, 0]);
        assert_eq!(str_at(p, "MinKernel"), "22.4.0");

        assert_eq!(
            data_at(
                get(&out, "Kernel/Emulate").as_dictionary().unwrap(),
                "Cpuid1Data"
            )[0],
            0x55
        );
        assert_eq!(
            get(&out, "Kernel/Quirks/AppleXcpmCfgLock"),
            &Value::Boolean(true)
        );
    }

    #[test]
    fn kernel_entries_are_validated() {
        let mut f = Fixture::new();
        f.plan.kernel_blocks = vec![KernelBlock {
            comment: String::new(),
            identifier: "com.apple.iokit.IOSkywalkFamily".into(),
            strategy: "Remove".into(),
            min_kernel: String::new(),
            max_kernel: String::new(),
            enabled: true,
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.kexts[0].arch = "arm64".into();
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.kernel_patches = vec![BinaryPatch {
            comment: "bad".into(),
            arch: "Any".into(),
            identifier: "kernel".into(),
            base: String::new(),
            find: "0011".into(),
            mask: "FF".into(),
            replace: "2233".into(),
            replace_mask: String::new(),
            count: 1,
            limit: 0,
            skip: 0,
            min_kernel: String::new(),
            max_kernel: String::new(),
            enabled: true,
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn misc_section() {
        let out = Fixture::new().config();
        assert_eq!(
            str_at(get(&out, "Misc/Security").as_dictionary().unwrap(), "Vault"),
            "Optional"
        );
        assert_eq!(
            get(&out, "Misc/Security/ScanPolicy").as_signed_integer(),
            Some(0)
        );
        assert_eq!(
            get(&out, "Misc/Security/SecureBootModel").as_string(),
            Some("Disabled")
        );
        assert_eq!(
            get(&out, "Misc/Boot/PickerMode").as_string(),
            Some("External")
        );
        assert_eq!(get(&out, "Misc/Debug/Target").as_signed_integer(), Some(67));
        assert!(get(&out, "Misc/Entries").as_array().unwrap().is_empty());

        let tools = entries(&out, "Misc/Tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(str_at(tools[0], "Path"), "OpenShell.efi");
        assert_eq!(str_at(tools[0], "Flavour"), "OpenShell:UEFIShell:Shell");
        assert_eq!(str_at(tools[0], "Name"), "UEFI Shell");
        assert_eq!(tools[0].get("Enabled"), Some(&Value::Boolean(true)));
        assert_eq!(tools[0].get("Auxiliary"), Some(&Value::Boolean(true)));
    }

    #[test]
    fn unknown_tool_gets_snapshot_defaults() {
        let mut f = Fixture::new();
        f.plan.tools = vec!["CleanNvram.efi".into()];
        f.tools = files(&["CleanNvram.efi"]);
        let out = f.config();
        let tools = entries(&out, "Misc/Tools");
        assert_eq!(
            keys(tools[0]),
            [
                "Arguments",
                "Auxiliary",
                "Comment",
                "Enabled",
                "Flavour",
                "FullNvramAccess",
                "Name",
                "Path",
                "RealPath",
                "TextMode"
            ]
        );
        assert_eq!(str_at(tools[0], "Name"), "CleanNvram.efi");
        assert_eq!(str_at(tools[0], "Flavour"), "Auto");
        assert_eq!(str_at(tools[0], "Arguments"), "");
        assert_eq!(tools[0].get("RealPath"), Some(&Value::Boolean(false)));
    }

    #[test]
    fn nvram_section() {
        let mut f = Fixture::new();
        f.plan.nvram_add = vec![
            NvramVariable {
                guid: "4d1fda02-38c7-4a6a-9cc6-4bcca8b30102".into(),
                key: "revpatch".into(),
                value: PlistScalar::Str("sbvmm".into()),
            },
            NvramVariable {
                guid: APPLE_BOOT_VARIABLE_GUID.into(),
                key: "bluetoothExternalDongleFailed".into(),
                value: PlistScalar::Data("00".into()),
            },
            NvramVariable {
                guid: "12345678-1234-1234-1234-123456789ABC".into(),
                key: "custom".into(),
                value: PlistScalar::Int(5),
            },
        ];
        f.plan.nvram_delete = vec![NvramVariable {
            guid: APPLE_BOOT_VARIABLE_GUID.into(),
            key: "nvda_drv".into(),
            value: PlistScalar::Bool(false),
        }];
        let out = f.config();

        let apple = get(&out, &format!("NVRAM/Add/{APPLE_BOOT_VARIABLE_GUID}"))
            .as_dictionary()
            .unwrap();
        assert_eq!(str_at(apple, "boot-args"), "-v keepsyms=1 debug=0x100");
        assert_eq!(
            data_at(apple, "csr-active-config"),
            [0x03, 0x08, 0x00, 0x00]
        );
        assert_eq!(data_at(apple, "prev-lang:kbd"), b"en-US:0");
        assert_eq!(str_at(apple, "run-efi-updater"), "No");
        assert_eq!(data_at(apple, "bluetoothExternalDongleFailed"), [0]);
        assert!(!apple.contains_key("ForceDisplayRotationInEFI"));
        assert!(!apple.keys().any(|k| k.starts_with('#')));
        assert!(apple.contains_key("SystemAudioVolume"));

        let oc = get(&out, "NVRAM/Add/4D1FDA02-38C7-4A6A-9CC6-4BCCA8B30102")
            .as_dictionary()
            .unwrap();
        assert_eq!(str_at(oc, "revpatch"), "sbvmm");
        assert!(oc.contains_key("rtc-blacklist"));
        let custom = get(&out, "NVRAM/Add/12345678-1234-1234-1234-123456789ABC")
            .as_dictionary()
            .unwrap();
        assert_eq!(custom["custom"].as_signed_integer(), Some(5));

        let delete = |guid: &str| -> Vec<String> {
            get(&out, &format!("NVRAM/Delete/{guid}"))
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_string().unwrap().to_string())
                .collect()
        };
        let apple_delete = delete(APPLE_BOOT_VARIABLE_GUID);
        for key in [
            "boot-args",
            "csr-active-config",
            "prev-lang:kbd",
            "run-efi-updater",
            "bluetoothExternalDongleFailed",
            "nvda_drv",
        ] {
            assert!(
                apple_delete.iter().any(|k| k == key),
                "{key} not deleted: {apple_delete:?}"
            );
        }
        assert!(!apple_delete
            .iter()
            .any(|k| k == "ForceDisplayRotationInEFI"));
        assert_eq!(apple_delete.iter().filter(|k| *k == "boot-args").count(), 1);
        assert_eq!(
            delete("4D1FDA02-38C7-4A6A-9CC6-4BCCA8B30102"),
            ["rtc-blacklist", "revpatch"]
        );
        assert_eq!(delete("12345678-1234-1234-1234-123456789ABC"), ["custom"]);
        assert!(lookup(&out, "NVRAM/LegacySchema").is_ok());
        assert_eq!(get(&out, "NVRAM/WriteFlash"), &Value::Boolean(true));
    }

    #[test]
    fn nvram_guid_is_validated() {
        let mut f = Fixture::new();
        f.plan.nvram_add = vec![NvramVariable {
            guid: "not-a-guid".into(),
            key: "x".into(),
            value: PlistScalar::Bool(true),
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn platform_info_section() {
        let out = Fixture::new().config();
        let generic = get(&out, "PlatformInfo/Generic").as_dictionary().unwrap();
        assert_eq!(str_at(generic, "SystemProductName"), "iMac19,1");
        assert_eq!(str_at(generic, "SystemSerialNumber"), "C02XG0FDH7JY");
        assert_eq!(str_at(generic, "MLB"), "C02839303QXH69FJA");
        assert_eq!(
            str_at(generic, "SystemUUID"),
            "DBB364D6-44B2-4A02-B922-AB4396F16DA8"
        );
        assert_eq!(
            data_at(generic, "ROM"),
            [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]
        );
        assert_eq!(generic["SpoofVendor"], Value::Boolean(true));
        assert_eq!(generic["ProcessorType"].as_signed_integer(), Some(0));
        assert_eq!(str_at(generic, "SystemMemoryStatus"), "Auto");
        assert_eq!(get(&out, "PlatformInfo/Automatic"), &Value::Boolean(true));
        assert_eq!(
            get(&out, "PlatformInfo/UpdateSMBIOSMode").as_string(),
            Some("Create")
        );
        assert_eq!(
            get(&out, "Kernel/Quirks/CustomSMBIOSGuid"),
            &Value::Boolean(false)
        );
    }

    #[test]
    fn custom_smbios_mode_enables_the_guid_quirk() {
        let mut f = Fixture::new();
        f.plan.platform_info = SettingMap::from([
            (
                "UpdateSMBIOSMode".to_string(),
                PlistScalar::Str("Custom".into()),
            ),
            (
                "Generic/ProcessorType".to_string(),
                PlistScalar::Int(0x0F01),
            ),
        ]);
        let out = f.config();
        assert_eq!(
            get(&out, "PlatformInfo/UpdateSMBIOSMode").as_string(),
            Some("Custom")
        );
        assert_eq!(
            get(&out, "Kernel/Quirks/CustomSMBIOSGuid"),
            &Value::Boolean(true)
        );
        assert_eq!(
            get(&out, "PlatformInfo/Generic/ProcessorType").as_signed_integer(),
            Some(3841)
        );
    }

    #[test]
    fn identity_is_validated() {
        let mut f = Fixture::new();
        f.identity.model = "iMac20,1".into();
        assert_eq!(f.write().unwrap_err().code, "IDENTITY_MODEL_MISMATCH");

        let mut f = Fixture::new();
        f.identity.rom = "1122334455".into();
        assert_eq!(f.write().unwrap_err().code, "IDENTITY_INVALID");

        let mut f = Fixture::new();
        f.identity.system_uuid = "nope".into();
        assert_eq!(f.write().unwrap_err().code, "IDENTITY_INVALID");

        let mut f = Fixture::new();
        f.identity.mlb = String::new();
        assert_eq!(f.write().unwrap_err().code, "IDENTITY_INVALID");
    }

    #[test]
    fn uefi_section() {
        let mut f = Fixture::new();
        f.plan.drivers.insert(
            0,
            DriverPlan {
                path: "OpenVariableRuntimeDxe.efi".into(),
                load_early: true,
                enabled: true,
                comment: "Emulated NVRAM".into(),
                source: "opencore".into(),
            },
        );
        f.drivers.push("OpenVariableRuntimeDxe.efi".into());
        f.plan.drivers.push(DriverPlan {
            path: "OpenRuntime.efi".into(),
            load_early: false,
            enabled: true,
            comment: "duplicate".into(),
            source: "opencore".into(),
        });
        f.plan.uefi_apfs = SettingMap::from([
            ("MinDate".to_string(), PlistScalar::Int(-1)),
            ("MinVersion".to_string(), PlistScalar::Int(-1)),
        ]);
        f.plan.uefi_quirks =
            SettingMap::from([("ReleaseUsbOwnership".to_string(), PlistScalar::Bool(true))]);
        f.plan.uefi_output =
            SettingMap::from([("ProvideConsoleGop".to_string(), PlistScalar::Bool(true))]);
        f.plan.uefi_input =
            SettingMap::from([("KeySupport".to_string(), PlistScalar::Bool(false))]);
        let out = f.config();

        let drivers = entries(&out, "UEFI/Drivers");
        let paths: Vec<_> = drivers.iter().map(|d| str_at(d, "Path")).collect();
        assert_eq!(
            paths,
            [
                "OpenVariableRuntimeDxe.efi",
                "OpenRuntime.efi",
                "HfsPlus.efi",
                "OpenCanopy.efi"
            ]
        );
        assert_eq!(
            keys(drivers[0]),
            [
                "Arguments",
                "Comment",
                "Enabled",
                "HideVerbose",
                "LoadEarly",
                "Path"
            ]
        );
        assert_eq!(drivers[0]["LoadEarly"], Value::Boolean(true));
        assert_eq!(str_at(drivers[0], "Comment"), "Emulated NVRAM");
        assert_eq!(drivers[1]["LoadEarly"], Value::Boolean(false));
        assert_eq!(
            drivers[2]["HideVerbose"],
            Value::Boolean(true),
            "HfsPlus keeps the Sample's HideVerbose"
        );
        assert_eq!(drivers[1]["HideVerbose"], Value::Boolean(false));
        assert!(drivers.iter().all(|d| d["Enabled"] == Value::Boolean(true)));

        assert_eq!(get(&out, "UEFI/APFS/MinDate").as_signed_integer(), Some(-1));
        assert_eq!(
            get(&out, "UEFI/Quirks/ReleaseUsbOwnership"),
            &Value::Boolean(true)
        );
        assert_eq!(get(&out, "UEFI/Input/KeySupport"), &Value::Boolean(false));
        assert!(get(&out, "UEFI/ReservedMemory")
            .as_array()
            .unwrap()
            .is_empty());
        assert!(get(&out, "UEFI/Unload").as_array().unwrap().is_empty());
    }

    #[test]
    fn missing_driver_files_are_left_out() {
        let mut f = Fixture::new();
        f.drivers = files(&["OpenRuntime.efi"]);
        let out = f.config();
        let paths: Vec<_> = entries(&out, "UEFI/Drivers")
            .iter()
            .map(|d| str_at(d, "Path"))
            .collect();
        assert_eq!(paths, ["OpenRuntime.efi"]);
        // No OpenCanopy.efi: the external picker cannot work.
        assert_eq!(
            get(&out, "Misc/Boot/PickerMode").as_string(),
            Some("Builtin")
        );
    }

    #[test]
    fn unknown_override_key_is_rejected_with_its_path() {
        let mut f = Fixture::new();
        f.plan
            .kernel_quirks
            .insert("AllowNvramReset".into(), PlistScalar::Bool(true));
        let err = f.write().unwrap_err();
        assert_eq!(err.code, "SCHEMA_KEY_UNKNOWN");
        assert_eq!(
            err.context.unwrap()["path"],
            "Kernel/Quirks/AllowNvramReset"
        );

        let mut f = Fixture::new();
        f.plan
            .platform_info
            .insert("Generic/Nope".into(), PlistScalar::Bool(true));
        assert_eq!(f.write().unwrap_err().code, "SCHEMA_KEY_UNKNOWN");

        let mut f = Fixture::new();
        f.plan
            .nvram_settings
            .insert("LegacyEnable".into(), PlistScalar::Bool(true));
        assert_eq!(f.write().unwrap_err().code, "SCHEMA_KEY_UNKNOWN");
    }

    #[test]
    fn override_type_must_match_the_template() {
        let mut f = Fixture::new();
        f.plan
            .misc_boot
            .insert("Timeout".into(), PlistScalar::Str("5".into()));
        let err = f.write().unwrap_err();
        assert_eq!(err.code, "SCHEMA_TYPE_MISMATCH");
        assert_eq!(err.context.unwrap()["path"], "Misc/Boot/Timeout");

        let mut f = Fixture::new();
        f.plan
            .booter_quirks
            .insert("ProvideMaxSlide".into(), PlistScalar::Bool(true));
        assert_eq!(f.write().unwrap_err().code, "SCHEMA_TYPE_MISMATCH");

        // A section dict cannot be replaced by a scalar.
        let mut f = Fixture::new();
        f.plan
            .nvram_settings
            .insert("Add".into(), PlistScalar::Bool(true));
        assert_eq!(f.write().unwrap_err().code, "SCHEMA_TYPE_MISMATCH");

        let mut f = Fixture::new();
        f.plan
            .kernel_emulate
            .insert("Cpuid1Data".into(), PlistScalar::Data("XYZ".into()));
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn sample_errors_are_reported() {
        let f = Fixture::new();
        let inputs = ConfigInputs {
            plan: &f.plan,
            kernel_add: &f.kexts,
            identity: &f.identity,
            ssdt_files: &f.ssdts,
            driver_files: &f.drivers,
            tool_files: &f.tools,
        };
        assert_eq!(
            write_config(b"garbage", &inputs).unwrap_err().code,
            "SAMPLE_PLIST_INVALID"
        );
        let array_root = b"<?xml version=\"1.0\"?><plist version=\"1.0\"><array/></plist>";
        assert_eq!(
            write_config(array_root, &inputs).unwrap_err().code,
            "SAMPLE_PLIST_INVALID"
        );
        let no_acpi = b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict/></plist>";
        assert_eq!(
            write_config(no_acpi, &inputs).unwrap_err().code,
            "SCHEMA_KEY_UNKNOWN"
        );
    }

    fn kernel_patch(comment: &str, find: &str, mask: &str, replace: &str) -> BinaryPatch {
        BinaryPatch {
            comment: comment.into(),
            arch: "x86_64".into(),
            identifier: "kernel".into(),
            base: String::new(),
            find: find.into(),
            mask: mask.into(),
            replace: replace.into(),
            replace_mask: String::new(),
            count: 1,
            limit: 0,
            skip: 0,
            min_kernel: "21.0.0".into(),
            max_kernel: "25.99.99".into(),
            enabled: true,
        }
    }

    #[test]
    fn comments_are_reduced_to_printable_ascii() {
        let mut f = Fixture::new();
        f.plan.kernel_patches = vec![kernel_patch(
            "Fix \u{2014} macOS \u{2265} 26 \u{2192} \u{201C}ok\u{201D}\tcaf\u{e9}",
            "0011",
            "",
            "2233",
        )];
        f.plan.acpi_deletes = vec![AcpiDelete {
            comment: "Delete CpuPm\u{2026}".into(),
            table_signature: "SSDT".into(),
            oem_table_id: "CpuPm".into(),
            all: true,
        }];
        f.plan.drivers[0].comment = "Runtime\u{00a0}services".into();
        let out = f.config();
        assert_eq!(
            str_at(entries(&out, "Kernel/Patch")[0], "Comment"),
            "Fix - macOS >= 26 -> \"ok\" caf?"
        );
        assert_eq!(
            str_at(entries(&out, "ACPI/Delete")[0], "Comment"),
            "Delete CpuPm..."
        );
        assert_eq!(
            str_at(entries(&out, "UEFI/Drivers")[0], "Comment"),
            "Runtime?services"
        );
    }

    #[test]
    fn patch_masks_must_cover_the_data() {
        // AMD_Vanilla style masked patch is fine.
        let mut f = Fixture::new();
        f.plan.kernel_patches = vec![kernel_patch(
            "core count",
            "C1E81A0000",
            "FFFDFF0000",
            "BA08000000",
        )];
        f.write().unwrap();

        // 0xE8 has bit 1 clear in the mask 0xFD: fine. 0xEA would not be.
        let mut f = Fixture::new();
        f.plan.kernel_patches = vec![kernel_patch("bad mask", "C1EA", "FFFD", "9090")];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        let mut patch = kernel_patch("bad replace mask", "0011", "", "2233");
        patch.replace_mask = "FF0F".into();
        f.plan.kernel_patches = vec![patch];
        let err = f.write().unwrap_err();
        assert_eq!(err.code, "CONFIG_VALUE_INVALID");
        assert_eq!(err.context.unwrap()["path"], "Kernel/Patch[0]");
    }

    #[test]
    fn cpuid_spoof_needs_a_matching_mask() {
        let mut f = Fixture::new();
        f.plan.kernel_emulate = SettingMap::from([(
            "Cpuid1Data".to_string(),
            PlistScalar::Data("55060A00000000000000000000000000".into()),
        )]);
        let err = f.write().unwrap_err();
        assert_eq!(err.code, "CONFIG_VALUE_INVALID");
        assert_eq!(err.context.unwrap()["path"], "Kernel/Emulate/Cpuid1Data");

        let mut f = Fixture::new();
        f.plan.kernel_emulate = SettingMap::from([
            (
                "Cpuid1Data".to_string(),
                PlistScalar::Data("55060A00".into()),
            ),
            (
                "Cpuid1Mask".to_string(),
                PlistScalar::Data("FFFFFFFF".into()),
            ),
        ]);
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn identifiers_paths_and_versions_follow_opencore_rules() {
        let mut f = Fixture::new();
        f.plan.kernel_patches = vec![kernel_patch("x", "00", "", "01")];
        f.plan.kernel_patches[0].identifier = "IOPCIFamily".into();
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.kernel_patches = vec![kernel_patch("x", "00", "", "01")];
        f.plan.kernel_patches[0].max_kernel = "25.x".into();
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.kexts[0].bundle_path = "Lilu".into();
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.kexts[0].executable_path = "Contents/MacOS/Lilu Kext".into();
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.booter_patches = vec![BinaryPatch {
            identifier: "boot.efi.bak".into(),
            ..kernel_patch("x", "00", "", "01")
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.drivers[0].path = "Open Runtime.efi".into();
        f.drivers[0] = "Open Runtime.efi".into();
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn non_printable_names_and_boot_args_are_rejected() {
        let mut f = Fixture::new();
        f.plan.boot_args.push("alcid=\u{2011}1".into());
        let err = f.write().unwrap_err();
        assert_eq!(err.code, "CONFIG_VALUE_INVALID");
        assert_eq!(
            err.context.unwrap()["path"],
            format!("NVRAM/Add/{APPLE_BOOT_VARIABLE_GUID}/boot-args")
        );

        let mut f = Fixture::new();
        f.plan.nvram_add = vec![NvramVariable {
            guid: APPLE_BOOT_VARIABLE_GUID.into(),
            key: "caf\u{e9}".into(),
            value: PlistScalar::Bool(true),
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");

        let mut f = Fixture::new();
        f.plan.device_properties = vec![DevicePropertyEntry {
            path: "PciRoot(0x0)/Pci(0x2,0x0)".into(),
            properties: vec![DeviceProperty {
                key: "model\u{7}".into(),
                value: PlistScalar::Str("x".into()),
            }],
            reason: String::new(),
        }];
        assert_eq!(f.write().unwrap_err().code, "CONFIG_VALUE_INVALID");
    }

    #[test]
    fn hidden_and_foreign_files_are_not_loaded() {
        let mut f = Fixture::new();
        f.ssdts.extend(files(&[
            "._SSDT-PLUG.aml",
            ".DS_Store",
            "README.txt",
            "DSDT.bin",
        ]));
        f.tools.extend(files(&["._OpenShell.efi", "notes.md"]));
        let out = f.config();
        let acpi: Vec<_> = entries(&out, "ACPI/Add")
            .iter()
            .map(|e| str_at(e, "Path"))
            .collect();
        assert_eq!(
            acpi,
            [
                "SSDT-PLUG.aml",
                "SSDT-EC-USBX.aml",
                "SSDT-AWAC-DISABLE.aml",
                "SSDT-EXTRA.aml",
                "DSDT.bin"
            ]
        );
        let tools: Vec<_> = entries(&out, "Misc/Tools")
            .iter()
            .map(|e| str_at(e, "Path"))
            .collect();
        assert_eq!(tools, ["OpenShell.efi"]);
    }

    #[test]
    fn opencanopy_is_not_loaded_for_the_builtin_picker() {
        let mut f = Fixture::new();
        f.plan
            .misc_boot
            .insert("PickerMode".into(), PlistScalar::Str("Builtin".into()));
        let out = f.config();
        let drivers = entries(&out, "UEFI/Drivers");
        let canopy = drivers
            .iter()
            .find(|d| str_at(d, "Path") == "OpenCanopy.efi")
            .unwrap();
        assert_eq!(canopy["Enabled"], Value::Boolean(false));
        assert_eq!(
            get(&out, "Misc/Boot/PickerMode").as_string(),
            Some("Builtin")
        );
        assert!(drivers
            .iter()
            .filter(|d| str_at(d, "Path") != "OpenCanopy.efi")
            .all(|d| d["Enabled"] == Value::Boolean(true)));
    }

    #[test]
    fn helpers() {
        assert_eq!(kernel_version("", "x").unwrap(), "");
        assert_eq!(kernel_version("25.99.99", "x").unwrap(), "25.99.99");
        assert_eq!(kernel_version("8", "x").unwrap(), "8");
        assert!(kernel_version("7.0.0", "x").is_err());
        assert!(kernel_version("25.4.0.1", "x").is_err());
        assert!(kernel_version("25..0", "x").is_err());
        assert!(kernel_version("100.0.0", "x").is_err());
        assert_eq!(identifier("kernel", "x", false).unwrap(), "kernel");
        assert!(identifier("com.apple.iokit.IOPCIFamily", "x", false).is_ok());
        assert!(identifier("Apple", "x", false).is_err());
        assert!(identifier("Apple", "x", true).is_ok());
        assert!(identifier("boot.efi", "x", true).is_ok());
        assert!(identifier("kernel", "x", true).is_err());
        assert!(identifier("com.apple kext", "x", false).is_err());
        assert!(oc_path(
            "VoodooPS2Controller.kext/Contents/PlugIns/VoodooInput.kext",
            "x",
            Some(".kext")
        )
        .is_ok());
        assert!(oc_path("memtest86/BOOTX64.efi", "x", Some(".EFI")).is_ok());
        assert!(oc_path("", "x", None).is_err());
        assert!(oc_path(".kext", "x", Some(".kext")).is_err());
        assert!(has_extension("SSDT.AML", ".aml"));
        assert!(!has_extension("aml", ".aml"));
        assert_eq!(ascii_id("SSDT", 4, "x").unwrap(), b"SSDT");
        assert_eq!(ascii_id("CpuPm", 8, "x").unwrap(), b"CpuPm\0\0\0");
        assert_eq!(ascii_id("APIC  ", 8, "x").unwrap(), b"APIC  \0\0");
        assert_eq!(ascii_id("4350552D", 4, "x").unwrap(), b"CPU-");
        assert!(ascii_id("SSDTX", 4, "x").is_err());
        assert!(ascii_id("Cpu\u{e9}", 8, "x").is_err());
        assert_eq!(decode_hex("", "x").unwrap(), Vec::<u8>::new());
        assert_eq!(decode_hex("0aFF", "x").unwrap(), [0x0A, 0xFF]);
        assert!(decode_hex("0", "x").is_err());
        assert_eq!(
            canonical_guid("7c436110-ab2a-4bbb-a880-fe41995c9f82", "x").unwrap(),
            APPLE_BOOT_VARIABLE_GUID
        );
        assert!(canonical_guid("7C436110AB2A4BBBA880FE41995C9F82", "x").is_err());
        let mut s = String::new();
        escape_into(&mut s, "a<b>&c\u{1}d");
        assert_eq!(s, "a&lt;b&gt;&amp;cd");
    }
}
