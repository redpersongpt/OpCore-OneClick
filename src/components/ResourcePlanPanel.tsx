import React from 'react';
import { AlertTriangle, CheckCircle2, Download, FileCog, Package, ShieldAlert } from 'lucide-react';
import type { ResourcePlan } from '../../electron/resourcePlanner';

interface ResourcePlanPanelProps {
  plan: ResourcePlan | null;
}

const OUTCOME_STYLE: Record<NonNullable<ResourcePlan['resources'][number]>['validationOutcome'], {
  label: string;
  className: string;
}> = {
  verified: {
    label: 'Verified',
    className: 'border-emerald-500/20 bg-emerald-500/12 text-emerald-300',
  },
  warning: {
    label: 'Warning',
    className: 'border-amber-500/20 bg-amber-500/12 text-amber-300',
  },
  blocked: {
    label: 'Blocked',
    className: 'border-red-500/20 bg-red-500/12 text-red-300',
  },
  pending_manual: {
    label: 'Pending',
    className: 'border-white/12 bg-white/6 text-white/65',
  },
};

function kindIcon(kind: ResourcePlan['resources'][number]['kind']) {
  if (kind === 'driver') return <Package className="w-3.5 h-3.5" />;
  if (kind === 'payload') return <Download className="w-3.5 h-3.5" />;
  return <FileCog className="w-3.5 h-3.5" />;
}

export default function ResourcePlanPanel({ plan }: ResourcePlanPanelProps) {
  if (!plan) {
    return (
      <div className="rounded-2xl border border-white/8 bg-white/3 p-4">
        <div className="text-[10px] font-bold uppercase tracking-widest text-white/25">Resource Plan</div>
        <div className="text-xs text-white/55 mt-2">Preparing advisory resource plan…</div>
      </div>
    );
  }

  return (
    <div className="rounded-2xl border border-white/8 bg-white/3 p-4 space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="text-[10px] font-bold uppercase tracking-widest text-white/25">
          Resource Plan
        </div>
        <div className="text-[10px] text-white/35">
          {plan.resources.length} tracked resources
        </div>
      </div>

      <div className="space-y-2">
        {plan.resources.map((resource) => {
          const outcome = OUTCOME_STYLE[resource.validationOutcome];
          return (
            <div key={`${resource.kind}:${resource.name}`} className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-semibold text-white">
                    {kindIcon(resource.kind)}
                    <span>{resource.name}</span>
                    <span className="text-[10px] uppercase tracking-widest text-white/30">{resource.sourceClass}</span>
                  </div>
                  <div className="text-[11px] text-white/45 mt-1">{resource.source}</div>
                  <div className="text-[11px] text-white/60 mt-1">
                    Expected identity/version: <span className="font-mono">{resource.expectedIdentityOrVersion}</span>
                  </div>
                </div>
                <div className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full border text-[10px] font-bold uppercase tracking-widest ${outcome.className}`}>
                  {resource.validationOutcome === 'verified'
                    ? <CheckCircle2 className="w-3.5 h-3.5" />
                    : resource.validationOutcome === 'blocked'
                    ? <ShieldAlert className="w-3.5 h-3.5" />
                    : <AlertTriangle className="w-3.5 h-3.5" />}
                  {outcome.label}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
