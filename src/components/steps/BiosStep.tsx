import React, { useEffect, useState, useMemo } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import {
  Check, ChevronRight, RotateCcw, ShieldCheck, AlertTriangle, X,
  Loader2, CheckCircle2, HelpCircle, Info,
} from 'lucide-react';
import type { BIOSConfig } from '../../../electron/configGenerator';
import type { FirmwareInfo } from '../../../electron/firmwarePreflight';
import type {
  BiosApplyMode,
  BiosOrchestratorState,
  BiosResumeStateResponse,
  BiosSettingPlan,
  BiosSettingSelection,
  BiosSessionStage,
  FirmwareRestartCapability,
} from '../../../electron/bios/types';

// ── Props ────────────────────────────────────────────────────────────────────

interface Props {
  biosConfig: BIOSConfig;
  biosStatus: { secureBootDisabled: boolean | 'unknown'; virtualizationEnabled: boolean | 'unknown' } | null;
  firmwareInfo: FirmwareInfo | null;
  orchestratorState: BiosOrchestratorState | null;
  resumeState: BiosResumeStateResponse | null;
  restartCapability: FirmwareRestartCapability | null;
  onApplySupportedChanges: (selectedChanges: Record<string, BiosSettingSelection>) => Promise<{ message: string }>;
  onVerifyAndContinue: (selectedChanges: Record<string, BiosSettingSelection>) => Promise<boolean>;
  onRestartToBios: (selectedChanges: Record<string, BiosSettingSelection>) => Promise<{ supported: boolean; error?: string }>;
}

type RestartState = 'idle' | 'confirming' | 'restarting' | 'unsupported';

// ── Summary counts ───────────────────────────────────────────────────────────

function useSummaryCounts(settings: BiosSettingPlan[]) {
  return useMemo(() => {
    const required = settings.filter(s => s.required);
    const verified = settings.filter(s => s.verificationStatus === 'verified');
    const pending = settings.filter(s => s.verificationStatus !== 'verified' && s.required);
    const manualOnly = settings.filter(s => s.supportLevel === 'manual');
    const autoEligible = settings.filter(s => s.supportLevel !== 'manual');
    return { total: settings.length, required: required.length, verified: verified.length, pending: pending.length, manualOnly: manualOnly.length, autoEligible: autoEligible.length };
  }, [settings]);
}

// ── Stage badge ──────────────────────────────────────────────────────────────

function stageBadge(stage: BiosSessionStage, readyToBuild: boolean): { label: string; cls: string } {
  if (readyToBuild && stage === 'complete') return { label: 'Complete', cls: 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20' };
  if (stage === 'verifying' || stage === 'resumed_from_firmware') return { label: 'Verifying', cls: 'text-blue-400 bg-blue-500/10 border-blue-500/20' };
  if (stage === 'partially_verified') return { label: 'Partially verified', cls: 'text-amber-400 bg-amber-500/10 border-amber-500/20' };
  if (stage === 'blocked') return { label: 'Blocked', cls: 'text-red-400 bg-red-500/10 border-red-500/20' };
  if (stage === 'awaiting_return' || stage === 'rebooting_to_firmware') return { label: 'Awaiting return', cls: 'text-blue-400 bg-blue-500/10 border-blue-500/20' };
  if (stage === 'auto_applying') return { label: 'Applying', cls: 'text-blue-400 bg-blue-500/10 border-blue-500/20' };
  if (stage === 'unsupported_host') return { label: 'Manual only', cls: 'text-white/50 bg-white/5 border-white/10' };
  return { label: 'Pending', cls: 'text-white/40 bg-white/5 border-white/10' };
}

// ── Main component ───────────────────────────────────────────────────────────

export default function BiosStep({
  biosConfig,
  biosStatus,
  firmwareInfo,
  orchestratorState,
  resumeState,
  restartCapability,
  onApplySupportedChanges,
  onVerifyAndContinue,
  onRestartToBios,
}: Props) {
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [restartState, setRestartState] = useState<RestartState>('idle');
  const [restartError, setRestartError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<'apply' | 'verify' | null>(null);
  const [applyModes, setApplyModes] = useState<Record<string, BiosApplyMode>>({});
  const [selectedSettingId, setSelectedSettingId] = useState<string | null>(null);

  // Hydrate from orchestrator
  useEffect(() => {
    if (!orchestratorState) return;
    setApplyModes(Object.fromEntries(
      orchestratorState.settings.map(s => [s.id, s.applyMode]),
    ));
    setChecked(new Set(
      orchestratorState.settings
        .filter(s => s.verificationStatus === 'verified')
        .map(s => s.id),
    ));
    const hasSelectedSetting = selectedSettingId
      ? orchestratorState.settings.some(s => s.id === selectedSettingId)
      : false;
    if ((!selectedSettingId || !hasSelectedSetting) && orchestratorState.settings.length > 0) {
      const firstPending = orchestratorState.settings.find(s => s.verificationStatus !== 'verified' && s.required);
      setSelectedSettingId((firstPending ?? orchestratorState.settings[0]).id);
    }
  }, [orchestratorState]); // eslint-disable-line react-hooks/exhaustive-deps

  // Derive settings list
  const allItems = useMemo(() => {
    if (orchestratorState?.settings.length) return orchestratorState.settings;
    return [
      ...biosConfig.enable.map(s => ({
        ...s, id: s.name, type: 'enable' as const, required: true,
        verificationStatus: 'unknown' as const,
        currentStatus: 'Unknown', currentValue: null, recommendedValue: s.value ?? 'Enable',
        confidence: 'low' as const, detectionMethod: 'Manual', riskLevel: 'low' as const,
        supportLevel: 'manual' as const, allowedApplyModes: ['manual' as const, 'skipped' as const],
        applyMode: 'manual' as const, verificationDetail: 'Not yet verified.',
      })),
      ...biosConfig.disable.map(s => ({
        ...s, id: s.name, type: 'disable' as const, required: true,
        verificationStatus: 'unknown' as const,
        currentStatus: 'Unknown', currentValue: null, recommendedValue: s.value ?? 'Disable',
        confidence: 'low' as const, detectionMethod: 'Manual', riskLevel: 'low' as const,
        supportLevel: 'manual' as const, allowedApplyModes: ['manual' as const, 'skipped' as const],
        applyMode: 'manual' as const, verificationDetail: 'Not yet verified.',
      })),
    ];
  }, [orchestratorState, biosConfig]);

  const counts = useSummaryCounts(allItems as BiosSettingPlan[]);
  const isMacHost = firmwareInfo?.hostContext === 'running_on_mac';
  const restartSupported = restartCapability?.supported === true;
  const restartMethod = restartCapability?.method ?? 'none';

  const requiredItems = allItems.filter(item => item.required);
  const allDone = requiredItems.every(item => checked.has(item.id) || item.verificationStatus === 'verified');
  const selectedSetting = allItems.find(s => s.id === selectedSettingId) ?? null;
  const stage = orchestratorState?.stage ?? 'idle';
  const badge = stageBadge(stage, orchestratorState?.readyToBuild ?? false);

  const toggle = (key: string) => setChecked(p => { const n = new Set(p); n.has(key) ? n.delete(key) : n.add(key); return n; });

  const buildSelectedChanges = (): Record<string, BiosSettingSelection> => Object.fromEntries(
    allItems.map(item => [
      item.id,
      { approved: checked.has(item.id) || item.verificationStatus === 'verified', applyMode: applyModes[item.id] ?? 'manual' },
    ]),
  );

  const handleRestartToBios = async () => {
    setRestartState('restarting');
    setRestartError(null);
    try {
      const result = await onRestartToBios(buildSelectedChanges());
      if (!result.supported) {
        setRestartState('unsupported');
        setRestartError(result.error || null);
      }
    } catch (e: any) {
      setRestartState('unsupported');
      setRestartError(e.message || 'Unknown error');
    }
  };

  const handleApplySupported = async () => {
    setBusyAction('apply');
    setActionMessage(null);
    try {
      const result = await onApplySupportedChanges(buildSelectedChanges());
      setActionMessage(result.message);
    } catch (e: any) {
      setActionMessage(e.message || 'Failed to apply changes.');
    } finally {
      setBusyAction(null);
    }
  };

  const handleVerifyAndContinue = async () => {
    setBusyAction('verify');
    setActionMessage(null);
    try {
      const ready = await onVerifyAndContinue(buildSelectedChanges());
      if (!ready) setActionMessage('Required settings must be verified before continuing.');
    } catch (e: any) {
      setActionMessage(e.message || 'Failed to verify settings.');
    } finally {
      setBusyAction(null);
    }
  };

  const handleMarkAllVerified = () => {
    const all = new Set(checked);
    allItems.forEach(item => all.add(item.id));
    setChecked(all);
    setActionMessage(null);
  };

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <div className="h-full flex flex-col gap-0">

      {/* ── Restarting overlay ─────────────────────────────────────── */}
      <AnimatePresence>
        {restartState === 'restarting' && (
          <motion.div
            initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
            className="absolute inset-0 z-50 bg-black/80 backdrop-blur-xl flex flex-col items-center justify-center gap-6 rounded-3xl"
          >
            <motion.div animate={{ rotate: 360 }} transition={{ duration: 1.5, repeat: Infinity, ease: 'linear' }}>
              <Loader2 className="w-10 h-10 text-blue-400" />
            </motion.div>
            <div className="text-center max-w-xs">
              <div className="text-xl font-bold text-white mb-2">Restarting to firmware</div>
              <div className="text-sm text-white/40 leading-relaxed">
                Your system will boot into UEFI settings. Return to the app afterward to continue.
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Confirm restart dialog ────────────────────────────────── */}
      <AnimatePresence>
        {restartState === 'confirming' && (
          <motion.div
            initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
            className="absolute inset-0 z-50 bg-black/70 backdrop-blur-lg flex items-center justify-center rounded-3xl p-8"
          >
            <motion.div
              initial={{ scale: 0.95, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={{ scale: 0.95, opacity: 0 }}
              className="bg-[#111] border border-white/10 rounded-2xl p-6 max-w-sm w-full shadow-2xl space-y-4"
            >
              <div className="flex items-center justify-between">
                <h3 className="text-base font-bold text-white">Restart to firmware?</h3>
                <button onClick={() => setRestartState('idle')} className="p-1.5 hover:bg-white/5 rounded-lg transition-colors cursor-pointer">
                  <X className="w-4 h-4 text-white/30" />
                </button>
              </div>
              <p className="text-sm text-white/50 leading-relaxed">
                Your system will <strong className="text-white/70">immediately restart</strong> into firmware settings. Save any open work first.
              </p>
              <p className="text-xs text-white/30 leading-relaxed">
                The app can reopen here after restart and continue from this step.
              </p>
              <div className="flex gap-3 pt-1">
                <button onClick={() => setRestartState('idle')} className="flex-1 px-4 py-2.5 bg-white/5 border border-white/10 rounded-xl text-sm font-semibold text-white/60 hover:bg-white/8 transition-all cursor-pointer">
                  Cancel
                </button>
                <button onClick={handleRestartToBios} className="flex-1 px-4 py-2.5 bg-amber-500 text-black rounded-xl text-sm font-bold hover:bg-amber-400 transition-all cursor-pointer">
                  Restart now
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── 1. Header zone ────────────────────────────────────────── */}
      <div className="flex-shrink-0 pb-5">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-4xl font-bold text-white tracking-tight">Firmware Preparation</h2>
            <p className="text-[#888888] text-sm font-medium mt-1.5 max-w-md leading-relaxed">
              {isMacHost
                ? 'Verify these firmware settings on your target PC before installing.'
                : 'These settings help macOS boot reliably on this hardware.'}
            </p>
          </div>
          <span className={`flex-shrink-0 px-3 py-1.5 rounded-lg text-[11px] font-bold border ${badge.cls}`}>
            {badge.label}
          </span>
        </div>
      </div>

      {/* ── 2. Summary strip ──────────────────────────────────────── */}
      <div className="flex-shrink-0 flex flex-wrap items-center gap-x-4 gap-y-1 pb-5 text-[11px] text-white/35 font-medium">
        <span className="flex items-center gap-1.5">
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
          {counts.verified} verified
        </span>
        <span className="flex items-center gap-1.5">
          <span className="w-1.5 h-1.5 rounded-full bg-amber-400" />
          {counts.pending} pending
        </span>
        <span className="text-white/15">|</span>
        <span>{counts.manualOnly} manual only</span>
        {counts.autoEligible > 0 && <span>{counts.autoEligible} auto-eligible</span>}
        <span className="text-white/15">|</span>
        <span>{counts.required} required</span>
      </div>

      {/* ── Resume notice ─────────────────────────────────────────── */}
      {orchestratorState && stage !== 'idle' && stage !== 'complete' && stage !== 'planned' && (
        <motion.div
          initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}
          className="flex-shrink-0 mb-4 flex items-start gap-3 px-4 py-3 rounded-xl bg-blue-500/5 border border-blue-500/10"
        >
          <Info className="w-4 h-4 text-blue-400/60 flex-shrink-0 mt-0.5" />
          <div className="text-xs text-blue-300/50 leading-relaxed">
            {stage === 'awaiting_return' || stage === 'resumed_from_firmware'
              ? 'Session resumed after firmware restart. The app is re-verifying settings before unlocking the build step.'
              : stage === 'partially_verified'
              ? 'Some settings are still pending. Verify the remaining items or restart to firmware to apply them.'
              : stage === 'blocked'
              ? 'Required settings could not be verified. Review the items below and verify or fix them.'
              : stage === 'unsupported_host'
              ? 'Automatic firmware restart is unavailable on this host. Use the manual path and verify the settings after returning.'
              : 'BIOS preparation is in progress.'}
          </div>
        </motion.div>
      )}

      {resumeState?.hasSession && (
        <motion.div
          initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}
          className={`flex-shrink-0 mb-4 flex items-start gap-3 px-4 py-3 rounded-xl border ${
            resumeState.stale
              ? 'bg-amber-500/5 border-amber-500/10'
              : 'bg-white/[0.02] border-white/5'
          }`}
        >
          <Info className={`w-4 h-4 flex-shrink-0 mt-0.5 ${resumeState.stale ? 'text-amber-400/60' : 'text-white/20'}`} />
          <div className={`text-xs leading-relaxed ${resumeState.stale ? 'text-amber-300/50' : 'text-white/30'}`}>
            {resumeState.message}
          </div>
        </motion.div>
      )}

      {/* ── macOS host notice ─────────────────────────────────────── */}
      {isMacHost && (
        <motion.div
          initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}
          className="flex-shrink-0 mb-4 flex items-start gap-3 px-4 py-3 rounded-xl bg-white/[0.02] border border-white/5"
        >
          <Info className="w-4 h-4 text-white/20 flex-shrink-0 mt-0.5" />
          <div className="text-xs text-white/30 leading-relaxed">
            Running on a Mac — firmware detection is unavailable. Check each setting on the target PC and mark it as verified.
          </div>
        </motion.div>
      )}

      {/* ── 3. Main content: settings list + detail panel ──────────── */}
      <div className="flex-1 min-h-0 flex flex-col xl:flex-row gap-4 xl:gap-0 overflow-hidden">

        {/* ── Settings list (left) ────────────────────────────────── */}
        <div className="flex-1 overflow-y-auto xl:pr-4 custom-scrollbar">
          <div className="space-y-px">
            {allItems.map(setting => {
              const isVerified = checked.has(setting.id) || setting.verificationStatus === 'verified';
              const isSelected = setting.id === selectedSettingId;
              const isRequired = setting.required;
              const statusIcon = isVerified
                ? <CheckCircle2 className="w-4 h-4 text-emerald-400" />
                : setting.verificationStatus === 'unverified'
                ? <AlertTriangle className="w-4 h-4 text-red-400" />
                : <HelpCircle className="w-4 h-4 text-white/20" />;

              return (
                <div
                  key={setting.id}
                  onClick={() => setSelectedSettingId(setting.id)}
                  className={`flex items-center gap-3 px-4 py-3 cursor-pointer transition-all rounded-lg group ${
                    isSelected ? 'bg-white/[0.04]' : 'hover:bg-white/[0.02]'
                  }`}
                >
                  {/* Checkbox */}
                  <button
                    onClick={e => { e.stopPropagation(); toggle(setting.id); }}
                    className={`w-5 h-5 rounded-md border flex items-center justify-center flex-shrink-0 transition-all cursor-pointer ${
                      isVerified
                        ? 'bg-emerald-500/20 border-emerald-500/40 text-emerald-400'
                        : 'border-white/15 hover:border-white/30'
                    }`}
                  >
                    {isVerified && <Check className="w-3 h-3 stroke-[3px]" />}
                  </button>

                  {/* Name + status */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className={`text-sm font-medium truncate ${isVerified ? 'text-white/40' : 'text-white/80'}`}>
                        {setting.plainTitle ?? setting.name}
                      </span>
                      {isRequired && !isVerified && (
                        <span className="text-[9px] font-bold text-amber-400/60 uppercase tracking-wider flex-shrink-0">Required</span>
                      )}
                    </div>
                    <div className="text-[11px] text-white/25 mt-0.5 truncate">
                      {setting.recommendedValue} · {setting.confidence} confidence
                    </div>
                  </div>

                  {/* Status icon */}
                  <div className="flex-shrink-0">{statusIcon}</div>

                  {/* Selection indicator */}
                  {isSelected && <ChevronRight className="w-3.5 h-3.5 text-white/15 flex-shrink-0" />}
                </div>
              );
            })}
          </div>
        </div>

        {/* ── Detail panel (right) ────────────────────────────────── */}
        <div className="w-full xl:w-[280px] xl:flex-shrink-0 border-t xl:border-t-0 xl:border-l border-white/5 pt-4 xl:pt-0 xl:pl-5 overflow-y-auto custom-scrollbar max-h-[240px] xl:max-h-none">
          <AnimatePresence mode="wait">
            {selectedSetting ? (
              <motion.div
                key={selectedSetting.id}
                initial={{ opacity: 0, x: 8 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -8 }}
                transition={{ duration: 0.15 }}
                className="space-y-4"
              >
                {/* Setting name */}
                <div>
                  <h3 className="text-base font-bold text-white/80">{selectedSetting.plainTitle ?? selectedSetting.name}</h3>
                  {selectedSetting.plainTitle && selectedSetting.plainTitle !== selectedSetting.name && (
                    <div className="text-[10px] text-white/20 font-mono mt-0.5">{selectedSetting.name}</div>
                  )}
                </div>

                {/* Description */}
                <p className="text-xs text-white/40 leading-relaxed">{selectedSetting.description}</p>

                {/* Why macOS cares */}
                {selectedSetting.jargonDef && (
                  <div className="space-y-1">
                    <div className="text-[10px] font-semibold text-white/25 uppercase tracking-wider">What this means</div>
                    <p className="text-xs text-white/30 leading-relaxed">{selectedSetting.jargonDef}</p>
                  </div>
                )}

                {/* Metadata */}
                <div className="space-y-2.5 pt-2 border-t border-white/5">
                  <DetailRow label="Status" value={selectedSetting.currentStatus} />
                  <DetailRow label="Recommended" value={selectedSetting.recommendedValue} />
                  <DetailRow label="Detection" value={selectedSetting.detectionMethod} />
                  <DetailRow label="Confidence" value={selectedSetting.confidence} />
                  <DetailRow label="Risk" value={selectedSetting.riskLevel} highlight={selectedSetting.riskLevel === 'high'} />
                  <DetailRow label="Verification" value={selectedSetting.verificationDetail} />
                  {selectedSetting.biosLocation && (
                    <DetailRow label="BIOS location" value={selectedSetting.biosLocation} mono />
                  )}
                </div>

                {/* Apply mode selector */}
                {selectedSetting.allowedApplyModes.length > 1 && (
                  <div className="space-y-1.5 pt-2 border-t border-white/5">
                    <div className="text-[10px] font-semibold text-white/25 uppercase tracking-wider">Apply mode</div>
                    <div className="flex gap-1.5">
                      {selectedSetting.allowedApplyModes.map(mode => {
                        const active = (applyModes[selectedSetting.id] ?? selectedSetting.applyMode) === mode;
                        return (
                          <button
                            key={mode}
                            onClick={() => setApplyModes(prev => ({ ...prev, [selectedSetting.id]: mode }))}
                            className={`px-2.5 py-1 rounded-lg text-[10px] font-semibold transition-colors cursor-pointer ${
                              active
                                ? 'bg-blue-500/15 text-blue-300 border border-blue-500/25'
                                : 'bg-white/[0.03] text-white/30 border border-white/5 hover:text-white/50'
                            }`}
                          >
                            {mode}
                          </button>
                        );
                      })}
                    </div>
                    <div className="text-[10px] text-white/15">
                      Backend: {selectedSetting.supportLevel}
                    </div>
                  </div>
                )}

                {/* Quick verify button */}
                {!(checked.has(selectedSetting.id) || selectedSetting.verificationStatus === 'verified') && (
                  <button
                    onClick={() => toggle(selectedSetting.id)}
                    className="w-full mt-2 px-4 py-2 rounded-xl bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 text-xs font-semibold hover:bg-emerald-500/15 transition-all cursor-pointer"
                  >
                    Mark as verified
                  </button>
                )}
              </motion.div>
            ) : (
              <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} className="pt-8 text-center">
                <p className="text-xs text-white/20">Select a setting to see details</p>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>

      {/* ── Firmware identity (subtle) ────────────────────────────── */}
      {firmwareInfo && !isMacHost && firmwareInfo.vendor !== 'Unknown' && (
        <div className="flex-shrink-0 pt-3 flex items-center gap-2 text-[10px] text-white/15">
          <ShieldCheck className="w-3 h-3" />
          <span>{firmwareInfo.vendor}</span>
          {firmwareInfo.version !== 'Unknown' && <span className="font-mono">{firmwareInfo.version}</span>}
          {firmwareInfo.firmwareMode !== 'Unknown' && <span>· {firmwareInfo.firmwareMode}</span>}
        </div>
      )}

      {/* ── 5. Action zone ────────────────────────────────────────── */}
      <div className="flex-shrink-0 pt-4 mt-3 border-t border-white/5">
        {/* Restart unsupported notice */}
        {restartState === 'unsupported' && (
          <motion.div
            initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }}
            className="mb-3 px-4 py-2.5 rounded-xl bg-white/[0.02] border border-white/5 text-xs text-white/35 leading-relaxed"
          >
            Firmware restart is not supported on this system.{' '}
            {restartError && <span className="text-white/25">{restartError} </span>}
            Enter BIOS manually using <span className="font-mono bg-white/5 px-1 rounded text-white/45">Del</span>,{' '}
            <span className="font-mono bg-white/5 px-1 rounded text-white/45">F2</span>, or{' '}
            <span className="font-mono bg-white/5 px-1 rounded text-white/45">F12</span> during POST.
          </motion.div>
        )}

        {/* Action message */}
        {actionMessage && (
          <div className="mb-3 text-[11px] text-white/35 leading-relaxed">{actionMessage}</div>
        )}

        <div className="flex flex-wrap items-center justify-between gap-3">
          {/* Left actions */}
          <div className="flex flex-wrap items-center gap-2">
            {/* Restart to firmware — capability-driven */}
            {restartSupported && (
              <button
                onClick={() => setRestartState('confirming')}
                className="flex items-center gap-1.5 px-3.5 py-2 rounded-xl border border-white/8 bg-white/[0.03] text-white/50 text-xs font-medium hover:bg-white/[0.06] hover:text-white/70 transition-all cursor-pointer"
              >
                <RotateCcw className="w-3.5 h-3.5" />
                Restart to firmware
              </button>
            )}

            {/* Apply automatic changes */}
            {orchestratorState && counts.autoEligible > 0 && (
              <button
                onClick={handleApplySupported}
                disabled={busyAction === 'apply'}
                className="px-3.5 py-2 rounded-xl border border-white/8 bg-white/[0.03] text-white/50 text-xs font-medium hover:bg-white/[0.06] hover:text-white/70 disabled:opacity-40 disabled:cursor-not-allowed transition-all cursor-pointer"
              >
                {busyAction === 'apply' ? 'Applying...' : 'Apply supported changes'}
              </button>
            )}

            {/* I already changed these */}
            <button
              onClick={handleMarkAllVerified}
              className="px-3.5 py-2 rounded-xl border border-white/8 bg-white/[0.03] text-white/50 text-xs font-medium hover:bg-white/[0.06] hover:text-white/70 transition-all cursor-pointer"
            >
              I've verified these settings
            </button>
          </div>

          {/* Right: continue */}
          <div className="flex items-center gap-3">
            <span className="text-[11px] text-white/20">
              {requiredItems.filter(item => checked.has(item.id) || item.verificationStatus === 'verified').length} / {requiredItems.length}
            </span>
            <button
              onClick={handleVerifyAndContinue}
              disabled={!allDone || busyAction === 'verify'}
              className={`px-6 py-2.5 rounded-xl text-sm font-semibold transition-all ${
                allDone
                  ? 'bg-blue-600 text-white hover:bg-blue-500 shadow-lg shadow-blue-600/15 cursor-pointer'
                  : 'bg-white/[0.03] text-white/25 cursor-not-allowed'
              }`}
            >
              {busyAction === 'verify' ? 'Verifying...' : 'Continue'}
            </button>
          </div>
        </div>

        {/* ── 6. Reboot resume notice ─────────────────────────────── */}
        {restartSupported ? (
          <div className="mt-3 text-[10px] text-white/15 leading-relaxed">
            The app can reopen here after a firmware restart and continue from this step.
          </div>
        ) : (
          <div className="mt-3 text-[10px] text-white/15 leading-relaxed">
            Firmware restart is not available through the current backend{restartMethod !== 'none' ? ` (${restartMethod})` : ''}. Use your vendor hotkey and return here to verify the changes.
          </div>
        )}
      </div>
    </div>
  );
}

// ── Detail row ───────────────────────────────────────────────────────────────

function DetailRow({ label, value, highlight, mono }: { label: string; value: string; highlight?: boolean; mono?: boolean }) {
  return (
    <div className="flex items-start gap-2">
      <span className="text-[10px] font-medium text-white/20 w-20 flex-shrink-0 pt-px">{label}</span>
      <span className={`text-[11px] leading-relaxed ${
        highlight ? 'text-amber-400/60' : 'text-white/35'
      } ${mono ? 'font-mono bg-white/[0.03] px-1.5 py-0.5 rounded' : ''}`}>
        {value}
      </span>
    </div>
  );
}
