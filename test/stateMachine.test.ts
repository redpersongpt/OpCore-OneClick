import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import {
  createBiosFlowMachine,
  createReleaseFlowMachine,
  createMachine,
  BIOS_FLOW_TRANSITIONS,
  RELEASE_FLOW_TRANSITIONS,
  invalidateOnTargetChange,
  canDeploy,
  canBuild,
  deriveBiosFlowState,
  deriveReleaseFlowState,
  evaluateBuildGuard,
  evaluateDeployGuard,
} from '../src/lib/stateMachine.js';

describe('Generic state machine', () => {
  test('starts at initial state', () => {
    const m = createMachine('a', { a: { GO: 'b' }, b: {} } as any);
    assert.equal(m.current(), 'a');
  });

  test('transitions correctly', () => {
    const m = createMachine('a', { a: { GO: 'b' }, b: {} } as any);
    const next = m.send('GO');
    assert.equal(next, 'b');
    assert.equal(m.current(), 'b');
  });

  test('throws on invalid transition', () => {
    const m = createMachine('a', { a: { GO: 'b' }, b: {} } as any);
    assert.throws(() => m.send('INVALID' as any), /Invalid transition/);
  });

  test('can() returns false for invalid transition', () => {
    const m = createMachine('a', { a: { GO: 'b' }, b: {} } as any);
    assert.equal(m.can('GO'), true);
    assert.equal(m.can('INVALID' as any), false);
  });

  test('matches() works', () => {
    type S = 'a' | 'b';
    type E = 'GO';
    const m = createMachine<S, E>('a', { a: { GO: 'b' }, b: {} });
    assert.equal(m.matches('a'), true);
    assert.equal(m.matches('b'), false);
    m.send('GO');
    assert.equal(m.matches('b'), true);
    assert.equal(m.matches('a', 'b'), true);
  });

  test('snapshot records history', () => {
    const m = createMachine('a', { a: { GO: 'b' }, b: { NEXT: 'c' }, c: {} } as any);
    m.send('GO');
    m.send('NEXT');
    const snap = m.snapshot();
    assert.deepEqual(snap.history, ['a', 'b', 'c']);
    assert.equal(snap.state, 'c');
  });

  test('reset returns to initial', () => {
    const m = createMachine('a', { a: { GO: 'b' }, b: {} } as any);
    m.send('GO');
    m.reset();
    assert.equal(m.current(), 'a');
  });

  test('guarded transition blocks when guard returns false', () => {
    const m = createMachine('a', {
      a: { GO: { target: 'b', guard: () => false } },
      b: {},
    } as any);
    assert.equal(m.can('GO'), false);
    assert.throws(() => m.send('GO'), /Invalid transition/);
  });

  test('guarded transition allows when guard returns true', () => {
    const m = createMachine('a', {
      a: { GO: { target: 'b', guard: () => true } },
      b: {},
    } as any);
    assert.equal(m.can('GO'), true);
    assert.equal(m.send('GO'), 'b');
  });
});

describe('BIOS flow state machine', () => {
  test('idle -> planned -> verifying -> complete (manual flow)', () => {
    const m = createBiosFlowMachine();
    assert.equal(m.current(), 'idle');
    m.send('PLAN');
    assert.equal(m.current(), 'planned');
    m.send('MARK_MANUAL_COMPLETE');
    assert.equal(m.current(), 'verifying');
    m.send('VERIFY_COMPLETE');
    assert.equal(m.current(), 'complete');
  });

  test('planned -> ready_for_reboot -> rebooting_to_firmware -> awaiting_return -> resumed (reboot flow)', () => {
    const m = createBiosFlowMachine('planned');
    m.send('REQUEST_REBOOT');
    assert.equal(m.current(), 'ready_for_reboot');
    m.send('REBOOT_ACCEPTED');
    assert.equal(m.current(), 'rebooting_to_firmware');
    m.send('USER_RETURNED');
    assert.equal(m.current(), 'awaiting_return');
    m.send('START_VERIFY');
    assert.equal(m.current(), 'resumed_from_firmware');
    m.send('VERIFY_COMPLETE');
    assert.equal(m.current(), 'complete');
  });

  test('reboot rejection goes to planned', () => {
    const m = createBiosFlowMachine('planned');
    m.send('REQUEST_REBOOT');
    m.send('REBOOT_REJECTED');
    assert.equal(m.current(), 'planned');
  });

  test('reboot unsupported goes to unsupported_host', () => {
    const m = createBiosFlowMachine('planned');
    m.send('REQUEST_REBOOT');
    m.send('REBOOT_UNSUPPORTED');
    assert.equal(m.current(), 'unsupported_host');
  });

  test('unsupported_host can be resolved via manual complete', () => {
    const m = createBiosFlowMachine('unsupported_host');
    m.send('MARK_MANUAL_COMPLETE');
    assert.equal(m.current(), 'verifying');
    m.send('VERIFY_COMPLETE');
    assert.equal(m.current(), 'complete');
  });

  test('blocked state allows re-verification', () => {
    const m = createBiosFlowMachine('blocked');
    m.send('MARK_MANUAL_COMPLETE');
    assert.equal(m.current(), 'verifying');
  });

  test('partially_verified allows another reboot attempt', () => {
    const m = createBiosFlowMachine('partially_verified');
    m.send('REQUEST_REBOOT');
    assert.equal(m.current(), 'ready_for_reboot');
  });

  test('cannot move from rebooting_to_firmware without USER_RETURNED', () => {
    const m = createBiosFlowMachine('rebooting_to_firmware');
    assert.equal(m.can('VERIFY_COMPLETE'), false);
    assert.equal(m.can('MARK_MANUAL_COMPLETE'), false);
    assert.equal(m.can('USER_RETURNED'), true);
  });

  test('cannot skip awaiting_return to complete', () => {
    const m = createBiosFlowMachine('awaiting_return');
    assert.equal(m.can('VERIFY_COMPLETE'), false);
  });

  test('RESET from any non-terminal state goes to idle', () => {
    for (const state of ['planned', 'auto_applying', 'ready_for_reboot', 'awaiting_return', 'resumed_from_firmware', 'verifying', 'partially_verified', 'complete', 'blocked', 'unsupported_host'] as const) {
      const m = createBiosFlowMachine(state);
      m.send('RESET');
      assert.equal(m.current(), 'idle', `RESET from ${state} should go to idle`);
    }
  });

  test('auto_applying -> planned on done', () => {
    const m = createBiosFlowMachine('planned');
    m.send('START_AUTO_APPLY');
    assert.equal(m.current(), 'auto_applying');
    m.send('AUTO_APPLY_DONE');
    assert.equal(m.current(), 'planned');
  });

  test('no fake awaiting_return from unsupported firmware restart', () => {
    const m = createBiosFlowMachine('planned');
    m.send('REQUEST_REBOOT');
    m.send('REBOOT_UNSUPPORTED');
    assert.equal(m.current(), 'unsupported_host');
    // Cannot reach awaiting_return from here
    assert.equal(m.can('USER_RETURNED'), false);
  });
});

describe('Release flow state machine', () => {
  test('full happy path: scan -> complete', () => {
    const m = createReleaseFlowMachine();
    m.send('SCAN_COMPLETE');
    assert.equal(m.current(), 'compatibility');
    m.send('COMPATIBILITY_PASS');
    assert.equal(m.current(), 'bios');
    m.send('BIOS_COMPLETE');
    assert.equal(m.current(), 'build');
    m.send('BUILD_COMPLETE');
    assert.equal(m.current(), 'validate');
    m.send('VALIDATION_PASS');
    assert.equal(m.current(), 'method');
    m.send('METHOD_SELECTED');
    assert.equal(m.current(), 'deploy');
    m.send('DEPLOY_COMPLETE');
    assert.equal(m.current(), 'complete');
  });

  test('compatibility fail goes to blocked', () => {
    const m = createReleaseFlowMachine('compatibility');
    m.send('COMPATIBILITY_FAIL');
    assert.equal(m.current(), 'blocked');
  });

  test('INVALIDATE_BUILD from build returns to bios', () => {
    const m = createReleaseFlowMachine('build');
    m.send('INVALIDATE_BUILD');
    assert.equal(m.current(), 'bios');
  });

  test('INVALIDATE_BUILD from validate returns to bios', () => {
    const m = createReleaseFlowMachine('validate');
    m.send('INVALIDATE_BUILD');
    assert.equal(m.current(), 'bios');
  });

  test('INVALIDATE_BUILD from method returns to bios', () => {
    const m = createReleaseFlowMachine('method');
    m.send('INVALIDATE_BUILD');
    assert.equal(m.current(), 'bios');
  });

  test('INVALIDATE_BUILD from complete returns to bios', () => {
    const m = createReleaseFlowMachine('complete');
    m.send('INVALIDATE_BUILD');
    assert.equal(m.current(), 'bios');
  });

  test('blocked can recover via COMPATIBILITY_PASS', () => {
    const m = createReleaseFlowMachine('blocked');
    m.send('COMPATIBILITY_PASS');
    assert.equal(m.current(), 'bios');
  });

  test('blocked can recover via BIOS_COMPLETE', () => {
    const m = createReleaseFlowMachine('blocked');
    m.send('BIOS_COMPLETE');
    assert.equal(m.current(), 'build');
  });

  test('cannot deploy without completing prior steps', () => {
    const m = createReleaseFlowMachine('build');
    assert.equal(m.can('METHOD_SELECTED'), false);
    assert.equal(m.can('DEPLOY_COMPLETE'), false);
  });

  test('RESET from any state returns to scan', () => {
    for (const state of ['compatibility', 'bios', 'build', 'validate', 'method', 'deploy', 'complete', 'blocked'] as const) {
      const m = createReleaseFlowMachine(state);
      m.send('RESET');
      assert.equal(m.current(), 'scan', `RESET from ${state} should go to scan`);
    }
  });
});

describe('Invalidation rules', () => {
  test('invalidateOnTargetChange moves release flow back to bios', () => {
    const releaseFlow = createReleaseFlowMachine('validate');
    const biosFlow = createBiosFlowMachine('complete');
    invalidateOnTargetChange({ releaseFlow, biosFlow });
    assert.equal(releaseFlow.current(), 'bios');
  });

  test('canDeploy requires bios complete and release at deploy', () => {
    const releaseFlow = createReleaseFlowMachine('deploy');
    const biosFlow = createBiosFlowMachine('complete');
    const result = canDeploy({ releaseFlow, biosFlow });
    assert.equal(result.eligible, true);
  });

  test('canDeploy fails when bios is not complete', () => {
    const releaseFlow = createReleaseFlowMachine('deploy');
    const biosFlow = createBiosFlowMachine('partially_verified');
    const result = canDeploy({ releaseFlow, biosFlow });
    assert.equal(result.eligible, false);
  });

  test('canBuild requires bios complete', () => {
    const releaseFlow = createReleaseFlowMachine('build');
    const biosFlow = createBiosFlowMachine('complete');
    const result = canBuild({ releaseFlow, biosFlow });
    assert.equal(result.eligible, true);
  });

  test('canBuild fails when bios is blocked', () => {
    const releaseFlow = createReleaseFlowMachine('build');
    const biosFlow = createBiosFlowMachine('blocked');
    const result = canBuild({ releaseFlow, biosFlow });
    assert.equal(result.eligible, false);
  });

  test('deriveBiosFlowState preserves resumed and partial stages', () => {
    assert.equal(deriveBiosFlowState({ stage: 'resumed_from_firmware', readyToBuild: false }), 'resumed_from_firmware');
    assert.equal(deriveBiosFlowState({ stage: 'partially_verified', readyToBuild: false }), 'partially_verified');
  });

  test('deriveReleaseFlowState blocks when compatibility is blocked', () => {
    const state = deriveReleaseFlowState({
      step: 'method-select',
      hasProfile: true,
      compatibilityBlocked: true,
      biosFlowState: 'complete',
      buildReady: true,
      hasEfi: true,
      validationBlocked: false,
    });

    assert.equal(state, 'blocked');
  });

  test('deriveReleaseFlowState maps post-build and deploy steps correctly', () => {
    const methodState = deriveReleaseFlowState({
      step: 'method-select',
      hasProfile: true,
      compatibilityBlocked: false,
      biosFlowState: 'complete',
      buildReady: true,
      hasEfi: true,
      validationBlocked: false,
    });
    const deployState = deriveReleaseFlowState({
      step: 'usb-select',
      hasProfile: true,
      compatibilityBlocked: false,
      biosFlowState: 'complete',
      buildReady: true,
      hasEfi: true,
      validationBlocked: false,
    });

    assert.equal(methodState, 'method');
    assert.equal(deployState, 'deploy');
  });

  test('evaluateBuildGuard blocks blocked compatibility before build', () => {
    const result = evaluateBuildGuard({
      compatibilityBlocked: true,
      biosFlowState: 'complete',
      releaseFlowState: 'build',
    });

    assert.equal(result.allowed, false);
    assert.match(result.reason ?? '', /compatibility is blocked/i);
  });

  test('evaluateBuildGuard allows bios-complete build entry', () => {
    const result = evaluateBuildGuard({
      compatibilityBlocked: false,
      biosFlowState: 'complete',
      releaseFlowState: 'bios',
    });

    assert.equal(result.allowed, true);
  });

  test('evaluateDeployGuard blocks deploy when validation fails', () => {
    const result = evaluateDeployGuard({
      compatibilityBlocked: false,
      biosFlowState: 'complete',
      releaseFlowState: 'deploy',
      validationBlocked: true,
      hasEfi: true,
    });

    assert.equal(result.allowed, false);
    assert.match(result.reason ?? '', /validation is blocked/i);
  });

  test('evaluateDeployGuard blocks deploy without EFI', () => {
    const result = evaluateDeployGuard({
      compatibilityBlocked: false,
      biosFlowState: 'complete',
      releaseFlowState: 'deploy',
      validationBlocked: false,
      hasEfi: false,
    });

    assert.equal(result.allowed, false);
    assert.match(result.reason ?? '', /validated efi is required/i);
  });

  test('evaluateDeployGuard allows deploy only from deploy state with bios complete', () => {
    const result = evaluateDeployGuard({
      compatibilityBlocked: false,
      biosFlowState: 'complete',
      releaseFlowState: 'deploy',
      validationBlocked: false,
      hasEfi: true,
    });

    assert.equal(result.allowed, true);
  });
});
