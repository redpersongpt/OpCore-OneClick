import { api } from '../bridge/api';
import type { AppVersionInfo, BuildResult, CompatibilityReport, HardwareProfile } from '../bridge/types';
import { macosLabel } from './macos';

export interface DiagnosticsInput {
  info: AppVersionInfo | null;
  profile: HardwareProfile | null;
  report: CompatibilityReport | null;
  result: BuildResult | null;
}

/** Plain-text snapshot attached to bug reports (redacted later by `buildIssueUrl`). */
export async function collectDiagnostics(input: DiagnosticsInput, logLines = 40): Promise<string> {
  const [session, tail] = await Promise.all([
    api.logGetSessionId().catch(() => 'unknown'),
    api.logGetTail(logLines).catch(() => ''),
  ]);
  const { info, profile, report, result } = input;
  const lines = [
    `OpCore-OneClick ${info ? `v${info.version} (OpenCore ${info.opencoreVersion}, ${info.hostOs}/${info.arch})` : ''}`,
    `Session: ${session}`,
    profile
      ? `CPU: ${profile.cpu.name} [${profile.cpu.platform}] ${profile.cpu.cores}C · ${profile.formFactor} · source=${profile.source}`
      : 'CPU: (no profile)',
    profile ? `GPU: ${profile.gpus.map((g) => `${g.name} [${g.family}${g.disabled ? ', disabled' : ''}]`).join('; ') || '-'}` : '',
    profile?.chipset ? `Chipset: ${profile.chipset}` : '',
    report ? `Compatibility: ${report.level}${report.target ? ` for ${macosLabel(report.target)}` : ''}` : '',
    result
      ? `Build: ${macosLabel(result.target)} · ${result.plan.smbios.model} · OpenCore ${result.opencoreVersion} · valid=${result.validation.valid}`
      : '',
    result ? `Kexts: ${result.kexts.map((k) => `${k.name}(${k.status})`).join(', ')}` : '',
    ...tail
      .split('\n')
      .filter((l) => /WARN|ERROR/i.test(l))
      .slice(-15),
  ];
  return lines.filter((l) => l.trim() !== '').join('\n');
}
