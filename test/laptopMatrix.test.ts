/**
 * Laptop Matrix Test Wall — OpCore-OneClick EFI Generator
 *
 * Systematic validation of laptop-specific EFI generation across:
 * - ThinkPad-class machines (Haswell → Comet Lake)
 * - Generic office laptops (Dell Latitude, HP EliteBook)
 * - Laptop ig-platform-ids (must differ from desktop)
 * - Laptop SSDTs (EC-USBX-LAPTOP, PNLF, XOSI, IMEI)
 * - Laptop kexts (battery, input, EC)
 * - Target OS honesty for old laptops
 */

import { describe, it, expect } from 'vitest';
import {
  getSMBIOSForProfile,
  getQuirksForGeneration,
  getRequiredResources,
  generateConfigPlist,
} from '../electron/configGenerator.js';
import type { HardwareProfile } from '../electron/configGenerator.js';

function laptop(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i5-8250U',
    architecture: 'Intel',
    generation: 'Coffee Lake',
    motherboard: 'Lenovo ThinkPad T480',
    gpu: 'Intel UHD 620',
    gpuDevices: [{ name: 'Intel UHD 620' }],
    ram: '8 GB',
    coreCount: 4,
    targetOS: 'macOS Ventura',
    smbios: 'MacBookPro15,2',
    kexts: [],
    ssdts: [],
    bootArgs: '',
    isLaptop: true,
    ...overrides,
  } as HardwareProfile;
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. THINKPAD GENERATION MATRIX — SMBIOS
// ═══════════════════════════════════════════════════════════════════════════════

describe('ThinkPad SMBIOS by generation', () => {
  const THINKPAD_SMBIOS: Array<{ model: string; gen: HardwareProfile['generation']; os: string; expected: string }> = [
    { model: 'T440',  gen: 'Haswell',     os: 'macOS Monterey 12', expected: 'MacBookPro11,4' },
    { model: 'T450',  gen: 'Broadwell',   os: 'macOS Monterey 12', expected: 'MacBookPro12,1' },
    { model: 'T460',  gen: 'Skylake',     os: 'macOS Ventura 13',  expected: 'MacBookPro14,1' },
    { model: 'T470',  gen: 'Kaby Lake',   os: 'macOS Ventura 13',  expected: 'MacBookPro14,1' },
    { model: 'T480',  gen: 'Coffee Lake', os: 'macOS Ventura 13',  expected: 'MacBookPro15,2' },
    { model: 'T490',  gen: 'Comet Lake',  os: 'macOS Ventura 13',  expected: 'MacBookPro16,1' },
    { model: 'X240',  gen: 'Haswell',     os: 'macOS Monterey 12', expected: 'MacBookPro11,4' },
    { model: 'X260',  gen: 'Skylake',     os: 'macOS Ventura 13',  expected: 'MacBookPro14,1' },
    { model: 'X280',  gen: 'Coffee Lake', os: 'macOS Ventura 13',  expected: 'MacBookPro15,2' },
  ];

  for (const { model, gen, os, expected } of THINKPAD_SMBIOS) {
    it(`ThinkPad ${model} (${gen}) on ${os} → ${expected}`, () => {
      expect(getSMBIOSForProfile(laptop({
        generation: gen,
        motherboard: `Lenovo ThinkPad ${model}`,
        targetOS: os,
      }))).toBe(expected);
    });
  }
});

// ═══════════════════════════════════════════════════════════════════════════════
// 2. LAPTOP SSDT POLICY
// ═══════════════════════════════════════════════════════════════════════════════

describe('Laptop SSDT policy', () => {
  it('all laptops get SSDT-PNLF (backlight)', () => {
    for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).toContain('SSDT-PNLF.aml');
    }
  });

  it('PS2 laptop path does NOT require SSDT-GPIO (GPIO is I2C-only)', () => {
    for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).not.toContain('SSDT-GPIO.aml');
    }
  });

  it('Sandy/Ivy Bridge laptops get SSDT-XOSI (pre-I2C era)', () => {
    for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).toContain('SSDT-XOSI.aml');
      expect(ssdts, gen).not.toContain('SSDT-GPIO.aml');
    }
  });

  it('Skylake+ laptops get SSDT-EC-USBX-LAPTOP (not desktop variant)', () => {
    for (const gen of ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).toContain('SSDT-EC-USBX-LAPTOP.aml');
      expect(ssdts, gen).not.toContain('SSDT-EC-USBX.aml');
    }
  });

  it('Haswell/Broadwell laptops get SSDT-EC-LAPTOP (no USBX, not desktop EC)', () => {
    for (const gen of ['Haswell', 'Broadwell'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).toContain('SSDT-EC-LAPTOP.aml');
      expect(ssdts, gen).not.toContain('SSDT-EC.aml');
      expect(ssdts, gen).not.toContain('SSDT-EC-USBX.aml');
    }
  });

  it('Sandy Bridge / Ivy Bridge laptops get SSDT-IMEI', () => {
    for (const gen of ['Sandy Bridge', 'Ivy Bridge'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).toContain('SSDT-IMEI.aml');
    }
  });

  it('Coffee Lake laptop gets SSDT-PMC (300-series mobile chipset)', () => {
    const { ssdts } = getRequiredResources(laptop({ generation: 'Coffee Lake' }));
    expect(ssdts).toContain('SSDT-PMC.aml');
  });

  it('Kaby Lake laptop does NOT get SSDT-PMC', () => {
    const { ssdts } = getRequiredResources(laptop({ generation: 'Kaby Lake' }));
    expect(ssdts).not.toContain('SSDT-PMC.aml');
  });

  it('laptops do NOT get SSDT-RHUB (desktop-only)', () => {
    for (const gen of ['Alder Lake', 'Raptor Lake'] as const) {
      const { ssdts } = getRequiredResources(laptop({ generation: gen }));
      expect(ssdts, gen).not.toContain('SSDT-RHUB.aml');
    }
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 3. LAPTOP KEXT POLICY
// ═══════════════════════════════════════════════════════════════════════════════

describe('Laptop kext policy', () => {
  it('all laptops get SMCBatteryManager', () => {
    for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
      const { kexts } = getRequiredResources(laptop({ generation: gen }));
      expect(kexts, gen).toContain('SMCBatteryManager.kext');
    }
  });

  it('all laptops get VoodooPS2Controller (safe input)', () => {
    for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
      const { kexts } = getRequiredResources(laptop({ generation: gen }));
      expect(kexts, gen).toContain('VoodooPS2Controller.kext');
    }
  });

  it('all laptops get ECEnabler', () => {
    for (const gen of ['Haswell', 'Broadwell', 'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'] as const) {
      const { kexts } = getRequiredResources(laptop({ generation: gen }));
      expect(kexts, gen).toContain('ECEnabler.kext');
    }
  });

  it('laptops do NOT get SMCProcessor (desktop-only)', () => {
    const { kexts } = getRequiredResources(laptop());
    expect(kexts).not.toContain('SMCProcessor.kext');
  });

  it('laptops do NOT get SMCSuperIO (desktop-only)', () => {
    const { kexts } = getRequiredResources(laptop());
    expect(kexts).not.toContain('SMCSuperIO.kext');
  });

  it('laptops do NOT get USBInjectAll (desktop-only)', () => {
    const { kexts } = getRequiredResources(laptop());
    expect(kexts).not.toContain('USBInjectAll.kext');
  });

  it('laptops do NOT get IntelMausi (desktop NIC)', () => {
    const { kexts } = getRequiredResources(laptop());
    expect(kexts).not.toContain('IntelMausi.kext');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 4. LAPTOP ig-platform-id — must differ from desktop
// ═══════════════════════════════════════════════════════════════════════════════

describe('Laptop ig-platform-id policy', () => {
  const LAPTOP_PLATFORM_IDS: Record<string, string> = {
    'Haswell':     'BgAmCg==', // 0x0A260006 — Dortania haswell laptop
    'Broadwell':   'BgAmFg==', // 0x16260006 — Dortania broadwell laptop
    'Skylake':     'AAAWGQ==', // 0x19160000 — Dortania skylake laptop
    'Kaby Lake':   'AAAbWQ==', // 0x591B0000 — Dortania kaby-lake laptop
    'Coffee Lake': 'CQClPg==', // 0x3EA50009 — Dortania coffee-lake laptop
    'Comet Lake':  'CQClPg==', // 0x3EA50009 — Dortania coffee-lake-plus laptop
  };

  for (const [gen, expectedId] of Object.entries(LAPTOP_PLATFORM_IDS)) {
    it(`${gen} laptop uses ${expectedId} (laptop-specific, not desktop)`, () => {
      const plist = generateConfigPlist(laptop({
        generation: gen as HardwareProfile['generation'],
        smbios: gen === 'Coffee Lake' ? 'MacBookPro15,2' : (gen === 'Comet Lake' ? 'MacBookPro16,1' : 'MacBookPro14,1'),
        gpuDevices: [{ name: 'Intel UHD 620' }],
      }));
      expect(plist).toContain(expectedId);
    });
  }

  it('laptop ig-platform-ids are different from desktop display IDs', () => {
    // Skylake desktop display: AAASGQ== (0x19120000) vs laptop: AAAWGQ== (0x19160000)
    const desktopPlist = generateConfigPlist({
      ...laptop({ generation: 'Skylake', smbios: 'iMac17,1' }),
      isLaptop: false,
    } as HardwareProfile);
    const laptopPlist = generateConfigPlist(laptop({
      generation: 'Skylake',
      smbios: 'MacBookPro14,1',
    }));
    expect(desktopPlist).toContain('AAASGQ==');
    expect(laptopPlist).toContain('AAAWGQ==');
    expect(laptopPlist).not.toContain('AAASGQ==');
  });

  it('laptop plist always has framebuffer patches (iGPU drives display)', () => {
    const plist = generateConfigPlist(laptop({ generation: 'Coffee Lake', smbios: 'MacBookPro15,2' }));
    expect(plist).toContain('framebuffer-patch-enable');
    expect(plist).toContain('framebuffer-stolenmem');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 5. OFFICE LAPTOP GENERATION MATRIX
// ═══════════════════════════════════════════════════════════════════════════════

describe('Office laptop generation matrix', () => {
  const OFFICE_LAPTOPS: Array<{
    name: string;
    gen: HardwareProfile['generation'];
    mb: string;
    os: string;
    checks: {
      smbios: string;
      hasPNLF: boolean;
      hasGPIO: boolean;
      hasBattery: boolean;
      hasPS2: boolean;
    };
  }> = [
    {
      name: 'Dell Latitude E5470 (Skylake)',
      gen: 'Skylake', mb: 'Dell Latitude E5470', os: 'macOS Ventura 13',
      checks: { smbios: 'MacBookPro14,1', hasPNLF: true, hasGPIO: false, hasBattery: true, hasPS2: true },
    },
    {
      name: 'HP EliteBook 840 G3 (Skylake)',
      gen: 'Skylake', mb: 'HP EliteBook 840 G3', os: 'macOS Ventura 13',
      checks: { smbios: 'MacBookPro14,1', hasPNLF: true, hasGPIO: false, hasBattery: true, hasPS2: true },
    },
    {
      name: 'Dell Latitude 7480 (Kaby Lake)',
      gen: 'Kaby Lake', mb: 'Dell Latitude 7480', os: 'macOS Ventura 13',
      checks: { smbios: 'MacBookPro14,1', hasPNLF: true, hasGPIO: false, hasBattery: true, hasPS2: true },
    },
    {
      name: 'HP EliteBook 840 G5 (Coffee Lake)',
      gen: 'Coffee Lake', mb: 'HP EliteBook 840 G5', os: 'macOS Ventura 13',
      checks: { smbios: 'MacBookPro15,2', hasPNLF: true, hasGPIO: false, hasBattery: true, hasPS2: true },
    },
    {
      name: 'Lenovo ThinkBook 14 (Comet Lake)',
      gen: 'Comet Lake', mb: 'Lenovo ThinkBook 14', os: 'macOS Ventura 13',
      checks: { smbios: 'MacBookPro16,1', hasPNLF: true, hasGPIO: false, hasBattery: true, hasPS2: true },
    },
    {
      name: 'Dell Latitude 5400 (Coffee Lake)',
      gen: 'Coffee Lake', mb: 'Dell Latitude 5400', os: 'macOS Sonoma 14',
      checks: { smbios: 'MacBookPro15,2', hasPNLF: true, hasGPIO: false, hasBattery: true, hasPS2: true },
    },
  ];

  for (const { name, gen, mb, os, checks } of OFFICE_LAPTOPS) {
    it(`${name}: correct SMBIOS, SSDTs, kexts`, () => {
      const p = laptop({ generation: gen, motherboard: mb, targetOS: os });
      expect(getSMBIOSForProfile(p)).toBe(checks.smbios);

      const { kexts, ssdts } = getRequiredResources(p);
      if (checks.hasPNLF) expect(ssdts).toContain('SSDT-PNLF.aml');
      expect(ssdts).not.toContain('SSDT-GPIO.aml');
      if (checks.hasBattery) expect(kexts).toContain('SMCBatteryManager.kext');
      if (checks.hasPS2) expect(kexts).toContain('VoodooPS2Controller.kext');

      // Must NOT have desktop-specific kexts
      expect(kexts).not.toContain('SMCProcessor.kext');
      expect(kexts).not.toContain('SMCSuperIO.kext');
      expect(kexts).not.toContain('USBInjectAll.kext');
      expect(kexts).not.toContain('IntelMausi.kext');
    });
  }
});

// ═══════════════════════════════════════════════════════════════════════════════
// 6. TARGET OS HONESTY FOR OLD LAPTOPS
// ═══════════════════════════════════════════════════════════════════════════════

describe('Target OS honesty for old laptops', () => {
  it('Haswell laptop on Tahoe throws (unsupported)', () => {
    expect(() => getSMBIOSForProfile(laptop({
      generation: 'Haswell',
      targetOS: 'macOS Tahoe 26',
    }))).toThrow(/not supported/i);
  });

  it('Broadwell laptop on Tahoe throws (unsupported)', () => {
    expect(() => getSMBIOSForProfile(laptop({
      generation: 'Broadwell',
      targetOS: 'macOS Tahoe 26',
    }))).toThrow(/not supported/i);
  });

  it('Skylake laptop on Tahoe gets MacBookPro16,1', () => {
    expect(getSMBIOSForProfile(laptop({
      generation: 'Skylake',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('MacBookPro16,1');
  });

  it('Coffee Lake laptop on Tahoe gets MacBookPro16,1', () => {
    expect(getSMBIOSForProfile(laptop({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('MacBookPro16,1');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 7. LAPTOP AUDIO PATH
// ═══════════════════════════════════════════════════════════════════════════════

describe('Laptop audio device path', () => {
  it('Coffee Lake+ laptop uses Pci(0x1f,0x3)', () => {
    const plist = generateConfigPlist(laptop({ generation: 'Coffee Lake', smbios: 'MacBookPro15,2' }));
    expect(plist).toContain('Pci(0x1f,0x3)');
  });

  it('Skylake laptop uses Pci(0x1b,0x0)', () => {
    const plist = generateConfigPlist(laptop({ generation: 'Skylake', smbios: 'MacBookPro14,1' }));
    expect(plist).toContain('Pci(0x1b,0x0)');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 8. LAPTOP QUIRK POLICY
// ═══════════════════════════════════════════════════════════════════════════════

describe('Laptop quirk policy', () => {
  it('HP laptop enables UnblockFsConnect', () => {
    const q = getQuirksForGeneration('Coffee Lake', 'HP EliteBook 840 G5');
    expect(q.UnblockFsConnect).toBe(true);
  });

  it('Lenovo laptop does NOT enable UnblockFsConnect', () => {
    const q = getQuirksForGeneration('Coffee Lake', 'Lenovo ThinkPad T480');
    expect(q.UnblockFsConnect).toBe(false);
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// 9. LAPTOP BOOT-ARGS
// ═══════════════════════════════════════════════════════════════════════════════

describe('Laptop boot-args', () => {
  it('Coffee Lake+ laptop gets -igfxblr (backlight fix)', () => {
    const plist = generateConfigPlist(laptop({ generation: 'Coffee Lake', smbios: 'MacBookPro15,2' }));
    expect(plist).toContain('-igfxblr');
  });

  it('Skylake laptop does NOT get -igfxblr', () => {
    const plist = generateConfigPlist(laptop({ generation: 'Skylake', smbios: 'MacBookPro14,1' }));
    expect(plist).not.toContain('-igfxblr');
  });

  it('laptop does NOT get dk.e1000=0 (desktop NIC fix)', () => {
    const plist = generateConfigPlist(laptop({ generation: 'Skylake', smbios: 'MacBookPro14,1' }));
    expect(plist).not.toContain('dk.e1000=0');
  });
});
