import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import type { CompatibilityReport } from '../electron/compatibility.js';
import type { ValidationResult } from '../electron/configValidator.js';
import type { DiskInfo } from '../electron/diskOps.js';
import {
  buildIssueReportDraft,
  createDiagnosticsSnapshot,
  inferIssueReportTrigger,
  openIssueReportUrl,
  type ReleaseFailureContext,
} from '../electron/releaseDiagnostics.js';

function makeCompatibilityReport(): CompatibilityReport {
  return {
    level: 'blocked',
    strategy: 'blocked',
    confidence: 'high',
    explanation: 'The selected macOS version is above the supported GPU ceiling.',
    manualVerificationRequired: false,
    isCompatible: false,
    maxOSVersion: 'macOS Sonoma 14',
    eligibleVersions: [{ id: '14', name: 'macOS Sonoma 14', icon: 'sonoma' }],
    recommendedVersion: 'macOS Sonoma 14',
    warnings: ['Use a supported iGPU output path.'],
    errors: ['Selected target exceeds the supported GPU ceiling.'],
    minReqMet: true,
  };
}

function makeValidationResult(): ValidationResult {
  return {
    overall: 'blocked',
    checkedAt: new Date().toISOString(),
    issues: [
      {
        code: 'MISSING_DRIVER',
        severity: 'blocked',
        message: 'Missing OpenRuntime.efi in /Users/alice/EFI.',
        detail: 'Expected /Users/alice/EFI/OC/Drivers/OpenRuntime.efi',
        component: 'OpenRuntime.efi',
        expectedPath: 'EFI/OC/Drivers/OpenRuntime.efi',
        actualCondition: 'File missing',
      },
    ],
    firstFailureTrace: {
      code: 'MISSING_DRIVER',
      component: 'OpenRuntime.efi',
      expectedPath: 'EFI/OC/Drivers/OpenRuntime.efi',
      source: 'generated',
      detail: 'Expected /Users/alice/EFI/OC/Drivers/OpenRuntime.efi',
    },
  };
}

function makeDisk(): DiskInfo {
  return {
    device: '/dev/disk4',
    devicePath: '/dev/disk/by-id/usb-Sensitive-1234',
    isSystemDisk: false,
    partitionTable: 'gpt',
    mountedPartitions: [],
    serialNumber: 'USB-12345',
    identityConfidence: 'strong',
    identityFieldsUsed: ['serialNumber', 'devicePath'],
  };
}

describe('release diagnostics', () => {
  test('builds a sanitized issue report for validation failures', () => {
    const snapshot = createDiagnosticsSnapshot({
      version: '2.2.1',
      platform: 'darwin',
      arch: 'arm64',
      compatMode: 'none',
      timestamp: '2026-03-20T12:00:00.000Z',
      sessionId: 'session-secret-value',
      hardware: 'Intel Core i7, Radeon RX 580, macOS',
      confidence: 'high',
      firmware: 'SB:false, VT:true, VT-d:false',
      lastTaskKind: 'efi-build',
      lastTaskStatus: 'failed',
      lastError: 'Failure at /Users/alice/EFI',
      failedKexts: ['WhateverGreen (github: timeout token=abc123)'],
      kextSources: { WhateverGreen: 'failed' },
      selectedDisk: makeDisk(),
      diskIdentity: { serialNumber: 'USB-12345', devicePath: '/dev/disk/by-id/usb-Sensitive-1234' },
      compatibilityReport: makeCompatibilityReport(),
      validationResult: makeValidationResult(),
      recoveryStats: {
        attempts: 2,
        lastHttpCode: 403,
        lastError: 'APPLE_AUTH_REJECT for alice@example.com',
        decision: 'manual-import',
        source: 'apple_primary',
      },
      recentLogs: [
        {
          t: '2026-03-20T12:00:00.000Z',
          level: 'ERROR',
          ctx: 'efi',
          msg: 'Validation failed at /Users/alice/EFI with flashconf.secret-token',
          sessionId: 'session-secret-value',
        },
      ],
      lastFailure: null,
    });

    assert.equal(snapshot.trigger, 'efi_validation_failure');
    assert.equal(snapshot.sessionFingerprint.length, 12);
    assert.equal(snapshot.diskContext.selectedDevice, 'disk4');
    assert.ok(snapshot.diskContext.identityFingerprint);

    const draft = buildIssueReportDraft(snapshot);
    assert.match(draft.title, /\[BUG\]\[EFI_VALIDATION_FAILURE\]/);
    assert.equal(draft.body.includes('/Users/alice'), false);
    assert.equal(draft.body.includes('alice@example.com'), false);
    assert.equal(draft.body.includes('USB-12345'), false);
    assert.equal(draft.body.includes('flashconf.secret-token'), false);
    assert.match(draft.body, /disk4/);
    assert.match(draft.body, /OpenRuntime\.efi/);
    assert.match(draft.body, /Selected target exceeds the supported GPU ceiling\./);
  });

  test('prefers explicit runtime failure context when present', () => {
    const lastFailure: ReleaseFailureContext = {
      trigger: 'unexpected_runtime_error',
      message: 'Unhandled renderer exception',
      detail: 'stack',
      channel: 'renderer-error',
      code: 'unhandledrejection',
      occurredAt: '2026-03-20T12:00:00.000Z',
    };

    const trigger = inferIssueReportTrigger({
      lastFailure,
      validationResult: makeValidationResult(),
      lastTaskKind: 'efi-build',
      lastTaskStatus: 'failed',
    });

    assert.equal(trigger, 'unexpected_runtime_error');
  });

  test('issue submission helper fails closed on timeout instead of hanging', async () => {
    const startedAt = Date.now();
    const success = await openIssueReportUrl(
      'https://example.com',
      async () => {
        await new Promise(() => undefined);
      },
      25,
    );

    assert.equal(success, false);
    assert.ok(Date.now() - startedAt < 250);
  });
});
