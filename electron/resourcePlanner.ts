import { getRequiredResources, type HardwareProfile } from './configGenerator.js';
import type { ValidationResult } from './configValidator.js';

export type ResourcePlanKind = 'kext' | 'ssdt' | 'driver' | 'payload';
export type ResourcePlanSourceClass = 'bundled' | 'generated' | 'downloaded' | 'manual';
export type ResourcePlanValidationOutcome = 'verified' | 'warning' | 'blocked' | 'pending_manual';

export interface ResourcePlanEntry {
  name: string;
  kind: ResourcePlanKind;
  source: string;
  expectedIdentityOrVersion: string;
  validationOutcome: ResourcePlanValidationOutcome;
  sourceClass: ResourcePlanSourceClass;
}

export interface ResourcePlan {
  resources: ResourcePlanEntry[];
}

function findValidationOutcome(
  resourceName: string,
  validationResult: ValidationResult | null | undefined,
): ResourcePlanValidationOutcome {
  if (!validationResult) return 'pending_manual';

  const issue = validationResult.issues.find((candidate) =>
    candidate.component === resourceName ||
    candidate.expectedPath.includes(resourceName),
  );
  if (!issue) return 'verified';
  return issue.severity === 'blocked' ? 'blocked' : 'warning';
}

export function buildResourcePlan(input: {
  profile: HardwareProfile;
  validationResult?: ValidationResult | null;
  kextRegistry: Record<string, { repo: string; assetFilter?: string }>;
  kextSources?: Record<string, 'github' | 'embedded' | 'failed'>;
}): ResourcePlan {
  const { kexts, ssdts } = getRequiredResources(input.profile);
  const resources: ResourcePlanEntry[] = [];

  for (const kext of kexts) {
    const registryEntry = input.kextRegistry[kext];
    const fetchedSource = input.kextSources?.[kext];
    const sourceClass: ResourcePlanSourceClass = fetchedSource === 'embedded'
      ? 'bundled'
      : fetchedSource === 'github'
      ? 'downloaded'
      : registryEntry
      ? 'downloaded'
      : 'bundled';
    const validationOutcome = fetchedSource === 'failed'
      ? 'blocked'
      : findValidationOutcome(kext, input.validationResult);

    resources.push({
      name: kext,
      kind: 'kext',
      source: registryEntry ? `https://github.com/${registryEntry.repo}/releases/latest` : 'Bundled application asset',
      expectedIdentityOrVersion: registryEntry?.assetFilter
        ? `${registryEntry.repo} (${registryEntry.assetFilter})`
        : registryEntry?.repo ?? 'Bundled local asset',
      validationOutcome,
      sourceClass,
    });
  }

  for (const ssdt of ssdts) {
    resources.push({
      name: ssdt,
      kind: 'ssdt',
      source: 'Generated locally from the hardware profile',
      expectedIdentityOrVersion: 'Generated ACPI artifact',
      validationOutcome: findValidationOutcome(ssdt, input.validationResult),
      sourceClass: 'generated',
    });
  }

  for (const driver of ['OpenRuntime.efi', 'HfsPlus.efi']) {
    resources.push({
      name: driver,
      kind: 'driver',
      source: 'OpenCorePkg release contents',
      expectedIdentityOrVersion: 'OpenCore base driver',
      validationOutcome: findValidationOutcome(driver, input.validationResult),
      sourceClass: 'downloaded',
    });
  }

  resources.push({
    name: `Recovery payload for ${input.profile.targetOS}`,
    kind: 'payload',
    source: 'Apple recovery endpoint or validated manual import',
    expectedIdentityOrVersion: input.profile.targetOS,
    validationOutcome: 'pending_manual',
    sourceClass: 'downloaded',
  });

  return { resources };
}
