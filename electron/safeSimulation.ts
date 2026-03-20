import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { HardwareProfile } from './configGenerator.js';
import { getRequiredResources } from './configGenerator.js';
import type { ValidationResult } from './configValidator.js';
import type { CompatibilityMatrix } from './compatibilityMatrix.js';
import type { BuildPlan, RecoveryDryRun } from './deterministicLayer.js';
import { buildResourcePlan, type ResourcePlan } from './resourcePlanner.js';

export const SAFE_SIMULATION_DISCLAIMER =
  'Simulation validates build inputs and current checks only. It is not a boot guarantee.';

export interface SafeSimulationResult {
  workspacePath: string;
  efiSummary: {
    efiPath: string;
    configHash: string | null;
    requiredResources: Array<{
      name: string;
      kind: 'kext' | 'ssdt' | 'driver';
      sourceClass: ResourcePlan['resources'][number]['sourceClass'];
    }>;
  };
  validationSummary: {
    overall: 'pass' | 'warning' | 'blocked';
    issues: Array<{
      code: string;
      severity: 'warning' | 'blocked';
      component: string;
      message: string;
    }>;
  };
  compatibilityMatrixSnapshot: CompatibilityMatrix['rows'];
  recoveryReadiness: {
    certainty: BuildPlan['certainty'];
    recommendation: string;
  };
  resourcePlan: ResourcePlan;
  blockers: string[];
  warnings: string[];
  disclaimer: typeof SAFE_SIMULATION_DISCLAIMER;
}

export interface SafeSimulationDependencies {
  userDataPath: string;
  createEfiWorkspace: (workspacePath: string, profile: HardwareProfile) => Promise<void>;
  validateEfi: (efiPath: string, profile: HardwareProfile) => Promise<ValidationResult>;
  buildCompatibilityMatrix: (profile: HardwareProfile) => CompatibilityMatrix;
  simulateBuild: (profile: HardwareProfile) => Promise<BuildPlan>;
  dryRunRecovery: (targetOS: string, smbios: string) => Promise<RecoveryDryRun>;
  kextRegistry: Record<string, { repo: string; assetFilter?: string }>;
  now?: () => number;
}

const SAFE_SIMULATION_ROOT = 'safe-simulations';
const SAFE_SIMULATION_MAX_AGE_MS = 24 * 60 * 60 * 1000;

function pushUnique(target: string[], value: string | null | undefined): void {
  const normalized = value?.trim();
  if (!normalized || target.includes(normalized)) return;
  target.push(normalized);
}

function hashFileIfPresent(filePath: string): string | null {
  if (!fs.existsSync(filePath)) return null;
  const hash = crypto.createHash('sha256');
  hash.update(fs.readFileSync(filePath));
  return hash.digest('hex');
}

export function cleanupExpiredSafeSimulationWorkspaces(
  rootPath: string,
  now = Date.now(),
  maxAgeMs = SAFE_SIMULATION_MAX_AGE_MS,
): void {
  if (!fs.existsSync(rootPath)) return;

  for (const entry of fs.readdirSync(rootPath, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const entryPath = path.resolve(rootPath, entry.name);
    try {
      const stat = fs.statSync(entryPath);
      if (now - stat.mtimeMs > maxAgeMs) {
        fs.rmSync(entryPath, { recursive: true, force: true });
      }
    } catch {
      fs.rmSync(entryPath, { recursive: true, force: true });
    }
  }
}

export async function runSafeSimulation(
  profile: HardwareProfile,
  deps: SafeSimulationDependencies,
): Promise<SafeSimulationResult> {
  const now = deps.now ?? Date.now;
  const rootPath = path.resolve(deps.userDataPath, SAFE_SIMULATION_ROOT);
  fs.mkdirSync(rootPath, { recursive: true });
  cleanupExpiredSafeSimulationWorkspaces(rootPath, now());

  const workspacePath = path.resolve(
    rootPath,
    `${new Date(now()).toISOString().replace(/[:.]/g, '-')}-${crypto.randomBytes(4).toString('hex')}`,
  );
  fs.mkdirSync(workspacePath, { recursive: true });

  await deps.createEfiWorkspace(workspacePath, profile);

  const [validation, matrix, buildPlan, recoveryDryRun] = await Promise.all([
    deps.validateEfi(workspacePath, profile),
    Promise.resolve(deps.buildCompatibilityMatrix(profile)),
    deps.simulateBuild(profile),
    deps.dryRunRecovery(profile.targetOS, profile.smbios),
  ]);

  const blockers: string[] = [];
  const warnings: string[] = [];
  const currentTarget = matrix.rows.find((row) => row.versionName === profile.targetOS) ?? null;

  if (currentTarget?.status === 'blocked') {
    pushUnique(blockers, currentTarget.reason);
  } else if (currentTarget?.status === 'partial') {
    pushUnique(warnings, currentTarget.reason);
  }

  for (const issue of validation.issues) {
    if (issue.severity === 'blocked') pushUnique(blockers, issue.message);
    else pushUnique(warnings, issue.message);
  }

  for (const blocker of buildPlan.blockers) {
    pushUnique(blockers, blocker);
  }

  if (recoveryDryRun.certainty === 'will_fail') {
    pushUnique(blockers, recoveryDryRun.recommendation);
  } else if (recoveryDryRun.certainty === 'may_fail') {
    pushUnique(warnings, recoveryDryRun.recommendation);
  }

  const configHash = hashFileIfPresent(path.resolve(workspacePath, 'EFI/OC/config.plist'));
  const { kexts, ssdts } = getRequiredResources(profile);
  const resourcePlan = buildResourcePlan({
    profile,
    validationResult: validation,
    kextRegistry: deps.kextRegistry,
  });

  return {
    workspacePath,
    efiSummary: {
      efiPath: workspacePath,
      configHash,
      requiredResources: [
        ...kexts.map((name) => ({ name, kind: 'kext' as const, sourceClass: deps.kextRegistry[name] ? 'downloaded' as const : 'bundled' as const })),
        ...ssdts.map((name) => ({ name, kind: 'ssdt' as const, sourceClass: 'generated' as const })),
      ],
    },
    validationSummary: {
      overall: validation.overall,
      issues: validation.issues.map((issue) => ({
        code: issue.code,
        severity: issue.severity,
        component: issue.component,
        message: issue.message,
      })),
    },
    compatibilityMatrixSnapshot: matrix.rows,
    recoveryReadiness: {
      certainty: recoveryDryRun.certainty,
      recommendation: recoveryDryRun.recommendation,
    },
    resourcePlan,
    blockers,
    warnings,
    disclaimer: SAFE_SIMULATION_DISCLAIMER,
  };
}
