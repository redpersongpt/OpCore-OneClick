# Architecture Map

OpCore-OneClick is a Tauri v2 app. The React frontend in `src/` only renders
state and asks for work through IPC. Everything that touches the network, the
disk, child processes or hardware lives in the Rust backend in `src-tauri/`.

```
┌──────────────────────────── webview (React) ────────────────────────────┐
│ pages/ (wizard steps) · components/ · stores/ (zustand)                 │
│ bridge/ ── invoke("command", args)                     listen(event) ◄─ │
│ bridge/generated/*.ts  (types generated from Rust by ts-rs)             │
└───────────────┬───────────────────────────────────────────▲─────────────┘
                │ IPC commands (JSON)                       │ events
┌───────────────▼────────────────────── Rust ───────────────┴─────────────┐
│ commands/   thin #[tauri::command] wrappers, task bookkeeping           │
│ domain/     pure logic: knowledge bases, profile, compatibility,        │
│             planner, ACPI/AML, config.plist writer, SMBIOS              │
│ services/   I/O: downloads + cache, artifact extraction, ocvalidate,    │
│             child processes and elevation                               │
│ platform/   per-OS hardware scan, ACPI dump, disk listing and flashing  │
│ safety/     flash confirmation tokens, disk identity fingerprints       │
│ tasks/      task registry, progress events, cancellation                │
└─────────────────────────────────────────────────────────────────────────┘
```

## Backend modules (`src-tauri/src`)

| Module | Role |
|---|---|
| `lib.rs` | Plugin setup (log, dialog, shell), managed state, command registration |
| `contracts.rs` | IPC types (`DetectedHardware`, `ScanResult`, `CompatibilityReport`, `BuildResult`, `DiskInfo`, progress payloads, ...). All derive `ts_rs::TS` |
| `error.rs` | `AppError { code, message, suggestion, recoverable }`, the error type of every command |
| `paths.rs` | `AppPaths`: data, download cache, builds, recovery images, dumped ACPI tables, scratch space |
| `domain/model.rs` | Core types: `MacOsVersion` (10.13 … 26), `CpuPlatform`, `GpuFamily`, `HardwareProfile`, `BuildOptions`, `BuildPlan` and its parts |
| `domain/cpu_db.rs` | CPU identification from CPUID family/model and the brand string |
| `domain/gpu_db.rs` | GPU identification and macOS support ranges by PCI id |
| `domain/chipset_db.rs` | PCH / FCH identification (drives SSDT and quirk choices) |
| `domain/device_db.rs` | Ethernet, Wi-Fi, Bluetooth, touchpad and storage controllers → driver decisions |
| `domain/codec_db.rs` | HDA codec names and AppleALC layout ids |
| `domain/smbios_db.rs` | Apple model database: min/max macOS, board-id, Secure Boot model |
| `domain/macos_db.rs` | Recovery board ids per release and per-release caveats |
| `domain/kext_catalog.rs` | Pinned downloads (OpenCore, OcBinaryData, every kext) with SHA-256 |
| `domain/profile.rs` | `DetectedHardware` → `HardwareProfile` (interpretation lives here, never in scanners) |
| `domain/compatibility.rs` | Per-component assessment and the list of macOS versions the machine can run |
| `domain/bios.rs` | Firmware settings checklist for the profile and target |
| `domain/planner.rs` | `HardwareProfile` + `BuildOptions` → declarative `BuildPlan` (SMBIOS, kexts, SSDTs, ACPI patches, quirks, DeviceProperties, NVRAM, drivers) |
| `domain/acpi/` | DSDT/SSDT parsing, AML encoder, SSDT generation |
| `domain/amd_patches.rs` | AMD kernel patches (AMD_Vanilla) |
| `domain/config_writer.rs` | `Docs/Sample.plist` of the downloaded OpenCore + `BuildPlan` → `config.plist` |
| `domain/kernel_add.rs` | `Kernel → Add` entries from the extracted kexts, in dependency order |
| `domain/smbios_gen.rs` | Serial, MLB, UUID and ROM (serials come from `macserial`) |
| `services/http.rs` | Downloader: timeouts, retries, resume, cancellation, SHA-256 verified cache, GitHub release lookup with rate-limit handling |
| `services/artifacts.rs` | Fetch and safely extract OpenCore, OcBinaryData and kexts (zip-slip safe) |
| `services/ocvalidate.rs` | Runs `ocvalidate` from the same OpenCore package and checks the EFI layout |
| `services/process.rs` | Child processes without a shell, timeouts, no console windows on Windows, elevation |
| `platform/common.rs` | PnP id and device location path parsing shared by the scanners |
| `platform/{windows,linux,macos}/scanner.rs` | Hardware facts and ACPI table dump for the host |
| `platform/{windows,linux,macos}/disk.rs` | Removable disk listing and flashing |
| `safety/flash_auth.rs` | HMAC-signed, single-use flash confirmation tokens |
| `safety/disk_identity.rs` | Disk fingerprints and collision checks |
| `tasks/` | `TaskRegistry` (progress, watchdog, `task:update` events) and `CancellationToken` |
| `commands/*.rs` | The IPC surface listed below |

## Frontend (`src`)

| Path | Role |
|---|---|
| `pages/` | Wizard steps: Welcome, Scan, Hardware, Compatibility, BIOS, Build, Review, Deploy (USB installer), Complete; Settings and Troubleshoot open as dialogs |
| `stores/` | zustand stores: `wizard` (step order, completed steps, navigation locks while a task runs) plus one store per area (hardware, compatibility, firmware, build, deploy, tasks) |
| `bridge/` | Typed wrappers around `invoke` for every command and typed listeners for the backend events |
| `bridge/generated/` | TypeScript types generated from `contracts.rs` and `domain/model.rs`. Do not edit; run `cargo test` in `src-tauri` to refresh them |

Steps unlock in order. Changing an input invalidates every later step: a new
scan or import reopens the hardware step, profile or target changes reopen
compatibility, and build options reopen the build (see
`docs/state-machines/wizard-flow.mmd`).

## Data flow

```
scan_hardware ─► DetectedHardware ─► profile::interpret ─► HardwareProfile ─┐
import_profile (JSON exported on the target PC) ────────────────────────────┤
                                                                            ▼
check_compatibility ─► CompatibilityReport (supported components, macOS options)
get_bios_settings / probe_firmware ─► firmware checklist
plan_build ─► BuildPlan (preview, no downloads)
build_efi ─► BuildResult ─► builds/<id>/EFI
download_recovery ─► recovery/<version>/com.apple.recovery.boot/{BaseSystem.dmg, BaseSystem.chunklist}
flash_prepare_confirmation ─► FlashConfirmation (token)
flash_usb(token) ─► GPT + FAT32 USB with EFI/ and com.apple.recovery.boot/
```

### EFI build pipeline (`commands/efi.rs::build_efi`, task kind `efi-build`)

1. `planner::plan` turns the profile and options into a `BuildPlan`.
2. `artifacts::fetch_opencore` downloads the pinned OpenCore release (RELEASE or
   DEBUG) and checks its SHA-256.
3. `X64/EFI/{BOOT,OC}` is copied; drivers and tools the plan does not use are
   removed.
4. `fetch_ocbinarydata` adds OpenCanopy resources and `HfsPlus.efi`.
5. Each `KextSelection` is fetched and installed. An optional kext that fails
   is skipped and dropped from the plan; a required one fails the build.
6. SSDTs are written: generated AML, OpenCore sample binaries or prebuilt
   Dortania tables.
7. `kernel_add::build_kernel_add` orders `Kernel → Add` from the installed
   bundles.
8. `smbios_gen::generate_identity` creates serial, MLB, UUID and ROM with the
   `macserial` from the same package.
9. `config_writer::write_config` fills that release's `Docs/Sample.plist`.
10. `ocvalidate::validate_efi` runs the matching `ocvalidate` and the layout
    checks.

The plan is data, so the Review step can show exactly what will be written
before anything is downloaded, and the same plan can be exported or rebuilt.

## IPC commands

| Area | Command | Notes |
|---|---|---|
| App | `get_app_info` | App, OpenCore, host OS and arch versions |
| | `check_for_updates` | Compares with the latest GitHub release |
| Hardware | `scan_hardware` | Task `hardware-scan`; dumps ACPI tables into `AppPaths.acpi` |
| | `refresh_profile` | Re-interprets a manually edited profile |
| | `get_catalog` | Options for the profile editor and version picker |
| | `export_profile` / `import_profile` | Build for this machine on another computer |
| EFI | `check_compatibility` | Optional target version |
| | `plan_build` | Preview only |
| | `get_bios_settings` | Checklist for profile + target |
| | `build_efi` | Task `efi-build` |
| | `validate_efi` | Any EFI folder or `config.plist` |
| | `export_efi` | Copies the built `EFI` folder to a chosen directory |
| Disk | `list_usb_devices`, `get_disk_info` | System disks are listed but flagged and blocked |
| | `check_privileges` | Whether the app is elevated or can ask for elevation |
| | `flash_prepare_confirmation` | Issues the flash token |
| | `flash_usb` | Task `usb-flash`; needs the token |
| Firmware | `probe_firmware` | UEFI mode, Secure Boot, VT-x, VT-d, Above 4G where readable |
| Recovery | `download_recovery` | Task `recovery-download`; resumable, chunklist verified |
| | `get_cached_recovery_info`, `clear_recovery_cache` | |
| Diagnostics | `log_get_session_id`, `log_get_tail`, `save_support_log`, `clear_app_cache` | |
| State | `get_persisted_state`, `save_state`, `clear_state` | Wizard state across restarts |
| Tasks | `task_list`, `task_cancel` | |

Errors are returned as `AppError`. The frontend shows `message` and
`suggestion` and can branch on `code`.

## Events

| Event | Payload | Emitted by |
|---|---|---|
| `task:update` | `TaskUpdate { taskId, kind, status, progress, message, detail }` | `TaskRegistry` for every task |
| `flash:progress` | `FlashProgress { taskId, phase, progress, message, error }`; phases `prepare`, `partition`, `format`, `copy-efi`, `copy-recovery`, `verify`, `complete`, `failed` | `flash_usb` |
| `recovery:progress` | `RecoveryProgress { taskId, version, phase, downloaded, total, progress, error }`; phases `resolving`, `downloading`, `verifying`, `complete`, `failed` | `download_recovery` |

Task kinds: `hardware-scan`, `efi-build`, `recovery-download`, `usb-flash`.
Statuses: `running`, `completed`, `failed`, `cancelled` (terminal states do not
change again). See `docs/state-machines/`.

## Safety model

Writing a USB drive is the only destructive operation. It is guarded in layers:

1. **Target selection.** `list_usb_devices` returns removable and external
   disks. A disk that holds the running OS, the boot loader or a page file is
   returned with `is_system_disk` and a `blocked_reason` and cannot be chosen.
2. **Confirmation token.** `flash_prepare_confirmation` signs (HMAC-SHA256 with a
   per-session random key) the disk identity (path, model, vendor, serial,
   size), the hash of the EFI folder and the recovery choice. The token expires
   after 5 minutes, is bound to the app session and can be used once.
3. **Re-check before writing.** `flash_usb` verifies the token, re-reads the
   disk and compares fingerprints, and refuses if the disk changed, is now a
   system disk, or is ambiguous with another connected disk.
4. **Least privilege.** On Windows the app runs elevated (diskpart and raw
   disk access need it). On Linux and macOS the UI runs as the user and only
   the disk script is elevated, through `pkexec` or the macOS administrator
   prompt. Programs are started without a shell, with fixed
   arguments, a timeout, and are killed when the timeout fires.
5. **Verification.** After copying, files on the USB are compared with the
   source. The recovery image is checked against Apple's chunklist before it
   is offered.

Integrity of what goes onto the drive:

- No third-party binaries ship with the app. OpenCore, OcBinaryData and kexts
  are downloaded from their upstream releases, pinned to tested versions and
  verified by SHA-256. The only exception is NootRX, which publishes rolling
  builds only; its archive is validated structurally. "Use latest releases"
  resolves newer versions through the GitHub API and falls back to the pins on
  any failure.
- Archives are extracted with path checks (no absolute paths, `..`, drive
  prefixes or symlinks) and a size cap.
- `config.plist` always starts from the `Sample.plist` of the same OpenCore
  release and is validated with that release's `ocvalidate`, so the schema
  matches the bootloader version.

Webview hardening:

- CSP `default-src 'self'`; besides Tauri's IPC origins (`ipc:`,
  `http://ipc.localhost`) the only origin the webview may contact is
  `https://api.github.com` (for a release check from the webview; the app's
  own check runs in Rust through `check_for_updates`). No scripts, styles,
  fonts or images are loaded from other origins, and all downloads happen in
  Rust.
- `capabilities/default.json` grants only core defaults, the window controls of
  the custom title bar, native dialogs, logging and `shell.open`, and
  `plugins.shell.open` restricts `open` to `https://` URLs.

## Platform notes

| Host | Scan | Flash |
|---|---|---|
| Windows 10/11 | CIM/WMI, PnP device data and the registry; ACPI tables read from the firmware | diskpart; the FAT32 partition is capped at 32 GB so Windows can format it |
| Linux | sysfs, `/proc` and in-process CPUID (no `lspci` or `dmidecode` needed); ACPI tables from `/sys/firmware/acpi/tables`, which only root can read, so a scan as a normal user skips them with a warning | One elevated script (`pkexec`) that checks the disk identity again, partitions, formats, copies and reads the files back |
| macOS | Mainly builds for another PC from a profile exported on the target | `diskutil` behind the administrator prompt |

The Linux `.deb` depends on `util-linux`, `fdisk`, `dosfstools` and `pkexec`
for these steps and recommends `pciutils` (its `pci.ids` gives readable device
names) and `udisks2`. The AppImage cannot declare dependencies, so the flash
script checks for every tool before it touches the disk.
