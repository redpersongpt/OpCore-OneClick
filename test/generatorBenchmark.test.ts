/**
 * Generator Benchmark Matrix — OpCore-OneClick EFI Generator
 *
 * This test suite systematically validates every generation × OS × GPU path
 * combination against Dortania-correct expected values. Each test case
 * encodes the policy decision that a generic generator would likely get wrong.
 *
 * Benchmark categories:
 * 1. SMBIOS correctness per generation/OS/GPU combo
 * 2. ig-platform-id correctness (display vs headless)
 * 3. SSDT set correctness per generation
 * 4. Kext set correctness per architecture/generation
 * 5. Quirk correctness per generation/chipset
 * 6. SIP policy correctness per GPU path
 * 7. Boot-args correctness per hardware combo
 * 8. Fail-fast honesty for unsupported combos
 * 9. PlatformInfo policy (UpdateSMBIOSMode, AdviseFeatures)
 * 10. Dependency integrity (Lilu→plugins, VirtualSMC→SMC*)
 */

import { describe, it, expect } from 'vitest';
import {
  getSMBIOSForProfile,
  getQuirksForGeneration,
  getRequiredResources,
  generateConfigPlist,
  getSIPPolicy,
} from '../electron/configGenerator.js';
import { getAMDPatches, AMD_PATCH_COMPLETENESS } from '../electron/amdPatches.js';
import type { HardwareProfile } from '../electron/configGenerator.js';

function profile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
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

// ═══════════════════════════════════════════════════════════════════════════════
// 1. SMBIOS CORRECTNESS — the #1 failure class in generic generators
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: SMBIOS correctness', () => {
  // Intel desktop — Tahoe path (most complex)
  const TAHOE_DESKTOP_SMBIOS: Array<{ gen: HardwareProfile['generation']; expected: string; reason: string }> = [
    { gen: 'Skylake',     expected: 'iMac20,1',  reason: 'Skylake desktop runs headless iGPU compute under iMac20,1 on Tahoe' },
    { gen: 'Kaby Lake',   expected: 'iMac20,1',  reason: 'Kaby Lake same as Skylake for Tahoe' },
    { gen: 'Coffee Lake', expected: 'iMac20,1',  reason: 'Coffee Lake is the natural iMac20,1 generation' },
    { gen: 'Comet Lake',  expected: 'iMac20,1',  reason: 'Comet Lake desktop uses iMac20,1 on Tahoe' },
    { gen: 'Rocket Lake', expected: 'MacPro7,1', reason: 'Rocket Lake has no macOS iGPU driver' },
    { gen: 'Alder Lake',  expected: 'MacPro7,1', reason: 'Alder Lake has no macOS iGPU driver' },
    { gen: 'Raptor Lake', expected: 'MacPro7,1', reason: 'Raptor Lake has no macOS iGPU driver' },
  ];

  for (const { gen, expected, reason } of TAHOE_DESKTOP_SMBIOS) {
    it(`Tahoe desktop: ${gen} → ${expected} (${reason})`, () => {
      expect(getSMBIOSForProfile(profile({
        generation: gen,
        targetOS: 'macOS Tahoe 26',
      }))).toBe(expected);
    });
  }

  // Tahoe should THROW for pre-Skylake
  for (const gen of ['Penryn', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell'] as const) {
    it(`Tahoe desktop: ${gen} throws (unsupported)`, () => {
      expect(() => getSMBIOSForProfile(profile({
        generation: gen,
        targetOS: 'macOS Tahoe 26',
      }))).toThrow(/not supported/i);
    });
  }

  // HEDT always gets MacPro7,1 on Tahoe
  for (const gen of ['Ivy Bridge-E', 'Haswell-E', 'Broadwell-E', 'Cascade Lake-X'] as const) {
    it(`Tahoe HEDT: ${gen} → MacPro7,1`, () => {
      expect(getSMBIOSForProfile(profile({
        generation: gen,
        targetOS: 'macOS Tahoe 26',
      }))).toBe('MacPro7,1');
    });
  }

  // Coffee Lake dGPU on Tahoe must be iMac20,1, NOT MacPro7,1
  it('Coffee Lake + AMD dGPU on Tahoe → iMac20,1 (not MacPro7,1)', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
      gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'AMD Radeon RX 580' }],
    }))).toBe('iMac20,1');
  });

  // AMD SMBIOS selection
  it('AMD Ryzen + Mac Pro era GPU → MacPro7,1', () => {
    expect(getSMBIOSForProfile(profile({
      architecture: 'AMD', generation: 'Ryzen', targetOS: 'macOS Ventura',
      gpuDevices: [{ name: 'AMD Radeon RX 6800 XT' }],
    }))).toBe('MacPro7,1');
  });

  it('AMD Ryzen without Mac Pro era GPU → iMacPro1,1', () => {
    expect(getSMBIOSForProfile(profile({
      architecture: 'AMD', generation: 'Ryzen', targetOS: 'macOS Ventura',
      gpuDevices: [{ name: 'AMD Radeon Vega 8' }],
    }))).toBe('iMacPro1,1');
  });

  it('AMD Threadripper → MacPro7,1', () => {
    expect(getSMBIOSForProfile(profile({
      architecture: 'AMD', generation: 'Threadripper', targetOS: 'macOS Ventura',
    }))).toBe('MacPro7,1');
  });

  // Ice Lake laptop
  it('Ice Lake laptop → MacBookPro16,2', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Ice Lake', isLaptop: true, targetOS: 'macOS Ventura',
    }))).toBe('MacBookPro16,2');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 2. ig-platform-id CORRECTNESS — headless vs display
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: ig-platform-id policy', () => {
  const IG_PLATFORM_IDS: Record<string, { display: string; headless: string }> = {
    'Haswell':     { display: 'AwAiDQ==', headless: 'BAASBA==' },
    'Broadwell':   { display: 'BwAiFg==', headless: 'BgAmFg==' },
    'Skylake':     { display: 'AAASGQ==', headless: 'AQASGQ==' },
    'Kaby Lake':   { display: 'AAASWQ==', headless: 'AwASWQ==' },
    'Coffee Lake': { display: 'BwCbPg==', headless: 'AwCRPg==' },
    'Comet Lake':  { display: 'BwCbPg==', headless: 'AwDImw==' },
  };

  for (const [gen, ids] of Object.entries(IG_PLATFORM_IDS)) {
    it(`${gen} display iGPU → ${ids.display}`, () => {
      const plist = generateConfigPlist(profile({
        generation: gen as HardwareProfile['generation'],
        smbios: gen === 'Coffee Lake' ? 'iMac19,1' : 'iMac20,1',
        gpuDevices: [{ name: 'Intel UHD 630' }],
      }));
      expect(plist).toContain(ids.display);
      expect(plist).not.toContain(ids.headless);
    });

    it(`${gen} headless iGPU (dGPU present) → ${ids.headless}`, () => {
      const plist = generateConfigPlist(profile({
        generation: gen as HardwareProfile['generation'],
        smbios: gen === 'Coffee Lake' ? 'iMac19,1' : 'iMac20,1',
        gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'AMD Radeon RX 580' }],
      }));
      expect(plist).toContain(ids.headless);
    });

    it(`${gen} headless has NO framebuffer patches`, () => {
      const plist = generateConfigPlist(profile({
        generation: gen as HardwareProfile['generation'],
        smbios: gen === 'Coffee Lake' ? 'iMac19,1' : 'iMac20,1',
        gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'AMD Radeon RX 580' }],
      }));
      expect(plist).not.toContain('framebuffer-patch-enable');
      expect(plist).not.toContain('framebuffer-stolenmem');
    });
  }

  // Alder/Raptor/Rocket Lake should have NO iGPU properties at all
  for (const gen of ['Alder Lake', 'Raptor Lake', 'Rocket Lake'] as const) {
    it(`${gen} has no iGPU DeviceProperties`, () => {
      const plist = generateConfigPlist(profile({
        generation: gen,
        smbios: 'MacPro7,1',
      }));
      expect(plist).not.toContain('ig-platform-id');
    });
  }
});

// ═══════════════════════════════════════════════════════════════════════════════
// 3. QUIRK POLICY — generation × chipset correctness
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: Quirk policy correctness', () => {
  // AppleXcpmExtraMsrs — HEDT only
  for (const gen of ['Ivy Bridge-E', 'Haswell-E', 'Broadwell-E', 'Cascade Lake-X'] as const) {
    it(`${gen} enables AppleXcpmExtraMsrs`, () => {
      expect(getQuirksForGeneration(gen).AppleXcpmExtraMsrs).toBe(true);
    });
  }

  // Non-HEDT should NOT have AppleXcpmExtraMsrs
  for (const gen of ['Sandy Bridge', 'Haswell', 'Coffee Lake', 'Comet Lake', 'Ryzen'] as const) {
    it(`${gen} does NOT enable AppleXcpmExtraMsrs`, () => {
      expect(getQuirksForGeneration(gen).AppleXcpmExtraMsrs).toBe(false);
    });
  }

  // FixupAppleEfiImages on Tahoe — all modern Intel + AMD
  for (const gen of ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake'] as const) {
    it(`${gen} on Tahoe enables FixupAppleEfiImages`, () => {
      expect(getQuirksForGeneration(gen, '', false, 'canonical', 'macOS Tahoe 26').FixupAppleEfiImages).toBe(true);
    });
  }
  it('AMD Ryzen on Tahoe enables FixupAppleEfiImages', () => {
    expect(getQuirksForGeneration('Ryzen', '', false, 'canonical', 'macOS Tahoe 26').FixupAppleEfiImages).toBe(true);
  });

  // FixupAppleEfiImages NOT on pre-Tahoe
  it('Coffee Lake on Ventura does NOT enable FixupAppleEfiImages', () => {
    expect(getQuirksForGeneration('Coffee Lake', '', false, 'canonical', 'macOS Ventura').FixupAppleEfiImages).toBe(false);
  });

  // Z390 needs ProtectUefiServices
  it('Coffee Lake Z390 enables ProtectUefiServices', () => {
    expect(getQuirksForGeneration('Coffee Lake', 'ASUS Z390').ProtectUefiServices).toBe(true);
  });

  // Z490 Comet Lake: SetupVirtualMap=false
  it('Comet Lake Z490: SetupVirtualMap=false, ProtectUefiServices=true', () => {
    const q = getQuirksForGeneration('Comet Lake', 'MSI Z490');
    expect(q.SetupVirtualMap).toBe(false);
    expect(q.ProtectUefiServices).toBe(true);
  });

  // AMD X570/B550: SetupVirtualMap=false
  it('AMD Ryzen B550: SetupVirtualMap=false', () => {
    expect(getQuirksForGeneration('Ryzen', 'MSI B550').SetupVirtualMap).toBe(false);
  });

  // ASUS boards: DisableRtcChecksum
  it('ASUS board enables DisableRtcChecksum', () => {
    expect(getQuirksForGeneration('Coffee Lake', 'ASUS ROG Strix Z390').DisableRtcChecksum).toBe(true);
  });
  it('ROG board enables DisableRtcChecksum', () => {
    expect(getQuirksForGeneration('Comet Lake', 'ROG Maximus Z490').DisableRtcChecksum).toBe(true);
  });
  it('TUF board enables DisableRtcChecksum', () => {
    expect(getQuirksForGeneration('Alder Lake', 'TUF Gaming Z690').DisableRtcChecksum).toBe(true);
  });
  it('MSI board does NOT enable DisableRtcChecksum', () => {
    expect(getQuirksForGeneration('Coffee Lake', 'MSI Z390').DisableRtcChecksum).toBe(false);
  });

  // Skylake+ should have AppleCpuPmCfgLock=false
  for (const gen of ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake', 'Alder Lake'] as const) {
    it(`${gen} has AppleCpuPmCfgLock=false (XCPM only)`, () => {
      expect(getQuirksForGeneration(gen).AppleCpuPmCfgLock).toBe(false);
    });
  }

  // Pre-Skylake should keep AppleCpuPmCfgLock=true
  for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
    it(`${gen} has AppleCpuPmCfgLock=true`, () => {
      expect(getQuirksForGeneration(gen).AppleCpuPmCfgLock).toBe(true);
    });
  }
});

// ═══════════════════════════════════════════════════════════════════════════════
// 4. SIP POLICY — context-aware, not blunt full-disable
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: SIP policy intelligence', () => {
  it('Standard supported GPU path → 0x67 (Dortania minimum)', () => {
    const sip = getSIPPolicy(
      profile({ targetOS: 'macOS Ventura' }),
      [{ name: 'AMD Radeon RX 580' }],
    );
    expect(sip.value).toBe('ZwAAAA=='); // 0x67
  });

  it('Fully supported Intel iGPU → 0x67', () => {
    const sip = getSIPPolicy(
      profile({ targetOS: 'macOS Ventura' }),
      [{ name: 'Intel UHD 630' }],
    );
    expect(sip.value).toBe('ZwAAAA==');
  });

  it('Standard AMD build → 0x67', () => {
    const sip = getSIPPolicy(
      profile({ architecture: 'AMD', generation: 'Ryzen', targetOS: 'macOS Ventura' }),
      [{ name: 'AMD Radeon RX 6800 XT' }],
    );
    expect(sip.value).toBe('ZwAAAA==');
  });

  it('SIP value appears in generated plist', () => {
    const plist = generateConfigPlist(profile({ targetOS: 'macOS Ventura 13' }));
    expect(plist).toContain('ZwAAAA=='); // 0x67 not 0xFFF
    expect(plist).not.toContain('/w8AAA=='); // old full-disable
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 5. BOOT-ARGS — targeted, not shotgun
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: Boot-args precision', () => {
  it('dk.e1000=0 only for Comet Lake+ (I225-V boards)', () => {
    const clPlist = generateConfigPlist(profile({ generation: 'Comet Lake' }));
    expect(clPlist).toContain('dk.e1000=0');

    const sklPlist = generateConfigPlist(profile({ generation: 'Skylake', smbios: 'iMac17,1' }));
    expect(sklPlist).not.toContain('dk.e1000=0');
  });

  it('agdpmod=pikera for Polaris on iMac SMBIOS', () => {
    const plist = generateConfigPlist(profile({
      generation: 'Coffee Lake',
      smbios: 'iMac19,1',
      gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'AMD Radeon RX 580' }],
    }));
    expect(plist).toContain('agdpmod=pikera');
  });

  it('Rocket Lake gets CPUID spoof', () => {
    const plist = generateConfigPlist(profile({
      generation: 'Rocket Lake', smbios: 'MacPro7,1',
    }));
    expect(plist).toContain('VQYKAAAAAAAAAAAAAAAAAA==');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 6. KEXT DEPENDENCY INTEGRITY
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: Kext dependency integrity', () => {
  it('Lilu always first in kext list', () => {
    const { kexts } = getRequiredResources(profile());
    expect(kexts[0]).toBe('Lilu.kext');
  });

  it('VirtualSMC always second in kext list', () => {
    const { kexts } = getRequiredResources(profile());
    expect(kexts[1]).toBe('VirtualSMC.kext');
  });

  it('SMCProcessor comes after VirtualSMC', () => {
    const { kexts } = getRequiredResources(profile());
    const smcIdx = kexts.indexOf('VirtualSMC.kext');
    const procIdx = kexts.indexOf('SMCProcessor.kext');
    expect(procIdx).toBeGreaterThan(smcIdx);
  });

  it('AppleALC comes after Lilu', () => {
    const { kexts } = getRequiredResources(profile());
    const liluIdx = kexts.indexOf('Lilu.kext');
    const alcIdx = kexts.indexOf('AppleALC.kext');
    expect(alcIdx).toBeGreaterThan(liluIdx);
  });

  it('AMD desktop: Lilu and VirtualSMC always present', () => {
    const { kexts } = getRequiredResources(profile({
      architecture: 'AMD', generation: 'Ryzen',
      gpuDevices: [{ name: 'AMD Radeon RX 6800' }],
    }));
    expect(kexts).toContain('Lilu.kext');
    expect(kexts).toContain('VirtualSMC.kext');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 7. SSDT MATRIX — per-generation correctness
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: SSDT matrix', () => {
  it('Pre-Sandy Bridge: NO SSDT-PLUG (no XCPM)', () => {
    for (const gen of ['Penryn', 'Nehalem', 'Westmere'] as const) {
      const { ssdts } = getRequiredResources(profile({ generation: gen }));
      expect(ssdts, gen).not.toContain('SSDT-PLUG.aml');
      expect(ssdts, gen).toContain('SSDT-EC.aml');
    }
  });

  it('HEDT gets SSDT-PLUG + SSDT-EC-USBX', () => {
    for (const gen of ['Haswell-E', 'Broadwell-E', 'Cascade Lake-X'] as const) {
      const { ssdts } = getRequiredResources(profile({ generation: gen }));
      expect(ssdts, gen).toContain('SSDT-PLUG.aml');
      expect(ssdts, gen).toContain('SSDT-EC-USBX.aml');
    }
  });

  it('Cascade Lake-X includes SSDT-UNC', () => {
    const { ssdts } = getRequiredResources(profile({ generation: 'Cascade Lake-X' }));
    expect(ssdts).toContain('SSDT-UNC.aml');
  });

  it('Ice Lake laptop includes SSDT-AWAC', () => {
    const { ssdts } = getRequiredResources(profile({ generation: 'Ice Lake', isLaptop: true }));
    expect(ssdts).toContain('SSDT-AWAC.aml');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 8. PLATFORMINFO POLICY
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: PlatformInfo policy', () => {
  it('AdviseFeatures always false', () => {
    const plist = generateConfigPlist(profile());
    expect(plist).toContain('<key>AdviseFeatures</key><false/>');
  });

  it('UpdateSMBIOSMode: Custom only for MacPro7,1', () => {
    const macPro = generateConfigPlist(profile({ generation: 'Alder Lake', smbios: 'MacPro7,1' }));
    expect(macPro).toContain('UpdateSMBIOSMode</key><string>Custom</string>');

    const iMac = generateConfigPlist(profile({ generation: 'Coffee Lake', smbios: 'iMac19,1' }));
    expect(iMac).toContain('UpdateSMBIOSMode</key><string>Create</string>');
  });

  it('DummyPowerManagement only for AMD', () => {
    const amd = generateConfigPlist(profile({ architecture: 'AMD', generation: 'Ryzen', coreCount: 8 }));
    expect(amd).toContain('<key>DummyPowerManagement</key><true/>');

    const intel = generateConfigPlist(profile());
    expect(intel).toContain('<key>DummyPowerManagement</key><false/>');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 9. AUDIO PATH — 300-series boundary
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: Audio device path boundary', () => {
  const MODERN_GENS = ['Coffee Lake', 'Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake'] as const;
  const LEGACY_GENS = ['Skylake', 'Kaby Lake', 'Haswell', 'Broadwell', 'Sandy Bridge', 'Ivy Bridge'] as const;

  for (const gen of MODERN_GENS) {
    it(`${gen} uses Pci(0x1f,0x3) (300-series PCH)`, () => {
      const plist = generateConfigPlist(profile({
        generation: gen,
        smbios: gen === 'Coffee Lake' ? 'iMac19,1' : (gen === 'Comet Lake' ? 'iMac20,1' : 'MacPro7,1'),
      }));
      expect(plist).toContain('Pci(0x1f,0x3)');
    });
  }

  for (const gen of LEGACY_GENS) {
    it(`${gen} uses Pci(0x1b,0x0) (legacy HDA)`, () => {
      const smbiosMap: Record<string, string> = {
        'Skylake': 'iMac17,1', 'Kaby Lake': 'iMac18,1',
        'Haswell': 'iMac14,4', 'Broadwell': 'iMac16,2',
        'Sandy Bridge': 'iMac12,2', 'Ivy Bridge': 'iMac13,2',
      };
      const plist = generateConfigPlist(profile({
        generation: gen,
        smbios: smbiosMap[gen] ?? 'iMac20,1',
      }));
      expect(plist).toContain('Pci(0x1b,0x0)');
    });
  }
});

// ═══════════════════════════════════════════════════════════════════════════════
// 10. AMD PATCH HONESTY — no fake success
// ═══════════════════════════════════════════════════════════════════════════════

describe('Benchmark: AMD patch honesty', () => {
  it('AMD patches contain no disabled placeholder patches', () => {
    const patches = getAMDPatches(8);
    const disabled = patches.filter(p => !p.Enabled);
    expect(disabled).toHaveLength(0);
  });

  it('AMD_PATCH_COMPLETENESS flags missing core count patches', () => {
    expect(AMD_PATCH_COMPLETENESS.hasCoreCountPatches).toBe(false);
    expect(AMD_PATCH_COMPLETENESS.missingPatches.length).toBeGreaterThan(0);
  });

  it('All AMD patches have non-empty Find and Replace', () => {
    const patches = getAMDPatches(8);
    for (const p of patches) {
      expect(p.Find, p.Comment).not.toBe('');
      expect(p.Replace, p.Comment).not.toBe('');
    }
  });
});
