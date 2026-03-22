import { describe, it, expect } from 'vitest';
import { structureError } from '../src/lib/structuredErrors.js';

describe('structureError — Windows flash path errors', () => {
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
