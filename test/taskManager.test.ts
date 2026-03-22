import { EventEmitter } from 'node:events';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createTaskRegistry } from '../electron/taskManager.js';

function createLogger() {
  return {
    timeline: vi.fn(),
    warn: vi.fn(),
  };
}

describe('createTaskRegistry watchdog', () => {
  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it('does not fail an active usb-flash child process after 60 seconds', () => {
    vi.useFakeTimers();
    const logger = createLogger();
    const registry = createTaskRegistry(() => {}, logger as any);
    const token = registry.create('usb-flash');
    const child = new EventEmitter() as EventEmitter & { kill: ReturnType<typeof vi.fn> };
    child.kill = vi.fn();
    token.registerProcess(child);

    vi.advanceTimersByTime(61_000);

    expect(registry.get(token.taskId)?.status).toBe('running');
  });

  it('still fails an active usb-flash child process after the extended threshold', () => {
    vi.useFakeTimers();
    const logger = createLogger();
    const registry = createTaskRegistry(() => {}, logger as any);
    const token = registry.create('usb-flash');
    const child = new EventEmitter() as EventEmitter & { kill: ReturnType<typeof vi.fn> };
    child.kill = vi.fn();
    token.registerProcess(child);

    vi.advanceTimersByTime(15 * 60_000 + 10_000);

    expect(registry.get(token.taskId)?.status).toBe('failed');
    expect(registry.get(token.taskId)?.error).toContain('900 seconds');
  });
});
