# Internal QA Checklist (macOS One-Click)

This document contains manual test scenarios to verify the resilience and safety of the application. 
No external tools are required except the `SIM_MODE` environment variable.

## 1. Safety & Data Protection
- [ ] **System Disk Block**: Try to select your main OS drive in the USB step. It must be hard-blocked.
- [ ] **MBR Block**: Use an MBR-formatted USB. The UI must show a hard block with remediation instructions.
- [ ] **Unknown Partition Block**: Use a drive with a corrupted/missing partition table. The UI must block the confirmation flow.
- [ ] **Insufficient Space**: Select a small USB (< 16 GB) for a full recovery install. The UI and backend must block progress.
- [ ] **Destructive Confirmation**: Every destructive path (USB Flash, Local Partition) must require explicit typed/checkbox confirmation.

## 2. Failure Simulations
To run these, launch the app with: `SIM_MODE=scenario npm run dev`

- [ ] **Disk Write Failure** (`SIM_MODE=disk:write-fail`):
  - Start a USB flash.
  - UI should catch the error immediately and fallback to the USB selection step with a clear error notification.
- [ ] **USB Disconnect** (`SIM_MODE=usb:disconnect`):
  - Start a flash. 
  - UI should detect the lost device and prevent further write attempts.
- [ ] **Download Interruption** (`SIM_MODE=download:interrupted`):
  - Start recovery download.
  - App should show a "Retry" option. Resume should work correctly from the previous byte offset.
- [ ] **Corrupted Download** (`SIM_MODE=download:corrupt`):
  - Let download finish.
  - Checksum failure must be caught *before* any flash operation starts.
- [ ] **Hardware Detection Timeout** (`SIM_MODE=hw:timeout`):
  - Start hardware scan.
  - Loading state should persist correctly without crashing the sidebar.

## 3. Edge Case Hardening
- [ ] **ID Change**: Unplug and replug the USB drive during the 4-checkbox confirmation. The "Flash" button should fail safely because the device node changed.
- [ ] **Abort-on-Close**: Start a USB flash. Try to close the app window. A critical alert must appear, and the window must stay open until the task ends.
- [ ] **Resume Persistence**: Close the app mid-download. Re-open. It should automatically return to the download step and resume.

## 4. Diagnostics Audit
- [ ] **Snapshot Content**: Fail a task, then click "Copy Diagnostics". Paste into a text editor.
- [ ] **Verify Fields**: Ensure `diskInfo`, `hwSummary`, and `firmwareSummary` are present.
- [ ] **Sanitization**: Ensure no real usernames (e.g., `/Users/yourname`) are in the diagnostics string.
