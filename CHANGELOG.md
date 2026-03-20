# Changelog

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
