import { describe, it, expect } from 'vitest';
import {
  getSMBIOSForProfile,
  getQuirksForGeneration,
  getRequiredResources,
  generateConfigPlist,
} from '../electron/configGenerator.js';
import type { HardwareProfile } from '../electron/configGenerator.js';
import { getUnsupportedSsdtRequests } from '../electron/ssdtSourcePolicy.js';

function fakeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i7-10700K',
    architecture: 'Intel',
    generation: 'Comet Lake',
    motherboard: 'ASUS ROG Z490',
    gpu: 'Intel UHD 630',
    ram: '16 GB',
    coreCount: 8,
    targetOS: 'macOS Ventura',
    smbios: 'iMac20,1',
    kexts: [],
    ssdts: [],
    bootArgs: '',
    isLaptop: false,
    ...overrides,
  } as HardwareProfile;
}

// ─── SMBIOS selection ────────────────────────────────────────────────────────

describe('getSMBIOSForProfile — generation coverage', () => {
  const generations = [
    'Penryn', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell',
    'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake',
    'Rocket Lake', 'Alder Lake', 'Raptor Lake',
  ];

  for (const gen of generations) {
    it(`returns a valid SMBIOS for Intel desktop ${gen}`, () => {
      const smbios = getSMBIOSForProfile(fakeProfile({ generation: gen as HardwareProfile['generation'] }));
      expect(smbios).toBeTruthy();
      expect(smbios.length).toBeGreaterThan(0);
      // Must be a real Apple model identifier pattern
      expect(smbios).toMatch(/^(iMac|MacPro|MacBookPro|iMacPro)\d+,\d+$/);
    });
  }

  it('returns MacPro7,1 for AMD Threadripper', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      architecture: 'AMD', generation: 'Threadripper',
    }))).toBe('MacPro7,1');
  });

  it('returns iMacPro1,1 for AMD Ryzen desktop', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      architecture: 'AMD', generation: 'Ryzen',
    }))).toBe('iMacPro1,1');
  });

  it('returns valid SMBIOS for unknown generation (fallback)', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({ generation: 'Unknown' as any }));
    expect(smbios).toMatch(/^(iMac|MacPro|MacBookPro|iMacPro)\d+,\d+$/);
  });

  it('returns laptop SMBIOS for laptop profile', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake', isLaptop: true,
    }));
    expect(smbios).toContain('MacBookPro');
  });

  it('returns VM SMBIOS for VM profile', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({ isVM: true } as any));
    expect(['iMacPro1,1', 'MacPro7,1']).toContain(smbios);
  });
});

// ─── Tahoe SMBIOS correctness (#16) ─────────────────────────────────────────

describe('getSMBIOSForProfile — Tahoe (macOS 26)', () => {
  it('Coffee Lake desktop + Tahoe returns iMac20,1 not iMac19,1', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
    }));
    expect(smbios).toBe('iMac20,1');
  });

  it('Comet Lake desktop + Tahoe returns iMac20,1 or iMac20,2', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Comet Lake',
      targetOS: 'macOS Tahoe 26',
    }));
    expect(['iMac20,1', 'iMac20,2', 'MacPro7,1']).toContain(smbios);
  });

  it('Kaby Lake desktop + Tahoe returns iMac20,1 (not iMac18,x)', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Kaby Lake',
      targetOS: 'macOS Tahoe 26',
    }));
    expect(smbios).toBe('iMac20,1');
  });

  it('Coffee Lake laptop + Tahoe returns MacBookPro16,1', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
      isLaptop: true,
    }));
    expect(smbios).toBe('MacBookPro16,1');
  });

  it('AMD Ryzen + Tahoe returns iMacPro1,1', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      targetOS: 'macOS Tahoe 26',
    }));
    expect(smbios).toBe('iMacPro1,1');
  });

  it('Coffee Lake + Sequoia still returns iMac19,1', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Sequoia 15',
    }));
    expect(smbios).toBe('iMac19,1');
  });
});

// ─── Codeless kext ExecutablePath (#17) ─────────────────────────────────────

describe('generateConfigPlist — codeless kext handling', () => {
  it('AppleMCEReporterDisabler has empty ExecutablePath', () => {
    const plist = generateConfigPlist(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      coreCount: 8,
      smbios: 'iMacPro1,1',
      targetOS: 'macOS Monterey',
      kexts: ['Lilu.kext', 'VirtualSMC.kext', 'AppleMCEReporterDisabler.kext'],
    }));
    // AppleMCEReporterDisabler should have empty ExecutablePath
    // The kext name appears in BundlePath AND Comment, so grab everything after the last occurrence
    const parts = plist.split('AppleMCEReporterDisabler.kext');
    const mceBlock = parts[parts.length - 1];
    expect(mceBlock).toContain('<key>ExecutablePath</key><string></string>');
  });

  it('normal kexts have non-empty ExecutablePath', () => {
    const plist = generateConfigPlist(fakeProfile({
      kexts: ['Lilu.kext', 'VirtualSMC.kext'],
    }));
    expect(plist).toContain('<key>ExecutablePath</key><string>Contents/MacOS/Lilu</string>');
    expect(plist).toContain('<key>ExecutablePath</key><string>Contents/MacOS/VirtualSMC</string>');
  });
});

// ─── audioLayoutId nullish coalescing ───────────────────────────────────────

describe('generateConfigPlist — audioLayoutId', () => {
  it('audioLayoutId 0 is preserved (not replaced with 1)', () => {
    const plist = generateConfigPlist(fakeProfile({ audioLayoutId: 0 }));
    // Layout ID 0 should encode to AAAAAA== (base64 of 0x00 0x00 0x00 0x00)
    expect(plist).toContain('AAAAAA==');
  });
});

// ─── AMD core count guard ────────────────────────────────────────────────────

describe('generateConfigPlist — AMD core count safety', () => {
  it('throws when AMD profile has no coreCount', () => {
    expect(() => generateConfigPlist(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      coreCount: 0,
    }))).toThrow(/core count/i);
  });

  it('throws when AMD profile has undefined coreCount', () => {
    expect(() => generateConfigPlist(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      coreCount: undefined as any,
    }))).toThrow(/core count/i);
  });

  it('succeeds for AMD profile with valid coreCount', () => {
    const plist = generateConfigPlist(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      coreCount: 8,
      smbios: 'iMacPro1,1',
    }));
    expect(plist).toContain('<plist');
    expect(plist).toContain('</plist>');
  });
});

// ─── Config plist structure ──────────────────────────────────────────────────

describe('generateConfigPlist — structural correctness', () => {
  it('generates valid plist XML for Intel Comet Lake', () => {
    const plist = generateConfigPlist(fakeProfile());
    expect(plist).toContain('<?xml');
    expect(plist).toContain('<plist');
    expect(plist).toContain('</plist>');
    expect(plist).toContain('iMac20,1'); // SMBIOS in SystemProductName
  });

  it('generates valid plist XML for Intel laptop', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      smbios: 'MacBookPro15,2',
    }));
    expect(plist).toContain('<plist');
    expect(plist).toContain('MacBookPro15,2');
  });

  it('includes Lilu and VirtualSMC in kernel add', () => {
    const plist = generateConfigPlist(fakeProfile());
    expect(plist).toContain('Lilu.kext');
    expect(plist).toContain('VirtualSMC.kext');
  });
});

// ─── Quirks generation ───────────────────────────────────────────────────────

describe('getQuirksForGeneration', () => {
  it('legacy generations use EnableWriteUnprotector', () => {
    for (const gen of ['Penryn', 'Sandy Bridge', 'Ivy Bridge'] as HardwareProfile['generation'][]) {
      const q = getQuirksForGeneration(gen);
      expect(q.EnableWriteUnprotector).toBe(true);
      expect(q.RebuildAppleMemoryMap).toBe(false);
    }
  });

  it('modern generations do NOT use EnableWriteUnprotector by default', () => {
    for (const gen of ['Coffee Lake', 'Comet Lake', 'Alder Lake'] as HardwareProfile['generation'][]) {
      const q = getQuirksForGeneration(gen);
      expect(q.EnableWriteUnprotector).toBeFalsy();
    }
  });

  it('conservative strategy enables extra safety quirks', () => {
    const q = getQuirksForGeneration('Coffee Lake', '', false, 'conservative');
    expect(q.DevirtualiseMmio).toBe(true);
    expect(q.SetupVirtualMap).toBe(true);
    expect(q.DisableIoMapper).toBe(true);
  });

  it('VM profile enables ProvideCurrentCpuInfo', () => {
    const q = getQuirksForGeneration('Coffee Lake', '', true);
    expect(q.ProvideCurrentCpuInfo).toBe(true);
  });
});

// ─── Required resources ──────────────────────────────────────────────────────

describe('getRequiredResources', () => {
  it('always includes Lilu and VirtualSMC', () => {
    const r = getRequiredResources(fakeProfile());
    expect(r.kexts).toContain('Lilu.kext');
    expect(r.kexts).toContain('VirtualSMC.kext');
  });

  it('includes WhateverGreen for Intel', () => {
    const r = getRequiredResources(fakeProfile({ architecture: 'Intel' }));
    expect(r.kexts).toContain('WhateverGreen.kext');
  });

  it('includes AppleMCEReporterDisabler for AMD Monterey+', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      targetOS: 'macOS Monterey',
    }));
    expect(r.kexts).toContain('AppleMCEReporterDisabler.kext');
  });

  it('does NOT include AppleMCEReporterDisabler for Intel', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'Intel',
      generation: 'Comet Lake',
      targetOS: 'macOS Monterey',
    }));
    expect(r.kexts).not.toContain('AppleMCEReporterDisabler.kext');
  });

  it('includes laptop kexts for laptop profile', () => {
    const r = getRequiredResources(fakeProfile({ isLaptop: true }));
    expect(r.kexts).toContain('SMCBatteryManager.kext');
    expect(r.kexts).toContain('VoodooPS2Controller.kext');
  });

  it('uses SSDT-AWAC for Coffee Lake desktops', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'Intel',
      generation: 'Coffee Lake',
      motherboard: 'ASUS Z390',
    }));
    expect(r.ssdts).toContain('SSDT-AWAC.aml');
  });

  it('uses the canonical AMD desktop EC/USBX SSDT name', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'MSI B650',
    }));
    expect(r.ssdts).toContain('SSDT-EC-USBX-DESKTOP.aml');
    expect(r.ssdts).not.toContain('SSDT-EC-USBX-AMD.aml');
  });

  it('includes SSDT-CPUR for newer AMD chipsets that require it', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'Gigabyte B650 AORUS',
    }));
    expect(r.ssdts).toContain('SSDT-CPUR.aml');
  });

  it('does not include SSDT-CPUR for older AMD chipsets', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'ASUS X570',
    }));
    expect(r.ssdts).not.toContain('SSDT-CPUR.aml');
  });

  it('keeps the shipped SSDT matrix sourceable for AMD desktop, Coffee Lake, Alder Lake, and Raptor Lake', () => {
    const scenarios = [
      {
        name: 'AMD desktop',
        profile: fakeProfile({
          architecture: 'AMD',
          generation: 'Ryzen',
          motherboard: 'MSI B650',
        }),
        expectedSsdts: ['SSDT-EC-USBX-DESKTOP.aml', 'SSDT-CPUR.aml'],
      },
      {
        name: 'Coffee Lake',
        profile: fakeProfile({
          architecture: 'Intel',
          generation: 'Coffee Lake',
          motherboard: 'ASUS Prime Z390-A',
        }),
        expectedSsdts: ['SSDT-PLUG.aml', 'SSDT-AWAC.aml', 'SSDT-EC-USBX.aml', 'SSDT-PMC.aml'],
      },
      {
        name: 'Alder Lake',
        profile: fakeProfile({
          architecture: 'Intel',
          generation: 'Alder Lake',
          motherboard: 'MSI Z690 Tomahawk',
        }),
        expectedSsdts: ['SSDT-PLUG-ALT.aml', 'SSDT-AWAC.aml', 'SSDT-EC-USBX.aml'],
      },
      {
        name: 'Raptor Lake',
        profile: fakeProfile({
          architecture: 'Intel',
          generation: 'Raptor Lake',
          motherboard: 'Gigabyte Z790 AORUS',
        }),
        expectedSsdts: ['SSDT-PLUG-ALT.aml', 'SSDT-AWAC.aml', 'SSDT-EC-USBX.aml'],
      },
    ];

    for (const scenario of scenarios) {
      const resources = getRequiredResources(scenario.profile);
      expect(resources.ssdts, scenario.name).toEqual(scenario.expectedSsdts);
      expect(getUnsupportedSsdtRequests(resources.ssdts), scenario.name).toEqual([]);
    }
  });
});
