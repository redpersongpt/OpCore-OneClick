//! The build itself: OpenCore → OcBinaryData → EFI layout → kexts → SSDTs →
//! AMD kernel patches → Kernel->Add → identity → config.plist → save →
//! ocvalidate. Everything up to "save" happens in a staging directory.

use std::path::{Path, PathBuf};

use crate::contracts::{ArtifactStatus, BuildResult, SsdtResult};
use crate::domain::amd_patches;
use crate::domain::config_writer::{self, ConfigInputs};
use crate::domain::kernel_add::{self, KernelAddEntry};
use crate::domain::kext_catalog;
use crate::domain::model::{
    BuildOptions, BuildPlan, CpuVendor, DeviceBus, HardwareProfile, PickerStyle, PlatformIdentity, ProfileNic,
    SsdtSource,
};
use crate::domain::smbios_gen;
use crate::error::AppError;
use crate::services::artifacts::{self, OpenCorePackage};
use crate::services::http::Downloader;
use crate::services::ocvalidate;
use crate::tasks::cancellation::CancellationToken;

use super::assemble::{self, DriverSources};
use super::kexts::KextStage;
use super::manifest::{self, BuildManifest};
use super::progress::{Phase, ProgressSink, Reporter};
use super::staging::{self, StagingDir};
use super::{amd, blocking, is_plain_file_name, retention, ssdt, while_doing};

/// Where and how to build. One build at a time per `builds_dir`: a new run
/// removes staging directories it finds there.
pub struct BuildEnv<'a> {
    /// `builds/`; the finished build is `builds/<build-id>`.
    pub builds_dir: &'a Path,
    /// Extraction scratch space for downloaded archives.
    pub work_dir: &'a Path,
    pub downloader: &'a Downloader,
    pub cancel: &'a CancellationToken,
    pub progress: &'a dyn ProgressSink,
    /// Builds kept afterwards, this one included (0 keeps everything).
    pub keep_builds: usize,
}

/// Build `plan` into a new build directory and validate it.
pub async fn run(
    env: &BuildEnv<'_>,
    profile: &HardwareProfile,
    options: &BuildOptions,
    plan: BuildPlan,
) -> Result<BuildResult, AppError> {
    check_plan(&plan, profile)?;
    let reporter = Reporter::new(env.progress);
    staging::clean_stale_staging(env.builds_dir);
    let build_id = staging::new_build_id();
    let staging = StagingDir::create(env.builds_dir, &build_id)?;
    tracing::info!(build = %build_id, target = plan.target.id(), smbios = %plan.smbios.model, "EFI build started");

    let mut job =
        Job { env, reporter: &reporter, profile, options, plan, efi: staging.path().join("EFI"), warnings: Vec::new() };
    let assembled = job.assemble().await?;
    env.cancel.check()?;

    reporter.phase(Phase::Save, "Saving the build");
    let build_dir = blocking(move || staging.commit()).await?;
    let efi = build_dir.join("EFI");

    reporter.phase(Phase::Validate, "Validating with ocvalidate");
    let validation = ocvalidate::validate_efi(&efi, assembled.package.ocvalidate().as_deref()).await;
    if !validation.valid {
        tracing::warn!(build = %build_id, issues = validation.issues.len(), "the new EFI has blocking validation issues");
    }

    let Job { plan, warnings, .. } = job;
    let result = BuildResult {
        build_id: build_id.clone(),
        efi_path: build_dir.to_string_lossy().into_owned(),
        config_plist_path: efi.join("OC").join("config.plist").to_string_lossy().into_owned(),
        target: plan.target,
        opencore_version: assembled.package.version.clone(),
        identity: assembled.identity,
        plan,
        kexts: assembled.kexts,
        ssdts: assembled.ssdts,
        validation,
        warnings,
    };
    let manifest = BuildManifest::new(result.clone(), assembled.package.debug, Some(assembled.package.root.clone()));
    if let Err(e) = manifest::write(&build_dir, &manifest) {
        tracing::warn!(error = %e, "could not write the build manifest");
    }
    if env.keep_builds > 0 {
        let removed = retention::prune_builds(env.builds_dir, env.keep_builds, Some(&build_dir));
        if !removed.is_empty() {
            tracing::info!(count = removed.len(), "old builds removed");
        }
    }
    tracing::info!(build = %build_id, valid = result.validation.valid, "EFI build finished");
    Ok(result)
}

/// The plan must name a model, and an AMD CPU on real hardware must get the
/// AMD_Vanilla kernel patches (without them macOS cannot boot at all, Dortania
/// AMD guide "Kernel -> Patch"). Everything else has failsafe defaults.
fn check_plan(plan: &BuildPlan, profile: &HardwareProfile) -> Result<(), AppError> {
    if plan.smbios.model.trim().is_empty() {
        return Err(AppError::new("PLAN_INCOMPLETE", "The build plan has no SMBIOS model"));
    }
    if plan.amd_core_count == Some(0) {
        return Err(AppError::new("PLAN_INCOMPLETE", "The build plan has an AMD core count of 0"));
    }
    if profile.cpu.vendor == CpuVendor::Amd && profile.vm.is_none() && plan.amd_core_count.is_none() {
        return Err(AppError::new("PLAN_INCOMPLETE", "The build plan has no core count for the AMD kernel patches")
            .recoverable()
            .with_suggestion("Enter the number of physical CPU cores in the hardware profile and plan again."));
    }
    Ok(())
}

struct Job<'a> {
    env: &'a BuildEnv<'a>,
    reporter: &'a Reporter<'a>,
    profile: &'a HardwareProfile,
    options: &'a BuildOptions,
    plan: BuildPlan,
    /// `EFI` inside the staging directory.
    efi: PathBuf,
    warnings: Vec<String>,
}

struct Assembled {
    package: OpenCorePackage,
    identity: PlatformIdentity,
    kexts: Vec<crate::contracts::KextResult>,
    ssdts: Vec<SsdtResult>,
}

impl Job<'_> {
    fn cancel(&self) -> &CancellationToken {
        self.env.cancel
    }

    async fn assemble(&mut self) -> Result<Assembled, AppError> {
        let package = self.fetch_opencore().await?;
        let ocbinarydata = self.fetch_ocbinarydata().await?;
        self.layout(&package, ocbinarydata).await?;
        let kexts = self.kexts().await?;
        let ssdts = self.ssdts(&package).await?;
        self.amd_patches().await?;
        let identity = self.config(&package).await?;
        Ok(Assembled { package, identity, kexts, ssdts })
    }

    async fn fetch_opencore(&mut self) -> Result<OpenCorePackage, AppError> {
        self.cancel().check()?;
        let flavour = if self.options.debug_opencore { "DEBUG" } else { "RELEASE" };
        let pin = kext_catalog::opencore_release();
        self.reporter.phase(Phase::OpenCore, format!("Getting OpenCore {} {flavour}", pin.version));
        let reporter = self.reporter;
        let progress =
            move |done: u64, total: Option<u64>| reporter.bytes(Phase::OpenCore, 0, 1, "OpenCore", done, total);
        let package = artifacts::fetch_opencore_with_progress(
            self.env.downloader,
            self.env.work_dir,
            self.options.debug_opencore,
            self.options.use_latest_releases,
            self.env.cancel,
            Some(&progress),
        )
        .await
        .map_err(|e| while_doing(e, "Could not get OpenCore"))?;
        tracing::info!(version = %package.version, debug = package.debug, status = ?package.status, "OpenCore ready");
        Ok(package)
    }

    /// OcBinaryData is needed for OpenCanopy's resources and for drivers
    /// such as HfsPlus.efi. When it cannot be downloaded the build continues
    /// with fallbacks (text picker, OpenHfsPlus.efi).
    async fn fetch_ocbinarydata(&mut self) -> Result<Option<PathBuf>, AppError> {
        self.cancel().check()?;
        let canopy = assemble::wants_canopy(&self.plan, self.options.picker);
        if !canopy && !assemble::wants_ocbinarydata_drivers(&self.plan) {
            return Ok(None);
        }
        self.reporter.phase(Phase::Resources, "Getting OcBinaryData (picker resources, HFS+ driver)");
        let reporter = self.reporter;
        let progress =
            move |done: u64, total: Option<u64>| reporter.bytes(Phase::Resources, 0, 1, "OcBinaryData", done, total);
        match artifacts::fetch_ocbinarydata_with_progress(
            self.env.downloader,
            self.env.work_dir,
            self.env.cancel,
            Some(&progress),
        )
        .await
        {
            Ok(root) => Ok(Some(root)),
            Err(e) if e.code == "TASK_CANCELLED" => Err(e),
            Err(e) => {
                tracing::warn!(error = %e, "OcBinaryData unavailable");
                self.warnings.push(format!("OcBinaryData could not be downloaded ({})", e.message));
                Ok(None)
            }
        }
    }

    /// Boot loaders, planned drivers and tools, picker resources.
    async fn layout(&mut self, package: &OpenCorePackage, ocbinarydata: Option<PathBuf>) -> Result<(), AppError> {
        self.cancel().check()?;
        self.reporter.phase(Phase::Assemble, "Assembling the EFI folder");
        let canopy = assemble::wants_canopy(&self.plan, self.options.picker);
        let audio = assemble::wants_audio_assist(&self.plan);
        let package_efi = package.x64_efi();
        let efi = self.efi.clone();
        let mut drivers = std::mem::take(&mut self.plan.drivers);
        if self.options.picker == PickerStyle::Text {
            // The text picker never loads OpenCanopy; do not ship it.
            drivers.retain(|d| !d.path.eq_ignore_ascii_case("OpenCanopy.efi"));
        }
        let mut tools = std::mem::take(&mut self.plan.tools);
        let (drivers, tools, resources_ok, warnings) = blocking(move || {
            let mut warnings = Vec::new();
            assemble::copy_core(&package_efi, &efi)?;
            let oc = efi.join("OC");
            let mut resources_ok = true;
            if canopy {
                resources_ok = match ocbinarydata.as_deref() {
                    Some(root) => match assemble::install_resources(root, &oc.join("Resources"), audio) {
                        Ok(()) => true,
                        Err(e) => {
                            warnings.push(format!("OpenCanopy resources could not be installed ({})", e.message));
                            false
                        }
                    },
                    None => false,
                };
                if !resources_ok {
                    drivers.retain(|d| !d.path.eq_ignore_ascii_case("OpenCanopy.efi"));
                }
            }
            let bin_drivers = ocbinarydata.as_ref().map(|root| root.join("Drivers"));
            let sources = DriverSources {
                opencore: &package_efi.join("OC").join("Drivers"),
                ocbinarydata: bin_drivers.as_deref(),
            };
            assemble::install_drivers(&mut drivers, &sources, &oc.join("Drivers"), &mut warnings)?;
            assemble::install_tools(
                &mut tools,
                &package_efi.join("OC").join("Tools"),
                &oc.join("Tools"),
                &mut warnings,
            )?;
            Ok((drivers, tools, resources_ok, warnings))
        })
        .await?;
        self.plan.drivers = drivers;
        self.plan.tools = tools;
        self.warnings.extend(warnings);
        if canopy && !resources_ok {
            assemble::use_text_picker(&mut self.plan);
            self.warnings.push("The graphical boot picker needs OcBinaryData; the text picker is used instead".into());
        }
        Ok(())
    }

    /// Download every catalog archive once and install the selected bundles.
    async fn kexts(&mut self) -> Result<Vec<crate::contracts::KextResult>, AppError> {
        let kexts_dir = self.efi.join("OC").join("Kexts");
        let mut stage = KextStage::new(std::mem::take(&mut self.plan.kexts));
        let groups = stage.groups();
        let count = groups.len();
        self.reporter.phase(Phase::Kexts, format!("Getting {count} kext packages"));
        for (index, group) in groups.into_iter().enumerate() {
            self.cancel().check()?;
            let id = group.catalog_id.clone();
            self.reporter.item(Phase::Kexts, index, count, &id, format!("Getting {id}"));
            let fetched = match kext_catalog::entry(&id) {
                None => Err(AppError::new("KEXT_UNKNOWN", format!("{id} is not in the kext catalog"))),
                Some(entry) => {
                    let reporter = self.reporter;
                    let item = id.clone();
                    let progress = move |done: u64, total: Option<u64>| {
                        reporter.bytes(Phase::Kexts, index, count, &item, done, total)
                    };
                    artifacts::fetch_kext_with_progress(
                        self.env.downloader,
                        entry,
                        self.env.work_dir,
                        self.options.use_latest_releases,
                        self.env.cancel,
                        Some(&progress),
                    )
                    .await
                }
            };
            let dir = kexts_dir.clone();
            stage = blocking(move || {
                match fetched {
                    Ok(fetched) => stage.install(&group, &fetched, &dir)?,
                    Err(e) => stage.archive_failed(&group, e)?,
                }
                Ok(stage)
            })
            .await?;
        }
        let dir = kexts_dir.clone();
        let stage = blocking(move || {
            stage.drop_unmet_dependencies(&dir);
            Ok(stage)
        })
        .await?;
        let (kept, results, warnings) = stage.finish();
        self.plan.kexts = kept;
        self.warnings.extend(warnings);
        Ok(results)
    }

    /// Write every planned SSDT into EFI/OC/ACPI.
    async fn ssdts(&mut self, package: &OpenCorePackage) -> Result<Vec<SsdtResult>, AppError> {
        let acpi_dir = self.efi.join("OC").join("ACPI");
        let samples = package.acpi_samples();
        let planned = std::mem::take(&mut self.plan.ssdts);
        let count = planned.len();
        self.reporter.phase(Phase::Acpi, format!("Writing {count} ACPI tables"));
        let mut kept = Vec::new();
        let mut results = Vec::new();
        for (index, table) in planned.into_iter().enumerate() {
            self.cancel().check()?;
            self.reporter.item(Phase::Acpi, index, count, &table.file_name, format!("Adding {}", table.file_name));
            let found = if is_plain_file_name(&table.file_name, ".aml") {
                self.ssdt_bytes(&table.source, &samples).await
            } else {
                Err(AppError::new("SSDT_INVALID", format!("'{}' is not an ACPI table file name", table.file_name)))
            };
            let (bytes, status) = match found {
                Ok(found) => found,
                Err(e) if e.code == "TASK_CANCELLED" => return Err(e),
                Err(e) if table.required => {
                    return Err(while_doing(e, &format!("Could not provide {} (required)", table.file_name)));
                }
                Err(e) => {
                    let disabled = ssdt::disable_dependent_patches(&mut self.plan.acpi_patches, &table.file_name);
                    let mut warning = format!("{} was left out: {}", table.file_name, e.message);
                    if disabled > 0 {
                        warning.push_str(&format!(" ({disabled} ACPI patch(es) that need it were disabled)"));
                    }
                    self.warnings.push(warning);
                    results.push(SsdtResult {
                        file_name: table.file_name.clone(),
                        source: ssdt::source_id(&table.source).into(),
                        status: ArtifactStatus::Skipped,
                        reason: table.reason.clone(),
                    });
                    continue;
                }
            };
            let path = acpi_dir.join(&table.file_name);
            blocking(move || {
                std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
                std::fs::write(&path, bytes).map_err(AppError::from)
            })
            .await?;
            results.push(SsdtResult {
                file_name: table.file_name.clone(),
                source: ssdt::source_id(&table.source).into(),
                status,
                reason: table.reason.clone(),
            });
            kept.push(table);
        }
        self.plan.ssdts = kept;
        Ok(results)
    }

    async fn ssdt_bytes(&self, source: &SsdtSource, samples: &Path) -> Result<(Vec<u8>, ArtifactStatus), AppError> {
        match source {
            SsdtSource::Generated { aml_hex, .. } => Ok((ssdt::generated_bytes(aml_hex)?, ArtifactStatus::Generated)),
            SsdtSource::OcSample { file } => {
                let (samples, file) = (samples.to_path_buf(), file.clone());
                Ok((blocking(move || ssdt::oc_sample_bytes(&samples, &file)).await?, ArtifactStatus::Bundled))
            }
            SsdtSource::Dortania { file } => {
                let cached = kext_catalog::dortania_ssdt(file)
                    .and_then(|pin| pin.sha256)
                    .is_some_and(|sha| self.env.downloader.is_cached(sha));
                let bytes = artifacts::fetch_dortania_ssdt(self.env.downloader, file, self.env.cancel).await?;
                Ok((bytes, if cached { ArtifactStatus::Cached } else { ArtifactStatus::Downloaded }))
            }
        }
    }

    /// AMD_Vanilla patches for the target, with this CPU's core count.
    async fn amd_patches(&mut self) -> Result<(), AppError> {
        let Some(cores) = self.plan.amd_core_count else { return Ok(()) };
        self.cancel().check()?;
        let pin = amd_patches::pin_for_target(self.plan.target);
        self.reporter.phase(Phase::KernelPatches, format!("Getting AMD kernel patches ({})", pin.version));
        let reporter = self.reporter;
        let progress =
            move |done: u64, total: Option<u64>| reporter.bytes(Phase::KernelPatches, 0, 1, "AMD_Vanilla", done, total);
        let bytes = self
            .env
            .downloader
            .fetch_bytes(pin.url, pin.sha256, self.env.cancel, Some(&progress))
            .await
            .map_err(|e| while_doing(e, "Could not get the AMD kernel patches"))?;
        let (patches, notes) = amd::prepare(&bytes, cores, self.profile)?;
        tracing::info!(count = patches.len(), cores, "AMD_Vanilla patches added");
        self.plan.kernel_patches.extend(patches);
        self.plan.post_install.extend(notes);
        Ok(())
    }

    /// Kernel->Add from the bundles on disk, the SMBIOS identity, config.plist.
    async fn config(&mut self, package: &OpenCorePackage) -> Result<PlatformIdentity, AppError> {
        self.cancel().check()?;
        self.reporter.phase(Phase::Config, "Generating config.plist");
        let oc = self.efi.join("OC");
        let kexts = self.plan.kexts.clone();
        let kexts_dir = oc.join("Kexts");
        let entries: Vec<KernelAddEntry> = blocking(move || kernel_add::build_kernel_add(&kexts, &kexts_dir)).await?;

        let identity = self.identity(package).await?;

        let ssdt_files: Vec<String> = self.plan.ssdts.iter().map(|s| s.file_name.clone()).collect();
        let driver_files = file_names(&oc.join("Drivers"));
        let tool_files = file_names(&oc.join("Tools"));
        let sample = std::fs::read(package.sample_plist()).map_err(|e| {
            AppError::new("SAMPLE_PLIST_MISSING", format!("OpenCore's Sample.plist cannot be read: {e}"))
        })?;
        let config = config_writer::write_config(
            &sample,
            &ConfigInputs {
                plan: &self.plan,
                kernel_add: &entries,
                identity: &identity,
                ssdt_files: &ssdt_files,
                driver_files: &driver_files,
                tool_files: &tool_files,
            },
        )
        .map_err(|e| {
            // A newer OpenCore can drop or retype keys the plan sets.
            if package.version == kext_catalog::opencore_release().version || e.suggestion.is_some() {
                e
            } else {
                e.with_suggestion(format!(
                    "OpenCore {} changed its config.plist format. Turn off \"use latest releases\" to build with the tested OpenCore {}.",
                    package.version,
                    kext_catalog::opencore_release().version
                ))
            }
        })?;
        let path = oc.join("config.plist");
        blocking(move || std::fs::write(&path, config).map_err(AppError::from)).await?;
        Ok(identity)
    }

    /// The identity from the options when it belongs to the planned model
    /// (keeps iServices stable across rebuilds), else a fresh one.
    async fn identity(&mut self, package: &OpenCorePackage) -> Result<PlatformIdentity, AppError> {
        let model = self.plan.smbios.model.clone();
        if let Some(previous) = self.options.identity.as_ref() {
            if previous.model != model {
                self.warnings.push(format!(
                    "The saved serial numbers belong to {} but this build uses {model}; new ones were generated",
                    previous.model
                ));
            } else if identity_usable(previous) {
                return Ok(previous.clone());
            } else {
                self.warnings
                    .push(format!("The saved serial numbers for {model} are damaged; new ones were generated"));
            }
        }
        let macserial = package.macserial();
        let mac = primary_mac(self.profile);
        blocking(move || smbios_gen::generate_identity(&model, macserial.as_deref(), mac.as_deref())).await
    }
}

/// MAC for PlatformInfo ROM: a built-in NIC's burned-in address, so it stays
/// stable (Dortania "Fixing iServices": use the MAC address of the network
/// card). Order: PCI Ethernet, PCI Wi-Fi, then USB adapters. Addresses that
/// cannot be a ROM (zero, broadcast, multicast) are skipped, and locally
/// administered ones (randomised Wi-Fi MACs, virtual adapters) are used only
/// when nothing else is left.
pub fn primary_mac(profile: &HardwareProfile) -> Option<String> {
    let built_in = |n: &&ProfileNic| n.bus == DeviceBus::Pci;
    let candidates: Vec<&str> = profile
        .ethernet
        .iter()
        .filter(built_in)
        .chain(profile.wifi.iter().filter(built_in))
        .chain(profile.ethernet.iter().filter(|n| !built_in(n)))
        .chain(profile.wifi.iter().filter(|n| !built_in(n)))
        .filter_map(|n| n.mac_address.as_deref().map(str::trim))
        .filter(|mac| smbios_gen::rom_from_mac(mac).is_some())
        .collect();
    let universal = |mac: &&&str| smbios_gen::rom_from_mac(mac).is_some_and(|rom| !locally_administered(&rom));
    candidates.iter().find(universal).or_else(|| candidates.first()).map(|m| m.to_string())
}

/// Bit 1 of the first octet: set for addresses not assigned by a vendor.
fn locally_administered(rom_hex: &str) -> bool {
    rom_hex.get(..2).and_then(|b| u8::from_str_radix(b, 16).ok()).is_some_and(|b| b & 0x02 != 0)
}

/// A saved identity config.plist will accept: alphanumeric serial and MLB,
/// a UUID, and a 6-byte ROM.
fn identity_usable(identity: &PlatformIdentity) -> bool {
    let alnum = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    alnum(&identity.serial)
        && alnum(&identity.mlb)
        && uuid::Uuid::parse_str(&identity.system_uuid).is_ok()
        && identity.rom.len() == 12
        && identity.rom.chars().all(|c| c.is_ascii_hexdigit())
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{MacOsVersion, ProfileNic};
    use crate::domain::planner::empty_plan;

    fn nic(bus: DeviceBus, mac: Option<&str>) -> ProfileNic {
        ProfileNic { bus, mac_address: mac.map(str::to_string), ..ProfileNic::default() }
    }

    #[test]
    fn primary_mac_prefers_built_in_ethernet() {
        let mut profile = HardwareProfile {
            ethernet: vec![
                nic(DeviceBus::Usb, Some("00:E0:4C:68:00:01")),
                nic(DeviceBus::Pci, Some("A4:BB:CC:00:11:22")),
            ],
            wifi: Some(nic(DeviceBus::Pci, Some("A4:BB:CC:00:11:33"))),
            ..HardwareProfile::default()
        };
        assert_eq!(primary_mac(&profile).as_deref(), Some("A4:BB:CC:00:11:22"));
        // An empty or unusable built-in address falls through to the next NIC.
        profile.ethernet[1].mac_address = Some("  ".into());
        assert_eq!(primary_mac(&profile).as_deref(), Some("A4:BB:CC:00:11:33"));
        profile.ethernet[1].mac_address = Some("00:00:00:00:00:00".into());
        assert_eq!(primary_mac(&profile).as_deref(), Some("A4:BB:CC:00:11:33"));
        profile.wifi = None;
        assert_eq!(primary_mac(&profile).as_deref(), Some("00:E0:4C:68:00:01"), "USB adapter as a last resort");
        profile.ethernet.clear();
        assert_eq!(primary_mac(&profile), None);
    }

    #[test]
    fn randomised_macs_are_a_last_resort() {
        let profile = HardwareProfile {
            ethernet: vec![nic(DeviceBus::Usb, Some("00:E0:4C:68:00:01"))],
            wifi: Some(nic(DeviceBus::Pci, Some("DA:A1:19:00:11:33"))),
            ..HardwareProfile::default()
        };
        assert_eq!(primary_mac(&profile).as_deref(), Some("00:E0:4C:68:00:01"));
        let only_random = HardwareProfile { ethernet: vec![], ..profile };
        assert_eq!(primary_mac(&only_random).as_deref(), Some("DA:A1:19:00:11:33"));
    }

    #[test]
    fn plans_without_a_model_are_refused() {
        let intel = HardwareProfile::default();
        let plan = empty_plan(MacOsVersion::Sequoia);
        assert_eq!(check_plan(&plan, &intel).unwrap_err().code, "PLAN_INCOMPLETE");
        let mut plan = empty_plan(MacOsVersion::Sequoia);
        plan.smbios.model = "iMac19,1".into();
        assert!(check_plan(&plan, &intel).is_ok());
        plan.amd_core_count = Some(0);
        assert!(check_plan(&plan, &intel).is_err());
    }

    #[test]
    fn amd_builds_need_the_core_count() {
        let mut amd = HardwareProfile::default();
        amd.cpu.vendor = CpuVendor::Amd;
        let mut plan = empty_plan(MacOsVersion::Sonoma);
        plan.smbios.model = "MacPro7,1".into();
        let err = check_plan(&plan, &amd).unwrap_err();
        assert_eq!(err.code, "PLAN_INCOMPLETE");
        assert!(err.suggestion.is_some());
        plan.amd_core_count = Some(8);
        assert!(check_plan(&plan, &amd).is_ok());
        // A VM may present an Intel CPU model and needs no AMD patches.
        plan.amd_core_count = None;
        amd.vm = Some(crate::domain::model::VmKind::Kvm);
        assert!(check_plan(&plan, &amd).is_ok());
    }

    #[test]
    fn saved_identities_are_checked_before_reuse() {
        let good = PlatformIdentity {
            model: "iMac19,1".into(),
            serial: "C02XG0FDJV3Q".into(),
            mlb: "C02923600GUJV3QAD".into(),
            system_uuid: "8F5B8B1E-2E0B-4E43-9E5A-6F0B3C2D1A00".into(),
            rom: "A483E7123456".into(),
        };
        assert!(identity_usable(&good));
        for broken in [
            PlatformIdentity { serial: String::new(), ..good.clone() },
            PlatformIdentity { mlb: "C029-2360".into(), ..good.clone() },
            PlatformIdentity { system_uuid: "nope".into(), ..good.clone() },
            PlatformIdentity { rom: "A483E712".into(), ..good.clone() },
            PlatformIdentity { rom: "A483E712345G".into(), ..good.clone() },
        ] {
            assert!(!identity_usable(&broken), "{broken:?}");
        }
    }
}
