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
    test: m => m.includes('INSUFFICIENT_SPACE') || m.includes('capacity, but this operation requires'),
    structured: {
      title: 'Insufficient USB capacity',
      what: 'The selected USB drive is too small for the full macOS recovery installer.',
      nextStep: 'Use a larger USB drive (at least 16 GB) or select the "EFI Only" deployment method.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('SAFETY BLOCK') || m.includes('system/boot disk') || m.includes('system disk'),
    structured: {
      title: 'System disk blocked',
      what: 'The selected drive is your main system disk. Flashing it would erase your operating system.',
      nextStep: 'Select a different removable USB drive from the list and try again.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('MBR_PARTITION_TABLE') || m.includes('MBR partition'),
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
    test: m => m.includes('DEVICE_NOT_FOUND') || m.includes('not found') || m.includes('disconnected'),
    structured: {
      title: 'Drive not found',
      what: 'The selected drive could not be found. It may have been disconnected.',
      nextStep: 'Reconnect the drive, click Refresh, then select it again.',
      retryable: true,
    },
  },
  {
    test: m => m.includes('Permission denied') || m.includes('EACCES') || m.includes('EPERM') || m.includes('as Administrator') || m.includes('sudo'),
    structured: {
      title: 'Permission denied',
      what: 'The app does not have permission to write to this drive.',
      nextStep:
        'On Windows, close the app and re-run it as Administrator. On Linux/macOS, re-run with sudo.',
      retryable: false,
    },
  },
  {
    test: m => m.includes('ENOSPC') || m.includes('not enough disk space') || m.includes('disk space'),
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
    test: m => m.includes('Timed out') || m.includes('timed out') || m.includes('timeout'),
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
      what: 'Writing to the USB drive failed. The drive may be faulty or write-protected.',
      nextStep:
        'Try a different USB drive. Make sure it is not write-protected and has at least 16 GB of space.',
      retryable: true,
      retryNote: 'with a different USB drive',
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
