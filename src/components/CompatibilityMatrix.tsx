import React from 'react';
import { AlertTriangle, CheckCircle, ChevronRight, Info, ShieldAlert, Sparkles } from 'lucide-react';
import type { CompatibilityMatrixRow } from '../../electron/compatibilityMatrix';
import { getMacOSPalette } from '../lib/macosPalette';

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
  experimental: {
    label: 'Experimental',
    icon: <Info className="w-3.5 h-3.5 text-sky-400" />,
    badgeClassName: 'bg-sky-500/12 border-sky-500/20 text-sky-300',
    cardClassName: 'border-sky-500/16 bg-sky-500/5',
    disabled: false,
  },
  risky: {
    label: 'Risky',
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
    <div className="grid gap-3 lg:grid-cols-2">
      {rows.map((row) => {
        const style = STATUS_STYLE[row.status];
        const palette = getMacOSPalette(row.versionName);
        const selected = selectedVersion === row.versionName;
        const interactive = !!onSelect && !style.disabled;
        const content = (
          <div className={`group relative min-h-[212px] overflow-hidden rounded-[26px] border p-5 text-left transition-all ${row.recommended ? `${palette.heroClassName} shadow-[0_18px_60px_rgba(37,99,235,0.14)]` : 'bg-white/[0.045]'} ${style.cardClassName} ${selected ? 'ring-1 ring-blue-400/40 border-blue-400/35 shadow-[0_0_0_1px_rgba(96,165,250,0.12)]' : 'border-white/8'} ${interactive ? 'hover:border-white/18 hover:bg-white/[0.065]' : ''} ${row.status === 'blocked' ? 'opacity-80' : ''}`}>
            <div className={`pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-br ${palette.fieldClassName}`} />
            <div className={`pointer-events-none absolute -right-12 top-[-42px] h-36 w-36 rounded-full blur-3xl ${palette.glowClassName}`} />
            <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-white/20" />
            <div className="flex h-full flex-col gap-4">
              <div className="flex items-start justify-between gap-3">
                <div className="space-y-2">
                  <div className="text-[10px] font-bold uppercase tracking-[0.2em] text-white/38">
                    {palette.tone}
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="text-xl font-black tracking-tight text-white">{row.versionName}</div>
                    {row.recommended && (
                      <span className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10px] font-black uppercase tracking-[0.18em] ${palette.badgeClassName}`}>
                        <Sparkles className="h-3.5 w-3.5" />
                        Recommended
                      </span>
                    )}
                    {selected && (
                      <span className="rounded-full border border-white/10 bg-white/10 px-2 py-1 text-[9px] font-bold uppercase tracking-[0.16em] text-white/75">
                        Selected
                      </span>
                    )}
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <div className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.16em] ${style.badgeClassName}`}>
                      {style.icon}
                      {style.label}
                    </div>
                    <div className="rounded-full border border-white/10 bg-black/18 px-2.5 py-1 text-[10px] font-medium text-white/48">
                      {row.status === 'supported'
                        ? 'Cleanest first pass'
                        : row.status === 'experimental'
                        ? 'Community-proven with extra tuning'
                        : row.status === 'risky'
                        ? 'Manual fixes likely'
                        : 'Visible for reference only'}
                    </div>
                  </div>
                </div>
              </div>

              <div className="text-sm leading-relaxed text-white/72">
                {row.reason}
              </div>

              <div className="mt-auto border-t border-white/8 pt-3 text-[11px] leading-relaxed text-white/46">
                {row.status === 'blocked'
                  ? 'Not available for build. Use the recommended version instead of forcing this target.'
                  : row.status === 'supported'
                  ? 'Best starting point if you want the least friction during the first build.'
                  : row.status === 'experimental'
                  ? 'Reasonable real-world path, but expect firmware tuning, older ceilings, or one or two manual fixes.'
                  : 'Worth trying only if you are prepared to troubleshoot audio, sleep, graphics, or input issues yourself.'}
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
                <ChevronRight className="absolute right-5 top-5 w-4 h-4 text-white/25 group-hover:text-white/70 transition-colors" />
              )}
            </div>
          </button>
        );
      })}
    </div>
  );
}
