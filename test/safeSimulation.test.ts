import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import type { HardwareProfile } from '../electron/configGenerator.js';
import {
  cleanupExpiredSafeSimulationWorkspaces,
  runSafeSimulation,
  SAFE_SIMULATION_DISCLAIMER,
} from '../electron/safeSimulation.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i7-8700K',
    architecture: 'Intel',
    generation: 'Coffee Lake',
    coreCount: 6,
    gpu: 'Intel UHD Graphics 630',
    gpuDevices: [{ name: 'Intel UHD Graphics 630', vendorName: 'Intel' }],
    ram: '16 GB',
    motherboard: 'Z390 AORUS PRO',
    targetOS: 'macOS Sequoia 15',
    smbios: 'iMac19,1',
    kexts: ['Lilu'],
    ssdts: ['SSDT-EC-USBX'],
    bootArgs: '-v',
    isLaptop: false,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `safe-sim-${prefix}-`));
}

describe('safe simulation', () => {
  test('returns the full output contract, uses a temp workspace, and propagates blockers/warnings', async () => {
    const userDataPath = makeTempDir('contract');
    const profile = makeProfile();
    let createdWorkspacePath: string | null = null;

    const result = await runSafeSimulation(profile, {
      userDataPath,
      now: () => 1_700_000_000_000,
      createEfiWorkspace: async (workspacePath) => {
        createdWorkspacePath = workspacePath;
        fs.mkdirSync(path.join(workspacePath, 'EFI/OC'), { recursive: true });
        fs.writeFileSync(path.join(workspacePath, 'EFI/OC/config.plist'), '<plist />');
      },
      validateEfi: async () => ({
        overall: 'blocked',
        checkedAt: new Date().toISOString(),
        firstFailureTrace: null,
        issues: [
          {
            code: 'SIM_BLOCK',
            severity: 'blocked',
            message: 'Required kext is missing from the preview workspace.',
            detail: null,
            component: 'Lilu.kext',
            expectedPath: 'EFI/OC/Kexts/Lilu.kext',
            actualCondition: 'Missing bundle',
          },
          {
            code: 'SIM_WARN',
            severity: 'warning',
            message: 'Preview used generated placeholder SSDTs only.',
            detail: null,
            component: 'SSDT-EC-USBX',
            expectedPath: 'EFI/OC/ACPI/SSDT-EC-USBX.aml',
            actualCondition: 'Placeholder generated',
          },
        ],
      }),
      buildCompatibilityMatrix: () => ({
        recommendedVersion: 'macOS Sequoia 15',
        rows: [
          {
            versionId: '15',
            versionName: 'macOS Sequoia 15',
            icon: 'sequoia',
            numeric: 15,
            status: 'partial',
            reason: 'Manual verification is still required.',
            recommended: true,
            reportLevel: 'supported_with_warnings',
          },
        ],
      }),
      simulateBuild: async () => ({
        timestamp: new Date().toISOString(),
        profile: profile.smbios,
        certainty: 'may_fail',
        blockers: ['GitHub source for Lilu is unavailable.'],
        totalComponents: 2,
        verifiedComponents: 1,
        failedComponents: 1,
        estimatedBuildTimeMs: 1000,
        components: [
          {
            name: 'Lilu',
            type: 'kext',
            verified: false,
            certainty: 'may_fail',
            detail: 'GitHub source unavailable',
          },
          {
            name: 'SSDT-EC-USBX',
            type: 'ssdt',
            verified: true,
            certainty: 'will_succeed',
            detail: 'Generated locally',
          },
        ],
      }),
      dryRunRecovery: async () => ({
        timestamp: new Date().toISOString(),
        targetOS: profile.targetOS,
        boardId: 'Mac-TEST',
        endpointReachable: true,
        testRequestResult: 'timeout',
        httpCode: null,
        certainty: 'may_fail',
        recommendation: 'Recovery endpoint timed out during the dry-run.',
      }),
      kextRegistry: {
        Lilu: { repo: 'acidanthera/Lilu' },
      },
    });

    assert.ok(createdWorkspacePath);
    assert.equal(result.workspacePath, createdWorkspacePath);
    assert.match(result.workspacePath, /safe-simulations/);
    assert.ok(fs.existsSync(result.workspacePath));
    assert.equal(result.disclaimer, SAFE_SIMULATION_DISCLAIMER);
    assert.equal(result.validationSummary.overall, 'blocked');
    assert.ok(result.efiSummary.configHash);
    assert.equal('buildReady' in (result as unknown as Record<string, unknown>), false);
    assert.equal('confirmationToken' in (result as unknown as Record<string, unknown>), false);
    assert.ok(result.resourcePlan.resources.every((resource) =>
      resource.source &&
      resource.expectedIdentityOrVersion &&
      resource.validationOutcome &&
      resource.sourceClass,
    ));
    assert.ok(result.blockers.includes('Required kext is missing from the preview workspace.'));
    assert.ok(result.blockers.includes('GitHub source for Lilu is unavailable.'));
    assert.ok(result.warnings.includes('Manual verification is still required.'));
    assert.ok(result.warnings.includes('Preview used generated placeholder SSDTs only.'));
    assert.ok(result.warnings.includes('Recovery endpoint timed out during the dry-run.'));
  });

  test('cleans up expired simulation workspaces without deleting fresh ones', () => {
    const root = makeTempDir('cleanup');
    const oldDir = path.join(root, 'old-sim');
    const freshDir = path.join(root, 'fresh-sim');
    fs.mkdirSync(oldDir, { recursive: true });
    fs.mkdirSync(freshDir, { recursive: true });

    const now = Date.now();
    fs.utimesSync(oldDir, new Date(now - 3 * 24 * 60 * 60 * 1000), new Date(now - 3 * 24 * 60 * 60 * 1000));
    fs.utimesSync(freshDir, new Date(now), new Date(now));

    cleanupExpiredSafeSimulationWorkspaces(root, now, 24 * 60 * 60 * 1000);

    assert.equal(fs.existsSync(oldDir), false);
    assert.equal(fs.existsSync(freshDir), true);
  });
});
