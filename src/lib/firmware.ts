import type { BiosSetting, FirmwareCheck, FirmwareReport } from '../bridge/types';

export type CheckState = 'ok' | 'action' | 'inferred' | 'unknown' | 'na';

/**
 * The firmware probe reports either the contract vocabulary
 * ("ok" | "action" | "unknown" | "not_applicable") or the older evidence
 * vocabulary ("confirmed" | "failing" | "inferred" | "unverified").
 */
export function checkState(check: FirmwareCheck): CheckState {
  switch (check.status) {
    case 'ok':
    case 'confirmed':
      return 'ok';
    case 'action':
    case 'failing':
      return 'action';
    case 'inferred':
      return 'inferred';
    case 'not_applicable':
      return 'na';
    default:
      return 'unknown';
  }
}

export function firmwareChecks(report: FirmwareReport): FirmwareCheck[] {
  return [report.uefiMode, report.secureBoot, report.vtX, report.vtD, report.above4g];
}

const MATCHERS: { pattern: RegExp; pick: (r: FirmwareReport) => FirmwareCheck }[] = [
  { pattern: /secure\s*boot/i, pick: (r) => r.secureBoot },
  { pattern: /vt-?d|iommu|amd-?vi/i, pick: (r) => r.vtD },
  { pattern: /vt-?x|svm|virtuali[sz]ation|amd-?v\b/i, pick: (r) => r.vtX },
  { pattern: /above\s*4g/i, pick: (r) => r.above4g },
  // Not a bare "legacy": "Legacy USB Support" has nothing to do with the boot mode.
  { pattern: /\bcsm\b|compatibility support|legacy boot|boot mode|\buefi\b/i, pick: (r) => r.uefiMode },
];

/** The probe result that corresponds to a checklist item, if any. */
export function probeFor(setting: BiosSetting, report: FirmwareReport | null): FirmwareCheck | null {
  if (!report) return null;
  for (const m of MATCHERS) {
    if (m.pattern.test(setting.name)) return m.pick(report);
  }
  return null;
}
