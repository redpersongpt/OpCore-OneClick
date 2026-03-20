// ── Community Knowledge Panel ───────────────────────────────────────────────
// Shows known issues and fixes for similar hardware, from a curated static dataset.

import React, { useState } from 'react';
import {
  ChevronDown, ChevronRight, Users, AlertTriangle, Lightbulb, ShieldAlert,
  CheckCircle,
} from 'lucide-react';
import type { CommunityIssue } from '../data/communityKnowledge';

interface Props {
  issues: CommunityIssue[];
}

function SeverityBadge({ severity }: { severity: CommunityIssue['severity'] }) {
  const styles: Record<string, string> = {
    critical: 'text-red-400 bg-red-500/10 border-red-500/20',
    common_fix: 'text-amber-400 bg-amber-500/10 border-amber-500/20',
    tip: 'text-white/40 bg-white/5 border-white/10',
  };
  const labels: Record<string, string> = {
    critical: 'Critical',
    common_fix: 'Common Fix',
    tip: 'Tip',
  };
  return (
    <span className={`px-2 py-0.5 rounded-md text-[9px] font-bold uppercase border ${styles[severity] ?? styles.tip}`}>
      {labels[severity] ?? severity}
    </span>
  );
}

function SeverityIcon({ severity }: { severity: CommunityIssue['severity'] }) {
  switch (severity) {
    case 'critical':
      return <ShieldAlert className="w-4 h-4 text-red-400 flex-shrink-0" />;
    case 'common_fix':
      return <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0" />;
    default:
      return <Lightbulb className="w-4 h-4 text-white/30 flex-shrink-0" />;
  }
}

function IssueRow({ issue }: { issue: CommunityIssue; key?: string }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="border-b border-white/[0.03] last:border-0">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-3 px-5 py-3 hover:bg-white/[0.02] transition-colors cursor-pointer text-left"
      >
        <SeverityIcon severity={issue.severity} />
        <div className="flex-1 min-w-0">
          <span className="text-sm font-semibold text-white/70">{issue.title}</span>
          <p className="text-xs text-white/30 leading-relaxed mt-0.5 line-clamp-1">{issue.description}</p>
        </div>
        <SeverityBadge severity={issue.severity} />
        {expanded ? <ChevronDown className="w-3.5 h-3.5 text-white/15" /> : <ChevronRight className="w-3.5 h-3.5 text-white/15" />}
      </button>
      {expanded && (
        <div className="px-5 pb-4 ml-7 space-y-3">
          <p className="text-xs text-white/40 leading-relaxed">{issue.description}</p>
          <div className="flex items-start gap-2 px-3 py-2.5 rounded-lg bg-emerald-500/5 border border-emerald-500/10">
            <CheckCircle className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0 mt-0.5" />
            <div>
              <span className="text-[10px] font-bold text-emerald-400/50 uppercase tracking-widest">Fix / Workaround</span>
              <p className="text-xs text-emerald-300/60 leading-relaxed mt-1">{issue.fix}</p>
            </div>
          </div>
          <p className="text-[10px] text-white/15">Source: {issue.source}</p>
        </div>
      )}
    </div>
  );
}

export default function CommunityPanel({ issues }: Props) {
  if (issues.length === 0) return null;

  const critical = issues.filter(i => i.severity === 'critical');
  const rest = issues.filter(i => i.severity !== 'critical');

  return (
    <div className="rounded-2xl border border-white/6 overflow-hidden">
      <div className="flex items-center gap-3 px-5 py-3.5 bg-white/[0.02]">
        <Users className="w-4 h-4 text-white/30" />
        <span className="text-xs font-bold text-white/60 uppercase tracking-widest flex-1">
          Community Knowledge
        </span>
        <span className="text-[10px] text-white/20 font-mono">
          {issues.length} {issues.length === 1 ? 'issue' : 'issues'}
        </span>
      </div>
      <div className="border-t border-white/5">
        {critical.map(issue => <IssueRow key={issue.id} issue={issue} />)}
        {rest.map(issue => <IssueRow key={issue.id} issue={issue} />)}
      </div>
    </div>
  );
}
