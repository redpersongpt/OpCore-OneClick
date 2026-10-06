import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildResult, plan, profile, report } from './fixtures';

const api = vi.hoisted(() => ({
  scanHardware: vi.fn(),
  importProfile: vi.fn(),
  checkCompatibility: vi.fn(),
  planBuild: vi.fn(),
  buildEfi: vi.fn(),
  taskCancel: vi.fn(),
}));
vi.mock('../bridge/api', () => ({ api }));

const win = vi.hoisted(() => {
  const state: { handler: ((event: { preventDefault: () => void }) => void) | null } = { handler: null };
  const window = {
    onCloseRequested: vi.fn(async (handler: (event: { preventDefault: () => void }) => void) => {
      state.handler = handler;
      return () => {
        state.handler = null;
      };
    }),
    close: vi.fn(async () => undefined),
    destroy: vi.fn(async () => undefined),
    minimize: vi.fn(async () => undefined),
    toggleMaximize: vi.fn(async () => undefined),
  };
  return { state, window };
});
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => win.window }));

import CloseConfirmDialog from '../components/layout/CloseConfirmDialog';
import Header from '../components/layout/Header';
import TaskBar from '../components/layout/TaskBar';
import { closeRisk, requestClose, useCloseGuard } from '../hooks/useCloseGuard';
import { I18nProvider } from '../i18n';
import {
  BUILD_PHASES,
  itemDone,
  parseBuildDetail,
  phaseStates,
  recordItem,
  type BuildItems,
} from '../lib/buildProgress';
import Build from '../pages/Build';
import { useApp } from '../stores/app';
import { toBuildOptions, useBuild } from '../stores/build';
import { compatKey, useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import { resetAll } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { routeTaskUpdate } from '../stores/routing';
import { isCancellable, useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

function renderEn(ui: ReactElement) {
  return render(<I18nProvider initial="en">{ui}</I18nProvider>);
}

const buildUpdate = (detail: unknown, progress = 0.5, status: 'running' | 'completed' = 'running') => ({
  taskId: 'b1',
  kind: 'efi-build',
  status,
  progress,
  message: 'Downloading Lilu (0.1 MB of 0.3 MB)',
  detail,
});

beforeEach(() => {
  resetAll();
  Object.values(api).forEach((fn) => fn.mockReset());
  win.window.close.mockClear();
  win.window.destroy.mockClear();
  win.state.handler = null;
  useApp.setState({ closeConfirmOpen: false });
});

describe('build progress detail', () => {
  it('parses the efi-build detail and rejects malformed payloads', () => {
    expect(parseBuildDetail({ phase: 'kexts', step: 5, total: 10, item: 'Lilu', index: 2, count: 14, downloaded: 1000, size: 4000 })).toEqual({
      phase: 'kexts',
      step: 5,
      total: 10,
      item: 'Lilu',
      index: 2,
      count: 14,
      downloaded: 1000,
      size: 4000,
    });
    expect(parseBuildDetail({ phase: 'config', step: 8, total: 10 })).toMatchObject({ phase: 'config', item: null, index: null, size: null });
    expect(parseBuildDetail({ phase: 'download-everything' })).toBeNull();
    expect(parseBuildDetail(null)).toBeNull();
    expect(parseBuildDetail('kexts')).toBeNull();
    // Byte counts without an item, or a size of 0, carry no meaning.
    expect(parseBuildDetail({ phase: 'kexts', downloaded: 5, size: 0 })).toMatchObject({ downloaded: null, size: null });
    expect(parseBuildDetail({ phase: 'kexts', item: 'Lilu', downloaded: 5, size: 0 })).toMatchObject({ downloaded: 5, size: null });
  });

  it('records items with their newest byte counts', () => {
    let items: BuildItems = {};
    items = recordItem(items, parseBuildDetail({ phase: 'kexts', item: 'Lilu', index: 1, count: 2 })!);
    items = recordItem(items, parseBuildDetail({ phase: 'kexts', item: 'Lilu', index: 1, count: 2, downloaded: 10, size: 20 })!);
    items = recordItem(items, parseBuildDetail({ phase: 'kexts', item: 'WhateverGreen', index: 2, count: 2 })!);
    items = recordItem(items, parseBuildDetail({ phase: 'acpi' })!);
    expect(items.kexts).toEqual([
      { name: 'Lilu', index: 1, count: 2, downloaded: 10, size: 20 },
      { name: 'WhateverGreen', index: 2, count: 2, downloaded: null, size: null },
    ]);
    expect(items.acpi).toBeUndefined();
    const current = parseBuildDetail({ phase: 'kexts', item: 'WhateverGreen', index: 2, count: 2 });
    expect(itemDone(items.kexts![0], current, 'kexts')).toBe(true);
    expect(itemDone(items.kexts![1], current, 'kexts')).toBe(false);
    expect(itemDone(items.kexts![1], parseBuildDetail({ phase: 'config' }), 'kexts')).toBe(true);
  });

  it('marks phases the build passed without reporting as skipped', () => {
    const seen = ['plan', 'opencore', 'assemble', 'kexts'] as const;
    const states = phaseStates('kexts', seen, 'running');
    expect(states.slice(0, 5)).toEqual(['done', 'done', 'skipped', 'done', 'active']);
    expect(states.slice(5)).toEqual(['pending', 'pending', 'pending', 'pending', 'pending']);
    expect(phaseStates('kexts', seen, 'failed')[4]).toBe('failed');
    expect(phaseStates('validate', [...BUILD_PHASES], 'done').every((s) => s === 'done')).toBe(true);
  });

  it('a build being saved or validated can no longer be cancelled', () => {
    expect(isCancellable(buildUpdate({ phase: 'kexts' }))).toBe(true);
    expect(isCancellable(buildUpdate({ phase: 'save' }))).toBe(false);
    expect(isCancellable(buildUpdate({ phase: 'validate' }))).toBe(false);
    expect(isCancellable({ ...buildUpdate(null), kind: 'usb-flash' })).toBe(false);
    expect(isCancellable({ ...buildUpdate(null), kind: 'hardware-scan' })).toBe(true);
    expect(isCancellable(buildUpdate({ phase: 'kexts' }, 1, 'completed'))).toBe(false);
  });
});

describe('Build page progress', () => {
  function setup() {
    const p = profile();
    useHardware.getState().setProfile(p, null);
    useCompat.setState({ report: report('15'), reportKey: compatKey(p, '15'), requestKey: compatKey(p, '15'), target: '15' });
    for (const s of ['welcome', 'scan', 'hardware', 'compatibility', 'bios'] as const) useWizard.getState().complete(s);
    api.planBuild.mockResolvedValue(plan('15'));
    api.checkCompatibility.mockResolvedValue(report('15'));
  }

  it('shows the phase list with per-kext progress and download sizes', async () => {
    setup();
    let finish: (v: unknown) => void = () => undefined;
    api.buildEfi.mockReturnValue(new Promise((r) => (finish = r)));
    renderEn(<Build />);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Download and build' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Download and build' }));

    act(() => {
      routeTaskUpdate(buildUpdate({ phase: 'plan', step: 1, total: 10 }, 0));
      routeTaskUpdate(buildUpdate({ phase: 'opencore', step: 2, total: 10, item: 'OpenCore', index: 1, count: 1, downloaded: 2_500_000, size: 2_500_000 }, 0.25));
      routeTaskUpdate(buildUpdate({ phase: 'kexts', step: 5, total: 10, item: 'Lilu', index: 1, count: 3, downloaded: 300_000, size: 300_000 }, 0.5));
      routeTaskUpdate(buildUpdate({ phase: 'kexts', step: 5, total: 10, item: 'WhateverGreen', index: 2, count: 3, downloaded: 400_000, size: 1_200_000 }, 0.55));
    });

    expect(await screen.findByText('Step 5 of 10: Kexts')).toBeInTheDocument();
    const steps = screen.getByRole('list', { name: 'Build steps' });
    expect(within(steps).getByText('Boot picker resources')).toBeInTheDocument();
    expect(within(steps).getByText('2 of 3')).toBeInTheDocument();
    // A single download (the OpenCore package) shows its size next to the phase.
    expect(within(steps).getByText('2.5 MB')).toBeInTheDocument();
    expect(within(steps).getByText('Lilu')).toBeInTheDocument();
    expect(within(steps).getByText('300 KB')).toBeInTheDocument();
    expect(within(steps).getByText('400 KB / 1.2 MB')).toBeInTheDocument();
    expect(screen.getByText('Downloading WhateverGreen')).toBeInTheDocument();
    expect(screen.getByText('400 KB of 1.2 MB')).toBeInTheDocument();
    // The picker resources phase was never reported: it is shown as not needed.
    expect(within(steps).getAllByText('not needed').length).toBeGreaterThan(0);

    finish(buildResult('15'));
    await waitFor(() => expect(useWizard.getState().step).toBe('review'));
  });

  it('cancels through task_cancel and stops offering it once the build is being saved', async () => {
    setup();
    api.buildEfi.mockReturnValue(new Promise(() => undefined));
    api.taskCancel.mockResolvedValue(true);
    renderEn(<Build />);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Download and build' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Download and build' }));
    act(() => routeTaskUpdate(buildUpdate({ phase: 'save', step: 9, total: 10 }, 0.9)));
    const section = await screen.findByText('Step 9 of 10: Saving');
    expect(section).toBeInTheDocument();
    const cancel = screen.getAllByRole('button', { name: 'Cancel' });
    cancel.forEach((button) => expect(button).toBeDisabled());

    act(() => routeTaskUpdate(buildUpdate({ phase: 'kexts', step: 5, total: 10 }, 0.5)));
    const enabled = screen.getAllByRole('button', { name: 'Cancel' }).find((b) => !(b as HTMLButtonElement).disabled);
    expect(enabled).toBeDefined();
    fireEvent.click(enabled!);
    await waitFor(() => expect(api.taskCancel).toHaveBeenCalledWith('b1'));
  });

  it('explains when the backend refuses the cancel', async () => {
    setup();
    api.buildEfi.mockReturnValue(new Promise(() => undefined));
    api.taskCancel.mockResolvedValue(false);
    void useBuild.getState().build(profile(), toBuildOptions(useBuild.getState().draft, '15', null));
    act(() => routeTaskUpdate(buildUpdate({ phase: 'config', step: 8, total: 10 }, 0.85)));
    await useBuild.getState().cancel();
    expect(useBuild.getState()).toMatchObject({ cancelled: false, cancelRefused: true });
    expect(useTasks.getState().cancels.b1).toBe('refused');
    renderEn(<Build />);
    expect(await screen.findByText(/can no longer be cancelled/)).toBeInTheDocument();
  });
});

describe('cancel from the task bar during a build', () => {
  it('the build page follows a cancel sent from the task bar', async () => {
    const p = profile();
    useHardware.getState().setProfile(p, null);
    useCompat.setState({ report: report('15'), reportKey: compatKey(p, '15'), requestKey: compatKey(p, '15'), target: '15' });
    for (const s of ['welcome', 'scan', 'hardware', 'compatibility', 'bios'] as const) useWizard.getState().complete(s);
    api.planBuild.mockResolvedValue(plan('15'));
    api.buildEfi.mockReturnValue(new Promise(() => undefined));
    api.taskCancel.mockResolvedValue(true);
    void useBuild.getState().build(p, toBuildOptions(useBuild.getState().draft, '15', null));
    act(() => routeTaskUpdate(buildUpdate({ phase: 'kexts', step: 5, total: 10 }, 0.5)));
    await useTasks.getState().cancel('b1');
    renderEn(<Build />);
    const button = await screen.findByRole('button', { name: 'Cancelling…' });
    expect(button).toBeDisabled();
    // A second request from the build page is not sent, and nothing claims the cancel was refused.
    await useBuild.getState().cancel();
    expect(api.taskCancel).toHaveBeenCalledTimes(1);
    expect(useBuild.getState().cancelRefused).toBe(false);
    expect(screen.queryByText(/can no longer be cancelled/)).toBeNull();
  });
});

describe('task bar', () => {
  it('shows the localized build phase and the current kext', () => {
    routeTaskUpdate(buildUpdate({ phase: 'kexts', step: 5, total: 10, item: 'Lilu', index: 3, count: 12 }, 0.5));
    renderEn(<TaskBar />);
    expect(screen.getByText(/Kexts: Lilu \(3 of 12\)/)).toBeInTheDocument();
    expect(screen.getByText('50%')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  });

  it('shows USB write progress without a cancel button', () => {
    useDeploy.setState({ flashProgress: { taskId: 'f1', phase: 'copy-recovery', progress: 0.6, message: 'Copying', error: null } });
    routeTaskUpdate({ taskId: 'f1', kind: 'usb-flash', status: 'running', progress: 0.6, message: 'Copying the recovery image', detail: null });
    renderEn(<TaskBar />);
    expect(screen.getByText('Writing USB drive')).toBeInTheDocument();
    expect(screen.getByText(/Copy the recovery image/)).toBeInTheDocument();
    expect(screen.getByRole('progressbar', { name: 'Writing USB drive' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
  });

  it('cancels a scan from the task bar and disables the button afterwards', async () => {
    api.taskCancel.mockResolvedValue(true);
    routeTaskUpdate({ taskId: 's1', kind: 'hardware-scan', status: 'running', progress: 0.3, message: 'Reading the hardware', detail: null });
    renderEn(<TaskBar />);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(api.taskCancel).toHaveBeenCalledWith('s1'));
    const pending = screen.getByRole('button', { name: 'Cancelling…' });
    expect(pending).toBeDisabled();
    // The cancel is on its way; nothing claims the step cannot be cancelled.
    expect(pending).not.toHaveAttribute('title');
  });
});

describe('closing the window', () => {
  function Guard() {
    useCloseGuard();
    return null;
  }

  it('asks before closing while an operation runs, also for a system close', async () => {
    renderEn(
      <>
        <Guard />
        <Header />
        <CloseConfirmDialog />
      </>,
    );
    await waitFor(() => expect(win.state.handler).not.toBeNull());

    // Nothing running: a close request goes through.
    const idle = { preventDefault: vi.fn() };
    win.state.handler!(idle);
    expect(idle.preventDefault).not.toHaveBeenCalled();
    expect(closeRisk()).toBeNull();

    // A USB write runs: Alt+F4 / taskbar close is held back and the dialog explains why.
    act(() => useWizard.getState().lock('flash'));
    const busy = { preventDefault: vi.fn() };
    act(() => win.state.handler!(busy));
    expect(busy.preventDefault).toHaveBeenCalled();
    expect(await screen.findByText(/The USB drive is being written/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Keep the app open' }));
    expect(useApp.getState().closeConfirmOpen).toBe(false);

    // The title bar button asks too; "Close anyway" destroys the window.
    fireEvent.click(screen.getByRole('button', { name: 'Close window' }));
    expect(win.window.close).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole('button', { name: 'Close anyway' }));
    expect(win.window.destroy).toHaveBeenCalled();
  });

  it('a running backend task also counts, and an idle app closes directly', () => {
    routeTaskUpdate({ taskId: 'r1', kind: 'recovery-download', status: 'running', progress: 0.1, message: null, detail: null });
    expect(closeRisk()).toBe('busy');
    requestClose();
    expect(useApp.getState().closeConfirmOpen).toBe(true);
    useTasks.getState().clear();
    useApp.setState({ closeConfirmOpen: false });
    requestClose();
    expect(win.window.close).toHaveBeenCalled();
  });

  it('the window capability allows destroy, which a guarded close needs', () => {
    const capability = JSON.parse(
      readFileSync(join(__dirname, '..', '..', 'src-tauri', 'capabilities', 'default.json'), 'utf8'),
    ) as { permissions: string[] };
    expect(capability.permissions).toEqual(expect.arrayContaining(['core:window:allow-destroy', 'core:window:allow-close']));
  });
});
