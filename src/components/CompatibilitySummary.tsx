import React from 'react';
import { CheckCircle, AlertTriangle, ShieldAlert, Info, HelpCircle } from 'lucide-react';
import type { CompatibilityReport } from '../../electron/compatibility';

interface Props {
  report: CompatibilityReport | null;
}

const CompatibilitySummary: React.FC<Props> = ({ report }) => {
  if (!report) return null;

  const getLevelStyle = () => {
    switch (report.level) {
      case 'supported':
        return {
          icon: <CheckCircle className="w-5 h-5 text-emerald-400" />,
          bg: 'bg-emerald-500/8',
          border: 'border-emerald-500/20',
          text: 'text-emerald-300',
          label: 'Supported Target'
        };
      case 'supported_with_warnings':
        return {
          icon: <AlertTriangle className="w-5 h-5 text-amber-400" />,
          bg: 'bg-amber-500/8',
          border: 'border-amber-500/20',
          text: 'text-amber-300',
          label: 'Supported with Warnings'
        };
      case 'partial_support':
        return {
          icon: <Info className="w-5 h-5 text-blue-400" />,
          bg: 'bg-blue-500/8',
          border: 'border-blue-500/20',
          text: 'text-blue-300',
          label: 'Partial Support'
        };
      case 'low_confidence':
        return {
          icon: <HelpCircle className="w-5 h-5 text-rose-400" />,
          bg: 'bg-rose-500/8',
          border: 'border-rose-500/20',
          text: 'text-rose-300',
          label: 'Low Confidence'
        };
      case 'blocked':
        return {
          icon: <ShieldAlert className="w-5 h-5 text-red-400" />,
          bg: 'bg-red-500/8',
          border: 'border-red-500/20',
          text: 'text-red-300',
          label: 'Blocked'
        };
      default:
        return {
          icon: <Info className="w-5 h-5 text-white/50" />,
          bg: 'bg-white/5',
          border: 'border-white/10',
          text: 'text-white/70',
          label: 'Unknown'
        };
    }
  };

  const style = getLevelStyle();

  const confidenceLabel = report.confidence === 'high' ? 'High confidence detection'
    : report.confidence === 'medium' ? 'Medium confidence — some values inferred'
    : 'Low confidence — manual verification recommended';

  return (
    <div className={`p-4 rounded-2xl border ${style.bg} ${style.border} space-y-3`}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          {style.icon}
          <div>
            <div className={`text-sm font-bold ${style.text}`}>{style.label}</div>
            <div className="text-[10px] text-white/30 font-medium">
              {confidenceLabel}
            </div>
          </div>
        </div>
      </div>

      <div className="text-xs text-white/70 leading-relaxed">
        {report.explanation}
      </div>

      {report.warnings.length > 0 && (
        <div className="pt-2 border-t border-white/5 space-y-1">
          {report.warnings.map((w, i) => (
            <div key={i} className="flex items-start gap-2 text-[11px] text-amber-300/60">
              <span className="mt-1 w-1 h-1 rounded-full bg-amber-400/40 flex-shrink-0" />
              {w}
            </div>
          ))}
        </div>
      )}

      {report.manualVerificationRequired && (
        <div className="mt-2 flex items-center gap-2 px-2 py-1.5 rounded-lg bg-white/5 border border-white/10">
          <AlertTriangle className="w-3 h-3 text-amber-400/70" />
          <span className="text-[10px] text-amber-200/50 font-medium">Manual BIOS configuration still required</span>
        </div>
      )}
    </div>
  );
};

export default CompatibilitySummary;
