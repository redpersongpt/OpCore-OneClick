import React, { useState } from 'react';
import { Clipboard, Check } from 'lucide-react';
import { redactSensitiveText } from '../lib/diagnosticRedaction';

interface Props {
  /** Optional extra context to append to the diagnostics block. */
  extraContext?: string;
  className?: string;
}

export default function CopyDiagnosticsButton({ extraContext, className }: Props) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCopy = async () => {
    setError(null);
    try {
      // Gather diagnostics from the main process
      const d = await window.electron.getDiagnostics();
      const kextSourceSummary = d.kextSources && Object.keys(d.kextSources).length > 0
        ? Object.entries(d.kextSources).map(([name, source]) => `${name}: ${source}`).join(', ')
        : 'No kext fetch yet';
      const compatibilitySummary = d.compatibilityState
        ? `${d.compatibilityState.level} | ${d.compatibilityState.recommendedVersion || 'No recommended version'} | ${d.compatibilityState.explanation}`
        : 'Not yet evaluated';
      const validationSummary = d.validationSummary
        ? `${d.validationSummary.overall} | issues=${d.validationSummary.issueCount}`
        : 'No validation result';
      const validationTraceSummary = d.validationSummary?.firstFailureTrace
        ? `${d.validationSummary.firstFailureTrace.code} | ${d.validationSummary.firstFailureTrace.component} | ${d.validationSummary.firstFailureTrace.expectedPath} | ${d.validationSummary.firstFailureTrace.source} | ${d.validationSummary.firstFailureTrace.detail}`
        : 'No validation trace';
      const diskSummary = d.diskContext.selectedDevice
        ? `${d.diskContext.selectedDevice} | table=${d.diskContext.partitionTable ?? 'unknown'} | system=${d.diskContext.isSystemDisk ?? 'unknown'}`
        : 'No disk selected';

      const lines = [
        'OpCore-OneClick Diagnostics',
        '===========================',
        `Timestamp:     ${d.timestamp}`,
        `App Version:   ${d.version}`,
        `Platform:      ${d.platform} (${d.arch})`,
        `Session:       ${d.sessionFingerprint}`,
        `Trigger:       ${d.trigger}`,
        `Compat Mode:   ${d.compatMode}`,
        '',
        'Environment:',
        `Hardware:      ${d.hardware || 'Hardware scan not completed'}`,
        `Confidence:    ${d.confidence || 'Not yet scanned'}`,
        `Compatibility: ${compatibilitySummary}`,
        `Firmware:      ${d.firmware || 'Not probed'}`,
        '',
        'Kexts & Recovery:',
        `Failed Kexts:  ${d.failedKexts.length > 0 ? d.failedKexts.join(', ') : 'No failures'}`,
        `Kext Sources:  ${kextSourceSummary}`,
        `Validation:    ${validationSummary}`,
        `Trace:         ${validationTraceSummary}`,
        `Recovery:      Attempts=${d.recoveryStats?.attempts ?? 0}, HTTP=${(d.recoveryStats as any)?.lastHttpCode ?? 'n/a'}, Error=${d.recoveryStats?.lastError ?? 'none'}, Decision=${d.recoveryStats?.decision ?? 'pending'}`,
        '',
        'Current Context:',
        `Selected Disk: ${diskSummary}`,
        `Disk Identity: ${d.diskContext.identityFingerprint ? `confidence=${d.diskContext.identityConfidence ?? 'unknown'}, fields=${d.diskContext.identityFields.join(', ') || 'none'}, fingerprint=${d.diskContext.identityFingerprint}` : 'No disk identity captured'}`,
        `Last Task:     ${d.lastTaskKind || 'No tasks yet'} (${d.lastTaskStatus ?? 'n/a'})`,
        `Last Error:    ${d.lastError ?? 'None'}`,
        `Failure Note:  ${d.lastFailure?.message ?? 'None'}`,
        '',
        'Recent Logs:',
        ...(d.recentLogs.length > 0
          ? d.recentLogs.map((entry) => `- ${entry.at} [${entry.level}] ${entry.ctx}: ${entry.message}`)
          : ['- No recent WARN/ERROR logs captured']),
      ];

      if (extraContext) {
        lines.push('');
        lines.push('Extra Context:');
        try {
          if (typeof extraContext === 'string' && extraContext.startsWith('{')) {
            lines.push(redactSensitiveText(JSON.stringify(JSON.parse(extraContext), null, 2)));
          } else {
            lines.push(redactSensitiveText(String(extraContext)));
          }
        } catch {
          lines.push(redactSensitiveText(String(extraContext)));
        }
      }

      const text = lines.join('\n');
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2500);
    } catch (e: any) {
      setError('Could not copy — check clipboard permissions.');
    }
  };

  return (
    <div className={`inline-flex flex-col gap-1 ${className ?? ''}`}>
      <button
        onClick={handleCopy}
        className="flex items-center justify-center gap-2 px-3 py-1.5 bg-white/5 border border-white/10 rounded-xl text-xs font-bold text-white/50 hover:text-white/80 hover:bg-white/8 transition-all cursor-pointer w-full"
      >
        {copied ? (
          <Check className="w-3.5 h-3.5 text-emerald-400" />
        ) : (
          <Clipboard className="w-3.5 h-3.5" />
        )}
        {copied ? 'Copied' : 'Copy diagnostics'}
      </button>
      {error && (
        <span className="text-[10px] text-red-400/70">{error}</span>
      )}
    </div>
  );
}
