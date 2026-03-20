# Changelog

## 2.2.2 - 2026-03-20

- Fixed the Electron packaging entry so Windows and Linux packages include the correct compiled main-process bootstrap.
- Split build and packaging scripts so release packaging can target Windows and Linux without producing a macOS package.
- Moved public release packaging to native GitHub Actions runners for Windows and Linux and removed the macOS publish job from the release workflow.

## 2.2.1 - 2026-03-20

- Hardened diagnostics and issue reporting so build, validation, recovery, disk, simulation, and runtime failures produce sanitized bug-report drafts instead of leaking raw paths or identifiers.
- Sanitized structured logs and diagnostics output to remove tokens, personal paths, raw serial-like values, and full device identifiers from copied reports.
- Tightened renderer failure handling so runtime exceptions surface a stable error overlay instead of silently disappearing.
- Preserved the existing destructive safety architecture while keeping imported profiles, simulation, backup manifests, and resource plans advisory-only.
- Removed renderer-side environment-key injection and cleaned release metadata for public distribution.
