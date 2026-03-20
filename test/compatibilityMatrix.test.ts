import React from 'react';
import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import { renderToStaticMarkup } from 'react-dom/server';
import type { HardwareProfile } from '../electron/configGenerator.js';
import { checkCompatibility } from '../electron/compatibility.js';
import {
  buildCompatibilityMatrix,
  classifyCompatibilityMatrixStatus,
  type CompatibilityMatrixRow,
} from '../electron/compatibilityMatrix.js';
import CompatibilityMatrix from '../src/components/CompatibilityMatrix.js';

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
    kexts: ['Lilu', 'WhateverGreen'],
    ssdts: ['SSDT-EC-USBX'],
    bootArgs: '-v',
    isLaptop: false,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

describe('compatibility matrix', () => {
  test('classifies every row from the existing compatibility engine and includes a reason', () => {
    const profile = makeProfile();
    const matrix = buildCompatibilityMatrix(profile);

    for (const row of matrix.rows) {
      const directReport = checkCompatibility({
        ...profile,
        targetOS: row.versionName,
      });
      assert.equal(row.status, classifyCompatibilityMatrixStatus(directReport));
      assert.ok(row.reason.trim().length > 0);
    }
  });

  test('keeps the recommended version consistent with the base compatibility report', () => {
    const profile = makeProfile({
      targetOS: 'macOS Tahoe 26',
    });
    const report = checkCompatibility(profile);
    const matrix = buildCompatibilityMatrix(profile);

    assert.equal(matrix.recommendedVersion, report.recommendedVersion);
    assert.equal(matrix.rows.filter((row) => row.recommended).length, report.recommendedVersion ? 1 : 0);
  });

  test('keeps the recommended row anchored to the compatibility report for older laptops', () => {
    const profile = makeProfile({
      cpu: 'Intel Core i5-4300M',
      generation: 'Haswell',
      isLaptop: true,
      gpu: 'Intel HD Graphics 4600',
      gpuDevices: [{ name: 'Intel HD Graphics 4600', vendorName: 'Intel' }],
      motherboard: 'ThinkPad T440p',
      targetOS: 'macOS Monterey 12',
      smbios: 'MacBookPro11,1',
    });

    const matrix = buildCompatibilityMatrix(profile);

    assert.equal(matrix.recommendedVersion, 'macOS Big Sur 11');
  });

  test('preserves blocked version ceilings from existing compatibility logic', () => {
    const profile = makeProfile({
      cpu: 'Intel Core i7-4770',
      generation: 'Haswell',
      gpu: 'Intel HD Graphics 4600',
      gpuDevices: [{ name: 'Intel HD Graphics 4600', vendorName: 'Intel' }],
      motherboard: 'Z87',
      targetOS: 'macOS Sequoia 15',
    });
    const matrix = buildCompatibilityMatrix(profile);

    const montereyRow = matrix.rows.find((row) => row.versionName === 'macOS Monterey 12');
    const sonomaRow = matrix.rows.find((row) => row.versionName === 'macOS Sonoma 14');

    assert.ok(montereyRow);
    assert.ok(sonomaRow);
    assert.notEqual(montereyRow?.status, 'blocked');
    assert.equal(sonomaRow?.status, 'blocked');
  });

  test('renders supported, experimental, risky, and blocked rows with reasons', () => {
    const rows: CompatibilityMatrixRow[] = [
      {
        versionId: '15',
        versionName: 'macOS Sequoia 15',
        icon: 'sequoia',
        numeric: 15,
        status: 'supported',
        reason: 'Strong canonical target.',
        recommended: true,
        reportLevel: 'supported',
      },
      {
        versionId: '14',
        versionName: 'macOS Sonoma 14',
        icon: 'sonoma',
        numeric: 14,
        status: 'experimental',
        reason: 'Older but still community-proven laptop path.',
        recommended: false,
        reportLevel: 'experimental',
      },
      {
        versionId: '13',
        versionName: 'macOS Ventura 13',
        icon: 'ventura',
        numeric: 13,
        status: 'risky',
        reason: 'Boot path exists, but manual fixes are likely.',
        recommended: false,
        reportLevel: 'risky',
      },
      {
        versionId: '26',
        versionName: 'macOS Tahoe 26',
        icon: 'tahoe',
        numeric: 26,
        status: 'blocked',
        reason: 'Selected target exceeds the supported GPU ceiling.',
        recommended: false,
        reportLevel: 'blocked',
      },
    ];

    const html = renderToStaticMarkup(
      React.createElement(CompatibilityMatrix, {
        rows,
        selectedVersion: 'macOS Sonoma 14',
      }),
    );

    assert.match(html, /Supported/);
    assert.match(html, /Experimental/);
    assert.match(html, /Risky/);
    assert.match(html, /Blocked/);
    assert.match(html, /Strong canonical target\./);
    assert.match(html, /Older but still community-proven laptop path\./);
    assert.match(html, /Boot path exists, but manual fixes are likely\./);
    assert.match(html, /Selected target exceeds the supported GPU ceiling\./);
    assert.match(html, /Recommended/);
    assert.match(html, /Selected/);
    assert.match(html, /Manual fixes likely/);
  });
});
