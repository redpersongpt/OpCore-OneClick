import React from 'react';
import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { AppUpdateState } from '../electron/appUpdater.js';
import UpdaterPanel from '../src/components/UpdaterPanel';

function makeState(overrides: Partial<AppUpdateState> = {}): AppUpdateState {
  return {
    currentVersion: '2.4.3',
    checking: false,
    downloading: false,
    installing: false,
    lastCheckedAt: 1_710_000_000_000,
    available: false,
    supported: true,
    latestVersion: null,
    releaseUrl: 'https://github.com/redpersongpt/macOS-One-Click/releases/latest',
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

describe('UpdaterPanel', () => {
  test('shows an unmistakable refreshing state when refresh is in progress', () => {
    const html = renderToStaticMarkup(
      <UpdaterPanel
        state={makeState({ checking: true })}
        onRefresh={() => {}}
        onPrimaryAction={() => {}}
        onOpenRelease={() => {}}
      />,
    );

    assert.match(html, /Refreshing update status/);
    assert.match(html, /Refreshing now/);
    assert.match(html, /Refreshing…/);
    assert.match(html, /animate-spin/);
  });

  test('uses stacked responsive actions that stay within the container on narrow widths', () => {
    const html = renderToStaticMarkup(
      <UpdaterPanel
        state={makeState({
          available: true,
          latestVersion: 'v2.4.3',
          assetName: 'macOS-OneClick.Setup.2.4.3.exe',
        })}
        onRefresh={() => {}}
        onPrimaryAction={() => {}}
        onOpenRelease={() => {}}
      />,
    );

    assert.match(html, /mt-4 flex flex-col gap-3 sm:flex-row/);
    assert.match(html, /w-full min-w-0 items-center justify-center/);
    assert.match(html, /sm:flex-1/);
    assert.match(html, /break-words text-center leading-tight/);
  });

  test('keeps long status and asset text wrapped during an in-progress update', () => {
    const html = renderToStaticMarkup(
      <UpdaterPanel
        state={makeState({
          downloading: true,
          available: true,
          latestVersion: 'v2.4.3',
          assetName: 'macOS-OneClick.Setup.2.4.3-super-long-build-name-for-small-windows.exe',
          downloadedBytes: 512,
          totalBytes: 1024,
        })}
        onRefresh={() => {}}
        onPrimaryAction={() => {}}
        onOpenRelease={() => {}}
      />,
    );

    assert.match(html, /Downloading update…/);
    assert.match(html, /Asset: macOS-OneClick\.Setup\.2\.4\.3-super-long-build-name-for-small-windows\.exe/);
    assert.match(html, /break-words/);
    assert.match(html, /50%/);
  });

  test('switches to restart-required copy without reviving the old update-ready state', () => {
    const html = renderToStaticMarkup(
      <UpdaterPanel
        state={makeState({
          latestVersion: 'v2.4.3',
          readyToInstall: false,
          restartRequired: true,
        })}
        onRefresh={() => {}}
        onPrimaryAction={() => {}}
        onOpenRelease={() => {}}
      />,
    );

    assert.match(html, /Set up complete - please restart the app/);
    assert.match(html, /Restart app/);
    assert.doesNotMatch(html, /Update ready to finish/);
  });
});
