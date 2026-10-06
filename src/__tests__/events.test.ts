import { beforeEach, describe, expect, it, vi } from 'vitest';

const listenMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

const api = vi.hoisted(() => ({
  flashUsb: vi.fn(),
  flashPrepareConfirmation: vi.fn(),
  downloadRecovery: vi.fn(),
  taskCancel: vi.fn(),
}));
vi.mock('../bridge/api', () => ({ api }));

import { clampFraction, normalizeFlashProgress, normalizeRecoveryProgress, normalizeTaskUpdate, subscribe } from '../bridge/events';
import { flashMilestones, milestoneStates } from '../lib/flash';
import { toNum } from '../lib/num';
import { useDeploy } from '../stores/deploy';
import { routeFlashProgress, routeRecoveryProgress, routeTaskUpdate } from '../stores/routing';
import { useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

describe('event payload normalisation', () => {
  it('clampFraction never yields NaN', () => {
    expect(clampFraction(NaN)).toBeNull();
    expect(clampFraction(Infinity)).toBeNull();
    expect(clampFraction('0.5')).toBeNull();
    expect(clampFraction(1.7)).toBe(1);
    expect(clampFraction(-1)).toBe(0);
    expect(clampFraction(0.25)).toBe(0.25);
  });

  it('accepts a valid task update and rejects malformed ones', () => {
    expect(normalizeTaskUpdate({ taskId: 'a', kind: 'efi-build', status: 'running', progress: NaN, message: null, detail: null })).toEqual({
      taskId: 'a',
      kind: 'efi-build',
      status: 'running',
      progress: null,
      message: null,
      detail: null,
    });
    expect(normalizeTaskUpdate({ taskId: 'a', kind: 'efi-build', status: 'paused' })).toBeNull();
    expect(normalizeTaskUpdate(null)).toBeNull();
    expect(normalizeTaskUpdate('task')).toBeNull();
  });

  it('computes recovery progress from bytes when the backend omits it', () => {
    const p = normalizeRecoveryProgress({ taskId: 't', version: '15', phase: 'downloading', downloaded: 250, total: 1000, progress: null, error: null });
    expect(p?.progress).toBe(0.25);
    expect(toNum(p?.downloaded)).toBe(250);
    const unknownTotal = normalizeRecoveryProgress({ taskId: 't', version: '15', phase: 'resolving', downloaded: 0, total: 0 });
    expect(unknownTotal?.progress).toBeNull();
    expect(unknownTotal?.total).toBeNull();
    // The old `{percent, status}` shape must not produce a NaN progress bar.
    expect(normalizeRecoveryProgress({ percent: 50, status: 'x' })).toBeNull();
    expect(normalizeRecoveryProgress({ taskId: 't', version: '27', phase: 'downloading', downloaded: 1 })).toBeNull();
  });

  it('normalises flash progress', () => {
    expect(normalizeFlashProgress({ taskId: 'f', phase: 'copy-efi', progress: 2, message: 'Copying' })).toEqual({
      taskId: 'f',
      phase: 'copy-efi',
      progress: 1,
      message: 'Copying',
      error: null,
    });
    expect(normalizeFlashProgress({ phase: 'erase', detail: 'x' })).toBeNull();
  });
});

describe('flash milestones', () => {
  it('follows the backend phases and tolerates unknown ones', () => {
    const ms = flashMilestones(true);
    expect(ms).toEqual(['prepare', 'partition', 'format', 'copy-efi', 'copy-recovery', 'verify']);
    expect(flashMilestones(false)).not.toContain('copy-recovery');
    expect(milestoneStates(ms, ['prepare', 'mystery', 'format'], 'running')).toEqual([
      'done',
      'done',
      'active',
      'pending',
      'pending',
      'pending',
    ]);
    expect(milestoneStates(ms, ['prepare', 'partition'], 'failed')[1]).toBe('failed');
    expect(milestoneStates(ms, [], 'running')[0]).toBe('active');
    expect(milestoneStates(ms, ['prepare'], 'done').every((s) => s === 'done')).toBe(true);
  });
});

describe('event routing', () => {
  beforeEach(() => {
    useTasks.getState().clear();
    useWizard.getState().reset();
    useDeploy.setState({ flashStatus: 'idle', flashPhases: [], flashTaskId: null, recoveryDownloading: false });
  });

  it('task updates feed the task list, ignoring garbage', () => {
    routeTaskUpdate({ taskId: 'a', kind: 'hardware-scan', status: 'running', progress: 0.5, message: 'CPU', detail: null });
    routeTaskUpdate({ nope: true });
    expect(useTasks.getState().order).toEqual(['a']);
    expect(useTasks.getState().visible()?.taskId).toBe('a');
    routeTaskUpdate({ taskId: 'a', kind: 'hardware-scan', status: 'completed', progress: 1, message: null, detail: null });
    expect(useTasks.getState().latest('hardware-scan')?.status).toBe('completed');
    useTasks.getState().dismiss('a');
    expect(useTasks.getState().visible()).toBeNull();
  });

  it('an older failed task does not resurface after a newer one is dismissed', () => {
    routeTaskUpdate({ taskId: 'old', kind: 'hardware-scan', status: 'failed', progress: null, message: 'WMI failed', detail: null });
    routeTaskUpdate({ taskId: 'new', kind: 'hardware-scan', status: 'completed', progress: 1, message: null, detail: null });
    expect(useTasks.getState().visible()?.taskId).toBe('new');
    useTasks.getState().dismiss('new');
    expect(useTasks.getState().visible()).toBeNull();
    routeTaskUpdate({ taskId: 'b', kind: 'efi-build', status: 'running', progress: 0.1, message: null, detail: null });
    expect(useTasks.getState().visible()?.taskId).toBe('b');
  });

  it('flash progress is applied only while a flash runs, and only for its task', async () => {
    routeFlashProgress({ taskId: 'f1', phase: 'format', progress: 0.3, message: 'x' });
    expect(useDeploy.getState().flashProgress).toBeNull();

    let resolve: () => void = () => undefined;
    api.flashUsb.mockReturnValue(new Promise<void>((r) => (resolve = r)));
    useDeploy.setState({ confirmation: { token: 'tok', device: '/dev/sdb', expiresAt: 0, diskDisplay: 'USB', efiHash: 'h', recovery: '15' } });
    const flashing = useDeploy.getState().flash('/efi');
    expect(useWizard.getState().locks).toContain('flash');
    routeFlashProgress({ taskId: 'f1', phase: 'partition', progress: 0.2, message: 'p' });
    routeFlashProgress({ taskId: 'other', phase: 'verify', progress: 0.9, message: 'v' });
    routeFlashProgress({ taskId: 'f1', phase: 'copy-efi', progress: 0.5, message: 'c' });
    expect(useDeploy.getState().flashPhases).toEqual(['partition', 'copy-efi']);
    expect(api.flashUsb).toHaveBeenCalledWith('/dev/sdb', '/efi', 'tok', '15');
    resolve();
    expect(await flashing).toBe(true);
    expect(useDeploy.getState().flashStatus).toBe('done');
    expect(useDeploy.getState().flashed).toBe(true);
    expect(useWizard.getState().locks).toEqual([]);
  });

  it('a rejected flash ends in the failed state with the backend error', async () => {
    api.flashUsb.mockRejectedValue({ code: 'DISK_BUSY', message: 'Volume in use', suggestion: 'Close Explorer windows' });
    useDeploy.setState({ confirmation: { token: 'tok', device: '/dev/sdb', expiresAt: 0, diskDisplay: 'USB', efiHash: 'h', recovery: null } });
    expect(await useDeploy.getState().flash('/efi')).toBe(false);
    expect(useDeploy.getState().flashStatus).toBe('failed');
    expect(useDeploy.getState().flashError?.suggestion).toBe('Close Explorer windows');
    useDeploy.getState().resetFlash();
    expect(useDeploy.getState().flashStatus).toBe('idle');
  });

  it('writes the recovery the token was issued for and forgets an older successful write', async () => {
    api.flashUsb.mockResolvedValueOnce(undefined).mockRejectedValueOnce({ code: 'IO_ERROR', message: 'write failed' });
    const confirmation = { token: 'tok', device: '/dev/sdb', expiresAt: 0, diskDisplay: 'USB', efiHash: 'h', recovery: '14' as const };
    useDeploy.setState({ confirmation, flashed: false });
    expect(await useDeploy.getState().flash('/efi')).toBe(true);
    expect(api.flashUsb).toHaveBeenLastCalledWith('/dev/sdb', '/efi', 'tok', '14');
    expect(useDeploy.getState().flashed).toBe(true);

    // A rebuild (or any invalidation) means the drive no longer holds the current EFI.
    useDeploy.getState().resetFlash();
    expect(useDeploy.getState().flashed).toBe(false);

    // A failed second write leaves the drive erased.
    useDeploy.setState({ flashed: true, confirmation: { ...confirmation, token: 'tok2', recovery: null } });
    expect(await useDeploy.getState().flash('/efi')).toBe(false);
    expect(api.flashUsb).toHaveBeenLastCalledWith('/dev/sdb', '/efi', 'tok2', null);
    expect(useDeploy.getState().flashed).toBe(false);
  });

  it('recovery progress is applied only for the version being downloaded', async () => {
    let resolve: (v: unknown) => void = () => undefined;
    api.downloadRecovery.mockReturnValue(new Promise((r) => (resolve = r)));
    const download = useDeploy.getState().downloadRecovery('15');
    routeRecoveryProgress({ taskId: 'r', version: '14', phase: 'downloading', downloaded: 1, total: 2 });
    expect(useDeploy.getState().recoveryProgress).toBeNull();
    routeRecoveryProgress({ taskId: 'r', version: '15', phase: 'downloading', downloaded: 1, total: 4 });
    expect(useDeploy.getState().recoveryProgress?.progress).toBe(0.25);
    expect(useDeploy.getState().recoveryTaskId).toBe('r');
    resolve({ available: true, version: '15', dmgPath: '/r.dmg', chunklistPath: '/r.chunklist', sizeBytes: 900, verified: true });
    await download;
    expect(useDeploy.getState().recoveryDownloading).toBe(false);
    expect(useDeploy.getState().recoveryInfo?.verified).toBe(true);
  });
});

describe('subscribe', () => {
  beforeEach(() => {
    listenMock.mockReset();
  });

  it('unlistens when disposed before listen() resolved (StrictMode double mount)', async () => {
    const unlisten = vi.fn();
    let resolveListen: (fn: () => void) => void = () => undefined;
    listenMock.mockReturnValue(new Promise((r) => (resolveListen = r)));
    const handler = vi.fn();
    const dispose = subscribe('task:update', handler);
    dispose();
    resolveListen(unlisten);
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);
    const callback = listenMock.mock.calls[0][1] as (e: { payload: unknown }) => void;
    callback({ payload: 1 });
    expect(handler).not.toHaveBeenCalled();
  });

  it('delivers payloads and unlistens once on dispose', async () => {
    const unlisten = vi.fn();
    listenMock.mockResolvedValue(unlisten);
    const handler = vi.fn();
    const dispose = subscribe('flash:progress', handler);
    await Promise.resolve();
    await Promise.resolve();
    const callback = listenMock.mock.calls[0][1] as (e: { payload: unknown }) => void;
    callback({ payload: { a: 1 } });
    expect(handler).toHaveBeenCalledWith({ a: 1 });
    dispose();
    dispose();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('survives a missing IPC bridge', () => {
    listenMock.mockImplementation(() => {
      throw new Error('window.__TAURI_INTERNALS__ is undefined');
    });
    expect(() => subscribe('task:update', vi.fn())()).not.toThrow();
  });
});
