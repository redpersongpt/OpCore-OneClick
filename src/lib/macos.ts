import type { MacOsVersion } from '../bridge/types';

/** Oldest first. macOS 26 Tahoe is the last release for Intel/AMD machines. */
export const MACOS_ORDER: readonly MacOsVersion[] = ['10.13', '10.14', '10.15', '11', '12', '13', '14', '15', '26'];

export const MACOS_NAMES: Record<MacOsVersion, string> = {
  '10.13': 'High Sierra',
  '10.14': 'Mojave',
  '10.15': 'Catalina',
  '11': 'Big Sur',
  '12': 'Monterey',
  '13': 'Ventura',
  '14': 'Sonoma',
  '15': 'Sequoia',
  '26': 'Tahoe',
};

/** "Sequoia" */
export function macosName(version: MacOsVersion): string {
  return MACOS_NAMES[version];
}

/** "macOS Sequoia 15" */
export function macosLabel(version: MacOsVersion): string {
  return `macOS ${MACOS_NAMES[version]} ${version}`;
}

export function compareMacos(a: MacOsVersion, b: MacOsVersion): number {
  return MACOS_ORDER.indexOf(a) - MACOS_ORDER.indexOf(b);
}
