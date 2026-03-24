import React from 'react';
import { AlertTriangle, CheckCircle2, RefreshCcw, ShieldCheck, X } from 'lucide-react';

export interface FailureRecoveryAction {
  label: string;
  onClick: () => void | Promise<void>;
  tone?: 'primary' | 'secondary' | 'subtle';
  disabled?: boolean;
}

interface Props {
  title: string;
  whatFailed: string;
  likelyCause: string;
  nextActions: string[];
  technicalDetails?: Array<{ label: string; value: string; mono?: boolean }>;
  onDismiss: () => void;
  actions: FailureRecoveryAction[];
  extra?: React.ReactNode;
}

function buttonClass(tone: FailureRecoveryAction['tone'] = 'subtle'): string {
  if (tone === 'primary') {
    return 'bg-blue-600 text-white hover:bg-blue-500 shadow-lg shadow-blue-600/20';
  }
  if (tone === 'secondary') {
    return 'bg-rose-500/10 border-rose-500/20 text-rose-200 hover:bg-rose-500/20';
  }
  return 'bg-white/6 border-white/10 text-white/75 hover:bg-white/10';
}

export default function FailureRecoveryPanel({
  title,
  whatFailed,
  likelyCause,
  nextActions,
  technicalDetails = [],
  onDismiss,
  actions,
  extra,
}: Props) {
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center px-4 pb-5 sm:items-center sm:px-6">
      <div className="absolute inset-0 bg-black/70 backdrop-blur-lg" onClick={onDismiss} />
      <div className="relative w-full max-w-3xl overflow-hidden rounded-[28px] border border-white/12 bg-[#0b0b0d]/96 shadow-2xl">
        <div className="border-b border-white/8 bg-white/[0.03] px-6 py-5">
          <div className="flex items-start justify-between gap-4">
            <div className="flex items-start gap-3">
              <div className="mt-0.5 rounded-2xl border border-rose-500/20 bg-rose-500/10 p-2.5 text-rose-300">
                <AlertTriangle className="h-5 w-5" />
              </div>
              <div className="space-y-1">
                <div className="text-[11px] font-black uppercase tracking-[0.24em] text-white/35">Something needs attention</div>
                <h2 className="text-2xl font-black tracking-tight text-white">{title}</h2>
              </div>
            </div>
            <button onClick={onDismiss} className="rounded-xl p-2 text-white/30 transition-colors hover:bg-white/6 hover:text-white/70">
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        <div className="grid gap-4 px-6 py-6 lg:grid-cols-[1.1fr_0.9fr]">
          <section className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="mb-2 flex items-center gap-2 text-[11px] font-black uppercase tracking-[0.2em] text-white/35">
              <AlertTriangle className="h-3.5 w-3.5" />
              What Failed
            </div>
            <p className="text-sm leading-relaxed text-white/80">{whatFailed}</p>
          </section>

          <section className="rounded-2xl border border-white/8 bg-white/[0.03] p-4">
            <div className="mb-2 flex items-center gap-2 text-[11px] font-black uppercase tracking-[0.2em] text-white/35">
              <ShieldCheck className="h-3.5 w-3.5" />
              Why It Likely Failed
            </div>
            <p className="text-sm leading-relaxed text-white/75">{likelyCause}</p>
          </section>

          <section className="rounded-2xl border border-white/8 bg-white/[0.03] p-4 lg:col-span-2">
            <div className="mb-3 flex items-center gap-2 text-[11px] font-black uppercase tracking-[0.2em] text-white/35">
              <CheckCircle2 className="h-3.5 w-3.5" />
              What To Do Next
            </div>
            <div className="grid gap-2 md:grid-cols-2">
              {nextActions.map((action) => (
                <div key={action} className="rounded-2xl border border-white/8 bg-black/20 px-3 py-3 text-sm leading-relaxed text-white/75">
                  {action}
                </div>
              ))}
            </div>
          </section>

          {technicalDetails.length > 0 && (
            <section className="rounded-2xl border border-white/8 bg-white/[0.03] p-4 lg:col-span-2">
              <details>
                <summary className="cursor-pointer list-none text-[11px] font-black uppercase tracking-[0.2em] text-white/35">
                  Concise Technical Detail
                </summary>
                <div className="mt-3 grid gap-2 md:grid-cols-2">
                  {technicalDetails.map((detail) => (
                    <div key={`${detail.label}-${detail.value}`} className="rounded-2xl border border-white/8 bg-black/20 px-3 py-3">
                      <div className="text-[10px] font-black uppercase tracking-[0.18em] text-white/30">{detail.label}</div>
                      <div className={`mt-1 text-sm leading-relaxed text-white/75 ${detail.mono ? 'font-mono text-[12px]' : ''}`}>
                        {detail.value}
                      </div>
                    </div>
                  ))}
                </div>
              </details>
            </section>
          )}

          {extra && (
            <section className="lg:col-span-2">
              {extra}
            </section>
          )}
        </div>

        <div className="border-t border-white/8 px-6 py-5">
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            {actions.map((action) => (
              <button
                key={action.label}
                disabled={action.disabled}
                onClick={() => void action.onClick()}
                className={`rounded-2xl border px-4 py-3 text-sm font-bold transition-all ${buttonClass(action.tone)} disabled:cursor-not-allowed disabled:opacity-35`}
              >
                {action.label === 'Retry' && <RefreshCcw className="mr-2 inline h-4 w-4" />}
                {action.label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
