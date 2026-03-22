import { getRequiredResources, type HardwareProfile } from './configGenerator.js';
import type { ValidationResult } from './configValidator.js';
import type { KextRegistryEntry } from './kextSourcePolicy.js';
import { getSsdtSourcePolicy } from './ssdtSourcePolicy.js';

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
  kextRegistry: Record<string, KextRegistryEntry>;
  kextSources?: Record<string, 'github' | 'embedded' | 'direct' | 'failed'>;
}): ResourcePlan {
  const { kexts, ssdts } = getRequiredResources(input.profile);
  const resources: ResourcePlanEntry[] = [];

  for (const kext of kexts) {
    const registryEntry = input.kextRegistry[kext];
    const fetchedSource = input.kextSources?.[kext];
    const sourceClass: ResourcePlanSourceClass = fetchedSource === 'embedded'
      ? 'bundled'
      : fetchedSource === 'github' || fetchedSource === 'direct'
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
      source: registryEntry?.directUrl
        ? registryEntry.directUrl
        : registryEntry
        ? `https://github.com/${registryEntry.repo}/releases/latest`
        : 'Bundled application asset',
      expectedIdentityOrVersion: registryEntry?.directUrl
        ? (registryEntry.staticVersion ? `Direct asset (${registryEntry.staticVersion})` : 'Direct asset download')
        : registryEntry?.assetFilter
        ? `${registryEntry.repo} (${registryEntry.assetFilter})`
        : registryEntry?.repo ?? 'Bundled local asset',
      validationOutcome,
      sourceClass,
    });
  }

  for (const ssdt of ssdts) {
    const ssdtPolicy = getSsdtSourcePolicy(ssdt);
    const hasSupplemental = !!ssdtPolicy?.supplementalDownload;
    const candidateNames = ssdtPolicy?.packageCandidates?.join(', ') ?? ssdt;
    resources.push({
      name: ssdt,
      kind: 'ssdt',
      source: hasSupplemental
        ? `OpenCore package samples or Dortania supplemental download`
        : 'OpenCore package pre-compiled ACPI samples',
      expectedIdentityOrVersion: ssdtPolicy
        ? `Package candidates: ${candidateNames}`
        : 'No source policy defined',
      validationOutcome: ssdtPolicy
        ? findValidationOutcome(ssdt, input.validationResult)
        : 'blocked',
      sourceClass: hasSupplemental ? 'downloaded' : 'bundled',
    });
  }

  for (const driver of ['OpenRuntime.efi', 'OpenHfsPlus.efi']) {
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
