import type { ILogger } from './logger.js';

// ─── Task types ───────────────────────────────────────────────────────────────

export type TaskKind =
  | 'efi-build' | 'kext-fetch' | 'recovery-download'
  | 'usb-flash' | 'partition-prep';

export type TaskStatus =
  | 'pending' | 'running' | 'paused' | 'cancelled' | 'failed' | 'complete';

// Task progress payloads — renderer uses these for display
export interface KextFetchProgress {
  kind: 'kext-fetch'; kextName: string; version: string; index: number; total: number; source?: 'github' | 'embedded' | 'failed';
}
export interface RecoveryDownloadProgress {
  kind: 'recovery-download'; percent: number; status: string;
  bytesDownloaded: number; dmgDest: string; clDest: string;
  sourceId?: string;
  trustLevel?: string;
}
export interface UsbFlashProgress {
  kind: 'usb-flash'; phase: string; detail: string;
}
export interface EfiBuildProgress {
  kind: 'efi-build'; phase: string; detail: string;
}
export interface PartitionPrepProgress {
  kind: 'partition-prep'; phase: string; detail: string;
}
export type TaskProgress =
  | KextFetchProgress | RecoveryDownloadProgress | UsbFlashProgress
  | EfiBuildProgress | PartitionPrepProgress;

export interface TaskState {
  taskId: string;
  kind: TaskKind;
  status: TaskStatus;
  progress: TaskProgress | null;
  error: string | null;
  startedAt: number;
  endedAt: number | null;
  lastUpdateAt: number;
}

export interface TaskUpdatePayload { task: TaskState; }

// ─── Op token (cancellation) ──────────────────────────────────────────────────

export interface OpToken {
  readonly taskId: string;
  readonly aborted: boolean;
  abort(): void;
  check(): void; // throws 'Task <id> was cancelled' if aborted
  registerProcess(child: any): void;
}

// ─── Registry ─────────────────────────────────────────────────────────────────

export interface ITaskRegistry {
  create(kind: TaskKind): OpToken;
  updateProgress(taskId: string, progress: TaskProgress): void;
  complete(taskId: string): void;
  fail(taskId: string, error: string): void;
  cancel(taskId: string): boolean;
  get(taskId: string): TaskState | undefined;
  list(): TaskState[];
}

// ─── Implementation ───────────────────────────────────────────────────────────

interface TokenState {
  aborted: boolean;
  processes: Set<any>;
}

interface ThrottleState {
  lastEmit: number;
  lastPct: number;
}

function isTerminalStatus(status: TaskStatus): boolean {
  return status === 'complete' || status === 'failed' || status === 'cancelled';
}

export function createTaskRegistry(
  pushFn: (p: TaskUpdatePayload) => void,
  logger: ILogger
): ITaskRegistry {
  const tasks = new Map<string, TaskState>();
  const tokens = new Map<string, TokenState>();
  const throttleMap = new Map<string, ThrottleState>();

  function create(kind: TaskKind): OpToken {
    const taskId = `${kind}-${Date.now()}`;
    const now = Date.now();
    const state: TaskState = {
      taskId,
      kind,
      status: 'running',
      progress: null,
      error: null,
      startedAt: now,
      endedAt: null,
      lastUpdateAt: now,
    };
    const tokenState: TokenState = { aborted: false, processes: new Set() };

    tasks.set(taskId, state);
    tokens.set(taskId, tokenState);

    pushFn({ task: { ...state } });
    logger.timeline('task_start', taskId, { kind });

    const token: OpToken = {
      get taskId() { return taskId; },
      get aborted() { return tokenState.aborted; },
      abort() { 
        tokenState.aborted = true; 
        tokenState.processes.forEach(p => {
          try { p.kill('SIGKILL'); } catch (_) {}
        });
        tokenState.processes.clear();
      },
      check() {
        if (tokenState.aborted) throw new Error(`Task ${taskId} was cancelled`);
      },
      registerProcess(child: any) {
        if (tokenState.aborted) {
          try { child.kill('SIGKILL'); } catch (_) {}
          return;
        }
        tokenState.processes.add(child);
        child.on('exit', () => tokenState.processes.delete(child));
      }
    };

    return token;
  }

  function abortTask(taskId: string): TaskState | undefined {
    const tokenState = tokens.get(taskId);
    if (tokenState) {
      tokenState.aborted = true;
      tokenState.processes.forEach(p => {
        try { p.kill('SIGKILL'); } catch (_) {}
      });
      tokenState.processes.clear();
    }
    tokens.delete(taskId);
    throttleMap.delete(taskId);
    return tasks.get(taskId);
  }

  function updateProgress(taskId: string, progress: TaskProgress): void {
    const state = tasks.get(taskId);
    if (!state) return;

    if (isTerminalStatus(state.status)) {
      logger.warn('task_manager', `Ignoring late progress for terminal task ${taskId}`, {
        status: state.status,
        kind: state.kind,
        progressKind: progress.kind,
      });
      return;
    }

    // Per-kind throttle for recovery-download
    if (progress.kind === 'recovery-download') {
      const pct = progress.percent;
      const force = pct >= 100 || pct <= 8;
      if (!force) {
        const ts = throttleMap.get(taskId) ?? { lastEmit: 0, lastPct: -1 };
        const now = Date.now();
        if (now - ts.lastEmit <= 250 && Math.abs(pct - ts.lastPct) <= 1) return;
        throttleMap.set(taskId, { lastEmit: now, lastPct: pct });
      } else {
        throttleMap.set(taskId, { lastEmit: Date.now(), lastPct: pct });
      }
    }

    state.progress = progress;
    state.lastUpdateAt = Date.now();
    pushFn({ task: { ...state } });
    // No timeline entry for progress — too verbose
  }

  function complete(taskId: string): void {
    const state = tasks.get(taskId);
    if (!state) return;
    if (isTerminalStatus(state.status)) return;
    state.status = 'complete';
    state.endedAt = Date.now();
    state.lastUpdateAt = Date.now();
    pushFn({ task: { ...state } });
    logger.timeline('task_complete', taskId, {});
    abortTask(taskId);
  }

  function fail(taskId: string, error: string): void {
    const state = tasks.get(taskId);
    if (!state) return;
    if (isTerminalStatus(state.status)) return;
    state.status = 'failed';
    state.error = error;
    state.endedAt = Date.now();
    state.lastUpdateAt = Date.now();
    pushFn({ task: { ...state } });
    logger.timeline('task_failed', taskId, { error });
    abortTask(taskId);
  }

  function cancel(taskId: string): boolean {
    const state = tasks.get(taskId);
    if (!state || isTerminalStatus(state.status)) return false;
    abortTask(taskId);
    state.status = 'cancelled';
    state.endedAt = Date.now();
    state.lastUpdateAt = Date.now();
    pushFn({ task: { ...state } });
    logger.timeline('task_cancelled', taskId, {});
    return true;
  }

  // Watchdog timer to detect stalled tasks (no updates for > 60s)
  const watchdog = setInterval(() => {
    const now = Date.now();
    for (const state of tasks.values()) {
      if (state.status === 'running' && (now - state.lastUpdateAt) > 60_000) {
        logger.warn('task_manager', `Task ${state.taskId} (${state.kind}) stalled — no update for 60s`, {
          lastUpdate: new Date(state.lastUpdateAt).toISOString()
        });
        logger.timeline('watchdog_trigger', state.taskId, { kind: state.kind, lastUpdate: state.lastUpdateAt });
        abortTask(state.taskId);
        state.status = 'failed';
        state.error = `Operation stalled: no progress received for 60 seconds. Please check your connection or try again.`;
        state.endedAt = Date.now();
        state.lastUpdateAt = Date.now();
        pushFn({ task: { ...state } });
        logger.timeline('task_failed', state.taskId, { error: state.error });
      }
    }
  }, 10_000);

  function get(taskId: string): TaskState | undefined {
    return tasks.get(taskId);
  }

  function list(): TaskState[] {
    return Array.from(tasks.values());
  }

  return { create, updateProgress, complete, fail, cancel, get, list };
}
