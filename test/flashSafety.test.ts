import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { buildHardwareFingerprint } from '../electron/bios/sessionState.js';
import type { HardwareProfile } from '../electron/configGenerator.js';
import type { DiskInfo } from '../electron/diskOps.js';
import {
  canProceedWithFlash,
  compareDiskIdentity,
  compareFlashAuthorizationSnapshots,
  computeEfiStateHash,
  computeInstallerPayloadHash,
  createFlashAuthorizationSnapshot,
  createFlashConfirmationStore,
  findDiskIdentityCollisions,
  validateFlashConfirmation,
  validateFlashConfirmationRecord,
  type FlashAuthorizationSnapshot,
} from '../electron/flashSafety.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i5-9600K',
    architecture: 'Intel',
    generation: 'Coffee Lake',
    coreCount: 6,
    gpu: 'Intel UHD Graphics 630',
    gpuDevices: [{ name: 'Intel UHD Graphics 630', vendorName: 'Intel' }],
    ram: '16 GB',
    motherboard: 'Gigabyte Z390 Aorus Elite',
    targetOS: 'macOS Sequoia 15',
    smbios: 'iMac19,1',
    kexts: [],
    ssdts: [],
    bootArgs: '',
    isLaptop: false,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

function makeDisk(overrides: Partial<DiskInfo> = {}): DiskInfo {
  return {
    device: '/dev/disk4',
    devicePath: '/dev/disk4',
    isSystemDisk: false,
    partitionTable: 'gpt',
    mountedPartitions: [],
    sizeBytes: 32 * 1e9,
    model: 'USB Flash',
    vendor: 'Sandisk',
    serialNumber: 'SER123',
    transport: 'usb',
    removable: true,
    identityConfidence: 'strong',
    identityFieldsUsed: ['serialNumber', 'devicePath', 'vendor', 'transport', 'partitionTable', 'sizeBytes'],
    ...overrides,
  };
}

function makeContext(overrides: Partial<Parameters<typeof canProceedWithFlash>[0]> = {}): Parameters<typeof canProceedWithFlash>[0] {
  const currentDisk = makeDisk();
  return {
    selectedDevice: currentDisk.device,
    currentDisk,
    expectedIdentity: currentDisk,
    collisionDevices: [],
    deployGuardAllowed: true,
    deployGuardReason: null,
    biosReady: true,
    efiValidationClean: true,
    explicitUserConfirmation: true,
    confirmationValidated: { valid: true, reason: null, code: null },
    ...overrides,
  };
}

function encodeBase64Url(value: string): string {
  return Buffer.from(value, 'utf8').toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

function decodeBase64Url(value: string): string {
  const normalized = value.replace(/-/g, '+').replace(/_/g, '/');
  const padding = (4 - (normalized.length % 4)) % 4;
  return Buffer.from(normalized + '='.repeat(padding), 'base64').toString('utf8');
}

function parseToken(token: string): [string, string, string, string] {
  const parts = token.split('.');
  assert.equal(parts.length, 4);
  return parts as [string, string, string, string];
}

function tamperPayload(token: string, mutate: (payload: Record<string, unknown>) => void): string {
  const [prefix, sessionId, payload, signature] = parseToken(token);
  const parsed = JSON.parse(decodeBase64Url(payload)) as Record<string, unknown>;
  mutate(parsed);
  return `${prefix}.${sessionId}.${encodeBase64Url(JSON.stringify(parsed))}.${signature}`;
}

function tamperSignature(token: string): string {
  const [prefix, sessionId, payload, signature] = parseToken(token);
  const normalized = signature.replace(/-/g, '+').replace(/_/g, '/');
  const padding = (4 - (normalized.length % 4)) % 4;
  const bytes = Buffer.from(normalized + '='.repeat(padding), 'base64');
  bytes[0] = bytes[0] ^ 0xff;
  const tampered = bytes.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
  return `${prefix}.${sessionId}.${payload}.${tampered}`;
}

function writeBaseEfiTree(root: string, order: 'default' | 'reverse' = 'default'): void {
  const directoryOperations = [
    () => fs.mkdirSync(path.join(root, 'EFI/BOOT'), { recursive: true }),
    () => fs.mkdirSync(path.join(root, 'EFI/OC/Drivers'), { recursive: true }),
    () => fs.mkdirSync(path.join(root, 'EFI/OC/ACPI'), { recursive: true }),
    () => fs.mkdirSync(path.join(root, 'EFI/OC/Kexts/Lilu.kext/Contents/MacOS'), { recursive: true }),
  ];
  const fileOperations = [
    () => fs.writeFileSync(path.join(root, 'EFI/OC/config.plist'), '<plist><dict><key>Test</key><string>A</string></dict></plist>'),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/OpenCore.efi'), Buffer.alloc(128, 1)),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/OpenCore.efi.version'), '1.0.0'),
    () => fs.writeFileSync(path.join(root, 'EFI/BOOT/BOOTx64.efi'), Buffer.alloc(64, 2)),
    () => fs.writeFileSync(path.join(root, 'EFI/BOOT/BOOTx64.efi.version'), '1.0.0'),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/OpenRuntime.efi'), Buffer.alloc(64, 3)),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/OpenRuntime.efi.version'), '1.0.0'),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/HfsPlus.efi'), Buffer.alloc(64, 4)),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/HfsPlus.efi.version'), '1.0.0'),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/ACPI/SSDT-EC.aml'), Buffer.alloc(48, 5)),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Kexts/Lilu.kext/Contents/Info.plist'), '<plist><dict><key>CFBundleExecutable</key><string>Lilu</string></dict></plist>'),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Kexts/Lilu.kext/Contents/MacOS/Lilu'), Buffer.alloc(96, 6)),
    () => fs.writeFileSync(path.join(root, 'EFI/OC/Kexts/Lilu.kext/.version'), '1.6.8'),
  ];

  const orderedDirectories = order === 'reverse' ? [...directoryOperations].reverse() : directoryOperations;
  const orderedFiles = order === 'reverse' ? [...fileOperations].reverse() : fileOperations;
  for (const operation of orderedDirectories) operation();
  for (const operation of orderedFiles) operation();
}

function writeRecoveryPayload(root: string, order: 'default' | 'reverse' = 'default'): void {
  const directoryOperations = [
    () => fs.mkdirSync(path.join(root, 'com.apple.recovery.boot'), { recursive: true }),
  ];
  const fileOperations = [
    () => fs.writeFileSync(path.join(root, 'com.apple.recovery.boot/BaseSystem.dmg'), Buffer.alloc(128, 7)),
    () => fs.writeFileSync(path.join(root, 'com.apple.recovery.boot/BaseSystem.chunklist'), Buffer.alloc(64, 8)),
    () => fs.writeFileSync(path.join(root, 'com.apple.recovery.boot/boot.efi'), Buffer.alloc(32, 9)),
  ];

  const orderedDirectories = order === 'reverse' ? [...directoryOperations].reverse() : directoryOperations;
  const orderedFiles = order === 'reverse' ? [...fileOperations].reverse() : fileOperations;
  for (const operation of orderedDirectories) operation();
  for (const operation of orderedFiles) operation();
}

function makeTempEfi(order: 'default' | 'reverse' = 'default', includeRecovery = false): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'flash-safety-'));
  writeBaseEfiTree(dir, order);
  if (includeRecovery) writeRecoveryPayload(dir, order);
  return dir;
}

function createStore(secret = 'main-process-secret', currentTime = 1000, sessionId = 'session-a') {
  return createFlashConfirmationStore(() => currentTime, sessionId, secret);
}

function makeSnapshot(overrides: Partial<FlashAuthorizationSnapshot> = {}): FlashAuthorizationSnapshot {
  const defaultHardwareFingerprint = buildHardwareFingerprint(makeProfile());
  return createFlashAuthorizationSnapshot({
    stage: overrides.stage ?? 'snapshot_b',
    capturedAt: overrides.capturedAt ?? 1001,
    sessionId: overrides.sessionId ?? 'session-a',
    device: overrides.device === undefined ? '/dev/disk4' : overrides.device,
    diskFingerprint: overrides.diskFingerprint === undefined ? makeDisk() : overrides.diskFingerprint,
    efiStateHash: overrides.efiStateHash === undefined ? 'efi-a' : overrides.efiStateHash,
    payloadStateHash: overrides.payloadStateHash === undefined ? null : overrides.payloadStateHash,
    hardwareFingerprint: overrides.hardwareFingerprint === undefined ? defaultHardwareFingerprint : overrides.hardwareFingerprint,
  });
}

function issueRecord(
  store: ReturnType<typeof createFlashConfirmationStore>,
  overrides: {
    device?: string;
    expectedIdentity?: Partial<DiskInfo>;
    efiStateHash?: string;
    payloadStateHash?: string | null;
    hardwareFingerprint?: string;
    ttlMs?: number;
  } = {},
) {
  return store.issue({
    device: overrides.device ?? '/dev/disk4',
    expectedIdentity: overrides.expectedIdentity ?? makeDisk(),
    efiStateHash: overrides.efiStateHash ?? 'efi-a',
    payloadStateHash: overrides.payloadStateHash ?? null,
    hardwareFingerprint: overrides.hardwareFingerprint ?? buildHardwareFingerprint(makeProfile()),
    ttlMs: overrides.ttlMs,
  });
}

function validateToken(
  store: ReturnType<typeof createFlashConfirmationStore>,
  token: string | null,
  overrides: {
    sessionId?: string | null;
    device?: string | null;
    currentDisk?: DiskInfo | null;
    efiStateHash?: string | null;
    payloadStateHash?: string | null;
    hardwareFingerprint?: string | null;
    consume?: boolean;
    now?: number;
  } = {},
) {
  return validateFlashConfirmation({
    lookup: overrides.consume ? store.consume(token) : store.peek(token),
    snapshot: makeSnapshot({
      sessionId: overrides.sessionId === undefined ? store.sessionId : overrides.sessionId,
      device: overrides.device === undefined ? '/dev/disk4' : overrides.device,
      diskFingerprint: overrides.currentDisk === undefined ? makeDisk() : overrides.currentDisk,
      efiStateHash: overrides.efiStateHash === undefined ? 'efi-a' : overrides.efiStateHash,
      payloadStateHash: overrides.payloadStateHash === undefined ? null : overrides.payloadStateHash,
      hardwareFingerprint: overrides.hardwareFingerprint === undefined ? buildHardwareFingerprint(makeProfile()) : overrides.hardwareFingerprint,
    }),
    now: overrides.now ?? 1001,
  });
}

describe('flash safety decision wall', () => {
  test('blocks when no disk is selected', () => {
    const result = canProceedWithFlash(makeContext({ selectedDevice: null }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'NO_DEVICE_SELECTED');
  });

  test('blocks when target disk disappears', () => {
    const result = canProceedWithFlash(makeContext({ currentDisk: null }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'TARGET_DISAPPEARED');
  });

  test('blocks flashing without explicit confirmation even with a valid token', () => {
    const result = canProceedWithFlash(makeContext({ explicitUserConfirmation: false }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'CONFIRMATION_REQUIRED');
  });

  test('blocks flashing on system disk', () => {
    const result = canProceedWithFlash(makeContext({ currentDisk: makeDisk({ isSystemDisk: true }) }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'SYSTEM_DISK');
  });

  test('blocks MBR and unknown partition tables', () => {
    assert.equal(canProceedWithFlash(makeContext({ currentDisk: makeDisk({ partitionTable: 'mbr' }) })).code, 'UNSAFE_PARTITION_TABLE');
    assert.equal(canProceedWithFlash(makeContext({ currentDisk: makeDisk({ partitionTable: 'unknown' }) })).code, 'UNSAFE_PARTITION_TABLE');
  });

  test('blocks weak identity confidence', () => {
    const result = canProceedWithFlash(makeContext({ currentDisk: makeDisk({ identityConfidence: 'weak', serialNumber: undefined }) }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'IDENTITY_WEAK');
  });

  test('blocks disk identity changes after selection', () => {
    const result = canProceedWithFlash(makeContext({
      expectedIdentity: makeDisk({ serialNumber: 'SER123' }),
      currentDisk: makeDisk({ serialNumber: 'SER999' }),
    }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'IDENTITY_CHANGED');
  });

  test('blocks multiple disk collisions when identity is not unique', () => {
    const expected = makeDisk({ serialNumber: undefined, identityConfidence: 'medium' });
    const peer = makeDisk({
      device: '/dev/disk5',
      devicePath: '/dev/disk5',
      serialNumber: undefined,
      sizeBytes: expected.sizeBytes,
      model: expected.model,
      vendor: expected.vendor,
      transport: expected.transport,
      partitionTable: expected.partitionTable,
      removable: expected.removable,
      identityConfidence: 'medium',
    });
    const collisions = findDiskIdentityCollisions(expected, '/dev/disk4', [peer]);
    const result = canProceedWithFlash(makeContext({
      currentDisk: expected,
      expectedIdentity: expected,
      collisionDevices: collisions,
    }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'IDENTITY_COLLISION');
  });

  test('blocks flashing if EFI becomes invalid after initial validation', () => {
    const result = canProceedWithFlash(makeContext({ efiValidationClean: false }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'EFI_INVALID');
  });

  test('blocks flashing if BIOS readiness changes before deploy', () => {
    const result = canProceedWithFlash(makeContext({ biosReady: false }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'BIOS_NOT_READY');
  });

  test('blocks flashing if deploy guard fails', () => {
    const result = canProceedWithFlash(makeContext({ deployGuardAllowed: false, deployGuardReason: 'Blocked by release guard' }));
    assert.equal(result.allowed, false);
    assert.equal(result.code, 'DEPLOY_GUARD_FAILED');
  });
});

describe('signed flash confirmation tokens', () => {
  test('renderer cannot forge a valid token signed with another secret', () => {
    const mainStore = createStore('main-secret');
    const attackerStore = createStore('attacker-secret');
    const forged = issueRecord(attackerStore).token;
    const validation = validateToken(mainStore, forged);
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_SIGNATURE_INVALID');
  });

  test('cross-session tokens are always rejected', () => {
    const issuingStore = createStore('shared-secret', 1000, 'session-a');
    const nextSessionStore = createStore('shared-secret', 1000, 'session-b');
    const record = issueRecord(issuingStore);
    const validation = validateToken(nextSessionStore, record.token, { sessionId: 'session-b' });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_SESSION_CHANGED');
  });

  test('tampering with payload invalidates token', () => {
    const store = createStore();
    const record = issueRecord(store, { payloadStateHash: 'payload-a' });
    const tampered = tamperPayload(record.token, (payload) => {
      payload.payloadStateHash = 'payload-b';
    });
    const validation = validateToken(store, tampered, { payloadStateHash: 'payload-a' });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_SIGNATURE_INVALID');
  });

  test('tampering with signature invalidates token', () => {
    const store = createStore();
    const record = issueRecord(store);
    const validation = validateToken(store, tamperSignature(record.token));
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_SIGNATURE_INVALID');
  });

  test('malformed token fails', () => {
    const store = createStore();
    const validation = validateToken(store, 'not-a-valid-token');
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_MALFORMED');
  });

  test('expired token fails', () => {
    const store = createStore('main-process-secret', 1000);
    const record = issueRecord(store, { ttlMs: 10 });
    const validation = validateToken(store, record.token, { now: 2000 });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_EXPIRED');
  });

  test('consumed token reuse fails', () => {
    const store = createStore();
    const record = issueRecord(store);
    const firstUse = validateToken(store, record.token, { consume: true });
    assert.equal(firstUse.valid, true);

    const replay = validateToken(store, record.token, { consume: true });
    assert.equal(replay.valid, false);
    assert.equal(replay.code, 'CONFIRMATION_CONSUMED');
  });

  test('double consumption is atomic under concurrent attempts', async () => {
    const store = createStore();
    const record = issueRecord(store);
    const results = await Promise.all(
      Array.from({ length: 6 }, () => Promise.resolve().then(() => store.consume(record.token).state)),
    );

    assert.equal(results.filter((state) => state === 'pending').length, 1);
    assert.equal(results.filter((state) => state === 'consumed').length, 5);
  });

  test('concurrent execution attempts fail safely after first consumption', async () => {
    const store = createStore();
    const record = issueRecord(store);
    const snapshot = makeSnapshot();
    const validations = await Promise.all(
      Array.from({ length: 4 }, () => Promise.resolve().then(() => validateFlashConfirmation({
        lookup: store.consume(record.token),
        snapshot,
        now: 1001,
      }))),
    );

    assert.equal(validations.filter((result) => result.valid).length, 1);
    assert.equal(validations.filter((result) => result.code === 'CONFIRMATION_CONSUMED').length, 3);
  });
});

describe('snapshot validation', () => {
  test('token validates when snapshot A and B match', () => {
    const store = createStore();
    const record = issueRecord(store, { payloadStateHash: 'payload-a' });
    const validation = validateToken(store, record.token, { payloadStateHash: 'payload-a' });
    assert.equal(validation.valid, true);
  });

  test('token invalidates when selected device changes', () => {
    const store = createStore();
    const record = issueRecord(store);
    const validation = validateToken(store, record.token, { device: '/dev/disk5' });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_DEVICE_CHANGED');
  });

  test('token invalidates when disk identity changes between snapshots A and B', () => {
    const store = createStore();
    const record = issueRecord(store);
    const validation = validateToken(store, record.token, {
      currentDisk: makeDisk({ serialNumber: 'SER999' }),
    });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_DISK_CHANGED');
  });

  test('token invalidates when EFI changes between snapshots A and B', () => {
    const store = createStore();
    const record = issueRecord(store);
    const validation = validateToken(store, record.token, { efiStateHash: 'efi-b' });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_EFI_CHANGED');
  });

  test('payload hash mismatch invalidates authorization', () => {
    const store = createStore();
    const record = issueRecord(store, { payloadStateHash: 'payload-a' });
    const validation = validateToken(store, record.token, { payloadStateHash: 'payload-b' });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_PAYLOAD_CHANGED');
  });

  test('token invalidates when hardware fingerprint changes between snapshots A and B', () => {
    const store = createStore();
    const record = issueRecord(store);
    const validation = validateToken(store, record.token, {
      hardwareFingerprint: buildHardwareFingerprint(makeProfile({ motherboard: 'Board B' })),
    });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_HARDWARE_CHANGED');
  });

  test('token invalidates when target disappears between stages', () => {
    const store = createStore();
    const record = issueRecord(store);
    const validation = validateToken(store, record.token, { currentDisk: null });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_DISK_CHANGED');
  });

  test('pre-write validation blocks when disk changes between snapshots B and C', () => {
    const record = issueRecord(createStore());
    const snapshotC = makeSnapshot({
      stage: 'snapshot_c',
      diskFingerprint: makeDisk({ serialNumber: 'SER999' }),
    });
    const validation = validateFlashConfirmationRecord({ record, snapshot: snapshotC, now: 1001 });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_DISK_CHANGED');
  });

  test('pre-write validation blocks when EFI changes between snapshots B and C', () => {
    const record = issueRecord(createStore());
    const snapshotC = makeSnapshot({ stage: 'snapshot_c', efiStateHash: 'efi-b' });
    const validation = validateFlashConfirmationRecord({ record, snapshot: snapshotC, now: 1001 });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_EFI_CHANGED');
  });

  test('pre-write validation blocks when hardware changes between snapshots B and C', () => {
    const record = issueRecord(createStore());
    const snapshotC = makeSnapshot({
      stage: 'snapshot_c',
      hardwareFingerprint: buildHardwareFingerprint(makeProfile({ motherboard: 'Board B' })),
    });
    const validation = validateFlashConfirmationRecord({ record, snapshot: snapshotC, now: 1001 });
    assert.equal(validation.valid, false);
    assert.equal(validation.code, 'CONFIRMATION_HARDWARE_CHANGED');
  });

  test('explicit B/C snapshot comparisons expose mismatch fields', () => {
    const snapshotB = makeSnapshot({ stage: 'snapshot_b', payloadStateHash: 'payload-a' });
    const snapshotC = makeSnapshot({ stage: 'snapshot_c', payloadStateHash: 'payload-b' });
    const comparison = compareFlashAuthorizationSnapshots(snapshotB, snapshotC);
    assert.equal(comparison.ok, false);
    assert.deepEqual(comparison.mismatchFields, ['payload']);
  });
});

describe('flash safety helpers', () => {
  test('compareDiskIdentity detects mismatched serial', () => {
    const result = compareDiskIdentity(makeDisk({ serialNumber: 'SER123' }), makeDisk({ serialNumber: 'SER999' }));
    assert.equal(result.ok, false);
    assert.ok(result.mismatches.some(item => /serial/i.test(item)));
  });

  test('duplicate serial numbers are treated as collisions', () => {
    const collisions = findDiskIdentityCollisions(
      makeDisk({ serialNumber: 'DUPLICATE-1' }),
      '/dev/disk4',
      [makeDisk({ device: '/dev/disk5', devicePath: '/dev/disk5', serialNumber: 'DUPLICATE-1' })],
    );
    assert.deepEqual(collisions, ['/dev/disk5']);
  });
});

describe('EFI hash determinism', () => {
  test('identical EFI trees produce identical hash', () => {
    const left = makeTempEfi('default');
    const right = makeTempEfi('default');
    try {
      assert.equal(computeEfiStateHash(left), computeEfiStateHash(right));
    } finally {
      fs.rmSync(left, { recursive: true, force: true });
      fs.rmSync(right, { recursive: true, force: true });
    }
  });

  test('changing config.plist changes hash', () => {
    const dir = makeTempEfi();
    try {
      const before = computeEfiStateHash(dir);
      fs.writeFileSync(path.join(dir, 'EFI/OC/config.plist'), '<plist><dict><key>Test</key><string>B</string></dict></plist>');
      const after = computeEfiStateHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('changing kext contents changes hash', () => {
    const dir = makeTempEfi();
    try {
      const before = computeEfiStateHash(dir);
      fs.writeFileSync(path.join(dir, 'EFI/OC/Kexts/Lilu.kext/Contents/MacOS/Lilu'), Buffer.alloc(96, 9));
      const after = computeEfiStateHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('changing ACPI contents changes hash', () => {
    const dir = makeTempEfi();
    try {
      const before = computeEfiStateHash(dir);
      fs.writeFileSync(path.join(dir, 'EFI/OC/ACPI/SSDT-EC.aml'), Buffer.alloc(48, 8));
      const after = computeEfiStateHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('changing a driver changes hash', () => {
    const dir = makeTempEfi();
    try {
      const before = computeEfiStateHash(dir);
      fs.writeFileSync(path.join(dir, 'EFI/OC/Drivers/OpenRuntime.efi'), Buffer.alloc(64, 7));
      const after = computeEfiStateHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('file creation order differences do not change hash', () => {
    const left = makeTempEfi('default');
    const right = makeTempEfi('reverse');
    try {
      assert.equal(computeEfiStateHash(left), computeEfiStateHash(right));
    } finally {
      fs.rmSync(left, { recursive: true, force: true });
      fs.rmSync(right, { recursive: true, force: true });
    }
  });

  test('path traversal ordering differences do not change hash', () => {
    const left = makeTempEfi('default');
    const right = fs.mkdtempSync(path.join(os.tmpdir(), 'flash-safety-'));
    try {
      writeBaseEfiTree(right, 'default');
      fs.writeFileSync(path.join(right, 'EFI/OC/Drivers/ZDriver.efi'), Buffer.alloc(8, 1));
      fs.writeFileSync(path.join(right, 'EFI/OC/Drivers/ADriver.efi'), Buffer.alloc(8, 2));

      fs.writeFileSync(path.join(left, 'EFI/OC/Drivers/ADriver.efi'), Buffer.alloc(8, 2));
      fs.writeFileSync(path.join(left, 'EFI/OC/Drivers/ZDriver.efi'), Buffer.alloc(8, 1));

      assert.equal(computeEfiStateHash(left), computeEfiStateHash(right));
    } finally {
      fs.rmSync(left, { recursive: true, force: true });
      fs.rmSync(right, { recursive: true, force: true });
    }
  });

  test('missing a required file changes hash', () => {
    const dir = makeTempEfi();
    try {
      const before = computeEfiStateHash(dir);
      fs.rmSync(path.join(dir, 'EFI/OC/OpenCore.efi'));
      const after = computeEfiStateHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('duplicated directory content changes hash', () => {
    const dir = makeTempEfi();
    try {
      const before = computeEfiStateHash(dir);
      fs.cpSync(
        path.join(dir, 'EFI/OC/Kexts/Lilu.kext'),
        path.join(dir, 'EFI/OC/Kexts/LiluCopy.kext'),
        { recursive: true },
      );
      const after = computeEfiStateHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('symlink entries are hashed deterministically', () => {
    const left = makeTempEfi();
    const right = makeTempEfi();
    try {
      fs.symlinkSync('../OC/OpenCore.efi', path.join(left, 'EFI/BOOT/OpenCoreAlias.efi'));
      fs.symlinkSync('../OC/OpenCore.efi', path.join(right, 'EFI/BOOT/OpenCoreAlias.efi'));
      assert.equal(computeEfiStateHash(left), computeEfiStateHash(right));

      const before = computeEfiStateHash(right);
      fs.rmSync(path.join(right, 'EFI/BOOT/OpenCoreAlias.efi'));
      fs.symlinkSync('../OC/Drivers/OpenRuntime.efi', path.join(right, 'EFI/BOOT/OpenCoreAlias.efi'));
      const after = computeEfiStateHash(right);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(left, { recursive: true, force: true });
      fs.rmSync(right, { recursive: true, force: true });
    }
  });

  test('hidden metadata files do not influence hash', () => {
    const left = makeTempEfi();
    const right = makeTempEfi();
    try {
      fs.writeFileSync(path.join(right, '.DS_Store'), 'ignored');
      fs.writeFileSync(path.join(right, 'EFI/OC/._config.plist'), 'ignored');
      assert.equal(computeEfiStateHash(left), computeEfiStateHash(right));
    } finally {
      fs.rmSync(left, { recursive: true, force: true });
      fs.rmSync(right, { recursive: true, force: true });
    }
  });
});

describe('recovery payload hashing', () => {
  test('same recovery payload tree produces the same hash', () => {
    const left = makeTempEfi('default', true);
    const right = makeTempEfi('reverse', true);
    try {
      assert.equal(computeInstallerPayloadHash(left), computeInstallerPayloadHash(right));
    } finally {
      fs.rmSync(left, { recursive: true, force: true });
      fs.rmSync(right, { recursive: true, force: true });
    }
  });

  test('payload hash changes when BaseSystem.dmg changes', () => {
    const dir = makeTempEfi('default', true);
    try {
      const before = computeInstallerPayloadHash(dir);
      fs.writeFileSync(path.join(dir, 'com.apple.recovery.boot/BaseSystem.dmg'), Buffer.alloc(128, 10));
      const after = computeInstallerPayloadHash(dir);
      assert.notEqual(before, after);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('payload hash is null when no recovery payload exists', () => {
    const dir = makeTempEfi();
    try {
      assert.equal(computeInstallerPayloadHash(dir), null);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe('hardware fingerprint stability', () => {
  test('removable-device churn and transient build fields do not change the hardware fingerprint', () => {
    const base = makeProfile();
    const mutated = {
      ...base,
      ram: '64 GB',
      targetOS: 'macOS Tahoe 26',
      smbios: 'MacPro7,1',
      kexts: ['WhateverGreen'],
      ssdts: ['SSDT-EC'],
      bootArgs: '-v keepsyms=1',
      removableDevices: ['/dev/disk9', '/dev/disk10'],
    } as HardwareProfile & { removableDevices: string[] };

    assert.equal(buildHardwareFingerprint(base), buildHardwareFingerprint(mutated));
  });

  test('GPU enumeration order does not change the hardware fingerprint', () => {
    const left = makeProfile({
      gpuDevices: [
        { name: 'AMD Radeon RX 6600 XT', vendorName: 'AMD', vendorId: '1002', deviceId: '73ff' },
        { name: 'Intel UHD Graphics 630', vendorName: 'Intel', vendorId: '8086', deviceId: '3e98' },
      ],
      gpu: 'AMD Radeon RX 6600 XT',
    });
    const right = makeProfile({
      gpuDevices: [
        { name: 'Intel UHD Graphics 630', vendorName: 'Intel', vendorId: '8086', deviceId: '3e98' },
        { name: 'AMD Radeon RX 6600 XT', vendorName: 'AMD', vendorId: '1002', deviceId: '73ff' },
      ],
      gpu: 'AMD Radeon RX 6600 XT',
    });

    assert.equal(buildHardwareFingerprint(left), buildHardwareFingerprint(right));
  });
});
