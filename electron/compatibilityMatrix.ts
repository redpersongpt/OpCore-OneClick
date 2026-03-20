import type { HardwareProfile } from './configGenerator.js';
import {
  checkCompatibility,
  type CompatibilityLevel,
  type CompatibilityReport,
} from './compatibility.js';
import { MACOS_VERSIONS } from './hackintoshRules.js';

export type CompatibilityMatrixStatus = 'supported' | 'experimental' | 'risky' | 'blocked';

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
    case 'experimental':
      return 'experimental';
    case 'risky':
      return 'risky';
    case 'blocked':
    default:
      return 'blocked';
  }
}

function summarizeCompatibilityReason(report: CompatibilityReport): string {
  if (report.errors.length > 0) {
    return report.errors[0];
  }
  if (report.level === 'experimental' || report.level === 'risky') {
    return report.warnings[0] ?? report.explanation;
  }
  if (report.communityEvidence?.summary) {
    return report.communityEvidence.summary;
  }
  if (report.manualVerificationRequired) {
    return report.explanation;
  }
  return report.explanation;
}

function choosePlanningRecommendation(
  rows: CompatibilityMatrixRow[],
  baselineRecommendedVersion: string,
): string {
  return baselineRecommendedVersion
    || rows.find((row) => row.status === 'supported' || row.status === 'experimental')?.versionName
    || rows.find((row) => row.status !== 'blocked')?.versionName
    || baselineRecommendedVersion;
}

export function buildCompatibilityMatrix(
  profile: HardwareProfile,
): CompatibilityMatrix {
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
  const recommendedVersion = choosePlanningRecommendation(rows, baseline.recommendedVersion);

  for (const row of rows) {
    row.recommended = row.versionName === recommendedVersion;
  }

  return {
    rows,
    recommendedVersion,
  };
}
