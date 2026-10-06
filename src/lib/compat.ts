import type { CompatibilityReport, MacOsOption, MacOsVersion, PlanNote } from '../bridge/types';
import { compareMacos } from './macos';

/**
 * - `ok`: the selected release is supported for this hardware.
 * - `expert`: support is only partial or unconfirmed (release not marked as
 *   supported, a blocking note, or a contradictory overall verdict); the user
 *   may continue only after an explicit expert override.
 * - `blocked`: this hardware cannot run the selected release.
 */
export type CompatGate = 'ok' | 'expert' | 'blocked';

export function findOption(report: CompatibilityReport | null, version: MacOsVersion | null): MacOsOption | null {
  if (!report || !version) return null;
  return report.versions.find((v) => v.version === version) ?? null;
}

function hasBlockingNotes(notes: readonly PlanNote[]): boolean {
  return notes.some((n) => n.level === 'blocking');
}

/** Gate for continuing with `version`, based on a report evaluated for that version. */
export function compatGate(report: CompatibilityReport | null, version: MacOsVersion | null): CompatGate {
  const option = findOption(report, version);
  if (!report || !option) return 'blocked';
  if (option.supported) {
    return hasBlockingNotes(report.notes) || report.level === 'unsupported' ? 'expert' : 'ok';
  }
  return report.level === 'unsupported' ? 'blocked' : 'expert';
}

/** Target to preselect when the user has not chosen one. */
export function defaultTarget(report: CompatibilityReport): MacOsVersion | null {
  if (report.target && report.versions.some((v) => v.version === report.target)) return report.target;
  if (report.recommended) return report.recommended;
  const supported = report.versions.filter((v) => v.supported).map((v) => v.version);
  supported.sort(compareMacos);
  return supported.length > 0 ? supported[supported.length - 1] : null;
}

/** Versions newest first, as shown in the picker. */
export function sortedVersions(report: CompatibilityReport): MacOsOption[] {
  return [...report.versions].sort((a, b) => compareMacos(b.version, a.version));
}
