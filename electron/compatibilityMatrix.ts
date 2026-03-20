import type { HardwareProfile } from './configGenerator.js';
import {
  checkCompatibility,
  type CompatibilityLevel,
  type CompatibilityReport,
} from './compatibility.js';
import { MACOS_VERSIONS } from './hackintoshRules.js';

export type CompatibilityMatrixStatus = 'supported' | 'partial' | 'blocked';

export interface CompatibilityMatrixRow {
  versionId: string;
  versionName: string;
  icon: string;
  numeric: number;
  status: CompatibilityMatrixStatus;
  reason: string;
  recommended: boolean;
  reportLevel: CompatibilityLevel;
}

export interface CompatibilityMatrix {
  rows: CompatibilityMatrixRow[];
  recommendedVersion: string;
}

export function classifyCompatibilityMatrixStatus(report: CompatibilityReport): CompatibilityMatrixStatus {
  switch (report.level) {
    case 'supported':
      return 'supported';
    case 'supported_with_warnings':
    case 'partial_support':
    case 'low_confidence':
      return 'partial';
    case 'blocked':
    default:
      return 'blocked';
  }
}

function summarizeCompatibilityReason(report: CompatibilityReport): string {
  if (report.errors.length > 0) {
    return report.errors[0];
  }
  if (report.level === 'supported_with_warnings' || report.level === 'partial_support') {
    return report.warnings[0] ?? report.explanation;
  }
  if (report.level === 'low_confidence') {
    return report.explanation;
  }
  return report.explanation;
}

export function buildCompatibilityMatrix(profile: HardwareProfile): CompatibilityMatrix {
  const baseline = checkCompatibility(profile);
  const rows = MACOS_VERSIONS.map((version) => {
    const report = checkCompatibility({
      ...profile,
      targetOS: version.name,
    });

    return {
      versionId: version.id,
      versionName: version.name,
      icon: version.icon,
      numeric: version.numeric,
      status: classifyCompatibilityMatrixStatus(report),
      reason: summarizeCompatibilityReason(report),
      recommended: version.name === baseline.recommendedVersion,
      reportLevel: report.level,
    } satisfies CompatibilityMatrixRow;
  });

  return {
    rows,
    recommendedVersion: baseline.recommendedVersion,
  };
}
