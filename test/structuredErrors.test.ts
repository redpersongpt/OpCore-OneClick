import { describe, it, expect } from 'vitest';
import { structureError } from '../src/lib/structuredErrors.js';

describe('structureError — Windows flash path errors', () => {
  it('classifies flash prepare BIOS blockers separately from write failures', () => {
    const e = structureError("Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: BIOS readiness is no longer satisfied. Re-verify firmware settings before flashing.");
    expect(e.title).toContain('BIOS readiness');
    expect(e.retryable).toBe(true);
  });

  it('classifies flash prepare compatibility blockers separately from write failures', () => {
    const e = structureError("Error invoking remote method 'flash:prepare-confirmation': Error: Compatibility is blocked. Fix the compatibility report before deployment.");
    expect(e.title).toContain('compatibility');
    expect(e.retryable).toBe(true);
  });

  it('classifies missing flash disk identity separately from write failures', () => {
    const e = structureError("Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: No disk identity fingerprint was captured for this selection. Re-select the drive before flashing.");
    expect(e.title).toContain('Disk identity');
    expect(e.retryable).toBe(true);
  });

  it('classifies stalled operations separately from write failures', () => {
    const e = structureError("Error invoking remote method 'flash-usb': Error: Operation stalled: no progress received for 900 seconds. Please check the current step and try again.");
    expect(e.title).toContain('stalled');
    expect(e.retryable).toBe(true);
  });

  it('classifies diskpart partition creation failure', () => {
    const e = structureError('diskpart failed to create a partition on disk 2');
    expect(e.title).toContain('partition creation');
    expect(e.retryable).toBe(true);
  });

  it('classifies diskpart format failure separately', () => {
    const e = structureError('diskpart created a partition on disk 2, but failed to format it as FAT32 OPENCORE');
    expect(e.title).toContain('format');
    expect(e.retryable).toBe(true);
  });

  it('classifies diskpart general preparation failure', () => {
    const e = structureError('diskpart could not prepare disk 3');
    expect(e.title).toContain('Windows disk preparation');
  });

  it('classifies drive letter assignment failure', () => {
    const e = structureError('Disk 2 has a partition but Windows did not assign a drive letter to it');
    expect(e.title).toContain('drive letter');
    expect(e.retryable).toBe(true);
  });

  it('classifies OPENCORE label lookup failure distinctly', () => {
    const e = structureError('Disk 2 has a FAT32 partition with a drive letter, but the OPENCORE label could not be confirmed');
    expect(e.title).toContain('OPENCORE volume lookup');
    expect(e.retryable).toBe(true);
  });

  it('classifies unknown partition table', () => {
    const e = structureError('Cannot read partition table for disk 1');
    expect(e.title).toContain('partition table');
    expect(e.retryable).toBe(true);
  });

  it('classifies MBR partition table', () => {
    const e = structureError('Device has an MBR partition table');
    expect(e.title).toContain('MBR');
    expect(e.retryable).toBe(true);
  });

  it('classifies system disk block', () => {
    const e = structureError('SAFETY BLOCK: this is your system disk');
    expect(e.title).toContain('System disk');
    expect(e.retryable).toBe(true);
  });
});

describe('structureError — error classification precedence', () => {
  it('partition-table error is distinct from system-disk error', () => {
    const ptError = structureError('cannot read partition table');
    const sysError = structureError('safety block: system/boot disk');
    expect(ptError.title).not.toBe(sysError.title);
  });

  it('diskpart error is distinct from generic flash error', () => {
    const dpError = structureError('diskpart failed to create a partition on disk 2');
    const flashError = structureError('flash write failed on /dev/sdb');
    expect(dpError.title).not.toBe(flashError.title);
  });

  it('format failure is distinct from partition creation failure', () => {
    const createError = structureError('diskpart failed to create a partition on disk 2');
    const formatError = structureError('diskpart created a partition on disk 2, but failed to format it as FAT32 OPENCORE');
    expect(createError.title).not.toBe(formatError.title);
  });

  it('returns a generic fallback for unknown errors', () => {
    const e = structureError('something completely unexpected happened');
    expect(e.title).toBe('An error occurred');
    expect(e.retryable).toBe(true);
  });
});

describe('structureError — EFI and build errors', () => {
  it('classifies EFI build failure', () => {
    const e = structureError('EFI build failed: missing OpenCore.efi');
    expect(e.title).toContain('EFI build');
  });

  it('classifies SMBIOS incompatibility', () => {
    const e = structureError('SMBIOS iMac19,1 is incompatible with Tahoe');
    expect(e.title).toContain('SMBIOS');
  });

  it('classifies insufficient space', () => {
    const e = structureError('Device has 4 GB capacity, but this operation requires 16 GB. insufficient_space');
    expect(e.title).toContain('Insufficient');
  });
});

describe('structureError — recovery and download errors', () => {
  it('classifies Apple recovery rejection', () => {
    const e = structureError('Apple recovery server rejected the request');
    expect(e.title).toContain('Apple rejected');
    expect(e.retryable).toBe(true);
  });

  it('classifies download failure', () => {
    const e = structureError('Download failed at 45%');
    expect(e.title).toContain('Download failed');
    expect(e.retryable).toBe(true);
  });

  it('classifies timeout', () => {
    const e = structureError('Operation timed out after 120s');
    expect(e.title).toContain('timed out');
    expect(e.retryable).toBe(true);
  });
});

describe('structureError — #23 drive letter assignment', () => {
  it('classifies "did not assign a drive letter" with retryable=true', () => {
    const e = structureError('Disk 4 has a FAT32 OPENCORE partition, but Windows did not assign a drive letter to it.');
    expect(e.title).toContain('drive letter');
    expect(e.retryable).toBe(true);
  });
});

describe('structureError — #24 primary data partition detection', () => {
  it('classifies "Could not determine the primary data partition" with retryable=false', () => {
    const e = structureError('Could not determine the primary data partition for disk 2. The disk has no basic data partition of 20 GB or more that qualifies as a shrink target.');
    expect(e.title).toContain('Cannot find data partition');
    expect(e.retryable).toBe(false);
  });
});

describe('structureError — GitHub rate limit', () => {
  it('classifies explicit rate limit message', () => {
    const e = structureError('GitHub API rate limit exceeded — resets at 11:30:00 PM');
    expect(e.title).toContain('rate limit');
    expect(e.retryable).toBe(true);
    expect(e.retryNote).toBeDefined();
  });

  it('classifies GitHub 403 as rate limit', () => {
    const e = structureError('GitHub API HTTP 403 for acidanthera/Lilu');
    expect(e.title).toContain('rate limit');
    expect(e.retryable).toBe(true);
  });

  it('rate limit is distinct from generic download failure', () => {
    const rl = structureError('GitHub API rate limit exceeded — resets at 12:00:00 AM');
    const dl = structureError('Download failed at 45%');
    expect(rl.title).not.toBe(dl.title);
  });
});

describe('structureError — kext unavailability', () => {
  it('classifies kext fetch failure with kext name in message', () => {
    const e = structureError('Failed to fetch WhateverGreen.kext: connection refused | No embedded fallback for WhateverGreen.kext — internet access required (acidanthera/WhateverGreen)');
    expect(e.title).toContain('Kext unavailable');
    expect(e.retryable).toBe(true);
  });

  it('classifies "no embedded fallback" for a named kext', () => {
    const e = structureError('No embedded fallback for CPUTopologyRebuild.kext — internet access required (b00t0x/CpuTopologyRebuild)');
    expect(e.title).toContain('Kext unavailable');
    expect(e.retryable).toBe(true);
  });

  it('classifies "no usable release asset" for a kext', () => {
    const e = structureError('No usable release asset was found for NootedRed.kext.');
    expect(e.title).toContain('Kext unavailable');
    expect(e.retryable).toBe(true);
  });

  it('kext unavailable is distinct from generic download failure', () => {
    const kext = structureError('Failed to fetch Lilu.kext: connection refused');
    const dl = structureError('Download failed at 45%');
    expect(kext.title).not.toBe(dl.title);
  });
});

describe('structureError — download failed message update', () => {
  it('classifies generic download failure with updated message', () => {
    const e = structureError('Download failed at 45%');
    expect(e.title).toBe('Download failed');
    expect(e.retryable).toBe(true);
  });

  it('download failed message covers recovery context', () => {
    const e = structureError('Download failed at 45%');
    expect(e.nextStep).toContain('internet connection');
  });
});

describe('structureError — #36 required kext fail-fast', () => {
  it('classifies required kext unavailable from the fail-fast error', () => {
    const e = structureError(
      'Required kext unavailable: NootedRed.kext — GitHub API fetch failed: connection refused | ' +
      'No embedded fallback for NootedRed.kext — internet access required (ChefKissInc/NootedRed). ' +
      'No embedded fallback exists for NootedRed.kext. Internet access required to download this kext from GitHub.',
    );
    expect(e.title).toContain('Kext unavailable');
    expect(e.retryable).toBe(true);
  });

  it('NootedRed-like kext failure does not match hardware scan', () => {
    const e = structureError(
      'Required kext unavailable: NootedRed.kext — No embedded fallback for NootedRed.kext',
    );
    expect(e.title).not.toContain('Hardware scan');
  });
});

describe('structureError — #37B flash-usb cannot surface hardware_scan_failed', () => {
  it('flash error containing "hardware" does not match hardware scan', () => {
    const e = structureError('flash write failed: hardware not responding on /dev/sdb');
    expect(e.title).not.toBe('Hardware scan failed');
  });

  it('flash error containing "scan" does not match hardware scan', () => {
    const e = structureError('diskpart scanning partition table failed during flash-usb');
    expect(e.title).not.toBe('Hardware scan failed');
  });

  it('actual hardware scan error still matches', () => {
    const e = structureError('hardware scan could not complete — system query error');
    expect(e.title).toBe('Hardware scan failed');
    expect(e.retryable).toBe(true);
  });

  it('hardware detection failure still matches', () => {
    const e = structureError('hardware detection failed on this system');
    expect(e.title).toBe('Hardware scan failed');
  });

  it('stale error with "hardware" and "usb" does not match hardware scan', () => {
    const e = structureError('hardware write error on usb device');
    expect(e.title).not.toBe('Hardware scan failed');
  });
});

describe('structureError — #37C compound format failure', () => {
  it('classifies compound diskpart + Format-Volume failure', () => {
    const e = structureError(
      'diskpart created a partition on disk 2, but failed to format it as FAT32 OPENCORE. ' +
      'Both diskpart inline format and PowerShell Format-Volume fallback failed.',
    );
    expect(e.title).toContain('format');
    expect(e.retryable).toBe(true);
  });

  it('stage-annotated generic failure is classified', () => {
    const e = structureError('diskpart could not prepare disk 3 (stage: format). Something went wrong.');
    expect(e.title).toContain('disk preparation');
    expect(e.retryable).toBe(true);
  });
});
