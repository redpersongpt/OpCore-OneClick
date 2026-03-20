import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { HardwareProfile } from './configGenerator.js';
import type { DiskInfo } from './diskOps.js';
import { buildHardwareFingerprint } from './bios/sessionState.js';

export const FLASH_CONFIRMATION_TTL_MS = 5 * 60 * 1000;
export const FLASH_CONFIRMATION_TOKEN_VERSION = 2;
export const FLASH_CONFIRMATION_CONSUMED_RETENTION_MS = 30 * 60 * 1000;

const FLASH_CONFIRMATION_TOKEN_PREFIX = 'flashconf';
const DEFAULT_FLASH_CONFIRMATION_SECRET_BYTES = 32;
const EFI_HASH_REQUIRED_DIRECTORIES = [
  'EFI',
  'EFI/BOOT',
  'EFI/OC',
  'EFI/OC/ACPI',
  'EFI/OC/Drivers',
  'EFI/OC/Kexts',
] as const;
const EFI_HASH_REQUIRED_FILES = [
  'EFI/BOOT/BOOTx64.efi',
  'EFI/OC/config.plist',
  'EFI/OC/OpenCore.efi',
] as const;
const EFI_HASH_ROOTS = [
  'EFI/BOOT',
  'EFI/OC',
] as const;
const RECOVERY_PAYLOAD_ROOT = 'com.apple.recovery.boot' as const;
const RECOVERY_PAYLOAD_REQUIRED_DIRECTORIES = [
  RECOVERY_PAYLOAD_ROOT,
] as const;
const RECOVERY_PAYLOAD_REQUIRED_FILES = [
  `${RECOVERY_PAYLOAD_ROOT}/BaseSystem.dmg`,
] as const;
const HASH_IGNORED_NAMES = new Set([
  '.DS_Store',
  'Thumbs.db',
  'desktop.ini',
  'Icon\r',
]);

export type DiskIdentityFingerprint = Partial<Pick<
  DiskInfo,
  'serialNumber' |
  'devicePath' |
  'vendor' |
  'transport' |
  'partitionTable' |
  'sizeBytes' |
  'model' |
  'removable'
>>;

export type FlashAuthorizationSnapshotStage = 'snapshot_a' | 'snapshot_b' | 'snapshot_c';
export type FlashAuthorizationMismatchField = 'session' | 'device' | 'disk' | 'efi' | 'payload' | 'hardware';

export interface FlashAuthorizationSnapshot {
  stage: FlashAuthorizationSnapshotStage;
  capturedAt: number;
  sessionId: string | null;
  device: string | null;
  diskFingerprint: DiskIdentityFingerprint | null;
  efiStateHash: string | null;
  payloadStateHash: string | null;
  hardwareFingerprint: string | null;
}

export interface FlashAuthorizationSnapshotComparison {
  ok: boolean;
  mismatchFields: FlashAuthorizationMismatchField[];
  mismatchDetails: string[];
}

export interface FlashConfirmationClaims {
  version: number;
  sessionId: string;
  nonce: string;
  issuedAt: number;
  expiresAt: number;
  device: string;
  diskFingerprint: DiskIdentityFingerprint;
  efiStateHash: string;
  payloadStateHash: string | null;
  hardwareFingerprint: string;
}

export interface FlashConfirmationRecord extends FlashConfirmationClaims {
  token: string;
}

export interface FlashConfirmationLookup {
  state:
    | 'missing'
    | 'malformed'
    | 'signature_invalid'
    | 'session_mismatch'
    | 'unknown'
    | 'consumed'
    | 'pending';
  record: FlashConfirmationRecord | null;
}

export interface FlashConfirmationValidation {
  valid: boolean;
  reason: string | null;
  code:
    | 'CONFIRMATION_MISSING'
    | 'CONFIRMATION_MALFORMED'
    | 'CONFIRMATION_SIGNATURE_INVALID'
    | 'CONFIRMATION_SESSION_CHANGED'
    | 'CONFIRMATION_UNKNOWN'
    | 'CONFIRMATION_CONSUMED'
    | 'CONFIRMATION_EXPIRED'
    | 'CONFIRMATION_DEVICE_CHANGED'
    | 'CONFIRMATION_DISK_CHANGED'
    | 'CONFIRMATION_EFI_CHANGED'
    | 'CONFIRMATION_PAYLOAD_CHANGED'
    | 'CONFIRMATION_HARDWARE_CHANGED'
    | null;
  mismatchFields?: FlashAuthorizationMismatchField[];
}

export interface FlashSafetyDecision {
  allowed: boolean;
  reason: string | null;
  code:
    | 'NO_DEVICE_SELECTED'
    | 'TARGET_DISAPPEARED'
    | 'CONFIRMATION_REQUIRED'
    | 'IDENTITY_FINGERPRINT_MISSING'
    | 'IDENTITY_CHANGED'
    | 'IDENTITY_AMBIGUOUS'
    | 'IDENTITY_WEAK'
    | 'IDENTITY_COLLISION'
    | 'SYSTEM_DISK'
    | 'UNSAFE_PARTITION_TABLE'
    | 'EFI_INVALID'
    | 'BIOS_NOT_READY'
    | 'DEPLOY_GUARD_FAILED'
    | null;
}

export interface FlashSafetyContext {
  selectedDevice: string | null;
  currentDisk: DiskInfo | null;
  expectedIdentity: Partial<DiskInfo> | DiskIdentityFingerprint | null;
  collisionDevices: string[];
  deployGuardAllowed: boolean;
  deployGuardReason: string | null;
  biosReady: boolean;
  efiValidationClean: boolean;
  explicitUserConfirmation: boolean;
  confirmationValidated: FlashConfirmationValidation;
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

function encodeBase64Url(value: Buffer | string): string {
  const buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, 'utf8');
  return buffer.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

function decodeBase64Url(value: string): Buffer | null {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) return null;
  const normalized = value.replace(/-/g, '+').replace(/_/g, '/');
  const padding = (4 - (normalized.length % 4)) % 4;
  try {
    return Buffer.from(normalized + '='.repeat(padding), 'base64');
  } catch {
    return null;
  }
}

function normalizeSecret(secret?: Buffer | string): Buffer {
  if (!secret) return crypto.randomBytes(DEFAULT_FLASH_CONFIRMATION_SECRET_BYTES);
  const buffer = Buffer.isBuffer(secret) ? Buffer.from(secret) : Buffer.from(secret, 'utf8');
  if (buffer.length === 0) throw new Error('Flash confirmation secret must not be empty');
  return buffer;
}

function normalizeRelativePath(rootPath: string, absolutePath: string): string {
  return path.relative(rootPath, absolutePath).replace(/\\/g, '/');
}

function normalizeLinkTarget(target: string): string {
  return target.replace(/\\/g, '/');
}

function shouldIgnoreHashEntry(name: string): boolean {
  return HASH_IGNORED_NAMES.has(name) || name.startsWith('._');
}

function describeDirent(entry: fs.Dirent): string {
  if (entry.isFile()) return 'file';
  if (entry.isDirectory()) return 'directory';
  if (entry.isSymbolicLink()) return 'symlink';
  if ((entry as fs.Dirent & { isSocket?: () => boolean }).isSocket?.()) return 'socket';
  if ((entry as fs.Dirent & { isFIFO?: () => boolean }).isFIFO?.()) return 'fifo';
  if ((entry as fs.Dirent & { isCharacterDevice?: () => boolean }).isCharacterDevice?.()) return 'character-device';
  if ((entry as fs.Dirent & { isBlockDevice?: () => boolean }).isBlockDevice?.()) return 'block-device';
  return 'other';
}

function getPathKind(targetPath: string): 'missing' | 'file' | 'directory' | 'symlink' | 'other' {
  try {
    const stat = fs.lstatSync(targetPath);
    if (stat.isSymbolicLink()) return 'symlink';
    if (stat.isDirectory()) return 'directory';
    if (stat.isFile()) return 'file';
    return 'other';
  } catch {
    return 'missing';
  }
}

function computeDeterministicTreeHash(input: {
  rootPath: string;
  roots: readonly string[];
  requiredDirectories?: readonly string[];
  requiredFiles?: readonly string[];
}): string {
  const basePath = path.resolve(input.rootPath);
  const manifest: string[] = [];

  for (const requiredDir of input.requiredDirectories ?? []) {
    const requiredPath = path.resolve(basePath, requiredDir);
    const kind = getPathKind(requiredPath);
    manifest.push(
      kind === 'directory'
        ? `REQUIRED_DIR|${requiredDir}`
        : kind === 'symlink'
          ? `REQUIRED_DIR_SYMLINK|${requiredDir}|${normalizeLinkTarget(fs.readlinkSync(requiredPath))}`
          : `MISSING_DIR|${requiredDir}|${kind}`,
    );
  }

  for (const requiredFile of input.requiredFiles ?? []) {
    const requiredPath = path.resolve(basePath, requiredFile);
    const kind = getPathKind(requiredPath);
    manifest.push(
      kind === 'file'
        ? `REQUIRED_FILE|${requiredFile}`
        : kind === 'symlink'
          ? `REQUIRED_FILE_SYMLINK|${requiredFile}|${normalizeLinkTarget(fs.readlinkSync(requiredPath))}`
          : `MISSING_FILE|${requiredFile}|${kind}`,
    );
  }

  const walk = (directoryPath: string): void => {
    const entries = fs.readdirSync(directoryPath, { withFileTypes: true })
      .filter((entry) => !shouldIgnoreHashEntry(entry.name))
      .sort((a, b) => compareStrings(a.name, b.name));

    for (const entry of entries) {
      const fullPath = path.resolve(directoryPath, entry.name);
      const relativePath = normalizeRelativePath(basePath, fullPath);

      if (entry.isSymbolicLink()) {
        manifest.push(`L|${relativePath}|${normalizeLinkTarget(fs.readlinkSync(fullPath))}`);
        continue;
      }
      if (entry.isDirectory()) {
        manifest.push(`D|${relativePath}`);
        walk(fullPath);
        continue;
      }
      if (entry.isFile()) {
        const content = fs.readFileSync(fullPath);
        const digest = crypto.createHash('sha256').update(content).digest('hex');
        manifest.push(`F|${relativePath}|${content.length}|${digest}`);
        continue;
      }

      manifest.push(`O|${relativePath}|${describeDirent(entry)}`);
    }
  };

  for (const root of input.roots) {
    const rootDirectory = path.resolve(basePath, root);
    const kind = getPathKind(rootDirectory);
    if (kind === 'directory') {
      manifest.push(`ROOT_DIR|${root}`);
      walk(rootDirectory);
      continue;
    }
    if (kind === 'symlink') {
      manifest.push(`ROOT_SYMLINK|${root}|${normalizeLinkTarget(fs.readlinkSync(rootDirectory))}`);
      continue;
    }
    if (kind !== 'missing') {
      manifest.push(`ROOT_NON_DIRECTORY|${root}|${kind}`);
    }
  }

  return crypto.createHash('sha256').update(manifest.join('\n')).digest('hex');
}

function serializeFlashConfirmationClaims(claims: FlashConfirmationClaims): string {
  return stableSerialize({
    version: claims.version,
    sessionId: claims.sessionId,
    nonce: claims.nonce,
    issuedAt: claims.issuedAt,
    expiresAt: claims.expiresAt,
    device: claims.device,
    diskFingerprint: claims.diskFingerprint as StableSerializable,
    efiStateHash: claims.efiStateHash,
    payloadStateHash: claims.payloadStateHash,
    hardwareFingerprint: claims.hardwareFingerprint,
  });
}

function signFlashConfirmationClaims(secret: Buffer, claims: FlashConfirmationClaims): string {
  const payload = encodeBase64Url(serializeFlashConfirmationClaims(claims));
  const signature = crypto.createHmac('sha256', secret).update(payload).digest();
  return `${FLASH_CONFIRMATION_TOKEN_PREFIX}.${claims.sessionId}.${payload}.${encodeBase64Url(signature)}`;
}

function buildRecordFromClaims(token: string, claims: FlashConfirmationClaims): FlashConfirmationRecord {
  return {
    token,
    version: claims.version,
    sessionId: claims.sessionId,
    nonce: claims.nonce,
    issuedAt: claims.issuedAt,
    expiresAt: claims.expiresAt,
    device: claims.device,
    diskFingerprint: { ...claims.diskFingerprint },
    efiStateHash: claims.efiStateHash,
    payloadStateHash: claims.payloadStateHash,
    hardwareFingerprint: claims.hardwareFingerprint,
  };
}

function parseDiskFingerprint(value: unknown): DiskIdentityFingerprint | null {
  if (!value || Array.isArray(value) || typeof value !== 'object') return null;

  const fingerprint: DiskIdentityFingerprint = {};
  for (const [key, entryValue] of Object.entries(value as Record<string, unknown>)) {
    if (
      entryValue !== undefined &&
      typeof entryValue !== 'string' &&
      typeof entryValue !== 'number' &&
      typeof entryValue !== 'boolean'
    ) {
      return null;
    }

    if (entryValue !== undefined) {
      (fingerprint as Record<string, string | number | boolean>)[key] = entryValue as string | number | boolean;
    }
  }

  return fingerprint;
}

function parseFlashConfirmationClaims(payload: string): FlashConfirmationClaims | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch {
    return null;
  }

  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
  const record = parsed as Record<string, unknown>;
  const diskFingerprint = parseDiskFingerprint(record.diskFingerprint);

  if (
    record.version !== FLASH_CONFIRMATION_TOKEN_VERSION ||
    typeof record.sessionId !== 'string' ||
    record.sessionId.length === 0 ||
    typeof record.nonce !== 'string' ||
    record.nonce.length < 16 ||
    typeof record.issuedAt !== 'number' ||
    !Number.isFinite(record.issuedAt) ||
    typeof record.expiresAt !== 'number' ||
    !Number.isFinite(record.expiresAt) ||
    typeof record.device !== 'string' ||
    record.device.length === 0 ||
    typeof record.efiStateHash !== 'string' ||
    record.efiStateHash.length === 0 ||
    (record.payloadStateHash !== null && record.payloadStateHash !== undefined && typeof record.payloadStateHash !== 'string') ||
    typeof record.hardwareFingerprint !== 'string' ||
    record.hardwareFingerprint.length === 0 ||
    !diskFingerprint
  ) {
    return null;
  }

  return {
    version: record.version,
    sessionId: record.sessionId,
    nonce: record.nonce,
    issuedAt: record.issuedAt,
    expiresAt: record.expiresAt,
    device: record.device,
    diskFingerprint,
    efiStateHash: record.efiStateHash,
    payloadStateHash: typeof record.payloadStateHash === 'string' ? record.payloadStateHash : null,
    hardwareFingerprint: record.hardwareFingerprint,
  };
}

function verifyFlashConfirmationToken(
  token: string,
  secret: Buffer,
  currentSessionId: string,
):
  | { state: 'pending'; record: FlashConfirmationRecord }
  | { state: 'malformed' | 'signature_invalid' | 'session_mismatch'; record: FlashConfirmationRecord | null } {
  const parts = token.split('.');
  if (parts.length !== 4 || parts[0] !== FLASH_CONFIRMATION_TOKEN_PREFIX) {
    return { state: 'malformed', record: null };
  }

  const headerSessionId = parts[1];
  const payload = parts[2];
  const providedSignature = parts[3];
  if (!headerSessionId || headerSessionId !== currentSessionId) {
    return { state: 'session_mismatch', record: null };
  }

  const expectedSignature = crypto.createHmac('sha256', secret).update(payload).digest();
  const providedSignatureBuffer = decodeBase64Url(providedSignature);
  if (!providedSignatureBuffer) {
    return { state: 'malformed', record: null };
  }
  if (providedSignatureBuffer.length !== expectedSignature.length) {
    return { state: 'signature_invalid', record: null };
  }
  if (!crypto.timingSafeEqual(providedSignatureBuffer, expectedSignature)) {
    return { state: 'signature_invalid', record: null };
  }

  const payloadBuffer = decodeBase64Url(payload);
  if (!payloadBuffer) {
    return { state: 'malformed', record: null };
  }

  const claims = parseFlashConfirmationClaims(payloadBuffer.toString('utf8'));
  if (!claims) {
    return { state: 'malformed', record: null };
  }
  if (claims.sessionId !== currentSessionId || claims.sessionId !== headerSessionId) {
    return { state: 'session_mismatch', record: buildRecordFromClaims(token, claims) };
  }

  return { state: 'pending', record: buildRecordFromClaims(token, claims) };
}

export function buildFlashConfirmationValidationFromSnapshotComparison(input: {
  comparison: FlashAuthorizationSnapshotComparison;
}): FlashConfirmationValidation {
  const { mismatchFields, mismatchDetails } = input.comparison;
  if (mismatchFields.includes('session')) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation belongs to a different main-process session. Re-open the confirmation dialog and confirm again.',
      code: 'CONFIRMATION_SESSION_CHANGED',
      mismatchFields,
    };
  }
  if (mismatchFields.includes('device')) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: The selected target disk changed after confirmation. Re-select the drive and confirm again.',
      code: 'CONFIRMATION_DEVICE_CHANGED',
      mismatchFields,
    };
  }
  if (mismatchFields.includes('disk')) {
    return {
      valid: false,
      reason: `SAFETY BLOCK: Disk identity changed after confirmation. Mismatch: ${mismatchDetails.filter(item => item.startsWith('disk:')).map(item => item.replace(/^disk:\s*/, '')).join(', ') || 'target disk mismatch'}.`,
      code: 'CONFIRMATION_DISK_CHANGED',
      mismatchFields,
    };
  }
  if (mismatchFields.includes('efi')) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: The EFI changed after confirmation. Rebuild or revalidate the EFI, then confirm again.',
      code: 'CONFIRMATION_EFI_CHANGED',
      mismatchFields,
    };
  }
  if (mismatchFields.includes('payload')) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: The recovery or installer payload changed after confirmation. Re-acquire the payload and confirm again.',
      code: 'CONFIRMATION_PAYLOAD_CHANGED',
      mismatchFields,
    };
  }
  if (mismatchFields.includes('hardware')) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: The hardware context changed after confirmation. Re-scan hardware and reopen the confirmation dialog.',
      code: 'CONFIRMATION_HARDWARE_CHANGED',
      mismatchFields,
    };
  }

  return {
    valid: false,
    reason: 'SAFETY BLOCK: Flash authorization state no longer matches the confirmed state. Re-open the confirmation dialog and try again.',
    code: 'CONFIRMATION_UNKNOWN',
    mismatchFields,
  };
}

export function normalizeIdentityValue(value?: string | number | boolean | null): string | number | boolean | undefined {
  if (typeof value === 'string') {
    const normalized = value.trim();
    return normalized ? normalized.toLowerCase() : undefined;
  }
  if (typeof value === 'number' || typeof value === 'boolean') return value;
  return undefined;
}

export function buildDiskIdentityFingerprint(info: Partial<DiskInfo> | DiskIdentityFingerprint | null | undefined): DiskIdentityFingerprint {
  if (!info) return {};
  const fingerprint: DiskIdentityFingerprint = {};
  const fields: Array<keyof DiskIdentityFingerprint> = ['serialNumber', 'devicePath', 'vendor', 'transport', 'partitionTable', 'sizeBytes', 'model', 'removable'];
  for (const field of fields) {
    const normalized = normalizeIdentityValue(info[field] as string | number | boolean | null | undefined);
    if (normalized !== undefined) {
      (fingerprint as Record<string, string | number | boolean>)[field] = normalized;
    }
  }
  return fingerprint;
}

export function createFlashAuthorizationSnapshot(input: {
  stage: FlashAuthorizationSnapshotStage;
  capturedAt?: number;
  sessionId: string | null;
  device: string | null;
  diskFingerprint: Partial<DiskInfo> | DiskIdentityFingerprint | null;
  efiStateHash: string | null;
  payloadStateHash?: string | null;
  hardwareFingerprint: string | null;
}): FlashAuthorizationSnapshot {
  return {
    stage: input.stage,
    capturedAt: input.capturedAt ?? Date.now(),
    sessionId: input.sessionId,
    device: input.device,
    diskFingerprint: input.diskFingerprint ? buildDiskIdentityFingerprint(input.diskFingerprint) : null,
    efiStateHash: input.efiStateHash,
    payloadStateHash: input.payloadStateHash ?? null,
    hardwareFingerprint: input.hardwareFingerprint,
  };
}

export function createFlashAuthorizationSnapshotFromRecord(
  record: FlashConfirmationRecord,
  stage: FlashAuthorizationSnapshotStage = 'snapshot_a',
  capturedAt = record.issuedAt,
): FlashAuthorizationSnapshot {
  return createFlashAuthorizationSnapshot({
    stage,
    capturedAt,
    sessionId: record.sessionId,
    device: record.device,
    diskFingerprint: record.diskFingerprint,
    efiStateHash: record.efiStateHash,
    payloadStateHash: record.payloadStateHash,
    hardwareFingerprint: record.hardwareFingerprint,
  });
}

export function compareDiskIdentity(
  expected: Partial<DiskInfo> | DiskIdentityFingerprint,
  current: Partial<DiskInfo> | DiskIdentityFingerprint,
) {
  const fieldsUsed = new Set<string>();
  const mismatches: string[] = [];
  const addExactCheck = (field: keyof DiskIdentityFingerprint, requiredWhenExpected = false) => {
    const prev = normalizeIdentityValue(expected[field] as string | number | boolean | null | undefined);
    const next = normalizeIdentityValue(current[field] as string | number | boolean | null | undefined);
    if (prev === undefined) return;
    fieldsUsed.add(String(field));
    if (next === undefined) {
      mismatches.push(`${String(field)} missing`);
      return;
    }
    if (field === 'sizeBytes') {
      if (Math.abs(Number(prev) - Number(next)) > 1024 * 1024) mismatches.push('sizeBytes changed');
      return;
    }
    if (prev !== next) mismatches.push(requiredWhenExpected ? `${String(field)} mismatch` : `${String(field)} changed`);
  };

  const hasStableId = !!normalizeIdentityValue(expected.serialNumber as string | number | boolean | null | undefined);
  if (hasStableId) addExactCheck('serialNumber', true);
  addExactCheck('devicePath');
  addExactCheck('transport');
  addExactCheck('removable');
  addExactCheck('partitionTable');
  addExactCheck('sizeBytes', true);
  addExactCheck('vendor');
  addExactCheck('model');

  const confidence = hasStableId
    ? 'strong'
    : fieldsUsed.size >= 5
      ? 'medium'
      : 'ambiguous';

  return {
    ok: mismatches.length === 0,
    confidence,
    fieldsUsed: Array.from(fieldsUsed),
    mismatches,
    fingerprint: buildDiskIdentityFingerprint(current),
  };
}

export function compareFlashAuthorizationSnapshots(
  expected: FlashAuthorizationSnapshot,
  current: FlashAuthorizationSnapshot,
): FlashAuthorizationSnapshotComparison {
  const mismatchFields: FlashAuthorizationMismatchField[] = [];
  const mismatchDetails: string[] = [];

  if (expected.sessionId !== current.sessionId) {
    mismatchFields.push('session');
    mismatchDetails.push('session: session changed');
  }

  if (expected.device !== current.device) {
    mismatchFields.push('device');
    mismatchDetails.push(`device: expected ${expected.device ?? 'none'} but saw ${current.device ?? 'none'}`);
  }

  const diskComparison = compareDiskIdentity(expected.diskFingerprint ?? {}, current.diskFingerprint ?? {});
  if (!diskComparison.ok) {
    mismatchFields.push('disk');
    mismatchDetails.push(...diskComparison.mismatches.map(item => `disk: ${item}`));
  }

  if (expected.efiStateHash !== current.efiStateHash) {
    mismatchFields.push('efi');
    mismatchDetails.push('efi: EFI state hash changed');
  }

  if (expected.payloadStateHash !== current.payloadStateHash) {
    mismatchFields.push('payload');
    mismatchDetails.push('payload: recovery or installer payload hash changed');
  }

  if (expected.hardwareFingerprint !== current.hardwareFingerprint) {
    mismatchFields.push('hardware');
    mismatchDetails.push('hardware: hardware fingerprint changed');
  }

  return {
    ok: mismatchFields.length === 0,
    mismatchFields,
    mismatchDetails,
  };
}

export function findDiskIdentityCollisions(
  expectedIdentity: Partial<DiskInfo> | DiskIdentityFingerprint,
  currentDevice: string,
  peers: Array<DiskInfo | null | undefined>,
): string[] {
  const expectedFingerprint = buildDiskIdentityFingerprint(expectedIdentity);
  const collisions: string[] = [];
  const expectedSerial = normalizeIdentityValue(expectedFingerprint.serialNumber);

  for (const peer of peers) {
    if (!peer || peer.device === currentDevice) continue;
    const peerFingerprint = buildDiskIdentityFingerprint(peer);
    const peerSerial = normalizeIdentityValue(peerFingerprint.serialNumber);
    if (expectedSerial !== undefined && peerSerial !== undefined && expectedSerial === peerSerial) {
      collisions.push(peer.device);
      continue;
    }

    const sharedFields = Object.keys(expectedFingerprint).filter((key) => expectedFingerprint[key as keyof DiskIdentityFingerprint] === peerFingerprint[key as keyof DiskIdentityFingerprint]);
    if (sharedFields.length >= 4) collisions.push(peer.device);
  }

  return collisions;
}

export function computeEfiStateHash(efiPath: string): string {
  return computeDeterministicTreeHash({
    rootPath: efiPath,
    roots: EFI_HASH_ROOTS,
    requiredDirectories: EFI_HASH_REQUIRED_DIRECTORIES,
    requiredFiles: EFI_HASH_REQUIRED_FILES,
  });
}

export function computeInstallerPayloadHash(efiPath: string): string | null {
  const rootKind = getPathKind(path.resolve(efiPath, RECOVERY_PAYLOAD_ROOT));
  if (rootKind === 'missing') return null;

  return computeDeterministicTreeHash({
    rootPath: efiPath,
    roots: [RECOVERY_PAYLOAD_ROOT],
    requiredDirectories: RECOVERY_PAYLOAD_REQUIRED_DIRECTORIES,
    requiredFiles: RECOVERY_PAYLOAD_REQUIRED_FILES,
  });
}

export function createFlashConfirmationRecord(input: {
  sessionId: string;
  device: string;
  expectedIdentity: Partial<DiskInfo> | DiskIdentityFingerprint;
  efiStateHash: string;
  payloadStateHash?: string | null;
  hardwareFingerprint?: string;
  profile?: HardwareProfile;
  now?: number;
  ttlMs?: number;
  secret?: Buffer | string;
}): FlashConfirmationRecord {
  const issuedAt = input.now ?? Date.now();
  const ttlMs = input.ttlMs ?? FLASH_CONFIRMATION_TTL_MS;
  const hardwareFingerprint = input.hardwareFingerprint ?? (input.profile ? buildHardwareFingerprint(input.profile) : null);
  if (!hardwareFingerprint) {
    throw new Error('Flash confirmation record requires a hardware fingerprint or profile');
  }

  const claims: FlashConfirmationClaims = {
    version: FLASH_CONFIRMATION_TOKEN_VERSION,
    sessionId: input.sessionId,
    nonce: crypto.randomBytes(16).toString('hex'),
    issuedAt,
    expiresAt: issuedAt + ttlMs,
    device: input.device,
    diskFingerprint: buildDiskIdentityFingerprint(input.expectedIdentity),
    efiStateHash: input.efiStateHash,
    payloadStateHash: input.payloadStateHash ?? null,
    hardwareFingerprint,
  };

  const token = signFlashConfirmationClaims(normalizeSecret(input.secret), claims);
  return buildRecordFromClaims(token, claims);
}

export function validateFlashConfirmationRecord(input: {
  record: FlashConfirmationRecord;
  snapshot: FlashAuthorizationSnapshot;
  now?: number;
}): FlashConfirmationValidation {
  if ((input.now ?? Date.now()) > input.record.expiresAt) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation expired. Re-open the confirmation dialog and verify the drive again.',
      code: 'CONFIRMATION_EXPIRED',
    };
  }

  const comparison = compareFlashAuthorizationSnapshots(
    createFlashAuthorizationSnapshotFromRecord(input.record, 'snapshot_a', input.record.issuedAt),
    input.snapshot,
  );
  if (!comparison.ok) {
    return buildFlashConfirmationValidationFromSnapshotComparison({ comparison });
  }

  return { valid: true, reason: null, code: null };
}

export function validateFlashConfirmation(input: {
  lookup: FlashConfirmationLookup;
  snapshot: FlashAuthorizationSnapshot;
  now?: number;
}): FlashConfirmationValidation {
  if (input.lookup.state === 'missing') {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flashing requires a fresh signed confirmation token. Re-open the confirmation dialog and try again.',
      code: 'CONFIRMATION_MISSING',
    };
  }
  if (input.lookup.state === 'malformed') {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation token is malformed. Re-open the confirmation dialog and confirm the drive again.',
      code: 'CONFIRMATION_MALFORMED',
    };
  }
  if (input.lookup.state === 'signature_invalid') {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation token signature is invalid. Re-open the confirmation dialog and confirm the drive again.',
      code: 'CONFIRMATION_SIGNATURE_INVALID',
    };
  }
  if (input.lookup.state === 'session_mismatch') {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation belongs to a previous or different main-process session. Re-open the confirmation dialog and confirm again.',
      code: 'CONFIRMATION_SESSION_CHANGED',
    };
  }
  if (input.lookup.state === 'unknown') {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation token is unknown to this session. Re-open the confirmation dialog and try again.',
      code: 'CONFIRMATION_UNKNOWN',
    };
  }
  if (input.lookup.state === 'consumed') {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation token has already been consumed. Re-open the confirmation dialog before any retry.',
      code: 'CONFIRMATION_CONSUMED',
    };
  }

  const record = input.lookup.record;
  if (!record) {
    return {
      valid: false,
      reason: 'SAFETY BLOCK: Flash confirmation token is unavailable. Re-open the confirmation dialog and try again.',
      code: 'CONFIRMATION_UNKNOWN',
    };
  }

  return validateFlashConfirmationRecord({
    record,
    snapshot: input.snapshot,
    now: input.now,
  });
}

export function canProceedWithFlash(context: FlashSafetyContext): FlashSafetyDecision {
  if (!context.selectedDevice) {
    return { allowed: false, reason: 'SAFETY BLOCK: No target disk is selected for flashing.', code: 'NO_DEVICE_SELECTED' };
  }
  if (!context.currentDisk) {
    return { allowed: false, reason: `SAFETY BLOCK: Target disk ${context.selectedDevice} is no longer available. Reconnect the drive and re-select it.`, code: 'TARGET_DISAPPEARED' };
  }
  if (!context.explicitUserConfirmation) {
    return { allowed: false, reason: 'SAFETY BLOCK: Flashing requires explicit user confirmation.', code: 'CONFIRMATION_REQUIRED' };
  }
  if (!context.confirmationValidated.valid) {
    return { allowed: false, reason: context.confirmationValidated.reason, code: 'CONFIRMATION_REQUIRED' };
  }
  if (!context.expectedIdentity) {
    return { allowed: false, reason: 'SAFETY BLOCK: No disk identity fingerprint was captured for this selection. Re-select the drive before flashing.', code: 'IDENTITY_FINGERPRINT_MISSING' };
  }
  if (context.currentDisk.isSystemDisk) {
    return { allowed: false, reason: `SAFETY BLOCK: ${context.currentDisk.device} is your system disk and can never be flashed.`, code: 'SYSTEM_DISK' };
  }
  if (context.currentDisk.partitionTable !== 'gpt') {
    return {
      allowed: false,
      reason: context.currentDisk.partitionTable === 'mbr'
        ? `SAFETY BLOCK: ${context.currentDisk.device} uses an MBR partition table. Only GPT targets are allowed for flashing.`
        : `SAFETY BLOCK: ${context.currentDisk.device} has an unreadable or unknown partition table. Flashing is blocked.`,
      code: 'UNSAFE_PARTITION_TABLE',
    };
  }
  if (context.currentDisk.identityConfidence === 'weak' || context.currentDisk.identityConfidence === 'ambiguous') {
    return {
      allowed: false,
      reason: `SAFETY BLOCK: Disk identity confidence for ${context.currentDisk.device} is ${context.currentDisk.identityConfidence}. Use a drive with stronger stable identity fields before flashing.`,
      code: 'IDENTITY_WEAK',
    };
  }

  const identityCheck = compareDiskIdentity(context.expectedIdentity, context.currentDisk);
  if (!identityCheck.ok) {
    return {
      allowed: false,
      reason: `SAFETY BLOCK: Target disk identity changed since selection. Mismatch: ${identityCheck.mismatches.join(', ')}.`,
      code: 'IDENTITY_CHANGED',
    };
  }
  if (identityCheck.confidence === 'ambiguous') {
    return {
      allowed: false,
      reason: `SAFETY BLOCK: Disk identity for ${context.currentDisk.device} is ambiguous. Available stable fields: ${identityCheck.fieldsUsed.join(', ') || 'none'}.`,
      code: 'IDENTITY_AMBIGUOUS',
    };
  }
  if (context.collisionDevices.length > 0) {
    return {
      allowed: false,
      reason: `SAFETY BLOCK: Disk identity collision detected with ${context.collisionDevices.join(', ')}. Disconnect duplicate drives and retry.`,
      code: 'IDENTITY_COLLISION',
    };
  }
  if (!context.biosReady) {
    return { allowed: false, reason: 'SAFETY BLOCK: BIOS readiness is no longer satisfied. Re-verify firmware settings before flashing.', code: 'BIOS_NOT_READY' };
  }
  if (!context.efiValidationClean) {
    return { allowed: false, reason: 'SAFETY BLOCK: EFI validation is no longer clean. Rebuild or revalidate the EFI before flashing.', code: 'EFI_INVALID' };
  }
  if (!context.deployGuardAllowed) {
    return { allowed: false, reason: context.deployGuardReason ?? 'SAFETY BLOCK: Deploy guard failed.', code: 'DEPLOY_GUARD_FAILED' };
  }

  return { allowed: true, reason: null, code: null };
}

export function createFlashConfirmationStore(
  now: () => number = () => Date.now(),
  sessionId: string = crypto.randomUUID(),
  secret?: Buffer | string,
) {
  const entries = new Map<string, {
    record: FlashConfirmationRecord;
    state: 'issued' | 'consumed';
    consumedAt?: number;
  }>();
  const signingSecret = normalizeSecret(secret);

  const prune = (currentTime: number): void => {
    for (const [nonce, entry] of entries) {
      if (entry.state === 'issued' && entry.record.expiresAt < currentTime) {
        entries.delete(nonce);
        continue;
      }
      if (entry.state === 'consumed' && (entry.consumedAt ?? currentTime) + FLASH_CONFIRMATION_CONSUMED_RETENTION_MS < currentTime) {
        entries.delete(nonce);
      }
    }
  };

  const resolveLookup = (token: string | null | undefined, consume: boolean): FlashConfirmationLookup => {
    const currentTime = now();
    prune(currentTime);

    if (!token) return { state: 'missing', record: null };

    const verified = verifyFlashConfirmationToken(token, signingSecret, sessionId);
    if (verified.state !== 'pending') return verified;

    const entry = entries.get(verified.record.nonce);
    if (!entry) return { state: 'unknown', record: verified.record };
    if (entry.state === 'consumed') return { state: 'consumed', record: entry.record };
    if (consume) {
      entry.state = 'consumed';
      entry.consumedAt = currentTime;
    }
    return { state: 'pending', record: entry.record };
  };

  return {
    sessionId,
    issue(input: {
      device: string;
      expectedIdentity: Partial<DiskInfo> | DiskIdentityFingerprint;
      efiStateHash: string;
      payloadStateHash?: string | null;
      hardwareFingerprint?: string;
      profile?: HardwareProfile;
      ttlMs?: number;
    }): FlashConfirmationRecord {
      const record = createFlashConfirmationRecord({
        ...input,
        sessionId,
        now: now(),
        ttlMs: input.ttlMs,
        secret: signingSecret,
      });
      entries.set(record.nonce, { record, state: 'issued' });
      return record;
    },
    peek(token: string | null | undefined): FlashConfirmationLookup {
      return resolveLookup(token, false);
    },
    consume(token: string | null | undefined): FlashConfirmationLookup {
      return resolveLookup(token, true);
    },
    clear(): void {
      entries.clear();
    },
  };
}
