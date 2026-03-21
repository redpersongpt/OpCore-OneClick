import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import {
  buildUserFacingErrorMessage,
  createClassifiedIpcError,
  type ClassifiedError,
} from '../electron/errorMessaging.js';
import { buildFailureRecoveryViewModel } from '../src/lib/failureRecovery.js';
import { getSuggestionPayload } from '../src/lib/suggestionEngine.js';
import { structureError } from '../src/lib/structuredErrors.js';

describe('error recovery messaging', () => {
  test('preserves the real explanation when classified message is generic', () => {
    const classified: ClassifiedError = {
      category: 'app_error',
      message: 'Operation failed',
      explanation: 'Build will fail: 2 kext(s) unavailable: Lilu.kext',
      suggestion: 'Fix the blocker first.',
    };

    assert.equal(
      buildUserFacingErrorMessage(classified),
      'Build will fail: 2 kext(s) unavailable: Lilu.kext',
    );
  });

  test('combines specific classified messages with their explanation', () => {
    const classified: ClassifiedError = {
      category: 'environment_error',
      message: 'Permission denied',
      explanation: 'The application lacks the necessary system privileges to write directly to the hardware.',
      suggestion: 'Re-run with elevated privileges.',
    };

    const wrapped = createClassifiedIpcError(classified);
    assert.match(wrapped.message, /permission denied:/i);
    assert.match(wrapped.message, /necessary system privileges/i);
  });

  test('maps lower-cased permission and build blockers to structured guidance', () => {
    const permission = structureError('permission denied: raw disk access failed');
    const buildBlocked = structureError('build will fail: 2 kext(s) unavailable: Lilu.kext');
    const biosBlocked = structureError('bios_requirements_not_met: Secure Boot is not verified');

    assert.match(permission.title, /permission denied/i);
    assert.match(permission.nextStep, /administrator|sudo/i);
    assert.match(buildBlocked.title, /build is blocked/i);
    assert.match(buildBlocked.nextStep, /fix the blocker/i);
    assert.match(biosBlocked.title, /bios settings still need attention/i);
    assert.match(biosBlocked.nextStep, /recheck bios/i);
  });

  test('classifies classified recovery rejection and pre-build failures without falling back to unknown retry', () => {
    const recovery = getSuggestionPayload({
      errorMessage: 'Apple recovery server rejected the request: Apple’s recovery service refused the download request.',
      step: 'recovery-download',
    });
    const prebuild = getSuggestionPayload({
      errorMessage: 'Build will fail: 2 kext(s) unavailable: Lilu.kext',
      step: 'building',
    });

    assert.equal(recovery.code, 'recovery_auth_rejected');
    assert.doesNotMatch(recovery.suggestion ?? '', /retry the efi build/i);
    assert.equal(prebuild.code, 'build_precheck_failed');
    assert.doesNotMatch(prebuild.suggestion ?? '', /retry the efi build/i);
  });

  test('failure recovery view prefers structured titles and next actions over generic placeholders', () => {
    const view = buildFailureRecoveryViewModel({
      message: 'Something went wrong',
      rawMessage: 'permission denied: raw disk access failed',
    });

    assert.ok(view);
    assert.match(view!.title, /permission denied/i);
    assert.match(view!.whatFailed, /does not have permission/i);
    assert.equal(view!.nextActions.length, 1);
    assert.match(view!.nextActions[0] ?? '', /administrator|sudo/i);
  });

  test('does not escalate generic flash failures into a faulty-drive claim without media-style errors', () => {
    const suggestion = getSuggestionPayload({
      errorMessage: 'USB flash write failed. Verification failed: EFI\\\\OC\\\\OpenCore.efi not found on USB after copy',
      step: 'usb-select',
      platform: 'win32',
      retryCount: 2,
    });

    assert.equal(suggestion.code, 'flash_write_error');
    assert.doesNotMatch(suggestion.explanation ?? '', /likely faulty/i);
    assert.doesNotMatch(suggestion.suggestion ?? '', /replace the usb drive with a known-good drive/i);
  });

  test('keeps permission-style flash failures on the permission path instead of blaming the drive', () => {
    const suggestion = getSuggestionPayload({
      errorMessage: 'USB flash write failed: permission denied: raw disk access failed',
      step: 'usb-select',
      platform: 'win32',
      retryCount: 2,
    });

    assert.equal(suggestion.code, 'flash_write_error');
    assert.match(suggestion.message ?? '', /permissions/i);
    assert.match(suggestion.suggestion ?? '', /administrator/i);
  });
});
