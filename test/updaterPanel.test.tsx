import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import React from 'react';
import UpdaterPanel from '../src/components/UpdaterPanel';
import type { AppUpdateState } from '../electron/appUpdater';

function baseState(overrides: Partial<AppUpdateState> = {}): AppUpdateState {
  return {
    currentVersion: '2.4.4',
    checking: false,
    downloading: false,
    installing: false,
    lastCheckedAt: Date.now() - 30_000,
    available: false,
    supported: true,
    latestVersion: null,
    releaseUrl: null,
    releaseNotes: null,
    assetName: null,
    assetSize: null,
    downloadedBytes: 0,
    totalBytes: null,
    downloadedPath: null,
    readyToInstall: false,
    restartRequired: false,
    error: null,
    ...overrides,
  };
}

const noop = vi.fn();

describe('UpdaterPanel – action visibility across states', () => {
  it('available update: shows Download button and Release link', () => {
    render(
      <UpdaterPanel
        state={baseState({ available: true, latestVersion: '2.5.0', assetName: 'OpCore-OneClick-2.5.0.dmg' })}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    expect(screen.getByText('Download update')).toBeInTheDocument();
    expect(screen.getByText('Release')).toBeInTheDocument();
    expect(screen.getByText('2.5.0 available')).toBeInTheDocument();
  });

  it('downloading: shows Downloading button and progress bar', () => {
    const { container } = render(
      <UpdaterPanel
        state={baseState({ available: true, downloading: true, downloadedBytes: 500, totalBytes: 1000 })}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    // "Downloading…" appears in both the headline and the disabled button — both visible
    const downloadingEls = screen.getAllByText('Downloading…');
    expect(downloadingEls.length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('50%')).toBeInTheDocument();
    // Progress bar should be visible
    const progressBar = container.querySelector('[style*="width: 50%"]');
    expect(progressBar).toBeTruthy();
  });

  it('ready to install: shows Install button', () => {
    render(
      <UpdaterPanel
        state={baseState({ available: true, readyToInstall: true, downloadedBytes: 1000, totalBytes: 1000 })}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    expect(screen.getByText('Install update')).toBeInTheDocument();
    expect(screen.getByText('100%')).toBeInTheDocument();
  });

  it('restart required: shows Restart button', () => {
    render(
      <UpdaterPanel
        state={baseState({ restartRequired: true, latestVersion: '2.5.0' })}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    expect(screen.getByText('Restart to update')).toBeInTheDocument();
    expect(screen.getByText('Restart to finish updating')).toBeInTheDocument();
  });

  it('error state: shows error message and Release link', () => {
    render(
      <UpdaterPanel
        state={baseState({ available: true, error: 'Network timeout' })}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    expect(screen.getByText('Network timeout')).toBeInTheDocument();
    expect(screen.getByText('Release')).toBeInTheDocument();
  });

  it('up to date: no primary action button visible', () => {
    render(
      <UpdaterPanel
        state={baseState()}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    expect(screen.getByText('Up to date')).toBeInTheDocument();
    expect(screen.queryByText('Download update')).toBeNull();
    expect(screen.queryByText('Install update')).toBeNull();
    expect(screen.queryByText('Restart to update')).toBeNull();
  });

  it('panel is compact – no min-height constraint', () => {
    const { container } = render(
      <UpdaterPanel
        state={baseState({ available: true, downloading: true, downloadedBytes: 250, totalBytes: 1000 })}
        onRefresh={noop}
        onPrimaryAction={noop}
        onOpenRelease={noop}
      />,
    );
    const panel = container.firstChild as HTMLElement;
    // Verify no min-height inline style or class
    expect(panel.style.minHeight).toBeFalsy();
    expect(panel.className).not.toMatch(/min-h-/);
  });
});
