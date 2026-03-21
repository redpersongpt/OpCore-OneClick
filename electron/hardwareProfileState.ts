import type { HardwareProfile } from './configGenerator.js';
import type { HardwareProfileArtifact } from './hardwareProfileArtifact.js';

const UNKNOWN_PATTERNS = [
  /^unknown\b/i,
  /^generic\b/i,
  /^n\/a$/i,
  /^not detected$/i,
  /^not available$/i,
  /^unavailable$/i,
  /^none$/i,
  /^null$/i,
] as const;

function normalizeText(value: string | null | undefined): string | null {
  if (typeof value !== 'string') return null;
  const normalized = value.trim().replace(/\s+/g, ' ');
  if (!normalized || normalized === '—') return null;
  return normalized.toLowerCase();
}

export function isUnknownHardwareValue(value: string | null | undefined): boolean {
  const normalized = normalizeText(value);
  if (!normalized) return true;
  return UNKNOWN_PATTERNS.some((pattern) => pattern.test(normalized));
}

function uniqueStrings(values: Array<string | null | undefined>): string[] {
  return Array.from(new Set(values.filter((value): value is string => typeof value === 'string' && value.length > 0))).sort();
}

function uniqueDisplayStrings(values: Array<string | null | undefined>): string[] {
  const deduped = new Map<string, string>();
  for (const value of values) {
    if (typeof value !== 'string') continue;
    const trimmed = value.trim().replace(/\s+/g, ' ');
    const normalized = normalizeText(trimmed);
    if (!normalized || deduped.has(normalized)) continue;
    deduped.set(normalized, trimmed);
  }
  return Array.from(deduped.values()).sort((left, right) => left.localeCompare(right));
}

function gpuTokens(profile: HardwareProfile): string[] {
  if (Array.isArray(profile.gpuDevices) && profile.gpuDevices.length > 0) {
    const normalized = profile.gpuDevices
      .map((device) => {
        const name = normalizeText(device.name);
        const vendor = normalizeText(device.vendorName);
        const vendorId = normalizeText(device.vendorId);
        const deviceId = normalizeText(device.deviceId);
        if (!name && !vendor && !vendorId && !deviceId) return null;
        return [vendor, name, vendorId, deviceId].filter((value): value is string => !!value).join(':');
      });
    const filtered = uniqueStrings(normalized);
    if (filtered.length > 0) return filtered;
  }

  return uniqueStrings(
    (profile.gpu || '')
      .split(/\s*\/\s*/)
      .map((entry) => isUnknownHardwareValue(entry) ? null : normalizeText(entry)),
  );
}

function gpuNames(profile: HardwareProfile): string[] {
  if (Array.isArray(profile.gpuDevices) && profile.gpuDevices.length > 0) {
    const deviceNames = uniqueStrings(profile.gpuDevices.map((device) => normalizeText(device.name)));
    if (deviceNames.length > 0) return deviceNames;
  }

  return uniqueStrings(
    (profile.gpu || '')
      .split(/\s*\/\s*/)
      .map((entry) => isUnknownHardwareValue(entry) ? null : normalizeText(entry)),
  );
}

function equalsKnownString(left: string | null | undefined, right: string | null | undefined): boolean {
  const a = normalizeText(left);
  const b = normalizeText(right);
  return !!a && !!b && a === b;
}

function profileStrength(profile: HardwareProfile | null | undefined): number {
  if (!profile) return 0;
  let score = 0;
  if (!isUnknownHardwareValue(profile.cpu)) score += 6;
  if (profile.architecture !== 'Unknown') score += 3;
  if (profile.generation !== 'Unknown') score += 3;
  if (!isUnknownHardwareValue(profile.gpu) || gpuTokens(profile).length > 0) score += 5;
  if (!isUnknownHardwareValue(profile.motherboard)) score += 4;
  if (!isUnknownHardwareValue(profile.ram)) score += 2;
  if (typeof profile.coreCount === 'number' && profile.coreCount > 0) score += 1;
  if (profile.scanConfidence === 'high') score += 3;
  else if (profile.scanConfidence === 'medium') score += 2;
  else if (profile.scanConfidence === 'low') score += 1;
  return score;
}

export function likelySameHardwareProfile(
  current: HardwareProfile | null | undefined,
  incoming: HardwareProfile | null | undefined,
): boolean {
  if (!current || !incoming) return false;

  const cpuConflict = !isUnknownHardwareValue(current.cpu) && !isUnknownHardwareValue(incoming.cpu)
    && !equalsKnownString(current.cpu, incoming.cpu);
  const archConflict = current.architecture !== 'Unknown' && incoming.architecture !== 'Unknown'
    && current.architecture !== incoming.architecture;
  const generationConflict = current.generation !== 'Unknown' && incoming.generation !== 'Unknown'
    && current.generation !== incoming.generation;
  const boardConflict = !isUnknownHardwareValue(current.motherboard) && !isUnknownHardwareValue(incoming.motherboard)
    && !equalsKnownString(current.motherboard, incoming.motherboard);
  const gpuTokenOverlap = gpuTokens(current).some((token) => gpuTokens(incoming).includes(token));
  const gpuNameOverlap = gpuNames(current).some((name) => gpuNames(incoming).includes(name));
  const gpuOverlap = gpuTokenOverlap || gpuNameOverlap;
  const gpuConflict = gpuTokens(current).length > 0 && gpuTokens(incoming).length > 0 && !gpuOverlap;

  if (cpuConflict || archConflict || generationConflict || boardConflict || gpuConflict) {
    return false;
  }

  let sharedSignals = 0;
  if (equalsKnownString(current.cpu, incoming.cpu)) sharedSignals += 3;
  if (equalsKnownString(current.motherboard, incoming.motherboard)) sharedSignals += 2;
  if (gpuOverlap) sharedSignals += 2;
  if (current.architecture !== 'Unknown' && current.architecture === incoming.architecture) sharedSignals += 1;
  if (current.generation !== 'Unknown' && current.generation === incoming.generation) sharedSignals += 1;
  if (current.isLaptop === incoming.isLaptop) sharedSignals += 1;
  return sharedSignals >= 3;
}

function mergeGpuDevices(
  current: HardwareProfile['gpuDevices'],
  incoming: HardwareProfile['gpuDevices'],
): HardwareProfile['gpuDevices'] {
  const incomingUseful = Array.isArray(incoming) && incoming.some((device) => !isUnknownHardwareValue(device.name));
  if (incomingUseful) return incoming;
  return current;
}

function strongerScanConfidence(
  current: HardwareProfile['scanConfidence'],
  incoming: HardwareProfile['scanConfidence'],
): HardwareProfile['scanConfidence'] {
  const rank: Record<'high' | 'medium' | 'low', number> = { high: 3, medium: 2, low: 1 };
  if (!current) return incoming;
  if (!incoming) return current;
  return rank[incoming] >= rank[current] ? incoming : current;
}

export function mergeHardwareProfilesForSameMachine(
  current: HardwareProfile,
  incoming: HardwareProfile,
): HardwareProfile {
  const merged: HardwareProfile = {
    ...incoming,
    cpu: isUnknownHardwareValue(incoming.cpu) ? current.cpu : incoming.cpu,
    architecture: incoming.architecture === 'Unknown' ? current.architecture : incoming.architecture,
    generation: incoming.generation === 'Unknown' ? current.generation : incoming.generation,
    coreCount: typeof incoming.coreCount === 'number' && incoming.coreCount > 0 ? incoming.coreCount : current.coreCount,
    gpu: isUnknownHardwareValue(incoming.gpu) ? current.gpu : incoming.gpu,
    gpuDevices: mergeGpuDevices(current.gpuDevices, incoming.gpuDevices),
    ram: isUnknownHardwareValue(incoming.ram) ? current.ram : incoming.ram,
    motherboard: isUnknownHardwareValue(incoming.motherboard) ? current.motherboard : incoming.motherboard,
    targetOS: current.targetOS || incoming.targetOS,
    smbios: current.smbios || incoming.smbios,
    kexts: current.kexts,
    ssdts: current.ssdts,
    bootArgs: current.bootArgs || incoming.bootArgs,
    isLaptop: incoming.isLaptop || current.isLaptop,
    isVM: incoming.isVM || current.isVM,
    audioLayoutId: incoming.audioLayoutId ?? current.audioLayoutId,
    strategy: current.strategy,
    scanConfidence: strongerScanConfidence(current.scanConfidence, incoming.scanConfidence),
  };

  if (Array.isArray(merged.gpuDevices) && merged.gpuDevices.length > 0) {
    const mergedNames = uniqueDisplayStrings(merged.gpuDevices.map((device) => device.name));
    if (mergedNames.length > 0) {
      merged.gpu = mergedNames.join(' / ');
    }
  }

  return merged;
}

export function buildPlanningIdentitySignature(profile: HardwareProfile | null | undefined): string {
  if (!profile) return 'missing';
  return JSON.stringify({
    cpu: normalizeText(profile.cpu),
    architecture: profile.architecture === 'Unknown' ? null : profile.architecture,
    generation: profile.generation === 'Unknown' ? null : profile.generation,
    motherboard: normalizeText(profile.motherboard),
    gpus: gpuTokens(profile),
    formFactor: profile.isLaptop ? 'laptop' : 'desktop',
    isVM: profile.isVM === true,
  });
}

export interface HardwareScanReconciliation {
  profile: HardwareProfile;
  likelySameMachine: boolean;
  usedExistingFields: boolean;
  shouldInvalidateBuild: boolean;
}

export function reconcileHardwareScanProfile(
  current: HardwareProfile | null | undefined,
  incoming: HardwareProfile,
): HardwareScanReconciliation {
  if (!current) {
    return {
      profile: incoming,
      likelySameMachine: false,
      usedExistingFields: false,
      shouldInvalidateBuild: false,
    };
  }

  if (!likelySameHardwareProfile(current, incoming)) {
    return {
      profile: incoming,
      likelySameMachine: false,
      usedExistingFields: false,
      shouldInvalidateBuild: true,
    };
  }

  const merged = mergeHardwareProfilesForSameMachine(current, incoming);
  const usedExistingFields = profileStrength(merged) > profileStrength(incoming);

  return {
    profile: merged,
    likelySameMachine: true,
    usedExistingFields,
    shouldInvalidateBuild: buildPlanningIdentitySignature(current) !== buildPlanningIdentitySignature(merged),
  };
}

export function canRestoreLatestScannedArtifact(
  artifact: HardwareProfileArtifact | null | undefined,
): boolean {
  if (!artifact) return false;
  return artifact.source === 'live_scan' || artifact.source === 'legacy_scan';
}
