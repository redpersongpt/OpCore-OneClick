// ── Remediation guide ─────────────────────────────────────────────────────────
// Plain-language explanations and fix instructions for every blocked or warned
// state in the application. Keyed by error/block code so UI can look up copy
// without embedding prose in component logic.

export type RemediationCode =
  | 'SYSTEM_DISK'
  | 'MBR_PARTITION_TABLE'
  | 'UNKNOWN_PARTITION_TABLE'
  | 'NO_ADMIN'
  | 'FIRMWARE_UNVERIFIED'
  | 'NETWORK_UNAVAILABLE'
  | 'UNSUPPORTED_HOST'
  | 'COMPATIBILITY_MODE'
  | 'DOWNLOAD_FAILED'
  | 'DISK_WRITE_FAILED'
  | 'NOT_REMOVABLE'
  | 'DEVICE_TOO_SMALL'
  | 'LOW_DISK_SPACE'
  | 'MISSING_BINARY'
  | 'FIRMWARE_FAILING'
  | 'NO_USB_DETECTED'
  | 'SCAN_TIMEOUT'
  | 'PRECHECK_TIMEOUT';

export interface Remediation {
  /** Short headline — shown inline next to blocked item. */
  shortLabel: string;
  /** One-paragraph explanation of what is wrong and why. */
  explanation: string;
  /** Concrete steps the user can take to fix this. */
  howToFix: string;
}

export const REMEDIATION_GUIDE: Record<RemediationCode, Remediation> = {
  SYSTEM_DISK: {
    shortLabel: 'System drive — cannot flash',
    explanation:
      'This drive appears to be your main system disk (the one your operating system is installed on). Flashing this drive would erase your OS and all your data.',
    howToFix:
      'Select a removable USB drive instead. If no USB drives appear in the list, insert a USB drive of 16 GB or larger and click Refresh.',
  },
  MBR_PARTITION_TABLE: {
    shortLabel: 'MBR partition table — must convert',
    explanation:
      'This drive uses an MBR (Master Boot Record) partition table. OpenCore requires a GPT (GUID Partition Table) structure to boot successfully.',
    howToFix:
      'Convert the disk to GPT using your system\'s disk utility (this will erase all data), then retry. Or select a different drive.',
  },
  UNKNOWN_PARTITION_TABLE: {
    shortLabel: 'Cannot read partition table',
    explanation:
      "The drive's partition structure could not be read. This can happen with brand-new drives, drives formatted with an unusual scheme, or drives that are faulty.",
    howToFix:
      'Reconnect the drive and click Refresh. If the problem persists, reformat the drive as GPT using Disk Utility (macOS), GParted (Linux), or diskpart (Windows), then try again.',
  },
  NO_ADMIN: {
    shortLabel: 'Administrator privileges required',
    explanation:
      'Writing to disks requires administrator access. The app does not currently have the necessary permissions.',
    howToFix:
      'On Windows, right-click the application and choose "Run as administrator". On Linux/macOS, re-run the app with elevated permissions (sudo).',
  },
  FIRMWARE_UNVERIFIED: {
    shortLabel: 'BIOS settings unverified',
    explanation:
      'One or more BIOS/UEFI settings could not be automatically detected. The app cannot confirm these settings are correct without firmware-level access.',
    howToFix:
      "Check each listed setting manually in your target PC's BIOS/UEFI firmware. Restart the target PC, press Del, F2, or F12 during POST to enter BIOS, and verify each setting listed on the BIOS Setup screen.",
  },
  NETWORK_UNAVAILABLE: {
    shortLabel: 'No internet connection',
    explanation:
      'No internet connection was detected. The macOS recovery files must be downloaded from Apple servers and cannot be packaged with the app.',
    howToFix:
      'Connect this machine to the internet using a wired Ethernet connection (preferred) or Wi-Fi, then click Retry.',
  },
  UNSUPPORTED_HOST: {
    shortLabel: 'Host OS not fully supported',
    explanation:
      'This host operating system is not in the fully tested configuration. Some hardware detection or disk operations may not work correctly.',
    howToFix:
      'For best results, run this app on Windows 10/11 or Ubuntu 22.04+. You can continue, but some features may be limited.',
  },
  COMPATIBILITY_MODE: {
    shortLabel: 'Running in compatibility mode',
    explanation:
      "Your system is running in compatibility mode due to a detected hardware or driver issue. Some hardware detection features may be limited.",
    howToFix:
      'Update your graphics drivers or run on a system with a supported GPU. You can continue — most features still work in this mode.',
  },
  DOWNLOAD_FAILED: {
    shortLabel: 'Download interrupted',
    explanation:
      'The macOS recovery download was interrupted. This is often caused by a lost network connection or a temporary server error.',
    howToFix:
      'Click Retry — the download will resume from where it left off if the partial file is still present. If retry fails repeatedly, check your internet connection.',
  },
  DISK_WRITE_FAILED: {
    shortLabel: 'USB write failed',
    explanation:
      'Writing to the USB drive failed part-way through. This may indicate a faulty drive, a write-protected drive, or a permission problem.',
    howToFix:
      'Try a different USB drive. Make sure the drive is not write-protected (check for a physical lock switch). On Windows, re-run as Administrator.',
  },
  NOT_REMOVABLE: {
    shortLabel: 'Internal drive — cannot flash',
    explanation:
      'This drive is identified as an internal drive, not a removable USB device. Only external USB drives are allowed for flashing.',
    howToFix:
      'Use a removable USB drive of 16 GB or larger. Insert one and click Refresh to see it in the list.',
  },
  DEVICE_TOO_SMALL: {
    shortLabel: 'Drive too small (need ≥ 16 GB)',
    explanation:
      'The macOS recovery image and OpenCore EFI require at least 16 GB of space on the USB drive.',
    howToFix: 'Use a USB drive with at least 16 GB of storage.',
  },
  LOW_DISK_SPACE: {
    shortLabel: 'Low free space on this machine',
    explanation:
      'The recovery download and EFI build require significant free space on the host machine (where this app is running), separate from the USB drive.',
    howToFix:
      'Free up at least 8 GB of space on your main drive. Delete temporary files, empty the trash, or move large files to external storage.',
  },
  MISSING_BINARY: {
    shortLabel: 'Required tool not found',
    explanation:
      'A required system utility is missing. The app uses platform tools like diskutil, diskpart, and dd for safe disk operations.',
    howToFix:
      'On macOS, these tools are built in. On Linux, install parted and dosfstools (sudo apt install parted dosfstools). On Windows, diskpart is built into the OS.',
  },
  FIRMWARE_FAILING: {
    shortLabel: 'BIOS setting needs to be changed',
    explanation:
      'A required BIOS/UEFI setting is currently set to a value that will prevent macOS from booting. This must be fixed before continuing.',
    howToFix:
      "Reboot into your target PC's BIOS. The required change and where to find it are shown on the BIOS Setup screen. Save and exit BIOS, then return here.",
  },
  NO_USB_DETECTED: {
    shortLabel: 'No USB drives detected',
    explanation:
      'No removable USB drives were found. The app only lists drives that are confirmed removable to reduce the risk of flashing the wrong device.',
    howToFix:
      'Plug in a USB drive of 16 GB or larger and click Refresh. If the drive still does not appear, try a different USB port.',
  },
  SCAN_TIMEOUT: {
    shortLabel: 'Hardware scan timed out',
    explanation:
      'The hardware scan did not complete within the expected time. This can happen on machines with many peripherals or slow system queries.',
    howToFix:
      'Click Retry to scan again. If the problem persists, check that system management tools (WMI on Windows, lshw on Linux) are functioning.',
  },
  PRECHECK_TIMEOUT: {
    shortLabel: 'System check timed out',
    explanation:
      'The system check did not complete within the expected time. One or more checks did not return a result.',
    howToFix:
      'Click Retry. If a specific check is slow, check whether your antivirus or security software is blocking system queries.',
  },
};
