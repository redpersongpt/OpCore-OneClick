import path from 'path';
import fs from 'fs';
import os from 'os';
import util from 'util';
import crypto from 'node:crypto';
import { exec } from 'child_process';

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
  shrinkPartition(disk: string, sizeGB: number): Promise<void>;
  createBootPartition(options: CreateBootPartitionOptions): Promise<void>;
  getHardDrives(): Promise<Array<{ name: string; device: string; size: string; type: string }>>;
  listUsbDevices(): Promise<Array<{ name: string; device: string; size: string }>>;
}

import { sim } from './simulation.js';

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

  function escapeRegExp(value: string): string {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  function getWindowsDiskNumber(device: string): string | null {
    return device.match(/PhysicalDrive(\d+)/i)?.[1]
      || device.match(/^disk(\d+)$/i)?.[1]
      || null;
  }

  async function getWindowsDriveLetterForLabel(diskNum: string, label: string, register?: (child: any) => void): Promise<string> {
    const { stdout } = await runCmd(
      `powershell -NoProfile -Command "(Get-Partition -DiskNumber ${diskNum} | Get-Volume | Where-Object { $_.FileSystemLabel -eq '${label}' }).DriveLetter"`,
      register
    );
    return stdout.trim();
  }

  async function getWindowsPrimaryPartitionNumber(diskNum: string): Promise<string> {
    const { stdout } = await runCmd(
      `powershell -NoProfile -Command "(Get-Partition -DiskNumber ${diskNum} | Where-Object { $_.Type -eq 'Basic' -and $_.Size -gt 50GB } | Sort-Object -Property Size -Descending | Select-Object -First 1 -ExpandProperty PartitionNumber)"`
    );
    return stdout.trim();
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
        const diskNum = device.match(/PhysicalDrive(\d+)/)?.[1];
        if (diskNum) {
          const { stdout } = await runCmd(`powershell -NoProfile -Command "Get-Disk -Number ${diskNum} | Select-Object -ExpandProperty PartitionStyle"`);
          if (stdout.trim().toUpperCase() === 'GPT') return 'gpt';
          if (stdout.trim().toUpperCase() === 'MBR') return 'mbr';
        }
      }
    } catch {}
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
        const diskNum = device.match(/PhysicalDrive(\d+)/)?.[1];
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
        const diskNum = device.match(/PhysicalDrive(\d+)/)?.[1] || device.match(/disk(\d+)/)?.[1];
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
    if (sysDisk && (device === sysDisk || device.startsWith(sysDisk + 's') || device.replace(/p?\d+$/, '') === sysDisk)) {
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
        const diskNum = device.match(/PhysicalDrive(\d+)/)?.[1];
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
    const [sysDisk, partTable, mountedPartitions, devSize, model, identity] = await Promise.all([
      getSystemDiskId().catch(() => ''),
      getDiskPartitionTable(device),
      getMountedPartitions(device).catch(() => [] as string[]),
      getDeviceSize(device).catch(() => 0),
      getDeviceModel(device).catch(() => ''),
      identityPromise,
    ]);
    const isSystemDisk = !!sysDisk && (device === sysDisk || device.startsWith(sysDisk + 's') || device.replace(/p?\d+$/, '') === sysDisk);
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
        onPhase('erase', `Unmounting ${device}`);
        checkAborted();
        log('DEBUG', 'usb-flash', 'Unmounting disk', { device });
        await runCmd(`diskutil unmountDisk ${device}`, registerProcess);

        onPhase('format', `Erasing ${device} as FAT32 GPT`);
        checkAborted();
        log('DEBUG', 'usb-flash', 'Erasing disk as FAT32 GPT', { device });
        await runCmd(`diskutil eraseDisk FAT32 OPENCORE GPTFormat ${device}`, registerProcess);

        const { mountPoint } = await resolveDarwinMountedVolume(device, 'OPENCORE', registerProcess);

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

      } else if (process.platform === 'linux') {
        const part = `${device}1`;
        const tmpMount = path.join(os.tmpdir(), `oc_usb_${Date.now()}`);

        onPhase('format', `Partitioning and formatting ${device}`);
        checkAborted();
        log('DEBUG', 'usb-flash', 'Partitioning and formatting', { device, part });
        await runCmd(`umount ${device}* 2>/dev/null || true`, registerProcess);
        await runCmd(`parted ${device} --script mklabel gpt mkpart primary fat32 1MiB 100%`, registerProcess);

        for (let i = 0; i < 10; i++) {
          await new Promise(r => setTimeout(r, 400));
          if (fs.existsSync(part)) break;
        }
        if (!fs.existsSync(part)) throw new Error(`Partition ${part} did not appear after partitioning — try again`);
        await runCmd(`mkfs.fat -F 32 -n OPENCORE ${part}`, registerProcess);

        fs.mkdirSync(tmpMount, { recursive: true });
        onPhase('copy', `Copying EFI to ${tmpMount}`);
        checkAborted();
        try {
          await runCmd(`mount ${part} ${tmpMount}`, registerProcess);
          log('DEBUG', 'usb-flash', 'Copying EFI to USB', { tmpMount });
          await runCmd(`cp -r "${path.join(efiPath, 'EFI')}" "${tmpMount}/"`, registerProcess);

          // Copy recovery payload if present
          const recoveryDir = path.join(efiPath, 'com.apple.recovery.boot');
          if (fs.existsSync(recoveryDir)) {
            onPhase('copy', `Copying recovery payload to ${tmpMount}`);
            checkAborted();
            log('DEBUG', 'usb-flash', 'Copying recovery payload', { tmpMount });
            await runCmd(`cp -r "${recoveryDir}" "${tmpMount}/"`, registerProcess);
          }

          await runCmd('sync', registerProcess);

          // Verify: EFI must exist on target
          onPhase('verify', 'Verifying written files');
          if (!fs.existsSync(path.join(tmpMount, 'EFI', 'OC', 'OpenCore.efi'))) {
            throw new Error('Verification failed: EFI/OC/OpenCore.efi not found on USB after copy');
          }
          if (fs.existsSync(recoveryDir) && !fs.existsSync(path.join(tmpMount, 'com.apple.recovery.boot', 'BaseSystem.dmg'))) {
            throw new Error('Verification failed: com.apple.recovery.boot/BaseSystem.dmg not found on USB after copy');
          }
        } finally {
          onPhase('eject', `Unmounting ${tmpMount}`);
          try { await runCmd(`umount ${tmpMount}`, registerProcess); log('DEBUG', 'usb-flash', 'Unmounted'); } catch (_) {}
          try { fs.rmdirSync(tmpMount); } catch (_) {}
        }

      } else if (process.platform === 'win32') {
        const diskNum = getWindowsDiskNumber(device);
        if (!diskNum) throw new Error(`Invalid device path: ${device}`);
        const script = `select disk ${diskNum}\nclean\nconvert gpt\ncreate partition primary\nformat fs=fat32 quick label=OPENCORE\nassign\n`;
        const scriptPath = path.join(os.tmpdir(), 'oc_diskpart.txt');
        fs.writeFileSync(scriptPath, script);

        onPhase('format', `Running diskpart on disk ${diskNum}`);
        checkAborted();
        log('DEBUG', 'usb-flash', 'Running diskpart', { diskNum });
        await runCmd(`diskpart /s "${scriptPath}"`, registerProcess);

        await new Promise(r => setTimeout(r, 1500));
        // Resolve drive letter from the specific disk number, not a global label search.
        // A global label search can match the wrong volume if another OPENCORE disk exists.
        const driveLetter = await getWindowsDriveLetterForLabel(diskNum, 'OPENCORE', registerProcess);
        if (!driveLetter) throw new Error(`Could not determine drive letter for disk ${diskNum} — diskpart may have failed`);

        onPhase('copy', `Copying EFI to ${driveLetter}:`);
        checkAborted();
        log('DEBUG', 'usb-flash', 'Copying EFI', { driveLetter });
        await runCmd(`xcopy /E /I /H /Y "${path.join(efiPath, 'EFI')}" "${driveLetter}:\\EFI"`, registerProcess);

        // Copy recovery payload if present
        const recoveryDir = path.join(efiPath, 'com.apple.recovery.boot');
        if (fs.existsSync(recoveryDir)) {
          onPhase('copy', `Copying recovery payload to ${driveLetter}:`);
          checkAborted();
          log('DEBUG', 'usb-flash', 'Copying recovery payload', { driveLetter });
          await runCmd(`xcopy /E /I /H /Y "${recoveryDir}" "${driveLetter}:\\com.apple.recovery.boot"`, registerProcess);
        }

        // Verify: EFI must exist on target
        onPhase('verify', 'Verifying written files');
        if (!fs.existsSync(path.join(`${driveLetter}:`, 'EFI', 'OC', 'OpenCore.efi'))) {
          throw new Error('Verification failed: EFI\\OC\\OpenCore.efi not found on USB after copy');
        }
        if (fs.existsSync(recoveryDir) && !fs.existsSync(path.join(`${driveLetter}:`, 'com.apple.recovery.boot', 'BaseSystem.dmg'))) {
          throw new Error('Verification failed: com.apple.recovery.boot\\BaseSystem.dmg not found on USB after copy');
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

  async function shrinkPartition(disk: string, sizeGB: number): Promise<void> {
    if (process.platform === 'win32') {
      const diskNum = getWindowsDiskNumber(disk);
      if (!diskNum) throw new Error(`Invalid disk identifier: ${disk}`);
      const partitionNum = await getWindowsPrimaryPartitionNumber(diskNum);
      if (!partitionNum) throw new Error(`Could not determine the primary data partition for disk ${diskNum}`);
      const script = `select disk ${diskNum}\nselect partition ${partitionNum}\nshrink desired=${sizeGB * 1024} minimum=8192\n`;
      const scriptPath = path.join(os.tmpdir(), 'shrink.txt');
      fs.writeFileSync(scriptPath, script);
      await runCmd(`diskpart /s "${scriptPath}"`);
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
      const script = `select disk ${diskNum}\ncreate partition primary size=16384\nformat fs=fat32 quick label=BOOTSTRAP\nassign\n`;
      const scriptPath = path.join(os.tmpdir(), 'create_part.txt');
      fs.writeFileSync(scriptPath, script);

      onPhase('create-partition', `Creating partition on disk ${diskNum}`);
      await runCmd(`diskpart /s "${scriptPath}"`, registerProcess);
      await new Promise(r => setTimeout(r, 1500));

      // Resolve drive letter from the specific disk, not a global label search
      const driveLetter = await getWindowsDriveLetterForLabel(diskNum, 'BOOTSTRAP', registerProcess);
      if (!driveLetter) throw new Error(`Could not determine BOOTSTRAP drive letter for disk ${diskNum}`);

      onPhase('copy', `Copying EFI to ${driveLetter}:`);
      await runCmd(`xcopy /E /Y /I /H "${path.join(efiPath, 'EFI')}" "${driveLetter}:\\EFI"`, registerProcess);

    } else if (process.platform === 'darwin') {
      onPhase('create-partition', `Creating BOOTSTRAP partition on ${disk}`);
      await runCmd(`diskutil addPartition ${disk} MS-DOS BOOTSTRAP 16G`, registerProcess);
      const { mountPoint } = await resolveDarwinMountedVolume(disk, 'BOOTSTRAP', registerProcess);

      onPhase('copy', `Copying EFI to ${mountPoint}`);
      await runCmd(`cp -r "${path.join(efiPath, 'EFI')}" "${mountPoint}/"`, registerProcess);

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
      const { stdout } = await runCmd('powershell -NoProfile -Command "Get-Disk | Where-Object BusType -ne \'USB\' | Select-Object Number,FriendlyName,Size,BusType | ConvertTo-Json"');
      const data = JSON.parse(stdout || '[]');
      const arr = Array.isArray(data) ? data : [data];
      for (const d of arr) {
        drives.push({ name: d.FriendlyName, device: `disk${d.Number}`, size: `${(d.Size / 1e9).toFixed(1)} GB`, type: d.BusType });
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
      try {
        const { stdout } = await runCmd('powershell -NoProfile -Command "Get-Disk | Where-Object BusType -eq \'USB\' | Select-Object Number,FriendlyName,Size | ConvertTo-Json"');
        const disks = JSON.parse(stdout);
        const diskArray = Array.isArray(disks) ? disks : [disks];
        for (const disk of diskArray) {
          drives.push({
            name: disk.FriendlyName,
            device: `\\\\.\\PhysicalDrive${disk.Number}`,
            size: `${(disk.Size / 1e9).toFixed(1)} GB`
          });
        }
      } catch {}
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
    shrinkPartition,
    createBootPartition,
    getHardDrives,
    listUsbDevices,
  };
}

// Re-export getFreeSpaceMB for use in main.ts preflight
export { getFreeSpaceMB };
