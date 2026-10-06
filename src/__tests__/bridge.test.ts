import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { api } from '../bridge/api';
import { describeError, isCancellation, toAppError } from '../bridge/errors';
import { profile } from './fixtures';

describe('toAppError', () => {
  it('keeps every field of a backend AppError', () => {
    const err = toAppError({
      code: 'DOWNLOAD_FAILED',
      message: 'Could not fetch Lilu',
      severity: 'warning',
      recoverable: true,
      suggestion: 'Check internet access and retry the EFI build.',
      context: { kext: 'Lilu' },
    });
    expect(err).toEqual({
      code: 'DOWNLOAD_FAILED',
      message: 'Could not fetch Lilu',
      severity: 'warning',
      recoverable: true,
      suggestion: 'Check internet access and retry the EFI build.',
      context: { kext: 'Lilu' },
    });
  });

  it('is idempotent', () => {
    const once = toAppError({ code: 'X', message: 'y', suggestion: 'z' });
    expect(toAppError(once)).toEqual(once);
  });

  it('parses JSON strings and plain strings', () => {
    expect(toAppError('{"code":"IO_ERROR","message":"Disk not writable"}').code).toBe('IO_ERROR');
    const plain = toAppError('command panicked');
    expect(plain.message).toBe('command panicked');
    expect(plain.code).toBe('UNKNOWN_ERROR');
    expect(plain.suggestion).toBeNull();
  });

  it('maps a missing IPC bridge to a clear code', () => {
    const err = toAppError(new TypeError("Cannot read properties of undefined (reading 'invoke')"));
    expect(err.code).toBe('IPC_UNAVAILABLE');
    expect(err.suggestion).not.toBeNull();
  });

  it('never returns "[object Object]"', () => {
    const err = toAppError({ detail: { nested: true } });
    expect(err.message).toContain('nested');
    expect(toAppError(undefined).message).toBe('Unknown error');
  });

  it('normalises unknown severities and blank suggestions', () => {
    const err = toAppError({ code: 'A', message: 'b', severity: 'fatal', suggestion: '  ' });
    expect(err.severity).toBe('error');
    expect(err.suggestion).toBeNull();
  });

  it('describes and classifies errors', () => {
    expect(describeError(toAppError({ code: 'A', message: 'b', suggestion: 'c' }))).toBe('[A] b (c)');
    expect(isCancellation(toAppError({ code: 'TASK_CANCELLED', message: 'x' }))).toBe(true);
    expect(isCancellation(toAppError({ code: 'IO_ERROR', message: 'x' }))).toBe(false);
    // A dismissed password prompt is a failure the user must see, not a cancel.
    expect(isCancellation(toAppError({ code: 'ELEVATION_CANCELLED', message: 'x' }))).toBe(false);
  });
});

describe('api wrappers', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('use the backend command names and camelCase arguments', async () => {
    invokeMock.mockResolvedValue(null);
    const p = profile();
    await api.checkCompatibility(p, '15');
    await api.flashUsb('/dev/sdb', '/efi', 'tok', null);
    await api.exportEfi('/efi', '/dest');
    await api.taskCancel('t1');
    await api.getCachedRecoveryInfo('26');
    await api.logGetTail();
    expect(invokeMock.mock.calls).toEqual([
      ['check_compatibility', { profile: p, target: '15' }],
      ['flash_usb', { device: '/dev/sdb', efiPath: '/efi', token: 'tok', recovery: null }],
      ['export_efi', { efiPath: '/efi', destination: '/dest' }],
      ['task_cancel', { taskId: 't1' }],
      ['get_cached_recovery_info', { version: '26' }],
      ['log_get_tail', { lines: null }],
    ]);
  });

  it('reject with a normalised AppError', async () => {
    invokeMock.mockRejectedValue({ code: 'NOT_ELEVATED', message: 'Need admin', suggestion: 'Restart as administrator' });
    await expect(api.listUsbDevices()).rejects.toMatchObject({
      code: 'NOT_ELEVATED',
      suggestion: 'Restart as administrator',
      severity: 'error',
      recoverable: false,
    });
  });

  it('only call commands the backend registers', () => {
    const lib = readFileSync(join(__dirname, '..', '..', 'src-tauri', 'src', 'lib.rs'), 'utf8');
    const registered = new Set([...lib.matchAll(/commands::\w+::(\w+),/g)].map((m) => m[1]));
    const source = readFileSync(join(__dirname, '..', 'bridge', 'api.ts'), 'utf8');
    const wrapped = [...source.matchAll(/call<[^>]+>\('(\w+)'/g)].map((m) => m[1]);
    expect(wrapped.length).toBeGreaterThanOrEqual(31);
    expect(wrapped.filter((c) => !registered.has(c))).toEqual([]);
  });
});

describe('generated contract types', () => {
  const dir = join(__dirname, '..', 'bridge', 'generated');
  const barrel = readFileSync(join(__dirname, '..', 'bridge', 'types.ts'), 'utf8');

  it('the barrel only re-exports files that exist', () => {
    const generated = new Set(
      readdirSync(dir)
        .filter((f) => f.endsWith('.ts'))
        .map((f) => f.replace(/\.ts$/, '')),
    );
    const exported = [...barrel.matchAll(/from '\.\/generated\/(\w+)'/g)].map((m) => m[1]);
    expect(exported.length).toBeGreaterThan(50);
    expect(exported.filter((name) => !generated.has(name))).toEqual([]);
  });

  it('contract types are never declared by hand', () => {
    expect(barrel).not.toMatch(/^\s*(export\s+)?(interface|type)\s+\w+\s*[={<]/m);
  });
});
