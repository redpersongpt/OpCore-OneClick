//! Opt-in checks against the actual host and pinned upstream downloads.

use std::path::{Path, PathBuf};

use app_lib::build::{self, progress::BuildProgress, BuildEnv};
use app_lib::domain::{
    model::{BuildOptions, HardwareProfile, MacOsVersion},
    planner, profile,
};
use app_lib::platform;
use app_lib::safety::disks;
use app_lib::services::http::Downloader;
use app_lib::tasks::cancellation::CancellationToken;

fn output_dir() -> PathBuf {
    let root = std::env::var_os("HOST_SMOKE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-smoke"));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn save(name: &str, value: &impl serde::Serialize) {
    std::fs::write(
        output_dir().join(name),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "scans the real host and enumerates its disks"]
async fn scan_profile_and_system_disk_guard() {
    let root = output_dir();
    let detected = platform::scan(&root.join("acpi"), &CancellationToken::new())
        .await
        .unwrap();
    save("scan.json", &detected);
    assert!(!detected.cpu.name.trim().is_empty(), "CPU was not detected");
    assert!(
        detected.cpu.cores > 0,
        "physical core count was not detected"
    );
    let profile = profile::build_profile(&detected);
    save("profile.json", &profile);
    assert!(!profile.cpu.name.trim().is_empty());
    let report = app_lib::domain::compatibility::assess(&profile, None);
    save("compatibility.json", &report);
    let inventory = platform::list_disks().await.unwrap();
    save("disks.json", &inventory);
    for disk in inventory.iter().filter(|d| d.is_system_disk) {
        assert!(disks::is_blocked(disk));
        assert!(disk.blocked_reason.as_ref().is_some_and(|r| !r.is_empty()));
    }
    // Internal system disks are normally excluded entirely; a system booted
    // from external storage must instead be listed with a blocking reason.
    for device in system_devices().await {
        if let Some(disk) = inventory
            .iter()
            .find(|d| d.device_path.eq_ignore_ascii_case(&device))
        {
            assert!(
                disks::is_blocked(disk),
                "system disk is selectable: {disk:?}"
            );
        } else {
            assert_eq!(
                platform::disk_info(&device).await.unwrap_err().code,
                "DISK_NOT_FOUND"
            );
        }
    }
    eprintln!(
        "Host scan and disk guard passed; output: {}",
        root.display()
    );
}

async fn system_devices() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        let output = app_lib::services::process::powershell(
            "$ErrorActionPreference='Stop'; Get-Partition -DriveLetter $env:SystemDrive.Substring(0,1) | ForEach-Object { $_.DiskNumber }",
            std::time::Duration::from_secs(60)).await.unwrap().ensure_success("system disk lookup").unwrap();
        let devices: Vec<_> = output
            .stdout
            .lines()
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .map(|n| format!(r"\\.\PhysicalDrive{n}"))
            .collect();
        assert!(!devices.is_empty(), "system disk lookup returned no disk");
        devices
    }
    #[cfg(target_os = "linux")]
    {
        let output = app_lib::services::process::run(
            "lsblk",
            &["-J", "-p", "-o", "NAME,TYPE,MOUNTPOINTS"],
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .ensure_success("system disk lookup")
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        fn holds_root(v: &serde_json::Value) -> bool {
            v["mountpoints"]
                .as_array()
                .is_some_and(|m| m.iter().any(|p| p.as_str() == Some("/")))
                || v["children"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(holds_root))
        }
        let devices: Vec<_> = json["blockdevices"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| holds_root(v))
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert!(
            !devices.is_empty(),
            "lsblk did not resolve the root filesystem: {json}"
        );
        devices
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        platform::list_disks()
            .await
            .unwrap()
            .into_iter()
            .filter(|d| d.is_system_disk)
            .map(|d| d.device_path)
            .collect()
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "reads Windows firmware tables"]
fn windows_dsdt_dump() {
    let dir = output_dir().join("firmware-tables");
    let result = platform::windows::acpi_dump::dump(&dir);
    eprintln!(
        "ACPI tables: {:?}; warnings: {:?}",
        result.written, result.warnings
    );
    let dsdt = std::fs::read(dir.join("DSDT.aml")).expect("Windows must expose a DSDT");
    assert_eq!(&dsdt[..4], b"DSDT");
    assert!(dsdt.len() >= 36);
    assert_eq!(
        u32::from_le_bytes(dsdt[4..8].try_into().unwrap()) as usize,
        dsdt.len()
    );
    assert_eq!(dsdt.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)), 0);
}

#[tokio::test]
#[ignore = "downloads real EFI components and executes the host ocvalidate binary"]
async fn network_efi_build_and_ocvalidate() {
    let root = output_dir();
    let downloader = Downloader::new(root.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    let progress = |p: BuildProgress| eprintln!("{p:?}");
    let env = BuildEnv {
        builds_dir: &root.join("builds"),
        work_dir: &root.join("work"),
        downloader: &downloader,
        cancel: &cancel,
        progress: &progress,
        keep_builds: 0,
    };
    // Hosted runners have virtual display adapters; build a known target PC
    // while the other test exercises their real hardware inventory.
    let haswell: HardwareProfile =
        serde_json::from_str(include_str!("fixtures/profiles/haswell_i74790k_rx580.json")).unwrap();
    let x58: HardwareProfile =
        serde_json::from_str(include_str!("fixtures/profiles/x58_xeon_x5670_rx580.json")).unwrap();
    let mut penryn: HardwareProfile =
        serde_json::from_str(include_str!("fixtures/profiles/penryn_q9550_hd4670.json")).unwrap();
    penryn.gpus = haswell
        .gpus
        .iter()
        .filter(|g| g.family == app_lib::domain::model::GpuFamily::AmdPolaris)
        .cloned()
        .collect();
    for (name, profile, target) in [
        ("haswell-monterey", &haswell, MacOsVersion::Monterey),
        ("x58-monterey", &x58, MacOsVersion::Monterey),
        ("x58-ventura", &x58, MacOsVersion::Ventura),
        ("penryn-mojave", &penryn, MacOsVersion::Mojave),
    ] {
        let options = BuildOptions {
            target,
            ..BuildOptions::default()
        };
        let plan = planner::plan(profile, &options).unwrap();
        save(&format!("{name}-plan.json"), &plan);
        let result = build::run(&env, profile, &options, plan).await.unwrap();
        save(&format!("{name}-result.json"), &result);
        assert!(
            result.validation.ocvalidate_ran,
            "ocvalidate did not run: {name}"
        );
        assert!(result.validation.valid, "{name}: {:#?}", result.validation);
        assert!(Path::new(&result.config_plist_path).is_file());
        assert!(Path::new(&result.efi_path)
            .join("EFI/BOOT/BOOTx64.efi")
            .is_file());
        eprintln!("EFI build passed ({name}): {}", result.efi_path);
    }
}
