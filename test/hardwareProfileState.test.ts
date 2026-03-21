import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import type { HardwareProfile } from '../electron/configGenerator.js';
import type { HardwareProfileArtifact } from '../electron/hardwareProfileArtifact.js';
import {
  canRestoreLatestScannedArtifact,
  reconcileHardwareScanProfile,
} from '../electron/hardwareProfileState.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i5-4300U',
    architecture: 'Intel',
    generation: 'Haswell',
    coreCount: 2,
    gpu: 'Intel HD Graphics 4400',
    gpuDevices: [{
      name: 'Intel HD Graphics 4400',
      vendorName: 'Intel',
      vendorId: '8086',
      deviceId: '0416',
    }],
    ram: '8 GB',
    motherboard: 'Lenovo ThinkPad T440',
    targetOS: 'macOS Monterey 12',
    smbios: 'MacBookPro11,1',
    kexts: ['Lilu.kext'],
    ssdts: ['SSDT-EC-USBX.aml'],
    bootArgs: '',
    isLaptop: true,
    isVM: false,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

function makeArtifact(
  source: HardwareProfileArtifact['source'],
  profile: HardwareProfile = makeProfile(),
): HardwareProfileArtifact {
  return {
    kind: 'hardware_profile_artifact',
    version: 1,
    capturedAt: 1_710_000_000_000,
    source,
    digest: `${source}-digest`,
    profile,
    interpretation: null,
  };
}

describe('hardwareProfileState', () => {
  test('successful hardware state does not regress to unknown from a weaker later scan on the same machine', () => {
    const current = makeProfile();
    const incoming = makeProfile({
      gpu: 'Unknown GPU',
      gpuDevices: [],
      ram: 'Unknown RAM',
      motherboard: 'Unknown Board',
      scanConfidence: 'low',
    });

    const reconciled = reconcileHardwareScanProfile(current, incoming);

    assert.equal(reconciled.likelySameMachine, true);
    assert.equal(reconciled.usedExistingFields, true);
    assert.equal(reconciled.shouldInvalidateBuild, false);
    assert.equal(reconciled.profile.gpu, current.gpu);
    assert.equal(reconciled.profile.motherboard, current.motherboard);
    assert.equal(reconciled.profile.targetOS, current.targetOS);
  });

  test('a T440-style partial rescan keeps valid identity when cpu and gpu still match', () => {
    const current = makeProfile();
    const incoming = makeProfile({
      ram: 'Unknown',
      motherboard: 'Unknown',
      gpuDevices: [{
        name: 'Intel HD Graphics 4400',
        vendorName: 'Intel',
      }],
      scanConfidence: 'medium',
    });

    const reconciled = reconcileHardwareScanProfile(current, incoming);

    assert.equal(reconciled.profile.cpu, 'Intel Core i5-4300U');
    assert.equal(reconciled.profile.gpu, 'Intel HD Graphics 4400');
    assert.equal(reconciled.profile.motherboard, 'Lenovo ThinkPad T440');
    assert.equal(reconciled.profile.isLaptop, true);
  });

  test('real hardware changes still invalidate the existing build identity', () => {
    const current = makeProfile();
    const incoming = makeProfile({
      cpu: 'Intel Core i7-7700',
      generation: 'Kaby Lake',
      gpu: 'Intel HD Graphics 630',
      gpuDevices: [{
        name: 'Intel HD Graphics 630',
        vendorName: 'Intel',
        vendorId: '8086',
        deviceId: '5912',
      }],
      motherboard: 'HP ProDesk 400 G3 SFF',
      isLaptop: false,
    });

    const reconciled = reconcileHardwareScanProfile(current, incoming);

    assert.equal(reconciled.likelySameMachine, false);
    assert.equal(reconciled.usedExistingFields, false);
    assert.equal(reconciled.shouldInvalidateBuild, true);
    assert.equal(reconciled.profile.cpu, 'Intel Core i7-7700');
  });

  test('startup restore only reuses safe live or legacy scan artifacts', () => {
    assert.equal(canRestoreLatestScannedArtifact(makeArtifact('live_scan')), true);
    assert.equal(canRestoreLatestScannedArtifact(makeArtifact('legacy_scan')), true);
    assert.equal(canRestoreLatestScannedArtifact(makeArtifact('manual_planning')), false);
    assert.equal(canRestoreLatestScannedArtifact(makeArtifact('imported_artifact')), false);
  });
});
