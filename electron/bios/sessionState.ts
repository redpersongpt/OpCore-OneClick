import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { HardwareProfile } from '../configGenerator.js';
import type { BiosSessionState, BiosSessionStage, BiosSettingSelection } from './types.js';

function getSessionFile(userDataPath: string): string {
  return path.resolve(userDataPath, 'bios_session.json');
}

function compareStrings(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

function normalizeFingerprintValue(value?: string | number | null): string | number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    return normalized || null;
  }
  return null;
}

function stableSerialize(value: unknown): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(item => stableSerialize(item)).join(',')}]`;

  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, entryValue]) => entryValue !== undefined)
    .sort(([a], [b]) => compareStrings(a, b))
    .map(([key, entryValue]) => `${JSON.stringify(key)}:${stableSerialize(entryValue)}`);

  return `{${entries.join(',')}}`;
}

export function buildHardwareFingerprint(profile: HardwareProfile): string {
  const gpuIdentifiers = Array.isArray(profile.gpuDevices) && profile.gpuDevices.length > 0
    ? profile.gpuDevices
      .map((gpu) => [
        normalizeFingerprintValue(gpu.vendorName),
        normalizeFingerprintValue(gpu.name),
        normalizeFingerprintValue(gpu.vendorId),
        normalizeFingerprintValue(gpu.deviceId),
      ].filter((part): part is string | number => part !== null).join(':'))
      .filter((identifier) => identifier.length > 0)
    : [normalizeFingerprintValue(profile.gpu)].filter((identifier): identifier is string => typeof identifier === 'string');

  const canonical = stableSerialize({
    architecture: normalizeFingerprintValue(profile.architecture),
    generation: normalizeFingerprintValue(profile.generation),
    cpu: normalizeFingerprintValue(profile.cpu),
    coreCount: normalizeFingerprintValue(profile.coreCount),
    motherboard: normalizeFingerprintValue(profile.motherboard),
    formFactor: profile.isLaptop ? 'laptop' : 'desktop',
    gpus: Array.from(new Set(gpuIdentifiers)).sort(compareStrings),
  });

  return crypto.createHash('sha256').update(canonical).digest('hex');
}

export function loadBiosSession(userDataPath: string): BiosSessionState | null {
  try {
    const sessionFile = getSessionFile(userDataPath);
    if (!fs.existsSync(sessionFile)) return null;
    const parsed = JSON.parse(fs.readFileSync(sessionFile, 'utf-8')) as BiosSessionState;
    return parsed;
  } catch {
    return null;
  }
}

export function saveBiosSession(userDataPath: string, session: BiosSessionState | null): void {
  const sessionFile = getSessionFile(userDataPath);
  if (!session) {
    try {
      if (fs.existsSync(sessionFile)) fs.unlinkSync(sessionFile);
    } catch {}
    return;
  }

  fs.writeFileSync(sessionFile, JSON.stringify(session, null, 2));
}

export function createBiosSession(input: {
  userDataPath: string;
  profile: HardwareProfile;
  vendor: BiosSessionState['vendor'];
  stage: BiosSessionStage;
  rebootRequested: boolean;
  selectedChanges: Record<string, BiosSettingSelection>;
  previousSessionId?: string | null;
}): BiosSessionState {
  const session: BiosSessionState = {
    sessionId: input.previousSessionId || crypto.randomUUID(),
    hardwareFingerprint: buildHardwareFingerprint(input.profile),
    selectedChanges: input.selectedChanges as BiosSessionState['selectedChanges'],
    stage: input.stage,
    vendor: input.vendor,
    rebootRequested: input.rebootRequested,
    timestamp: Date.now(),
  };
  saveBiosSession(input.userDataPath, session);
  return session;
}

export function updateBiosSessionStage(
  userDataPath: string,
  stage: BiosSessionStage,
): BiosSessionState | null {
  const existing = loadBiosSession(userDataPath);
  if (!existing) return null;
  const rebootStages: BiosSessionStage[] = ['rebooting_to_firmware', 'awaiting_return', 'ready_for_reboot'];
  const updated: BiosSessionState = {
    ...existing,
    stage,
    rebootRequested: rebootStages.includes(stage),
    timestamp: Date.now(),
  };
  saveBiosSession(userDataPath, updated);
  return updated;
}

export function clearBiosSession(userDataPath: string): void {
  saveBiosSession(userDataPath, null);
}
