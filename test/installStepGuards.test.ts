import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import { evaluateStepTransition, type StepGuardState } from '../src/lib/installStepGuards.js';
import type { FlowGuardResult } from '../src/lib/stateMachine.js';
import type { HardwareProfile } from '../electron/configGenerator.js';
import type { CompatibilityReport } from '../electron/compatibility.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i7-8700K',
    gpu: 'Intel UHD Graphics 630',
    ram: '16 GB',
    motherboard: 'Z390',
    architecture: 'Intel',
    generation: 'Coffee Lake',
    targetOS: 'macOS Sequoia 15',
    smbios: 'iMac19,1',
    kexts: [],
    ssdts: [],
    bootArgs: '',
    isLaptop: false,
    gpuDevices: [{ name: 'Intel UHD Graphics 630', vendorName: 'Intel' }],
    strategy: 'canonical',
    ...overrides,
  };
}

function makeGuard(overrides: Partial<FlowGuardResult> = {}): FlowGuardResult {
  return {
    allowed: true,
    reason: null,
    currentState: 'bios',
    biosState: 'complete',
    ...overrides,
  };
}

function makeState(overrides: Partial<StepGuardState> = {}): StepGuardState {
  const compat: CompatibilityReport = {
    level: 'supported',
    strategy: 'canonical',
    confidence: 'high',
    explanation: 'Supported',
    manualVerificationRequired: false,
    isCompatible: true,
    maxOSVersion: 'macOS Tahoe 26',
    eligibleVersions: [{ id: 'tahoe', name: 'macOS Tahoe 26', icon: 'sparkles' }],
    recommendedVersion: 'macOS Sequoia 15',
    warnings: [],
    errors: [],
    minReqMet: true,
    communityEvidence: null,
    nextActions: [],
    advisoryConfidence: {
      score: 90,
      label: 'High confidence',
      explanation: 'High confidence test fixture.',
    },
    mostLikelyFailurePoints: [],
  };

  return {
    profile: makeProfile(),
    compat,
    hasLiveHardwareContext: true,
    biosReady: true,
    biosAccepted: false,
    buildReady: true,
    efiPath: '/tmp/efi',
    biosConf: { enable: [], disable: [] },
    selectedUsb: '/dev/disk4',
    compatibilityBlocked: false,
    validationBlocked: false,
    postBuildReady: true,
    localBuildGuard: makeGuard(),
    localDeployGuard: makeGuard({ currentState: 'deploy' }),
    ...overrides,
  };
}

describe('install step guards', () => {
  test('blocks BIOS step when compatibility is blocked', () => {
    const result = evaluateStepTransition('bios', makeState({
      compatibilityBlocked: true,
    }));

    assert.equal(result.ok, false);
    assert.equal(result.redirect, 'report');
  });

  test('blocks build step when shared build guard blocks', () => {
    const result = evaluateStepTransition('building', makeState({
      localBuildGuard: makeGuard({
        allowed: false,
        reason: 'BIOS preparation must be complete before building.',
        biosState: 'blocked',
      }),
    }));

    assert.equal(result.ok, false);
    assert.equal(result.redirect, 'bios');
    assert.match(result.reason ?? '', /bios preparation/i);
  });

  test('blocks method selection when validation is blocked', () => {
    const result = evaluateStepTransition('method-select', makeState({
      validationBlocked: true,
      postBuildReady: false,
    }));

    assert.equal(result.ok, false);
    assert.equal(result.redirect, 'report');
    assert.match(result.reason ?? '', /efi validation/i);
  });

  test('keeps accepted BIOS sessions on the BIOS path when post-build prerequisites are still missing', () => {
    const result = evaluateStepTransition('method-select', makeState({
      biosReady: false,
      biosAccepted: true,
      postBuildReady: false,
      buildReady: true,
      efiPath: '/tmp/efi',
    }));

    assert.equal(result.ok, false);
    assert.equal(result.redirect, 'bios');
    assert.match(result.reason ?? '', /validated efi/i);
  });

  test('allows accepted BIOS state to enter partition prep once the EFI is ready', () => {
    const result = evaluateStepTransition('part-prep', makeState({
      biosReady: false,
      biosAccepted: true,
      postBuildReady: true,
    }));

    assert.equal(result.ok, true);
  });

  test('blocks flashing when no target drive is selected', () => {
    const result = evaluateStepTransition('flashing', makeState({
      selectedUsb: null,
    }));

    assert.equal(result.ok, false);
    assert.match(result.reason ?? '', /select a target drive/i);
  });

  test('does not allow accepted BIOS state to reach flashing', () => {
    const result = evaluateStepTransition('flashing', makeState({
      biosReady: false,
      biosAccepted: true,
      localDeployGuard: makeGuard({
        allowed: false,
        reason: 'BIOS preparation must be complete before deployment.',
        currentState: 'deploy',
        biosState: 'blocked',
      }),
    }));

    assert.equal(result.ok, false);
    assert.match(result.reason ?? '', /bios preparation/i);
  });

  test('blocks flashing when deploy guard blocks even with a drive selected', () => {
    const result = evaluateStepTransition('flashing', makeState({
      validationBlocked: true,
      localDeployGuard: makeGuard({
        allowed: false,
        reason: 'EFI validation is blocked. Rebuild before deployment.',
        currentState: 'deploy',
      }),
    }));

    assert.equal(result.ok, false);
    assert.equal(result.redirect, 'report');
    assert.match(result.reason ?? '', /rebuild before deployment/i);
  });

  test('requires a scanned profile before version selection', () => {
    const result = evaluateStepTransition('version-select', makeState({
      profile: null,
    }));

    assert.equal(result.ok, false);
    assert.equal(result.redirect, 'precheck');
  });

  test('blocks BIOS and deploy-facing steps when profile is planning-only', () => {
    const state = makeState({
      hasLiveHardwareContext: false,
    });

    const biosResult = evaluateStepTransition('bios', state);
    const buildResult = evaluateStepTransition('building', state);
    const flashResult = evaluateStepTransition('flashing', state);

    assert.equal(biosResult.ok, false);
    assert.equal(buildResult.ok, false);
    assert.equal(flashResult.ok, false);
    assert.equal(biosResult.redirect, 'report');
    assert.equal(buildResult.redirect, 'report');
    assert.equal(flashResult.redirect, 'report');
    assert.match(buildResult.reason ?? '', /planning inputs only/i);
  });

  test('live scan restores BIOS/build access after a planning-only import', () => {
    const blockedState = makeState({ hasLiveHardwareContext: false });
    const liveState = makeState({ hasLiveHardwareContext: true });

    assert.equal(evaluateStepTransition('building', blockedState).ok, false);
    assert.equal(evaluateStepTransition('building', liveState).ok, true);
    assert.equal(evaluateStepTransition('flashing', blockedState).ok, false);
    assert.equal(evaluateStepTransition('flashing', liveState).ok, true);
  });
});
