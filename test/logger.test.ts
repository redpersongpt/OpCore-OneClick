import { test, describe } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createLogger } from '../electron/logger.js';

// Helper to create an isolated temp dir for each test
function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `logger-test-${prefix}-`));
}

function makeConfig(dir: string, overrides: Partial<Parameters<typeof createLogger>[0]> = {}) {
  return {
    logFile:         path.join(dir, 'app.log'),
    opsFile:         path.join(dir, 'operations.log'),
    minLevel:        'DEBUG' as const,
    maxFileSizeBytes: 10 * 1024 * 1024, // 10 MB default
    rotationCount:   3,
    flushIntervalMs: 500,
    crashSafeSync:   true,
    isPackaged:      true, // suppress console noise in tests
    ...overrides,
  };
}

describe('logger', () => {
  test('write-behind buffer flushes on interval', async () => {
    const dir = makeTempDir('flush');
    try {
      const logger = createLogger(makeConfig(dir, { flushIntervalMs: 50, crashSafeSync: false }));
      logger.debug('test', 'buffer entry 1');
      logger.debug('test', 'buffer entry 2');
      // Buffer may not have flushed yet (crashSafeSync: false so no sync write)
      // Wait past the flush interval
      await new Promise(r => setTimeout(r, 150));
      const content = fs.readFileSync(path.join(dir, 'app.log'), 'utf-8');
      assert.ok(content.includes('buffer entry 1'), 'First buffered entry should be in log after flush interval');
      assert.ok(content.includes('buffer entry 2'), 'Second buffered entry should be in log after flush interval');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('WARN writes synchronously (crashSafeSync)', () => {
    const dir = makeTempDir('warn-sync');
    try {
      const logger = createLogger(makeConfig(dir, { flushIntervalMs: 60_000, crashSafeSync: true }));
      logger.warn('test', 'sync warning message');
      // crashSafeSync writes WARN synchronously — file must exist immediately
      const logFile = path.join(dir, 'app.log');
      assert.ok(fs.existsSync(logFile), 'Log file must exist after synchronous WARN write');
      const content = fs.readFileSync(logFile, 'utf-8');
      assert.ok(content.includes('sync warning message'), 'WARN message must be present immediately without waiting for flush interval');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('crash recovery detection writes crash_recovery timeline entry', () => {
    const dir = makeTempDir('crash');
    try {
      // Manually plant a session.lock to simulate a prior unclean session
      const lockFile = path.join(dir, 'session.lock');
      fs.writeFileSync(lockFile, JSON.stringify({ sessionId: 'old-session-id', startedAt: new Date().toISOString() }));

      // Creating a new logger should detect the lock and write a crash_recovery entry
      createLogger(makeConfig(dir));

      const opsContent = fs.readFileSync(path.join(dir, 'operations.log'), 'utf-8');
      const entries = opsContent.trim().split('\n').filter(Boolean).map(l => JSON.parse(l));
      const crashEntry = entries.find((e: any) => e.kind === 'crash_recovery');
      assert.ok(crashEntry, 'operations.log must contain a crash_recovery entry');
      assert.equal((crashEntry as any).detail.previousSession, 'old-session-id', 'crash_recovery entry must reference the previous session id');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('log rotation creates app.log.old when size limit exceeded', () => {
    const dir = makeTempDir('rotate');
    try {
      // Use crashSafeSync:false so writes go through the buffer path.
      // rotateIfNeeded is called when flush() drains the buffer — this is the
      // rotation code path. maxFileSizeBytes:100 ensures a single entry exceeds
      // the limit and triggers a rename on the second flush() call.
      const logger = createLogger(makeConfig(dir, { maxFileSizeBytes: 100, crashSafeSync: false }));
      // Write enough entries to exceed 100 bytes in the buffer
      for (let i = 0; i < 5; i++) {
        logger.debug('test', `rotation entry ${i} padding text to make entries larger than 100 bytes total`);
      }
      // First flush: rotateIfNeeded checks the file — file may not exist yet on first flush,
      // so rotation may not happen. Write the content, then on the next flush the file
      // will be over the limit.
      logger.flush(); // writes ~1000 bytes to app.log
      // Write more entries
      for (let i = 0; i < 3; i++) {
        logger.debug('test', `post-flush rotation entry ${i} more padding text to push over 100 bytes`);
      }
      logger.flush(); // now app.log is >100 bytes → rotateIfNeeded fires → app.log.old is created
      const oldFile = path.join(dir, 'app.log.old');
      assert.ok(fs.existsSync(oldFile), 'app.log.old must exist after log rotation due to size limit');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('readTail returns exactly the last N entries', () => {
    const dir = makeTempDir('tail');
    try {
      const logger = createLogger(makeConfig(dir, { crashSafeSync: true }));
      for (let i = 0; i < 10; i++) {
        logger.warn('test', `entry-${i}`);
      }
      const tail = logger.readTail(3);
      assert.equal(tail.length, 3, 'readTail(3) must return exactly 3 entries');
      // The last 3 entries should be entry-7, entry-8, entry-9
      assert.equal(tail[0].msg, 'entry-7', 'First of last 3 should be entry-7');
      assert.equal(tail[1].msg, 'entry-8', 'Second of last 3 should be entry-8');
      assert.equal(tail[2].msg, 'entry-9', 'Third of last 3 should be entry-9 (most recent)');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('timeline entries go to opsFile', () => {
    const dir = makeTempDir('timeline');
    try {
      const logger = createLogger(makeConfig(dir));
      logger.timeline('task_start', 'test-task-id-123', { kind: 'kext-fetch', detail: 'payload' });

      const opsContent = fs.readFileSync(path.join(dir, 'operations.log'), 'utf-8');
      const entries = opsContent.trim().split('\n').filter(Boolean).map(l => JSON.parse(l));
      const taskStartEntry = entries.find((e: any) => e.kind === 'task_start');
      assert.ok(taskStartEntry, 'operations.log must contain the task_start timeline entry');
      assert.equal((taskStartEntry as any).taskId, 'test-task-id-123', 'timeline entry must carry the taskId');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('sanitizes paths, tokens, and raw identifiers in log output', () => {
    const dir = makeTempDir('sanitize');
    try {
      const logger = createLogger(makeConfig(dir, { crashSafeSync: true }));
      logger.warn('flash-auth', 'Failure at /Users/alice/EFI with flashconf.secret-token', {
        serialNumber: 'USB-12345',
        device: '/dev/disk4',
        efiPath: '/Users/alice/EFI',
        token: 'flashconf.secret-token',
      });

      const content = fs.readFileSync(path.join(dir, 'app.log'), 'utf-8');
      assert.equal(content.includes('/Users/alice'), false);
      assert.equal(content.includes('USB-12345'), false);
      assert.equal(content.includes('flashconf.secret-token'), false);
      assert.equal(content.includes('/dev/disk4'), false);
      assert.equal(content.includes('disk4'), true);
      assert.equal(content.includes('[path:EFI]'), true);

      const tail = logger.readTail(1)[0] as any;
      assert.equal(tail.serialNumber, '[redacted-identifier]');
      assert.equal(tail.device, 'disk4');
      assert.equal(tail.efiPath, '[path:EFI]');
      assert.equal(tail.token, '[redacted-token]');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
