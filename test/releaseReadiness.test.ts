import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { checkCompatibility } from '../electron/compatibility.js';
import { getBIOSSettings, type HardwareProfile } from '../electron/configGenerator.js';
import { buildBiosOrchestratorState } from '../electron/bios/orchestrator.js';
import { buildHardwareFingerprint } from '../electron/bios/sessionState.js';
import type { BiosSessionState } from '../electron/bios/types.js';
import type { FirmwareInfo, FirmwareRequirement } from '../electron/firmwarePreflight.js';
import { validateEfi } from '../electron/configValidator.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i5-9600K',
    architecture: 'Intel',
    generation: 'Coffee Lake',
    coreCount: 6,
    gpu: 'Intel UHD Graphics 630',
    gpuDevices: [{ name: 'Intel UHD Graphics 630', vendorName: 'Intel' }],
    ram: '16 GB',
    motherboard: 'Gigabyte Z390 Aorus Elite',
    targetOS: 'macOS Sequoia 15',
    smbios: 'iMac19,1',
    kexts: [],
    ssdts: [],
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop: false,
    isVM: false,
    audioLayoutId: 1,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

function makeRequirement(id: FirmwareRequirement['id'], status: FirmwareRequirement['status'], detectedValue: string | null, source = 'test probe'): FirmwareRequirement {
  return {
    id,
    name: id,
    description: id,
    why: id,
    consequence: id,
    requiredValue: 'test',
    detectedValue,
    status,
    source,
    critical: id === 'uefi-mode' || id === 'secure-boot',
  };
}

function makeFirmwareInfo(overrides: Partial<FirmwareInfo> = {}): FirmwareInfo {
  return {
    hostContext: 'is_target',
    vendor: 'Dell Inc.',
    version: '1.0.0',
    releaseDate: '2025-01-01',
    isUefi: true,
    secureBoot: false,
    vtEnabled: true,
    vtdEnabled: false,
    above4GDecoding: true,
    firmwareMode: 'UEFI',
    confidence: 'high',
    requirements: [
      makeRequirement('uefi-mode', 'confirmed', 'UEFI'),
      makeRequirement('secure-boot', 'confirmed', 'Disabled'),
      makeRequirement('vt-x', 'confirmed', 'Supported by CPU'),
      makeRequirement('vt-d', 'confirmed', 'Likely disabled'),
      makeRequirement('above4g', 'confirmed', 'Likely enabled'),
    ],
    ...overrides,
  };
}

function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `release-qa-${prefix}-`));
}

function basePlist(kernelAdd = '', drivers = '<dict><key>Enabled</key><true/><key>Path</key><string>HfsPlus.efi</string></dict><dict><key>Enabled</key><true/><key>Path</key><string>OpenRuntime.efi</string></dict>'): string {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>ACPI</key>
  <dict>
    <key>Add</key>
    <array />
  </dict>
  <key>Kernel</key>
  <dict>
    <key>Add</key>
    <array>${kernelAdd}</array>
    <key>Patch</key>
    <array />
  </dict>
  <key>Misc</key>
  <dict>
    <key>Security</key>
    <dict>
      <key>SecureBootModel</key>
      <string>Default</string>
    </dict>
  </dict>
  <key>UEFI</key>
  <dict>
    <key>Drivers</key>
    <array>${drivers}</array>
  </dict>
</dict>
</plist>
<!-- padding -->
<!-- padding -->
<!-- padding -->
<!-- padding -->`;
}

function writeBaseEfi(root: string) {
  fs.mkdirSync(path.join(root, 'EFI/BOOT'), { recursive: true });
  fs.mkdirSync(path.join(root, 'EFI/OC/Drivers'), { recursive: true });
  fs.mkdirSync(path.join(root, 'EFI/OC/Kexts'), { recursive: true });
  fs.mkdirSync(path.join(root, 'EFI/OC/ACPI'), { recursive: true });
  fs.writeFileSync(path.join(root, 'EFI/OC/config.plist'), basePlist());
  fs.writeFileSync(path.join(root, 'EFI/OC/OpenCore.efi'), Buffer.alloc(120 * 1024, 1));
  fs.writeFileSync(path.join(root, 'EFI/BOOT/BOOTx64.efi'), Buffer.alloc(40 * 1024, 1));
  fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/OpenRuntime.efi'), Buffer.alloc(40 * 1024, 1));
  fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/HfsPlus.efi'), Buffer.alloc(40 * 1024, 1));
}

describe('Release compatibility break flows', () => {
  test('AMD laptop is never green-lit', () => {
    const report = checkCompatibility(makeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      cpu: 'AMD Ryzen 7 6800H',
      isLaptop: true,
      gpu: 'AMD Radeon 680M',
      gpuDevices: [{ name: 'AMD Radeon 680M', vendorName: 'AMD' }],
    }));

    assert.notEqual(report.level, 'supported');
    assert.notEqual(report.level, 'supported_with_warnings');
  });

  test('older Intel CPU targeting Ventura is downgraded and blocked at the too-new target', () => {
    const report = checkCompatibility(makeProfile({
      generation: 'Ivy Bridge',
      cpu: 'Intel Core i7-3770',
      gpu: 'Intel HD Graphics 4000',
      gpuDevices: [{ name: 'Intel HD Graphics 4000', vendorName: 'Intel' }],
      targetOS: 'macOS Ventura 13',
    }));

    assert.equal(report.level, 'blocked');
    assert.ok(report.eligibleVersions.some(version => version.id === '11'));
    assert.ok(report.errors.some(error => /choose macos big sur 11/i.test(error)));
  });
});

describe('Release validation break flows', () => {
  test('missing HfsPlus.efi hard-blocks validation', async () => {
    const dir = makeTempDir('hfsplus');
    try {
      writeBaseEfi(dir);
      fs.rmSync(path.join(dir, 'EFI/OC/Drivers/HfsPlus.efi'));
      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.expectedPath === 'EFI/OC/Drivers/HfsPlus.efi'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('broken driver reference in config.plist hard-blocks validation', async () => {
    const dir = makeTempDir('driver-ref');
    try {
      writeBaseEfi(dir);
      fs.writeFileSync(path.join(dir, 'EFI/OC/config.plist'), basePlist('', '<dict><key>Enabled</key><true/><key>Path</key><string>MissingDriver.efi</string></dict>'));
      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.code === 'DRIVER_MISSING'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('invalid ACPI source files hard-block validation', async () => {
    const dir = makeTempDir('acpi-dsl');
    try {
      writeBaseEfi(dir);
      fs.writeFileSync(path.join(dir, 'EFI/OC/ACPI/SSDT-Broken.dsl'), 'DefinitionBlock ("", "SSDT", 2, "TEST", "BROKEN", 0) {}');
      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.code === 'ACPI_DSL_PRESENT'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('Lilu missing while plugin kext remains hard-blocks validation', async () => {
    const dir = makeTempDir('lilu-dep');
    try {
      writeBaseEfi(dir);
      fs.mkdirSync(path.join(dir, 'EFI/OC/Kexts/WhateverGreen.kext/Contents/MacOS'), { recursive: true });
      fs.writeFileSync(path.join(dir, 'EFI/OC/Kexts/WhateverGreen.kext/Contents/Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>WhateverGreen</string>
</dict>
</plist>`);
      fs.writeFileSync(path.join(dir, 'EFI/OC/Kexts/WhateverGreen.kext/Contents/MacOS/WhateverGreen'), Buffer.alloc(2048, 1));
      fs.writeFileSync(path.join(dir, 'EFI/OC/config.plist'), basePlist(`
      <dict>
        <key>BundlePath</key>
        <string>WhateverGreen.kext</string>
      </dict>`));
      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.code === 'KEXT_LILU_DEPENDENCY'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe('BIOS orchestrator release gates', () => {
  test('default safe-mode BIOS state blocks build until required settings are verified', () => {
    const profile = makeProfile();
    const state = buildBiosOrchestratorState({
      profile,
      biosConfig: getBIOSSettings(profile),
      firmwareInfo: makeFirmwareInfo({ hostContext: 'running_on_mac', confidence: 'not_applicable', requirements: [
        makeRequirement('uefi-mode', 'not_applicable', null),
        makeRequirement('secure-boot', 'not_applicable', null),
        makeRequirement('vt-x', 'not_applicable', null),
        makeRequirement('vt-d', 'not_applicable', null),
        makeRequirement('above4g', 'not_applicable', null),
      ] }),
      platform: 'darwin',
      safeMode: true,
      session: null,
    });

    assert.equal(state.readyToBuild, false);
    assert.ok(state.blockingIssues.length > 0);
  });

  test('manual BIOS confirmations can complete the gate for a Mac-host workflow', () => {
    const profile = makeProfile();
    const macHostInfo = makeFirmwareInfo({
      hostContext: 'running_on_mac',
      confidence: 'not_applicable',
      requirements: [
        makeRequirement('uefi-mode', 'not_applicable', null),
        makeRequirement('secure-boot', 'not_applicable', null),
        makeRequirement('vt-x', 'not_applicable', null),
        makeRequirement('vt-d', 'not_applicable', null),
        makeRequirement('above4g', 'not_applicable', null),
      ],
    });

    const initial = buildBiosOrchestratorState({
      profile,
      biosConfig: getBIOSSettings(profile),
      firmwareInfo: macHostInfo,
      platform: 'darwin',
      safeMode: true,
      session: null,
    });

    const selectedChanges = Object.fromEntries(
      initial.settings.map(setting => [
        setting.id,
        {
          approved: setting.required,
          applyMode: 'manual',
        },
      ]),
    ) as BiosSessionState['selectedChanges'];

    const session: BiosSessionState = {
      sessionId: 'bios-test-session',
      hardwareFingerprint: buildHardwareFingerprint(profile),
      selectedChanges,
      stage: 'complete',
      vendor: 'Dell',
      rebootRequested: false,
      timestamp: Date.now(),
    };

    const completed = buildBiosOrchestratorState({
      profile,
      biosConfig: getBIOSSettings(profile),
      firmwareInfo: macHostInfo,
      platform: 'darwin',
      safeMode: true,
      session,
    });

    assert.equal(completed.readyToBuild, true);
    assert.equal(completed.stage, 'complete');
  });
});
