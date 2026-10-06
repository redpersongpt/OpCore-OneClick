import { listen } from '@tauri-apps/api/event';
import type { FlashProgress, MacOsVersion, RecoveryProgress, TaskStatus, TaskUpdate } from './types';

export const EVENTS = {
  taskUpdate: 'task:update',
  flashProgress: 'flash:progress',
  recoveryProgress: 'recovery:progress',
} as const;

const TASK_STATUSES: readonly TaskStatus[] = ['running', 'completed', 'failed', 'cancelled'];
const MACOS_VERSIONS: readonly MacOsVersion[] = ['10.13', '10.14', '10.15', '11', '12', '13', '14', '15', '26'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function str(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/** Clamp a 0..1 fraction; anything that is not a finite number becomes null. */
export function clampFraction(value: unknown): number | null {
  const n = finiteNumber(value);
  if (n === null) return null;
  return Math.min(1, Math.max(0, n));
}

export function normalizeTaskUpdate(raw: unknown): TaskUpdate | null {
  if (!isRecord(raw)) return null;
  const taskId = str(raw.taskId);
  const kind = str(raw.kind);
  const status = str(raw.status) as TaskStatus | null;
  if (!taskId || !kind || !status || !TASK_STATUSES.includes(status)) return null;
  return {
    taskId,
    kind,
    status,
    progress: clampFraction(raw.progress),
    message: str(raw.message),
    detail: raw.detail ?? null,
  };
}

export function normalizeFlashProgress(raw: unknown): FlashProgress | null {
  if (!isRecord(raw)) return null;
  const taskId = str(raw.taskId);
  const phase = str(raw.phase);
  if (!taskId || !phase) return null;
  return {
    taskId,
    phase,
    progress: clampFraction(raw.progress) ?? 0,
    message: str(raw.message) ?? '',
    error: str(raw.error),
  };
}

export function normalizeRecoveryProgress(raw: unknown): RecoveryProgress | null {
  if (!isRecord(raw)) return null;
  const taskId = str(raw.taskId);
  const phase = str(raw.phase);
  const version = str(raw.version) as MacOsVersion | null;
  if (!taskId || !phase || !version || !MACOS_VERSIONS.includes(version)) return null;
  const downloaded = Math.max(0, finiteNumber(raw.downloaded) ?? 0);
  const totalRaw = finiteNumber(raw.total);
  const total = totalRaw !== null && totalRaw > 0 ? totalRaw : null;
  let progress = clampFraction(raw.progress);
  if (progress === null && total !== null) progress = Math.min(1, downloaded / total);
  return {
    taskId,
    version,
    phase,
    downloaded: downloaded,
    total: total === null ? null : total,
    progress,
    error: str(raw.error),
  };
}

/**
 * Subscribe to a backend event. `listen` resolves asynchronously, so the
 * returned disposer also handles the case where the component unmounted
 * before the subscription was established (React StrictMode double mount).
 */
export function subscribe(event: string, handler: (payload: unknown) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;

  try {
    listen<unknown>(event, (e) => {
      if (!disposed) handler(e.payload);
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Not running inside the desktop runtime; nothing to listen to.
      });
  } catch {
    // `listen` can throw synchronously when the IPC bridge is missing.
  }

  return () => {
    disposed = true;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  };
}
