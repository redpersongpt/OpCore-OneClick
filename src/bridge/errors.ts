/**
 * Error contract of every backend command (`src-tauri/src/error.rs`).
 * Commands reject with this object; anything else (IPC failures, panics,
 * thrown JS errors) is normalised into the same shape by `toAppError`.
 */
export type ErrorSeverity = 'error' | 'warning' | 'info';

export interface AppError {
  code: string;
  message: string;
  severity: ErrorSeverity;
  recoverable: boolean;
  suggestion: string | null;
  context: unknown;
}

const SEVERITIES: readonly ErrorSeverity[] = ['error', 'warning', 'info'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function fromRecord(record: Record<string, unknown>, fallbackCode: string): AppError | null {
  const message = typeof record.message === 'string' ? record.message.trim() : '';
  const code = typeof record.code === 'string' && record.code.trim() ? record.code.trim() : '';
  if (!message && !code) return null;
  const severity = SEVERITIES.includes(record.severity as ErrorSeverity)
    ? (record.severity as ErrorSeverity)
    : 'error';
  return {
    code: code || fallbackCode,
    message: message || code,
    severity,
    recoverable: record.recoverable === true,
    suggestion:
      typeof record.suggestion === 'string' && record.suggestion.trim() ? record.suggestion.trim() : null,
    context: record.context ?? null,
  };
}

function tryParseJson(text: string): unknown {
  const trimmed = text.trim();
  if (!trimmed.startsWith('{')) return undefined;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return undefined;
  }
}

/** Normalise any rejection value into an `AppError`. Idempotent. */
export function toAppError(err: unknown, fallbackCode = 'UNKNOWN_ERROR'): AppError {
  if (isRecord(err) && !(err instanceof Error)) {
    const parsed = fromRecord(err, fallbackCode);
    if (parsed) return parsed;
  }

  if (err instanceof Error) {
    const ipcMissing = /__TAURI_INTERNALS__|reading 'invoke'|transformCallback/.test(err.message);
    return {
      code: ipcMissing ? 'IPC_UNAVAILABLE' : fallbackCode,
      message: ipcMissing ? 'The desktop backend is not available in this window.' : err.message || err.name,
      severity: 'error',
      recoverable: !ipcMissing,
      suggestion: ipcMissing ? 'Start the app with the desktop runtime instead of a plain browser.' : null,
      context: null,
    };
  }

  if (typeof err === 'string') {
    const json = tryParseJson(err);
    if (isRecord(json)) {
      const parsed = fromRecord(json, fallbackCode);
      if (parsed) return parsed;
    }
    return {
      code: fallbackCode,
      message: err.trim() || 'Unknown error',
      severity: 'error',
      recoverable: false,
      suggestion: null,
      context: null,
    };
  }

  let message = 'Unknown error';
  if (err !== undefined && err !== null) {
    try {
      message = JSON.stringify(err);
    } catch {
      message = String(err);
    }
  }
  return { code: fallbackCode, message, severity: 'error', recoverable: false, suggestion: null, context: null };
}

/** True when the error says the user (or a task_cancel call) stopped the operation. */
export function isCancellation(err: AppError): boolean {
  return /CANCEL/i.test(err.code);
}

/** One-line text for logs and bug reports. */
export function describeError(err: AppError): string {
  return err.suggestion ? `[${err.code}] ${err.message} (${err.suggestion})` : `[${err.code}] ${err.message}`;
}
