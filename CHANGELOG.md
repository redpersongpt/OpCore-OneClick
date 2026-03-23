# Changelog

Project note: the app is now branded as `OpCore-OneClick`. Legacy repo/update coordinates and the persisted app-data path remain unchanged for continuity.

## 2.7.14 - 2026-03-23

### Fix Windows in-app updater install loop
- Replaced the PowerShell update worker with a `.cmd` batch script. PowerShell silently failed on systems with restrictive execution policies, leaving the app in an infinite "install again" loop on every relaunch.
- The batch approach has no execution policy dependency and uses `cmd.exe` with `windowsHide: true` so no visible console window flashes during the install handoff.
- Added startup cleanup of stale update-worker scripts from previous versions.

## 2.7.13 - 2026-03-23

### Fix #38 diskpart error misclassified as recovery download failure
- Suggestion engine's "recovery download" pattern was matching diskpart errors because v2.7.11's compound format message contained "Format-Volume recovery failed" — the word "recovery" + "failed" triggered the wrong pattern.
- Narrowed recovery download pattern to exclude diskpart/flash-usb/partition/format-volume context.
- Renamed "Format-Volume recovery" to "Format-Volume fallback" everywhere to prevent future classifier poisoning.
- Removed duplicate `void checkForAppUpdates()` call in `handlePrimaryUpdateAction` (introduced in v2.7.11).

## 2.7.12 - 2026-03-23

### Fix electron:dev startup and browser black screen
- `electron:dev` now pre-compiles electron code before starting concurrently. Electron opens ~300ms after Vite is ready instead of waiting for a full tsc run (~10s) after Vite is up. No more duplicate parallel compilation.
- Guard startup updater effects with `'electron' in window` check. In browser context (plain localhost:5173), the app no longer throws on mount from trying to call `window.electron.checkForUpdates()` when the preload isn't wired.

## 2.7.11 - 2026-03-23

### Fix #36 kext fail-fast, #37A updater dead-click, #37B/#37C flash error classification
- **#36**: Required kexts with no valid source (e.g. NootedRed offline, no embedded fallback) now fail fast before EFI validation. Removed pre-creation of empty kext directories that masked fetch failures. Stub directories from failed fetches are cleaned up.
- **#37A**: "Check for updates" button no longer silently does nothing — falls through to `checkForAppUpdates()` when no other state matches.
- **#37B**: Flash-usb errors containing "hardware" or "scan" no longer falsely match `hardware_scan_failed`. Hardware scan classifier narrowed to actual scan/detection context only.
- **#37C**: Compound diskpart + Format-Volume failures now report both stages failed. Generic diskpart fallback includes stage annotation.

## 2.7.10 - 2026-03-23

### EFI Generation fixes for Coffee Lake, Z390, and Tahoe
- SIP disabled (csr-active-config /w8AAA==) per Dortania spec for Coffee Lake and newer.
- Dual-boot safety: Set UpdateSMBIOSMode to 'Custom' to prevent SMBIOS leaking into Windows.
- Intel NIC stability: Added dk.e1000=0 boot-arg for I219/I225 controllers.
- RTC stability: DisableRtcChecksum=true for Z390/Z370 boards (Fixes BIOS resets).
- Added USBInjectAll.kext for Intel desktop systems to ensure USB port enumeration.
- Added FixupAppleEfiImages quirk for Tahoe (26+) on Coffee Lake and newer.

## 2.7.9 - 2026-03-23

### Simulation and Kext Probing
- Fix simulation hard-blocking on directUrl probe failures (NootRX/NootedRed).
- directUrl unreachable now results in 'may_fail' certainty instead of 'will_fail', allowing build to proceed with warnings.

## 2.7.8 - 2026-03-23

### Kext Fetcher fixes
- Fix NootedRed hard-fail by adding nightly.link direct URL fallback.
- Fix [object Object] error string in kext fetcher logs.

## 2.7.7 - 2026-03-23

### Reference Audit & SSDT Updates
- Added SSDT-RHUB for Alder/Raptor Lake and Comet Lake Z490 boards.
- Added ECEnabler.kext to laptop profiles.
- Added ECEnabler, SMCProcessor, SMCSuperIO, IntelMausi to kext registry.

## 2.7.6 - 2026-03-23

### EFI generator hardening
- Corrected ig-platform-ids, quirks, audio path, and SSDT coverage.
- Fix flash error routing and updater state polling bugs.

## 2.7.4 - 2026-03-23

### In-app MBR to GPT conversion
- Added an in-app destructive conversion path for selected USB targets that are still MBR, so the app can erase them, convert them to GPT, and leave behind a FAT32 `OPENCORE` volume instead of forcing a manual shell step first.
- Reused the same Windows GPT/FAT32 preparation logic for both direct conversion and flash preparation so the diskpart path stays consistent across both flows.
- Tightened reuse rules so an existing `OPENCORE` volume is only reused when the disk itself already reports GPT, preventing an MBR disk from slipping through on label alone.

## 2.7.3 - 2026-03-23

### Updater install / relaunch fix
- Downloaded updates now persist as a real staged updater session instead of living only in memory, so reopening the app no longer drops back into a fake “download again” loop.
- Windows installer handoff now records the pending install, closes the app only after a valid detached worker is launched, and relaunches the app after a successful silent install.
- Startup reconciliation now distinguishes applied, failed, interrupted, stale, and already-installed update states and restores the correct updater UI instead of falling back to a generic idle/download state.
- The updater panel now shows an explicit “Applying update…” state while install handoff is in progress.

## 2.7.2 - 2026-03-23

### Windows flash error-surface fix
- The Windows flash watchdog no longer fails an active USB flash after 60 seconds just because a long-running child process is still copying data without app-level progress updates.
- Stalled or cancelled Windows flash tasks no longer fall back to generic USB/admin failure guidance when the real problem is task state handling.
- Flash prepare blockers now stay classified as BIOS, compatibility, or disk-selection/identity precondition failures instead of surfacing as generic USB write failures.
- Diagnostics now retain the selected disk context for prepare-time flash failures, even if the destructive write never starts.

## 2.5.6 - 2026-03-22

### Windows flash-prep correctness fix
- Made the Windows `diskpart` prep script explicit by re-selecting the created partition before formatting and assigning it, instead of relying on implicit focus.
- Replaced the old “any partition exists?” + label-only lookup path with deterministic partition-state inspection on the selected disk after `diskpart`.
- Flash prep now distinguishes real stage failures on Windows: partition creation, FAT32 format, drive-letter assignment, and OPENCORE label lookup.
- Added a partition-based fallback when Windows creates the right FAT32 partition and drive letter but label lookup lags or comes back empty, so that case no longer masquerades as partition creation failure.

### Release workflow fix
- Fixed the direct `gh release create` publish step by passing the repository explicitly in the publish job, which runs without a checked-out `.git` directory.

## 2.5.4 - 2026-03-22

### Release workflow cleanup
- Replaced the Node 20-only release-publish action with a direct `gh release create` step so the release workflow no longer depends on a deprecated Node 20 action.

## 2.5.3 - 2026-03-22

### GitHub rename follow-up
- Updated release, issue, homepage, and updater lookup URLs to the live `redpersongpt/OpCore-OneClick` repo after the GitHub repo rename landed.
- Kept the existing `appId` and persisted `userData` path unchanged, but stopped relying on GitHub's redirect for updater checks.

## 2.5.2 - 2026-03-22

### SSDT sourcing follow-up
- Fixed the SSDT source-policy regression in the in-progress follow-up patch so the Electron build path matches the source-policy module again.
- Added explicit source coverage for the shipped AMD desktop, Coffee Lake, Alder Lake, and Raptor Lake SSDT sets.
- EFI build now fails clearly if a required SSDT has no supported source policy instead of falling into a type/runtime mismatch.

### Release workflow cleanup
- Updated the release workflow action versions to current Node 24-capable majors to clear the GitHub Actions Node 20 deprecation warnings from the release pipeline.

## 2.5.1 - 2026-03-22

### Product rebrand
- The app is now branded as `OpCore-OneClick` across the packaged app, window title, diagnostics, support logs, updater copy, and CLI usage text.
- Existing updater/release continuity was preserved. The app still checks releases from `redpersongpt/macOS-One-Click` because the GitHub repo slug is unchanged.
- Internal compatibility-sensitive paths were intentionally kept where needed. The existing `appId`, persisted `userData` path, and legacy `macos-oneclick` CLI alias remain in place for continuity.

## 2.5.0 - 2026-03-22

### Linux privilege model rewrite (#18)
- The app no longer tells Linux users to run the entire GUI as root. Disk operations now use pkexec (polkit) for per-command elevation, the same model macOS uses with system prompts.
- Removed `--no-sandbox` from default Linux executable args. The auto no-sandbox workaround is kept only for backwards compatibility when already running as root.
- Updated all privilege-related error messages, suggestions, and remediations to say "install polkit" instead of "run with sudo."

### CLI support (#14)
- Added a headless CLI entry point (`opcore-oneclick`, legacy alias `macos-oneclick`, or `node dist-electron/electron/cli.js`).
- Commands: `scan`, `compatible`, `report`, `matrix`, `version`.
- JSON (`--json`) and human-readable output. Correct exit codes (0=ok, 1=error/blocked, 2=usage).
- `--target` flag for specifying macOS version on `compatible` and `report` commands.
- Read-only — destructive operations (flash/build) require the GUI.

### Architecture cleanup
- Extracted `detectCpuGeneration`, `detectArchitecture`, `mapDetectedToProfile` from main.ts into `electron/hardwareMapper.ts` so both the Electron app and CLI share the same mapping logic.

## 2.4.7 - 2026-03-22

### Windows flash and GPT detection fix (#15, #16, #17)
- Fixed GPT detection on Windows: `Get-Disk` now targets the specific disk number and wraps in try/catch so GPT drives are no longer misreported as "unknown partition table."
- Improved diskpart error handling: silent `.catch(() => {})` on drive-letter assignment replaced with logged error path; retry window widened from 12×500ms to 20×400ms.
- Final flash error now distinguishes "partition exists but no letter" from "no partition created at all."
- Added structured error entries for diskpart prep failure and drive-letter assignment failure with actionable remediation copy.
- SMBIOS recomputed on target OS change so Tahoe builds get the correct model.
- SSDT sourcing reads real AML from OpenCore package instead of empty placeholder files.
- VersionStep: "Use {version}" vs "Continue with {version}" button logic; cursor-pointer on all interactive buttons.
- Resource plan no longer blanks while a new plan loads — keeps previous plan visible.
- Recovery download step shows accurate 700 MB copy instead of implying full OS size.

### Test coverage
- Added 7 new test files covering diskOps safety commands, structured error classification, flash safety gates, kext source policy, release flow SMBIOS, config generator, and VersionStep button states.

## 2.4.6 - 2026-03-22

### Codeless kext validation fix (#13)
- Fixed EFI build failing on codeless kexts like AppleMCEReporterDisabler that have only an Info.plist and no binary in Contents/MacOS. Both the build validator and the install validator now recognize codeless kexts as valid.

## 2.4.5 - 2026-03-22

### UI layout and polish (#10)
- Rewrote the updater panel to a compact single-row layout so download/install/restart actions stay visible without scrolling.
- Removed dead blank space from compatibility matrix cards (hard min-height floor).
- Tightened spacing and shortened copy across version selection, hardware report, and resource plan screens.
- Made the landing page scrollable so the updater is never clipped below the viewport.

### Test coverage (#9, #10, #11)
- Added 83 regression tests covering updater action visibility, sidebar single-active state, resource plan persistence across Build EFI re-entry, and compatibility matrix layout invariants.

## 2.4.4 - 2026-03-22

### Windows disk and flash fixes (#9, #11)
- Fixed Windows disk identity so `PhysicalDriveN` paths are canonical end-to-end instead of mixing `diskN` aliases that broke partition-table and system-disk checks.
- Widened USB detection beyond `BusType=USB` to catch bridge-backed removable media through `InterfaceType`, `PNPDeviceID`, and `MediaType`.
- Separated partition-table errors from system-disk errors so users see the right remediation instead of a generic safety block.
- Capped FAT32 partition size on drives larger than 32 GB so `diskpart` prep does not exceed the FAT32 volume limit.
- Added a diskpart retry path for the common Windows lock/mount-handle failure.
- Restored flash authorization from saved scan artifacts on app restart so a valid session survives relaunch.

### Kext delivery, resource plan, and UI fixes (#10)
- Kext resolution now routes explicitly to bundled, GitHub, direct download, embedded fallback, or hard fail — no more fake offline stubs.
- Added direct-download paths for AMD kexts and other non-API-friendly sources.
- Resource plan persists across Build EFI re-entry when the hardware profile has not changed.
- Sidebar highlights exactly one active step instead of allowing duplicates.
- Updater polling runs only during active check/download/install states.
- Shortened recommendation, compatibility, and report copy.

## 2.3.8 - 2026-03-21

- Fixed USB flash-prep disk identity loss on Windows so valid removable targets keep a stable identity from selection through flash confirmation.
- Fixed a stale disk-info race where an older lookup could overwrite the currently selected USB target.
- Fixed Windows disk info IPC instability by degrading slow removable-media queries safely instead of failing the whole lookup.

## 2.3.6 - 2026-03-21

- Fixed the remaining EFI-build stall after BIOS Continue by auto-starting the build flow instead of landing on a second manual Begin gate.
- Prevented stale BIOS refresh responses from clearing an accepted BIOS session after Continue, so the non-destructive build path stays coherent until EFI generation starts.
- Added an in-app "Update to latest version" button that opens the latest GitHub release page from both the landing screen and the main shell.
- Timed out BIOS-state firmware probing in the main process and emitted EFI-build progress before BIOS validation so hung firmware reads fail clearly instead of leaving the build screen waiting indefinitely.
- Kept destructive safety unchanged: accepted BIOS sessions still do not unlock partitioning or flashing without a real ready BIOS state.

## 2.3.3 - 2026-03-20

- Fixed the BIOS-step recovery bug where the combined recheck/continue flow could fall into a generic unknown-error surface instead of staying in a clear BIOS-specific path.
- Split the BIOS step into distinct Recheck BIOS and Continue actions so recheck reruns firmware detection while continue uses the current known checklist state without silently probing again.
- Added BIOS-specific recovery messaging and session persistence for continuing from the current BIOS state, while keeping the later build/deploy safety gates unchanged.

## 2.3.2 - 2026-03-20

- Fixed a release-blocking EFI build pipeline bug where renderer-side step transitions could read stale state after a successful build and leave the app looking stuck instead of moving forward or failing clearly.
- Added build-flow stall detection, clearer recovery context, and richer issue-report diagnostics for stuck build, kext, and recovery phases without weakening any destructive safety checks.
- Removed the Safe / Exploratory planning-mode split and collapsed the app onto one clearer compatibility guidance path.
- Reworked macOS version selection into a calmer, more professional recommendation layout with a stronger featured starting point and cleaner state presentation.

## 2.3.1 - 2026-03-20

- Hardened packaged startup recovery so renderer/load failures surface a clean recovery path instead of raw diagnostics or blank-screen dead ends.
- Added sanitized support-log export and cleaner failure recovery messaging without changing flash, BIOS, validation, backup, or destructive authorization rules.
- Refined the macOS selection and BIOS preparation screens for calmer hierarchy, clearer recommendations, steadier back navigation, and better checklist readability.

## 2.3.0 - 2026-03-20

- Added guided fix suggestions, community match levels, likely failure points, and decision-trace context for Experimental and Risky Hackintosh paths.
- Added Safe Mode and Exploratory Mode planning surfaces so advanced users can stretch community-proven hardware paths without weakening destructive safety.
- Kept destructive write protection unchanged: flash token safety, live disk identity checks, EFI validation, BIOS readiness, backup policy, and confirmation flow are all intact.
- Preserved the packaged startup-path fix so published Windows and Linux builds do not regress into a black-screen launch failure.

## 2.2.2 - 2026-03-20

- Fixed the packaged startup path so released apps load the renderer bundle from the correct `dist/index.html` location instead of opening to a black screen.
- Fixed the Electron packaging entry so Windows and Linux packages include the correct compiled main-process bootstrap.
- Split build and packaging scripts so release packaging can target Windows and Linux without producing a macOS package.
- Moved public release packaging to native GitHub Actions runners for Windows and Linux and removed the macOS publish job from the release workflow.

## 2.2.1 - 2026-03-20

- Hardened diagnostics and issue reporting so build, validation, recovery, disk, simulation, and runtime failures produce sanitized bug-report drafts instead of leaking raw paths or identifiers.
- Sanitized structured logs and diagnostics output to remove tokens, personal paths, raw serial-like values, and full device identifiers from copied reports.
- Tightened renderer failure handling so runtime exceptions surface a stable error overlay instead of silently disappearing.
- Preserved the existing destructive safety architecture while keeping imported profiles, simulation, backup manifests, and resource plans advisory-only.
- Removed renderer-side environment-key injection and cleaned release metadata for public distribution.
