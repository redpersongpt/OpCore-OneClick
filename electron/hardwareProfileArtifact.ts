import crypto from 'node:crypto';
import type { HardwareProfile } from './configGenerator.js';
import type { HardwareInterpretation } from './hardwareInterpret.js';

export const HARDWARE_PROFILE_ARTIFACT_KIND = 'hardware_profile_artifact';
export const HARDWARE_PROFILE_ARTIFACT_VERSION = 1;

export type HardwareProfileArtifactSource =
  | 'live_scan'
  | 'legacy_scan'
  | 'manual_planning'
  | 'imported_artifact';

export interface HardwareProfileInterpretationMetadata {
  overallConfidence: 'high' | 'medium' | 'low';
  summary: string;
  manualVerificationNeeded: string[];
}

export interface HardwareProfileArtifact {
  kind: typeof HARDWARE_PROFILE_ARTIFACT_KIND;
  version: typeof HARDWARE_PROFILE_ARTIFACT_VERSION;
  capturedAt: number;
  source: HardwareProfileArtifactSource;
  digest: string;
  profile: HardwareProfile;
  interpretation: HardwareProfileInterpretationMetadata | null;
}

type StableSerializable =
  | null
  | boolean
  | number
  | string
  | StableSerializable[]
  | { [key: string]: StableSerializable | undefined };

function compareStrings(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

function stableSerialize(value: StableSerializable): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(item => stableSerialize(item)).join(',')}]`;

  const entries = Object.entries(value)
    .filter(([, entryValue]) => entryValue !== undefined)
    .sort(([a], [b]) => compareStrings(a, b))
    .map(([key, entryValue]) => `${JSON.stringify(key)}:${stableSerialize(entryValue as StableSerializable)}`);

  return `{${entries.join(',')}}`;
}

function assertPlainRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function assertNoUnknownKeys(record: Record<string, unknown>, allowedKeys: readonly string[], label: string): void {
  const allowed = new Set(allowedKeys);
  const unknown = Object.keys(record).filter((key) => !allowed.has(key));
  if (unknown.length > 0) {
    throw new Error(`${label} contains unknown field(s): ${unknown.join(', ')}`);
  }
}

function parseRequiredString(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new Error(`${label} must be a string`);
  const normalized = value.trim();
  if (normalized.length === 0) throw new Error(`${label} must not be empty`);
  return normalized;
}

function parseOptionalString(value: unknown, label: string): string | undefined {
  if (value === undefined || value === null) return undefined;
  return parseRequiredString(value, label);
}

function parseOptionalBoolean(value: unknown, label: string): boolean | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'boolean') throw new Error(`${label} must be a boolean`);
  return value;
}

function parseOptionalFiniteNumber(value: unknown, label: string): number | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error(`${label} must be a finite number`);
  return value;
}

function parseRequiredFiniteNumber(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error(`${label} must be a finite number`);
  return value;
}

function parseStringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`);
  return Array.from(new Set(value.map((entry, index) => parseRequiredString(entry, `${label}[${index}]`)))).sort(compareStrings);
}

function parseGpuDevices(value: unknown): HardwareProfile['gpuDevices'] {
  if (value === undefined || value === null) return undefined;
  if (!Array.isArray(value)) throw new Error('profile.gpuDevices must be an array');

  return value.map((entry, index) => {
    const record = assertPlainRecord(entry, `profile.gpuDevices[${index}]`);
    assertNoUnknownKeys(record, ['name', 'vendorName', 'vendorId', 'deviceId'], `profile.gpuDevices[${index}]`);
    return {
      name: parseRequiredString(record.name, `profile.gpuDevices[${index}].name`),
      vendorName: parseOptionalString(record.vendorName, `profile.gpuDevices[${index}].vendorName`),
      vendorId: parseOptionalString(record.vendorId, `profile.gpuDevices[${index}].vendorId`),
      deviceId: parseOptionalString(record.deviceId, `profile.gpuDevices[${index}].deviceId`),
    };
  }).sort((left, right) => {
    const leftKey = `${left.vendorName ?? ''}:${left.name}:${left.vendorId ?? ''}:${left.deviceId ?? ''}`;
    const rightKey = `${right.vendorName ?? ''}:${right.name}:${right.vendorId ?? ''}:${right.deviceId ?? ''}`;
    return compareStrings(leftKey, rightKey);
  });
}

export function normalizeHardwareProfile(value: unknown): HardwareProfile {
  const record = assertPlainRecord(value, 'profile');
  assertNoUnknownKeys(record, [
    'cpu',
    'architecture',
    'generation',
    'coreCount',
    'gpu',
    'gpuDevices',
    'ram',
    'motherboard',
    'targetOS',
    'smbios',
    'kexts',
    'ssdts',
    'bootArgs',
    'isLaptop',
    'isVM',
    'audioLayoutId',
    'strategy',
    'scanConfidence',
  ], 'profile');

  const architecture = parseRequiredString(record.architecture, 'profile.architecture');
  if (!['Intel', 'AMD', 'Apple Silicon', 'Unknown'].includes(architecture)) {
    throw new Error(`profile.architecture is not supported: ${architecture}`);
  }
  const generation = parseRequiredString(record.generation, 'profile.generation');
  const allowedGenerations: HardwareProfile['generation'][] = [
    'Penryn',
    'Bulldozer',
    'Sandy Bridge',
    'Ivy Bridge',
    'Haswell',
    'Broadwell',
    'Skylake',
    'Kaby Lake',
    'Coffee Lake',
    'Comet Lake',
    'Rocket Lake',
    'Alder Lake',
    'Raptor Lake',
    'Ivy Bridge-E',
    'Haswell-E',
    'Broadwell-E',
    'Cascade Lake-X',
    'Ryzen',
    'Threadripper',
    'Apple Silicon',
    'Unknown',
  ];
  if (!allowedGenerations.includes(generation as HardwareProfile['generation'])) {
    throw new Error(`profile.generation is not supported: ${generation}`);
  }

  const strategy = parseOptionalString(record.strategy, 'profile.strategy');
  if (strategy && !['canonical', 'conservative', 'blocked'].includes(strategy)) {
    throw new Error(`profile.strategy is not supported: ${strategy}`);
  }
  const scanConfidence = parseOptionalString(record.scanConfidence, 'profile.scanConfidence');
  if (scanConfidence && !['high', 'medium', 'low'].includes(scanConfidence)) {
    throw new Error(`profile.scanConfidence is not supported: ${scanConfidence}`);
  }

  const normalized: HardwareProfile = {
    cpu: parseRequiredString(record.cpu, 'profile.cpu'),
    architecture: architecture as HardwareProfile['architecture'],
    generation: generation as HardwareProfile['generation'],
    coreCount: parseOptionalFiniteNumber(record.coreCount, 'profile.coreCount'),
    gpu: parseRequiredString(record.gpu, 'profile.gpu'),
    gpuDevices: parseGpuDevices(record.gpuDevices),
    ram: parseRequiredString(record.ram, 'profile.ram'),
    motherboard: parseRequiredString(record.motherboard, 'profile.motherboard'),
    targetOS: parseRequiredString(record.targetOS, 'profile.targetOS'),
    smbios: parseRequiredString(record.smbios, 'profile.smbios'),
    kexts: parseStringArray(record.kexts, 'profile.kexts'),
    ssdts: parseStringArray(record.ssdts, 'profile.ssdts'),
    bootArgs: parseRequiredString(record.bootArgs, 'profile.bootArgs'),
    isLaptop: parseOptionalBoolean(record.isLaptop, 'profile.isLaptop') ?? false,
    isVM: parseOptionalBoolean(record.isVM, 'profile.isVM'),
    audioLayoutId: parseOptionalFiniteNumber(record.audioLayoutId, 'profile.audioLayoutId'),
    strategy: strategy as HardwareProfile['strategy'] | undefined,
    scanConfidence: scanConfidence as HardwareProfile['scanConfidence'] | undefined,
  };

  return normalized;
}

export function extractHardwareProfileInterpretationMetadata(
  interpretation: HardwareInterpretation | null | undefined,
): HardwareProfileInterpretationMetadata | null {
  if (!interpretation) return null;
  return {
    overallConfidence: interpretation.overallConfidence,
    summary: interpretation.summary.trim(),
    manualVerificationNeeded: Array.from(new Set(
      interpretation.manualVerificationNeeded
        .map((item) => item.trim())
        .filter(Boolean),
    )).sort(compareStrings),
  };
}

export function normalizeHardwareProfileInterpretationMetadata(value: unknown): HardwareProfileInterpretationMetadata | null {
  if (value === undefined || value === null) return null;
  const record = assertPlainRecord(value, 'interpretation');
  assertNoUnknownKeys(record, ['overallConfidence', 'summary', 'manualVerificationNeeded'], 'interpretation');

  const overallConfidence = parseRequiredString(record.overallConfidence, 'interpretation.overallConfidence');
  if (!['high', 'medium', 'low'].includes(overallConfidence)) {
    throw new Error(`interpretation.overallConfidence is not supported: ${overallConfidence}`);
  }

  return {
    overallConfidence: overallConfidence as HardwareProfileInterpretationMetadata['overallConfidence'],
    summary: parseRequiredString(record.summary, 'interpretation.summary'),
    manualVerificationNeeded: parseStringArray(record.manualVerificationNeeded, 'interpretation.manualVerificationNeeded'),
  };
}

export function buildHardwareProfileArtifactDigest(input: {
  profile: HardwareProfile;
  interpretation: HardwareProfileInterpretationMetadata | null;
}): string {
  const payload = stableSerialize({
    version: HARDWARE_PROFILE_ARTIFACT_VERSION,
    profile: input.profile as unknown as StableSerializable,
    interpretation: input.interpretation as unknown as StableSerializable,
  });
  return crypto.createHash('sha256').update(payload).digest('hex');
}

export function createHardwareProfileArtifact(input: {
  profile: unknown;
  interpretation?: unknown;
  capturedAt?: number;
  source?: HardwareProfileArtifactSource;
}): HardwareProfileArtifact {
  const profile = normalizeHardwareProfile(input.profile);
  const interpretation = normalizeHardwareProfileInterpretationMetadata(input.interpretation);
  const capturedAt = input.capturedAt ?? Date.now();
  if (!Number.isFinite(capturedAt)) throw new Error('capturedAt must be a finite number');
  const source = input.source ?? 'manual_planning';
  if (!['live_scan', 'legacy_scan', 'manual_planning', 'imported_artifact'].includes(source)) {
    throw new Error(`artifact source is not supported: ${source}`);
  }

  return {
    kind: HARDWARE_PROFILE_ARTIFACT_KIND,
    version: HARDWARE_PROFILE_ARTIFACT_VERSION,
    capturedAt,
    source,
    digest: buildHardwareProfileArtifactDigest({ profile, interpretation }),
    profile,
    interpretation,
  };
}

export function parseHardwareProfileArtifact(value: unknown): HardwareProfileArtifact {
  const record = assertPlainRecord(value, 'artifact');
  assertNoUnknownKeys(record, ['kind', 'version', 'capturedAt', 'source', 'digest', 'profile', 'interpretation'], 'artifact');

  if (record.kind !== HARDWARE_PROFILE_ARTIFACT_KIND) {
    throw new Error(`artifact.kind is not supported: ${String(record.kind)}`);
  }
  if (record.version !== HARDWARE_PROFILE_ARTIFACT_VERSION) {
    throw new Error(`artifact.version is not supported: ${String(record.version)}`);
  }

  const capturedAt = parseRequiredFiniteNumber(record.capturedAt, 'artifact.capturedAt');
  const artifact = createHardwareProfileArtifact({
    profile: record.profile,
    interpretation: record.interpretation,
    capturedAt,
    source: parseRequiredString(record.source, 'artifact.source') as HardwareProfileArtifactSource,
  });
  const digest = parseRequiredString(record.digest, 'artifact.digest');
  if (artifact.digest !== digest) {
    throw new Error('artifact.digest does not match the normalized artifact payload');
  }
  return artifact;
}
