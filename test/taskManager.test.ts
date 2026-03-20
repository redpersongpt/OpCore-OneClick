import { test, describe } from 'vitest';
import assert from 'node:assert/strict';
import { createTaskRegistry } from '../electron/taskManager.js';
import type { TaskUpdatePayload, TaskKind } from '../electron/taskManager.js';

// Minimal mock logger — satisfies ILogger without any file I/O
const mockLogger = {
  debug:    () => {},
  info:     () => {},
  warn:     () => {},
  error:    () => {},
  fatal:    () => {},
  withTask: function() { return mockLogger; },
  timeline: () => {},
  flush:    () => {},
  readTail: () => [],
  get sessionId() { return 'test-session'; },
} as any;

function makeRegistry() {
  const pushCalls: TaskUpdatePayload[] = [];
  const pushFn = (p: TaskUpdatePayload) => pushCalls.push(p);
  const registry = createTaskRegistry(pushFn, mockLogger);
  return { registry, pushCalls };
}

describe('taskManager', () => {
  test('create() returns a running task', () => {
    const { registry, pushCalls } = makeRegistry();
    const token = registry.create('kext-fetch');
    const state = registry.get(token.taskId);
    assert.ok(state, 'Task must exist in registry after create()');
    assert.equal(state!.status, 'running', 'Newly created task must have status "running"');
    assert.equal(state!.kind, 'kext-fetch');
    assert.equal(pushCalls.length, 1, 'pushFn must be called once on create');
  });

  test('complete() transitions to complete and sets endedAt', () => {
    const { registry, pushCalls } = makeRegistry();
    const token = registry.create('efi-build');
    const initialCallCount = pushCalls.length;
    registry.complete(token.taskId);
    const state = registry.get(token.taskId);
    assert.ok(state, 'Task must still be retrievable after complete()');
    assert.equal(state!.status, 'complete');
    assert.ok(state!.endedAt !== null, 'endedAt must be set after complete()');
    assert.equal(pushCalls.length, initialCallCount + 1, 'pushFn must be called once more on complete');
  });

  test('fail() transitions to failed with the error message', () => {
    const { registry } = makeRegistry();
    const token = registry.create('recovery-download');
    registry.fail(token.taskId, 'network error');
    const state = registry.get(token.taskId);
    assert.ok(state);
    assert.equal(state!.status, 'failed');
    assert.equal(state!.error, 'network error');
    assert.ok(state!.endedAt !== null, 'endedAt must be set after fail()');
  });

  test('cancel() sets token.aborted to true and status to cancelled', () => {
    const { registry } = makeRegistry();
    const token = registry.create('usb-flash');
    assert.equal(token.aborted, false);
    registry.cancel(token.taskId);
    assert.equal(token.aborted, true, 'token.aborted must be true after cancel()');
    const state = registry.get(token.taskId);
    assert.ok(state);
    assert.equal(state!.status, 'cancelled');
  });

  test('token.check() throws after cancel()', () => {
    const { registry } = makeRegistry();
    const token = registry.create('usb-flash');
    registry.cancel(token.taskId);
    assert.throws(
      () => token.check(),
      /cancelled/i,
      'token.check() must throw when task has been cancelled'
    );
  });

  test('recovery-download progress is throttled (250ms window)', async () => {
    const { registry, pushCalls } = makeRegistry();
    const token = registry.create('recovery-download');
    const countAtStart = pushCalls.length; // 1 from create

    // Rapid-fire 5 progress updates at 50% — within 10ms (well within 250ms throttle)
    for (let i = 0; i < 5; i++) {
      registry.updateProgress(token.taskId, {
        kind: 'recovery-download', percent: 50, status: 'Downloading',
        bytesDownloaded: 1000 * (i + 1), dmgDest: '/tmp/x.dmg', clDest: '/tmp/x'
      });
    }

    // Only the first call should have passed through (pct jump <= 1, within 250ms)
    const emitted = pushCalls.length - countAtStart;
    assert.ok(emitted <= 2, `Expected ≤2 throttled emissions but got ${emitted}`);
  });

  test('kext-fetch progress is never throttled (emits every call)', () => {
    const { registry, pushCalls } = makeRegistry();
    const token = registry.create('kext-fetch');
    const countAtStart = pushCalls.length;

    for (let i = 0; i < 5; i++) {
      registry.updateProgress(token.taskId, {
        kind: 'kext-fetch', kextName: `Kext${i}`, version: '1.0', index: i, total: 10
      });
    }

    assert.equal(pushCalls.length - countAtStart, 5, 'kext-fetch must emit on every updateProgress call (no throttle)');
  });

  test('list() returns all tasks', () => {
    const { registry } = makeRegistry();
    registry.create('kext-fetch');
    registry.create('recovery-download');
    registry.create('usb-flash');
    assert.equal(registry.list().length, 3, 'list() must return all 3 created tasks');
  });

  test('updateProgress after complete() throws (strict state machine)', () => {
    const { registry } = makeRegistry();
    const token = registry.create('kext-fetch');
    registry.complete(token.taskId);

    // Strict state machine: updateProgress on a terminal task must throw
    assert.throws(
      () => registry.updateProgress(token.taskId, {
        kind: 'kext-fetch', kextName: 'Lilu', version: '1.6', index: 1, total: 5
      }),
      (err: Error) => err.message.includes('terminal task') && err.message.includes(token.taskId),
      'Must throw with terminal task message'
    );
  });

  test('updateProgress after cancel() throws (strict state machine)', () => {
    const { registry } = makeRegistry();
    const token = registry.create('recovery-download');
    registry.cancel(token.taskId);

    assert.throws(
      () => registry.updateProgress(token.taskId, {
        kind: 'recovery-download', percent: 50, status: 'late event',
        bytesDownloaded: 0, dmgDest: '', clDest: '',
      }),
      (err: Error) => err.message.includes('terminal task'),
      'Must throw after cancel'
    );
  });
});
