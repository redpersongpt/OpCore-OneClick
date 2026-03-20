import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { checkCompatibility } from '../electron/compatibility.js';
import { getRequiredResources, getSMBIOSForProfile, type HardwareProfile } from '../electron/configGenerator.js';
import { validateEfi } from '../electron/configValidator.js';
import {
  isValidationBlockingDeployment,
  recoveryResumeDecision,
  restoreFlowDecision,
  targetSelectionDecision,
} from '../src/lib/releaseFlow.js';
import {
  needsAppleMceReporterDisabler,
  requiresSecureBootModelForAirportItlwm,
  shouldSelectXhciUnsupported,
  validateKextSelection,
} from '../src/data/kextRegistry.js';

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

function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `hackintosh-${prefix}-`));
}

function minimalConfigPlist(options: {
  secureBootModel?: string;
  kernelAddEntries?: string;
} = {}): string {
  const {
    secureBootModel = 'Default',
    kernelAddEntries = '',
  } = options;
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
    <array>
${kernelAddEntries}
    </array>
    <key>Patch</key>
    <array />
  </dict>
  <key>Misc</key>
  <dict>
    <key>Security</key>
    <dict>
      <key>SecureBootModel</key>
      <string>${secureBootModel}</string>
    </dict>
  </dict>
  <key>UEFI</key>
  <dict>
    <key>Drivers</key>
    <array>
      <dict>
        <key>Enabled</key>
        <true/>
        <key>Path</key>
        <string>HfsPlus.efi</string>
      </dict>
      <dict>
        <key>Enabled</key>
        <true/>
        <key>Path</key>
        <string>OpenRuntime.efi</string>
      </dict>
    </array>
  </dict>
</dict>
</plist>
<!-- padding for validator size check -->
<!-- padding for validator size check -->
<!-- padding for validator size check -->
<!-- padding for validator size check -->`;
}

function writeMinimalEfi(root: string, opts: {
  includeConfig?: boolean;
  includeOpenCore?: boolean;
  includeBoot?: boolean;
  includeOpenRuntime?: boolean;
  includeHfsPlus?: boolean;
} = {}) {
  const {
    includeConfig = true,
    includeOpenCore = true,
    includeBoot = true,
    includeOpenRuntime = true,
    includeHfsPlus = true,
  } = opts;

  fs.mkdirSync(path.join(root, 'EFI/BOOT'), { recursive: true });
  fs.mkdirSync(path.join(root, 'EFI/OC/Drivers'), { recursive: true });
  fs.mkdirSync(path.join(root, 'EFI/OC/Kexts'), { recursive: true });
  fs.mkdirSync(path.join(root, 'EFI/OC/ACPI'), { recursive: true });

  if (includeConfig) {
    fs.writeFileSync(path.join(root, 'EFI/OC/config.plist'), minimalConfigPlist());
  }
  if (includeOpenCore) {
    fs.writeFileSync(path.join(root, 'EFI/OC/OpenCore.efi'), Buffer.alloc(120 * 1024, 1));
    fs.writeFileSync(path.join(root, 'EFI/OC/OpenCore.efi.version'), '1.0.3');
  }
  if (includeBoot) {
    fs.writeFileSync(path.join(root, 'EFI/BOOT/BOOTx64.efi'), Buffer.alloc(40 * 1024, 1));
    fs.writeFileSync(path.join(root, 'EFI/BOOT/BOOTx64.efi.version'), '1.0.3');
  }
  if (includeOpenRuntime) {
    fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/OpenRuntime.efi'), Buffer.alloc(40 * 1024, 1));
    fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/OpenRuntime.efi.version'), '1.0.3');
  }
  if (includeHfsPlus) {
    fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/HfsPlus.efi'), Buffer.alloc(40 * 1024, 1));
    fs.writeFileSync(path.join(root, 'EFI/OC/Drivers/HfsPlus.efi.version'), '1.0.3');
  }
}

describe('Hackintosh compatibility logic', () => {
  test('unsupported RTX desktop with no iGPU is blocked', () => {
    const report = checkCompatibility(makeProfile({
      gpu: 'NVIDIA GeForce RTX 3070',
      gpuDevices: [{ name: 'NVIDIA GeForce RTX 3070', vendorName: 'NVIDIA' }],
    }));

    assert.equal(report.level, 'blocked');
    assert.ok(report.errors.some(error => /no supported display path/i.test(error)));
  });

  test('unsupported RTX laptop with Intel iGPU remains non-blocked but warned', () => {
    const report = checkCompatibility(makeProfile({
      isLaptop: true,
      gpu: 'Intel UHD Graphics 630 / NVIDIA GeForce RTX 3050 Ti',
      gpuDevices: [
        { name: 'Intel UHD Graphics 630', vendorName: 'Intel' },
        { name: 'NVIDIA GeForce RTX 3050 Ti', vendorName: 'NVIDIA' },
      ],
    }));

    assert.notEqual(report.level, 'blocked');
    assert.ok(report.warnings.some(warning => /discrete gpu/i.test(warning)));
  });

  test('older supported GPUs still expose eligible fallback macOS versions', () => {
    const report = checkCompatibility(makeProfile({
      gpu: 'NVIDIA GeForce GTX 1080',
      gpuDevices: [{ name: 'NVIDIA GeForce GTX 1080', vendorName: 'NVIDIA' }],
      targetOS: 'macOS Sequoia 15',
    }));

    assert.equal(report.level, 'blocked');
    assert.ok(report.eligibleVersions.some(version => version.id === '10.13'));
    assert.ok(report.errors.some(error => /Choose macOS High Sierra 10\.13/i.test(error)));
  });

  test('Intel GT1 / UHD 610-only systems are rejected', () => {
    const report = checkCompatibility(makeProfile({
      gpu: 'Intel UHD Graphics 610',
      gpuDevices: [{ name: 'Intel UHD Graphics 610', vendorName: 'Intel' }],
    }));

    assert.equal(report.level, 'blocked');
    assert.ok(report.errors.some(error => /display path/i.test(error)));
  });

  test('Coffee Lake desktop SMBIOS stays on iMac19,1', () => {
    const smbios = getSMBIOSForProfile(makeProfile());
    assert.equal(smbios, 'iMac19,1');
  });

  test('Coffee Lake desktop with supported iGPU remains buildable', () => {
    const report = checkCompatibility(makeProfile());
    assert.notEqual(report.level, 'blocked');
    assert.equal(report.isCompatible, true);
  });

  test('AMD Ryzen with native AMD dGPU selects MacPro7,1', () => {
    const amdProfile = makeProfile({
      cpu: 'AMD Ryzen 7 5700X',
      architecture: 'AMD',
      generation: 'Ryzen',
      gpu: 'AMD Radeon RX 6600 XT',
      gpuDevices: [{ name: 'AMD Radeon RX 6600 XT', vendorName: 'AMD' }],
      motherboard: 'ASUS B550-F Gaming',
      smbios: '',
    });

    const smbios = getSMBIOSForProfile(amdProfile);
    assert.equal(smbios, 'MacPro7,1');
  });

  test('AMD Ryzen desktop with supported AMD dGPU remains buildable', () => {
    const report = checkCompatibility(makeProfile({
      cpu: 'AMD Ryzen 7 5700X',
      architecture: 'AMD',
      generation: 'Ryzen',
      gpu: 'AMD Radeon RX 6600 XT',
      gpuDevices: [{ name: 'AMD Radeon RX 6600 XT', vendorName: 'AMD' }],
      motherboard: 'ASUS B550-F Gaming',
      smbios: 'MacPro7,1',
    }));

    assert.notEqual(report.level, 'blocked');
    assert.equal(report.isCompatible, true);
  });

  test('persisted-state restore falls back to report when compatibility becomes blocked', () => {
    const decision = restoreFlowDecision(makeProfile({
      gpu: 'NVIDIA GeForce RTX 3070',
      gpuDevices: [{ name: 'NVIDIA GeForce RTX 3070', vendorName: 'NVIDIA' }],
    }), 'method-select');

    assert.equal(decision.restoredStep, 'report');
    assert.equal(decision.canReuseExistingEfi, false);
    assert.equal(decision.canResumeRecovery, false);
    assert.equal(decision.compatibility.level, 'blocked');
  });

  test('changing target macOS after scan recomputes compatibility and clears a stale blocked target', () => {
    const blockedProfile = makeProfile({
      gpu: 'NVIDIA GeForce GTX 1080',
      gpuDevices: [{ name: 'NVIDIA GeForce GTX 1080', vendorName: 'NVIDIA' }],
      targetOS: 'macOS Sequoia 15',
    });

    const blockedReport = checkCompatibility(blockedProfile);
    assert.equal(blockedReport.level, 'blocked');

    const decision = targetSelectionDecision(blockedProfile, 'macOS High Sierra 10.13');
    assert.equal(decision.profile.targetOS, 'macOS High Sierra 10.13');
    assert.equal(decision.compatibility.isCompatible, true);
    assert.equal(decision.nextStep, 'report');
    assert.equal(decision.resetExistingBuild, true);
  });

  test('recovery resume is blocked when the restored EFI is no longer valid', () => {
    const decision = recoveryResumeDecision({
      compatibilityBlocked: false,
      biosReady: true,
      efiReady: false,
    });

    assert.equal(decision.canResume, false);
    assert.equal(decision.redirect, 'report');
    assert.match(decision.message ?? '', /efi no longer passes validation/i);
  });
});

describe('Kext selection rules', () => {
  test('AirportItlwm requires SecureBootModel-enabled behavior', () => {
    assert.equal(requiresSecureBootModelForAirportItlwm(['AirportItlwm']), true);
    const issues = validateKextSelection(['Lilu', 'AirportItlwm'], {
      macOSVersion: 'macOS Sequoia 15',
      isAMD: false,
      secureBootModelEnabled: false,
    });
    assert.ok(issues.some(issue => issue.code === 'KEXT_AIRPORTITLWM_SECUREBOOT_REQUIRED' && issue.severity === 'blocked'));
  });

  test('AppleMCEReporterDisabler is version-sensitive', () => {
    assert.equal(needsAppleMceReporterDisabler({ isAMD: true, macOSVersion: 'macOS Big Sur 11' }), false);
    assert.equal(needsAppleMceReporterDisabler({ isAMD: true, macOSVersion: 'macOS Monterey 12' }), true);

    const bigSur = getRequiredResources(makeProfile({
      cpu: 'AMD Ryzen 5 5600X',
      architecture: 'AMD',
      generation: 'Ryzen',
      gpu: 'AMD Radeon RX 580',
      gpuDevices: [{ name: 'AMD Radeon RX 580', vendorName: 'AMD' }],
      targetOS: 'macOS Big Sur 11',
    }));
    const monterey = getRequiredResources(makeProfile({
      cpu: 'AMD Ryzen 5 5600X',
      architecture: 'AMD',
      generation: 'Ryzen',
      gpu: 'AMD Radeon RX 580',
      gpuDevices: [{ name: 'AMD Radeon RX 580', vendorName: 'AMD' }],
      targetOS: 'macOS Monterey 12',
    }));

    assert.equal(bigSur.kexts.includes('AppleMCEReporterDisabler.kext'), false);
    assert.equal(monterey.kexts.includes('AppleMCEReporterDisabler.kext'), true);
  });

  test('XHCI-unsupported is not broadly forced onto generic AMD systems', () => {
    assert.equal(shouldSelectXhciUnsupported({
      isAMD: true,
      isHEDT: false,
      motherboard: 'MSI B650 Tomahawk',
      macOSVersion: 'macOS Sequoia 15',
    }), false);
  });
});

describe('EFI validator', () => {
  test('catches missing OpenRuntime.efi', async () => {
    const dir = makeTempDir('efi-openruntime');
    try {
      writeMinimalEfi(dir, { includeOpenRuntime: false });
      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.expectedPath === 'EFI/OC/Drivers/OpenRuntime.efi'));
      assert.equal(isValidationBlockingDeployment(result), true);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('catches missing BOOTx64/OpenCore/config layout', async () => {
    const dir = makeTempDir('efi-layout');
    try {
      fs.mkdirSync(dir, { recursive: true });
      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.expectedPath === 'EFI/BOOT/BOOTx64.efi'));
      assert.ok(result.issues.some(issue => issue.expectedPath === 'EFI/OC/OpenCore.efi'));
      assert.ok(result.issues.some(issue => issue.expectedPath === 'EFI/OC/config.plist'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('catches kext bundles with missing declared executable', async () => {
    const dir = makeTempDir('efi-kext-exec');
    try {
      writeMinimalEfi(dir);
      fs.mkdirSync(path.join(dir, 'EFI/OC/Kexts/Broken.kext/Contents/MacOS'), { recursive: true });
      fs.writeFileSync(path.join(dir, 'EFI/OC/Kexts/Broken.kext/Contents/Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>BrokenBinary</string>
</dict>
</plist>`);
      fs.writeFileSync(path.join(dir, 'EFI/OC/config.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>ACPI</key>
  <dict><key>Add</key><array /></dict>
  <key>Kernel</key>
  <dict>
    <key>Add</key>
    <array>
      <dict>
        <key>BundlePath</key>
        <string>Broken.kext</string>
      </dict>
    </array>
    <key>Patch</key><array />
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
    <array>
      <dict><key>Enabled</key><true/><key>Path</key><string>HfsPlus.efi</string></dict>
      <dict><key>Enabled</key><true/><key>Path</key><string>OpenRuntime.efi</string></dict>
    </array>
  </dict>
</dict>
</plist>
<!-- padding for validator size check -->
<!-- padding for validator size check -->
<!-- padding for validator size check -->`);

      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.code === 'KEXT_EXECUTABLE_MISSING'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('blocks AirportItlwm when SecureBootModel is disabled', async () => {
    const dir = makeTempDir('efi-airportitlwm');
    try {
      writeMinimalEfi(dir);
      fs.mkdirSync(path.join(dir, 'EFI/OC/Kexts/AirportItlwm.kext/Contents/MacOS'), { recursive: true });
      fs.writeFileSync(path.join(dir, 'EFI/OC/Kexts/AirportItlwm.kext/Contents/Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>AirportItlwm</string>
</dict>
</plist>`);
      fs.writeFileSync(path.join(dir, 'EFI/OC/Kexts/AirportItlwm.kext/Contents/MacOS/AirportItlwm'), Buffer.alloc(2048, 1));
      fs.writeFileSync(path.join(dir, 'EFI/OC/config.plist'), minimalConfigPlist({
        secureBootModel: 'Disabled',
        kernelAddEntries: `      <dict>
        <key>BundlePath</key>
        <string>AirportItlwm.kext</string>
      </dict>`,
      }));

      const result = await validateEfi(dir, makeProfile(), {});
      assert.equal(result.overall, 'blocked');
      assert.ok(result.issues.some(issue => issue.code === 'AIRPORTITLWM_SECUREBOOT_REQUIRED'));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
