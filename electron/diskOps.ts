import path from 'path';
import fs from 'fs';
import os from 'os';
import util from 'util';
import crypto from 'node:crypto';
import { exec } from 'child_process';
import { elevateCommand } from './linuxElevate.js';

const execPromise = util.promisify(exec);

// ─── Public interfaces ────────────────────────────────────────────────────────

export type SafetyViolationCode =
  | 'SYSTEM_DISK' | 'MBR_PARTITION_TABLE' | 'DEVICE_NOT_FOUND'
  | 'EFI_TOO_LARGE' | 'INSUFFICIENT_SPACE' | 'EFI_MISSING_PLIST'
  | 'PARTITION_IN_USE' | 'UNKNOWN_PARTITION_TABLE';

export interface SafetyCheckViolation {
  code: SafetyViolationCode;
  severity: 'fatal' | 'warn';
  message: string;
}

export interface SafetyCheckResult {
  passed: boolean;
  violations: SafetyCheckViolation[];
}

export interface DiskInfo {
  device: string;
  devicePath?: string;
  isSystemDisk: boolean;
  partitionTable: 'gpt' | 'mbr' | 'unknown';
  mountedPartitions: string[];
  sizeBytes?: number;
  model?: string;
  vendor?: string;
  serialNumber?: string;
  transport?: string;
  removable?: boolean;
  identityConfidence?: 'strong' | 'medium' | 'weak' | 'ambiguous';
  identityFieldsUsed?: string[];
}

export interface ExistingEfiInspection {
  status: 'absent' | 'readable' | 'unreadable';
  reason?: string;
}

export interface ExistingEfiCopyResult extends ExistingEfiInspection {
  configHash: string | null;
}

export type UsbFlashPhase = 'safety' | 'erase' | 'format' | 'copy' | 'verify' | 'eject';
export type PartitionPhase = 'safety' | 'create-partition' | 'format' | 'mount' | 'copy' | 'unmount';
export type GptConversionPhase = 'safety' | 'erase' | 'format' | 'verify';

export interface FlashUsbOptions {
  device: string; efiPath: string; confirmed: boolean;
  onPhase: (phase: UsbFlashPhase, detail: string) => void;
  checkAborted: () => void;
  registerProcess?: (child: any) => void;
}

export interface CreateBootPartitionOptions {
  disk: string;
  efiPath: string;
  confirmed: boolean;
  onPhase: (phase: PartitionPhase, detail: string) => void;
  registerProcess?: (child: any) => void;
}

export interface ConvertDiskToGptOptions {
  device: string;
  confirmed: boolean;
  onPhase: (phase: GptConversionPhase, detail: string) => void;
  registerProcess?: (child: any) => void;
}


export type LogFunction = (level: string, ctx: string, msg: string, data?: Record<string, unknown>) => void;

// ─── Private helpers ──────────────────────────────────────────────────────────

function getDirSizeSync(dirPath: string): number {
  let total = 0;
  try {
    for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
      const fp = path.join(dirPath, entry.name);
      if (entry.isDirectory()) total += getDirSizeSync(fp);
      else total += fs.statSync(fp).size;
    }
  } catch {}
  return total;
}

async function getFreeSpaceMB(targetPath: string): Promise<number> {
  try {
    if (process.platform === 'win32') {
      const drive = targetPath.split(':')[0];
      const { stdout } = await execPromise(`powershell -NoProfile -Command "(Get-PSDrive -Name '${drive}' -ErrorAction SilentlyContinue).Free"`);
      return Math.floor(parseInt(stdout.trim()) / 1024 / 1024) || 0;
    } else {
      const { stdout } = await execPromise(`df -k "${targetPath}" | tail -1`);
      const parts = stdout.trim().split(/\s+/);
      return Math.floor(parseInt(parts[3]) / 1024) || 0;
    }
  } catch { return Infinity; }
}

// ─── Factory ──────────────────────────────────────────────────────────────────

export interface DiskOps {
  getSystemDiskId(): Promise<string>;
  getDiskPartitionTable(device: string): Promise<'gpt' | 'mbr' | 'unknown'>;
  getMountedPartitions(device: string): Promise<string[]>;
  getDeviceSize(device: string): Promise<number>;
  runSafetyChecks(device: string, efiPath: string | null, requiredSpaceBytes?: number): Promise<SafetyCheckResult>;
  getDiskInfo(device: string): Promise<DiskInfo>;
  inspectExistingEfi(device: string): Promise<ExistingEfiInspection>;
  copyExistingEfi(device: string, destinationPath: string): Promise<ExistingEfiCopyResult>;
  flashUsb(options: FlashUsbOptions): Promise<void>;
  convertDiskToGpt(options: ConvertDiskToGptOptions): Promise<DiskInfo>;
  shrinkPartition(disk: string, sizeGB: number, confirmed: boolean): Promise<void>;
  createBootPartition(options: CreateBootPartitionOptions): Promise<void>;
  getHardDrives(): Promise<Array<{ name: string; device: string; size: string; type: string }>>;
  listUsbDevices(): Promise<Array<{ name: string; device: string; size: string }>>;
}

import { sim } from './simulation.js';

/**
 * Parse the stdout of `(Get-Disk -Number N).PartitionStyle.ToString().ToUpper()`.
 * Returns null when the output is anything other than GPT/MBR (e.g. RAW, ERROR,
 * empty) so the caller can fall back to an alternative detection method.
 */
export function windowsGetDiskStyleOutput(stdout: string): 'gpt' | 'mbr' | null {
  const s = stdout.trim().toUpperCase();
  if (s === 'GPT') return 'gpt';
  if (s === 'MBR') return 'mbr';
  return null;
}

/**
 * Parse the output of the Win32_DiskPartition WMI fallback script.
 * The script emits 'GPT', 'MBR', 'RAW' (no partitions), or 'ERROR'.
 * Returns null for RAW/ERROR so 'unknown' propagates correctly.
 */
export function windowsWmiDiskStyleOutput(stdout: string): 'gpt' | 'mbr' | null {
  const s = stdout.trim().toUpperCase();
  if (s === 'GPT') return 'gpt';
  if (s === 'MBR') return 'mbr';
  return null;
}

/**
 * Select the best shrink target from a list of Windows partitions.
 * Excludes EFI System, MSR, and Windows Recovery partitions by GPT type GUID.
 * Requires the candidate to be at least 20 GB so we never accidentally target
 * a recovery stub or utility partition.
 * Returns the partition number of the largest qualifying candidate, or null.
 */
export function selectWindowsPrimaryDataPartition(partitions: Array<{
  partitionNumber: number;
  sizeBytes: number;
  gptType?: string;
}>): number | null {
  const EXCLUDED_GUIDS = new Set([
    '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}', // EFI System Partition
    '{e3c9e316-0b5c-4db8-817d-f92df00215ae}', // Microsoft Reserved (MSR)
    '{de94bba4-06d1-4d40-a16a-bfd50179d6ac}', // Windows Recovery Environment
  ]);
  const MIN_BYTES = 20 * 1024 * 1024 * 1024; // 20 GB minimum
  const candidates = partitions
    .filter(p =>
      p.partitionNumber > 0 &&
      p.sizeBytes >= MIN_BYTES &&
      !EXCLUDED_GUIDS.has((p.gptType ?? '').toLowerCase()),
    )
    .sort((a, b) => b.sizeBytes - a.sizeBytes);
  return candidates[0]?.partitionNumber ?? null;
}

/**
 * Build the diskpart script that assigns a drive letter to the target partition.
 * Does NOT set a GPT type GUID — setting the EFI GUID (c12a7328-…) prevents
 * Windows from assigning a drive letter, which was the root cause of issue #23.
 */
export function buildWindowsAssignLetterDiskpartScript(diskNum: string, partitionNum: number): string {
  return [
    `select disk ${diskNum}`,
    `select partition ${partitionNum}`,
    'assign noerr',
    'rescan',
    '',
  ].join('\n');
}

export function buildLinuxFirstPartitionPath(device: string): string {
  return /(?:nvme\d+n\d+|mmcblk\d+|loop\d+)$/i.test(device) ? `${device}p1` : `${device}1`;
}

export function buildWindowsPhysicalDrivePath(diskNumber: string | number): string {
  const normalized = String(diskNumber).trim();
  const fromDiskAlias = normalized.match(/^disk(\d+)$/i)?.[1];
  const fromPhysicalDrive = normalized.match(/PhysicalDrive(\d+)/i)?.[1];
  const resolved = fromDiskAlias ?? fromPhysicalDrive ?? normalized;
  return `\\\\.\\PhysicalDrive${resolved}`;
}

export function isWindowsUsbLikeDisk(input: {
  busType?: string | null;
  interfaceType?: string | null;
  pnpDeviceId?: string | null;
  mediaType?: string | null;
  isBoot?: boolean | null;
  isSystem?: boolean | null;
}): boolean {
  if (input.isBoot === true || input.isSystem === true) return false;

  const busType = String(input.busType ?? '').trim().toUpperCase();
  const interfaceType = String(input.interfaceType ?? '').trim().toUpperCase();
  const pnpDeviceId = String(input.pnpDeviceId ?? '').trim().toUpperCase();
  const mediaType = String(input.mediaType ?? '').trim().toUpperCase();

  return busType === 'USB'
    || interfaceType === 'USB'
    || pnpDeviceId.includes('USBSTOR')
    || pnpDeviceId.startsWith('USB\\')
    || mediaType.includes('REMOVABLE');
}

export function buildWindowsFlashDiskpartScript(diskNum: string, partitionSizeMB?: number): string {
  return [
    `select disk ${diskNum}`,
    'attributes disk clear readonly noerr',
    'offline disk noerr',
    'online disk noerr',
    'clean noerr',
    'convert gpt noerr',
    partitionSizeMB && partitionSizeMB > 0
      ? `create partition primary size=${partitionSizeMB} noerr`
      : 'create partition primary noerr',
    'rescan',
    '',
  ].join('\n');
}

/**
 * Phase-2 diskpart script: format and assign the newly created partition.
 * Deliberately does NOT use `noerr` on the format command so that failures
 * surface immediately instead of being silently swallowed (root cause of #38/#41).
 */
export function buildWindowsFormatDiskpartScript(diskNum: string, partitionNumber = 1): string {
  return [
    `select disk ${diskNum}`,
    `select partition ${partitionNumber}`,
    'format fs=fat32 quick label=OPENCORE',
    'assign noerr',
    'rescan',
    '',
  ].join('\n');
}

export function buildWindowsConvertToGptDiskpartScript(diskNum: string, partitionSizeMB?: number): string {
  return buildWindowsFlashDiskpartScript(diskNum, partitionSizeMB);
}

export function canReusePreparedOpenCoreVolume(partitionTable: 'gpt' | 'mbr' | 'unknown'): boolean {
  return partitionTable === 'gpt';
}

export function buildWindowsBootPartitionDiskpartScript(diskNum: string): string {
  return [
    `select disk ${diskNum}`,
    'create partition primary size=16384',
    'select partition 1',
    'format fs=fat32 quick label=BOOTSTRAP',
    'assign',
    '',
  ].join('\n');
}

export function getWindowsFat32PartitionSizeMB(deviceSizeBytes: number): number | undefined {
  return deviceSizeBytes > 32_000_000_000 ? 30_000 : undefined;
}

/**
 * Find an existing unletttered OPENCORE FAT32 partition that is safe to reuse.
 * Returns the single candidate partition if exactly one qualifies, or null.
 * Safety conditions: FAT32 filesystem, OPENCORE label, no drive letter currently
 * assigned, and at least 200 MB in size.  Requiring exactly one candidate avoids
 * ambiguity on disks that happen to have two similarly-labelled partitions.
 */
export function selectWindowsOpencoreReuseCandidate(
  partitions: WindowsFlashPreparedPartition[],
): WindowsFlashPreparedPartition | null {
  const MIN_SIZE_BYTES = 200 * 1024 * 1024; // 200 MB
  const candidates = partitions.filter(
    (p) =>
      p.fileSystem.trim().toUpperCase() === 'FAT32' &&
      p.fileSystemLabel.trim().toUpperCase() === 'OPENCORE' &&
      !p.driveLetter.trim() &&
      p.sizeBytes >= MIN_SIZE_BYTES,
  );
  return candidates.length === 1 ? candidates[0] : null;
}

export type WindowsFlashPreparationStage =
  | 'create-partition'
  | 'format'
  | 'assign'
  | 'label-lookup';

export interface WindowsFlashPreparedPartition {
  partitionNumber: number;
  driveLetter: string;
  fileSystem: string;
  fileSystemLabel: string;
  sizeBytes: number;
}

export interface WindowsFlashPreparationAssessment {
  status: 'ready' | 'failed';
  stage: WindowsFlashPreparationStage | null;
  driveLetter: string;
  targetPartitionNumber: number | null;
  usedPartitionFallback: boolean;
}

export function assessWindowsFlashPreparationState(input: {
  partitions: WindowsFlashPreparedPartition[];
  expectedLabel: string;
}): WindowsFlashPreparationAssessment {
  const normalized = input.partitions
    .map((partition) => ({
      partitionNumber: Number(partition.partitionNumber),
      driveLetter: String(partition.driveLetter ?? '').trim().toUpperCase(),
      fileSystem: String(partition.fileSystem ?? '').trim().toUpperCase(),
      fileSystemLabel: String(partition.fileSystemLabel ?? '').trim().toUpperCase(),
      sizeBytes: Number(partition.sizeBytes ?? 0),
    }))
    .filter((partition) => Number.isFinite(partition.partitionNumber) && partition.partitionNumber > 0)
    .sort((a, b) => {
      if (a.partitionNumber === 1 && b.partitionNumber !== 1) return -1;
      if (b.partitionNumber === 1 && a.partitionNumber !== 1) return 1;
      if (b.sizeBytes !== a.sizeBytes) return b.sizeBytes - a.sizeBytes;
      return a.partitionNumber - b.partitionNumber;
    });

  const target = normalized[0] ?? null;
  if (!target) {
    return {
      status: 'failed',
      stage: 'create-partition',
      driveLetter: '',
      targetPartitionNumber: null,
      usedPartitionFallback: false,
    };
  }

  if (target.fileSystem !== 'FAT32') {
    return {
      status: 'failed',
      stage: 'format',
      driveLetter: '',
      targetPartitionNumber: target.partitionNumber,
      usedPartitionFallback: false,
    };
  }

  if (!target.driveLetter) {
    return {
      status: 'failed',
      stage: 'assign',
      driveLetter: '',
      targetPartitionNumber: target.partitionNumber,
      usedPartitionFallback: false,
    };
  }

  const expectedLabel = String(input.expectedLabel ?? '').trim().toUpperCase();
  const labelMatches = expectedLabel.length > 0 && target.fileSystemLabel === expectedLabel;
  if (!labelMatches) {
    return {
      status: 'ready',
      stage: 'label-lookup',
      driveLetter: target.driveLetter,
      targetPartitionNumber: target.partitionNumber,
      usedPartitionFallback: true,
    };
  }

  return {
    status: 'ready',
    stage: null,
    driveLetter: target.driveLetter,
    targetPartitionNumber: target.partitionNumber,
    usedPartitionFallback: false,
  };
}

export function shouldRetryWindowsFlashPreparation(input: {
  attempt: number;
  maxAttempts: number;
  diskpartFailed: boolean;
  driveLetter: string;
  stage?: WindowsFlashPreparationStage | null;
}): boolean {
  if (!input.diskpartFailed) return false;
  if (input.driveLetter) return false;
  if (input.attempt >= input.maxAttempts - 1) return false;
  return input.stage === 'create-partition' || input.stage === 'format' || input.stage === 'assign';
}

export function createDiskOps(log: LogFunction): DiskOps {

  /**
   * Internal helper to execute commands and register the child process
   * for reliable cancellation/cleanup.
   */
  async function runCmd(cmd: string, register?: (child: any) => void): Promise<{ stdout: string; stderr: string }> {
    return new Promise((resolve, reject) => {
      const child = exec(cmd, (error, stdout, stderr) => {
        if (error) reject(error);
        else resolve({ stdout, stderr });
      });
      if (register) register(child);
    });
  }

  async function withBestEffort<T>(promise: Promise<T>, fallback: T, timeoutMs = 6_000): Promise<T> {
    return await Promise.race([
      promise.catch(() => fallback),
      new Promise<T>((resolve) => {
        setTimeout(() => resolve(fallback), timeoutMs);
      }),
    ]);
  }

  function escapeRegExp(value: string): string {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  function getWindowsDiskNumber(device: string): string | null {
    return device.match(/PhysicalDrive(\d+)/i)?.[1]
      || device.match(/^disk(\d+)$/i)?.[1]
      || null;
  }

  async function listWindowsDisks(register?: (child: any) => void): Promise<Array<{
    Number: number;
    FriendlyName?: string;
    Size?: number;
    BusType?: string;
    Path?: string;
    IsBoot?: boolean;
    IsSystem?: boolean;
    InterfaceType?: string;
    PNPDeviceID?: string;
    MediaType?: string;
    Model?: string;
  }>> {
    try {
      const { stdout } = await runCmd(
        'powershell -NoProfile -Command "$cim = Get-CimInstance Win32_DiskDrive | Select-Object Index,InterfaceType,PNPDeviceID,MediaType,Model; Get-Disk | ForEach-Object { $disk = $_; $drive = $cim | Where-Object { $_.Index -eq $disk.Number } | Select-Object -First 1; [pscustomobject]@{ Number = $disk.Number; FriendlyName = $disk.FriendlyName; Size = $disk.Size; BusType = if ($disk.BusType) { $disk.BusType.ToString() } else { $null }; Path = $disk.Path; IsBoot = $disk.IsBoot; IsSystem = $disk.IsSystem; InterfaceType = $drive.InterfaceType; PNPDeviceID = $drive.PNPDeviceID; MediaType = $drive.MediaType; Model = $drive.Model } } | ConvertTo-Json -Compress"',
        register,
      );
      const parsed = JSON.parse(stdout || '[]');
      if (Array.isArray(parsed)) return parsed;
      return parsed ? [parsed] : [];
    } catch {
      return [];
    }
  }

  async function getWindowsDriveLetterForLabel(diskNum: string, label: string, register?: (child: any) => void): Promise<string> {
    try {
      const { stdout } = await runCmd(
        `powershell -NoProfile -Command "try { (Get-Partition -DiskNumber ${diskNum} -ErrorAction Stop | Get-Volume -ErrorAction SilentlyContinue | Where-Object { $_.FileSystemLabel -eq '${label}' }).DriveLetter } catch { '' }"`,
        register
      );
      return stdout.trim();
    } catch {
      return '';
    }
  }

  async function getWindowsPreparedPartitions(
    diskNum: string,
    register?: (child: any) => void,
  ): Promise<WindowsFlashPreparedPartition[]> {
    try {
      const { stdout } = await runCmd(
        `powershell -NoProfile -Command "try { $parts = Get-Partition -DiskNumber ${diskNum} -ErrorAction Stop | Sort-Object PartitionNumber; $rows = foreach ($part in $parts) { $vol = $part | Get-Volume -ErrorAction SilentlyContinue; [pscustomobject]@{ PartitionNumber = $part.PartitionNumber; DriveLetter = if ($vol -and $vol.DriveLetter) { $vol.DriveLetter } elseif ($part.DriveLetter) { $part.DriveLetter } else { '' }; FileSystem = if ($vol -and $vol.FileSystem) { $vol.FileSystem } else { '' }; FileSystemLabel = if ($vol -and $vol.FileSystemLabel) { $vol.FileSystemLabel } else { '' }; SizeBytes = $part.Size } }; $rows | ConvertTo-Json -Compress } catch { '[]' }"`,
        register,
      );
      const parsed = JSON.parse(stdout || '[]');
      const rows = Array.isArray(parsed) ? parsed : parsed ? [parsed] : [];
      return rows.map((row) => ({
        partitionNumber: Number(row.PartitionNumber),
        driveLetter: typeof row.DriveLetter === 'string' ? row.DriveLetter.trim() : '',
        fileSystem: typeof row.FileSystem === 'string' ? row.FileSystem.trim() : '',
        fileSystemLabel: typeof row.FileSystemLabel === 'string' ? row.FileSystemLabel.trim() : '',
        sizeBytes: typeof row.SizeBytes === 'number' ? row.SizeBytes : Number(row.SizeBytes ?? 0),
      })).filter((row) => Number.isFinite(row.partitionNumber) && row.partitionNumber > 0);
    } catch {
      return [];
    }
  }

  async function waitForWindowsPreparedPartitions(
    diskNum: string,
    register?: (child: any) => void,
    attempts = 10,
    delayMs = 400,
  ): Promise<WindowsFlashPreparedPartition[]> {
    let last: WindowsFlashPreparedPartition[] = [];
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      last = await getWindowsPreparedPartitions(diskNum, register);
      if (last.length > 0) return last;
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    return last;
  }

  async function waitForWindowsDriveLetterForLabel(
    diskNum: string,
    label: string,
    register?: (child: any) => void,
    attempts = 20,
    delayMs = 400,
  ): Promise<string> {
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      const driveLetter = await getWindowsDriveLetterForLabel(diskNum, label, register);
      if (driveLetter) return driveLetter;
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    return '';
  }

  async function detachWindowsDriveLetters(
    diskNum: string,
    register?: (child: any) => void,
  ): Promise<void> {
    // Try soft dismount (/D) first, then hard-remove (/P) for any handle-locked volume.
    // /P is safe here: this function is only called immediately before a destructive
    // diskpart clean that wipes the entire disk anyway.
    await runCmd(
      `powershell -NoProfile -Command "Get-Partition -DiskNumber ${diskNum} | Where-Object { $_.DriveLetter } | ForEach-Object { $letter = $_.DriveLetter + ':'; try { mountvol $letter /D } catch {}; try { mountvol $letter /P } catch {} }"`,
      register,
    );
  }

  async function assignWindowsDriveLetter(
    diskNum: string,
    partitionNumber: number | null,
    label: string,
    register?: (child: any) => void,
  ): Promise<string> {
    const partNum = partitionNumber ?? 1;

    // Approach 1: diskpart assign.
    // NOTE: do NOT use `set id=c12a7328-…` (EFI GUID) here — setting that GUID
    // causes Windows to hide the partition from the drive-letter registry,
    // making the subsequent `assign` silently produce no letter (issue #23).
    const script = buildWindowsAssignLetterDiskpartScript(diskNum, partNum);
    const scriptPath = path.join(os.tmpdir(), `assign-letter-${crypto.randomUUID()}.txt`);
    fs.writeFileSync(scriptPath, script);
    try {
      await runCmd(`diskpart /s "${scriptPath}"`, register);
    } catch (e) {
      log('WARN', 'diskOps', 'diskpart assign-letter failed', { diskNum, partNum, error: (e as Error).message });
    } finally {
      try { fs.unlinkSync(scriptPath); } catch {}
    }
    const letter1 = await waitForWindowsDriveLetterForLabel(diskNum, label, register, 8, 500);
    if (letter1) return letter1;

    // Approach 2: PowerShell Set-Partition (more reliable on some systems).
    log('DEBUG', 'diskOps', 'diskpart assign produced no letter — trying Set-Partition', { diskNum, partNum });
    try {
      await runCmd(
        `powershell -NoProfile -Command "try { Set-Partition -DiskNumber ${diskNum} -PartitionNumber ${partNum} -AssignDriveLetter -ErrorAction Stop } catch {}"`,
        register,
      );
    } catch {}
    const letter2 = await waitForWindowsDriveLetterForLabel(diskNum, label, register, 10, 500);
    if (letter2) return letter2;

    // Approach 3: Add-PartitionAccessPath — issue #30: on some systems Windows
    // refuses auto-letter but accepts an explicit access-path assignment.
    log('DEBUG', 'diskOps', 'Set-Partition produced no letter — trying Add-PartitionAccessPath', { diskNum, partNum });
    try {
      await runCmd(
        `powershell -NoProfile -Command "try { Add-PartitionAccessPath -DiskNumber ${diskNum} -PartitionNumber ${partNum} -AssignDriveLetter -ErrorAction Stop } catch {}"`,
        register,
      );
    } catch {}
    const letter3 = await waitForWindowsDriveLetterForLabel(diskNum, label, register, 10, 600);
    if (letter3) return letter3;

    // Last resort: poll without label constraint — the volume may have received
    // a letter but the label query timing can lag behind.
    log('DEBUG', 'diskOps', 'All assign approaches produced no labelled letter — scanning partitions directly', { diskNum, partNum });
    const partitions = await getWindowsPreparedPartitions(diskNum, register);
    const withLetter = partitions.find(
      (p) => p.partitionNumber === partNum && p.driveLetter.trim(),
    );
    if (withLetter?.driveLetter) return withLetter.driveLetter.trim();

    return '';
  }

  async function getWindowsPrimaryPartitionNumber(diskNum: string): Promise<string> {
    // Fetch all partitions with their size and GPT type GUID as JSON, then
    // select the best shrink target using selectWindowsPrimaryDataPartition.
    // Filtering by $_.Type -eq 'Basic' is unreliable: some Windows versions
    // return 'Unknown' for a valid data partition (issue #24).
    const { stdout } = await runCmd(
      `powershell -NoProfile -Command "try { Get-Partition -DiskNumber ${diskNum} -ErrorAction Stop | Select-Object PartitionNumber, Size, GptType | ConvertTo-Json -Compress } catch { '[]' }"`
    );
    let target: number | null = null;
    try {
      const raw = JSON.parse(stdout.trim() || '[]');
      const rows: any[] = Array.isArray(raw) ? raw : (raw ? [raw] : []);
      const partitions = rows
        .map((r) => ({
          partitionNumber: Number(r.PartitionNumber),
          sizeBytes: Number(r.Size ?? 0),
          gptType: typeof r.GptType === 'string' ? r.GptType.trim().toLowerCase() : '',
        }))
        .filter((p) => Number.isFinite(p.partitionNumber) && p.partitionNumber > 0);
      target = selectWindowsPrimaryDataPartition(partitions);
    } catch {}
    return target != null ? String(target) : '';
  }

  function hashFile(filePath: string): string {
    return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
  }

  function copyDirContents(sourcePath: string, destinationPath: string): void {
    fs.mkdirSync(destinationPath, { recursive: true });
    fs.cpSync(sourcePath, destinationPath, {
      recursive: true,
      force: true,
      dereference: false,
    });
  }

  async function getDarwinEfiPartition(device: string, register?: (child: any) => void): Promise<string | null> {
    const { stdout } = await runCmd(`diskutil list ${device} 2>/dev/null`, register);
    for (const line of stdout.split('\n')) {
      if (!/\bEFI\b/i.test(line)) continue;
      const match = line.match(/\b(disk\d+s\d+)\b/);
      if (match?.[1]) return `/dev/${match[1]}`;
    }
    return null;
  }

  async function getLinuxEfiPartition(device: string, register?: (child: any) => void): Promise<{ path: string; mountPoint: string | null } | null> {
    const { stdout } = await runCmd(`lsblk -J -o PATH,PARTTYPE,PARTLABEL,LABEL,FSTYPE,MOUNTPOINT "${device}" 2>/dev/null`, register);
    const parsed = JSON.parse(stdout);
    const root = Array.isArray(parsed.blockdevices) ? parsed.blockdevices[0] : null;
    const children = Array.isArray(root?.children) ? root.children : [];
    const efiGuid = 'c12a7328-f81f-11d2-ba4b-00a0c93ec93b';
    const entry = children.find((child: any) => {
      const partType = typeof child?.parttype === 'string' ? child.parttype.toLowerCase() : '';
      const partLabel = typeof child?.partlabel === 'string' ? child.partlabel.toLowerCase() : '';
      const label = typeof child?.label === 'string' ? child.label.toLowerCase() : '';
      return partType === efiGuid || partLabel.includes('efi') || label.includes('efi');
    });
    if (!entry?.path) return null;
    return {
      path: entry.path,
      mountPoint: typeof entry.mountpoint === 'string' && entry.mountpoint.trim() ? entry.mountpoint.trim() : null,
    };
  }

  async function getWindowsEfiPartition(device: string, register?: (child: any) => void): Promise<{ partitionNumber: number; driveLetter: string | null } | null> {
    const diskNum = getWindowsDiskNumber(device);
    if (!diskNum) return null;
    const { stdout } = await runCmd(
      `powershell -NoProfile -Command "$p = Get-Partition -DiskNumber ${diskNum} | Where-Object { ($_.GptType -eq '{C12A7328-F81F-11D2-BA4B-00A0C93EC93B}') -or ($_.Type -eq 'System') } | Select-Object -First 1 PartitionNumber,DriveLetter; if ($p) { $p | ConvertTo-Json -Compress }"`,
      register,
    );
    if (!stdout.trim()) return null;
    const parsed = JSON.parse(stdout);
    return {
      partitionNumber: Number(parsed.PartitionNumber),
      driveLetter: typeof parsed.DriveLetter === 'string' && parsed.DriveLetter.trim()
        ? parsed.DriveLetter.trim()
        : null,
    };
  }

  async function withMountedExistingEfi<T>(
    device: string,
    fn: (context: { mountPoint: string; partitionIdentifier: string }) => Promise<T>,
    register?: (child: any) => void,
  ): Promise<{ kind: 'ok'; value: T } | { kind: 'absent' } | { kind: 'error'; reason: string }> {
    if (process.platform === 'darwin') {
      const partitionIdentifier = await getDarwinEfiPartition(device, register);
      if (!partitionIdentifier) return { kind: 'absent' };

      let mountedTemporarily = false;
      try {
        let { stdout: infoOut } = await runCmd(`diskutil info ${partitionIdentifier} 2>/dev/null`, register);
        let mountPoint = infoOut.match(/Mount Point:\s*(.+)/)?.[1]?.trim() ?? '';
        if (!mountPoint || mountPoint === 'Not Mounted') {
          await runCmd(`diskutil mount ${partitionIdentifier}`, register);
          mountedTemporarily = true;
          infoOut = (await runCmd(`diskutil info ${partitionIdentifier} 2>/dev/null`, register)).stdout;
          mountPoint = infoOut.match(/Mount Point:\s*(.+)/)?.[1]?.trim() ?? '';
        }
        if (!mountPoint || mountPoint === 'Not Mounted') {
          return { kind: 'error', reason: `EFI partition exists but could not be mounted (${partitionIdentifier}).` };
        }
        return { kind: 'ok', value: await fn({ mountPoint, partitionIdentifier }) };
      } catch (error) {
        return { kind: 'error', reason: `EFI inspection failed on ${partitionIdentifier}: ${(error as Error).message}` };
      } finally {
        if (mountedTemporarily) {
          try { await runCmd(`diskutil unmount ${partitionIdentifier}`, register); } catch {}
        }
      }
    }

    if (process.platform === 'linux') {
      let tempMount: string | null = null;
      const partition = await getLinuxEfiPartition(device, register).catch(() => null);
      if (!partition?.path) return { kind: 'absent' };

      try {
        let mountPoint = partition.mountPoint;
        if (!mountPoint) {
          tempMount = path.join(os.tmpdir(), `oc_efi_backup_${Date.now()}`);
          fs.mkdirSync(tempMount, { recursive: true });
          await runCmd(`mount -o ro "${partition.path}" "${tempMount}"`, register);
          mountPoint = tempMount;
        }
        return { kind: 'ok', value: await fn({ mountPoint, partitionIdentifier: partition.path }) };
      } catch (error) {
        return { kind: 'error', reason: `EFI partition exists but could not be mounted read-only (${partition.path}). ${(error as Error).message}` };
      } finally {
        if (tempMount) {
          try { await runCmd(`umount "${tempMount}"`, register); } catch {}
          try { fs.rmSync(tempMount, { recursive: true, force: true }); } catch {}
        }
      }
    }

    if (process.platform === 'win32') {
      const diskNum = getWindowsDiskNumber(device);
      const partition = await getWindowsEfiPartition(device, register).catch(() => null);
      if (!diskNum || !partition?.partitionNumber) return { kind: 'absent' };

      let assignedTemporarily = false;
      let driveLetter = partition.driveLetter;
      try {
        if (!driveLetter) {
          await runCmd(
            `powershell -NoProfile -Command "Add-PartitionAccessPath -DiskNumber ${diskNum} -PartitionNumber ${partition.partitionNumber} -AssignDriveLetter"`,
            register,
          );
          assignedTemporarily = true;
          driveLetter = (await getWindowsEfiPartition(device, register))?.driveLetter ?? null;
        }
        if (!driveLetter) {
          return { kind: 'error', reason: `EFI partition exists but no temporary drive letter could be assigned on disk ${diskNum}.` };
        }
        const mountPoint = `${driveLetter}:\\`;
        return { kind: 'ok', value: await fn({ mountPoint, partitionIdentifier: `${device}#${partition.partitionNumber}` }) };
      } catch (error) {
        return { kind: 'error', reason: `EFI inspection failed on disk ${diskNum}: ${(error as Error).message}` };
      } finally {
        if (assignedTemporarily && driveLetter) {
          try {
            await runCmd(
              `powershell -NoProfile -Command "Remove-PartitionAccessPath -DiskNumber ${diskNum} -PartitionNumber ${partition.partitionNumber} -AccessPath '${driveLetter}:\\\\'"`,
              register,
            );
          } catch {}
        }
      }
    }

    return { kind: 'error', reason: `EFI inspection is unsupported on platform ${process.platform}.` };
  }

  async function resolveDarwinMountedVolume(device: string, label: string, register?: (child: any) => void): Promise<{ identifier: string; mountPoint: string }> {
    const wholeDisk = device.replace(/^\/dev\//, '');
    const labelPattern = new RegExp(`\\b${escapeRegExp(label)}\\b.*\\b(${escapeRegExp(wholeDisk)}s\\d+)\\b`);

    for (let i = 0; i < 10; i++) {
      const { stdout: listOut } = await runCmd(`diskutil list ${device} 2>/dev/null`, register);
      const match = listOut.match(labelPattern);
      if (match?.[1]) {
        const identifier = `/dev/${match[1]}`;
        const { stdout: infoOut } = await runCmd(`diskutil info ${identifier} 2>/dev/null`, register);
        const mountPoint = infoOut.match(/Mount Point:\s*(.+)/)?.[1]?.trim();
        if (mountPoint && mountPoint !== 'Not Mounted') return { identifier, mountPoint };
      }
      await new Promise(r => setTimeout(r, 500));
    }

    throw new Error(`Could not resolve mounted ${label} volume for ${device}`);
  }

  async function listLinuxPartitions(device: string, register?: (child: any) => void): Promise<string[]> {
    const { stdout } = await runCmd(`lsblk -J -o NAME,TYPE ${device} 2>/dev/null`, register);
    try {
      const parsed = JSON.parse(stdout);
      const blockdevices = Array.isArray(parsed.blockdevices) ? parsed.blockdevices : [];
      const children = blockdevices.flatMap((d: any) => Array.isArray(d.children) ? d.children : []);
      return children
        .filter((c: any) => c?.type === 'part' && typeof c?.name === 'string')
        .map((c: any) => c.name);
    } catch {
      return [];
    }
  }

  async function readDarwinJson(command: string, register?: (child: any) => void): Promise<any | null> {
    try {
      const { stdout } = await runCmd(`${command} | plutil -convert json -o - -`, register);
      return JSON.parse(stdout);
    } catch {
      return null;
    }
  }

  async function getDarwinIdentity(device: string): Promise<Partial<DiskInfo>> {
    const info = await readDarwinJson(`diskutil info -plist "${device}" 2>/dev/null`);
    if (!info) return {};

    const serialNumber = typeof info.DiskUUID === 'string' ? info.DiskUUID.trim() : '';
    const transport = typeof info.BusProtocol === 'string' ? info.BusProtocol.trim() : '';
    const removable = typeof info.RemovableMediaOrExternalDevice === 'boolean'
      ? info.RemovableMediaOrExternalDevice
      : typeof info.Removable === 'boolean'
        ? info.Removable
        : undefined;
    const vendor = typeof info.IORegistryEntryName === 'string' && info.IORegistryEntryName.trim()
      ? info.IORegistryEntryName.trim()
      : typeof info.MediaType === 'string' && info.MediaType.trim()
        ? info.MediaType.trim()
        : '';
    const devicePath = typeof info.DeviceNode === 'string' && info.DeviceNode.trim()
      ? info.DeviceNode.trim()
      : device;

    const identityFieldsUsed = ['devicePath', 'partitionTable', 'sizeBytes'];
    if (serialNumber) identityFieldsUsed.unshift('serialNumber');
    if (transport) identityFieldsUsed.push('transport');
    if (typeof removable === 'boolean') identityFieldsUsed.push('removable');
    if (vendor) identityFieldsUsed.push('vendor');
    if (info.DeviceIdentifier || info.ParentWholeDisk) identityFieldsUsed.push('deviceIdentifier');

    return {
      devicePath,
      serialNumber: serialNumber || undefined,
      transport: transport || undefined,
      removable,
      vendor: vendor || undefined,
      identityConfidence: serialNumber ? 'strong' : identityFieldsUsed.length >= 5 ? 'medium' : 'weak',
      identityFieldsUsed,
    };
  }

  async function getLinuxIdentity(device: string): Promise<Partial<DiskInfo>> {
    try {
      const { stdout } = await runCmd(`lsblk -J -b -o PATH,MODEL,SERIAL,VENDOR,TRAN,RM,SIZE "${device}" 2>/dev/null`);
      const parsed = JSON.parse(stdout);
      const item = Array.isArray(parsed.blockdevices) ? parsed.blockdevices[0] : null;
      if (!item) return {};
      const identityFieldsUsed = ['devicePath', 'partitionTable', 'sizeBytes'];
      if (item.serial) identityFieldsUsed.unshift('serialNumber');
      if (item.vendor) identityFieldsUsed.push('vendor');
      if (item.tran) identityFieldsUsed.push('transport');
      if (typeof item.rm === 'boolean' || item.rm === 0 || item.rm === 1) identityFieldsUsed.push('removable');
      return {
        devicePath: typeof item.path === 'string' ? item.path : device,
        vendor: typeof item.vendor === 'string' && item.vendor.trim() ? item.vendor.trim() : undefined,
        serialNumber: typeof item.serial === 'string' && item.serial.trim() ? item.serial.trim() : undefined,
        transport: typeof item.tran === 'string' && item.tran.trim() ? item.tran.trim() : undefined,
        removable: item.rm === true || item.rm === 1 || item.rm === '1',
        identityConfidence: item.serial ? 'strong' : identityFieldsUsed.length >= 5 ? 'medium' : 'weak',
        identityFieldsUsed,
      };
    } catch {
      return {};
    }
  }

  async function getWindowsIdentity(device: string): Promise<Partial<DiskInfo>> {
    const diskNum = getWindowsDiskNumber(device);
    if (!diskNum) return {};
    try {
      const { stdout } = await runCmd(`powershell -NoProfile -Command "$disk = Get-Disk -Number ${diskNum}; $drive = Get-CimInstance Win32_DiskDrive | Where-Object { $_.Index -eq ${diskNum} } | Select-Object SerialNumber,Manufacturer,PNPDeviceID,InterfaceType,Model; [pscustomobject]@{ Path = $disk.Path; SerialNumber = $drive.SerialNumber; Vendor = $drive.Manufacturer; Transport = if ($disk.BusType) { $disk.BusType.ToString() } else { $drive.InterfaceType }; Removable = $disk.IsBoot -eq $false -and $disk.IsSystem -eq $false -and $disk.BusType -ne 'NVMe'; Model = if ($disk.FriendlyName) { $disk.FriendlyName } else { $drive.Model } } | ConvertTo-Json -Compress"`);
      const info = JSON.parse(stdout);
      const identityFieldsUsed = ['devicePath', 'partitionTable', 'sizeBytes'];
      if (info.SerialNumber) identityFieldsUsed.unshift('serialNumber');
      if (info.Vendor) identityFieldsUsed.push('vendor');
      if (info.Transport) identityFieldsUsed.push('transport');
      if (typeof info.Removable === 'boolean') identityFieldsUsed.push('removable');
      return {
        devicePath: typeof info.Path === 'string' && info.Path.trim() ? info.Path.trim() : device,
        vendor: typeof info.Vendor === 'string' && info.Vendor.trim() ? info.Vendor.trim() : undefined,
        serialNumber: typeof info.SerialNumber === 'string' && info.SerialNumber.trim() ? info.SerialNumber.trim() : undefined,
        transport: typeof info.Transport === 'string' && info.Transport.trim() ? info.Transport.trim() : undefined,
        removable: typeof info.Removable === 'boolean' ? info.Removable : undefined,
        model: typeof info.Model === 'string' && info.Model.trim() ? info.Model.trim() : undefined,
        identityConfidence: info.SerialNumber ? 'strong' : identityFieldsUsed.length >= 5 ? 'medium' : 'weak',
        identityFieldsUsed,
      };
    } catch {
      return {};
    }
  }

  async function resolveDarwinApfsContainer(device: string, register?: (child: any) => void): Promise<{ container: string; physicalStore: string }> {
    const wholeDisk = device.replace(/^\/dev\//, '');
    const apfsList = await readDarwinJson('diskutil apfs list -plist 2>/dev/null', register);
    const containers = Array.isArray(apfsList?.Containers) ? apfsList.Containers : [];
    for (const container of containers) {
      const stores = Array.isArray(container?.PhysicalStores) ? container.PhysicalStores : [];
      const match = stores.find((store: any) => store?.DeviceIdentifier === `${wholeDisk}s2` || String(store?.DeviceIdentifier || '').startsWith(`${wholeDisk}s`));
      if (match?.DeviceIdentifier && container?.ContainerReference) {
        return { container: String(container.ContainerReference), physicalStore: String(match.DeviceIdentifier) };
      }
    }
    throw new Error(`Could not safely resolve APFS container for ${device}`);
  }

  async function getSystemDiskId(): Promise<string> {
    try {
      if (process.platform === 'darwin') {
        const { stdout } = await runCmd('diskutil info / 2>/dev/null');
        const m = stdout.match(/Part of Whole:\s*(\S+)/);
        return m ? `/dev/${m[1]}` : '';
      } else if (process.platform === 'linux') {
        const { stdout } = await runCmd("lsblk -no PKNAME $(findmnt -n -o SOURCE /) 2>/dev/null || cat /proc/mounts | awk '$2==\"/\"{print $1}' | sed 's/[0-9]*$//' | sed 's/p[0-9]*$//'");
        return stdout.trim() ? `/dev/${stdout.trim().replace(/^\/dev\//, '')}` : '';
      } else if (process.platform === 'win32') {
        const { stdout } = await runCmd('powershell -NoProfile -Command "Get-Partition -DriveLetter C | Get-Disk | Select-Object -ExpandProperty Number"');
        const n = stdout.trim();
        return n ? `\\\\.\\PhysicalDrive${n}` : '';
      }
    } catch {}
    return '';
  }

  async function getDiskPartitionTable(device: string): Promise<'gpt' | 'mbr' | 'unknown'> {
    try {
      if (process.platform === 'darwin') {
        const { stdout } = await runCmd(`diskutil info ${device} 2>/dev/null`);
        if (stdout.match(/Partition Map Scheme:\s*GUID/i) || stdout.match(/Content.*GUID/i)) return 'gpt';
        if (stdout.match(/Partition Map Scheme:\s*.*MBR/i) || stdout.match(/Content.*FDisk/i)) return 'mbr';
      } else if (process.platform === 'linux') {
        const { stdout } = await runCmd(`parted ${device} --script print 2>/dev/null`);
        if (stdout.includes('gpt')) return 'gpt';
        if (stdout.includes('msdos')) return 'mbr';
      } else if (process.platform === 'win32') {
        const diskNum = getWindowsDiskNumber(device);
        if (diskNum) {
          const { stdout } = await runCmd(`powershell -NoProfile -Command "try { (Get-Disk -Number ${diskNum} -ErrorAction Stop).PartitionStyle.ToString().ToUpper() } catch { 'ERROR' }"`);
          const primary = windowsGetDiskStyleOutput(stdout);
          if (primary) return primary;
          // Get-Disk failed or returned an unexpected value (RAW/ERROR).
          // This happens on some USB controllers where the Storage Management API
          // (root\Microsoft\Windows\Storage) doesn't work, but the disk is physically
          // accessible — exactly the case where Disk Management shows GPT but the app
          // was incorrectly blocking with UNKNOWN_PARTITION_TABLE.
          // Fall back to Win32_DiskPartition (root\cimv2 legacy WMI), which is more
          // broadly compatible and is what Disk Management itself uses.
          log('WARN', 'diskOps', `Get-Disk returned "${stdout.trim()}" for disk ${diskNum}, trying WMI fallback`, { device });
          const { stdout: wmiOut } = await runCmd(
            `powershell -NoProfile -Command "try { $p = Get-WmiObject Win32_DiskPartition -Filter 'DiskIndex = ${diskNum}' -ErrorAction Stop | Select-Object -First 1; if ($p) { if ($p.Type -like 'GPT*') { 'GPT' } else { 'MBR' } } else { 'RAW' } } catch { 'ERROR' }"`
          );
          const fallback = windowsWmiDiskStyleOutput(wmiOut);
          if (fallback) return fallback;
          // RAW (no partitions yet) or ERROR — unknown is the correct result
          log('WARN', 'diskOps', `WMI fallback returned "${wmiOut.trim()}" for disk ${diskNum}`, { device });
        } else {
          log('WARN', 'diskOps', 'Could not extract disk number from device path', { device });
        }
      }
    } catch (e) {
      log('WARN', 'diskOps', 'Partition table detection failed', { device, error: (e as Error).message });
    }
    return 'unknown';
  }

  async function getMountedPartitions(device: string): Promise<string[]> {
    const mounts: string[] = [];
    try {
      if (process.platform === 'darwin') {
        const { stdout: listOut } = await runCmd(`diskutil list ${device} 2>/dev/null`);
        const partMatches = listOut.matchAll(/\/dev\/(disk\d+s\d+)/g);
        for (const m of partMatches) {
          try {
            const { stdout: infoOut } = await runCmd(`diskutil info /dev/${m[1]} 2>/dev/null`);
            const mp = infoOut.match(/Mount Point:\s*(.+)/);
            if (mp && mp[1].trim() && mp[1].trim() !== 'Not Mounted') {
              mounts.push(mp[1].trim());
            }
          } catch {}
        }
      } else if (process.platform === 'linux') {
        const { stdout } = await runCmd(`lsblk -no MOUNTPOINT ${device} 2>/dev/null`);
        for (const line of stdout.split('\n')) {
          const mp = line.trim();
          if (mp) mounts.push(mp);
        }
      } else if (process.platform === 'win32') {
        const diskNum = getWindowsDiskNumber(device);
        if (diskNum) {
          const { stdout } = await runCmd(`powershell -NoProfile -Command "Get-Partition -DiskNumber ${diskNum} | Get-Volume | Select-Object DriveLetter | ConvertTo-Json"`);
          try {
            const parsed = JSON.parse(stdout);
            const arr = Array.isArray(parsed) ? parsed : [parsed];
            for (const v of arr) {
              if (v.DriveLetter) mounts.push(`${v.DriveLetter}:`);
            }
          } catch {}
        }
      }
    } catch {}
    return mounts;
  }

  async function getDeviceSize(device: string): Promise<number> {
    try {
      if (process.platform === 'darwin') {
        const { stdout } = await runCmd(`diskutil info ${device} 2>/dev/null`);
        const m = stdout.match(/Disk Size:\s+[\d.]+\s+\w+\s+\((\d+)\s+Bytes\)/);
        if (m) return parseInt(m[1]);
      } else if (process.platform === 'linux') {
        const { stdout } = await runCmd(`lsblk -bno SIZE ${device} 2>/dev/null`);
        return parseInt(stdout.trim()) || 0;
      } else if (process.platform === 'win32') {
        const diskNum = getWindowsDiskNumber(device);
        if (diskNum) {
          const { stdout } = await runCmd(`powershell -NoProfile -Command "(Get-Disk -Number ${diskNum}).Size"`);
          return parseInt(stdout.trim()) || 0;
        }
      }
    } catch { return 0; }
    return 0;
  }

  async function runSafetyChecks(device: string, efiPath: string | null, requiredSpaceBytes?: number): Promise<SafetyCheckResult> {
    const violations: SafetyCheckViolation[] = [];

    // Run checks in parallel where possible
    const [sysDisk, partTable, mountedPartitions, devSize] = await Promise.all([
      getSystemDiskId().catch(() => ''),
      getDiskPartitionTable(device),
      getMountedPartitions(device).catch(() => [] as string[]),
      getDeviceSize(device).catch(() => 0),
    ]);

    // 1. DEVICE_NOT_FOUND
    if (process.platform !== 'win32' && !fs.existsSync(device)) {
      const v: SafetyCheckViolation = {
        code: 'DEVICE_NOT_FOUND',
        severity: 'fatal',
        message: `Device ${device} not found — the drive may have been disconnected`,
      };
      violations.push(v);
      log('WARN', 'diskOps', v.message, { code: v.code });
    }

    // 1b. INSUFFICIENT_SPACE
    if (requiredSpaceBytes && devSize > 0 && devSize < requiredSpaceBytes) {
      const v: SafetyCheckViolation = {
        code: 'INSUFFICIENT_SPACE',
        severity: 'fatal',
        message: `Device ${device} has ${Math.round(devSize / 1e9 * 10) / 10} GB capacity, but this operation requires at least ${Math.round(requiredSpaceBytes / 1e9 * 10) / 10} GB. Please use a larger USB drive.`,
      };
      violations.push(v);
      log('ERROR', 'diskOps', v.message, { code: v.code, devSize, requiredSpaceBytes });
    }

    // 2. SYSTEM_DISK
    const isWindowsSystemDisk = process.platform === 'win32'
      && !!sysDisk
      && !!getWindowsDiskNumber(device)
      && getWindowsDiskNumber(device) === getWindowsDiskNumber(sysDisk);
    if (isWindowsSystemDisk || (sysDisk && process.platform !== 'win32' && (device === sysDisk || device.startsWith(sysDisk + 's') || device.replace(/p?\d+$/, '') === sysDisk))) {
      const v: SafetyCheckViolation = {
        code: 'SYSTEM_DISK',
        severity: 'fatal',
        message: `SAFETY BLOCK: ${device} is your system boot disk — cannot flash the OS drive`,
      };
      violations.push(v);
      log('WARN', 'diskOps', v.message, { code: v.code, sysDisk });
    }

    // 3. UNKNOWN_PARTITION_TABLE — hard block: unreadable partition table means we
    //    cannot guarantee the device is safe to overwrite.
    if (partTable === 'unknown') {
      const v: SafetyCheckViolation = {
        code: 'UNKNOWN_PARTITION_TABLE',
        severity: 'fatal',
        message: `Cannot read partition table for ${device}. The device structure is unrecognisable — writing to it could silently corrupt data on an unidentified disk. Eject and reconnect the drive, or use Disk Utility / diskpart to reformat it as GPT, then retry.`,
      };
      violations.push(v);
      log('ERROR', 'diskOps', v.message, { code: v.code });
    }

    // 4. MBR_PARTITION_TABLE
    if (partTable === 'mbr') {
      const v: SafetyCheckViolation = {
        code: 'MBR_PARTITION_TABLE',
        severity: 'fatal',
        message: `Device ${device} has an MBR partition table. Flash only supports GPT. Convert the disk first or use a different drive.`,
      };
      violations.push(v);
      log('WARN', 'diskOps', v.message, { code: v.code });
    }

    // 5. EFI_MISSING_PLIST
    if (efiPath && !fs.existsSync(path.join(efiPath, 'EFI', 'OC', 'config.plist'))) {
      const v: SafetyCheckViolation = {
        code: 'EFI_MISSING_PLIST',
        severity: 'fatal',
        message: `EFI build at ${efiPath} is missing config.plist — rebuild required`,
      };
      violations.push(v);
      log('WARN', 'diskOps', v.message, { code: v.code, efiPath });
    }

    // 6. EFI_TOO_LARGE
    if (efiPath) {
      const efiSizeBytes = getDirSizeSync(path.join(efiPath, 'EFI'));
      if (efiSizeBytes > 480 * 1024 * 1024) {
        const v: SafetyCheckViolation = {
          code: 'EFI_TOO_LARGE',
          severity: 'fatal',
          message: `EFI directory is ${Math.round(efiSizeBytes / 1024 / 1024)} MB — exceeds FAT32 partition limit of 480 MB. Remove debug drivers and retry.`,
        };
        violations.push(v);
        log('WARN', 'diskOps', v.message, { code: v.code });
      }
    }

    // 7. PARTITION_IN_USE
    if (mountedPartitions.length > 0) {
      const v: SafetyCheckViolation = {
        code: 'PARTITION_IN_USE',
        severity: 'warn',
        message: `Device ${device} has mounted partitions: ${mountedPartitions.join(', ')} — unmount before flashing`,
      };
      violations.push(v);
      log('WARN', 'diskOps', v.message, { code: v.code, mountedPartitions });
    }

    const passed = !violations.some(v => v.severity === 'fatal');
    return { passed, violations };
  }

  async function getDeviceModel(device: string): Promise<string> {
    try {
      if (process.platform === 'darwin') {
        const { stdout } = await runCmd(`diskutil info ${device} 2>/dev/null`);
        const m = stdout.match(/Device \/ Media Name:\s*(.+)/);
        return m?.[1]?.trim() || '';
      } else if (process.platform === 'linux') {
        const { stdout } = await runCmd(`lsblk -no MODEL ${device} 2>/dev/null`);
        return stdout.trim() || '';
      } else if (process.platform === 'win32') {
        const diskNum = getWindowsDiskNumber(device);
        if (diskNum) {
          const { stdout } = await runCmd(`powershell -NoProfile -Command "(Get-Disk -Number ${diskNum}).FriendlyName"`);
          return stdout.trim() || '';
        }
      }
    } catch {}
    return '';
  }

  async function getDiskInfo(device: string): Promise<DiskInfo> {
    const identityPromise: Promise<Partial<DiskInfo>> = process.platform === 'darwin'
      ? getDarwinIdentity(device)
      : process.platform === 'linux'
        ? getLinuxIdentity(device)
        : process.platform === 'win32'
          ? getWindowsIdentity(device)
          : Promise.resolve({});
    const queryTimeoutMs = process.platform === 'win32' ? 5_000 : 6_000;
    const [sysDisk, partTable, mountedPartitions, devSize, model, identity] = await Promise.all([
      withBestEffort(getSystemDiskId(), '', queryTimeoutMs),
      withBestEffort(getDiskPartitionTable(device), 'unknown' as const, queryTimeoutMs),
      withBestEffort(getMountedPartitions(device), [] as string[], queryTimeoutMs),
      withBestEffort(getDeviceSize(device), 0, queryTimeoutMs),
      withBestEffort(getDeviceModel(device), '', queryTimeoutMs),
      withBestEffort(identityPromise, {} as Partial<DiskInfo>, queryTimeoutMs),
    ]);
    const isSystemDisk = process.platform === 'win32'
      ? !!sysDisk && !!getWindowsDiskNumber(device) && getWindowsDiskNumber(device) === getWindowsDiskNumber(sysDisk)
      : !!sysDisk && (device === sysDisk || device.startsWith(sysDisk + 's') || device.replace(/p?\d+$/, '') === sysDisk);
    return {
      device,
      isSystemDisk,
      partitionTable: partTable,
      mountedPartitions,
      sizeBytes: devSize,
      model: identity.model ?? model,
      devicePath: identity.devicePath ?? device,
      vendor: identity.vendor,
      serialNumber: identity.serialNumber,
      transport: identity.transport,
      removable: identity.removable,
      identityConfidence: identity.identityConfidence ?? (identity.serialNumber ? 'strong' : 'weak'),
      identityFieldsUsed: identity.identityFieldsUsed ?? ['devicePath', 'partitionTable', 'sizeBytes'],
    };
  }

  async function waitForGptDiskInfo(device: string, attempts = 10, delayMs = 400): Promise<DiskInfo> {
    let lastInfo = await getDiskInfo(device);
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      if (lastInfo.partitionTable === 'gpt') return lastInfo;
      await new Promise((resolve) => setTimeout(resolve, delayMs));
      lastInfo = await getDiskInfo(device);
    }
    return lastInfo;
  }

  async function prepareWindowsOpenCoreVolume(
    device: string,
    onPhase: (phase: 'erase' | 'format', detail: string) => void,
    registerProcess?: (child: any) => void,
    checkAborted?: () => void,
  ): Promise<{ diskNum: string; driveLetter: string }> {
    const diskNum = getWindowsDiskNumber(device);
    if (!diskNum) throw new Error(`Invalid device path: ${device}`);
    const deviceSizeBytes = await getDeviceSize(device).catch(() => 0);
    const partitionSizeMB = getWindowsFat32PartitionSizeMB(deviceSizeBytes);
    const currentPartitionTable = await getDiskPartitionTable(device).catch(() => 'unknown' as const);
    let driveLetter = '';

    if (canReusePreparedOpenCoreVolume(currentPartitionTable)) {
      driveLetter = await getWindowsDriveLetterForLabel(diskNum, 'OPENCORE', registerProcess);
      if (driveLetter) {
        log('INFO', 'diskOps', 'Found existing OPENCORE volume — reusing prepared drive', { diskNum, driveLetter });
        onPhase('format', `Using existing OPENCORE partition on disk ${diskNum}`);
        return { diskNum, driveLetter };
      }

      const preCheckPartitions = await getWindowsPreparedPartitions(diskNum, registerProcess);
      const reuseCandidate = selectWindowsOpencoreReuseCandidate(preCheckPartitions);
      if (reuseCandidate) {
        log('INFO', 'diskOps', 'Found OPENCORE FAT32 partition with no drive letter — assigning', {
          diskNum,
          partitionNumber: reuseCandidate.partitionNumber,
          sizeBytes: reuseCandidate.sizeBytes,
        });
        onPhase('format', `Assigning drive letter to existing OPENCORE partition on disk ${diskNum}`);
        driveLetter = await assignWindowsDriveLetter(
          diskNum, reuseCandidate.partitionNumber, 'OPENCORE', registerProcess,
        );
        if (driveLetter) {
          log('INFO', 'diskOps', 'Reused existing OPENCORE partition', { diskNum, driveLetter });
          return { diskNum, driveLetter };
        }
      }
    }

    const mountedPartitions = await getMountedPartitions(device).catch(() => [] as string[]);
    if (mountedPartitions.length > 0) {
      onPhase('erase', `Detaching mounted volumes from disk ${diskNum}`);
      log('WARN', 'diskOps', 'Detaching mounted Windows volumes before GPT conversion/flash prep', {
        diskNum,
        mountedPartitions,
      });
      await detachWindowsDriveLetters(diskNum, registerProcess);
      await new Promise((resolve) => setTimeout(resolve, 500));
    }

    // Phase 1: partition creation script (uses noerr for safety on clean/convert)
    const phase1Script = buildWindowsConvertToGptDiskpartScript(diskNum, partitionSizeMB);
    const phase1Path = path.join(os.tmpdir(), `oc-diskpart-p1-${crypto.randomUUID()}.txt`);
    fs.writeFileSync(phase1Path, phase1Script);

    let lastDiskpartError: Error | null = null;
    let lastAssessment: WindowsFlashPreparationAssessment = {
      status: 'failed',
      stage: 'create-partition',
      driveLetter: '',
      targetPartitionNumber: null,
      usedPartitionFallback: false,
    };
    let lastObservedPartitions: WindowsFlashPreparedPartition[] = [];
    const maxDiskpartAttempts = 3;
    const retryDelays = [500, 1500, 3000];

    try {
      for (let attempt = 0; attempt < maxDiskpartAttempts; attempt += 1) {
        onPhase('format', attempt === 0
          ? `Running diskpart on disk ${diskNum}`
          : `Retrying disk ${diskNum} after clearing stale Windows mounts`);
        checkAborted?.();
        log('DEBUG', 'diskOps', 'Running diskpart phase 1 (partition creation) for GPT conversion / flash prep', {
          diskNum,
          attempt: attempt + 1,
        });
        let diskpartFailed = false;
        try {
          await runCmd(`diskpart /s "${phase1Path}"`, registerProcess);
          lastDiskpartError = null;
        } catch (error) {
          diskpartFailed = true;
          lastDiskpartError = error as Error;
          log('WARN', 'diskOps', 'diskpart reported an error during GPT conversion / flash prep', {
            diskNum,
            attempt: attempt + 1,
            error: (error as Error).message,
          });
        }

        // Wait for Windows to register the new partition before attempting format
        await new Promise((resolve) => setTimeout(resolve, 1000));

        lastObservedPartitions = await waitForWindowsPreparedPartitions(
          diskNum,
          registerProcess,
          12,
          500,
        );
        lastAssessment = assessWindowsFlashPreparationState({
          partitions: lastObservedPartitions,
          expectedLabel: 'OPENCORE',
        });

        // Phase 2: If partition exists but is not formatted, run format as a separate
        // diskpart invocation WITHOUT noerr so format errors surface (#38, #41).
        if (lastAssessment.status === 'failed' && lastAssessment.stage === 'format') {
          const partNum = lastAssessment.targetPartitionNumber ?? 1;
          log('INFO', 'diskOps', 'Partition created but not formatted — running phase 2 format', {
            diskNum, partNum, attempt: attempt + 1,
          });

          // Clear handle locks: Explorer/Shell auto-mounts new partitions immediately
          await detachWindowsDriveLetters(diskNum, registerProcess).catch(() => {});
          await new Promise((resolve) => setTimeout(resolve, 500));

          // Phase 2 diskpart: format WITHOUT noerr
          const phase2Script = buildWindowsFormatDiskpartScript(diskNum, partNum);
          const phase2Path = path.join(os.tmpdir(), `oc-diskpart-p2-${crypto.randomUUID()}.txt`);
          fs.writeFileSync(phase2Path, phase2Script);
          try {
            onPhase('format', `Formatting partition ${partNum} on disk ${diskNum}`);
            await runCmd(`diskpart /s "${phase2Path}"`, registerProcess);
            log('INFO', 'diskOps', 'Phase 2 format succeeded', { diskNum, partNum });
          } catch (fmtErr) {
            log('WARN', 'diskOps', 'Phase 2 diskpart format failed', {
              diskNum, partNum, error: (fmtErr as Error).message,
            });
          } finally {
            try { fs.unlinkSync(phase2Path); } catch {}
          }

          // Re-assess after phase 2
          await new Promise((resolve) => setTimeout(resolve, 500));
          lastObservedPartitions = await waitForWindowsPreparedPartitions(diskNum, registerProcess, 10, 500);
          lastAssessment = assessWindowsFlashPreparationState({
            partitions: lastObservedPartitions,
            expectedLabel: 'OPENCORE',
          });
        }

        if (lastAssessment.status === 'failed' && lastAssessment.stage === 'assign') {
          const assignedLetter = await assignWindowsDriveLetter(
            diskNum,
            lastAssessment.targetPartitionNumber,
            'OPENCORE',
            registerProcess,
          );
          if (assignedLetter) {
            lastObservedPartitions = await getWindowsPreparedPartitions(diskNum, registerProcess);
            lastAssessment = assessWindowsFlashPreparationState({
              partitions: lastObservedPartitions,
              expectedLabel: 'OPENCORE',
            });
          }
        }

        if (lastAssessment.status === 'ready') {
          driveLetter = lastAssessment.driveLetter;
          if (lastAssessment.usedPartitionFallback) {
            log('WARN', 'diskOps', 'Using partition-based drive letter fallback after OPENCORE label lookup failed', {
              diskNum,
              partitionNumber: lastAssessment.targetPartitionNumber,
              driveLetter,
              partitions: lastObservedPartitions,
            });
          }
          break;
        }

        if (!shouldRetryWindowsFlashPreparation({
          attempt,
          maxAttempts: maxDiskpartAttempts,
          diskpartFailed,
          driveLetter: '',
          stage: lastAssessment.stage,
        })) {
          break;
        }

        log('WARN', 'diskOps', 'Retrying diskpart after Windows GPT conversion / flash prep failure', {
          diskNum,
          attempt: attempt + 1,
          stage: lastAssessment.stage,
          partitions: lastObservedPartitions,
        });
        await detachWindowsDriveLetters(diskNum, registerProcess).catch(() => {});
        await new Promise((resolve) => setTimeout(resolve, retryDelays[attempt] ?? 1500));
      }
    } finally {
      try { fs.unlinkSync(phase1Path); } catch {}
    }

    if (driveLetter) return { diskNum, driveLetter };

    if (lastAssessment.stage === 'create-partition') {
      throw new Error(
        `diskpart failed to create a partition on disk ${diskNum}. ` +
        'The selected disk never produced a visible partition after the cleanup and GPT conversion steps. ' +
        'Close all programs using this drive, unplug and reconnect it, then try again. ' +
        'If it keeps failing, open an elevated Command Prompt and run: diskpart → select disk ' + diskNum + ' → clean → convert gpt → create partition primary → select partition 1 → format fs=fat32 quick label=OPENCORE → assign'
      );
    }

    if (lastAssessment.stage === 'format') {
      const partNum = lastAssessment.targetPartitionNumber ?? 1;
      log('WARN', 'diskOps', 'All diskpart attempts left partition unformatted — attempting Format-Volume fallback', { diskNum, partNum });
      onPhase('format', `Recovering: formatting partition ${partNum} on disk ${diskNum}`);

      // Clear handles before Format-Volume — Explorer/Shell grabs new partitions immediately
      await detachWindowsDriveLetters(diskNum, registerProcess).catch(() => {});
      await new Promise((resolve) => setTimeout(resolve, 800));

      let formatRecovered = false;
      try {
        await runCmd(
          `powershell -NoProfile -Command "try { Format-Volume -DiskNumber ${diskNum} -PartitionNumber ${partNum} -FileSystem FAT32 -NewFileSystemLabel 'OPENCORE' -Confirm:$false -Force -ErrorAction Stop } catch { throw }"`,
          registerProcess,
        );
        const recoveredPartitions = await waitForWindowsPreparedPartitions(diskNum, registerProcess, 10, 500);
        const recoveredAssessment = assessWindowsFlashPreparationState({
          partitions: recoveredPartitions,
          expectedLabel: 'OPENCORE',
        });
        if (recoveredAssessment.status === 'ready') {
          driveLetter = recoveredAssessment.driveLetter;
          formatRecovered = true;
        } else if (recoveredAssessment.stage === 'assign') {
          const assignedLetter = await assignWindowsDriveLetter(
            diskNum,
            recoveredAssessment.targetPartitionNumber,
            'OPENCORE',
            registerProcess,
          );
          if (assignedLetter) {
            driveLetter = assignedLetter;
            formatRecovered = true;
          }
        }
      } catch (error) {
        log('WARN', 'diskOps', 'Format-Volume fallback also failed', {
          diskNum,
          partNum,
          error: (error as Error).message,
        });
      }
      if (formatRecovered && driveLetter) return { diskNum, driveLetter };
      throw new Error(
        `diskpart created a partition on disk ${diskNum}, but failed to format it as FAT32 OPENCORE. ` +
        'Both diskpart format and PowerShell Format-Volume fallback failed. ' +
        'Stage: partition exists → format failed → Format-Volume fallback also failed. ' +
        'Close Explorer windows, antivirus scans, or backup tools touching this drive, then retry. ' +
        'If it keeps failing, format partition 1 manually as FAT32 and label it OPENCORE before retrying.'
      );
    }

    if (lastAssessment.stage === 'assign') {
      log('WARN', 'diskOps', 'All diskpart attempts left no drive letter — final aggressive assign', {
        diskNum,
        partitionNumber: lastAssessment.targetPartitionNumber,
      });
      const lastResortLetter = await assignWindowsDriveLetter(
        diskNum,
        lastAssessment.targetPartitionNumber,
        'OPENCORE',
        registerProcess,
      );
      if (lastResortLetter) {
        log('INFO', 'diskOps', 'Final aggressive assign succeeded', { diskNum, driveLetter: lastResortLetter });
        return { diskNum, driveLetter: lastResortLetter };
      }
      throw new Error(
        `Disk ${diskNum} has a FAT32 OPENCORE partition, but Windows did not assign a drive letter after multiple attempts (diskpart, Set-Partition, Add-PartitionAccessPath). ` +
        'This usually means another process is holding a lock on the new volume, or a stale mount entry is blocking assignment. ' +
        'Unplug the drive, wait 10 seconds, reconnect it, and try again. ' +
        'If it keeps failing, open Disk Management (diskmgmt.msc) and manually assign a drive letter to partition 1.'
      );
    }

    if (lastAssessment.stage === 'label-lookup') {
      throw new Error(
        `Disk ${diskNum} has a FAT32 partition with a drive letter, but the OPENCORE label could not be confirmed. ` +
        'The app will not guess at an unlabeled volume on a destructive path. ' +
        'Rename the new volume to OPENCORE in Disk Management, then retry.'
      );
    }

    throw new Error(
      lastDiskpartError
        ? `diskpart could not prepare disk ${diskNum} (stage: ${lastAssessment.stage}). ${lastDiskpartError.message}`
        : `diskpart could not prepare disk ${diskNum} (stage: ${lastAssessment.stage}). Close all programs using this drive, unplug and reconnect it, then try again.`
    );
  }

  async function inspectExistingEfi(device: string): Promise<ExistingEfiInspection> {
    const result = await withMountedExistingEfi(device, async ({ mountPoint }) => {
      const efiPath = path.join(mountPoint, 'EFI');
      try {
        if (!fs.existsSync(efiPath) || !fs.statSync(efiPath).isDirectory()) {
          return { status: 'absent' as const };
        }
        fs.readdirSync(efiPath);
        return { status: 'readable' as const };
      } catch (error) {
        return {
          status: 'unreadable' as const,
          reason: `Existing EFI could not be read from ${mountPoint}: ${(error as Error).message}`,
        };
      }
    });

    if (result.kind === 'absent') return { status: 'absent' };
    if (result.kind === 'error') return { status: 'unreadable', reason: result.reason };
    return result.value;
  }

  async function copyExistingEfi(device: string, destinationPath: string): Promise<ExistingEfiCopyResult> {
    const result = await withMountedExistingEfi(device, async ({ mountPoint }) => {
      const sourceEfiPath = path.join(mountPoint, 'EFI');
      if (!fs.existsSync(sourceEfiPath) || !fs.statSync(sourceEfiPath).isDirectory()) {
        return { status: 'absent' as const, configHash: null };
      }

      try {
        const destinationEfiPath = path.join(destinationPath, 'EFI');
        fs.rmSync(destinationEfiPath, { recursive: true, force: true });
        copyDirContents(sourceEfiPath, destinationEfiPath);
        const configPath = path.join(destinationEfiPath, 'OC', 'config.plist');
        return {
          status: 'readable' as const,
          configHash: fs.existsSync(configPath) ? hashFile(configPath) : null,
        };
      } catch (error) {
        return {
          status: 'unreadable' as const,
          reason: `Existing EFI could not be copied from ${mountPoint}: ${(error as Error).message}`,
          configHash: null,
        };
      }
    });

    if (result.kind === 'absent') return { status: 'absent', configHash: null };
    if (result.kind === 'error') return { status: 'unreadable', reason: result.reason, configHash: null };
    return result.value;
  }

  async function convertDiskToGpt(options: ConvertDiskToGptOptions): Promise<DiskInfo> {
    if (!options.confirmed) {
      throw new Error('SAFETY BLOCK: convertDiskToGpt requires explicit user confirmation (confirmed=true)');
    }

    const { device, onPhase, registerProcess } = options;
    log('INFO', 'diskOps', 'Converting disk to GPT inside app', { device, platform: process.platform });

    const initialInfo = await getDiskInfo(device);
    if (initialInfo.isSystemDisk) {
      throw new Error(`SAFETY BLOCK: ${device} is the system/boot disk — cannot erase and convert it to GPT`);
    }
    if (initialInfo.partitionTable === 'unknown') {
      throw new Error(`SAFETY BLOCK: Cannot read partition table for ${device} — refusing to erase an unidentified disk`);
    }
    if (initialInfo.partitionTable === 'gpt') {
      return initialInfo;
    }

    if (process.platform === 'darwin') {
      onPhase('erase', `Unmounting ${device}`);
      await runCmd(`diskutil unmountDisk ${device}`, registerProcess);
      onPhase('format', `Erasing ${device} as FAT32 GPT`);
      await runCmd(`diskutil eraseDisk FAT32 OPENCORE GPTFormat ${device}`, registerProcess);
    } else if (process.platform === 'linux') {
      const part = buildLinuxFirstPartitionPath(device);
      onPhase('erase', `Unmounting mounted volumes from ${device}`);
      await runCmd(elevateCommand(`umount ${device}* 2>/dev/null || true`), registerProcess);
      onPhase('format', `Partitioning and formatting ${device} as GPT/FAT32`);
      await runCmd(elevateCommand(`parted ${device} --script mklabel gpt mkpart primary fat32 1MiB 100%`), registerProcess);
      for (let attempt = 0; attempt < 10; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, 400));
        if (fs.existsSync(part)) break;
      }
      if (!fs.existsSync(part)) throw new Error(`Partition ${part} did not appear after partitioning — try again`);
      await runCmd(elevateCommand(`mkfs.fat -F 32 -n OPENCORE ${part}`), registerProcess);
      await runCmd('sync', registerProcess).catch(() => {});
    } else if (process.platform === 'win32') {
      onPhase('erase', `Preparing ${device} for GPT conversion`);
      await prepareWindowsOpenCoreVolume(
        device,
        (phase, detail) => onPhase(phase, detail),
        registerProcess,
      );
    } else {
      throw new Error(`convertDiskToGpt: unsupported platform ${process.platform}`);
    }

    onPhase('verify', `Verifying GPT partition table on ${device}`);
    const refreshedInfo = await waitForGptDiskInfo(device);
    if (refreshedInfo.partitionTable !== 'gpt') {
      throw new Error(`GPT conversion completed, but ${device} did not report a GPT partition table afterwards`);
    }
    return refreshedInfo;
  }

  async function flashUsb(options: FlashUsbOptions): Promise<void> {
    // Second line of defence: diskOps must never write without explicit confirmation,
    // regardless of which code path called it.
    if (!options.confirmed) {
      throw new Error('SAFETY BLOCK: flashUsb requires explicit user confirmation (confirmed=true)');
    }
    const { device, efiPath, onPhase, checkAborted, registerProcess } = options;
    log('INFO', 'usb-flash', 'Starting USB flash', { device, efiPath, platform: process.platform });

    await sim.failIf('disk:write-fail', 'Target device rejected write block at offset 0');
    await sim.failIf('usb:disconnect', 'USB device was disconnected during initialisation');

    try {
      if (process.platform === 'darwin') {
        let mountedIdentifier: string | null = null;
        try {
          onPhase('erase', `Unmounting ${device}`);
          checkAborted();
          log('DEBUG', 'usb-flash', 'Unmounting disk', { device });
          await runCmd(`diskutil unmountDisk ${device}`, registerProcess);

          onPhase('format', `Erasing ${device} as FAT32 GPT`);
          checkAborted();
          log('DEBUG', 'usb-flash', 'Erasing disk as FAT32 GPT', { device });
          await runCmd(`diskutil eraseDisk FAT32 OPENCORE GPTFormat ${device}`, registerProcess);

          const { identifier, mountPoint } = await resolveDarwinMountedVolume(device, 'OPENCORE', registerProcess);
          mountedIdentifier = identifier;

          onPhase('copy', `Copying EFI to ${mountPoint}`);
          checkAborted();
          log('DEBUG', 'usb-flash', 'Copying EFI', { mountPoint });
          await runCmd(`cp -r "${path.join(efiPath, 'EFI')}" "${mountPoint}/"`, registerProcess);

          // Copy recovery payload if present
          const recoveryDir = path.join(efiPath, 'com.apple.recovery.boot');
          if (fs.existsSync(recoveryDir)) {
            onPhase('copy', `Copying recovery payload to ${mountPoint}`);
            checkAborted();
            log('DEBUG', 'usb-flash', 'Copying recovery payload', { mountPoint });
            await runCmd(`cp -r "${recoveryDir}" "${mountPoint}/"`, registerProcess);
          }

          // Verify: EFI must exist on target
          onPhase('verify', 'Verifying written files');
          if (!fs.existsSync(path.join(mountPoint, 'EFI', 'OC', 'OpenCore.efi'))) {
            throw new Error('Verification failed: EFI/OC/OpenCore.efi not found on USB after copy');
          }
          if (fs.existsSync(recoveryDir) && !fs.existsSync(path.join(mountPoint, 'com.apple.recovery.boot', 'BaseSystem.dmg'))) {
            throw new Error('Verification failed: com.apple.recovery.boot/BaseSystem.dmg not found on USB after copy');
          }

          onPhase('eject', `Ejecting ${device}`);
          await runCmd(`diskutil eject ${device}`, registerProcess);
          mountedIdentifier = null;
        } finally {
          if (mountedIdentifier) {
            try { await runCmd(`diskutil unmount ${mountedIdentifier}`, registerProcess); } catch (_) {}
          }
        }
      } else if (process.platform === 'linux') {
        const part = buildLinuxFirstPartitionPath(device);
        const tmpMount = path.join(os.tmpdir(), `oc_usb_${Date.now()}`);

        onPhase('format', `Partitioning and formatting ${device}`);
        checkAborted();
        log('DEBUG', 'usb-flash', 'Partitioning and formatting', { device, part });
        await runCmd(elevateCommand(`umount ${device}* 2>/dev/null || true`), registerProcess);
        await runCmd(elevateCommand(`parted ${device} --script mklabel gpt mkpart primary fat32 1MiB 100%`), registerProcess);

        for (let i = 0; i < 10; i++) {
          await new Promise(r => setTimeout(r, 400));
          if (fs.existsSync(part)) break;
        }
        if (!fs.existsSync(part)) throw new Error(`Partition ${part} did not appear after partitioning — try again`);
        await runCmd(elevateCommand(`mkfs.fat -F 32 -n OPENCORE ${part}`), registerProcess);

        fs.mkdirSync(tmpMount, { recursive: true });
        onPhase('copy', `Copying EFI to ${tmpMount}`);
        checkAborted();
        try {
          await runCmd(elevateCommand(`mount ${part} ${tmpMount}`), registerProcess);
          log('DEBUG', 'usb-flash', 'Copying EFI to USB', { tmpMount });
          await runCmd(elevateCommand(`cp -r "${path.join(efiPath, 'EFI')}" "${tmpMount}/"`), registerProcess);

          // Copy recovery payload if present — non-fatal so EFI flash still succeeds
          const recoveryDir = path.join(efiPath, 'com.apple.recovery.boot');
          let recoveryCopyOk = false;
          if (fs.existsSync(recoveryDir)) {
            onPhase('copy', `Copying recovery payload to ${tmpMount}`);
            checkAborted();
            log('DEBUG', 'usb-flash', 'Copying recovery payload', { tmpMount });
            try {
              await runCmd(elevateCommand(`cp -r "${recoveryDir}" "${tmpMount}/"`), registerProcess);
              recoveryCopyOk = true;
            } catch (recErr) {
              log('WARN', 'usb-flash', 'Recovery payload copy failed — EFI is still valid without it', {
                tmpMount,
                error: (recErr as Error).message,
              });
            }
          }

          await runCmd('sync', registerProcess);

          // Verify: EFI must exist on target
          onPhase('verify', 'Verifying written files');
          if (!fs.existsSync(path.join(tmpMount, 'EFI', 'OC', 'OpenCore.efi'))) {
            throw new Error('Verification failed: EFI/OC/OpenCore.efi not found on USB after copy');
          }
          if (recoveryCopyOk && !fs.existsSync(path.join(tmpMount, 'com.apple.recovery.boot', 'BaseSystem.dmg'))) {
            log('WARN', 'usb-flash', 'Recovery payload was copied but BaseSystem.dmg not found on target — recovery may be incomplete', { tmpMount });
          }
        } finally {
          onPhase('eject', `Unmounting ${tmpMount}`);
          try { await runCmd(elevateCommand(`umount ${tmpMount}`), registerProcess); log('DEBUG', 'usb-flash', 'Unmounted'); } catch (_) {}
          try { fs.rmdirSync(tmpMount); } catch (_) {}
        }

      } else if (process.platform === 'win32') {
        const { diskNum, driveLetter } = await prepareWindowsOpenCoreVolume(
          device,
          (phase, detail) => onPhase(phase, detail),
          registerProcess,
          checkAborted,
        );

        try {
          onPhase('copy', `Copying EFI to ${driveLetter}:`);
          checkAborted();
          log('DEBUG', 'usb-flash', 'Copying EFI', { driveLetter });
          await runCmd(`xcopy /E /I /H /Y "${path.join(efiPath, 'EFI')}" "${driveLetter}:\\EFI"`, registerProcess);

          // Copy recovery payload if present — non-fatal so EFI flash still succeeds
          const recoveryDir = path.join(efiPath, 'com.apple.recovery.boot');
          let recoveryCopyOk = false;
          if (fs.existsSync(recoveryDir)) {
            onPhase('copy', `Copying recovery payload to ${driveLetter}:`);
            checkAborted();
            log('DEBUG', 'usb-flash', 'Copying recovery payload', { driveLetter });
            try {
              await runCmd(`xcopy /E /I /H /Y "${recoveryDir}" "${driveLetter}:\\com.apple.recovery.boot"`, registerProcess);
              recoveryCopyOk = true;
            } catch (recErr) {
              log('WARN', 'usb-flash', 'Recovery payload copy failed — EFI is still valid without it', {
                driveLetter,
                error: (recErr as Error).message,
              });
            }
          }

          // Verify: EFI must exist on target
          onPhase('verify', 'Verifying written files');
          if (!fs.existsSync(path.join(`${driveLetter}:`, 'EFI', 'OC', 'OpenCore.efi'))) {
            throw new Error('Verification failed: EFI\\OC\\OpenCore.efi not found on USB after copy');
          }
          if (recoveryCopyOk && !fs.existsSync(path.join(`${driveLetter}:`, 'com.apple.recovery.boot', 'BaseSystem.dmg'))) {
            log('WARN', 'usb-flash', 'Recovery payload was copied but BaseSystem.dmg not found on target — recovery may be incomplete', { driveLetter });
          }
        } catch (e) {
          throw e;
        }
      } else {
        throw new Error(`flashUsb: unsupported platform ${process.platform}`);
      }
    } catch (e) {
      log('ERROR', 'usb-flash', 'Flash failed', { device, error: (e as Error).message });
      throw e;
    }

    log('INFO', 'usb-flash', 'USB flash complete', { device });
  }

  async function shrinkPartition(disk: string, sizeGB: number, confirmed: boolean): Promise<void> {
    if (!confirmed) {
      throw new Error('SAFETY BLOCK: shrinkPartition requires explicit user confirmation (confirmed=true)');
    }
    if (process.platform === 'win32') {
      const diskNum = getWindowsDiskNumber(disk);
      if (!diskNum) throw new Error(`Invalid disk identifier: ${disk}`);
      const partitionNum = await getWindowsPrimaryPartitionNumber(diskNum);
      if (!partitionNum) throw new Error(
        `Could not determine the primary data partition for disk ${diskNum}. ` +
        'The disk has no basic data partition of 20 GB or more that qualifies as a shrink target. ' +
        'Confirm the correct disk is selected, that it has a main Windows installation partition (C:) ' +
        'larger than 20 GB, and that it is not an EFI, MSR, or recovery-only partition.',
      );
      const script = `select disk ${diskNum}\nselect partition ${partitionNum}\nshrink desired=${sizeGB * 1024} minimum=8192\n`;
      const scriptPath = path.join(os.tmpdir(), `shrink-${crypto.randomUUID()}.txt`);
      fs.writeFileSync(scriptPath, script);
      try {
        await runCmd(`diskpart /s "${scriptPath}"`);
      } finally {
        try { fs.unlinkSync(scriptPath); } catch {}
      }
    } else if (process.platform === 'darwin') {
      const { container } = await resolveDarwinApfsContainer(disk);
      await runCmd(`diskutil apfs resizeContainer ${container} ${sizeGB}g`);
    } else if (process.platform === 'linux') {
      throw new Error('Partition shrink is not supported on Linux in this build');
    }
  }

  async function createBootPartition(options: CreateBootPartitionOptions): Promise<void> {
    if (!options.confirmed) {
      throw new Error('SAFETY BLOCK: createBootPartition requires explicit user confirmation (confirmed=true)');
    }
    const { disk, efiPath, onPhase, registerProcess } = options;
    log('INFO', 'part', 'Creating boot partition', { disk, platform: process.platform });

    if (process.platform === 'win32') {
      const diskNum = getWindowsDiskNumber(disk);
      if (!diskNum) throw new Error(`Invalid disk identifier: ${disk}`);
      const script = buildWindowsBootPartitionDiskpartScript(diskNum);
      const scriptPath = path.join(os.tmpdir(), `create-part-${crypto.randomUUID()}.txt`);
      fs.writeFileSync(scriptPath, script);

      try {
        onPhase('create-partition', `Creating partition on disk ${diskNum}`);
        await runCmd(`diskpart /s "${scriptPath}"`, registerProcess);

        // Resolve drive letter from the specific disk, not a global label search
        const driveLetter = await waitForWindowsDriveLetterForLabel(diskNum, 'BOOTSTRAP', registerProcess);
        if (!driveLetter) throw new Error(`Could not determine BOOTSTRAP drive letter for disk ${diskNum} — Windows has not mounted the new volume yet`);

        onPhase('copy', `Copying EFI to ${driveLetter}:`);
        await runCmd(`xcopy /E /Y /I /H "${path.join(efiPath, 'EFI')}" "${driveLetter}:\\EFI"`, registerProcess);
      } finally {
        try { fs.unlinkSync(scriptPath); } catch {}
      }

    } else if (process.platform === 'darwin') {
      let mountedIdentifier: string | null = null;
      onPhase('create-partition', `Creating BOOTSTRAP partition on ${disk}`);
      try {
        await runCmd(`diskutil addPartition ${disk} MS-DOS BOOTSTRAP 16G`, registerProcess);
        const { identifier, mountPoint } = await resolveDarwinMountedVolume(disk, 'BOOTSTRAP', registerProcess);
        mountedIdentifier = identifier;

        onPhase('copy', `Copying EFI to ${mountPoint}`);
        await runCmd(`cp -r "${path.join(efiPath, 'EFI')}" "${mountPoint}/"`, registerProcess);
      } finally {
        if (mountedIdentifier) {
          try { await runCmd(`diskutil unmount ${mountedIdentifier}`, registerProcess); } catch {}
        }
      }

    } else if (process.platform === 'linux') {
      const beforePartitions = await listLinuxPartitions(disk, registerProcess);
      onPhase('create-partition', `Creating FAT32 partition on ${disk}`);
      await runCmd(`parted ${disk} --script mkpart primary fat32 -16GiB 100%`, registerProcess);

      let partName = '';
      for (let i = 0; i < 10; i++) {
        await new Promise(r => setTimeout(r, 300));
        const afterPartitions = await listLinuxPartitions(disk, registerProcess);
        partName = afterPartitions.find(name => !beforePartitions.includes(name)) || '';
        if (partName) break;
      }
      if (!partName) {
        throw new Error(`Could not identify new partition on ${disk} after partition creation`);
      }
      const part = `/dev/${partName}`;

      onPhase('format', `Formatting ${part} as FAT32`);
      await runCmd(`mkfs.fat -F 32 -n BOOTSTRAP ${part}`, registerProcess);

      const tmpMount = path.join(os.tmpdir(), `oc_boot_${Date.now()}`);
      fs.mkdirSync(tmpMount, { recursive: true });

      onPhase('mount', `Mounting ${part} at ${tmpMount}`);
      try {
        await runCmd(`mount ${part} ${tmpMount}`, registerProcess);

        onPhase('copy', `Copying EFI to ${tmpMount}`);
        await runCmd(`cp -r "${path.join(efiPath, 'EFI')}" "${tmpMount}/"`, registerProcess);
        await runCmd('sync', registerProcess);
      } finally {
        onPhase('unmount', `Unmounting ${tmpMount}`);
        try { await runCmd(`umount ${tmpMount}`, registerProcess); } catch (_) {}
        try { fs.rmdirSync(tmpMount); } catch (_) {}
      }
    } else {
      throw new Error(`createBootPartition: unsupported platform ${process.platform}`);
    }

    log('INFO', 'part', 'Boot partition created', { disk });
  }

  async function getHardDrives(): Promise<Array<{ name: string; device: string; size: string; type: string }>> {
    const drives: Array<{ name: string; device: string; size: string; type: string }> = [];
    if (process.platform === 'win32') {
      const disks = await listWindowsDisks();
      for (const disk of disks) {
        if (isWindowsUsbLikeDisk({
          busType: disk.BusType,
          interfaceType: disk.InterfaceType,
          pnpDeviceId: disk.PNPDeviceID,
          mediaType: disk.MediaType,
          isBoot: disk.IsBoot,
          isSystem: disk.IsSystem,
        })) continue;
        drives.push({
          name: disk.FriendlyName || disk.Model || `Disk ${disk.Number}`,
          device: buildWindowsPhysicalDrivePath(disk.Number),
          size: typeof disk.Size === 'number' ? `${(disk.Size / 1e9).toFixed(1)} GB` : 'Unknown',
          type: disk.BusType || disk.InterfaceType || 'Disk',
        });
      }
    } else if (process.platform === 'darwin') {
      const { stdout } = await runCmd('diskutil list internal physical');
      const matches = stdout.matchAll(/\/dev\/(disk\d+)/g);
      for (const m of matches) {
        const info = await runCmd(`diskutil info ${m[1]}`);
        const name = info.stdout.match(/Device \/ Media Name:\s*(.+)/)?.[1]?.trim() || m[1];
        const size = info.stdout.match(/Disk Size:\s*(.+?)(?:\s*\(|$)/)?.[1]?.trim() || 'Unknown';
        drives.push({ name, device: m[1], size, type: 'Internal' });
      }
    }
    return drives;
  }

  async function listUsbDevices(): Promise<Array<{ name: string; device: string; size: string }>> {
    const drives: Array<{ name: string; device: string; size: string }> = [];

    if (process.platform === 'darwin') {
      try {
        const { stdout } = await runCmd('diskutil list external physical');
        const diskMatches = stdout.matchAll(/\/dev\/(disk\d+)/g);
        for (const match of diskMatches) {
          const device = `/dev/${match[1]}`;
          try {
            const info = await runCmd(`diskutil info ${device}`);
            const nameMatch = info.stdout.match(/Device \/ Media Name:\s*(.+)/);
            const sizeMatch = info.stdout.match(/Disk Size:\s*(.+?)(?:\s*\(|$)/);
            drives.push({
              name: nameMatch?.[1]?.trim() || device,
              device,
              size: sizeMatch?.[1]?.trim() || 'Unknown'
            });
          } catch {}
        }
      } catch {}
    } else if (process.platform === 'linux') {
      try {
        const { stdout } = await runCmd('lsblk -J -o NAME,SIZE,RM,TYPE,MODEL');
        const parsed = JSON.parse(stdout);
        for (const dev of parsed.blockdevices || []) {
          if (dev.rm && dev.type === 'disk') {
            drives.push({
              name: dev.model || dev.name,
              device: `/dev/${dev.name}`,
              size: dev.size
            });
          }
        }
      } catch {}
    } else if (process.platform === 'win32') {
      const disks = await listWindowsDisks();
      for (const disk of disks) {
        if (!isWindowsUsbLikeDisk({
          busType: disk.BusType,
          interfaceType: disk.InterfaceType,
          pnpDeviceId: disk.PNPDeviceID,
          mediaType: disk.MediaType,
          isBoot: disk.IsBoot,
          isSystem: disk.IsSystem,
        })) continue;
        drives.push({
          name: disk.FriendlyName || disk.Model || `Disk ${disk.Number}`,
          device: buildWindowsPhysicalDrivePath(disk.Number),
          size: typeof disk.Size === 'number' ? `${(disk.Size / 1e9).toFixed(1)} GB` : 'Unknown',
        });
      }
    }

    return drives;
  }

  return {
    getSystemDiskId,
    getDiskPartitionTable,
    getMountedPartitions,
    getDeviceSize,
    runSafetyChecks,
    getDiskInfo,
    inspectExistingEfi,
    copyExistingEfi,
    flashUsb,
    convertDiskToGpt,
    shrinkPartition,
    createBootPartition,
    getHardDrives,
    listUsbDevices,
  };
}

// Re-export getFreeSpaceMB for use in main.ts preflight
export { getFreeSpaceMB };
