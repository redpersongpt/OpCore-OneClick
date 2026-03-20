// ── EFI Diff View ───────────────────────────────────────────────────────────
// Computes a human-readable diff between two EFI build snapshots.

import type { HardwareProfile } from '../../electron/configGenerator';

export interface EfiBuildSnapshot {
  kexts: string[];
  smbios: string;
  bootArgs: string;
  targetOS: string;
  ssdts: string[];
  timestamp: number;
}

export interface DiffEntry {
  type: 'added' | 'removed' | 'changed';
  category: 'kext' | 'smbios' | 'boot_arg' | 'target_os' | 'ssdt';
  label: string;
  oldValue?: string;
  newValue?: string;
}

export interface EfiDiff {
  changes: DiffEntry[];
  summary: string;
  hasChanges: boolean;
}

export function snapshotFromProfile(profile: HardwareProfile): EfiBuildSnapshot {
  return {
    kexts: [...profile.kexts].sort(),
    smbios: profile.smbios,
    bootArgs: profile.bootArgs,
    targetOS: profile.targetOS,
    ssdts: [...profile.ssdts].sort(),
    timestamp: Date.now(),
  };
}

export function computeEfiDiff(prev: EfiBuildSnapshot, next: EfiBuildSnapshot): EfiDiff {
  const changes: DiffEntry[] = [];

  // Kext changes
  const prevKexts = new Set(prev.kexts);
  const nextKexts = new Set(next.kexts);

  for (const k of nextKexts) {
    if (!prevKexts.has(k)) {
      changes.push({ type: 'added', category: 'kext', label: k });
    }
  }
  for (const k of prevKexts) {
    if (!nextKexts.has(k)) {
      changes.push({ type: 'removed', category: 'kext', label: k });
    }
  }

  // SSDT changes
  const prevSSDTs = new Set(prev.ssdts);
  const nextSSDTs = new Set(next.ssdts);

  for (const s of nextSSDTs) {
    if (!prevSSDTs.has(s)) {
      changes.push({ type: 'added', category: 'ssdt', label: s });
    }
  }
  for (const s of prevSSDTs) {
    if (!nextSSDTs.has(s)) {
      changes.push({ type: 'removed', category: 'ssdt', label: s });
    }
  }

  // SMBIOS change
  if (prev.smbios !== next.smbios) {
    changes.push({
      type: 'changed',
      category: 'smbios',
      label: 'SMBIOS',
      oldValue: prev.smbios,
      newValue: next.smbios,
    });
  }

  // Boot args change
  if (prev.bootArgs !== next.bootArgs) {
    // Find specific arg changes
    const prevArgs = new Set(prev.bootArgs.split(/\s+/).filter(Boolean));
    const nextArgs = new Set(next.bootArgs.split(/\s+/).filter(Boolean));

    for (const a of nextArgs) {
      if (!prevArgs.has(a)) {
        changes.push({ type: 'added', category: 'boot_arg', label: a });
      }
    }
    for (const a of prevArgs) {
      if (!nextArgs.has(a)) {
        changes.push({ type: 'removed', category: 'boot_arg', label: a });
      }
    }
  }

  // Target OS change
  if (prev.targetOS !== next.targetOS) {
    changes.push({
      type: 'changed',
      category: 'target_os',
      label: 'Target macOS',
      oldValue: prev.targetOS,
      newValue: next.targetOS,
    });
  }

  // Summary
  const added = changes.filter(c => c.type === 'added').length;
  const removed = changes.filter(c => c.type === 'removed').length;
  const changed = changes.filter(c => c.type === 'changed').length;
  const parts: string[] = [];
  if (added > 0) parts.push(`${added} added`);
  if (removed > 0) parts.push(`${removed} removed`);
  if (changed > 0) parts.push(`${changed} changed`);
  const summary = parts.length > 0
    ? `${parts.join(', ')} since last build`
    : 'No changes since last build';

  return { changes, summary, hasChanges: changes.length > 0 };
}
