import { redactSensitiveText } from './diagnosticRedaction.js';
import { structureError } from './structuredErrors.js';

export interface FailureRecoveryPayload {
  message: string;
  rawMessage?: string;
  explanation?: string;
  decisionSummary?: string;
  suggestion?: string;
  suggestionRecommended?: boolean;
  suggestionReason?: string;
  suggestionOutcome?: string;
  suggestionConfidenceReason?: string;
  suggestionRisk?: string;
  alternatives?: Array<{
    text: string;
    confidence: string;
    confidenceReason: string;
    group: string;
    reason: string;
    expectedOutcome: string;
    risk?: string;
    recommended: boolean;
  }>;
  category?: string;
  contextNote?: string;
  code?: string;
  severity?: string;
  validationCode?: string;
  validationComponent?: string;
  validationPath?: string;
  validationSource?: string;
  validationDetail?: string;
  targetStep?: string;
}

export interface FailureRecoveryViewModel {
  title: string;
  whatFailed: string;
  likelyCause: string;
  nextActions: string[];
  technicalDetails: Array<{ label: string; value: string; mono?: boolean }>;
  targetStep: string | null;
  severity: string | null;
  retrySuggested: boolean;
}

export function parseFailureRecoveryPayload(input: string | FailureRecoveryPayload | null): FailureRecoveryPayload | null {
  if (!input) return null;
  if (typeof input === 'object') return input;

  try {
    if (input.trim().startsWith('{')) {
      return JSON.parse(input) as FailureRecoveryPayload;
    }
  } catch {
    // fall through to plain-text message
  }

  return { message: redactSensitiveText(input) };
}

function compact(values: Array<string | null | undefined>): string[] {
  return values
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));
}

function shouldPreferStructuredTitle(title: string | null | undefined): boolean {
  const normalized = title?.trim().toLowerCase();
  return normalized === 'an error occurred' || normalized === 'something went wrong' || normalized === '';
}

export function buildFailureRecoveryViewModel(input: string | FailureRecoveryPayload | null): FailureRecoveryViewModel | null {
  const payload = parseFailureRecoveryPayload(input);
  if (!payload) return null;
  const structured = structureError(payload.rawMessage ?? payload.message ?? '');

  const nextActions = compact([
    payload.suggestion,
    ...((payload.alternatives ?? [])
      .filter((option) => option.recommended || option.group !== 'learn_more')
      .slice(0, 2)
      .map((option) => option.text)),
    !payload.suggestion ? structured.nextStep : null,
  ]);

  const technicalDetails = compact([
    payload.rawMessage ? `Error::${payload.rawMessage}` : null,
    payload.code ? `Code::${payload.code}` : null,
    payload.validationCode ? `Validation code::${payload.validationCode}` : null,
    payload.validationComponent ? `Component::${payload.validationComponent}` : null,
    payload.validationPath ? `Path::${payload.validationPath}` : null,
    payload.validationSource ? `Source::${payload.validationSource}` : null,
    payload.validationDetail ? `Detail::${payload.validationDetail}` : null,
  ]).map((entry) => {
    const [label, value] = entry.split('::');
    return {
      label,
      value: redactSensitiveText(value),
      mono: label === 'Code' || label === 'Validation code' || label === 'Path',
    };
  });

  return {
    title: redactSensitiveText(
      shouldPreferStructuredTitle(payload.message)
        ? structured.title
        : (payload.message || structured.title || 'Something went wrong'),
    ),
    whatFailed: redactSensitiveText(payload.explanation || structured.what || payload.message || 'An unexpected error interrupted the current step.'),
    likelyCause: redactSensitiveText(
      payload.decisionSummary
        || payload.suggestionReason
        || payload.contextNote
        || payload.validationDetail
        || structured.what
        || 'The step could not finish. This is usually caused by a missing file, a network issue, or incomplete hardware data.',
    ),
    nextActions: nextActions.length > 0
      ? nextActions.map((action) => redactSensitiveText(action))
      : [redactSensitiveText(structured.nextStep)],
    technicalDetails,
    targetStep: payload.targetStep ?? null,
    severity: payload.severity ?? null,
    retrySuggested: nextActions.length > 0,
  };
}
