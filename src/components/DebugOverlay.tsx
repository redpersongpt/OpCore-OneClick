import React, { useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { X, Clipboard, Check, FolderOpen } from 'lucide-react';
import type { HardwareProfile } from '../../electron/configGenerator';
import type { TaskState } from '../../electron/taskManager';

export interface DebugOverlayProps {
  // App state
  appVersion: string;
  platform: string;
  sessionId: string;

  // Wizard state
  currentStep: string;
  selectedVersion: string | null;
  installMethod: string;
  profile: HardwareProfile | null;

  // Safety state
  beginnerSafetyMode: boolean;
  selectedDisk: string | null;
  diskTier: string | null;
  flashConfirmed: boolean;

  // Task state
  tasks: Map<string, TaskState>;

  // Detection state
  firmwareHostContext: string | null;
  hwConfidence: string | null;
  secureBootStatus: boolean | 'unknown' | null;

  // Errors
  lastError: string | null;
  lastWarning: string | null;

  // Rich telemetry (Task 4)
  recentEvents: any[];
  watchdogTriggers: number;
  recoveryAttempts: number;

  // Suggestion engine state
  lastSuggestion: { code: string; category: string; title: string } | null;

  // Actions
  onCopyDiagnostics: () => void;
  onCopyLog: () => void;
  onClearLog: () => void;
  onOpenLogFolder: () => void;
  onClose: () => void;
}

/* ── Tiny helper row ─────────────────────────────────────────────── */
function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex justify-between items-baseline gap-3 py-[2px]">
      <span className="text-[10px] text-white/30 whitespace-nowrap">{label}</span>
      <span className="text-[10px] font-mono text-white/60 text-right truncate max-w-[180px]">
        {value ?? '(none)'}
      </span>
    </div>
  );
}

function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[9px] font-bold uppercase tracking-widest text-white/25 mt-4 mb-1 first:mt-0">
      {children}
    </div>
  );
}

export default function DebugOverlay(props: DebugOverlayProps) {
  const {
    appVersion, platform, sessionId,
    currentStep, selectedVersion, installMethod, profile,
    beginnerSafetyMode, selectedDisk, diskTier, flashConfirmed,
    tasks,
    firmwareHostContext, hwConfidence, secureBootStatus,
    lastError, lastWarning,
    recentEvents, watchdogTriggers, recoveryAttempts,
    lastSuggestion,
    onCopyDiagnostics, onCopyLog, onClearLog, onOpenLogFolder, onClose,
  } = props;

  const [copiedDiag, setCopiedDiag] = useState(false);
  const [copiedLog, setCopiedLog] = useState(false);
  const [clearedLog, setClearedLog] = useState(false);

  // Derive active task info from the tasks map
  const activeTaskEntry = Array.from(tasks.values()).find(
    (t: TaskState) => t.status === 'running' || t.status === 'pending'
  );

  const activeTaskKind = activeTaskEntry?.kind ?? '(none)';
  const activeTaskPhase = activeTaskEntry?.status ?? '(none)';
  const activeTaskProgress = (() => {
    const p = activeTaskEntry?.progress as { percent?: number } | null | undefined;
    if (p?.percent !== undefined) return `${Math.round(p.percent)}%`;
    return '(n/a)';
  })();
  const activeTaskCancellable = activeTaskEntry ? 'yes' : 'n/a';

  const smbios = profile?.smbios ?? '(none)';
  const cpuLabel = profile?.cpu ?? '(none)';
  const secureBootLabel =
    secureBootStatus === true ? 'on' :
    secureBootStatus === false ? 'off' :
    secureBootStatus === 'unknown' ? 'unknown' :
    '(n/a)';

  const handleCopyDiag = () => {
    onCopyDiagnostics();
    setCopiedDiag(true);
    setTimeout(() => setCopiedDiag(false), 2000);
  };

  const handleCopyLog = () => {
    onCopyLog();
    setCopiedLog(true);
    setTimeout(() => setCopiedLog(false), 2000);
  };

  return (
    <motion.div
      initial={{ x: 320, opacity: 0 }}
      animate={{ x: 0, opacity: 1 }}
      exit={{ x: 320, opacity: 0 }}
      transition={{ type: 'spring', stiffness: 300, damping: 30 }}
      className="fixed top-0 right-0 bottom-0 w-[320px] z-50 bg-[#0a0a0a] border-l border-white/10 flex flex-col overflow-hidden"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-white/10 flex-shrink-0">
        <span className="text-[11px] font-semibold tracking-wide text-white/50">Debug Panel</span>
        <button
          onClick={onClose}
          className="text-white/30 hover:text-white/60 transition-colors cursor-pointer"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-0">
        <SectionHeader>App</SectionHeader>
        <Row label="Version" value={appVersion} />
        <Row label="Platform" value={platform} />
        <Row label="Session" value={sessionId ? `${sessionId.slice(0, 8)}...` : '(none)'} />

        <SectionHeader>Wizard State</SectionHeader>
        <Row label="Step" value={currentStep} />
        <Row label="macOS" value={selectedVersion ?? '(none)'} />
        <Row label="Method" value={installMethod} />
        <Row label="Profile" value={cpuLabel} />
        <Row label="SMBIOS" value={smbios} />

        <SectionHeader>Safety</SectionHeader>
        <Row label="Beginner" value={beginnerSafetyMode ? 'ON' : 'OFF'} />
        <Row label="Disk" value={selectedDisk ?? '(none)'} />
        <Row label="Disk Tier" value={diskTier ?? '(none)'} />
        <Row label="Confirmed" value={flashConfirmed ? 'yes' : 'no'} />

        <SectionHeader>Tasks</SectionHeader>
        <Row label="Active" value={activeTaskKind} />
        <Row label="Phase" value={activeTaskPhase} />
        <Row label="Progress" value={activeTaskProgress} />
        <Row label="Cancel OK" value={activeTaskCancellable} />

        <SectionHeader>Detection</SectionHeader>
        <Row label="FW Host" value={firmwareHostContext ?? '(none)'} />
        <Row label="HW Conf." value={hwConfidence ?? '(none)'} />
        <Row label="Secure Boot" value={secureBootLabel} />

        <SectionHeader>Telemetry</SectionHeader>
        <Row label="Watchdog" value={watchdogTriggers} />
        <Row label="Recov. Try" value={recoveryAttempts} />
        <div className="mt-2 space-y-1">
          <div className="text-[8px] text-white/20 uppercase font-bold tracking-tight mb-1">Recent Events</div>
          {recentEvents.length === 0 && <div className="text-[10px] text-white/10 italic">No events logged</div>}
          {recentEvents.map((e, i) => (
            <div key={i} className="text-[9px] font-mono text-white/40 flex gap-2">
              <span className="text-white/20">[{new Date(e.t).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })}]</span>
              <span className="truncate">{e.kind}</span>
            </div>
          ))}
        </div>

        <SectionHeader>Suggestions</SectionHeader>
        <Row label="Last Code" value={lastSuggestion?.code ?? '(none)'} />
        <Row label="Category" value={lastSuggestion?.category ?? '(none)'} />
        <Row label="Title" value={lastSuggestion?.title ?? '(none)'} />

        <SectionHeader>Errors</SectionHeader>
        <Row label="Last Error" value={lastError ?? '(none)'} />
        <Row label="Last Warn" value={lastWarning ?? '(none)'} />
      </div>

      {/* Action buttons */}
      <div className="flex-shrink-0 px-4 py-3 border-t border-white/10 space-y-2">
        <div className="flex gap-2">
          <button
            onClick={handleCopyDiag}
            className="flex-1 flex items-center justify-center gap-1.5 px-2 py-1.5 bg-white/5 border border-white/10 rounded-lg text-[10px] font-medium text-white/50 hover:text-white/80 hover:bg-white/8 transition-all cursor-pointer"
          >
            {copiedDiag ? <Check className="w-3 h-3 text-emerald-400" /> : <Clipboard className="w-3 h-3" />}
            {copiedDiag ? 'Copied' : 'Copy Diag'}
          </button>
          <button
            onClick={handleCopyLog}
            className="flex-1 flex items-center justify-center gap-1.5 px-2 py-1.5 bg-white/5 border border-white/10 rounded-lg text-[10px] font-medium text-white/50 hover:text-white/80 hover:bg-white/8 transition-all cursor-pointer"
          >
            {copiedLog ? <Check className="w-3 h-3 text-emerald-400" /> : <Clipboard className="w-3 h-3" />}
            {copiedLog ? 'Copied' : 'Copy Log'}
          </button>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => { onClearLog(); setClearedLog(true); setTimeout(() => setClearedLog(false), 2000); }}
            className="flex-1 flex items-center justify-center gap-1.5 px-2 py-1.5 bg-white/5 border border-white/10 rounded-lg text-[10px] font-medium text-white/50 hover:text-rose-400 hover:bg-rose-500/5 transition-all cursor-pointer"
          >
            {clearedLog ? <Check className="w-3 h-3 text-rose-400" /> : <X className="w-3 h-3" />}
            {clearedLog ? 'Cleared' : 'Clear Logs'}
          </button>
          <button
            onClick={onOpenLogFolder}
            className="flex-[1.5] flex items-center justify-center gap-1.5 px-2 py-1.5 bg-white/5 border border-white/10 rounded-lg text-[10px] font-medium text-white/50 hover:text-white/80 hover:bg-white/8 transition-all cursor-pointer"
          >
            <FolderOpen className="w-3 h-3" />
            Log Folder
          </button>
        </div>
      </div>
    </motion.div>
  );
}
