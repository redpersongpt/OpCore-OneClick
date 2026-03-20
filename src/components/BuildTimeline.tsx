// ── Build Timeline ──────────────────────────────────────────────────────────
// Vercel/Linear-style real-time build log with step status and duration.

import React, { useEffect, useState, useRef } from 'react';
import { CheckCircle, Circle, Loader2, XCircle, Clock } from 'lucide-react';

export interface TimelineStep {
  id: string;
  label: string;
  status: 'pending' | 'running' | 'done' | 'error' | 'skipped';
  detail?: string;
  startedAt?: number;
  finishedAt?: number;
}

interface Props {
  steps: TimelineStep[];
  buildStartedAt?: number;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  const rem = Math.round(s % 60);
  return `${m}m ${rem}s`;
}

function StepIcon({ status }: { status: TimelineStep['status'] }) {
  switch (status) {
    case 'done':
      return <CheckCircle className="w-4 h-4 text-emerald-400" />;
    case 'running':
      return <Loader2 className="w-4 h-4 text-blue-400 animate-spin" />;
    case 'error':
      return <XCircle className="w-4 h-4 text-red-400" />;
    case 'skipped':
      return <Circle className="w-4 h-4 text-white/10" />;
    default:
      return <Circle className="w-4 h-4 text-white/15" />;
  }
}

function ElapsedTimer({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(Date.now());
  const rafRef = useRef<number>();

  useEffect(() => {
    let mounted = true;
    const tick = () => {
      if (!mounted) return;
      setNow(Date.now());
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => {
      mounted = false;
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [startedAt]);

  return (
    <span className="text-[10px] text-blue-400/50 font-mono tabular-nums">
      {formatDuration(now - startedAt)}
    </span>
  );
}

function TimelineRow({ step, isLast }: { step: TimelineStep; isLast: boolean; key?: string }) {
  const statusColor =
    step.status === 'done' ? 'text-white/60'
    : step.status === 'running' ? 'text-white/80'
    : step.status === 'error' ? 'text-red-300/80'
    : 'text-white/20';

  const duration = step.startedAt && step.finishedAt
    ? formatDuration(step.finishedAt - step.startedAt)
    : null;

  return (
    <div className="flex items-start gap-3">
      {/* Timeline rail */}
      <div className="flex flex-col items-center">
        <StepIcon status={step.status} />
        {!isLast && (
          <div className={`w-px flex-1 min-h-[20px] ${
            step.status === 'done' ? 'bg-emerald-400/20' :
            step.status === 'running' ? 'bg-blue-400/20' :
            'bg-white/5'
          }`} />
        )}
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0 pb-4">
        <div className="flex items-center gap-2">
          <span className={`text-sm font-medium ${statusColor}`}>{step.label}</span>
          {step.status === 'running' && step.startedAt && (
            <ElapsedTimer startedAt={step.startedAt} />
          )}
          {duration && step.status === 'done' && (
            <span className="text-[10px] text-white/20 font-mono">{duration}</span>
          )}
          {duration && step.status === 'error' && (
            <span className="text-[10px] text-red-400/40 font-mono">{duration}</span>
          )}
        </div>
        {step.detail && (
          <p className={`text-xs mt-0.5 leading-relaxed ${
            step.status === 'error' ? 'text-red-300/50' : 'text-white/25'
          }`}>{step.detail}</p>
        )}
      </div>
    </div>
  );
}

export default function BuildTimeline({ steps, buildStartedAt }: Props) {
  const totalDone = steps.filter(s => s.status === 'done').length;
  const totalSteps = steps.filter(s => s.status !== 'skipped').length;
  const hasError = steps.some(s => s.status === 'error');
  const allDone = totalDone === totalSteps && !hasError;

  const totalDuration = buildStartedAt && allDone
    ? formatDuration(Date.now() - buildStartedAt)
    : null;

  return (
    <div className="rounded-2xl border border-white/6 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-3 px-5 py-3.5 bg-white/[0.02]">
        <Clock className="w-4 h-4 text-white/30" />
        <span className="text-xs font-bold text-white/60 uppercase tracking-widest flex-1">Build Log</span>
        <div className="flex items-center gap-2">
          {!allDone && !hasError && (
            <span className="text-[10px] text-white/25 font-mono">{totalDone}/{totalSteps}</span>
          )}
          {allDone && totalDuration && (
            <span className="text-[10px] text-emerald-400/50 font-mono">Done in {totalDuration}</span>
          )}
          {hasError && (
            <span className="text-[10px] text-red-400/60 font-bold uppercase">Failed</span>
          )}
        </div>
      </div>

      {/* Progress bar */}
      <div className="h-px bg-white/5 relative">
        <div
          className={`h-full transition-all duration-500 ease-out ${
            hasError ? 'bg-red-400/60' : allDone ? 'bg-emerald-400/60' : 'bg-blue-400/60'
          }`}
          style={{ width: totalSteps > 0 ? `${(totalDone / totalSteps) * 100}%` : '0%' }}
        />
      </div>

      {/* Steps */}
      <div className="px-5 pt-4 pb-1">
        {steps.map((step, i) => (
          <TimelineRow key={step.id} step={step} isLast={i === steps.length - 1} />
        ))}
      </div>
    </div>
  );
}
