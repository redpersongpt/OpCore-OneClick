import React from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Check, Loader2, AlertTriangle, ChevronRight } from 'lucide-react';

export interface Stage { label: string; sublabel: string; done: boolean; active: boolean; }

// ── Phase label map (Phase 7a) ─────────────────────────────────────────────
// Maps raw task phase identifiers to plain-English descriptions.
const PHASE_LABELS: Record<string, string> = {
  download_start:    'Connecting to Apple servers',
  downloading:       'Downloading macOS recovery files',
  download_resume:   'Resuming download',
  download_complete: 'Download complete',
  verifying:         'Checking download integrity',
  partition:         'Preparing drive',
  flash_start:       'Initializing USB drive',
  flashing:          'Writing system files to USB',
  flash_verify:      'Verifying USB write integrity',
  efi_copy:          'Installing OpenCore bootloader',
  complete:          'Ready',
};

/** Map a raw phase/status text to a plain-English label if one exists. */
export function getPlainPhaseLabel(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  const key = raw.toLowerCase().replace(/[\s-]/g, '_');
  return PHASE_LABELS[key] ?? undefined;
}

// ── Phases that cannot be safely interrupted (Phase 7c) ───────────────────
const UNSAFE_TO_INTERRUPT_LABELS = [
  'Writing to USB drive — do not remove drive',
  'Writing boot files to USB',
  'Verifying USB was written correctly',
];

function isUnsafeToInterrupt(statusText: string | undefined): boolean {
  if (!statusText) return false;
  const plain = getPlainPhaseLabel(statusText) ?? statusText;
  return UNSAFE_TO_INTERRUPT_LABELS.some(u => plain.includes(u) || statusText.includes(u));
}

function ProgressBar({ progress }: { progress: number }) {
  return (
    <div className="w-full h-2 bg-white/8 rounded-full overflow-hidden">
      <motion.div
        className="h-full bg-gradient-to-r from-blue-500 to-blue-400 shadow-[0_0_12px_rgba(59,130,246,0.4)]"
        animate={{ width: `${progress}%` }}
        transition={{ ease: 'easeOut', duration: 0.4 }}
      />
    </div>
  );
}

function StageRow({ stage }: { stage: Stage; key?: number }) {
  return (
    <div className={`flex items-center gap-4 p-4 rounded-2xl border transition-all ${
      stage.active  ? 'bg-blue-500/8 border-blue-500/20' :
      stage.done    ? 'bg-white/4 border-white/6' :
                      'border-transparent opacity-40'
    }`}>
      <div className={`w-8 h-8 rounded-xl flex items-center justify-center flex-shrink-0 ${
        stage.done   ? 'bg-green-500/20' :
        stage.active ? 'bg-blue-500/15' :
                       'bg-white/5'
      }`}>
        {stage.done   ? <Check className="w-4 h-4 text-green-400" /> :
         stage.active ? <Loader2 className="w-4 h-4 text-blue-400 animate-spin" /> :
                        <div className="w-2 h-2 rounded-full bg-white/15" />}
      </div>
      <div>
        <div className={`text-sm font-bold ${stage.done ? 'text-white' : stage.active ? 'text-white' : 'text-[#555]'}`}>
          {stage.label}
        </div>
        {(stage.done || stage.active) && (
          <div className="text-xs text-[#888888]">{stage.sublabel}</div>
        )}
      </div>
    </div>
  );
}

// ── Milestone row (Phase 7b) ──────────────────────────────────────────────
function MilestoneRow({ label }: { label: string; key?: string }) {
  return (
    <motion.div
      initial={{ opacity: 0, x: -8 }}
      animate={{ opacity: 1, x: 0 }}
      className="flex items-center gap-2 px-3 py-1.5 rounded-xl bg-emerald-500/8 border border-emerald-500/20"
    >
      <Check className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />
      <span className="text-xs font-bold text-emerald-400">{label}</span>
    </motion.div>
  );
}

/** Configuration for the pre-task briefing card shown before the task begins. */
export interface BriefingConfig {
  heading: string;
  bullets: string[];
  estimatedMinutes: number;
  interruptionWarning: string;
}

// ── Pre-task briefing card (Phase 6) ──────────────────────────────────────────
function BriefingCard({
  briefing, icon: Icon, onBegin,
}: {
  briefing: BriefingConfig;
  icon: React.ElementType;
  onBegin: () => void;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      className="flex flex-col h-full space-y-6 py-4"
    >
      {/* Header */}
      <div className="flex items-center gap-4">
        <div className="w-12 h-12 rounded-2xl bg-blue-500/10 border border-blue-500/15 flex items-center justify-center flex-shrink-0">
          <Icon className="w-6 h-6 text-blue-400" />
        </div>
        <h2 className="text-3xl font-bold text-white tracking-tight">{briefing.heading}</h2>
      </div>

      {/* What will happen */}
      <div className="rounded-2xl border border-white/8 bg-white/3 overflow-hidden">
        <div className="px-5 py-3 border-b border-white/5">
          <span className="text-[9px] font-bold uppercase tracking-widest text-white/25">What will happen</span>
        </div>
        <div className="divide-y divide-white/4">
          {briefing.bullets.map((bullet, i) => (
            <div key={i} className="flex items-start gap-3 px-5 py-3">
              <div className="w-5 h-5 rounded-full bg-blue-500/15 flex items-center justify-center flex-shrink-0 mt-0.5">
                <span className="text-[9px] font-bold text-blue-400">{i + 1}</span>
              </div>
              <span className="text-sm text-white/60 leading-relaxed">{bullet}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Interruption warning */}
      <div className="flex items-start gap-3 px-4 py-3.5 rounded-2xl bg-amber-500/6 border border-amber-500/20 flex-shrink-0">
        <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
        <p className="text-xs text-amber-200/80 leading-relaxed">{briefing.interruptionWarning}</p>
      </div>

      {/* Estimated time */}
      <div className="flex items-center gap-2 px-1">
        <Loader2 className="w-3.5 h-3.5 text-white/20" />
        <span className="text-xs text-white/30">
          Estimated time: approximately {briefing.estimatedMinutes} minutes
        </span>
      </div>

      {/* Begin button */}
      <div className="mt-auto pt-4 border-t border-white/5 flex justify-end">
        <motion.button
          onClick={onBegin}
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          className="flex items-center gap-2 px-8 py-3.5 bg-blue-600 text-white rounded-xl text-sm font-bold hover:bg-blue-500 transition-all cursor-pointer shadow-lg shadow-blue-600/20"
        >
          Begin <ChevronRight className="w-4 h-4" />
        </motion.button>
      </div>
    </motion.div>
  );
}

// ── Generic progress step used for Building, Kext fetch, Recovery, Flashing ──
export default function ProgressStep({
  title, subtitle, icon: Icon, progress, statusText, stages,
  onCancel, onTroubleshoot, native = false,
  milestones = [], onBegin, briefing, notice,
}: {
  title: string; subtitle: string; icon: React.ElementType;
  progress: number; statusText?: string; stages: Stage[];
  onCancel?: () => void;
  onTroubleshoot?: () => void;
  native?: boolean;
  /** Completed milestone labels to display below the stage list (Phase 7b). */
  milestones?: string[];
  /** When provided, shown as a pre-task briefing card before the task starts (progress === 0). */
  onBegin?: () => void;
  briefing?: BriefingConfig;
  notice?: {
    tone: 'warning' | 'critical';
    title: string;
    message: string;
  } | null;
}) {
  // Translate raw phase string to plain English if possible
  const displayStatus = statusText
    ? (getPlainPhaseLabel(statusText) ?? statusText)
    : undefined;

  const dangerPhase = isUnsafeToInterrupt(statusText);

  // Show briefing card before the task has started
  if (onBegin && briefing && progress === 0) {
    return <BriefingCard briefing={briefing} icon={Icon} onBegin={onBegin} />;
  }

  if (native) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-center space-y-12 py-10">
        <div className="space-y-4 max-w-md w-full">
          <h2 className="text-4xl font-black text-white tracking-tight">{title}</h2>
          <p className="text-[#888] text-sm font-medium">{subtitle}</p>
        </div>

        {notice && (
          <div className={`w-full max-w-md rounded-2xl border px-4 py-3 text-left ${
            notice.tone === 'critical'
              ? 'border-rose-500/25 bg-rose-500/10 text-rose-100'
              : 'border-amber-500/25 bg-amber-500/10 text-amber-100'
          }`}>
            <div className="text-[11px] font-black uppercase tracking-[0.2em]">{notice.title}</div>
            <p className="mt-1 text-sm leading-relaxed text-white/80">{notice.message}</p>
          </div>
        )}

        <div className="w-full max-w-sm space-y-4">
          <div className="w-full h-[6px] bg-white/5 rounded-full overflow-hidden">
            <motion.div
              className="h-full bg-white shadow-[0_0_20px_rgba(255,255,255,0.2)]"
              animate={{ width: `${progress}%` }}
              transition={{ ease: 'easeOut', duration: 0.5 }}
            />
          </div>
          <div className="flex justify-between items-center px-1">
            <span className="text-[10px] font-bold text-white/30 uppercase tracking-[0.2em]">
              {displayStatus || 'Installing...'}
            </span>
            <span className="text-[10px] font-mono text-white/20">
              {Math.round(progress)}%
            </span>
          </div>
        </div>

        <div className="pt-8">
          <p className="text-[11px] text-[#555] font-medium">
            This may take several minutes depending on your connection.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full space-y-8 py-4 px-2">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-5">
          <div className="w-14 h-14 rounded-2xl bg-blue-500/10 border border-blue-500/15 flex items-center justify-center flex-shrink-0">
            <Icon className="w-7 h-7 text-blue-400" />
          </div>
          <div>
            <h2 className="text-3xl font-bold text-white tracking-tight">{title}</h2>
            <p className="text-[#888888] text-sm font-medium">{subtitle}</p>
          </div>
        </div>
        
        {/* Status Badge */}
        <div className={`px-3 py-1.5 rounded-full text-[10px] font-bold border transition-colors ${
          dangerPhase 
            ? 'bg-amber-500/10 border-amber-500/20 text-amber-400' 
            : 'bg-emerald-500/10 border-emerald-500/20 text-emerald-400'
        }`}>
          {dangerPhase ? 'Do not interrupt' : 'Safe to pause'}
        </div>
      </div>

      {/* Restart-only warning (Phase 7c) */}
      <AnimatePresence>
        {dangerPhase && (
          <motion.div
            key="restart-warning"
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/8 border border-amber-500/20 flex-shrink-0"
          >
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
            <p className="text-xs text-amber-300/80 leading-relaxed">
              <span className="font-bold">Do not close the app or remove the USB drive.</span>{' '}
              If this step is interrupted, it must be started again from the beginning.
            </p>
          </motion.div>
        )}
      </AnimatePresence>

      {notice && (
        <div className={`flex items-start gap-3 rounded-2xl border px-4 py-3 text-sm ${
          notice.tone === 'critical'
            ? 'border-rose-500/25 bg-rose-500/10 text-rose-100'
            : 'border-amber-500/25 bg-amber-500/10 text-amber-100'
        }`}>
          <AlertTriangle className={`mt-0.5 h-4 w-4 flex-shrink-0 ${notice.tone === 'critical' ? 'text-rose-300' : 'text-amber-300'}`} />
          <div className="space-y-1">
            <div className="text-[11px] font-black uppercase tracking-[0.2em]">
              {notice.title}
            </div>
            <p className="leading-relaxed text-white/80">{notice.message}</p>
          </div>
        </div>
      )}

      {/* Progress bar */}
      <div className="space-y-2">
        <ProgressBar progress={progress} />
        <div className="flex justify-between items-center">
          <span className="text-xs text-[#666666]">{displayStatus ?? '\u00a0'}</span>
          <span className="font-mono text-[11px] text-[#444]">{Math.round(progress)}%</span>
        </div>
      </div>

      {/* Stages */}
      <div className="space-y-2 flex-grow overflow-auto">
        {stages.map((s, i) => <StageRow key={i} stage={s} />)}
      </div>

      {/* Milestones (Phase 7b) */}
      <AnimatePresence>
        {milestones.length > 0 && (
          <motion.div
            key="milestones"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            className="space-y-2 flex-shrink-0"
          >
            {milestones.map(m => <MilestoneRow key={m} label={m} />)}
          </motion.div>
        )}
      </AnimatePresence>

      {/* Action Buttons */}
      <div className="flex gap-4 pt-4 mt-auto">
        {onCancel && (
          <button
            onClick={onCancel}
            className="flex-1 py-4 bg-red-500/10 border border-red-500/20 text-red-400 rounded-2xl font-bold text-base hover:bg-red-500/20 transition-all cursor-pointer flex items-center justify-center gap-2"
          >
            Go Back
          </button>
        )}
        {onTroubleshoot && (
          <button
            onClick={onTroubleshoot}
            className="flex-1 py-4 bg-white/5 border border-white/10 text-white rounded-2xl font-bold text-base hover:bg-white/10 transition-all cursor-pointer flex items-center justify-center gap-2"
          >
            Troubleshooting
          </button>
        )}
      </div>
    </div>
  );
}
