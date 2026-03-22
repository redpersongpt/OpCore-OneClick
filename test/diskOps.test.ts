import { describe, it, expect } from 'vitest';
import {
  assessWindowsFlashPreparationState,
  buildWindowsPhysicalDrivePath,
  isWindowsUsbLikeDisk,
  getWindowsFat32PartitionSizeMB,
  shouldRetryWindowsFlashPreparation,
  buildLinuxFirstPartitionPath,
  buildWindowsFlashDiskpartScript,
  buildWindowsBootPartitionDiskpartScript,
  windowsGetDiskStyleOutput,
  windowsWmiDiskStyleOutput,
  selectWindowsPrimaryDataPartition,
  buildWindowsAssignLetterDiskpartScript,
  selectWindowsOpencoreReuseCandidate,
} from '../electron/diskOps.js';

// ─── Windows disk path normalization ─────────────────────────────────────────

describe('buildWindowsPhysicalDrivePath', () => {
  it('normalizes disk0 alias to PhysicalDrive0', () => {
    expect(buildWindowsPhysicalDrivePath('disk0')).toBe('\\\\.\\PhysicalDrive0');
  });

  it('normalizes disk2 alias (case-insensitive)', () => {
    expect(buildWindowsPhysicalDrivePath('Disk2')).toBe('\\\\.\\PhysicalDrive2');
  });

  it('normalizes full PhysicalDrive path', () => {
    expect(buildWindowsPhysicalDrivePath('\\\\.\\PhysicalDrive3')).toBe('\\\\.\\PhysicalDrive3');
  });

  it('normalizes bare number', () => {
    expect(buildWindowsPhysicalDrivePath('1')).toBe('\\\\.\\PhysicalDrive1');
  });

  it('normalizes number as integer', () => {
    expect(buildWindowsPhysicalDrivePath(4)).toBe('\\\\.\\PhysicalDrive4');
  });

  it('handles whitespace', () => {
    expect(buildWindowsPhysicalDrivePath('  disk5  ')).toBe('\\\\.\\PhysicalDrive5');
  });
});

// ─── Windows USB detection ───────────────────────────────────────────────────

describe('isWindowsUsbLikeDisk', () => {
  it('detects BusType USB', () => {
    expect(isWindowsUsbLikeDisk({ busType: 'USB' })).toBe(true);
  });

  it('detects InterfaceType USB', () => {
    expect(isWindowsUsbLikeDisk({ interfaceType: 'USB' })).toBe(true);
  });

  it('detects USBSTOR in PNPDeviceID', () => {
    expect(isWindowsUsbLikeDisk({ pnpDeviceId: 'USBSTOR\\DISK&VEN_KINGSTON' })).toBe(true);
  });

  it('detects USB\\ prefix in PNPDeviceID', () => {
    expect(isWindowsUsbLikeDisk({ pnpDeviceId: 'USB\\VID_0781&PID_5583' })).toBe(true);
  });

  it('detects REMOVABLE in MediaType', () => {
    expect(isWindowsUsbLikeDisk({ mediaType: 'Removable Media' })).toBe(true);
  });

  it('rejects system boot disk even with USB bus', () => {
    expect(isWindowsUsbLikeDisk({ busType: 'USB', isBoot: true })).toBe(false);
  });

  it('rejects system disk even with USB bus', () => {
    expect(isWindowsUsbLikeDisk({ busType: 'USB', isSystem: true })).toBe(false);
  });

  it('rejects NVMe internal disk', () => {
    expect(isWindowsUsbLikeDisk({ busType: 'NVMe', interfaceType: 'SCSI' })).toBe(false);
  });

  it('rejects SATA internal disk', () => {
    expect(isWindowsUsbLikeDisk({ busType: 'SATA', interfaceType: 'IDE' })).toBe(false);
  });

  it('handles all null/undefined fields', () => {
    expect(isWindowsUsbLikeDisk({})).toBe(false);
  });

  it('case-insensitive matching', () => {
    expect(isWindowsUsbLikeDisk({ busType: 'usb' })).toBe(true);
  });
});

// ─── FAT32 partition size cap ────────────────────────────────────────────────

describe('getWindowsFat32PartitionSizeMB', () => {
  it('returns 30000 MB for 64 GB drive', () => {
    expect(getWindowsFat32PartitionSizeMB(64_000_000_000)).toBe(30_000);
  });

  it('returns 30000 MB for 256 GB drive', () => {
    expect(getWindowsFat32PartitionSizeMB(256_000_000_000)).toBe(30_000);
  });

  it('returns undefined for 16 GB drive (use full size)', () => {
    expect(getWindowsFat32PartitionSizeMB(16_000_000_000)).toBeUndefined();
  });

  it('returns undefined for exactly 32 GB (boundary)', () => {
    expect(getWindowsFat32PartitionSizeMB(32_000_000_000)).toBeUndefined();
  });

  it('returns 30000 for just over 32 GB', () => {
    expect(getWindowsFat32PartitionSizeMB(32_000_000_001)).toBe(30_000);
  });
});

// ─── Diskpart retry logic ────────────────────────────────────────────────────

describe('shouldRetryWindowsFlashPreparation', () => {
  it('retries on first diskpart failure with no drive letter', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: true, driveLetter: '', stage: 'create-partition',
    })).toBe(true);
  });

  it('does NOT retry on last attempt', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 1, maxAttempts: 2, diskpartFailed: true, driveLetter: '', stage: 'create-partition',
    })).toBe(false);
  });

  it('does NOT retry when diskpart succeeded', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: false, driveLetter: '', stage: 'create-partition',
    })).toBe(false);
  });

  it('does NOT retry when drive letter was assigned', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: true, driveLetter: 'E', stage: 'assign',
    })).toBe(false);
  });
});

// ─── Diskpart script generation ──────────────────────────────────────────────

describe('buildWindowsFlashDiskpartScript', () => {
  it('includes select disk N', () => {
    const script = buildWindowsFlashDiskpartScript('3');
    expect(script).toContain('select disk 3');
  });

  it('creates full-size partition when no sizeMB', () => {
    const script = buildWindowsFlashDiskpartScript('1');
    expect(script).toContain('create partition primary noerr');
    expect(script).not.toContain('size=');
  });

  it('creates capped partition when sizeMB given', () => {
    const script = buildWindowsFlashDiskpartScript('2', 30000);
    expect(script).toContain('create partition primary size=30000 noerr');
  });

  it('converts to GPT', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    expect(script).toContain('convert gpt noerr');
  });

  it('formats as FAT32 with OPENCORE label', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    expect(script).toContain('format fs=fat32 quick label=OPENCORE noerr');
  });

  it('explicitly re-selects partition 1 before format', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    expect(script).toContain('select partition 1 noerr');
  });
});

// ─── Diskpart script input validation ────────────────────────────────────────

describe('buildWindowsFlashDiskpartScript — input validation', () => {
  it('uses partition size cap for large drives', () => {
    const script = buildWindowsFlashDiskpartScript('2', 30000);
    expect(script).toContain('create partition primary size=30000 noerr');
    expect(script).toContain('select disk 2');
  });

  it('uses full partition for small drives', () => {
    const script = buildWindowsFlashDiskpartScript('1');
    expect(script).toContain('create partition primary noerr');
    expect(script).not.toContain('size=');
  });
});

// ─── Diskpart retry logic — partition not created (#11, #15, #16) ───────────

describe('shouldRetryWindowsFlashPreparation — failure modes', () => {
  it('retries when diskpart failed and no drive letter', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: true, driveLetter: '', stage: 'format',
    })).toBe(true);
  });

  it('does NOT retry on last attempt even if diskpart failed', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 1, maxAttempts: 2, diskpartFailed: true, driveLetter: '', stage: 'assign',
    })).toBe(false);
  });

  it('does NOT retry when diskpart succeeded', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: false, driveLetter: '', stage: 'assign',
    })).toBe(false);
  });

  it('does NOT retry when drive letter was found despite error', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: true, driveLetter: 'E', stage: 'label-lookup',
    })).toBe(false);
  });

  it('does NOT retry label lookup failure when a partition fallback exists', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 2, diskpartFailed: true, driveLetter: 'F', stage: 'label-lookup',
    })).toBe(false);
  });
});

// ─── Diskpart script: mandatory commands present ─────────────────────────────

describe('buildWindowsFlashDiskpartScript — safety commands', () => {
  it('always clears readonly first', () => {
    const script = buildWindowsFlashDiskpartScript('1');
    const lines = script.split('\n');
    const selectIdx = lines.findIndex(l => l.includes('select disk'));
    const readonlyIdx = lines.findIndex(l => l.includes('attributes disk clear readonly'));
    expect(readonlyIdx).toBeGreaterThan(selectIdx);
  });

  it('always runs clean before convert', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    const lines = script.split('\n');
    const cleanIdx = lines.findIndex(l => l === 'clean noerr');
    const convertIdx = lines.findIndex(l => l.includes('convert gpt'));
    expect(cleanIdx).toBeLessThan(convertIdx);
  });

  it('always assigns after format', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    const lines = script.split('\n');
    const formatIdx = lines.findIndex(l => l.includes('format fs=fat32'));
    const assignIdx = lines.findIndex(l => l === 'assign noerr');
    expect(assignIdx).toBeGreaterThan(formatIdx);
  });

  it('selects partition 1 between create and format', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    const lines = script.split('\n');
    const createIdx = lines.findIndex(l => l.startsWith('create partition primary'));
    const selectPartitionIdx = lines.findIndex(l => l === 'select partition 1 noerr');
    const formatIdx = lines.findIndex(l => l.includes('format fs=fat32'));
    expect(selectPartitionIdx).toBeGreaterThan(createIdx);
    expect(formatIdx).toBeGreaterThan(selectPartitionIdx);
  });

  it('ends with rescan', () => {
    const script = buildWindowsFlashDiskpartScript('0');
    const lines = script.split('\n').filter(l => l.length > 0);
    expect(lines[lines.length - 1]).toBe('rescan');
  });
});

// ─── Retry logic boundary conditions ─────────────────────────────────────────

describe('shouldRetryWindowsFlashPreparation — boundary conditions', () => {
  it('retries at attempt 0 out of 3', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 3, diskpartFailed: true, driveLetter: '', stage: 'create-partition',
    })).toBe(true);
  });

  it('retries at attempt 1 out of 3', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 1, maxAttempts: 3, diskpartFailed: true, driveLetter: '', stage: 'format',
    })).toBe(true);
  });

  it('does NOT retry at attempt 2 out of 3 (last attempt)', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 2, maxAttempts: 3, diskpartFailed: true, driveLetter: '', stage: 'assign',
    })).toBe(false);
  });

  it('does NOT retry with maxAttempts=1 (single attempt only)', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0, maxAttempts: 1, diskpartFailed: true, driveLetter: '', stage: 'create-partition',
    })).toBe(false);
  });
});

describe('assessWindowsFlashPreparationState', () => {
  it('classifies no partitions as create failure', () => {
    const assessment = assessWindowsFlashPreparationState({
      partitions: [],
      expectedLabel: 'OPENCORE',
    });
    expect(assessment.status).toBe('failed');
    expect(assessment.stage).toBe('create-partition');
  });

  it('classifies raw partition as format failure', () => {
    const assessment = assessWindowsFlashPreparationState({
      partitions: [{
        partitionNumber: 1,
        driveLetter: '',
        fileSystem: '',
        fileSystemLabel: '',
        sizeBytes: 15_264_000_000,
      }],
      expectedLabel: 'OPENCORE',
    });
    expect(assessment.status).toBe('failed');
    expect(assessment.stage).toBe('format');
  });

  it('classifies FAT32 partition without drive letter as assign failure', () => {
    const assessment = assessWindowsFlashPreparationState({
      partitions: [{
        partitionNumber: 1,
        driveLetter: '',
        fileSystem: 'FAT32',
        fileSystemLabel: 'OPENCORE',
        sizeBytes: 15_264_000_000,
      }],
      expectedLabel: 'OPENCORE',
    });
    expect(assessment.status).toBe('failed');
    expect(assessment.stage).toBe('assign');
  });

  it('falls back to the target partition when the label is missing but drive letter exists', () => {
    const assessment = assessWindowsFlashPreparationState({
      partitions: [{
        partitionNumber: 1,
        driveLetter: 'E',
        fileSystem: 'FAT32',
        fileSystemLabel: '',
        sizeBytes: 15_264_000_000,
      }],
      expectedLabel: 'OPENCORE',
    });
    expect(assessment.status).toBe('ready');
    expect(assessment.stage).toBe('label-lookup');
    expect(assessment.driveLetter).toBe('E');
    expect(assessment.usedPartitionFallback).toBe(true);
  });

  it('accepts the expected OPENCORE volume as ready', () => {
    const assessment = assessWindowsFlashPreparationState({
      partitions: [{
        partitionNumber: 1,
        driveLetter: 'F',
        fileSystem: 'FAT32',
        fileSystemLabel: 'OPENCORE',
        sizeBytes: 15_264_000_000,
      }],
      expectedLabel: 'OPENCORE',
    });
    expect(assessment.status).toBe('ready');
    expect(assessment.stage).toBeNull();
    expect(assessment.driveLetter).toBe('F');
  });

  it('targets partition 1 before larger later partitions', () => {
    const assessment = assessWindowsFlashPreparationState({
      partitions: [
        {
          partitionNumber: 2,
          driveLetter: 'G',
          fileSystem: 'FAT32',
          fileSystemLabel: 'OPENCORE',
          sizeBytes: 30_000_000_000,
        },
        {
          partitionNumber: 1,
          driveLetter: '',
          fileSystem: '',
          fileSystemLabel: '',
          sizeBytes: 1_024_000_000,
        },
      ],
      expectedLabel: 'OPENCORE',
    });
    expect(assessment.status).toBe('failed');
    expect(assessment.stage).toBe('format');
    expect(assessment.targetPartitionNumber).toBe(1);
  });
});

describe('buildWindowsBootPartitionDiskpartScript', () => {
  it('explicitly selects partition 1 before formatting BOOTSTRAP', () => {
    const script = buildWindowsBootPartitionDiskpartScript('4');
    expect(script).toContain('select disk 4');
    expect(script).toContain('create partition primary size=16384');
    expect(script).toContain('select partition 1');
    expect(script).toContain('format fs=fat32 quick label=BOOTSTRAP');
    expect(script).toContain('assign');
  });
});

// ─── Linux partition path building ───────────────────────────────────────────

describe('buildLinuxFirstPartitionPath', () => {
  it('appends 1 for sda', () => {
    expect(buildLinuxFirstPartitionPath('/dev/sda')).toBe('/dev/sda1');
  });

  it('appends p1 for nvme', () => {
    expect(buildLinuxFirstPartitionPath('/dev/nvme0n1')).toBe('/dev/nvme0n1p1');
  });

  it('appends p1 for mmcblk', () => {
    expect(buildLinuxFirstPartitionPath('/dev/mmcblk0')).toBe('/dev/mmcblk0p1');
  });
});

// ─── Windows partition style detection — Get-Disk output parser ──────────────
// Regression coverage for the GPT false-positive bug (issue #15):
// On some USB controllers the Storage Management API (used by Get-Disk) fails
// while the disk is physically GPT. These helpers parse the raw command output
// so both the primary query and the WMI fallback are independently testable.

describe('windowsGetDiskStyleOutput', () => {
  it('returns gpt for GPT', () => {
    expect(windowsGetDiskStyleOutput('GPT')).toBe('gpt');
  });

  it('returns gpt for GPT with CRLF (Windows line ending)', () => {
    expect(windowsGetDiskStyleOutput('GPT\r\n')).toBe('gpt');
  });

  it('returns gpt for lowercase gpt', () => {
    expect(windowsGetDiskStyleOutput('gpt')).toBe('gpt');
  });

  it('returns mbr for MBR', () => {
    expect(windowsGetDiskStyleOutput('MBR')).toBe('mbr');
  });

  it('returns null for RAW (triggers WMI fallback)', () => {
    expect(windowsGetDiskStyleOutput('RAW')).toBeNull();
  });

  it('returns null for ERROR (Get-Disk PS catch block — triggers WMI fallback)', () => {
    expect(windowsGetDiskStyleOutput('ERROR')).toBeNull();
  });

  it('returns null for empty string', () => {
    expect(windowsGetDiskStyleOutput('')).toBeNull();
  });

  it('returns null for unexpected output', () => {
    expect(windowsGetDiskStyleOutput('UNKNOWN')).toBeNull();
  });
});

describe('windowsWmiDiskStyleOutput', () => {
  it('returns gpt for GPT (disk has GPT partitions via Win32_DiskPartition)', () => {
    expect(windowsWmiDiskStyleOutput('GPT')).toBe('gpt');
  });

  it('returns gpt for GPT with CRLF', () => {
    expect(windowsWmiDiskStyleOutput('GPT\r\n')).toBe('gpt');
  });

  it('returns mbr for MBR (disk has MBR partitions)', () => {
    expect(windowsWmiDiskStyleOutput('MBR')).toBe('mbr');
  });

  it('returns null for RAW (disk has no partitions — unknown is correct)', () => {
    expect(windowsWmiDiskStyleOutput('RAW')).toBeNull();
  });

  it('returns null for ERROR (WMI also failed — unknown is correct)', () => {
    expect(windowsWmiDiskStyleOutput('ERROR')).toBeNull();
  });

  it('returns null for empty string', () => {
    expect(windowsWmiDiskStyleOutput('')).toBeNull();
  });
});

// ─── #23 regression: drive letter assignment — no EFI GUID ───────────────────
// The old assignWindowsDriveLetter set id=c12a7328-… (EFI GUID), which caused
// Windows to refuse drive letter assignment on the OPENCORE partition.

describe('buildWindowsAssignLetterDiskpartScript', () => {
  it('does not contain the EFI partition GUID', () => {
    const script = buildWindowsAssignLetterDiskpartScript('4', 1);
    expect(script).not.toContain('c12a7328');
    expect(script).not.toContain('set id');
  });

  it('selects the correct disk', () => {
    const script = buildWindowsAssignLetterDiskpartScript('4', 1);
    expect(script).toContain('select disk 4');
  });

  it('selects the assessed partition number, not always partition 1', () => {
    const script = buildWindowsAssignLetterDiskpartScript('2', 3);
    expect(script).toContain('select partition 3');
    expect(script).not.toContain('select partition 1');
  });

  it('contains assign noerr', () => {
    const script = buildWindowsAssignLetterDiskpartScript('0', 1);
    expect(script).toContain('assign noerr');
  });

  it('ends with rescan so Windows registers the new letter', () => {
    const script = buildWindowsAssignLetterDiskpartScript('1', 2);
    expect(script).toContain('rescan');
  });
});

// ─── #24 regression: primary data partition selection ────────────────────────
// Old code filtered $_.Type -eq 'Basic' (unreliable) and $_.Size -gt 50GB
// (fails small disks). New code excludes by GPT GUID and min 20 GB.

const GB = 1024 * 1024 * 1024;

describe('selectWindowsPrimaryDataPartition', () => {
  it('selects the largest non-system partition on a typical Windows GPT disk', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 100 * 1024 * 1024, gptType: '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' }, // EFI
      { partitionNumber: 2, sizeBytes: 16 * 1024 * 1024, gptType: '{e3c9e316-0b5c-4db8-817d-f92df00215ae}' },  // MSR
      { partitionNumber: 3, sizeBytes: 220 * GB, gptType: '{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}' },           // C:
      { partitionNumber: 4, sizeBytes: 700 * 1024 * 1024, gptType: '{de94bba4-06d1-4d40-a16a-bfd50179d6ac}' }, // Recovery
    ]);
    expect(result).toBe(3);
  });

  it('excludes the EFI System Partition by GUID', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 500 * GB, gptType: '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' },
    ]);
    expect(result).toBeNull();
  });

  it('excludes the MSR partition by GUID', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 500 * GB, gptType: '{e3c9e316-0b5c-4db8-817d-f92df00215ae}' },
    ]);
    expect(result).toBeNull();
  });

  it('excludes Windows Recovery by GUID', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 500 * GB, gptType: '{de94bba4-06d1-4d40-a16a-bfd50179d6ac}' },
    ]);
    expect(result).toBeNull();
  });

  it('returns null when all qualifying candidates are below 20 GB', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 10 * GB, gptType: '' },
    ]);
    expect(result).toBeNull();
  });

  it('accepts a 25 GB data partition (smaller than the old 50 GB floor)', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 100 * 1024 * 1024, gptType: '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' },
      { partitionNumber: 2, sizeBytes: 25 * GB, gptType: '' },
    ]);
    expect(result).toBe(2);
  });

  it('returns null for an empty partition list', () => {
    expect(selectWindowsPrimaryDataPartition([])).toBeNull();
  });

  it('picks the largest when multiple data partitions exist', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 2, sizeBytes: 50 * GB, gptType: '' },
      { partitionNumber: 3, sizeBytes: 200 * GB, gptType: '' },
      { partitionNumber: 4, sizeBytes: 80 * GB, gptType: '' },
    ]);
    expect(result).toBe(3);
  });

  it('is case-insensitive for GptType (PowerShell may return uppercase GUIDs)', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 200 * GB, gptType: '{C12A7328-F81F-11D2-BA4B-00A0C93EC93B}' }, // uppercase EFI
    ]);
    expect(result).toBeNull();
  });

  it('treats missing gptType as a non-system partition (MBR disk case)', () => {
    const result = selectWindowsPrimaryDataPartition([
      { partitionNumber: 1, sizeBytes: 200 * GB },
    ]);
    expect(result).toBe(1);
  });
});

// ─── selectWindowsOpencoreReuseCandidate ─────────────────────────────────────
// Regression for issue #26: large USB drives arrive pre-formatted as OPENCORE
// FAT32 with no drive letter; the app should reuse them instead of running diskpart.

const MB = 1024 * 1024;

describe('selectWindowsOpencoreReuseCandidate', () => {
  it('returns the single qualifying partition', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
    ]);
    expect(result).not.toBeNull();
    expect(result!.partitionNumber).toBe(1);
  });

  it('returns null when driveLetter is already assigned (do not double-assign)', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: 'E', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
    ]);
    expect(result).toBeNull();
  });

  it('returns null when fileSystem is not FAT32 (NTFS, RAW, etc.)', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'NTFS', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
    ]);
    expect(result).toBeNull();
  });

  it('returns null when label is not OPENCORE', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'USB_DRIVE', sizeBytes: 30_000 * MB },
    ]);
    expect(result).toBeNull();
  });

  it('returns null when partition is smaller than 200 MB', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 100 * MB },
    ]);
    expect(result).toBeNull();
  });

  it('returns null when there are two OPENCORE FAT32 partitions (ambiguous)', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
      { partitionNumber: 2, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
    ]);
    expect(result).toBeNull();
  });

  it('returns null for an empty partition list', () => {
    expect(selectWindowsOpencoreReuseCandidate([])).toBeNull();
  });

  it('is case-insensitive for fileSystem and label', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'fat32', fileSystemLabel: 'opencore', sizeBytes: 30_000 * MB },
    ]);
    expect(result).not.toBeNull();
  });

  it('accepts exactly 200 MB (boundary — minimum valid size)', () => {
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 200 * MB },
    ]);
    expect(result).not.toBeNull();
  });

  it('returns the unletttered partition when another OPENCORE FAT32 partition already has a letter', () => {
    // Partition 1 already has a drive letter (already reachable), partition 2 does not.
    // Only partition 2 passes the no-letter filter → exactly one candidate → return it.
    const result = selectWindowsOpencoreReuseCandidate([
      { partitionNumber: 1, driveLetter: 'D', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
      { partitionNumber: 2, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
    ]);
    expect(result).not.toBeNull();
    expect(result!.partitionNumber).toBe(2);
  });
});

// ─── Issue #30: drive letter assignment ───────────────────────────────────────

describe('assessWindowsFlashPreparationState — assign stage for #30', () => {
  it('reports assign stage when FAT32 OPENCORE exists without drive letter', () => {
    const result = assessWindowsFlashPreparationState({
      partitions: [
        { partitionNumber: 1, driveLetter: '', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
      ],
      expectedLabel: 'OPENCORE',
    });
    expect(result.status).toBe('failed');
    expect(result.stage).toBe('assign');
    expect(result.targetPartitionNumber).toBe(1);
  });

  it('becomes ready when drive letter is later assigned', () => {
    // Simulates what happens after assignWindowsDriveLetter succeeds
    const result = assessWindowsFlashPreparationState({
      partitions: [
        { partitionNumber: 1, driveLetter: 'G', fileSystem: 'FAT32', fileSystemLabel: 'OPENCORE', sizeBytes: 30_000 * MB },
      ],
      expectedLabel: 'OPENCORE',
    });
    expect(result.status).toBe('ready');
    expect(result.driveLetter).toBe('G');
    expect(result.stage).toBeNull();
  });

  it('reports correct stage for NTFS partition (format, not assign)', () => {
    const result = assessWindowsFlashPreparationState({
      partitions: [
        { partitionNumber: 1, driveLetter: 'E', fileSystem: 'NTFS', fileSystemLabel: 'MyDrive', sizeBytes: 100_000 * MB },
      ],
      expectedLabel: 'OPENCORE',
    });
    expect(result.status).toBe('failed');
    expect(result.stage).toBe('format');
  });

  it('falls back to partition 1 label-lookup when label does not match', () => {
    // Partition has a letter and is FAT32 but label is not OPENCORE
    const result = assessWindowsFlashPreparationState({
      partitions: [
        { partitionNumber: 1, driveLetter: 'E', fileSystem: 'FAT32', fileSystemLabel: 'MYUSB', sizeBytes: 16_000 * MB },
      ],
      expectedLabel: 'OPENCORE',
    });
    expect(result.status).toBe('ready');
    expect(result.stage).toBe('label-lookup');
    expect(result.usedPartitionFallback).toBe(true);
  });

  it('shouldRetryWindowsFlashPreparation returns true for assign stage', () => {
    expect(shouldRetryWindowsFlashPreparation({
      attempt: 0,
      maxAttempts: 2,
      diskpartFailed: true,
      driveLetter: '',
      stage: 'assign',
    })).toBe(true);
  });
});
