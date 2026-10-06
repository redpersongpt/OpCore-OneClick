import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildResult, option, plan, profile, report } from './fixtures';
import type { DiskInfo } from '../bridge/types';

const api = vi.hoisted(() => ({
  scanHardware: vi.fn(),
  importProfile: vi.fn(),
  checkCompatibility: vi.fn(),
  logGetSessionId: vi.fn(),
  logGetTail: vi.fn(),
  saveSupportLog: vi.fn(),
  clearAppCache: vi.fn(),
  clearRecoveryCache: vi.fn(),
  clearState: vi.fn(),
  checkForUpdates: vi.fn(),
  planBuild: vi.fn(),
  buildEfi: vi.fn(),
  checkPrivileges: vi.fn(),
  listUsbDevices: vi.fn(),
  getCachedRecoveryInfo: vi.fn(),
  flashPrepareConfirmation: vi.fn(),
  flashUsb: vi.fn(),
  taskCancel: vi.fn(),
}));
vi.mock('../bridge/api', () => ({ api }));

const dialog = vi.hoisted(() => ({ save: vi.fn(), open: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => dialog);

const shell = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('@tauri-apps/plugin-shell', () => shell);

import { I18nProvider } from '../i18n';
import Build from '../pages/Build';
import Compatibility from '../pages/Compatibility';
import Deploy from '../pages/Deploy';
import Scan from '../pages/Scan';
import Settings from '../pages/Settings';
import { useApp } from '../stores/app';
import { useBuild } from '../stores/build';
import { compatKey, useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import { resetAll } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { useWizard } from '../stores/wizard';

function renderEn(ui: ReactElement) {
  return render(<I18nProvider initial="en">{ui}</I18nProvider>);
}

beforeEach(() => {
  resetAll();
  Object.values(api).forEach((fn) => fn.mockReset());
  dialog.save.mockReset();
  dialog.open.mockReset();
  shell.open.mockReset();
  useApp.setState({ settingsOpen: false, info: null, update: null, updateError: null, persisted: null });
  try {
    window.localStorage.clear();
  } catch {
    // ignore
  }
});

describe('Scan page', () => {
  it('shows the scan error with retry and alternatives instead of switching to demo data', async () => {
    api.scanHardware.mockRejectedValue({
      code: 'SCAN_FAILED',
      message: 'WMI query failed',
      suggestion: 'Run the app as administrator',
    });
    useWizard.getState().complete('welcome');
    renderEn(<Scan />);

    expect(await screen.findByText('The hardware scan failed')).toBeInTheDocument();
    expect(screen.getByText('WMI query failed')).toBeInTheDocument();
    expect(screen.getByText('Run the app as administrator')).toBeInTheDocument();
    expect(useHardware.getState().isDemo).toBe(false);
    expect(useHardware.getState().profile).toBeNull();
    expect(api.scanHardware).toHaveBeenCalledTimes(1);

    api.scanHardware.mockResolvedValue({ detected: { warnings: [] }, profile: profile() });
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(useWizard.getState().step).toBe('hardware'));
    expect(api.scanHardware).toHaveBeenCalledTimes(2);
  });

  it('reports a failed re-scan and keeps the previous profile', async () => {
    useWizard.getState().complete('welcome');
    useHardware.getState().setProfile(profile(), null);
    useWizard.getState().complete('scan', false);
    api.scanHardware.mockRejectedValue({ code: 'SCAN_FAILED', message: 'WMI busy' });
    renderEn(<Scan />);
    fireEvent.click(screen.getByRole('button', { name: 'Scan again' }));
    expect(await screen.findByText('Scanning again failed; the previous profile is kept')).toBeInTheDocument();
    expect(screen.getByText('WMI busy')).toBeInTheDocument();
    expect(useHardware.getState().profile?.source).toBe('demo');
  });

  it('offers manual entry after a failed scan', async () => {
    api.scanHardware.mockRejectedValue({ code: 'SCAN_FAILED', message: 'nope' });
    useWizard.getState().complete('welcome');
    renderEn(<Scan />);
    fireEvent.click(await screen.findByRole('button', { name: 'Enter manually' }));
    expect(useHardware.getState().profile?.source).toBe('manual');
    expect(useWizard.getState().step).toBe('hardware');
  });

  it('imports a profile chosen in the open dialog', async () => {
    api.scanHardware.mockRejectedValue({ code: 'SCAN_FAILED', message: 'nope' });
    dialog.open.mockResolvedValue('/tmp/pc.json');
    api.importProfile.mockResolvedValue({ ...profile(), source: 'imported' });
    useWizard.getState().complete('welcome');
    renderEn(<Scan />);
    fireEvent.click(await screen.findByRole('button', { name: 'Import profile…' }));
    await waitFor(() => expect(useHardware.getState().profile?.source).toBe('imported'));
    expect(api.importProfile).toHaveBeenCalledWith('/tmp/pc.json');
  });
});

describe('Compatibility page', () => {
  it('re-checks on version change and requires the expert override for unconfirmed versions', async () => {
    const p = profile();
    useHardware.getState().setProfile(p, null);
    for (const s of ['welcome', 'scan', 'hardware'] as const) useWizard.getState().complete(s);
    const versions = [option('14'), option('15', true, { recommended: true }), option('26', false, { notes: ['AppleHDA was removed'] })];
    api.checkCompatibility.mockImplementation(async (_p, target) =>
      target === '26' ? report('26', 'partial', versions) : report('15', 'supported', versions),
    );

    renderEn(<Compatibility />);
    const next = await screen.findByRole('button', { name: 'Continue' });
    await waitFor(() => expect(next).toBeEnabled());
    expect(api.checkCompatibility).toHaveBeenCalledTimes(1);
    expect(api.checkCompatibility).toHaveBeenLastCalledWith(p, null);

    fireEvent.click(screen.getByRole('radio', { name: /Tahoe/ }));
    await waitFor(() => expect(api.checkCompatibility).toHaveBeenLastCalledWith(p, '26'));
    const expert = await screen.findByRole('checkbox', { name: 'I understand the risks and want to continue' });
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
    fireEvent.click(expert);
    expect(screen.getByRole('button', { name: 'Continue' })).toBeEnabled();
    expect(api.checkCompatibility).toHaveBeenCalledTimes(2);
  });

  it('does not loop when the check fails', async () => {
    useHardware.getState().setProfile(profile(), null);
    api.checkCompatibility.mockRejectedValue({ code: 'X', message: 'backend down' });
    renderEn(<Compatibility />);
    expect(await screen.findByText('backend down')).toBeInTheDocument();
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(api.checkCompatibility).toHaveBeenCalledTimes(1);
  });
});

describe('Settings', () => {
  beforeEach(() => {
    api.logGetSessionId.mockResolvedValue('session-123');
    api.logGetTail.mockResolvedValue('line one\nline two');
    api.saveSupportLog.mockResolvedValue(undefined);
    api.clearAppCache.mockResolvedValue(undefined);
    useApp.setState({
      settingsOpen: true,
      info: { version: '5.1.0', opencoreVersion: '1.0.8', hostOs: 'windows', arch: 'x86_64' },
      update: { current: '5.1.0', latest: '5.2.0', updateAvailable: true, url: 'https://github.com/redpersongpt/OpCore-OneClick/releases/tag/v5.2.0', notes: null },
    });
  });

  it('shows app info from get_app_info and the log tail', async () => {
    renderEn(<Settings />);
    expect(await screen.findByText('session-123')).toBeInTheDocument();
    expect(screen.getByText('v5.1.0')).toBeInTheDocument();
    expect(screen.getByText('1.0.8')).toBeInTheDocument();
    expect(screen.getByText(/line one/)).toBeInTheDocument();
    expect(api.logGetTail).toHaveBeenCalledWith(200);
  });

  it('opens the release page with the shell plugin', async () => {
    renderEn(<Settings />);
    fireEvent.click(await screen.findByRole('button', { name: 'Open the release page' }));
    await waitFor(() =>
      expect(shell.open).toHaveBeenCalledWith('https://github.com/redpersongpt/OpCore-OneClick/releases/tag/v5.2.0'),
    );
  });

  it('exports the support log and clears the cache, dropping the stale build', async () => {
    dialog.save.mockResolvedValue('/tmp/support.log');
    useBuild.setState({ result: buildResult() });
    renderEn(<Settings />);
    await screen.findByText('session-123');

    fireEvent.click(screen.getByRole('button', { name: 'Export support log…' }));
    await waitFor(() => expect(api.saveSupportLog).toHaveBeenCalledWith('/tmp/support.log'));

    fireEvent.click(screen.getByRole('button', { name: 'Clear downloads and builds' }));
    await waitFor(() => expect(api.clearAppCache).toHaveBeenCalled());
    await waitFor(() => expect(useBuild.getState().result).toBeNull());
  });

  it('disables cache clearing while an operation runs', async () => {
    useWizard.getState().lock('build');
    renderEn(<Settings />);
    expect(await screen.findByRole('button', { name: 'Clear downloads and builds' })).toBeDisabled();
  });

  it('switches the language', async () => {
    renderEn(<Settings />);
    fireEvent.change(await screen.findByLabelText('Language'), { target: { value: 'tr' } });
    expect(await screen.findByText('Ayarlar')).toBeInTheDocument();
    expect(useCompat.getState().target).toBeNull();
  });
});

describe('Build page', () => {
  function setup() {
    const p = profile();
    useHardware.getState().setProfile(p, null);
    useCompat.setState({ report: report('15'), reportKey: compatKey(p, '15'), requestKey: compatKey(p, '15'), target: '15' });
    for (const s of ['welcome', 'scan', 'hardware', 'compatibility', 'bios'] as const) useWizard.getState().complete(s);
    return p;
  }

  it('previews the plan before anything is built and builds with the chosen options', async () => {
    setup();
    let resolvePlan: (v: unknown) => void = () => undefined;
    api.planBuild.mockReturnValue(new Promise((r) => (resolvePlan = r)));
    api.buildEfi.mockResolvedValue(buildResult('15'));
    renderEn(<Build />);
    const start = screen.getByRole('button', { name: 'Download and build' });
    expect(start).toBeDisabled();
    await waitFor(() => expect(api.planBuild).toHaveBeenCalledTimes(1));
    await act(async () => resolvePlan(plan('15')));
    expect(await screen.findByText('iMac19,1')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Download and build' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Download and build' }));
    await waitFor(() => expect(useWizard.getState().step).toBe('review'));
    const [, options] = api.buildEfi.mock.calls[0];
    expect(options).toMatchObject({ target: '15', verbose: true, picker: 'graphical' });
  });

  it('shows the plan error and keeps the build disabled', async () => {
    setup();
    api.planBuild.mockRejectedValue({ code: 'PLAN_FAILED', message: 'No display path', suggestion: 'Keep a supported GPU enabled' });
    renderEn(<Build />);
    expect(await screen.findByText('No display path')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Download and build' })).toBeDisabled();
  });
});

describe('Deploy page', () => {
  const disk = (patch: Partial<DiskInfo>): DiskInfo => ({
    devicePath: '/dev/sdb',
    model: 'Ultra',
    vendor: 'SanDisk',
    serialNumber: null,
    sizeBytes: 32_000_000_000,
    sizeDisplay: '32.0 GB',
    transport: 'usb',
    removable: true,
    partitionTable: 'gpt',
    partitions: [],
    isSystemDisk: false,
    blockedReason: null,
    ...patch,
  });

  function setup(recoveryReady: boolean) {
    useHardware.getState().setProfile(profile(), null);
    useBuild.setState({ result: buildResult('15') });
    api.checkPrivileges.mockResolvedValue({ elevated: true, canElevate: true, detail: '' });
    api.listUsbDevices.mockResolvedValue([
      disk({ devicePath: '/dev/sda', model: 'System SSD', vendor: null, isSystemDisk: true, sizeDisplay: '512.1 GB', sizeBytes: 512_110_190_592 }),
      disk({}),
    ]);
    api.getCachedRecoveryInfo.mockResolvedValue(
      recoveryReady
        ? { available: true, version: '15', dmgPath: '/r/BaseSystem.dmg', chunklistPath: '/r/BaseSystem.chunklist', sizeBytes: 900_000_000, verified: true }
        : { available: false, version: null, dmgPath: null, chunklistPath: null, sizeBytes: null, verified: false },
    );
  }

  it('needs the recovery for the built target before writing', async () => {
    setup(false);
    renderEn(<Deploy />);
    expect(await screen.findByText('Download the recovery image first.')).toBeInTheDocument();
    expect(api.getCachedRecoveryInfo).toHaveBeenCalledWith('15');
    expect(screen.getByRole('button', { name: 'Write USB drive…' })).toBeDisabled();
  });

  it('disables system disks and asks to type the size before flashing', async () => {
    setup(true);
    api.flashPrepareConfirmation.mockResolvedValue({
      token: 'tok',
      device: '/dev/sdb',
      expiresAt: Date.now() + 300_000,
      diskDisplay: '/dev/sdb (SanDisk Ultra)',
      efiHash: 'h',
      recovery: '15',
    });
    api.flashUsb.mockResolvedValue(undefined);
    renderEn(<Deploy />);

    const system = await screen.findByRole('radio', { name: /System SSD/ });
    expect(system).toBeDisabled();
    fireEvent.click(screen.getByRole('radio', { name: /SanDisk Ultra/ }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Write USB drive…' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Write USB drive…' }));
    expect(api.flashPrepareConfirmation).toHaveBeenCalledWith('/dev/sdb', '/tmp/build/b1', '15');

    const input = await screen.findByLabelText(/type the drive size/);
    const erase = screen.getByRole('button', { name: 'Erase and write' });
    expect(erase).toBeDisabled();
    fireEvent.change(input, { target: { value: '32.0 GB' } });
    // The confirm button unlocks after a short cool-down.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Erase and write' })).toBeEnabled(), { timeout: 4000 });
    fireEvent.click(screen.getByRole('button', { name: 'Erase and write' }));
    await waitFor(() => expect(api.flashUsb).toHaveBeenCalledWith('/dev/sdb', '/tmp/build/b1', 'tok', '15'));
    await waitFor(() => expect(useDeploy.getState().flashStatus).toBe('done'));
    expect(await screen.findByText('USB drive ready')).toBeInTheDocument();
  });

  it('asks for the device name when two drives have the same size', async () => {
    setup(true);
    api.listUsbDevices.mockResolvedValue([disk({}), disk({ devicePath: '/dev/sdc', model: 'Cruzer' })]);
    api.flashPrepareConfirmation.mockResolvedValue({
      token: 'tok',
      device: '/dev/sdc',
      expiresAt: Date.now() + 300_000,
      diskDisplay: '/dev/sdc (SanDisk Cruzer)',
      efiHash: 'h',
      recovery: '15',
    });
    renderEn(<Deploy />);
    fireEvent.click(await screen.findByRole('radio', { name: /SanDisk Cruzer/ }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Write USB drive…' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Write USB drive…' }));
    const input = await screen.findByLabelText(/type the size and the device name/);
    fireEvent.change(input, { target: { value: '32.0 GB' } });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 2100));
    });
    expect(screen.getByRole('button', { name: 'Erase and write' })).toBeDisabled();
    fireEvent.change(input, { target: { value: '32.0 GB sdc' } });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Erase and write' })).toBeEnabled());
  });

  it('never shows a confirmation issued for another drive', async () => {
    setup(true);
    renderEn(<Deploy />);
    fireEvent.click(await screen.findByRole('radio', { name: /SanDisk Ultra/ }));
    act(() => {
      useDeploy.setState({
        confirmation: { token: 'tok', device: '/dev/sdz', expiresAt: Date.now() + 300_000, diskDisplay: 'x', efiHash: 'h', recovery: '15' },
      });
    });
    expect(screen.queryByRole('button', { name: 'Erase and write' })).toBeNull();
  });
});
