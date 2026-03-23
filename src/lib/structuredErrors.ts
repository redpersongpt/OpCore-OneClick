import {
  isFlashPrepareBiosBlockedMessage,
  isFlashPrepareCompatibilityBlockedMessage,
  isFlashPrepareDiskIdentityBlockedMessage,
  isFlashPrepareEfiValidationBlockedMessage,
  isFlashPrepareSelectedDiskMissingMessage,
  isGenericFlashPrepareBlockedMessage,
  normalizeErrorMessage,
  normalizeErrorMessageLower,
} from './errorMessage.js';

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
    test: m => isFlashPrepareBiosBlockedMessage(m),
    structured: {
      title: 'Flashing is blocked by BIOS readiness',
      what: 'The app rechecked the firmware prerequisites immediately before flashing, and they are no longer satisfied.',
      nextStep: 'Return to the BIOS step, fix or recheck the required firmware settings, then reopen the flash confirmation dialog.',
      retryable: true,
    },
  },
  {
    test: m => isFlashPrepareCompatibilityBlockedMessage(m),
    structured: {
      title: 'Flashing is blocked by compatibility',
      what: 'The selected macOS target is no longer deployable from the current compatibility state.',
      nextStep: 'Return to the report step, fix the compatibility blocker, then reopen the flash confirmation dialog.',
      retryable: true,
    },
  },
  {
    test: m => isFlashPrepareSelectedDiskMissingMessage(m),
    structured: {
      title: 'No target disk selected',
      what: 'The flash confirmation step was reached without a selected target disk.',
      nextStep: 'Go back to USB selection, refresh the drive list, and select the target drive again before flashing.',
      retryable: true,
    },
  },
  {
    test: m => isFlashPrepareDiskIdentityBlockedMessage(m),
    structured: {
      title: 'Disk identity is missing',
      what: 'The app could not confirm the physical identity of the selected drive at the destructive flash boundary.',
      nextStep: 'Reconnect the drive, re-select it, wait for the details to load, then reopen the flash confirmation dialog.',
      retryable: true,
    },
  },
  {
    test: m => isFlashPrepareEfiValidationBlockedMessage(m),
    structured: {
      title: 'Flashing is blocked by EFI validation',
      what: 'The app rechecked the EFI before flashing and it is no longer clean enough for deployment.',
      nextStep: 'Return to the report or build step, rebuild or revalidate the EFI, then reopen the flash confirmation dialog.',
      retryable: true,
    },
  },
  {
    test: m => isGenericFlashPrepareBlockedMessage(m),
    structured: {
      title: 'Flash preparation is blocked',
      what: 'A destructive safety check stopped the flash step before any USB write started.',
      nextStep: 'Return to the step named in the blocker, fix that prerequisite, then reopen the flash confirmation dialog.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('operation stalled') || m.includes('no progress received for 60 seconds'),
    structured: {
      title: 'Operation stalled',
      what: 'A long-running background task stopped reporting progress before the app could confirm the final state.',
      nextStep: 'Retry the interrupted step once. If it stalls again, copy the diagnostics instead of retrying blindly.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('was cancelled'),
    structured: {
      title: 'Operation cancelled',
      what: 'The current task was cancelled before the app could finish or confirm the final on-disk state.',
      nextStep: 'Retry the interrupted step once. If the same cancellation repeats, copy the diagnostics and report it.',
      retryable: true,
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
    test: m => m.includes('format-volume') && m.includes('also failed'),
    structured: {
      title: 'Windows format failed (both methods)',
      what: 'diskpart created a partition, but both diskpart inline format and PowerShell Format-Volume fallback failed to produce a FAT32 OPENCORE volume.',
      nextStep: 'Close all programs using this drive, unplug and reconnect it, then try again. If it keeps failing, open Disk Management and manually format partition 1 as FAT32 with the label OPENCORE.',
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
    test: m => m.includes('rate limit') || (m.includes('github') && (m.includes('403') || m.includes('429'))),
    structured: {
      title: 'GitHub API rate limit',
      what: 'The GitHub API rate limit was hit while fetching kext release metadata.',
      nextStep: 'Wait for the rate limit to reset (the exact reset time is shown in the build log), then retry the EFI build.',
      retryable: true,
      retryNote: 'after the GitHub rate limit resets',
    },
  },
  {
    test: m => m.includes('.kext') && (m.includes('failed to fetch') || m.includes('no embedded fallback') || m.includes('no usable release asset') || m.includes('internet access required')),
    structured: {
      title: 'Kext unavailable',
      what: 'A required kext could not be fetched from GitHub or any fallback source.',
      nextStep: 'Ensure you have internet access and retry. The specific kext name and its upstream repository are shown in the build log.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('download') && (m.includes('failed') || m.includes('error')),
    structured: {
      title: 'Download failed',
      what: 'A file download was interrupted or failed.',
      nextStep:
        'Check your internet connection and try again. If this is a macOS recovery download, it can be resumed by retrying.',
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
    test: m =>
      (m.includes('hardware scan') || m.includes('hardware detection') || m.includes('hardware_scan_failed'))
      && !m.includes('flash')
      && !m.includes('diskpart')
      && !m.includes('format')
      && !m.includes('partition')
      && !m.includes('write')
      && !m.includes('usb'),
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
  const msg = normalizeErrorMessage(raw);
  const lower = normalizeErrorMessageLower(raw);

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
