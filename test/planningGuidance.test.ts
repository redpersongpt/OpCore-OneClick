import React from 'react';
import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import { renderToStaticMarkup } from 'react-dom/server';
import CompatibilitySummary from '../src/components/CompatibilitySummary.js';
import SimulationPreview from '../src/components/SimulationPreview.js';
import type { CompatibilityReport } from '../electron/compatibility.js';
import type { SafeSimulationResult } from '../electron/safeSimulation.js';

function makeCompatibilityReport(): CompatibilityReport {
  return {
    level: 'risky',
    strategy: 'conservative',
    confidence: 'high',
    explanation: 'Older laptop path with community success, but manual fixes are likely.',
    manualVerificationRequired: true,
    isCompatible: true,
    maxOSVersion: 'macOS Monterey 12',
    eligibleVersions: [{ id: '12', name: 'macOS Monterey 12', icon: 'monterey' }],
    recommendedVersion: 'macOS Big Sur 11',
    warnings: ['Older laptop path: expect manual patches.'],
    errors: [],
    minReqMet: true,
    communityEvidence: {
      signal: 'strong',
      matchLevel: 'strong',
      matchExplanation: 'Strong match: same CPU generation, same system class, and a closely matching model family were found in documented success posts.',
      matchedCount: 2,
      bestMatchScore: 6,
      bestMatchConfidence: 'high',
      highestReportedVersion: 'macOS Big Sur 11',
      highestReportedVersionNumeric: 11,
      summary: '2 documented SUCCESS posts for similar hardware.',
      whatUsuallyWorks: ['Boot', 'GPU acceleration'],
      whatDidNotWork: ['Sleep', 'Trackpad'],
      sources: [],
    },
    nextActions: [{
      title: 'Treat sleep as optional first',
      detail: 'Disable sleep until the system is stable.',
      source: 'community',
      confidence: 'medium',
    }],
    advisoryConfidence: {
      score: 58,
      label: 'Medium confidence',
      explanation: 'Medium confidence based on community evidence and the selected target.',
    },
    mostLikelyFailurePoints: [{
      title: 'Sleep instability',
      detail: 'Expect sleep and wake to fail first.',
      likelihood: 'very likely',
      source: 'community',
    }],
  };
}

function makeSimulationResult(): SafeSimulationResult {
  return {
    workspacePath: '/tmp/moc-sim',
    efiSummary: {
      efiPath: '/tmp/moc-sim',
      configHash: 'abcdef1234567890',
      requiredResources: [{ name: 'Lilu', kind: 'kext', sourceClass: 'downloaded' }],
    },
    validationSummary: {
      overall: 'warning',
      issues: [{
        code: 'AUDIO',
        severity: 'warning',
        component: 'AppleALC',
        message: 'Audio layout still needs tuning.',
      }],
    },
    compatibilityMatrixSnapshot: [{
      versionId: '12',
      versionName: 'macOS Monterey 12',
      icon: 'monterey',
      numeric: 12,
      status: 'risky',
      reason: 'Manual fixes are likely.',
      recommended: true,
      reportLevel: 'risky',
    }],
    recoveryReadiness: {
      certainty: 'may_fail',
      recommendation: 'Recovery endpoint timed out.',
    },
    resourcePlan: {
      resources: [{
        name: 'Lilu',
        kind: 'kext',
        source: 'acidanthera/Lilu',
        expectedIdentityOrVersion: 'latest',
        validationOutcome: 'verified',
        sourceClass: 'downloaded',
      }],
    },
    blockers: [],
    warnings: ['Manual verification is still required.'],
    disclaimer: 'Simulation validates build inputs and current checks only. It is not a boot guarantee.',
  };
}

describe('planning guidance ui', () => {
  test('renders community match level and likely failure points in the compatibility summary', () => {
    const html = renderToStaticMarkup(
      React.createElement(CompatibilitySummary, { report: makeCompatibilityReport() }),
    );

    assert.match(html, /Community Match Level/);
    assert.match(html, /Most Likely Failure Points/);
    assert.match(html, /Sleep instability/);
  });

  test('renders likely failure points and concise compatibility framing in simulation preview', () => {
    const html = renderToStaticMarkup(
      React.createElement(SimulationPreview, {
        result: makeSimulationResult(),
        report: makeCompatibilityReport(),
      }),
    );

    assert.match(html, /Most Likely Failure Points/);
    assert.match(html, /Versions stay visible by confidence level so you can see the best starting point and the stretch targets side by side\./);
    assert.match(html, /Audio layout still needs tuning\./);
  });
});
