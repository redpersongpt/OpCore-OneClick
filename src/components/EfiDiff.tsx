// ── EFI Diff View ───────────────────────────────────────────────────────────
// Shows what changed between two EFI builds in a clean, scannable format.

import React from 'react';
import { Plus, Minus, ArrowRight, GitCompareArrows } from 'lucide-react';
import type { EfiDiff, DiffEntry } from '../lib/efiDiff';

interface Props {
  diff: EfiDiff;
}

const CATEGORY_LABEL: Record<string, string> = {
  kext: 'Kext',
  smbios: 'SMBIOS',
  boot_arg: 'Boot Arg',
  target_os: 'Target OS',
  ssdt: 'SSDT',
};

function DiffRow({ entry }: { entry: DiffEntry; key?: string }) {
  if (entry.type === 'added') {
    return (
      <div className="flex items-center gap-3 px-4 py-2.5 bg-emerald-500/[0.04] border-b border-white/[0.03] last:border-0">
        <Plus className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />
        <span className="text-[10px] font-bold text-white/25 uppercase w-16 flex-shrink-0">
          {CATEGORY_LABEL[entry.category] ?? entry.category}
        </span>
        <span className="text-sm text-emerald-300/80 font-medium">{entry.label}</span>
      </div>
    );
  }

  if (entry.type === 'removed') {
    return (
      <div className="flex items-center gap-3 px-4 py-2.5 bg-red-500/[0.04] border-b border-white/[0.03] last:border-0">
        <Minus className="w-3.5 h-3.5 text-red-400 flex-shrink-0" />
        <span className="text-[10px] font-bold text-white/25 uppercase w-16 flex-shrink-0">
          {CATEGORY_LABEL[entry.category] ?? entry.category}
        </span>
        <span className="text-sm text-red-300/80 font-medium line-through">{entry.label}</span>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-3 px-4 py-2.5 bg-amber-500/[0.04] border-b border-white/[0.03] last:border-0">
      <ArrowRight className="w-3.5 h-3.5 text-amber-400 flex-shrink-0" />
      <span className="text-[10px] font-bold text-white/25 uppercase w-16 flex-shrink-0">
        {CATEGORY_LABEL[entry.category] ?? entry.category}
      </span>
      <div className="flex items-center gap-2 text-sm">
        <span className="text-white/40 line-through">{entry.oldValue}</span>
        <ArrowRight className="w-3 h-3 text-white/15" />
        <span className="text-amber-300/80 font-medium">{entry.newValue}</span>
      </div>
    </div>
  );
}

export default function EfiDiffPanel({ diff }: Props) {
  if (!diff.hasChanges) {
    return (
      <div className="flex items-center gap-3 px-5 py-4 rounded-2xl bg-white/[0.02] border border-white/6">
        <GitCompareArrows className="w-4 h-4 text-white/20" />
        <span className="text-xs text-white/35">No changes since last build</span>
      </div>
    );
  }

  const added = diff.changes.filter(c => c.type === 'added').length;
  const removed = diff.changes.filter(c => c.type === 'removed').length;
  const changed = diff.changes.filter(c => c.type === 'changed').length;

  return (
    <div className="rounded-2xl border border-white/6 overflow-hidden">
      <div className="flex items-center gap-3 px-5 py-3.5 bg-white/[0.02]">
        <GitCompareArrows className="w-4 h-4 text-white/30" />
        <span className="text-xs font-bold text-white/60 uppercase tracking-widest flex-1">Build Diff</span>
        <div className="flex items-center gap-3 text-[10px] font-bold">
          {added > 0 && <span className="text-emerald-400/60">+{added}</span>}
          {removed > 0 && <span className="text-red-400/60">-{removed}</span>}
          {changed > 0 && <span className="text-amber-400/60">{changed} changed</span>}
        </div>
      </div>
      <div className="border-t border-white/5">
        {diff.changes.map((entry, i) => (
          <DiffRow key={`${entry.category}-${entry.label}-${i}`} entry={entry} />
        ))}
      </div>
    </div>
  );
}
