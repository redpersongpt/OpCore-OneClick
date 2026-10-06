//! EFI folder assembly from the OpenCore package and OcBinaryData: the boot
//! loaders, only the drivers and tools the plan asks for, and the OpenCanopy
//! resources. Missing optional pieces are dropped from the plan with a
//! warning so config.plist never references a file that is not there.

use std::path::Path;

use crate::domain::model::{BuildPlan, DriverPlan, PickerStyle, PlistScalar};
use crate::error::AppError;

use super::is_plain_file_name;
use super::staging::{copy_file, copy_tree, find_ci};

pub const SOURCE_OPENCORE: &str = "opencore";
pub const SOURCE_OCBINARYDATA: &str = "ocbinarydata";

const OPEN_CANOPY: &str = "OpenCanopy.efi";
const OPEN_RUNTIME: &str = "OpenRuntime.efi";
const OPEN_VARIABLE_RUNTIME: &str = "OpenVariableRuntimeDxe.efi";
const OPEN_HFS_PLUS: &str = "OpenHfsPlus.efi";

/// Drivers without which the EFI cannot boot an installer: the runtime
/// services driver, emulated NVRAM when planned, and an HFS+ driver for the
/// recovery image.
fn is_essential_driver(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == OPEN_RUNTIME.to_ascii_lowercase()
        || lower == OPEN_VARIABLE_RUNTIME.to_ascii_lowercase()
        || is_hfs_driver(&lower)
}

fn is_hfs_driver(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("hfsplus") || lower == OPEN_HFS_PLUS.to_ascii_lowercase()
}

fn is_named(driver: &DriverPlan, name: &str) -> bool {
    driver.path.eq_ignore_ascii_case(name)
}

/// The graphical picker is planned: OpenCanopy is an enabled driver and the
/// user asked for it.
pub fn wants_canopy(plan: &BuildPlan, picker: PickerStyle) -> bool {
    picker == PickerStyle::Graphical && plan.drivers.iter().any(|d| d.enabled && is_named(d, OPEN_CANOPY))
}

/// Some planned driver comes from OcBinaryData (HfsPlus.efi, ExFatDxe.efi...).
pub fn wants_ocbinarydata_drivers(plan: &BuildPlan) -> bool {
    plan.drivers.iter().any(|d| d.source.eq_ignore_ascii_case(SOURCE_OCBINARYDATA))
}

/// Use OpenCore's built-in text picker instead of OpenCanopy.
pub fn use_text_picker(plan: &mut BuildPlan) {
    plan.drivers.retain(|d| !is_named(d, OPEN_CANOPY));
    plan.misc_boot.insert("PickerMode".into(), PlistScalar::Str("Builtin".into()));
}

/// BOOT/BOOTx64.efi, OC/OpenCore.efi (with their `.contentFlavour` and
/// `.contentVisibility` markers) and the empty OC folder structure.
pub fn copy_core(package_efi: &Path, efi: &Path) -> Result<(), AppError> {
    copy_file(&package_efi.join("BOOT").join("BOOTx64.efi"), &efi.join("BOOT").join("BOOTx64.efi"))?;
    copy_file(&package_efi.join("OC").join("OpenCore.efi"), &efi.join("OC").join("OpenCore.efi"))?;
    for dir in ["BOOT", "OC"] {
        for marker in [".contentFlavour", ".contentVisibility"] {
            let src = package_efi.join(dir).join(marker);
            if src.is_file() {
                copy_file(&src, &efi.join(dir).join(marker))?;
            }
        }
    }
    for dir in ["ACPI", "Drivers", "Kexts", "Tools", "Resources"] {
        std::fs::create_dir_all(efi.join("OC").join(dir))?;
    }
    Ok(())
}

/// Where planned drivers can be copied from.
pub struct DriverSources<'a> {
    /// `X64/EFI/OC/Drivers` of the OpenCore package.
    pub opencore: &'a Path,
    /// `Drivers` of OcBinaryData, when it could be downloaded.
    pub ocbinarydata: Option<&'a Path>,
}

/// Copy the planned drivers into `dest` (EFI/OC/Drivers). A missing
/// HfsPlus.efi is replaced by OpenCore's OpenHfsPlus.efi; other missing
/// drivers fail the build when essential and are dropped otherwise. Returns
/// the driver file names now in `dest`.
pub fn install_drivers(
    drivers: &mut Vec<DriverPlan>,
    sources: &DriverSources,
    dest: &Path,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, AppError> {
    std::fs::create_dir_all(dest)?;
    let mut kept: Vec<DriverPlan> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for mut driver in std::mem::take(drivers) {
        if kept.iter().any(|d| d.path.eq_ignore_ascii_case(&driver.path)) {
            continue;
        }
        if !is_plain_file_name(&driver.path, ".efi") {
            if driver.enabled {
                return Err(AppError::new("DRIVER_INVALID", format!("'{}' is not a driver file name", driver.path)));
            }
            continue;
        }
        let from_ocbin = driver.source.eq_ignore_ascii_case(SOURCE_OCBINARYDATA);
        let order: [Option<&Path>; 2] = if from_ocbin {
            [sources.ocbinarydata, Some(sources.opencore)]
        } else {
            [Some(sources.opencore), sources.ocbinarydata]
        };
        let found = order.into_iter().flatten().find_map(|dir| find_ci(dir, &driver.path).filter(|p| p.is_file()));
        let src = match found {
            Some(src) => src,
            None if is_hfs_driver(&driver.path) => {
                // OpenHfsPlus is the open-source HFS+ driver shipped with OpenCore.
                let Some(fallback) = find_ci(sources.opencore, OPEN_HFS_PLUS).filter(|p| p.is_file()) else {
                    return Err(AppError::new(
                        "DRIVER_MISSING",
                        format!("Neither {} nor {OPEN_HFS_PLUS} is available", driver.path),
                    ));
                };
                if kept.iter().any(|d| is_named(d, OPEN_HFS_PLUS)) {
                    continue;
                }
                warnings.push(format!(
                    "{} (OcBinaryData) is not available; using OpenCore's {OPEN_HFS_PLUS} instead (slower, same function)",
                    driver.path
                ));
                driver.path = OPEN_HFS_PLUS.to_string();
                driver.source = SOURCE_OPENCORE.to_string();
                fallback
            }
            None if driver.enabled && is_essential_driver(&driver.path) => {
                return Err(AppError::new(
                    "DRIVER_MISSING",
                    format!("{} is not in the OpenCore package or OcBinaryData", driver.path),
                ));
            }
            None => {
                if driver.enabled {
                    warnings.push(format!("Driver {} is not available and was left out", driver.path));
                }
                continue;
            }
        };
        // Use the package's spelling: ocvalidate compares driver names exactly.
        if let Some(name) = src.file_name().and_then(|n| n.to_str()) {
            driver.path = name.to_string();
        }
        copy_file(&src, &dest.join(&driver.path))?;
        files.push(driver.path.clone());
        kept.push(driver);
    }
    normalize_load_early(&mut kept);
    *drivers = kept;
    Ok(files)
}

/// The LoadEarly rules ocvalidate enforces: OpenVariableRuntimeDxe.efi loads
/// early and before OpenRuntime.efi; OpenRuntime.efi loads early only together
/// with it; no other driver loads early (Configuration.tex, "OpenVariableRuntimeDxe").
pub fn normalize_load_early(drivers: &mut Vec<DriverPlan>) {
    let emulated_nvram = drivers.iter().any(|d| d.enabled && d.path == OPEN_VARIABLE_RUNTIME);
    for d in drivers.iter_mut() {
        d.load_early = if d.path == OPEN_VARIABLE_RUNTIME {
            true
        } else if d.path == OPEN_RUNTIME {
            emulated_nvram
        } else {
            false
        };
    }
    let variable = drivers.iter().position(|d| d.enabled && d.path == OPEN_VARIABLE_RUNTIME);
    let runtime = drivers.iter().position(|d| d.enabled && d.path == OPEN_RUNTIME);
    if let (Some(v), Some(r)) = (variable, runtime) {
        if v > r {
            let entry = drivers.remove(v);
            drivers.insert(r, entry);
        }
    }
}

/// Copy the planned tools from the OpenCore package; unknown tools are
/// dropped with a warning. Returns the tool file names now in `dest`.
pub fn install_tools(
    tools: &mut Vec<String>,
    package_tools: &Path,
    dest: &Path,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, AppError> {
    std::fs::create_dir_all(dest)?;
    let mut kept: Vec<String> = Vec::new();
    for tool in std::mem::take(tools) {
        if kept.iter().any(|t| t.eq_ignore_ascii_case(&tool)) {
            continue;
        }
        let src =
            is_plain_file_name(&tool, ".efi").then(|| find_ci(package_tools, &tool)).flatten().filter(|p| p.is_file());
        match src {
            Some(src) => {
                let name = src.file_name().and_then(|n| n.to_str()).map_or(tool, str::to_string);
                copy_file(&src, &dest.join(&name))?;
                kept.push(name);
            }
            None => warnings.push(format!("Tool {tool} is not in the OpenCore package and was left out")),
        }
    }
    *tools = kept.clone();
    Ok(kept)
}

/// OpenCanopy resources from OcBinaryData (`Resources/{Font,Image,Label}`;
/// `Audio` only for the audio-assisted picker, it is large).
pub fn install_resources(ocbinarydata_root: &Path, dest: &Path, with_audio: bool) -> Result<(), AppError> {
    let src = ocbinarydata_root.join("Resources");
    let mut folders = vec!["Font", "Image", "Label"];
    if with_audio {
        folders.push("Audio");
    }
    for folder in folders {
        let from = src.join(folder);
        if !from.is_dir() {
            return Err(AppError::new("RESOURCES_MISSING", format!("OcBinaryData has no Resources/{folder}")));
        }
        if copy_tree(&from, &dest.join(folder))? == 0 {
            return Err(AppError::new("RESOURCES_MISSING", format!("OcBinaryData Resources/{folder} is empty")));
        }
    }
    Ok(())
}

/// The picker should speak (needs `Resources/Audio`).
pub fn wants_audio_assist(plan: &BuildPlan) -> bool {
    matches!(plan.misc_boot.get("PickerAudioAssist"), Some(PlistScalar::Bool(true)))
}

#[cfg(test)]
mod tests {
    use super::super::staging::test_dir::TempDir;
    use super::*;
    use crate::domain::model::MacOsVersion;
    use crate::domain::planner::empty_plan;

    fn driver(path: &str, source: &str) -> DriverPlan {
        DriverPlan {
            path: path.into(),
            load_early: false,
            enabled: true,
            comment: String::new(),
            source: source.into(),
        }
    }

    fn files(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), b"MZ").unwrap();
        }
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn only_planned_drivers_are_copied() {
        let tmp = TempDir::new("drivers");
        let oc = tmp.path().join("oc");
        let bin = tmp.path().join("bin");
        files(&oc, &["OpenRuntime.efi", "OpenCanopy.efi", "OpenHfsPlus.efi", "ResetNvramEntry.efi", "Ext4Dxe.efi"]);
        files(&bin, &["HfsPlus.efi", "ExFatDxe.efi"]);
        let dest = tmp.path().join("EFI/OC/Drivers");
        let mut drivers = vec![
            driver("HfsPlus.efi", SOURCE_OCBINARYDATA),
            driver("OpenRuntime.efi", SOURCE_OPENCORE),
            driver("OpenCanopy.efi", SOURCE_OPENCORE),
            driver("openruntime.efi", SOURCE_OPENCORE),
            driver("ResetNvramEntry.efi", SOURCE_OPENCORE),
            driver("ToggleSipEntry.efi", SOURCE_OPENCORE),
        ];
        let mut warnings = Vec::new();
        let sources = DriverSources { opencore: &oc, ocbinarydata: Some(&bin) };
        let copied = install_drivers(&mut drivers, &sources, &dest, &mut warnings).unwrap();
        assert_eq!(copied, ["HfsPlus.efi", "OpenRuntime.efi", "OpenCanopy.efi", "ResetNvramEntry.efi"]);
        assert_eq!(listing(&dest), ["HfsPlus.efi", "OpenCanopy.efi", "OpenRuntime.efi", "ResetNvramEntry.efi"]);
        assert_eq!(drivers.len(), 4);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("ToggleSipEntry"));
    }

    #[test]
    fn hfs_falls_back_to_openhfsplus() {
        let tmp = TempDir::new("drivers");
        let oc = tmp.path().join("oc");
        files(&oc, &["OpenRuntime.efi", "OpenHfsPlus.efi"]);
        let dest = tmp.path().join("Drivers");
        let mut drivers = vec![driver("HfsPlus.efi", SOURCE_OCBINARYDATA), driver("OpenRuntime.efi", SOURCE_OPENCORE)];
        let mut warnings = Vec::new();
        let sources = DriverSources { opencore: &oc, ocbinarydata: None };
        let copied = install_drivers(&mut drivers, &sources, &dest, &mut warnings).unwrap();
        assert_eq!(copied, ["OpenHfsPlus.efi", "OpenRuntime.efi"]);
        assert_eq!(drivers[0].path, "OpenHfsPlus.efi");
        assert_eq!(drivers[0].source, SOURCE_OPENCORE);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn missing_essential_driver_fails() {
        let tmp = TempDir::new("drivers");
        let oc = tmp.path().join("oc");
        files(&oc, &["OpenHfsPlus.efi"]);
        let mut drivers = vec![driver("OpenRuntime.efi", SOURCE_OPENCORE)];
        let sources = DriverSources { opencore: &oc, ocbinarydata: None };
        let err = install_drivers(&mut drivers, &sources, &tmp.path().join("d"), &mut Vec::new()).unwrap_err();
        assert_eq!(err.code, "DRIVER_MISSING");

        let mut drivers = vec![driver("../OpenRuntime.efi", SOURCE_OPENCORE)];
        let err = install_drivers(&mut drivers, &sources, &tmp.path().join("d"), &mut Vec::new()).unwrap_err();
        assert_eq!(err.code, "DRIVER_INVALID");

        // A disabled essential driver that is missing is just dropped.
        let mut drivers = vec![DriverPlan { enabled: false, ..driver("OpenVariableRuntimeDxe.efi", SOURCE_OPENCORE) }];
        let mut warnings = Vec::new();
        assert!(install_drivers(&mut drivers, &sources, &tmp.path().join("d"), &mut warnings).unwrap().is_empty());
        assert!(drivers.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_early_rules() {
        let mut drivers = vec![
            driver("OpenRuntime.efi", SOURCE_OPENCORE),
            driver("HfsPlus.efi", SOURCE_OCBINARYDATA),
            DriverPlan { load_early: true, ..driver("OpenCanopy.efi", SOURCE_OPENCORE) },
            driver("OpenVariableRuntimeDxe.efi", SOURCE_OPENCORE),
        ];
        normalize_load_early(&mut drivers);
        let order: Vec<(&str, bool)> = drivers.iter().map(|d| (d.path.as_str(), d.load_early)).collect();
        assert_eq!(
            order,
            [
                ("OpenVariableRuntimeDxe.efi", true),
                ("OpenRuntime.efi", true),
                ("HfsPlus.efi", false),
                ("OpenCanopy.efi", false)
            ]
        );

        let mut drivers = vec![DriverPlan { load_early: true, ..driver("OpenRuntime.efi", SOURCE_OPENCORE) }];
        normalize_load_early(&mut drivers);
        assert!(!drivers[0].load_early, "OpenRuntime loads early only with emulated NVRAM");
    }

    #[test]
    fn tools_are_pruned_to_the_plan() {
        let tmp = TempDir::new("tools");
        let pkg = tmp.path().join("Tools");
        files(&pkg, &["OpenShell.efi", "ResetSystem.efi", "CleanNvram.efi"]);
        let dest = tmp.path().join("EFI/OC/Tools");
        let mut tools = vec!["OpenShell.efi".to_string(), "Missing.efi".to_string(), "openshell.efi".to_string()];
        let mut warnings = Vec::new();
        let copied = install_tools(&mut tools, &pkg, &dest, &mut warnings).unwrap();
        assert_eq!(copied, ["OpenShell.efi"]);
        assert_eq!(tools, ["OpenShell.efi"]);
        assert_eq!(listing(&dest), ["OpenShell.efi"]);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn core_files_and_markers() {
        let tmp = TempDir::new("core");
        let pkg = tmp.path().join("X64/EFI");
        files(&pkg.join("BOOT"), &["BOOTx64.efi", ".contentFlavour", ".contentVisibility"]);
        files(&pkg.join("OC"), &["OpenCore.efi", ".contentFlavour"]);
        files(&pkg.join("OC/Drivers"), &["OpenRuntime.efi"]);
        let efi = tmp.path().join("EFI");
        copy_core(&pkg, &efi).unwrap();
        assert_eq!(listing(&efi.join("BOOT")), [".contentFlavour", ".contentVisibility", "BOOTx64.efi"]);
        assert_eq!(
            listing(&efi.join("OC")),
            [".contentFlavour", "ACPI", "Drivers", "Kexts", "OpenCore.efi", "Resources", "Tools"]
        );
        assert!(listing(&efi.join("OC/Drivers")).is_empty());

        std::fs::remove_file(pkg.join("OC/OpenCore.efi")).unwrap();
        assert!(copy_core(&pkg, &tmp.path().join("EFI2")).is_err());
    }

    #[test]
    fn picker_decisions() {
        let mut plan = empty_plan(MacOsVersion::Sequoia);
        plan.drivers = vec![driver("OpenRuntime.efi", SOURCE_OPENCORE), driver("OpenCanopy.efi", SOURCE_OPENCORE)];
        assert!(wants_canopy(&plan, PickerStyle::Graphical));
        assert!(!wants_canopy(&plan, PickerStyle::Text));
        assert!(!wants_ocbinarydata_drivers(&plan));
        use_text_picker(&mut plan);
        assert!(!wants_canopy(&plan, PickerStyle::Graphical));
        assert_eq!(plan.misc_boot.get("PickerMode"), Some(&PlistScalar::Str("Builtin".into())));
        plan.drivers.push(driver("HfsPlus.efi", SOURCE_OCBINARYDATA));
        assert!(wants_ocbinarydata_drivers(&plan));
    }

    #[test]
    fn resources_copy_needs_every_folder() {
        let tmp = TempDir::new("res");
        let root = tmp.path().join("OcBinaryData");
        files(&root.join("Resources/Font"), &["Terminus.hex"]);
        files(&root.join("Resources/Image/Acidanthera/GoldenGate"), &["Background.icns"]);
        files(&root.join("Resources/Label"), &["Apple.lbl"]);
        files(&root.join("Resources/Audio"), &["OCEFIAudio_VoiceOver_Boot.mp3"]);
        let dest = tmp.path().join("EFI/OC/Resources");
        install_resources(&root, &dest, false).unwrap();
        assert!(dest.join("Image/Acidanthera/GoldenGate/Background.icns").is_file());
        assert!(!dest.join("Audio").exists());
        install_resources(&root, &dest, true).unwrap();
        assert!(dest.join("Audio/OCEFIAudio_VoiceOver_Boot.mp3").is_file());

        std::fs::remove_dir_all(root.join("Resources/Label")).unwrap();
        assert_eq!(install_resources(&root, &tmp.path().join("x"), false).unwrap_err().code, "RESOURCES_MISSING");
    }
}
