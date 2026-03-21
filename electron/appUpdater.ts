export interface ReleaseAssetInfo {
  name: string;
  browser_download_url: string;
  size?: number;
}

export interface LatestReleaseInfo {
  tag_name: string;
  html_url: string;
  body?: string;
  assets?: ReleaseAssetInfo[];
}

export interface AppUpdateState {
  currentVersion: string;
  checking: boolean;
  downloading: boolean;
  installing: boolean;
  lastCheckedAt: number | null;
  available: boolean;
  supported: boolean;
  latestVersion: string | null;
  releaseUrl: string | null;
  releaseNotes: string | null;
  assetName: string | null;
  assetSize: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
  downloadedPath: string | null;
  readyToInstall: boolean;
  restartRequired: boolean;
  error: string | null;
}

export function normalizeReleaseVersion(version: string): string {
  return version.trim().replace(/^v/i, '').split('-')[0];
}

export function compareReleaseVersions(a: string, b: string): number {
  const left = normalizeReleaseVersion(a).split('.').map((part) => parseInt(part, 10) || 0);
  const right = normalizeReleaseVersion(b).split('.').map((part) => parseInt(part, 10) || 0);
  const max = Math.max(left.length, right.length);
  for (let index = 0; index < max; index += 1) {
    const delta = (left[index] ?? 0) - (right[index] ?? 0);
    if (delta !== 0) return delta;
  }
  return 0;
}

export function pickReleaseAssetForPlatform(
  platform: NodeJS.Platform,
  assets: ReleaseAssetInfo[],
): ReleaseAssetInfo | null {
  if (platform === 'win32') {
    return assets.find((asset) => asset.name.toLowerCase().endsWith('.exe')) ?? null;
  }
  if (platform === 'linux') {
    return assets.find((asset) => asset.name.endsWith('.AppImage'))
      ?? assets.find((asset) => asset.name.endsWith('.deb'))
      ?? null;
  }
  return null;
}

export function isInstallerResidueEntryName(name: string): boolean {
  const normalized = name.trim().toLowerCase();
  return /macos[-_. ]?installer/.test(normalized)
    || /macossinstaller/.test(normalized)
    || normalized === 'installer'
    || normalized === 'installer-cache';
}
