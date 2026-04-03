/**
 * Dortania Conformance Test Wall
 *
 * Every test in this file is a direct assertion against a specific Dortania
 * OpenCore Install Guide recommendation. If a test fails, the generator has
 * drifted from the canonical source of truth.
 *
 * Source: https://dortania.github.io/OpenCore-Install-Guide/
 */

import { describe, it, expect } from 'vitest';
import {
  generateConfigPlist,
  getQuirksForGeneration,
  getRequiredResources,
  getSMBIOSForProfile,
  getSIPPolicy,
  type HardwareProfile,
} from '../electron/configGenerator.js';

// ── Test helpers ─────────────────────────────────────────────────────────────

function profile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: overrides.cpu ?? 'Intel Core i7-9700K',
    architecture: overrides.architecture ?? 'Intel',
    generation: overrides.generation ?? 'Coffee Lake',
    gpu: overrides.gpu ?? 'Intel UHD 630',
    gpuDevices: overrides.gpuDevices ?? [{ name: 'Intel UHD 630' }],
    ram: overrides.ram ?? '16 GB',
    motherboard: overrides.motherboard ?? 'Generic Z390',
    targetOS: overrides.targetOS ?? 'macOS Ventura 13',
    smbios: overrides.smbios ?? '',
    kexts: overrides.kexts ?? [],
    ssdts: overrides.ssdts ?? [],
    bootArgs: overrides.bootArgs ?? '-v',
    isLaptop: overrides.isLaptop ?? false,
    isVM: overrides.isVM ?? false,
    audioLayoutId: 'audioLayoutId' in overrides ? overrides.audioLayoutId : 1,
    strategy: overrides.strategy ?? 'canonical',
    coreCount: overrides.coreCount,
    ...overrides,
  };
}

function withSmbios(p: HardwareProfile): HardwareProfile {
  return { ...p, smbios: p.smbios || getSMBIOSForProfile(p) };
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. SMBIOS CONFORMANCE — Source: Dortania per-gen config.plist pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: SMBIOS selection', () => {
  // Desktop Intel
  it('Sandy Bridge desktop (pre-Monterey) → iMac12,2', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Sandy Bridge', targetOS: 'macOS Big Sur 11',
    }))).toBe('iMac12,2');
  });

  it('Ivy Bridge desktop (pre-Monterey) → iMac13,2', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Ivy Bridge', targetOS: 'macOS Big Sur 11',
    }))).toBe('iMac13,2');
  });

  it('Haswell desktop (iGPU only) → iMac14,4', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Haswell', targetOS: 'macOS Big Sur 11',
    }))).toBe('iMac14,4');
  });

  it('Haswell desktop (dGPU) → iMac15,1', () => {
    // Put dGPU first so getBestSupportedGpuPath picks it as primary
    expect(getSMBIOSForProfile(profile({
      generation: 'Haswell', targetOS: 'macOS Big Sur 11',
      gpuDevices: [{ name: 'AMD Radeon RX 580' }, { name: 'Intel HD 4600' }],
    }))).toBe('iMac15,1');
  });

  it('Broadwell desktop → iMac16,2', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Broadwell', targetOS: 'macOS Monterey 12',
    }))).toBe('iMac16,2');
  });

  it('Skylake desktop → iMac17,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Skylake', targetOS: 'macOS Monterey 12',
    }))).toBe('iMac17,1');
  });

  it('Kaby Lake desktop (iGPU) → iMac18,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Kaby Lake', targetOS: 'macOS Ventura 13',
    }))).toBe('iMac18,1');
  });

  it('Kaby Lake desktop (dGPU) → iMac18,3', () => {
    // dGPU first so it's picked as primary display path
    expect(getSMBIOSForProfile(profile({
      generation: 'Kaby Lake', targetOS: 'macOS Ventura 13',
      gpuDevices: [{ name: 'AMD Radeon RX 580' }, { name: 'Intel HD 630' }],
    }))).toBe('iMac18,3');
  });

  it('Coffee Lake desktop → iMac19,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Coffee Lake', targetOS: 'macOS Ventura 13',
    }))).toBe('iMac19,1');
  });

  it('Comet Lake desktop (iGPU) → iMac20,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Comet Lake', targetOS: 'macOS Ventura 13',
    }))).toBe('iMac20,1');
  });

  // HEDT — Source: Dortania config-HEDT pages
  it('Ivy Bridge-E → MacPro6,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Ivy Bridge-E', targetOS: 'macOS Monterey 12',
    }))).toBe('MacPro6,1');
  });

  it('Haswell-E → iMacPro1,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Haswell-E', targetOS: 'macOS Monterey 12',
    }))).toBe('iMacPro1,1');
  });

  it('Broadwell-E → iMacPro1,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Broadwell-E', targetOS: 'macOS Monterey 12',
    }))).toBe('iMacPro1,1');
  });

  it('Cascade Lake-X → iMacPro1,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Cascade Lake-X', targetOS: 'macOS Ventura 13',
    }))).toBe('iMacPro1,1');
  });

  // AMD — Source: Dortania AMD/zen.html, AMD/fx.html
  it('Ryzen with Polaris dGPU → MacPro7,1', () => {
    expect(getSMBIOSForProfile(profile({
      architecture: 'AMD', generation: 'Ryzen', targetOS: 'macOS Ventura 13',
      gpuDevices: [{ name: 'AMD Radeon RX 580' }],
    }))).toBe('MacPro7,1');
  });

  it('Bulldozer → iMacPro1,1', () => {
    expect(getSMBIOSForProfile(profile({
      architecture: 'AMD', generation: 'Bulldozer', targetOS: 'macOS Monterey 12',
      gpuDevices: [{ name: 'AMD Radeon RX 580' }],
    }))).toBe('iMacPro1,1');
  });

  // Laptop — Source: Dortania laptop config.plist pages
  it('Ice Lake laptop → MacBookAir9,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Ice Lake', isLaptop: true, targetOS: 'macOS Ventura 13',
    }))).toBe('MacBookAir9,1');
  });

  it('Coffee Lake laptop → MacBookPro15,2', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Coffee Lake', isLaptop: true, targetOS: 'macOS Ventura 13',
    }))).toBe('MacBookPro15,2');
  });

  it('Kaby Lake laptop → MacBookPro14,1', () => {
    expect(getSMBIOSForProfile(profile({
      generation: 'Kaby Lake', isLaptop: true, targetOS: 'macOS Ventura 13',
    }))).toBe('MacBookPro14,1');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 2. ig-platform-id CONFORMANCE — Source: Dortania per-gen config.plist pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: ig-platform-id values', () => {
  // Desktop display IDs (no discrete GPU)
  const DESKTOP_DISPLAY: Record<string, string> = {
    'Sandy Bridge': 'EAADAA==',  // 0x00030010
    'Ivy Bridge':   'CgBmAQ==',  // 0x0166000A
    'Haswell':      'AwAiDQ==',  // 0x0D220003
    'Broadwell':    'BwAiFg==',  // 0x16220007
    'Skylake':      'AAASGQ==',  // 0x19120000
    'Kaby Lake':    'AAASWQ==',  // 0x59120000
    'Coffee Lake':  'BwCbPg==',  // 0x3E9B0007
    'Comet Lake':   'BwCbPg==',  // 0x3E9B0007
  };

  for (const [gen, expectedId] of Object.entries(DESKTOP_DISPLAY)) {
    it(`desktop display: ${gen} → ${expectedId}`, () => {
      const p = withSmbios(profile({ generation: gen as HardwareProfile['generation'] }));
      const plist = generateConfigPlist(p);
      expect(plist).toContain(expectedId);
    });
  }

  // Desktop headless IDs (discrete GPU present)
  const DESKTOP_HEADLESS: Record<string, string> = {
    'Sandy Bridge': 'AAAFAA==',  // 0x00050000
    'Ivy Bridge':   'BwBiAQ==',  // 0x01620007
    'Haswell':      'BAASBA==',  // 0x04120004
    'Skylake':      'AQASGQ==',  // 0x19120001
    'Kaby Lake':    'AwASWQ==',  // 0x59120003
    'Coffee Lake':  'AwCRPg==',  // 0x3E910003
    'Comet Lake':   'AwDImw==',  // 0x9BC80003
  };

  for (const [gen, expectedId] of Object.entries(DESKTOP_HEADLESS)) {
    it(`desktop headless: ${gen} → ${expectedId}`, () => {
      const p = withSmbios(profile({
        generation: gen as HardwareProfile['generation'],
        gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'AMD Radeon RX 580' }],
      }));
      const plist = generateConfigPlist(p);
      expect(plist).toContain(expectedId);
    });
  }

  // Laptop display IDs — Source: Dortania laptop config.plist pages
  const LAPTOP_DISPLAY: Record<string, string> = {
    'Sandy Bridge': 'AAABAA==',  // 0x00010000
    'Ivy Bridge':   'BABmAQ==',  // 0x01660004
    'Haswell':      'BgAmCg==',  // 0x0A260006
    'Broadwell':    'BgAmFg==',  // 0x16260006
    'Skylake':      'AAAWGQ==',  // 0x19160000
    'Kaby Lake':    'AAAbWQ==',  // 0x591B0000
    'Coffee Lake':  'CQClPg==',  // 0x3EA50009
    'Comet Lake':   'CQClPg==',  // 0x3EA50009
    'Ice Lake':     'AABSig==',  // 0x8A520000
  };

  for (const [gen, expectedId] of Object.entries(LAPTOP_DISPLAY)) {
    it(`laptop display: ${gen} → ${expectedId}`, () => {
      const smbiosMap: Record<string, string> = {
        'Sandy Bridge': 'MacBookPro8,1',
        'Ivy Bridge': 'MacBookPro10,1',
        'Haswell': 'MacBookPro11,4',
        'Broadwell': 'MacBookPro12,1',
        'Skylake': 'MacBookPro14,1',
        'Kaby Lake': 'MacBookPro14,1',
        'Coffee Lake': 'MacBookPro15,2',
        'Comet Lake': 'MacBookPro16,1',
        'Ice Lake': 'MacBookAir9,1',
      };
      const p = profile({
        generation: gen as HardwareProfile['generation'],
        isLaptop: true,
        smbios: smbiosMap[gen] ?? 'MacBookPro16,1',
      });
      const plist = generateConfigPlist(p);
      expect(plist).toContain(expectedId);
    });
  }

  // No iGPU properties for Rocket Lake / Alder Lake / Raptor Lake
  for (const gen of ['Rocket Lake', 'Alder Lake', 'Raptor Lake'] as const) {
    it(`${gen} has no iGPU properties (dGPU-only, MacPro7,1)`, () => {
      const p = withSmbios(profile({
        generation: gen,
        gpuDevices: [{ name: 'AMD Radeon RX 6800 XT' }],
      }));
      const plist = generateConfigPlist(p);
      expect(plist).not.toContain('AAPL,ig-platform-id');
    });
  }
});

// ═══════════════════════════════════════════════════════════════════════════════
// 3. SSDT CONFORMANCE — Source: Dortania per-gen config.plist + ACPI guide
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: SSDT selection', () => {
  // Sandy/Ivy Bridge: no XCPM → no SSDT-PLUG
  for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
    it(`${gen} desktop: SSDT-EC only (no XCPM)`, () => {
      const r = getRequiredResources(profile({ generation: gen }));
      expect(r.ssdts).toContain('SSDT-EC.aml');
      expect(r.ssdts).not.toContain('SSDT-PLUG.aml');
    });
  }

  // Haswell/Broadwell: SSDT-PLUG + SSDT-EC (no USBX)
  for (const gen of ['Haswell', 'Broadwell'] as const) {
    it(`${gen} desktop: SSDT-PLUG + SSDT-EC (pre-USBX era)`, () => {
      const r = getRequiredResources(profile({ generation: gen }));
      expect(r.ssdts).toContain('SSDT-PLUG.aml');
      expect(r.ssdts).toContain('SSDT-EC.aml');
      expect(r.ssdts).not.toContain('SSDT-EC-USBX.aml');
    });
  }

  // Skylake/Kaby Lake: SSDT-PLUG + SSDT-EC-USBX
  for (const gen of ['Skylake', 'Kaby Lake'] as const) {
    it(`${gen} desktop: SSDT-PLUG + SSDT-EC-USBX`, () => {
      const r = getRequiredResources(profile({ generation: gen }));
      expect(r.ssdts).toContain('SSDT-PLUG.aml');
      expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    });
  }

  // Coffee Lake: SSDT-PLUG + SSDT-EC-USBX + SSDT-AWAC
  it('Coffee Lake desktop: SSDT-PLUG + SSDT-EC-USBX + SSDT-AWAC', () => {
    const r = getRequiredResources(profile({ generation: 'Coffee Lake' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).toContain('SSDT-AWAC.aml');
  });

  // Coffee Lake Z390: + SSDT-PMC
  it('Coffee Lake Z390: includes SSDT-PMC for NVRAM', () => {
    const r = getRequiredResources(profile({ generation: 'Coffee Lake', motherboard: 'ASUS Prime Z390-A' }));
    expect(r.ssdts).toContain('SSDT-PMC.aml');
  });

  // Comet Lake: SSDT-PLUG + SSDT-EC-USBX + SSDT-AWAC
  it('Comet Lake desktop: SSDT-PLUG + SSDT-EC-USBX + SSDT-AWAC', () => {
    const r = getRequiredResources(profile({ generation: 'Comet Lake' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).toContain('SSDT-AWAC.aml');
  });

  // Ice Lake: SSDT-PLUG + SSDT-EC-USBX + SSDT-AWAC
  it('Ice Lake laptop: SSDT-PLUG + SSDT-EC-USBX-LAPTOP + SSDT-AWAC', () => {
    const r = getRequiredResources(profile({ generation: 'Ice Lake', isLaptop: true }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX-LAPTOP.aml');
    expect(r.ssdts).toContain('SSDT-AWAC.aml');
  });

  // HEDT — Source: Dortania config-HEDT pages
  it('Ivy Bridge-E: SSDT-EC + SSDT-UNC (no SSDT-PLUG)', () => {
    const r = getRequiredResources(profile({ generation: 'Ivy Bridge-E' }));
    expect(r.ssdts).toContain('SSDT-EC.aml');
    expect(r.ssdts).toContain('SSDT-UNC.aml');
    expect(r.ssdts).not.toContain('SSDT-PLUG.aml');
  });

  it('Haswell-E: SSDT-PLUG + SSDT-EC-USBX + SSDT-UNC + SSDT-RTC0-RANGE', () => {
    const r = getRequiredResources(profile({ generation: 'Haswell-E' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).toContain('SSDT-UNC.aml');
    expect(r.ssdts).toContain('SSDT-RTC0-RANGE.aml');
  });

  it('Cascade Lake-X: SSDT-PLUG + SSDT-EC-USBX + SSDT-UNC', () => {
    const r = getRequiredResources(profile({ generation: 'Cascade Lake-X' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).toContain('SSDT-UNC.aml');
  });

  // AMD — Source: Dortania AMD/zen.html
  it('AMD Ryzen desktop: SSDT-EC-USBX (laptop-aware via ecUsbxSsdt)', () => {
    const r = getRequiredResources(profile({ architecture: 'AMD', generation: 'Ryzen', coreCount: 8 }));
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
  });

  // Laptop SSDTs — Source: Dortania laptop config.plist pages
  it('Laptop always gets SSDT-PNLF (backlight)', () => {
    const r = getRequiredResources(profile({ isLaptop: true }));
    expect(r.ssdts).toContain('SSDT-PNLF.aml');
  });

  it('PS2 laptop path does NOT include SSDT-GPIO (I2C-only SSDT)', () => {
    const r = getRequiredResources(profile({ isLaptop: true, generation: 'Coffee Lake' }));
    expect(r.ssdts).not.toContain('SSDT-GPIO.aml');
  });

  it('Sandy/Ivy Bridge laptop gets SSDT-XOSI (pre-I2C)', () => {
    const r = getRequiredResources(profile({ isLaptop: true, generation: 'Sandy Bridge' }));
    expect(r.ssdts).toContain('SSDT-XOSI.aml');
  });

  it('Sandy/Ivy Bridge laptop gets SSDT-IMEI', () => {
    for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
      const r = getRequiredResources(profile({ generation: gen, isLaptop: true }));
      expect(r.ssdts, gen).toContain('SSDT-IMEI.aml');
    }
  });

  it('Coffee Lake laptop gets SSDT-PMC', () => {
    const r = getRequiredResources(profile({ generation: 'Coffee Lake', isLaptop: true }));
    expect(r.ssdts).toContain('SSDT-PMC.aml');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 4. KEXT CONFORMANCE — Source: Dortania ktext.html + per-gen pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: kext selection', () => {
  it('All builds include Lilu + VirtualSMC', () => {
    const r = getRequiredResources(profile());
    expect(r.kexts).toContain('Lilu.kext');
    expect(r.kexts).toContain('VirtualSMC.kext');
  });

  it('Intel desktop includes WhateverGreen + AppleALC', () => {
    const r = getRequiredResources(profile({ architecture: 'Intel' }));
    expect(r.kexts).toContain('WhateverGreen.kext');
    expect(r.kexts).toContain('AppleALC.kext');
  });

  it('Intel desktop includes SMCProcessor + SMCSuperIO', () => {
    const r = getRequiredResources(profile({ architecture: 'Intel', isLaptop: false }));
    expect(r.kexts).toContain('SMCProcessor.kext');
    expect(r.kexts).toContain('SMCSuperIO.kext');
  });

  it('Laptop does NOT include SMCProcessor/SMCSuperIO', () => {
    const r = getRequiredResources(profile({ isLaptop: true }));
    expect(r.kexts).not.toContain('SMCProcessor.kext');
    expect(r.kexts).not.toContain('SMCSuperIO.kext');
  });

  it('Laptop includes SMCBatteryManager + VoodooPS2Controller', () => {
    const r = getRequiredResources(profile({ isLaptop: true }));
    expect(r.kexts).toContain('SMCBatteryManager.kext');
    expect(r.kexts).toContain('VoodooPS2Controller.kext');
  });

  it('AMD Ryzen includes AMDRyzenCPUPowerManagement', () => {
    const r = getRequiredResources(profile({ architecture: 'AMD', generation: 'Ryzen', coreCount: 8 }));
    expect(r.kexts).toContain('AMDRyzenCPUPowerManagement.kext');
  });

  it('AMD Monterey+ includes AppleMCEReporterDisabler', () => {
    const r = getRequiredResources(profile({
      architecture: 'AMD', generation: 'Ryzen', targetOS: 'macOS Monterey 12', coreCount: 8,
    }));
    expect(r.kexts).toContain('AppleMCEReporterDisabler.kext');
  });

  it('Sonoma+ includes RestrictEvents', () => {
    const r = getRequiredResources(profile({ targetOS: 'macOS Sonoma 14' }));
    expect(r.kexts).toContain('RestrictEvents.kext');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 5. BOOTER / KERNEL / UEFI QUIRKS — Source: Dortania per-gen config.plist pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: Booter quirks by generation', () => {
  // Legacy (Sandy/Ivy Bridge): EnableWriteUnprotector=true, RebuildAppleMemoryMap=false
  for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
    it(`${gen}: EnableWriteUnprotector=true, RebuildAppleMemoryMap=false`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.EnableWriteUnprotector).toBe(true);
      expect(q.RebuildAppleMemoryMap).toBe(false);
      expect(q.SyncRuntimePermissions).toBe(false);
    });
  }

  // Haswell/Broadwell: same as legacy
  for (const gen of ['Haswell', 'Broadwell'] as const) {
    it(`${gen}: EnableWriteUnprotector=true, RebuildAppleMemoryMap=false`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.EnableWriteUnprotector).toBe(true);
      expect(q.RebuildAppleMemoryMap).toBe(false);
    });
  }

  // Coffee Lake: modern quirk set
  it('Coffee Lake: DevirtualiseMmio=true, RebuildAppleMemoryMap=true, SetupVirtualMap=true', () => {
    const q = getQuirksForGeneration('Coffee Lake');
    expect(q.DevirtualiseMmio).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(true);
    expect(q.SyncRuntimePermissions).toBe(true);
    expect(q.EnableWriteUnprotector).toBe(false);
    expect(q.SetupVirtualMap).toBe(true);
  });

  // Comet Lake: SetupVirtualMap=false, ProtectUefiServices=true
  it('Comet Lake: SetupVirtualMap=false, ProtectUefiServices=true', () => {
    const q = getQuirksForGeneration('Comet Lake');
    expect(q.SetupVirtualMap).toBe(false);
    expect(q.ProtectUefiServices).toBe(true);
    expect(q.DevirtualiseMmio).toBe(true);
  });

  // Ice Lake laptop: modern quirk set with ProtectMemoryRegions
  it('Ice Lake laptop: ProtectMemoryRegions=true, DevirtualiseMmio=true', () => {
    const q = getQuirksForGeneration('Ice Lake', '', false, 'canonical', '', true);
    expect(q.ProtectMemoryRegions).toBe(true);
    expect(q.DevirtualiseMmio).toBe(true);
    expect(q.ProtectUefiServices).toBe(true);
  });

  // Skylake laptop: ProtectMemoryRegions=true
  it('Skylake laptop: ProtectMemoryRegions=true', () => {
    const q = getQuirksForGeneration('Skylake', '', false, 'canonical', '', true);
    expect(q.ProtectMemoryRegions).toBe(true);
  });
});

describe('Dortania: Kernel quirks by generation', () => {
  // Sandy/Ivy Bridge: AppleCpuPmCfgLock (pre-XCPM)
  for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
    it(`${gen}: AppleCpuPmCfgLock=true (pre-XCPM)`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.AppleCpuPmCfgLock).toBe(true);
    });
  }

  // Haswell+: AppleXcpmCfgLock (XCPM era), AppleCpuPmCfgLock=false
  for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
    it(`${gen}: AppleXcpmCfgLock=true, AppleCpuPmCfgLock=false`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.AppleXcpmCfgLock).toBe(true);
      expect(q.AppleCpuPmCfgLock).toBe(false);
    });
  }

  // HEDT: AppleXcpmExtraMsrs
  for (const gen of ['Ivy Bridge-E', 'Haswell-E', 'Broadwell-E', 'Cascade Lake-X'] as const) {
    it(`${gen}: AppleXcpmExtraMsrs=true`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.AppleXcpmExtraMsrs).toBe(true);
    });
  }

  // Ivy Bridge-E: needs BOTH AppleCpuPmCfgLock AND AppleXcpmExtraMsrs
  it('Ivy Bridge-E: AppleCpuPmCfgLock=true (pre-XCPM HEDT)', () => {
    const q = getQuirksForGeneration('Ivy Bridge-E');
    expect(q.AppleCpuPmCfgLock).toBe(true);
    expect(q.AppleXcpmExtraMsrs).toBe(true);
  });

  // AMD: DummyPowerManagement, no AppleCpuPm/AppleXcpm quirks
  it('AMD Ryzen: DummyPowerManagement in plist, no CfgLock quirks', () => {
    const q = getQuirksForGeneration('Ryzen');
    expect(q.AppleCpuPmCfgLock).toBe(false);
    expect(q.AppleXcpmCfgLock).toBe(false);
    expect(q.ProvideCurrentCpuInfo).toBe(true);
  });

  // All Intel: PanicNoKextDump + PowerTimeoutKernelPanic always true
  for (const gen of ['Sandy Bridge', 'Haswell', 'Coffee Lake', 'Comet Lake'] as const) {
    it(`${gen}: PanicNoKextDump=true, PowerTimeoutKernelPanic=true`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.PanicNoKextDump).toBe(true);
      expect(q.PowerTimeoutKernelPanic).toBe(true);
    });
  }
});

describe('Dortania: UEFI quirks', () => {
  // IgnoreInvalidFlexRatio: Sandy→Broadwell + HEDT
  for (const gen of ['Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell'] as const) {
    it(`${gen}: IgnoreInvalidFlexRatio=true`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.IgnoreInvalidFlexRatio).toBe(true);
    });
  }

  // HEDT also needs it
  for (const gen of ['Ivy Bridge-E', 'Haswell-E', 'Broadwell-E'] as const) {
    it(`${gen}: IgnoreInvalidFlexRatio=true`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.IgnoreInvalidFlexRatio).toBe(true);
    });
  }

  // Skylake+ does NOT need IgnoreInvalidFlexRatio
  for (const gen of ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
    it(`${gen}: IgnoreInvalidFlexRatio=false`, () => {
      const q = getQuirksForGeneration(gen);
      expect(q.IgnoreInvalidFlexRatio).toBe(false);
    });
  }

  // HP boards: UnblockFsConnect
  it('HP board: UnblockFsConnect=true', () => {
    const q = getQuirksForGeneration('Coffee Lake', 'HP ProDesk 400');
    expect(q.UnblockFsConnect).toBe(true);
  });

  // All builds: RequestBootVarRouting + ReleaseUsbOwnership
  it('RequestBootVarRouting=true universally', () => {
    for (const gen of ['Sandy Bridge', 'Coffee Lake', 'Ryzen'] as const) {
      const q = getQuirksForGeneration(gen);
      expect(q.RequestBootVarRouting).toBe(true);
    }
  });
});

describe('Dortania: AMD Bulldozer vs Ryzen quirk split', () => {
  it('Bulldozer: EnableWriteUnprotector=true, RebuildAppleMemoryMap=false', () => {
    const q = getQuirksForGeneration('Bulldozer');
    expect(q.EnableWriteUnprotector).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(false);
  });

  it('Ryzen: EnableWriteUnprotector=false, RebuildAppleMemoryMap=true', () => {
    const q = getQuirksForGeneration('Ryzen');
    expect(q.EnableWriteUnprotector).toBe(false);
    expect(q.RebuildAppleMemoryMap).toBe(true);
  });

  it('Ryzen X570: SetupVirtualMap=false', () => {
    const q = getQuirksForGeneration('Ryzen', 'MSI MEG X570');
    expect(q.SetupVirtualMap).toBe(false);
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 6. SIP / NVRAM / PLATFORMINFO — Source: Dortania per-gen config.plist pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: SIP policy (csr-active-config)', () => {
  it('Standard build: SIP enabled (0x00000000)', () => {
    const sip = getSIPPolicy(profile(), [{ name: 'Intel UHD 630' }]);
    expect(sip.value).toBe('AAAAAA==');
  });

  it('OCLP-dependent GPU path: near-full SIP disable (0xFEF)', () => {
    // Kepler GPU on Ventura (past its native support ceiling of Big Sur)
    const sip = getSIPPolicy(
      profile({ targetOS: 'macOS Ventura 13' }),
      [{ name: 'NVIDIA GeForce GTX 780' }],
    );
    expect(sip.value).toBe('7w8AAA=='); // 0x00000FEF
  });
});

describe('Dortania: PlatformInfo policy', () => {
  it('MacPro7,1 uses UpdateSMBIOSMode: Custom', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      architecture: 'AMD', generation: 'Ryzen', coreCount: 8,
      gpuDevices: [{ name: 'AMD Radeon RX 580' }],
    })));
    expect(plist).toContain('<key>UpdateSMBIOSMode</key><string>Custom</string>');
  });

  it('Non-MacPro7,1 uses UpdateSMBIOSMode: Create', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      generation: 'Coffee Lake',
    })));
    expect(plist).toContain('<key>UpdateSMBIOSMode</key><string>Create</string>');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 7. UEFI DRIVERS — Source: Dortania ktext.html
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: UEFI drivers', () => {
  it('All builds include OpenHfsPlus.efi + OpenRuntime.efi', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toContain('OpenHfsPlus.efi');
    expect(plist).toContain('OpenRuntime.efi');
  });

  it('ConnectDrivers is true', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toContain('<key>ConnectDrivers</key><true/>');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 8. CPUID SPOOFING — Source: Dortania per-gen pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: CPUID spoofing', () => {
  it('Haswell-E uses Haswell desktop CPUID spoof', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      generation: 'Haswell-E',
    })));
    // Cpuid1Data: C3060300 + zeros → wwYDAAAAAAAAAAAAAAAAAA==
    expect(plist).toContain('wwYDAAAAAAAAAAAAAAAAAA==');
  });

  it('Rocket/Alder/Raptor Lake use Comet Lake CPUID spoof', () => {
    for (const gen of ['Rocket Lake', 'Alder Lake', 'Raptor Lake'] as const) {
      const plist = generateConfigPlist(withSmbios(profile({
        generation: gen,
        gpuDevices: [{ name: 'AMD Radeon RX 6800 XT' }],
      })));
      expect(plist).toContain('VQYKAAAAAAAAAAAAAAAAAA==');
    }
  });

  it('Coffee Lake has no CPUID spoof', () => {
    const plist = generateConfigPlist(withSmbios(profile({ generation: 'Coffee Lake' })));
    expect(plist).not.toContain('VQYKAAAAAAAAAAAAAAAAAA==');
    expect(plist).not.toContain('wwYDAAAAAAAAAAAAAAAAAA==');
  });

  it('AMD has DummyPowerManagement=true', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      architecture: 'AMD', generation: 'Ryzen', coreCount: 8,
      gpuDevices: [{ name: 'AMD Radeon RX 580' }],
    })));
    expect(plist).toContain('<key>DummyPowerManagement</key><true/>');
  });

  it('Intel has DummyPowerManagement=false', () => {
    const plist = generateConfigPlist(withSmbios(profile({ generation: 'Coffee Lake' })));
    expect(plist).toContain('<key>DummyPowerManagement</key><false/>');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 9. BOOT-ARGS — Source: Dortania per-gen config.plist pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: boot-args policy', () => {
  it('alcid is always present', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toMatch(/alcid=\d+/);
  });

  it('Navi GPU adds agdpmod=pikera', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'AMD Radeon RX 5700 XT' }],
    })));
    expect(plist).toContain('agdpmod=pikera');
  });

  it('Unsupported NVIDIA adds -wegnoegpu', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      gpuDevices: [{ name: 'Intel UHD 630' }, { name: 'NVIDIA GeForce RTX 3080' }],
    })));
    expect(plist).toContain('-wegnoegpu');
  });

  it('Comet Lake+ adds dk.e1000=0', () => {
    for (const gen of ['Comet Lake', 'Rocket Lake', 'Alder Lake'] as const) {
      const plist = generateConfigPlist(withSmbios(profile({
        generation: gen,
        gpuDevices: gen === 'Comet Lake' ? [{ name: 'Intel UHD 630' }] : [{ name: 'AMD Radeon RX 6800 XT' }],
      })));
      expect(plist).toContain('dk.e1000=0');
    }
  });

  it('Pre-Comet Lake does NOT have dk.e1000=0', () => {
    const plist = generateConfigPlist(withSmbios(profile({ generation: 'Coffee Lake' })));
    expect(plist).not.toContain('dk.e1000=0');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 10. MISC/SECURITY — Source: Dortania per-gen config.plist pages
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: Misc/Security defaults', () => {
  it('ScanPolicy=0, Vault=Optional', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toContain('<key>ScanPolicy</key><integer>0</integer>');
    expect(plist).toContain('<key>Vault</key><string>Optional</string>');
  });

  it('AllowSetDefault=true, BlacklistAppleUpdate=true', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toContain('<key>AllowSetDefault</key><true/>');
    expect(plist).toContain('<key>BlacklistAppleUpdate</key><true/>');
  });

  it('run-efi-updater=No in NVRAM', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toContain('<key>run-efi-updater</key><string>No</string>');
  });

  it('prev-lang:kbd=en-US:0', () => {
    const plist = generateConfigPlist(withSmbios(profile()));
    expect(plist).toContain('<key>prev-lang:kbd</key><string>en-US:0</string>');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 11. REAL-WORLD HARDWARE VALIDATION
// ═══════════════════════════════════════════════════════════════════════════════

describe('Dortania: Skylake desktop + RX 580 (i7-6700K, MSI H110M, RTL8111H)', () => {
  // Exact hardware: i7-6700K (Skylake), RX 580 8GB, MSI H110M PRO-VD, ALC887, RTL8111H
  const skylakeRx580 = (): HardwareProfile => withSmbios(profile({
    cpu: 'Intel Core i7-6700K',
    generation: 'Skylake',
    gpu: 'AMD Radeon RX 580 8GB / Intel HD Graphics 530',
    gpuDevices: [
      { name: 'AMD Radeon RX 580 8GB' },
      { name: 'Intel HD Graphics 530' },
    ],
    motherboard: 'MSI H110M PRO-VD',
    targetOS: 'macOS Monterey 12',
    audioLayoutId: 1,
  }));

  it('SMBIOS: iMac17,1 (Skylake desktop, Monterey)', () => {
    expect(skylakeRx580().smbios).toBe('iMac17,1');
  });

  it('ig-platform-id: headless 0x19120001 (dGPU drives display)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).toContain('AQASGQ=='); // 0x19120001 headless
    expect(plist).not.toContain('framebuffer-stolenmem'); // no FB patches in headless
  });

  it('SSDTs: SSDT-PLUG + SSDT-EC-USBX', () => {
    const res = getRequiredResources(skylakeRx580());
    expect(res.ssdts).toContain('SSDT-PLUG.aml');
    expect(res.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(res.ssdts).not.toContain('SSDT-AWAC.aml'); // no AWAC on 100-series
  });

  it('kexts: Lilu, VirtualSMC, WhateverGreen, AppleALC, SMC plugins, ethernet kexts', () => {
    const res = getRequiredResources(skylakeRx580());
    expect(res.kexts).toContain('Lilu.kext');
    expect(res.kexts).toContain('VirtualSMC.kext');
    expect(res.kexts).toContain('WhateverGreen.kext');
    expect(res.kexts).toContain('AppleALC.kext');
    expect(res.kexts).toContain('SMCProcessor.kext');
    expect(res.kexts).toContain('SMCSuperIO.kext');
    expect(res.kexts).toContain('IntelMausi.kext');
    expect(res.kexts).toContain('RealtekRTL8111.kext');
  });

  it('boot-args: alcid=1 + agdpmod=pikera (iMac + AMD dGPU)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).toMatch(/alcid=1/);
    expect(plist).toContain('agdpmod=pikera');
  });

  it('SIP: enabled (0x00000000)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).toContain('<key>csr-active-config</key><data>AAAAAA==</data>');
  });

  it('quirks: Skylake legacy booter profile', () => {
    const q = getQuirksForGeneration('Skylake', 'MSI H110M PRO-VD');
    expect(q.EnableWriteUnprotector).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(false);
    expect(q.SetupVirtualMap).toBe(true);
    expect(q.AppleXcpmCfgLock).toBe(true);
    expect(q.AppleCpuPmCfgLock).toBe(false);
    expect(q.DisableIoMapper).toBe(true);
    expect(q.ProtectUefiServices).toBe(false); // not Z390
  });

  it('no dk.e1000=0 (pre-Comet Lake)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).not.toContain('dk.e1000=0');
  });

  it('UpdateSMBIOSMode: Create (not MacPro7,1)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).toContain('<key>UpdateSMBIOSMode</key><string>Create</string>');
  });

  it('DummyPowerManagement: false (Intel)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).toContain('<key>DummyPowerManagement</key><false/>');
  });

  it('no CPUID spoofing (Skylake is natively supported)', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).not.toContain('VQYKAAAAAAAAAAAAAAAAAA==');
    expect(plist).not.toContain('wwYDAAAAAAAAAAAAAAAAAA==');
  });

  it('drivers: OpenHfsPlus + OpenRuntime + OpenCanopy', () => {
    const plist = generateConfigPlist(skylakeRx580());
    expect(plist).toContain('OpenHfsPlus.efi');
    expect(plist).toContain('OpenRuntime.efi');
    expect(plist).toContain('OpenCanopy.efi');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 12. GAP CLOSURE: SSDT-GPIO, SSDT-RHUB, ACPI Delete, device-id spoofs,
//     audio codec, Intel Wi-Fi, AMD core count patches
// ═══════════════════════════════════════════════════════════════════════════════

import { resolveAudioLayoutId } from '../electron/configGenerator.js';
import { getAMDPatches, AMD_PATCH_COMPLETENESS } from '../electron/amdPatches.js';

describe('Dortania: SSDT-GPIO only for I2C path (not PS2 conservative path)', () => {
  it('PS2 laptop path does NOT include SSDT-GPIO for any generation', () => {
    for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake', 'Ice Lake'] as const) {
      const r = getRequiredResources(profile({ generation: gen, isLaptop: true }));
      expect(r.ssdts, `${gen} PS2 laptop should not require SSDT-GPIO`).not.toContain('SSDT-GPIO.aml');
    }
  });

  it('Sandy/Ivy Bridge laptop does NOT include SSDT-GPIO', () => {
    for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
      const r = getRequiredResources(profile({ generation: gen, isLaptop: true }));
      expect(r.ssdts).not.toContain('SSDT-GPIO.aml');
      expect(r.ssdts).toContain('SSDT-XOSI.aml');
    }
  });

  it('desktop does NOT include SSDT-GPIO', () => {
    const r = getRequiredResources(profile({ generation: 'Coffee Lake', isLaptop: false }));
    expect(r.ssdts).not.toContain('SSDT-GPIO.aml');
  });
});

describe('Dortania Gap: SSDT-RHUB for Ice Lake laptop', () => {
  it('Ice Lake laptop includes SSDT-RHUB', () => {
    const r = getRequiredResources(profile({ generation: 'Ice Lake', isLaptop: true }));
    expect(r.ssdts).toContain('SSDT-RHUB.aml');
  });

  it('Coffee Lake laptop does NOT include SSDT-RHUB', () => {
    const r = getRequiredResources(profile({ generation: 'Coffee Lake', isLaptop: true }));
    expect(r.ssdts).not.toContain('SSDT-RHUB.aml');
  });
});

describe('Dortania Gap: Sandy Bridge ACPI Delete entries', () => {
  it('Sandy Bridge plist contains CpuPm and Cpu0Ist ACPI Delete entries', () => {
    const p = withSmbios(profile({ generation: 'Sandy Bridge', targetOS: 'macOS Big Sur 11' }));
    const plist = generateConfigPlist(p);
    expect(plist).toContain('Delete CpuPm');
    expect(plist).toContain('Delete Cpu0Ist');
    // OemTableId for CpuPm: Q3B1UG0AAAA= (hex 437075506d000000)
    expect(plist).toContain('Q3B1UG0AAAA=');
    // OemTableId for Cpu0Ist: Q3B1MElzdAA= (hex 4370753049737400)
    expect(plist).toContain('Q3B1MElzdAA=');
  });

  it('Ivy Bridge also has ACPI Delete entries', () => {
    const p = withSmbios(profile({ generation: 'Ivy Bridge', targetOS: 'macOS Big Sur 11' }));
    const plist = generateConfigPlist(p);
    expect(plist).toContain('Delete CpuPm');
  });

  it('Coffee Lake does NOT have ACPI Delete entries', () => {
    const plist = generateConfigPlist(withSmbios(profile({ generation: 'Coffee Lake' })));
    expect(plist).not.toContain('Delete CpuPm');
  });
});

describe('Dortania Gap: device-id spoofing', () => {
  it('Sandy Bridge display injects device-id 26010000', () => {
    const p = withSmbios(profile({ generation: 'Sandy Bridge', targetOS: 'macOS Big Sur 11' }));
    const plist = generateConfigPlist(p);
    expect(plist).toContain('JgEAAA=='); // 26010000
  });

  it('Sandy Bridge headless injects device-id 02010000', () => {
    const p = withSmbios(profile({
      generation: 'Sandy Bridge', targetOS: 'macOS Big Sur 11',
      gpuDevices: [{ name: 'AMD Radeon RX 580' }, { name: 'Intel HD 3000' }],
    }));
    const plist = generateConfigPlist(p);
    expect(plist).toContain('AgEAAA=='); // 02010000
  });

  it('Haswell display injects device-id 12040000 (HD 4400→4600)', () => {
    const p = withSmbios(profile({
      generation: 'Haswell', targetOS: 'macOS Big Sur 11',
    }));
    const plist = generateConfigPlist(p);
    expect(plist).toContain('EgQAAA=='); // 12040000
  });

  it('Haswell headless does NOT inject device-id (not needed)', () => {
    const p = withSmbios(profile({
      generation: 'Haswell', targetOS: 'macOS Big Sur 11',
      gpuDevices: [{ name: 'AMD Radeon RX 580' }, { name: 'Intel HD 4600' }],
    }));
    const plist = generateConfigPlist(p);
    expect(plist).not.toContain('EgQAAA==');
  });

  it('Coffee Lake laptop injects device-id 9B3E0000 (UHD 620)', () => {
    const p = profile({
      generation: 'Coffee Lake', isLaptop: true,
      smbios: 'MacBookPro15,2',
    });
    const plist = generateConfigPlist(p);
    expect(plist).toContain('mz4AAA=='); // 9B3E0000
  });
});

describe('Dortania Gap: audio codec → layout-id', () => {
  it('ALC887 resolves to layout-id 1', () => {
    expect(resolveAudioLayoutId('Realtek ALC887')).toBe(1);
  });

  it('ALC1220 resolves to layout-id 1', () => {
    expect(resolveAudioLayoutId('Realtek ALC1220')).toBe(1);
  });

  it('ALC256 resolves to layout-id 5', () => {
    expect(resolveAudioLayoutId('Realtek ALC256')).toBe(5);
  });

  it('ALC269 resolves to layout-id 1', () => {
    expect(resolveAudioLayoutId('Realtek ALC269')).toBe(1);
  });

  it('ALC295 resolves to layout-id 1', () => {
    expect(resolveAudioLayoutId('Realtek ALC295')).toBe(1);
  });

  it('unknown codec falls back to 1', () => {
    expect(resolveAudioLayoutId('Unknown Codec')).toBe(1);
    expect(resolveAudioLayoutId(undefined)).toBe(1);
  });

  it('codec-aware layout-id appears in generated plist', () => {
    // Must NOT set audioLayoutId so codec detection kicks in
    const p = withSmbios(profile({ audioCodec: 'Realtek ALC256', audioLayoutId: undefined }));
    const plist = generateConfigPlist(p);
    // layout-id 5 → base64 of [5, 0, 0, 0] = BQAAAA==
    expect(plist).toContain('BQAAAA==');
  });

  it('explicit audioLayoutId overrides codec detection', () => {
    const p = withSmbios(profile({ audioCodec: 'Realtek ALC256', audioLayoutId: 11 }));
    const plist = generateConfigPlist(p);
    // layout-id 11 → base64 of [11, 0, 0, 0] = CwAAAA==
    expect(plist).toContain('CwAAAA==');
  });
});

describe('Dortania Gap: Intel Wi-Fi kext for laptops', () => {
  for (const gen of ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake', 'Ice Lake'] as const) {
    it(`${gen} laptop includes itlwm.kext`, () => {
      const r = getRequiredResources(profile({
        generation: gen,
        isLaptop: true,
        wifiChipset: 'Intel Wi-Fi 6 AX200',
      }));
      expect(r.kexts).toContain('itlwm.kext');
    });
  }

  it('desktop does NOT include itlwm', () => {
    const r = getRequiredResources(profile({
      generation: 'Coffee Lake',
      isLaptop: false,
      wifiChipset: 'Intel Wi-Fi 6 AX200',
    }));
    expect(r.kexts).not.toContain('itlwm.kext');
  });

  it('Sandy Bridge laptop does NOT include itlwm', () => {
    const r = getRequiredResources(profile({
      generation: 'Sandy Bridge',
      isLaptop: true,
      wifiChipset: 'Intel Wi-Fi 6 AX200',
    }));
    expect(r.kexts).not.toContain('itlwm.kext');
  });

  it('Broadcom laptops do NOT inherit the Intel itlwm path', () => {
    const r = getRequiredResources(profile({
      generation: 'Haswell',
      isLaptop: true,
      wifiChipset: 'Broadcom BCM4352',
      targetOS: 'macOS Ventura 13',
    }));
    expect(r.kexts).not.toContain('itlwm.kext');
    expect(r.kexts).toContain('AirportBrcmFixup.kext');
  });
});

describe('Dortania Gap: AMD cpuid_cores_per_package patches', () => {
  it('AMD_PATCH_COMPLETENESS reports core count patches present', () => {
    expect(AMD_PATCH_COMPLETENESS.hasCoreCountPatches).toBe(true);
    expect(AMD_PATCH_COMPLETENESS.missingPatches).toEqual([]);
  });

  it('getAMDPatches includes 4 core count patches', () => {
    const patches = getAMDPatches(8);
    const corePatches = patches.filter(p => p.Comment.includes('cpuid_cores_per_package'));
    expect(corePatches.length).toBe(4);
  });

  it('core count patches cover all kernel ranges', () => {
    const patches = getAMDPatches(8);
    const corePatches = patches.filter(p => p.Comment.includes('cpuid_cores_per_package'));
    const ranges = corePatches.map(p => `${p.MinKernel}-${p.MaxKernel}`);
    expect(ranges).toContain('17.0.0-18.99.99');  // 10.13-10.14
    expect(ranges).toContain('19.0.0-21.99.99');  // 10.15-12
    expect(ranges).toContain('22.0.0-22.3.99');   // 13.0-13.2
    expect(ranges).toContain('22.4.0-');           // 13.3+
  });

  it('Replace bytes encode the correct core count', () => {
    const patches6 = getAMDPatches(6);
    const patches16 = getAMDPatches(16);
    // 6 cores: B8 06 → patch 1 Replace starts with B8 06
    const p6 = patches6.find(p => p.MinKernel === '17.0.0' && p.Comment.includes('cpuid_cores_per_package'))!;
    const p16 = patches16.find(p => p.MinKernel === '17.0.0' && p.Comment.includes('cpuid_cores_per_package'))!;
    // Decode base64 and check byte 1 (core count)
    const buf6 = Buffer.from(p6.Replace, 'base64');
    const buf16 = Buffer.from(p16.Replace, 'base64');
    expect(buf6[0]).toBe(0xB8);
    expect(buf6[1]).toBe(6);
    expect(buf16[1]).toBe(16);
  });

  it('AMD plist includes core count patches', () => {
    const plist = generateConfigPlist(withSmbios(profile({
      architecture: 'AMD', generation: 'Ryzen', coreCount: 8,
      gpuDevices: [{ name: 'AMD Radeon RX 580' }],
    })));
    expect(plist).toContain('cpuid_cores_per_package');
  });
});
