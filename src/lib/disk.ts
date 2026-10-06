import type { DiskInfo } from '../bridge/types';
import { toNum } from './num';

/** A drive sold as "4 GB" reports a little under 4·10⁹ bytes. */
export const MIN_BYTES_WITH_RECOVERY = 3_500_000_000;
export const MIN_BYTES_EFI_ONLY = 256 * 1024 * 1024;
const RECOVERY_HEADROOM = 300 * 1024 * 1024;

export function minimumDiskBytes(includeRecovery: boolean, recoveryBytes: number | null): number {
  if (!includeRecovery) return MIN_BYTES_EFI_ONLY;
  const needed = recoveryBytes !== null && recoveryBytes > 0 ? recoveryBytes + RECOVERY_HEADROOM : 0;
  return Math.max(MIN_BYTES_WITH_RECOVERY, needed);
}

export type DiskBlock =
  | { kind: 'system' }
  | { kind: 'backend'; reason: string }
  | { kind: 'too_small'; minBytes: number };

/** Why a disk cannot be selected as the target, or null when it can. */
export function diskBlock(disk: DiskInfo, minBytes: number): DiskBlock | null {
  if (disk.isSystemDisk) return { kind: 'system' };
  if (disk.blockedReason) return { kind: 'backend', reason: disk.blockedReason };
  if ((toNum(disk.sizeBytes) ?? 0) < minBytes) return { kind: 'too_small', minBytes };
  return null;
}

export function diskTitle(disk: DiskInfo): string {
  const name = [disk.vendor, disk.model].filter((s) => s && s.trim()).join(' ').trim();
  return name || disk.devicePath;
}

const norm = (s: string) => s.trim().replace(/\s+/g, ' ').toLowerCase();

/** Last component of the device path: "sdb", "disk4", "PhysicalDrive3". */
export function deviceName(disk: DiskInfo): string {
  const parts = disk.devicePath.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : disk.devicePath;
}

/**
 * Text the user must type to confirm erasing `disk`: its size, plus the device
 * name when another listed drive has the same size (so the phrase always
 * names exactly one drive).
 */
export function confirmPhrase(disk: DiskInfo, others: readonly DiskInfo[] = []): string {
  const size = disk.sizeDisplay.trim();
  const ambiguous = others.some((d) => d.devicePath !== disk.devicePath && norm(d.sizeDisplay) === norm(size));
  return ambiguous || !size ? `${size} ${deviceName(disk)}`.trim() : size;
}

export function phraseMatches(input: string, phrase: string): boolean {
  return phrase.length > 0 && norm(input) === norm(phrase);
}
