import React from 'react';
import { AlertTriangle, CheckCircle, ChevronRight, ShieldAlert } from 'lucide-react';
import type { CompatibilityMatrixRow } from '../../electron/compatibilityMatrix';

interface CompatibilityMatrixProps {
  rows: CompatibilityMatrixRow[];
  selectedVersion?: string | null;
  onSelect?: (version: string) => void;
}

const STATUS_STYLE: Record<CompatibilityMatrixRow['status'], {
  label: string;
  icon: React.ReactNode;
  badgeClassName: string;
  cardClassName: string;
  disabled: boolean;
}> = {
  supported: {
    label: 'Supported',
    icon: <CheckCircle className="w-3.5 h-3.5 text-emerald-400" />,
    badgeClassName: 'bg-emerald-500/12 border-emerald-500/20 text-emerald-300',
    cardClassName: 'border-emerald-500/18 bg-emerald-500/6',
    disabled: false,
  },
  partial: {
    label: 'Partial',
    icon: <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />,
    badgeClassName: 'bg-amber-500/12 border-amber-500/20 text-amber-300',
    cardClassName: 'border-amber-500/16 bg-amber-500/5',
    disabled: false,
  },
  blocked: {
    label: 'Blocked',
    icon: <ShieldAlert className="w-3.5 h-3.5 text-red-400" />,
    badgeClassName: 'bg-red-500/12 border-red-500/20 text-red-300',
    cardClassName: 'border-red-500/16 bg-red-500/5',
    disabled: true,
  },
};

export default function CompatibilityMatrix({
  rows,
  selectedVersion,
  onSelect,
}: CompatibilityMatrixProps) {
  return (
    <div className="space-y-3">
      {rows.map((row, index) => {
        const style = STATUS_STYLE[row.status];
        const selected = selectedVersion === row.versionName;
        const interactive = !!onSelect && !style.disabled;
        const content = (
          <div className={`rounded-2xl border p-4 transition-all ${style.cardClassName} ${selected ? 'ring-1 ring-blue-400/40 border-blue-400/35' : 'border-white/8'} ${interactive ? 'hover:border-white/20 hover:bg-white/6' : ''}`}>
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="flex items-center gap-2">
                  <div className="text-sm font-bold text-white">{row.versionName}</div>
                  {row.recommended && (
                    <span className="px-2 py-0.5 rounded-full bg-blue-500/15 border border-blue-500/20 text-[9px] font-bold uppercase tracking-widest text-blue-300">
                      Recommended
                    </span>
                  )}
                  {selected && (
                    <span className="px-2 py-0.5 rounded-full bg-white/10 border border-white/10 text-[9px] font-bold uppercase tracking-widest text-white/70">
                      Selected
                    </span>
                  )}
                </div>
                <div className="text-[11px] text-white/55 mt-2 leading-relaxed">
                  {row.reason}
                </div>
              </div>
              <div className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full border text-[10px] font-bold uppercase tracking-widest ${style.badgeClassName}`}>
                {style.icon}
                {style.label}
              </div>
            </div>
          </div>
        );

        if (!onSelect) {
          return <div key={row.versionId}>{content}</div>;
        }

        return (
          <button
            key={row.versionId}
            type="button"
            disabled={style.disabled}
            onClick={() => onSelect(row.versionName)}
            className={`group w-full text-left ${style.disabled ? 'cursor-not-allowed opacity-85' : 'cursor-pointer'}`}
          >
            <div className="relative">
              {content}
              {interactive && (
                <ChevronRight className="absolute right-4 bottom-4 w-4 h-4 text-white/25 group-hover:text-white/70 transition-colors" />
              )}
            </div>
          </button>
        );
      })}
    </div>
  );
}
