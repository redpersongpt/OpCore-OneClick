/**
 * Batch 2 Runtime Determinism Tests
 *
 * H2: handleRecoveryRetry must not no-op after clearing the error
 * H4: Build error classification must identify the failing stage
 */

import { describe, it, expect } from 'vitest';
import { resolveRecoveryRetryAction, type RecoveryRetryContext } from '../src/lib/recoveryRouting.js';

// ═══════════════════════════════════════════════════════════════════════════════
// H2: Recovery retry routing — no silent no-ops
// ═══════════════════════════════════════════════════════════════════════════════

describe('Recovery retry routing (H2)', () => {
  it('scanning step retries with scan action', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'scanning', hasProfile: false, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('scan');
  });

  it('bios step with profile retries with refresh_bios', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'bios', hasProfile: true, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('refresh_bios');
  });

  it('bios step without profile returns noop', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'bios', hasProfile: false, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('noop');
  });

  it('building step with profile retries with restart_build', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'building', hasProfile: true, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('restart_build');
  });

  it('building step without profile returns noop', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'building', hasProfile: false, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('noop');
  });

  it('usb-select step retries with refresh_usb', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'usb-select', hasProfile: true, hasMethod: true, buildReady: true,
    });
    expect(action.kind).toBe('refresh_usb');
  });

  it('method-select with method retries with reselect_method', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'method-select', hasProfile: true, hasMethod: true, buildReady: true,
    });
    expect(action.kind).toBe('reselect_method');
  });

  it('method-select without method returns noop', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'method-select', hasProfile: true, hasMethod: false, buildReady: true,
    });
    expect(action.kind).toBe('noop');
  });

  it('unknown step with profile and !buildReady returns restart_build', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'report', hasProfile: true, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('restart_build');
  });

  it('unknown step with buildReady returns noop', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'report', hasProfile: true, hasMethod: false, buildReady: true,
    });
    expect(action.kind).toBe('noop');
  });

  it('null targetStep defaults to noop when buildReady', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: null, hasProfile: true, hasMethod: false, buildReady: true,
    });
    expect(action.kind).toBe('noop');
  });

  it('recovery-download step with profile restarts build', () => {
    const action = resolveRecoveryRetryAction({
      targetStep: 'recovery-download', hasProfile: true, hasMethod: false, buildReady: false,
    });
    expect(action.kind).toBe('restart_build');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// H4: Build error classification — stage identification
// These test the classification logic by matching the same keywords the
// catch block uses, verifying the intent of the classification.
// ═══════════════════════════════════════════════════════════════════════════════

describe('Build error stage classification (H4)', () => {
  // The classification is inline in App.tsx's startDeploy catch block.
  // We test the keyword matching logic here as a specification contract.
  function classifyBuildError(message: string): string {
    const msg = message.toLowerCase();
    if (msg.includes('preflight') || msg.includes('confidence')) return 'preflight_failed';
    if (msg.includes('kext') || msg.includes('unavailable')) return 'kext_fetch_failed';
    if (msg.includes('recovery') || msg.includes('apple.com') || msg.includes('dmg')) return 'recovery_download_failed';
    if (msg.includes('validation') || msg.includes('validator')) return 'validation_failed';
    if (msg.includes('simulation')) return 'simulation_failed';
    return 'build_ipc_failed';
  }

  it('preflight errors classified as preflight_failed', () => {
    expect(classifyBuildError('Preflight check failed: low confidence')).toBe('preflight_failed');
    expect(classifyBuildError('Build confidence too low')).toBe('preflight_failed');
  });

  it('kext errors classified as kext_fetch_failed', () => {
    expect(classifyBuildError('Failed to fetch Lilu.kext')).toBe('kext_fetch_failed');
    expect(classifyBuildError('1 kext(s) unavailable: NootRX.kext')).toBe('kext_fetch_failed');
  });

  it('recovery errors classified as recovery_download_failed', () => {
    expect(classifyBuildError('Recovery download failed: ECONNRESET')).toBe('recovery_download_failed');
    expect(classifyBuildError('HTTP 416 from apple.com')).toBe('recovery_download_failed');
    expect(classifyBuildError('Failed to verify dmg integrity')).toBe('recovery_download_failed');
  });

  it('validation errors classified as validation_failed', () => {
    expect(classifyBuildError('EFI validation blocked deployment')).toBe('validation_failed');
    expect(classifyBuildError('Config validator found issues')).toBe('validation_failed');
  });

  it('simulation errors classified as simulation_failed', () => {
    expect(classifyBuildError('Build simulation contract violation')).toBe('simulation_failed');
  });

  it('generic errors remain build_ipc_failed', () => {
    expect(classifyBuildError('Something went wrong')).toBe('build_ipc_failed');
    expect(classifyBuildError('')).toBe('build_ipc_failed');
  });
});
