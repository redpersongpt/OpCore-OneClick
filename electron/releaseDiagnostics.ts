import crypto from 'node:crypto';
import type { CompatibilityReport } from './compatibility.js';
import type { ValidationResult } from './configValidator.js';
import type { DiskInfo } from './diskOps.js';
import type { DiskIdentityFingerprint } from './flashSafety.js';
import type { LogEntry, TimelineEntry } from './logger.js';
import {
  redactSensitiveText,
  sanitizeTelemetryValue,
  summarizeDeviceIdentifier,
} from '../src/lib/diagnosticRedaction.js';

export type IssueReportTrigger =
  | 'manual_report'
  | 'startup_failure'
  | 'efi_build_failure'
  | 'efi_validation_failure'
  | 'unexpected_runtime_error'
  | 'recovery_failure'
  | 'disk_read_failure'
  | 'simulation_failure'
  | 'ipc_failure';

export interface ReleaseFailureContext {
  trigger: IssueReportTrigger;
  message: string;
  detail?: string | null;
  channel?: string | null;
  code?: string | null;
  occurredAt: string;
}

export interface PublicDiagnosticsSnapshot {
  version: string;
  platform: string;
  arch: string;
  compatMode: string;
  timestamp: string;
  sessionFingerprint: string;
  trigger: IssueReportTrigger;
  lastTaskKind: string | null;
  lastTaskStatus: string | null;
  lastError: string | null;
  hardware: string;
  confidence: string;
  firmware: string;
  compatibilityState: {
    level: CompatibilityReport['level'];
    recommendedVersion: string;
    explanation: string;
    warnings: string[];
    errors: string[];
  } | null;
  failedKexts: string[];
  kextSources: Record<string, 'github' | 'embedded' | 'failed'>;
  diskContext: {
    selectedDevice: string | null;
    partitionTable: DiskInfo['partitionTable'] | null;
    isSystemDisk: boolean | null;
    identityConfidence: string | null;
    identityFields: string[];
    identityFingerprint: string | null;
  };
  validationSummary: {
    overall: ValidationResult['overall'];
    issueCount: number;
    issues: Array<{
      code: string;
      severity: 'warning' | 'blocked';
      component: string;
      message: string;
      expectedPath: string;
    }>;
    firstFailureTrace: {
      code: string;
      component: string;
      expectedPath: string;
      source: string;
      detail: string;
    } | null;
  } | null;
  recoveryStats: {
    attempts: number;
    lastHttpCode: number | null;
    lastError: string | null;
    decision: string | null;
    source: string;
  };
  recentLogs: Array<{
    at: string;
    level: string;
    ctx: string;
    message: string;
  }>;
  lastFailure: ReleaseFailureContext | null;
}

export interface IssueReportDraft {
  title: string;
  body: string;
  trigger: IssueReportTrigger;
}

export interface CreateDiagnosticsSnapshotInput {
  version: string;
  platform: string;
  arch: string;
  compatMode: string;
  timestamp: string;
  sessionId: string;
  hardware: string;
  confidence: string;
  firmware: string;
  lastTaskKind: string | null;
  lastTaskStatus: string | null;
  lastError: string | null;
  failedKexts: string[];
  kextSources: Record<string, 'github' | 'embedded' | 'failed'>;
  selectedDisk: DiskInfo | null;
  diskIdentity: DiskIdentityFingerprint | null;
  compatibilityReport: CompatibilityReport | null;
  validationResult: ValidationResult | null;
  recoveryStats: {
    attempts: number;
    lastHttpCode: number | null;
    lastError: string | null;
    decision: string | null;
    source: string;
  };
  recentLogs: LogEntry[];
  lastFailure: ReleaseFailureContext | null;
}

function shortHash(value: string): string {
  return crypto.createHash('sha256').update(value).digest('hex').slice(0, 12);
}

function sanitizeList(values: string[], limit = 5): string[] {
  return values.slice(0, limit).map((value) => redactSensitiveText(value));
}

export function inferIssueReportTrigger(input: {
  lastFailure: ReleaseFailureContext | null;
  validationResult: ValidationResult | null;
  lastTaskKind: string | null;
  lastTaskStatus: string | null;
}): IssueReportTrigger {
  if (input.lastFailure?.trigger) return input.lastFailure.trigger;

  if (input.validationResult?.overall === 'blocked') {
    return 'efi_validation_failure';
  }

  if (input.lastTaskStatus === 'failed') {
    if (input.lastTaskKind === 'efi-build') return 'efi_build_failure';
    if (input.lastTaskKind === 'recovery-download') return 'recovery_failure';
  }

  return 'manual_report';
}

export function createDiagnosticsSnapshot(input: CreateDiagnosticsSnapshotInput): PublicDiagnosticsSnapshot {
  const trigger = inferIssueReportTrigger({
    lastFailure: input.lastFailure,
    validationResult: input.validationResult,
    lastTaskKind: input.lastTaskKind,
    lastTaskStatus: input.lastTaskStatus,
  });

  const sanitizedLogs = input.recentLogs
    .slice(-10)
    .map((entry) => sanitizeTelemetryValue(entry) as LogEntry)
    .map((entry) => ({
      at: entry.t,
      level: entry.level,
      ctx: entry.ctx,
      message: redactSensitiveText(entry.msg),
    }));

  return {
    version: input.version,
    platform: input.platform,
    arch: input.arch,
    compatMode: input.compatMode,
    timestamp: input.timestamp,
    sessionFingerprint: shortHash(input.sessionId),
    trigger,
    lastTaskKind: input.lastTaskKind,
    lastTaskStatus: input.lastTaskStatus,
    lastError: input.lastError ? redactSensitiveText(input.lastError) : null,
    hardware: redactSensitiveText(input.hardware),
    confidence: redactSensitiveText(input.confidence),
    firmware: redactSensitiveText(input.firmware),
    compatibilityState: input.compatibilityReport
      ? {
          level: input.compatibilityReport.level,
          recommendedVersion: input.compatibilityReport.recommendedVersion,
          explanation: redactSensitiveText(input.compatibilityReport.explanation),
          warnings: sanitizeList(input.compatibilityReport.warnings),
          errors: sanitizeList(input.compatibilityReport.errors),
        }
      : null,
    failedKexts: sanitizeList(input.failedKexts, 10),
    kextSources: input.kextSources,
    diskContext: {
      selectedDevice: summarizeDeviceIdentifier(input.selectedDisk?.device ?? null),
      partitionTable: input.selectedDisk?.partitionTable ?? null,
      isSystemDisk: input.selectedDisk?.isSystemDisk ?? null,
      identityConfidence: input.selectedDisk?.identityConfidence ?? null,
      identityFields: input.selectedDisk?.identityFieldsUsed ?? [],
      identityFingerprint: input.diskIdentity ? shortHash(JSON.stringify(input.diskIdentity)) : null,
    },
    validationSummary: input.validationResult
      ? {
          overall: input.validationResult.overall,
          issueCount: input.validationResult.issues.length,
          issues: input.validationResult.issues.slice(0, 8).map((issue) => ({
            code: issue.code,
            severity: issue.severity,
            component: redactSensitiveText(issue.component),
            message: redactSensitiveText(issue.message),
            expectedPath: redactSensitiveText(issue.expectedPath),
          })),
          firstFailureTrace: input.validationResult.firstFailureTrace
            ? {
                code: input.validationResult.firstFailureTrace.code,
                component: redactSensitiveText(input.validationResult.firstFailureTrace.component),
                expectedPath: redactSensitiveText(input.validationResult.firstFailureTrace.expectedPath),
                source: input.validationResult.firstFailureTrace.source,
                detail: redactSensitiveText(input.validationResult.firstFailureTrace.detail),
              }
            : null,
        }
      : null,
    recoveryStats: {
      attempts: input.recoveryStats.attempts,
      lastHttpCode: input.recoveryStats.lastHttpCode,
      lastError: input.recoveryStats.lastError ? redactSensitiveText(input.recoveryStats.lastError) : null,
      decision: input.recoveryStats.decision,
      source: redactSensitiveText(input.recoveryStats.source),
    },
    recentLogs: sanitizedLogs,
    lastFailure: input.lastFailure
      ? {
          ...input.lastFailure,
          message: redactSensitiveText(input.lastFailure.message),
          detail: input.lastFailure.detail ? redactSensitiveText(input.lastFailure.detail) : null,
        }
      : null,
  };
}

function triggerLabel(trigger: IssueReportTrigger): string {
  switch (trigger) {
    case 'startup_failure':
      return 'STARTUP_FAILURE';
    case 'efi_build_failure':
      return 'EFI_BUILD_FAILURE';
    case 'efi_validation_failure':
      return 'EFI_VALIDATION_FAILURE';
    case 'unexpected_runtime_error':
      return 'RUNTIME_ERROR';
    case 'recovery_failure':
      return 'RECOVERY_FAILURE';
    case 'disk_read_failure':
      return 'DISK_READ_FAILURE';
    case 'simulation_failure':
      return 'SIMULATION_FAILURE';
    case 'ipc_failure':
      return 'IPC_FAILURE';
    case 'manual_report':
    default:
      return 'MANUAL_REPORT';
  }
}

export function buildIssueReportDraft(snapshot: PublicDiagnosticsSnapshot, extraContext?: string | null): IssueReportDraft {
  const triggerCode = triggerLabel(snapshot.trigger);
  const summary = snapshot.lastFailure?.message
    ?? snapshot.lastError
    ?? snapshot.validationSummary?.issues[0]?.message
    ?? 'Unexpected application error';

  const title = `[BUG][${triggerCode}] ${summary.slice(0, 72)}`;

  const body = [
    `> Session fingerprint: \`${snapshot.sessionFingerprint}\``,
    `> Trigger: \`${triggerCode}\``,
    `> App version: ${snapshot.version} (${snapshot.platform}/${snapshot.arch})`,
    '',
    '## Failure Context',
    snapshot.lastFailure
      ? [
          `- Time: ${snapshot.lastFailure.occurredAt}`,
          `- Message: ${snapshot.lastFailure.message}`,
          snapshot.lastFailure.detail ? `- Detail: ${snapshot.lastFailure.detail}` : null,
          snapshot.lastFailure.channel ? `- Channel: ${snapshot.lastFailure.channel}` : null,
          snapshot.lastFailure.code ? `- Code: ${snapshot.lastFailure.code}` : null,
        ].filter(Boolean).join('\n')
      : `- Last error: ${snapshot.lastError ?? 'None captured'}`,
    '',
    '## Compatibility State',
    snapshot.compatibilityState
      ? [
          `- Level: ${snapshot.compatibilityState.level}`,
          `- Recommended version: ${snapshot.compatibilityState.recommendedVersion || 'None'}`,
          `- Explanation: ${snapshot.compatibilityState.explanation}`,
          `- Errors: ${snapshot.compatibilityState.errors.length > 0 ? snapshot.compatibilityState.errors.join(' | ') : 'None'}`,
          `- Warnings: ${snapshot.compatibilityState.warnings.length > 0 ? snapshot.compatibilityState.warnings.join(' | ') : 'None'}`,
        ].join('\n')
      : 'Not available',
    '',
    '## Validation State',
    snapshot.validationSummary
      ? [
          `- Overall: ${snapshot.validationSummary.overall}`,
          `- Issue count: ${snapshot.validationSummary.issueCount}`,
          snapshot.validationSummary.firstFailureTrace
            ? `- First failure: ${snapshot.validationSummary.firstFailureTrace.code} ${snapshot.validationSummary.firstFailureTrace.component} @ ${snapshot.validationSummary.firstFailureTrace.expectedPath} [${snapshot.validationSummary.firstFailureTrace.source}] — ${snapshot.validationSummary.firstFailureTrace.detail}`
            : '- First failure: None',
          ...snapshot.validationSummary.issues.map((issue) => `- ${issue.severity.toUpperCase()} ${issue.code}: ${issue.message} (${issue.component} @ ${issue.expectedPath})`),
        ].join('\n')
      : 'No validation result captured',
    '',
    '## Runtime Context',
    [
      `- Hardware: ${snapshot.hardware}`,
      `- Detection confidence: ${snapshot.confidence}`,
      `- Firmware: ${snapshot.firmware}`,
      `- Last task: ${snapshot.lastTaskKind ?? 'none'} (${snapshot.lastTaskStatus ?? 'n/a'})`,
      `- Selected disk: ${snapshot.diskContext.selectedDevice ?? 'none'} | table=${snapshot.diskContext.partitionTable ?? 'unknown'} | system=${snapshot.diskContext.isSystemDisk ?? 'unknown'}`,
      `- Disk identity confidence: ${snapshot.diskContext.identityConfidence ?? 'unknown'}`,
      `- Disk identity fields: ${snapshot.diskContext.identityFields.join(', ') || 'none'}`,
      `- Disk identity fingerprint: ${snapshot.diskContext.identityFingerprint ?? 'none'}`,
      `- Failed kexts: ${snapshot.failedKexts.length > 0 ? snapshot.failedKexts.join(', ') : 'None'}`,
    ].join('\n'),
    '',
    '## Recovery State',
    [
      `- Attempts: ${snapshot.recoveryStats.attempts}`,
      `- Last HTTP code: ${snapshot.recoveryStats.lastHttpCode ?? 'n/a'}`,
      `- Last error: ${snapshot.recoveryStats.lastError ?? 'none'}`,
      `- Decision: ${snapshot.recoveryStats.decision ?? 'pending'}`,
      `- Source: ${snapshot.recoveryStats.source}`,
    ].join('\n'),
    '',
    '## Recent Logs (sanitized)',
    '```text',
    snapshot.recentLogs.length > 0
      ? snapshot.recentLogs.map((entry) => `${entry.at} [${entry.level}] ${entry.ctx}: ${entry.message}`).join('\n')
      : 'No recent WARN/ERROR logs captured.',
    '```',
    ...(extraContext?.trim()
      ? ['', '## Extra Context', redactSensitiveText(extraContext)]
      : []),
  ].join('\n');

  return { title, body, trigger: snapshot.trigger };
}

export function buildSavedSupportLog(
  snapshot: PublicDiagnosticsSnapshot,
  opsTail: TimelineEntry[],
  extraContext?: string | null,
): string {
  const diagnostics = buildIssueReportDraft(snapshot).body;
  const sanitizedOps = opsTail
    .slice(-120)
    .map((entry) => sanitizeTelemetryValue(entry) as TimelineEntry)
    .map((entry) => {
      const detail = Object.entries(entry.detail ?? {})
        .map(([key, value]) => `${key}=${redactSensitiveText(String(value))}`)
        .join(', ');
      return `${entry.t} [${entry.kind}]${entry.taskId ? ` task=${entry.taskId}` : ''}${detail ? ` ${detail}` : ''}`;
    });

  const sections = [
    'macOS One-Click Support Log',
    '===========================',
    `Generated: ${snapshot.timestamp}`,
    `Session: ${snapshot.sessionFingerprint}`,
    '',
    diagnostics,
    '',
    '## Timeline (sanitized)',
    sanitizedOps.length > 0 ? sanitizedOps.join('\n') : 'No recent timeline events captured.',
  ];

  if (extraContext?.trim()) {
    sections.push('', '## Extra Context', redactSensitiveText(extraContext));
  }

  return sections.join('\n');
}

export async function openIssueReportUrl(
  url: string,
  openExternal: (targetUrl: string) => Promise<void>,
  timeoutMs = 2500,
): Promise<boolean> {
  try {
    const didOpen = await Promise.race([
      openExternal(url).then(() => true),
      new Promise<boolean>((resolve) => setTimeout(() => resolve(false), timeoutMs)),
    ]);
    return didOpen;
  } catch {
    return false;
  }
}
