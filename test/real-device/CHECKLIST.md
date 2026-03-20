# Real-Device Test Checklist
## macOS One-Click — Pre-Release Validation

Each scenario has exact pass/fail criteria. Mark each step ✅ PASS, ❌ FAIL, or ⏭ SKIP with a note.

---

## Environment Setup (run once before all scenarios)

```
App version: ___________   (check release/macOS-OneClick-*.dmg filename)
Test date:   ___________
Tester OS:   ___________   (e.g. "Windows 11 22H2", "Ubuntu 22.04", "macOS 14.4")
Node/Electron: __________  (from About panel or package.json)
Test USB:    ___________   (brand, capacity, e.g. "SanDisk Ultra 32GB")
USB device:  ___________   (e.g. /dev/disk4, /dev/sdb, \\.\PhysicalDrive2)
```

> **SAFETY RULE**: Never use a USB drive that contains data you cannot afford to lose.
> Always confirm the device path with `diskutil list` / `lsblk` / `Get-Disk` before running any test.

---

## SCENARIO 1: System Disk Guard

**Purpose**: Prove the OS boot disk can never be flashed.

**Setup**: Identify your system disk before starting.
- macOS: `diskutil info / | grep "Part of Whole"` → note the value (e.g. `disk0`)
- Linux: `lsblk -no PKNAME $(findmnt -n -o SOURCE /)` → note value (e.g. `nvme0n1`)
- Windows: `Get-Partition -DriveLetter C | Get-Disk | Select Number` → note value

**System disk identified**: _________________

**Steps**:
1. Launch the app, complete hardware scan and EFI build
2. At the USB Select step, if the system disk appears in the list: attempt to select it and click Flash
3. If the system disk does not appear (correctly filtered by `diskutil list external physical`): verify it is absent

**Pass criteria**:
- [ ] If attempted: error message contains "system boot disk" or "SAFETY BLOCK" and step does NOT advance to Flashing
- [ ] If absent from list: system disk device path is not shown to user at all
- [ ] `app.log` contains a WARN entry with `code: SYSTEM_DISK` and the correct device path
- [ ] `operations.log` contains `task_failed` entry with matching taskId

**Fail criteria**:
- Flash step proceeds against the system disk
- No error is shown
- Log is empty or missing SYSTEM_DISK entry

---

## SCENARIO 2: MBR Disk Guard + Modal

**Purpose**: Prove MBR disks are blocked with actionable instructions.

**Setup**: Prepare an MBR USB disk.
- macOS: `diskutil eraseDisk FAT32 TEST MBRFormat /dev/diskX`
- Linux: `sudo parted /dev/sdX --script mklabel msdos mkpart primary fat32 1MiB 100%`
- Windows: `Initialize-Disk -Number X -PartitionStyle MBR`

**Steps**:
1. Complete hardware scan and EFI build
2. At USB Select, select the MBR disk
3. Click Flash

**Pass criteria**:
- [ ] A modal appears (not a toast/error banner) with title containing "MBR Partition Table"
- [ ] Modal shows the device path
- [ ] Modal shows platform-specific conversion instructions (diskutil / parted / PowerShell)
- [ ] Only a Cancel button is shown (no "proceed anyway" option)
- [ ] `app.log` contains WARN with `code: MBR_PARTITION_TABLE`
- [ ] Flash does NOT proceed — disk is unchanged after dismissing modal

**Fail criteria**:
- Flash proceeds to a GPT-to-MBR conversion attempt
- Modal shows but has no actionable instructions
- Modal shows but has a "proceed anyway" button

---

## SCENARIO 3: Successful USB Flash (GPT)

**Purpose**: Prove a clean GPT USB is flashed correctly with a valid EFI.

**Setup**: Use a GPT-formatted USB.
- macOS: `diskutil eraseDisk FAT32 TEST GPTFormat /dev/diskX`
- Linux: `sudo parted /dev/sdX --script mklabel gpt mkpart primary fat32 1MiB 100%`
- Windows: Run diskpart: `select disk X` → `clean` → `convert gpt`

**Steps**:
1. Complete hardware scan, version select, EFI build, kext fetch
2. Select the GPT USB at USB Select step
3. Complete the flash

**Pass criteria**:
- [ ] Flash completes — app advances to Complete step
- [ ] Mount the USB and verify directory structure:
  ```
  /EFI/
    BOOT/
    OC/
      config.plist       ← must exist and be non-zero bytes
      ACPI/
      Drivers/
      Kexts/
      Resources/
  ```
- [ ] `config.plist` is valid XML: `plutil -lint /Volumes/OPENCORE/EFI/OC/config.plist` → `OK` (macOS)
  OR: `xmllint --noout /mount/EFI/OC/config.plist` (Linux)
- [ ] USB was ejected cleanly (not just unmounted with data loss risk)
- [ ] `app.log` contains INFO `usb-flash complete` with device path
- [ ] `operations.log` contains `task_complete` with `kind: usb-flash`
- [ ] Total EFI size on USB: `du -sh /Volumes/OPENCORE/EFI` is non-zero and < 480 MB

**Fail criteria**:
- App shows Complete but USB has no EFI directory
- `config.plist` is missing, empty, or malformed XML
- USB is mounted but not ejected after flash
- Any kext directory under `EFI/OC/Kexts/` is empty (stubs without real content)

---

## SCENARIO 4: EFI Integrity After Flash

**Purpose**: Verify the kexts are real binaries, not empty stubs.

**Setup**: Run after Scenario 3 (USB must be flashed and mounted).

**Steps**:
1. Mount the flashed USB
2. Check each kext bundle for its executable:

**macOS check**:
```bash
MOUNT=/Volumes/OPENCORE
for kext in "$MOUNT/EFI/OC/Kexts"/*.kext; do
  binary="$kext/Contents/MacOS/$(basename $kext .kext)"
  if [ -f "$binary" ]; then
    size=$(stat -f%z "$binary")
    echo "✅ $(basename $kext): $size bytes"
  elif [ -f "$kext/.version" ]; then
    echo "⚠️  $(basename $kext): STUB (offline fallback)"
  else
    echo "❌ $(basename $kext): MISSING BINARY"
  fi
done
```

**Linux check**:
```bash
MOUNT=/mnt/opencore
for kext in "$MOUNT/EFI/OC/Kexts"/*.kext; do
  binary="$kext/Contents/MacOS/$(basename $kext .kext)"
  if [ -f "$binary" ]; then
    size=$(stat -c%s "$binary")
    echo "PASS: $(basename $kext): $size bytes"
  elif [ -f "$kext/.version" ]; then
    echo "WARN: $(basename $kext): offline stub"
  else
    echo "FAIL: $(basename $kext): missing binary"
  fi
done
```

**Pass criteria**:
- [ ] `Lilu.kext` binary exists and is > 100 KB
- [ ] `VirtualSMC.kext` binary exists and is > 50 KB
- [ ] `WhateverGreen.kext` binary exists and is > 200 KB
- [ ] No kext shows "MISSING BINARY"
- [ ] Stubs (offline fallbacks) are documented in `app.log` with version `'offline'`

**Fail criteria**:
- Any required kext has a missing binary
- All kexts are stubs (no network access during build)
- A kext directory exists but `Contents/MacOS/` is absent

---

## SCENARIO 5: Cancellation Mid-Download

**Purpose**: Prove cancellation during recovery download leaves no corrupted state.

**Steps**:
1. Reach the recovery download step
2. After progress exceeds 10%: call `window.electron.taskCancel(<taskId>)` via DevTools console
   OR: use the Cancel button in the UI if wired
3. Observe UI state after cancellation

**Pass criteria**:
- [ ] UI transitions back to report step (or shows error state — not stuck on progress screen)
- [ ] `app.log` contains a WARN or INFO `Task X was cancelled`
- [ ] `operations.log` contains `task_cancelled` entry
- [ ] Re-running the recovery download from the same EFI path succeeds (no partial file corruption)
- [ ] If a partial `.dmg` file exists in the EFI path: it is either cleaned up OR correctly used as resume offset on retry
- [ ] Calling `window.electron.taskCancel` on the completed/cancelled task returns `false` (not a crash)

**Fail criteria**:
- UI shows "Downloading..." indefinitely after cancel
- `task_cancelled` is missing from operations.log
- Re-running download fails due to corrupted partial file

---

## SCENARIO 6: Partition Table Detection Accuracy

**Purpose**: Prove `getDiskInfo` returns correct partition table values.

**Setup**: Prepare two USBs — one GPT, one MBR (from Scenario 2 setup).

**Steps**:
Open DevTools console and run:
```javascript
// Test GPT disk:
const gptResult = await window.electron.getDiskInfo('/dev/diskX'); // replace with your GPT disk
console.log('GPT result:', JSON.stringify(gptResult));

// Test MBR disk:
const mbrResult = await window.electron.getDiskInfo('/dev/diskY'); // replace with your MBR disk
console.log('MBR result:', JSON.stringify(mbrResult));
```

**Pass criteria**:
- [ ] GPT disk: `{ partitionTable: "gpt", isSystemDisk: false, mountedPartitions: [...] }`
- [ ] MBR disk: `{ partitionTable: "mbr", isSystemDisk: false, ... }`
- [ ] System disk: `{ isSystemDisk: true, ... }`
- [ ] Response time < 2 seconds on each call

**Fail criteria**:
- GPT disk returns `"unknown"` for partition table
- System disk returns `isSystemDisk: false`
- Call hangs > 5 seconds

---

## SCENARIO 7: Preflight Validation

**Purpose**: Prove preflight catches missing binaries and disk space issues.

**Steps**:
Open DevTools console and run:
```javascript
const result = await window.electron.runPreflight();
console.log(JSON.stringify(result, null, 2));
```

**Pass criteria**:
- [ ] `adminPrivileges` matches actual privilege level (run as root/admin → `true`)
- [ ] All platform binaries show `true` if installed (e.g. `diskutil: true` on macOS)
- [ ] `freeSpaceMB` is within ±200MB of actual free space (check with `df -h` / Get-PSDrive)
- [ ] `ok: false` if running without admin and error issue is present
- [ ] `ok: true` when admin + all binaries present + > 5GB free

**Fail criteria**:
- `freeSpaceMB: Infinity` on a machine where disk space is accessible
- `adminPrivileges: true` when running as a normal user
- Binaries report `true` when they are not installed

---

## Post-Run Log Collection

After all scenarios, collect:
```bash
# macOS/Linux
cp ~/Library/Application\ Support/macOS-OneClick/app.log ./test-results-app.log
cp ~/Library/Application\ Support/macOS-OneClick/operations.log ./test-results-ops.log

# Windows
copy %APPDATA%\macOS-OneClick\app.log test-results-app.log
copy %APPDATA%\macOS-OneClick\operations.log test-results-ops.log
```

Attach both log files to the test report.

---

## Sign-Off

| Scenario | Status | Notes |
|----------|--------|-------|
| 1. System Disk Guard | | |
| 2. MBR Guard + Modal | | |
| 3. Successful USB Flash | | |
| 4. EFI Integrity | | |
| 5. Cancellation | | |
| 6. Partition Detection | | |
| 7. Preflight | | |

**Tester sign-off**: ___________________  **Date**: ___________

**Blocking issues found**: yes / no

If yes, file a GitHub issue before releasing.
