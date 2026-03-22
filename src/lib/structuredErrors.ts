// ── Structured error types ─────────────────────────────────────────────────────

export interface StructuredError {
  title: string;
  what: string;       // what happened — one sentence
  nextStep: string;   // what the user should do
  retryable: boolean;
  retryNote?: string; // e.g. "only after reconnecting the drive"
}

// Known error message fragments mapped to structured errors.
// Checked in order — first match wins.
const ERROR_MAP: Array<{
  test: (msg: string) => boolean;
  structured: StructuredError;
}> = [
  {
    test: m => m.includes('bios_recheck_failed') || m.includes('bios recheck failed'),
    structured: {
      title: 'BIOS recheck failed',
      what: 'The app could not refresh the firmware checklist from the current machine state.',
      nextStep: 'Stay on the BIOS step, review the checklist manually, and try Recheck BIOS again.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('bios_state_unavailable') || m.includes('bios state is unavailable'),
    structured: {
      title: 'BIOS state is unavailable',
      what: 'The current BIOS checklist is missing or stale, so the app cannot continue from this step safely.',
      nextStep: 'Use Recheck BIOS to rebuild the checklist for this hardware session.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('bios_requirements_not_met') || m.includes('required bios setting') || m.includes('bios preparation is incomplete'),
    structured: {
      title: 'BIOS settings still need attention',
      what: 'One or more required BIOS settings are still missing or unverified.',
      nextStep: 'Fix the failed or unknown BIOS items in firmware, then use Recheck BIOS to confirm them.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('bios_continue_blocked') || m.includes('could not continue from the bios step'),
    structured: {
      title: 'BIOS continue is blocked',
      what: 'The current BIOS checklist could not advance to EFI build from this state.',
      nextStep: 'Stay on the BIOS step, review the blocking reason, and use Continue again only after the prerequisite is resolved.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('build_blocked_by_guard') || m.includes('efi build is blocked'),
    structured: {
      title: 'EFI build is blocked',
      what: 'A build guard stopped EFI generation before it started.',
      nextStep: 'Fix the blocking prerequisite shown in the report or BIOS step, then retry the EFI build.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('build_ipc_failed') || m.includes('efi build failed'),
    structured: {
      title: 'EFI build failed',
      what: 'The EFI build IPC path returned a concrete runtime failure.',
      nextStep: 'Review the reported build error, correct the blocker, then retry the EFI build once.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('no supported display path') || m.includes('val_gpu_no_supported_path'),
    structured: {
      title: 'No supported display path',
      what: 'macOS has no supported GPU output path on this machine.',
      nextStep: 'Use a supported Intel iGPU or supported AMD GPU, or disable the unsupported dGPU only if another supported output path remains.',
      retryable: false,
    },
  },
  {
    test: m => m.includes('unsupported amd laptop') || m.includes('amd laptop'),
    structured: {
      title: 'Unsupported AMD laptop',
      what: 'This AMD laptop path is not a canonical supported Hackintosh target.',
      nextStep: 'Do not continue with this hardware unless you have a documented limited-support path and are prepared for manual work.',
      retryable: false,
    },
  },
  {
    test: m => m.includes('smbios') && (m.includes('invalid') || m.includes('incompatible') || m.includes('compat')),
    structured: {
      title: 'Incompatible SMBIOS',
      what: 'The selected SMBIOS does not match the hardware or the target macOS installer.',
      nextStep: 'Choose a supported SMBIOS for the CPU generation and display path instead of relying on compatibility bypasses.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('openruntime') && m.includes('mismatch'),
    structured: {
      title: 'OpenRuntime mismatch',
      what: 'OpenRuntime.efi does not match the rest of the OpenCore binary set.',
      nextStep: 'Rebuild the EFI from one consistent OpenCore release so BOOTx64.efi, OpenCore.efi, and OpenRuntime.efi match.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('driver missing') || m.includes('val_driver_missing') || m.includes('missing_file'),
    structured: {
      title: 'EFI integrity failure',
      what: 'Required OpenCore files or drivers are missing from the generated EFI.',
      nextStep: 'Rebuild the EFI and confirm the missing component is present before flashing or booting.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('airportitlwm') && m.includes('secureboot'),
    structured: {
      title: 'AirportItlwm requires SecureBootModel',
      what: 'AirportItlwm was selected without a SecureBootModel-enabled path.',
      nextStep: 'Enable SecureBootModel for AirportItlwm, or switch to Itlwm and accept that Recovery Wi-Fi will not work.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('insufficient_space') || m.includes('capacity, but this operation requires'),
    structured: {
      title: 'Insufficient USB capacity',
      what: 'The selected USB drive is too small for the full macOS recovery installer.',
      nextStep: 'Use a larger USB drive (at least 16 GB) or select the "EFI Only" deployment method.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('diskpart failed to create'),
    structured: {
      title: 'Windows partition creation failed',
      what: 'diskpart did not leave a visible partition on the selected USB drive.',
      nextStep: 'Close all programs using this drive, unplug and reconnect it, then try again. If it keeps failing, use diskpart or Disk Management to create a GPT primary partition before retrying.',
      retryable: true,
      retryNote: 'after reconnecting the drive',
    },
  },
  {
    test: m => m.includes('failed to format it as fat32 opencore'),
    structured: {
      title: 'Windows partition format failed',
      what: 'diskpart created a partition, but Windows did not report it as a FAT32 OPENCORE volume.',
      nextStep: 'Close all programs using this drive, unplug and reconnect it, then try again. If it keeps failing, try a different USB drive or use Disk Management to manually prepare a GPT FAT32 partition labeled OPENCORE.',
      retryable: true,
      retryNote: 'after reconnecting the drive',
    },
  },
  {
    test: m => m.includes('did not assign a drive letter') || m.includes('did not mount the new'),
    structured: {
      title: 'Windows drive letter assignment failed',
      what: 'The partition was created but Windows could not assign a drive letter to it.',
      nextStep: 'Unplug the drive, wait 5 seconds, reconnect it, and retry. If it persists, open Disk Management and manually assign a letter to the OPENCORE partition.',
      retryable: true,
      retryNote: 'after reconnecting the drive',
    },
  },
  {
    test: m => m.includes('opencore label could not be confirmed'),
    structured: {
      title: 'Windows OPENCORE volume lookup failed',
      what: 'The new FAT32 partition exists, but the OPENCORE label could not be confirmed on it.',
      nextStep: 'Rename the partition to OPENCORE in Disk Management, then retry the flash.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('diskpart could not prepare'),
    structured: {
      title: 'Windows disk preparation failed',
      what: 'diskpart could not finish preparing the selected USB drive.',
      nextStep: 'Close all programs using this drive, unplug and reconnect it, then try again. If it keeps failing, try a different USB drive or manually prepare the GPT FAT32 OPENCORE partition first.',
      retryable: true,
      retryNote: 'after reconnecting the drive',
    },
  },
  {
    test: m => m.includes('could not determine the primary data partition'),
    structured: {
      title: 'Cannot find data partition to shrink',
      what: 'The app could not identify a qualifying data partition on the selected disk.',
      nextStep: 'Make sure the correct disk is selected, that it has a main Windows installation partition (C:) larger than 20 GB, and that it is not an EFI, MSR, or recovery-only disk. Open Disk Management to verify the layout.',
      retryable: false,
    },
  },
  {
    test: m => m.includes('cannot read partition table') || m.includes('unknown_partition_table'),
    structured: {
      title: 'Cannot read partition table',
      what: 'The app could not determine whether the selected drive uses GPT or MBR.',
      nextStep: 'Reconnect the drive, click Refresh, and if needed reformat it as GPT before retrying the operation.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('uses mbr partition table') || m.includes('mbr_partition_table') || m.includes('mbr partition'),
    structured: {
      title: 'MBR partition table',
      what: 'The drive uses an MBR partition table. OpenCore requires GPT.',
      nextStep:
        'Reformat the drive as GPT using Disk Utility (macOS), GParted (Linux), or diskpart (Windows), then retry.',
      retryable: true,
      retryNote: 'after reformatting the drive as GPT',
    },
  },
  {
    test: m => m.includes('safety block') || m.includes('system/boot disk') || m.includes('system disk'),
    structured: {
      title: 'System disk blocked',
      what: 'The selected drive is your main system disk. Flashing it would erase your operating system.',
      nextStep: 'Select a different removable USB drive from the list and try again.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('device_not_found') || m.includes('not found') || m.includes('disconnected'),
    structured: {
      title: 'Drive not found',
      what: 'The selected drive could not be found. It may have been disconnected.',
      nextStep: 'Reconnect the drive, click Refresh, then select it again.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('permission denied') || m.includes('eacces') || m.includes('eperm') || m.includes('as administrator') || m.includes('sudo'),
    structured: {
      title: 'Permission denied',
      what: 'The app does not have permission to write to this drive.',
      nextStep:
        'On Windows, close the app and re-run it as Administrator. On Linux, install polkit (policykit-1) so the app can elevate disk commands. On macOS, approve the system prompt.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('apple recovery server rejected the request') || m.includes('apple rejected the recovery request'),
    structured: {
      title: 'Apple rejected the recovery request',
      what: 'Apple’s recovery service refused this download request for the selected macOS target.',
      nextStep: 'Try an older macOS version, use manual recovery import, or switch to EFI-only mode if you already have installer media.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('build will fail') || m.includes('pre-build check failed'),
    structured: {
      title: 'Build is blocked by a concrete pre-check',
      what: 'The app found a specific dependency or environment blocker before the EFI build could succeed.',
      nextStep: 'Fix the blocker shown in the report, then retry the build instead of repeating the same build blindly.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('efi build contract failed'),
    structured: {
      title: 'EFI verification failed after generation',
      what: 'The generated EFI did not pass the on-disk integrity contract.',
      nextStep: 'Inspect the missing or failed component named in the report, then rebuild the EFI once the dependency issue is fixed.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('enospc') || m.includes('not enough disk space') || m.includes('disk space'),
    structured: {
      title: 'Not enough disk space',
      what: 'There is not enough free space to complete this operation.',
      nextStep:
        'Free up at least 8 GB of space on your main drive, then retry.',
      retryable: true,
      retryNote: 'after freeing disk space',
    },
  },
  {
    test: m => m.includes('timed out') || m.includes('timeout'),
    structured: {
      title: 'Operation timed out',
      what: 'The operation did not complete within the expected time.',
      nextStep: 'Check your internet connection and try again. If the problem persists, restart the app.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('download') && (m.includes('failed') || m.includes('error')),
    structured: {
      title: 'Download failed',
      what: 'The macOS recovery download was interrupted or failed.',
      nextStep:
        'Click Retry — the download will resume from where it left off. Check your network connection if the error repeats.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('flash') || m.includes('write') || m.includes('dd:'),
    structured: {
      title: 'USB write failed',
      what: 'Writing to the USB drive failed. This can be caused by permissions, connection problems, verification failures, or the drive itself.',
      nextStep:
        'Retry once. If it fails again, check permissions, reconnect the drive, and review the exact write error before replacing the USB drive.',
      retryable: true,
      retryNote: 'after checking the exact write error',
    },
  },
  {
    test: m => m.includes('scan') || m.includes('hardware'),
    structured: {
      title: 'Hardware scan failed',
      what: 'The hardware scan could not complete. A system query returned an error.',
      nextStep: 'Click Retry. If the error persists, check that system management tools are working correctly.',
      retryable: true,
    },
  },
];

/**
 * Convert a raw error string or Error object into a StructuredError.
 * Falls back to a generic structure if no pattern matches.
 */
export function structureError(raw: string | Error): StructuredError {
  const msg = raw instanceof Error ? raw.message : String(raw ?? '');
  const lower = msg.toLowerCase();

  for (const entry of ERROR_MAP) {
    if (entry.test(lower)) return entry.structured;
  }

  // Generic fallback
  return {
    title: 'An error occurred',
    what: msg.length > 0 ? msg : 'An unexpected error occurred.',
    nextStep: 'Restart the app and try again. If the problem persists, use the "Copy Diagnostics" button to report the issue.',
    retryable: true,
  };
}
