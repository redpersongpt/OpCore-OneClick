import fs from 'fs';
import path from 'path';
import crypto from 'crypto';
import { sanitizeTelemetryValue } from '../src/lib/diagnosticRedaction.js';

export type LogLevel = 'DEBUG' | 'INFO' | 'WARN' | 'ERROR' | 'FATAL';

export type TimelineEventKind =
  | 'task_start' | 'task_progress' | 'task_complete' | 'task_failed' | 'task_cancelled'
  | 'phase_change' | 'safety_check' | 'app_start' | 'app_quit' | 'crash_recovery'
  | 'recovery_attempt' | 'recovery_failure' | 'efi_validation_fail' | 'flash_start' | 'flash_fail' | 'watchdog_trigger';

export interface LogEntry {
  t: string; level: LogLevel; ctx: string; msg: string;
  sessionId: string; taskId?: string;
  [key: string]: unknown;
}

export interface TimelineEntry {
  t: string; sessionId: string; taskId?: string;
  kind: TimelineEventKind; detail: Record<string, unknown>;
}

export interface LoggerConfig {
  logFile: string; opsFile: string; minLevel: LogLevel;
  maxFileSizeBytes: number; rotationCount: number;
  flushIntervalMs: number; crashSafeSync: boolean;
  isPackaged: boolean;
}

export interface ILogger {
  debug(ctx: string, msg: string, data?: Record<string, unknown>): void;
  info(ctx: string, msg: string, data?: Record<string, unknown>): void;
  warn(ctx: string, msg: string, data?: Record<string, unknown>): void;
  error(ctx: string, msg: string, data?: Record<string, unknown>): void;
  fatal(ctx: string, msg: string, data?: Record<string, unknown>): void;
  withTask(taskId: string): ILogger;
  timeline(kind: TimelineEventKind, taskId: string | undefined, detail: Record<string, unknown>): void;
  flush(): void;
  readTail(n: number): LogEntry[];
  readOpsTail(n: number): TimelineEntry[];
  readonly sessionId: string;
  readonly logPath: string;
}

const LEVEL_RANK: Record<LogLevel, number> = { DEBUG: 0, INFO: 1, WARN: 2, ERROR: 3, FATAL: 4 };

export function createLogger(config: LoggerConfig): ILogger {
  const sessionId = crypto.randomUUID();
  let buffer: string[] = [];
  let flushTimer: ReturnType<typeof setInterval> | null = null;
  let _taskId: string | undefined;

  // Ensure parent directories exist
  for (const f of [config.logFile, config.opsFile]) {
    try { fs.mkdirSync(path.dirname(f), { recursive: true }); } catch {}
  }

  // Crash recovery detection
  const lockFile = path.join(path.dirname(config.logFile), 'session.lock');
  try {
    if (fs.existsSync(lockFile)) {
      const prev = JSON.parse(fs.readFileSync(lockFile, 'utf-8'));
      const entry: TimelineEntry = {
        t: new Date().toISOString(), sessionId, kind: 'crash_recovery',
        detail: { previousSession: prev.sessionId, previousStart: prev.startedAt }
      };
      fs.appendFileSync(config.opsFile, JSON.stringify(entry) + '\n');
    }
  } catch {}

  // Write session lock
  try { fs.writeFileSync(lockFile, JSON.stringify({ sessionId, startedAt: new Date().toISOString() })); } catch {}

  function rotateIfNeeded(filePath: string): void {
    try {
      if (!fs.existsSync(filePath)) return;
      if (fs.statSync(filePath).size < config.maxFileSizeBytes) return;
      // Shift rotation files
      for (let i = config.rotationCount - 1; i >= 1; i--) {
        const older = `${filePath}.old.${i}`;
        const newer = `${filePath}.old.${i + 1}`;
        if (fs.existsSync(older)) try { fs.renameSync(older, newer); } catch {}
      }
      try { fs.renameSync(`${filePath}.old`, `${filePath}.old.1`); } catch {}
      try { fs.renameSync(filePath, `${filePath}.old`); } catch {}
    } catch {}
  }

  function writeEntry(entry: LogEntry): void {
    const sanitizedEntry = sanitizeTelemetryValue(entry) as LogEntry;
    const line = JSON.stringify(sanitizedEntry) + '\n';
    const isHighPriority = LEVEL_RANK[sanitizedEntry.level] >= LEVEL_RANK['WARN'];
    if (isHighPriority && config.crashSafeSync) {
      // Flush buffer first, then write synchronously
      if (buffer.length > 0) {
        try { rotateIfNeeded(config.logFile); fs.appendFileSync(config.logFile, buffer.join('')); } catch {}
        buffer = [];
      }
      try { fs.appendFileSync(config.logFile, line); } catch {}
    } else {
      buffer.push(line);
    }
    if (!config.isPackaged) {
      const pfx = `[${sanitizedEntry.level}][${sanitizedEntry.ctx}]`;
      if (sanitizedEntry.level === 'ERROR' || sanitizedEntry.level === 'FATAL') console.error(pfx, sanitizedEntry.msg, sanitizedEntry);
      else if (sanitizedEntry.level === 'WARN') console.warn(pfx, sanitizedEntry.msg, sanitizedEntry);
      else console.log(pfx, sanitizedEntry.msg, sanitizedEntry);
    }
  }

  function log(level: LogLevel, ctx: string, msg: string, data?: Record<string, unknown>): void {
    if (LEVEL_RANK[level] < LEVEL_RANK[config.minLevel]) return;
    const entry: LogEntry = {
      t: new Date().toISOString(), level, ctx, msg, sessionId,
      ...(_taskId ? { taskId: _taskId } : {}),
      ...(data ?? {}),
    };
    writeEntry(entry);
  }

  const flush = (): void => {
    if (buffer.length > 0) {
      try { rotateIfNeeded(config.logFile); fs.appendFileSync(config.logFile, buffer.join('')); } catch {}
      buffer = [];
    }
  };

  // Start write-behind timer
  flushTimer = setInterval(flush, config.flushIntervalMs);
  if (flushTimer.unref) flushTimer.unref(); // don't keep process alive

  const instance: ILogger = {
    get sessionId() { return sessionId; },
    get logPath() { return config.logFile; },
    debug: (ctx, msg, data) => log('DEBUG', ctx, msg, data),
    info:  (ctx, msg, data) => log('INFO',  ctx, msg, data),
    warn:  (ctx, msg, data) => log('WARN',  ctx, msg, data),
    error: (ctx, msg, data) => log('ERROR', ctx, msg, data),
    fatal: (ctx, msg, data) => {
      log('FATAL', ctx, msg, data);
      flush(); // always sync-flush on fatal
    },
    withTask(taskId: string): ILogger {
      // Return a scoped logger that stamps taskId on every entry
      const scoped: ILogger = {
        get sessionId() { return sessionId; },
        debug: (ctx, msg, data) => { const prev = _taskId; _taskId = taskId; log('DEBUG', ctx, msg, data); _taskId = prev; },
        info:  (ctx, msg, data) => { const prev = _taskId; _taskId = taskId; log('INFO',  ctx, msg, data); _taskId = prev; },
        warn:  (ctx, msg, data) => { const prev = _taskId; _taskId = taskId; log('WARN',  ctx, msg, data); _taskId = prev; },
        error: (ctx, msg, data) => { const prev = _taskId; _taskId = taskId; log('ERROR', ctx, msg, data); _taskId = prev; },
        fatal: (ctx, msg, data) => { const prev = _taskId; _taskId = taskId; log('FATAL', ctx, msg, data); _taskId = prev; flush(); },
        withTask: (id) => instance.withTask(id),
        timeline: (kind, tid, detail) => instance.timeline(kind, tid ?? taskId, detail),
        flush, 
        readTail: instance.readTail,
        readOpsTail: instance.readOpsTail,
        get logPath() { return instance.logPath; }
      };
      return scoped;
    },
    timeline(kind: TimelineEventKind, taskId: string | undefined, detail: Record<string, unknown>): void {
      const entry: TimelineEntry = { t: new Date().toISOString(), sessionId, kind, detail, ...(taskId ? { taskId } : {}) };
      try { fs.appendFileSync(config.opsFile, JSON.stringify(sanitizeTelemetryValue(entry)) + '\n'); } catch {}
    },
    flush,
    readTail(n: number): LogEntry[] {
      try {
        flush();
        const content = fs.readFileSync(config.logFile, 'utf-8');
        const lines = content.trim().split('\n').filter(Boolean);
        return lines.slice(-n).map(l => sanitizeTelemetryValue(JSON.parse(l)) as LogEntry).filter(Boolean);
      } catch { return []; }
    },
    readOpsTail(n: number): TimelineEntry[] {
      try {
        const content = fs.readFileSync(config.opsFile, 'utf-8');
        const lines = content.trim().split('\n').filter(Boolean);
        return lines.slice(-n).map(l => sanitizeTelemetryValue(JSON.parse(l)) as TimelineEntry).filter(Boolean);
      } catch { return []; }
    },
  };

  // Write app_start timeline entry
  instance.timeline('app_start', undefined, { version: process.env.npm_package_version ?? 'unknown', platform: process.platform });

  return instance;
}
