import React from 'react';
import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import { renderToStaticMarkup } from 'react-dom/server';
import type { HardwareProfile } from '../electron/configGenerator.js';
import { buildResourcePlan } from '../electron/resourcePlanner.js';
import ResourcePlanPanel from '../src/components/ResourcePlanPanel.js';

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
    kexts: ['Lilu.kext'],
    ssdts: ['SSDT-EC-USBX.aml'],
    bootArgs: '-v',
    isLaptop: false,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

describe('resource planner', () => {
  test('emits complete provenance and validation metadata for every resource without mutating inputs', () => {
    const profile = makeProfile();
    const registry = {
      'Lilu.kext': { repo: 'acidanthera/Lilu', assetFilter: 'RELEASE' },
    };
    const registryBefore = JSON.stringify(registry);
    const originalFetch = globalThis.fetch;
    let fetchCalled = false;
    globalThis.fetch = (async () => {
      fetchCalled = true;
      throw new Error('fetch should not be called');
    }) as typeof fetch;

    try {
      const plan = buildResourcePlan({
        profile,
        kextRegistry: registry,
      });

      assert.equal(fetchCalled, false);
      assert.equal(JSON.stringify(registry), registryBefore);
      assert.ok(plan.resources.length > 0);
      assert.ok(plan.resources.every((resource) =>
        resource.source &&
        resource.expectedIdentityOrVersion &&
        resource.validationOutcome &&
        resource.sourceClass,
      ));
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  test('maps validation issues and failed kext sources into planner outcomes', () => {
    const plan = buildResourcePlan({
      profile: makeProfile(),
      validationResult: {
        overall: 'warning',
        checkedAt: new Date().toISOString(),
        firstFailureTrace: null,
        issues: [
          {
            code: 'DRIVER_MISSING',
            severity: 'blocked',
            message: 'Driver referenced in config but not on disk: OpenRuntime.efi',
            detail: null,
            component: 'OpenRuntime.efi',
            expectedPath: 'EFI/OC/Drivers/OpenRuntime.efi',
            actualCondition: 'Missing',
          },
          {
            code: 'SSDT_WARN',
            severity: 'warning',
            message: 'Generated SSDT still needs manual review.',
            detail: null,
            component: 'SSDT-EC-USBX.aml',
            expectedPath: 'EFI/OC/ACPI/SSDT-EC-USBX.aml',
            actualCondition: 'Manual review',
          },
        ],
      },
      kextRegistry: {
        'Lilu.kext': { repo: 'acidanthera/Lilu', assetFilter: 'RELEASE' },
      },
      kextSources: {
        'Lilu.kext': 'failed',
      },
    });

    const lilu = plan.resources.find((resource) => resource.name === 'Lilu.kext');
    const driver = plan.resources.find((resource) => resource.name === 'OpenRuntime.efi');
    const ssdt = plan.resources.find((resource) => resource.name === 'SSDT-EC-USBX.aml');

    assert.equal(lilu?.validationOutcome, 'blocked');
    assert.equal(driver?.validationOutcome, 'blocked');
    assert.equal(ssdt?.validationOutcome, 'warning');
  });

  test('renders provenance and validation state per resource', () => {
    const html = renderToStaticMarkup(React.createElement(ResourcePlanPanel, {
      plan: {
        resources: [
          {
            name: 'Lilu.kext',
            kind: 'kext',
            source: 'https://github.com/acidanthera/Lilu/releases/latest',
            expectedIdentityOrVersion: 'acidanthera/Lilu (RELEASE)',
            validationOutcome: 'pending_manual',
            sourceClass: 'downloaded',
          },
          {
            name: 'OpenRuntime.efi',
            kind: 'driver',
            source: 'OpenCorePkg release contents',
            expectedIdentityOrVersion: 'OpenCore base driver',
            validationOutcome: 'verified',
            sourceClass: 'downloaded',
          },
        ],
      },
    }));

    assert.match(html, /Lilu\.kext/);
    assert.match(html, /OpenRuntime\.efi/);
    assert.match(html, /acidanthera\/Lilu \(RELEASE\)/);
    assert.match(html, /OpenCorePkg release contents/);
    assert.match(html, /Pending/);
    assert.match(html, /Verified/);
  });
});
