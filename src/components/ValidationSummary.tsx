import React from 'react';
import { CheckCircle, AlertTriangle, ShieldAlert, Loader2 } from 'lucide-react';
import type { ValidationResult } from '../../electron/configValidator';

interface Props {
  result: ValidationResult | null;
  isRunning: boolean;
}

const ValidationSummary: React.FC<Props> = ({ result, isRunning }) => {
  if (isRunning) {
    return (
      <div className="flex items-center gap-2 px-3 py-2 rounded-xl bg-white/5 border border-white/10">
        <Loader2 className="w-4 h-4 text-white/50 animate-spin" />
        <span className="text-xs text-white/50">Validating configuration...</span>
      </div>
    );
  }

  if (!result) return null;

  if (result.overall === 'pass') {
    return (
      <div className="flex items-center gap-2 px-3 py-2 rounded-xl bg-emerald-500/8 border border-emerald-500/20">
        <CheckCircle className="w-4 h-4 text-emerald-400" />
        <span className="text-xs text-emerald-300/90">Configuration verified</span>
      </div>
    );
  }

  const blockers = result.issues.filter(i => i.severity === 'blocked');
  const warnings = result.issues.filter(i => i.severity === 'warning');

  return (
    <div className="space-y-2">
      {blockers.length > 0 && (
        <div className="px-3 py-2.5 rounded-xl bg-red-500/8 border border-red-500/20">
          <div className="flex items-center gap-2 mb-1.5">
            <ShieldAlert className="w-4 h-4 text-red-400 flex-shrink-0" />
            <span className="text-xs font-semibold text-red-300">Flash blocked — {blockers.length} issue{blockers.length > 1 ? 's' : ''}</span>
          </div>
          <ul className="space-y-1 ml-6">
            {blockers.map((issue, i) => (
              <li key={i} className="text-[11px] text-red-300/70 leading-snug">
                {issue.message}
                <span className="block text-[10px] text-red-200/65">
                  {issue.component} at {issue.expectedPath}
                </span>
                <span className="block text-[10px] text-red-300/50">{issue.actualCondition}</span>
                {issue.detail && (
                  <span className="block text-[10px] text-red-300/40 font-mono">{issue.detail}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {warnings.length > 0 && (
        <div className="px-3 py-2.5 rounded-xl bg-amber-500/6 border border-amber-500/15">
          <div className="flex items-center gap-2 mb-1.5">
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0" />
            <span className="text-xs font-semibold text-amber-300">{warnings.length} warning{warnings.length > 1 ? 's' : ''}</span>
          </div>
          <ul className="space-y-1 ml-6">
            {warnings.map((issue, i) => (
              <li key={i} className="text-[11px] text-amber-300/70 leading-snug">
                {issue.message}
                <span className="block text-[10px] text-amber-200/65">
                  {issue.component} at {issue.expectedPath}
                </span>
                <span className="block text-[10px] text-amber-300/50">{issue.actualCondition}</span>
                {issue.detail && (
                  <span className="block text-[10px] text-amber-300/40 font-mono">{issue.detail}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
};

export default ValidationSummary;
