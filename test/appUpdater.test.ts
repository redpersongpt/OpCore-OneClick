import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import {
  compareReleaseVersions,
  isInstallerResidueEntryName,
  normalizeReleaseVersion,
  pickReleaseAssetForPlatform,
} from '../electron/appUpdater.js';

describe('appUpdater helpers', () => {
  test('normalizes prefixed release tags', () => {
    assert.equal(normalizeReleaseVersion('v2.3.9'), '2.3.9');
    assert.equal(normalizeReleaseVersion('V2.3.9-beta'), '2.3.9');
  });

  test('compares semantic versions numerically', () => {
    assert.equal(compareReleaseVersions('v2.3.9', '2.3.8') > 0, true);
    assert.equal(compareReleaseVersions('2.3.8', '2.3.8'), 0);
    assert.equal(compareReleaseVersions('2.3.8', '2.4.0') < 0, true);
  });

  test('picks platform-specific release assets', () => {
    const assets = [
      { name: 'macOS-OneClick.Setup.2.3.9.exe', browser_download_url: 'https://example.com/win.exe' },
      { name: 'macOS-OneClick-2.3.9.AppImage', browser_download_url: 'https://example.com/linux.AppImage' },
      { name: 'macos-one-click_2.3.9_amd64.deb', browser_download_url: 'https://example.com/linux.deb' },
    ];

    assert.equal(pickReleaseAssetForPlatform('win32', assets)?.name, 'macOS-OneClick.Setup.2.3.9.exe');
    assert.equal(pickReleaseAssetForPlatform('linux', assets)?.name, 'macOS-OneClick-2.3.9.AppImage');
    assert.equal(pickReleaseAssetForPlatform('darwin', assets), null);
  });

  test('detects updater residue entries in user data', () => {
    assert.equal(isInstallerResidueEntryName('macOSInstaller'), true);
    assert.equal(isInstallerResidueEntryName('macossinstaller'), true);
    assert.equal(isInstallerResidueEntryName('installer-cache'), true);
    assert.equal(isInstallerResidueEntryName('hardware-profiles'), false);
  });
});
