import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  buildHardwareProfileArtifactDigest,
  createHardwareProfileArtifact,
  parseHardwareProfileArtifact,
  type HardwareProfileInterpretationMetadata,
} from '../electron/hardwareProfileArtifact.js';
import { createHardwareProfileStore } from '../electron/hardwareProfileStore.js';
import type { HardwareProfile } from '../electron/configGenerator.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i7-8700K',
    architecture: 'Intel',
    generation: 'Coffee Lake',
    coreCount: 6,
    gpu: 'Intel UHD Graphics 630',
    gpuDevices: [
      {
        name: 'Intel UHD Graphics 630',
        vendorName: 'Intel',
        vendorId: '0x8086',
        deviceId: '0x3E92',
      },
    ],
    ram: '16 GB',
    motherboard: 'Z390 AORUS PRO',
    targetOS: 'macOS Sequoia 15',
    smbios: 'iMac19,1',
    kexts: ['Lilu', 'WhateverGreen'],
    ssdts: ['SSDT-EC-USBX'],
    bootArgs: '-v',
    isLaptop: false,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

function makeInterpretation(
  overrides: Partial<HardwareProfileInterpretationMetadata> = {},
): HardwareProfileInterpretationMetadata {
  return {
    overallConfidence: 'high',
    summary: 'Direct hardware scan completed.',
    manualVerificationNeeded: ['Verify BIOS settings'],
    ...overrides,
  };
}

function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `hardware-profile-${prefix}-`));
}

describe('hardware profile artifacts', () => {
  test('produces a stable digest for equivalent normalized input', () => {
    const artifactA = createHardwareProfileArtifact({
      profile: makeProfile({
        kexts: ['WhateverGreen', 'Lilu', 'Lilu'],
        ssdts: ['SSDT-EC-USBX', 'SSDT-EC-USBX'],
      }),
      interpretation: makeInterpretation({
        manualVerificationNeeded: ['Verify BIOS settings', 'Verify BIOS settings'],
      }),
      capturedAt: 1_700_000_000_000,
      source: 'live_scan',
    });
    const artifactB = createHardwareProfileArtifact({
      profile: makeProfile({
        kexts: ['Lilu', 'WhateverGreen'],
        ssdts: ['SSDT-EC-USBX'],
      }),
      interpretation: makeInterpretation(),
      capturedAt: 1_700_000_000_111,
      source: 'manual_planning',
    });

    assert.equal(
      artifactA.digest,
      artifactB.digest,
    );
    assert.equal(
      buildHardwareProfileArtifactDigest({ profile: artifactA.profile, interpretation: artifactA.interpretation }),
      artifactA.digest,
    );
  });

  test('rejects malformed, unknown, and incomplete artifact fields', () => {
    const artifact = createHardwareProfileArtifact({
      profile: makeProfile(),
      interpretation: makeInterpretation(),
      capturedAt: 1_700_000_000_000,
      source: 'live_scan',
    });

    assert.throws(() => parseHardwareProfileArtifact({
      ...artifact,
      unexpected: true,
    }), /unknown field/i);

    assert.throws(() => parseHardwareProfileArtifact({
      ...artifact,
      profile: {
        ...artifact.profile,
        injected: 'unsafe',
      },
    }), /unknown field/i);

    const withoutTimestamp = { ...artifact } as Record<string, unknown>;
    delete withoutTimestamp.capturedAt;
    assert.throws(() => parseHardwareProfileArtifact(withoutTimestamp), /capturedAt/i);
  });

  test('round-trips exported and imported artifacts through the store', () => {
    const dir = makeTempDir('roundtrip');
    const store = createHardwareProfileStore(dir);
    const artifact = createHardwareProfileArtifact({
      profile: makeProfile(),
      interpretation: makeInterpretation(),
      capturedAt: 1_700_000_000_000,
      source: 'live_scan',
    });

    const saved = store.saveLatest(artifact);
    const exportPath = path.join(dir, 'exports', 'artifact.json');
    store.exportArtifact(saved, exportPath);
    const imported = store.importFromFile(exportPath);

    assert.deepEqual(imported, saved);
    assert.equal(store.loadLatest()?.digest, saved.digest);
  });

  test('quarantines corrupted latest artifacts and handles unsupported versions', () => {
    const dir = makeTempDir('corrupt');
    const store = createHardwareProfileStore(dir);
    const artifact = createHardwareProfileArtifact({
      profile: makeProfile(),
      interpretation: makeInterpretation(),
      capturedAt: 1_700_000_000_000,
      source: 'live_scan',
    });

    store.saveLatest(artifact);
    fs.writeFileSync(store.latestPath, '{"not":"valid"}');
    assert.equal(store.loadLatest(), null);

    const invalidEntries = fs.readdirSync(path.join(store.rootPath, 'invalid'));
    assert.equal(invalidEntries.length, 1);

    const versionedPath = path.join(dir, 'unsupported-version.json');
    const invalidVersion = {
      ...artifact,
      version: 999,
    };
    fs.writeFileSync(versionedPath, JSON.stringify(invalidVersion, null, 2));
    assert.throws(() => store.importFromFile(versionedPath), /artifact\.version/i);
  });

  test('parses imported artifacts and preserves digest integrity', () => {
    const artifact = createHardwareProfileArtifact({
      profile: makeProfile({
        kexts: ['WhateverGreen', 'Lilu'],
      }),
      interpretation: makeInterpretation(),
      capturedAt: 1_700_000_000_000,
      source: 'imported_artifact',
    });
    const parsed = parseHardwareProfileArtifact(artifact);

    assert.equal(parsed.source, 'imported_artifact');
    assert.equal(parsed.digest, artifact.digest);
    assert.equal(artifact.digest.length, 64);
  });
});
