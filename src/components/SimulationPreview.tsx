import React from 'react';
import { AlertTriangle, CheckCircle, FlaskConical, ShieldAlert } from 'lucide-react';
import type { SafeSimulationResult } from '../../electron/safeSimulation';
import type { CompatibilityReport } from '../../electron/compatibility';

interface SimulationPreviewProps {
  result: SafeSimulationResult;
  report: CompatibilityReport;
}

export default function SimulationPreview({ result, report }: SimulationPreviewProps) {
  const supportedCount = result.compatibilityMatrixSnapshot.filter((row) => row.status === 'supported').length;
  const experimentalCount = result.compatibilityMatrixSnapshot.filter((row) => row.status === 'experimental').length;
  const riskyCount = result.compatibilityMatrixSnapshot.filter((row) => row.status === 'risky').length;
  const blockedCount = result.compatibilityMatrixSnapshot.filter((row) => row.status === 'blocked').length;
  const likelyFailurePoints = [
    ...report.mostLikelyFailurePoints.map((point) => ({
      title: point.title,
      detail: point.detail,
      likelihood: point.likelihood,
    })),
    ...result.validationSummary.issues.slice(0, 3).map((issue) => ({
      title: issue.component,
      detail: issue.message,
      likelihood: issue.severity === 'blocked' ? 'very likely' : 'likely',
    })),
  ].filter((point, index, points) => (
    points.findIndex((candidate) => candidate.title === point.title && candidate.detail === point.detail) === index
  )).slice(0, 3);

  return (
    <div className="rounded-2xl border border-blue-500/15 bg-blue-500/6 p-5 space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-blue-300/80">
            <FlaskConical className="w-3.5 h-3.5" />
            Safe Simulation Preview
          </div>
          <div className="text-sm font-semibold text-white mt-2">
            Temp workspace: <span className="font-mono text-white/70">{result.workspacePath}</span>
          </div>
        </div>
        <div className={`px-2.5 py-1 rounded-full border text-[10px] font-bold uppercase tracking-widest ${
          result.validationSummary.overall === 'pass'
            ? 'border-emerald-500/20 bg-emerald-500/12 text-emerald-300'
            : result.validationSummary.overall === 'warning'
            ? 'border-amber-500/20 bg-amber-500/12 text-amber-300'
            : 'border-red-500/20 bg-red-500/12 text-red-300'
        }`}>
          Validation {result.validationSummary.overall}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 text-xs">
        <div className="rounded-xl border border-white/8 bg-white/4 p-3">
          <div className="text-white/45 uppercase tracking-widest text-[10px] font-bold mb-1">EFI Summary</div>
          <div className="text-white/75">Config hash: {result.efiSummary.configHash?.slice(0, 16) ?? 'missing'}</div>
          <div className="text-white/45 mt-1">{result.efiSummary.requiredResources.length} generated resource entries</div>
        </div>
        <div className="rounded-xl border border-white/8 bg-white/4 p-3">
          <div className="text-white/45 uppercase tracking-widest text-[10px] font-bold mb-1">Compatibility Snapshot</div>
          <div className="text-white/75">{supportedCount} supported, {experimentalCount} experimental, {riskyCount} risky, {blockedCount} blocked</div>
          <div className="text-white/45 mt-1">Versions stay visible by confidence level so you can see the best starting point and the stretch targets side by side.</div>
        </div>
        <div className="rounded-xl border border-white/8 bg-white/4 p-3">
          <div className="text-white/45 uppercase tracking-widest text-[10px] font-bold mb-1">Recovery Readiness</div>
          <div className="text-white/75">{result.recoveryReadiness.certainty}</div>
          <div className="text-white/45 mt-1">{result.recoveryReadiness.recommendation}</div>
        </div>
        <div className="rounded-xl border border-white/8 bg-white/4 p-3">
          <div className="text-white/45 uppercase tracking-widest text-[10px] font-bold mb-1">Resource Plan</div>
          <div className="text-white/75">{result.resourcePlan.resources.length} advisory resource entries</div>
          <div className="text-white/45 mt-1">Simulation never downloads or writes disks. This is a planning manifest only.</div>
        </div>
      </div>

      {likelyFailurePoints.length > 0 && (
        <div className="space-y-2">
          <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-rose-200/80">
            <AlertTriangle className="w-3.5 h-3.5" />
            Most Likely Failure Points
          </div>
          {likelyFailurePoints.map((point) => (
            <div key={`${point.title}-${point.detail}`} className="rounded-xl border border-white/8 bg-white/4 p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="text-xs font-semibold text-white/78">{point.title}</div>
                <div className="text-[9px] uppercase tracking-widest text-white/35">{point.likelihood}</div>
              </div>
              <div className="text-[11px] text-white/55 leading-relaxed mt-1.5">{point.detail}</div>
            </div>
          ))}
        </div>
      )}

      {result.blockers.length > 0 && (
        <div className="space-y-2">
          <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-red-300/80">
            <ShieldAlert className="w-3.5 h-3.5" />
            Blockers
          </div>
          {result.blockers.map((blocker) => (
            <div key={blocker} className="flex items-start gap-2 text-xs text-red-200/75">
              <ShieldAlert className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{blocker}</span>
            </div>
          ))}
        </div>
      )}

      {result.warnings.length > 0 && (
        <div className="space-y-2">
          <div className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-amber-300/80">
            <AlertTriangle className="w-3.5 h-3.5" />
            Warnings
          </div>
          {result.warnings.map((warning) => (
            <div key={warning} className="flex items-start gap-2 text-xs text-amber-100/70">
              <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{warning}</span>
            </div>
          ))}
        </div>
      )}

      {result.blockers.length === 0 && result.warnings.length === 0 && (
        <div className="flex items-center gap-2 text-xs text-emerald-200/75">
          <CheckCircle className="w-4 h-4" />
          <span>No blockers or warnings were produced by the current advisory simulation inputs.</span>
        </div>
      )}

      <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-2 text-[11px] text-white/55">
        {result.disclaimer}
      </div>
    </div>
  );
}
