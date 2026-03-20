import React from 'react';
import { Archive, AlertTriangle, CheckCircle2, ShieldAlert } from 'lucide-react';
import type { EfiBackupPolicy } from '../../electron/efiBackup';

interface EfiBackupPanelProps {
  policy: EfiBackupPolicy | null;
}

export default function EfiBackupPanel({ policy }: EfiBackupPanelProps) {
  if (!policy) {
    return (
      <div className="rounded-2xl border border-white/8 bg-white/3 p-4">
        <div className="text-[10px] font-bold uppercase tracking-widest text-white/25">EFI Backup Policy</div>
        <div className="text-xs text-white/55 mt-2">Inspecting existing EFI state on the selected target…</div>
      </div>
    );
  }

  const tone = policy.status === 'required'
    ? {
        icon: <Archive className="w-4 h-4 text-blue-300" />,
        border: 'border-blue-500/18',
        bg: 'bg-blue-500/6',
        label: 'Backup Required',
        text: 'text-blue-200/80',
      }
    : policy.status === 'blocked'
    ? {
        icon: <ShieldAlert className="w-4 h-4 text-red-300" />,
        border: 'border-red-500/18',
        bg: 'bg-red-500/6',
        label: 'Backup Blocked',
        text: 'text-red-200/80',
      }
    : {
        icon: <CheckCircle2 className="w-4 h-4 text-emerald-300" />,
        border: 'border-emerald-500/18',
        bg: 'bg-emerald-500/6',
        label: 'Backup Not Needed',
        text: 'text-emerald-200/80',
      };

  return (
    <div className={`rounded-2xl border ${tone.border} ${tone.bg} p-4 space-y-3`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-white/75">
          {tone.icon}
          EFI Backup Policy
        </div>
        <div className={`text-[10px] font-bold uppercase tracking-widest ${tone.text}`}>
          {tone.label}
        </div>
      </div>

      <div className={`text-xs leading-relaxed ${tone.text}`}>
        {policy.reason}
      </div>

      {policy.latestBackup && (
        <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-2.5 text-[11px] text-white/55">
          <div>Latest backup ID: <span className="font-mono">{policy.latestBackup.backupId}</span></div>
          <div className="mt-1">Manifest: <span className="font-mono">{policy.latestBackup.manifestHash.slice(0, 16)}</span></div>
          <div className="mt-1">Config hash: <span className="font-mono">{policy.latestBackup.configHash?.slice(0, 16) ?? 'missing'}</span></div>
        </div>
      )}

      {policy.status === 'blocked' && (
        <div className="flex items-start gap-2 text-[11px] text-red-200/70">
          <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
          <span>Flashing must not proceed until the target EFI can be inspected and backed up safely.</span>
        </div>
      )}
    </div>
  );
}
