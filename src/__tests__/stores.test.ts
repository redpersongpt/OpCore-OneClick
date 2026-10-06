import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildResult, option, plan, profile, report } from './fixtures';

const api = vi.hoisted(() => ({
  scanHardware: vi.fn(),
  refreshProfile: vi.fn(),
  importProfile: vi.fn(),
  exportProfile: vi.fn(),
  checkCompatibility: vi.fn(),
  planBuild: vi.fn(),
  buildEfi: vi.fn(),
  taskCancel: vi.fn(),
  clearState: vi.fn(),
  validateEfi: vi.fn(),
  getAppInfo: vi.fn(),
  checkForUpdates: vi.fn(),
  getCatalog: vi.fn(),
  getPersistedState: vi.fn(),
}));

vi.mock('../bridge/api', () => ({ api }));

import { useApp } from '../stores/app';
import { buildKey, canReuseIdentity, DEFAULT_DRAFT, toBuildOptions, useBuild } from '../stores/build';
import { compatKey, useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import {
  applyProfile,
  editProfile,
  importProfile,
  persistedSnapshot,
  refreshProfile,
  resetAll,
  resume,
  runScan,
  selectTarget,
  setOptions,
  startDemo,
  startOver,
} from '../stores/flow';
import { useFirmware } from '../stores/firmware';
import { useHardware } from '../stores/hardware';
import { useTasks } from '../stores/tasks';
import { useWizard, type Step } from '../stores/wizard';

const ALL_BEFORE_DEPLOY: Step[] = ['welcome', 'scan', 'hardware', 'compatibility', 'bios', 'build', 'review'];

/** Simulate a finished session up to the deploy step. */
function completeThroughReview() {
  const p = profile();
  useHardware.getState().setProfile(p, null);
  useCompat.setState({ report: report('15'), reportKey: compatKey(p, '15'), requestKey: compatKey(p, '15'), target: '15' });
  useFirmware.setState({ settings: [], settingsKey: compatKey(p, '15') });
  const options = toBuildOptions(DEFAULT_DRAFT, '15', null);
  useBuild.setState({ result: buildResult('15'), resultKey: buildKey(p, options), plan: plan('15'), planKey: buildKey(p, options) });
  for (const s of ALL_BEFORE_DEPLOY) useWizard.getState().complete(s);
}

describe('stores', () => {
  beforeEach(() => {
    resetAll();
    Object.values(api).forEach((fn) => fn.mockReset());
    api.checkCompatibility.mockResolvedValue(report('15'));
    api.clearState.mockResolvedValue(undefined);
  });

  describe('hardware scan', () => {
    it('stores the scan result and locks navigation while scanning', async () => {
      let resolve: (v: unknown) => void = () => undefined;
      api.scanHardware.mockReturnValue(new Promise((r) => (resolve = r)));
      const pending = runScan();
      expect(useHardware.getState().scanning).toBe(true);
      expect(useWizard.getState().locks).toContain('scan');
      expect(await useHardware.getState().scan()).toBe(false); // concurrent call ignored
      resolve({ detected: { warnings: [] }, profile: profile() });
      expect(await pending).toBe(true);
      expect(useHardware.getState().profile?.cpu.platform).toBe('coffee_lake');
      expect(useWizard.getState().locks).toEqual([]);
      expect(api.scanHardware).toHaveBeenCalledTimes(1);
    });

    it('keeps the AppError on failure and never falls back to demo data', async () => {
      api.scanHardware.mockRejectedValue({ code: 'SCAN_FAILED', message: 'WMI unavailable', severity: 'error', recoverable: true, suggestion: 'Run as administrator', context: null });
      expect(await runScan()).toBe(false);
      const s = useHardware.getState();
      expect(s.profile).toBeNull();
      expect(s.isDemo).toBe(false);
      expect(s.scanError?.code).toBe('SCAN_FAILED');
      expect(s.scanError?.suggestion).toBe('Run as administrator');
      expect(s.scanAttempted).toBe(true);
    });

    it('an imported profile is never treated as a scan of this computer', async () => {
      useWizard.getState().complete('welcome');
      api.importProfile.mockResolvedValueOnce({ ...profile(), source: 'scan' });
      expect(await importProfile('/tmp/other-pc.json')).toBe(true);
      expect(useHardware.getState().profile?.source).toBe('imported');
      expect(useWizard.getState().step).toBe('hardware');
      api.importProfile.mockResolvedValueOnce({ ...profile(), source: 'manual' });
      await importProfile('/tmp/manual.json');
      expect(useHardware.getState().profile?.source).toBe('manual');
    });

    it('demo mode is only entered explicitly', () => {
      useWizard.getState().complete('welcome');
      startDemo();
      expect(useHardware.getState().isDemo).toBe(true);
      expect(useHardware.getState().profile?.source).toBe('demo');
      expect(useWizard.getState().step).toBe('hardware');
      expect(persistedSnapshot()).toBeNull();
    });
  });

  describe('invalidation', () => {
    it('editing the profile drops compatibility, BIOS settings and the build', () => {
      completeThroughReview();
      editProfile((p) => ({ ...p, cpu: { ...p.cpu, cores: 6 } }));
      expect(useHardware.getState().dirty).toBe(true);
      expect(useCompat.getState().report).toBeNull();
      expect(useCompat.getState().target).toBe('15');
      expect(useFirmware.getState().settings).toBeNull();
      expect(useBuild.getState().result).toBeNull();
      expect(useBuild.getState().plan).toBeNull();
      // Unapplied edits must go through the hardware step's Continue (refresh_profile) again.
      expect(useWizard.getState().completed).toEqual(['welcome', 'scan']);
      expect(useWizard.getState().step).toBe('hardware');
      expect(useWizard.getState().canVisit('compatibility')).toBe(false);
    });

    it('a refresh that changes nothing keeps the build', async () => {
      completeThroughReview();
      api.refreshProfile.mockImplementation(async (p) => p);
      expect(await refreshProfile()).toBe(true);
      expect(useBuild.getState().result).not.toBeNull();
      expect(useWizard.getState().completed).toEqual(ALL_BEFORE_DEPLOY);
    });

    it('a refresh that changes the profile invalidates later steps', async () => {
      completeThroughReview();
      api.refreshProfile.mockImplementation(async (p) => ({ ...p, cpu: { ...p.cpu, codename: 'Changed' } }));
      await refreshProfile();
      expect(useBuild.getState().result).toBeNull();
      expect(useWizard.getState().completed).not.toContain('compatibility');
    });

    it('changing the target on the compatibility step re-checks and drops the build', async () => {
      completeThroughReview();
      useWizard.getState().goTo('compatibility');
      api.checkCompatibility.mockResolvedValue(report('14'));
      selectTarget('14');
      expect(useCompat.getState().target).toBe('14');
      expect(useBuild.getState().result).toBeNull();
      expect(useFirmware.getState().settings).toBeNull();
      expect(useWizard.getState().completed).toEqual(['welcome', 'scan', 'hardware']);
      expect(api.checkCompatibility).toHaveBeenCalledWith(expect.anything(), '14');
      await vi.waitFor(() => expect(useCompat.getState().loading).toBe(false));
      expect(useCompat.getState().reportKey).toBe(compatKey(useHardware.getState().profile, '14'));
    });

    it('changing the target on the build step keeps BIOS confirmed', () => {
      completeThroughReview();
      useWizard.getState().goTo('build');
      selectTarget('14', 'build');
      expect(useWizard.getState().completed).toEqual(['welcome', 'scan', 'hardware', 'compatibility', 'bios']);
      expect(useWizard.getState().step).toBe('build');
      expect(useBuild.getState().result).toBeNull();
    });

    it('selecting the current target again does nothing', () => {
      completeThroughReview();
      selectTarget('15');
      expect(api.checkCompatibility).not.toHaveBeenCalled();
      expect(useBuild.getState().result).not.toBeNull();
    });

    it('changing a build option drops the build but not the BIOS step', () => {
      completeThroughReview();
      setOptions({ verbose: false });
      expect(useBuild.getState().draft.verbose).toBe(false);
      expect(useBuild.getState().result).toBeNull();
      expect(useWizard.getState().completed).toEqual(['welcome', 'scan', 'hardware', 'compatibility', 'bios']);
    });

    it('toggling identity reuse keeps the current EFI', () => {
      completeThroughReview();
      setOptions({ keepIdentity: false });
      expect(useBuild.getState().result).not.toBeNull();
      setOptions({ verbose: true }); // unchanged value: no-op
      expect(useBuild.getState().result).not.toBeNull();
    });

    it('a new hardware source resets everything after the scan', () => {
      completeThroughReview();
      useDeploy.setState({ flashStatus: 'failed' });
      useFirmware.getState().toggle('Secure Boot');
      applyProfile(profile(), null);
      expect(useCompat.getState().target).toBeNull();
      expect(useFirmware.getState().checked).toEqual({});
      expect(useDeploy.getState().flashStatus).toBe('idle');
      expect(useWizard.getState().completed).toEqual(['welcome', 'scan']);
    });
  });

  describe('build store', () => {
    it('build keys ignore the reused identity', () => {
      const p = profile();
      const withId = toBuildOptions({ ...DEFAULT_DRAFT, keepIdentity: true }, '15', buildResult().identity);
      const without = toBuildOptions({ ...DEFAULT_DRAFT, keepIdentity: false }, '15', buildResult().identity);
      expect(withId.identity).not.toBeNull();
      expect(without.identity).toBeNull();
      expect(buildKey(p, withId)).toBe(buildKey(p, without));
    });

    it('does not reuse an identity for a different forced SMBIOS model', () => {
      const options = toBuildOptions({ ...DEFAULT_DRAFT, smbiosOverride: 'MacPro7,1' }, '15', buildResult().identity);
      expect(options.identity).toBeNull();
    });

    it('reuses an identity only when the planned model matches it', () => {
      const identity = buildResult().identity; // iMac19,1
      expect(canReuseIdentity(DEFAULT_DRAFT, identity, null)).toBe(true);
      expect(canReuseIdentity(DEFAULT_DRAFT, identity, 'iMac19,1')).toBe(true);
      expect(canReuseIdentity(DEFAULT_DRAFT, identity, 'iMac20,1')).toBe(false);
      expect(toBuildOptions(DEFAULT_DRAFT, '26', identity, 'iMac20,1').identity).toBeNull();
      expect(canReuseIdentity({ ...DEFAULT_DRAFT, keepIdentity: false }, identity, 'iMac19,1')).toBe(false);
      expect(canReuseIdentity(DEFAULT_DRAFT, null, null)).toBe(false);
    });

    it('trims empty extra boot-args to null', () => {
      expect(toBuildOptions({ ...DEFAULT_DRAFT, extraBootArgs: '   ' }, '15', null).extraBootArgs).toBeNull();
      expect(toBuildOptions({ ...DEFAULT_DRAFT, extraBootArgs: ' alcid=11 ' }, '15', null).extraBootArgs).toBe('alcid=11');
    });

    it('routes efi-build task updates into the running build and records the identity', async () => {
      let resolve: (v: unknown) => void = () => undefined;
      api.buildEfi.mockReturnValue(new Promise((r) => (resolve = r)));
      const p = profile();
      const running = useBuild.getState().build(p, toBuildOptions(DEFAULT_DRAFT, '15', null));
      expect(useWizard.getState().locks).toContain('build');
      useBuild.getState().onTask({ taskId: 't1', kind: 'efi-build', status: 'running', progress: 0.4, message: 'Downloading kexts', detail: null });
      useBuild.getState().onTask({ taskId: 'other', kind: 'efi-build', status: 'running', progress: 0.9, message: 'x', detail: null });
      useBuild.getState().onTask({ taskId: 't1', kind: 'recovery-download', status: 'running', progress: 0.9, message: 'y', detail: null });
      expect(useBuild.getState()).toMatchObject({ taskId: 't1', progress: 0.4, message: 'Downloading kexts' });
      resolve(buildResult());
      await running;
      expect(useBuild.getState().identity?.model).toBe('iMac19,1');
      expect(useBuild.getState().validation?.valid).toBe(true);
      expect(useWizard.getState().locks).toEqual([]);
    });

    it('cancel finds the running build in the task list before its first update arrived', async () => {
      api.buildEfi.mockReturnValue(new Promise(() => undefined));
      api.taskCancel.mockResolvedValue(true);
      void useBuild.getState().build(profile(), toBuildOptions(DEFAULT_DRAFT, '15', null));
      useTasks.getState().apply({ taskId: 't7', kind: 'efi-build', status: 'running', progress: null, message: null, detail: null });
      await useBuild.getState().cancel();
      expect(api.taskCancel).toHaveBeenCalledWith('t7');
      await useBuild.getState().cancel();
      expect(api.taskCancel).toHaveBeenCalledTimes(1);
    });

    it('cancel calls task_cancel with the build task id', async () => {
      api.buildEfi.mockReturnValue(new Promise(() => undefined));
      api.taskCancel.mockResolvedValue(true);
      void useBuild.getState().build(profile(), toBuildOptions(DEFAULT_DRAFT, '15', null));
      useBuild.getState().onTask({ taskId: 't9', kind: 'efi-build', status: 'running', progress: null, message: null, detail: null });
      await useBuild.getState().cancel();
      expect(api.taskCancel).toHaveBeenCalledWith('t9');
      expect(useBuild.getState().cancelled).toBe(true);
    });
  });

  describe('compatibility store', () => {
    it('adopts the evaluated target when none was chosen', async () => {
      const p = profile();
      await useCompat.getState().check(p, null);
      const s = useCompat.getState();
      expect(s.target).toBe('15');
      expect(s.reportKey).toBe(compatKey(p, '15'));
      expect(s.requestKey).toBe(compatKey(p, '15'));
    });

    it('does not reuse a report evaluated for another release than the preselected one', async () => {
      const p = profile();
      // Backend evaluated Tahoe but recommends Sequoia: Sequoia is preselected and must be checked itself.
      api.checkCompatibility.mockResolvedValueOnce({ ...report('26'), recommended: '15', versions: [option('15', true, { recommended: true })] });
      await useCompat.getState().check(p, null);
      const s = useCompat.getState();
      expect(s.target).toBe('15');
      expect(s.reportKey).toBe(compatKey(p, null));
      expect(s.reportKey).not.toBe(compatKey(p, '15'));
      expect(s.requestKey).not.toBe(compatKey(p, '15'));
    });

    it('records the request key on failure so auto-fetch effects do not loop', async () => {
      const p = profile();
      api.checkCompatibility.mockRejectedValue({ code: 'X', message: 'boom' });
      await useCompat.getState().check(p, '15');
      expect(useCompat.getState().error?.message).toBe('boom');
      expect(useCompat.getState().requestKey).toBe(compatKey(p, '15'));
    });

    it('ignores a stale response that arrives after a newer request', async () => {
      const p = profile();
      let first: (v: unknown) => void = () => undefined;
      api.checkCompatibility.mockReturnValueOnce(new Promise((r) => (first = r))).mockResolvedValueOnce(report('14'));
      const a = useCompat.getState().check(p, '15');
      await useCompat.getState().check(p, '14');
      first(report('15'));
      await a;
      expect(useCompat.getState().report?.target).toBe('14');
    });
  });

  describe('app start', () => {
    beforeEach(() => {
      try {
        window.localStorage.clear();
      } catch {
        // ignore
      }
      api.getAppInfo.mockResolvedValue({ version: '5.1.0', opencoreVersion: '1.0.8', hostOs: 'windows', arch: 'x86_64' });
      api.getCatalog.mockResolvedValue({ macosVersions: [], cpuPlatforms: [], gpuFamilies: [], smbiosModels: [], formFactors: [], opencoreVersion: '1.0.8' });
      api.getPersistedState.mockResolvedValue({ currentStep: null, profile: null, target: null, identity: null, efiPath: null, timestamp: null });
      api.checkForUpdates.mockResolvedValue({ current: '5.1.0', latest: '5.2.0', updateAvailable: true, url: null, notes: null });
    });

    const start = async () => {
      useApp.setState({ initialized: false, update: null, info: null });
      await useApp.getState().init();
    };

    it('checks for updates once and reuses the answer on the next start', async () => {
      await start();
      expect(useApp.getState().update?.latest).toBe('5.2.0');
      await start();
      expect(useApp.getState().update?.latest).toBe('5.2.0');
      expect(api.checkForUpdates).toHaveBeenCalledTimes(1);
      expect(useApp.getState().info?.version).toBe('5.1.0');
    });

    it('checks again after the app itself was updated', async () => {
      await start();
      api.getAppInfo.mockResolvedValue({ version: '5.2.0', opencoreVersion: '1.0.8', hostOs: 'windows', arch: 'x86_64' });
      api.checkForUpdates.mockResolvedValue({ current: '5.2.0', latest: '5.2.0', updateAvailable: false, url: null, notes: null });
      await start();
      expect(api.checkForUpdates).toHaveBeenCalledTimes(2);
      expect(useApp.getState().update?.updateAvailable).toBe(false);
    });

    it('offers a saved session only when it has a profile', async () => {
      api.getPersistedState.mockResolvedValue({ currentStep: 'build', profile: profile(), target: '15', identity: null, efiPath: null, timestamp: null });
      await start();
      expect(useApp.getState().persisted?.target).toBe('15');
    });
  });

  describe('session', () => {
    it('startOver clears the stores and the saved state', async () => {
      completeThroughReview();
      await startOver();
      expect(useHardware.getState().profile).toBeNull();
      expect(useWizard.getState().step).toBe('scan');
      expect(api.clearState).toHaveBeenCalled();
    });

    it('resume restores profile, target and identity, then asks for a rebuild', () => {
      const p = profile();
      resume({ currentStep: 'deploy', profile: p, target: '14', identity: buildResult().identity, efiPath: '/old', timestamp: null });
      expect(useHardware.getState().profile?.cpu.name).toBe(p.cpu.name);
      expect(useCompat.getState().target).toBe('14');
      expect(useBuild.getState().identity?.serial).toBe('C02XXXXXXXXX');
      expect(useBuild.getState().previousEfiPath).toBe('/old');
      expect(useWizard.getState().step).toBe('compatibility');
      expect(useWizard.getState().completed).toEqual(['welcome', 'scan', 'hardware']);
    });

    it('resume is ignored while an operation runs', () => {
      completeThroughReview();
      useWizard.getState().lock('build');
      resume({ currentStep: 'deploy', profile: { ...profile(), cpu: { ...profile().cpu, name: 'Other' } }, target: '14', identity: null, efiPath: null, timestamp: null });
      expect(useHardware.getState().profile?.cpu.name).not.toBe('Other');
      expect(useWizard.getState().locks).toEqual(['build']);
    });

    it('persistedSnapshot carries the build identity and EFI path', () => {
      completeThroughReview();
      const snap = persistedSnapshot();
      expect(snap?.target).toBe('15');
      expect(snap?.efiPath).toBe('/tmp/build/b1');
      expect(snap?.identity?.model).toBe('iMac19,1');
    });
  });
});
