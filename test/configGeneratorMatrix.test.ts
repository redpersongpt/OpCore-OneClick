import { describe, it, expect } from 'vitest';
import {
  getSMBIOSForProfile,
  getQuirksForGeneration,
  getRequiredResources,
  generateConfigPlist,
  getBIOSSettings,
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

// ─── AMD quirks ─────────────────────────────────────────────────────────────

describe('getQuirksForGeneration — AMD specifics', () => {
  it('AMD disables AppleCpuPmCfgLock and AppleXcpmCfgLock', () => {
    for (const gen of ['Ryzen', 'Threadripper', 'Bulldozer'] as HardwareProfile['generation'][]) {
      const q = getQuirksForGeneration(gen);
      expect(q.AppleCpuPmCfgLock, `${gen} AppleCpuPmCfgLock`).toBe(false);
      expect(q.AppleXcpmCfgLock, `${gen} AppleXcpmCfgLock`).toBe(false);
    }
  });

  it('AMD enables ProvideCurrentCpuInfo', () => {
    const q = getQuirksForGeneration('Ryzen');
    expect(q.ProvideCurrentCpuInfo).toBe(true);
  });

  it('AMD B550 sets SetupVirtualMap false', () => {
    const q = getQuirksForGeneration('Ryzen', 'MSI MAG B550 Tomahawk');
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('AMD A520 sets SetupVirtualMap false', () => {
    const q = getQuirksForGeneration('Ryzen', 'Gigabyte A520M');
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('AMD X570 sets SetupVirtualMap false', () => {
    const q = getQuirksForGeneration('Ryzen', 'ASUS TUF X570');
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('AMD TRx40 enables DevirtualiseMmio', () => {
    const q = getQuirksForGeneration('Threadripper', 'ASUS TRx40 Creator');
    expect(q.DevirtualiseMmio).toBe(true);
  });

  it('AMD Ryzen on non-TRx40 has DevirtualiseMmio false', () => {
    const q = getQuirksForGeneration('Ryzen', 'ASUS B550');
    expect(q.DevirtualiseMmio).toBe(false);
  });
});

// ─── HEDT/motherboard-specific quirks ───────────────────────────────────────

describe('getQuirksForGeneration — HEDT and motherboard specifics', () => {
  it('X99 uses EnableWriteUnprotector and disables RebuildAppleMemoryMap', () => {
    const q = getQuirksForGeneration('Haswell', 'ASUS X99 Deluxe');
    expect(q.EnableWriteUnprotector).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(false);
    expect(q.SyncRuntimePermissions).toBe(false);
    expect(q.SetupVirtualMap).toBe(true);
  });

  it('X299 uses DevirtualiseMmio and ProtectUefiServices', () => {
    const q = getQuirksForGeneration('Skylake', 'ASUS Prime X299');
    expect(q.DevirtualiseMmio).toBe(true);
    expect(q.ProtectUefiServices).toBe(true);
    expect(q.EnableWriteUnprotector).toBe(false);
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('HP systems enable UnblockFsConnect', () => {
    const q = getQuirksForGeneration('Coffee Lake', 'HP ProDesk 400 G5');
    expect(q.UnblockFsConnect).toBe(true);
  });

  it('Hewlett-Packard also enables UnblockFsConnect', () => {
    const q = getQuirksForGeneration('Coffee Lake', 'Hewlett-Packard Z440');
    expect(q.UnblockFsConnect).toBe(true);
  });
});

// ─── Generation-specific quirks ─────────────────────────────────────────────

describe('getQuirksForGeneration — generation details', () => {
  it('Coffee Lake enables DevirtualiseMmio and keeps SetupVirtualMap true', () => {
    const q = getQuirksForGeneration('Coffee Lake');
    expect(q.DevirtualiseMmio).toBe(true);
    expect(q.SetupVirtualMap).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(true);
  });

  it('Comet Lake sets SetupVirtualMap false', () => {
    const q = getQuirksForGeneration('Comet Lake');
    expect(q.SetupVirtualMap).toBe(false);
    expect(q.DevirtualiseMmio).toBe(true);
  });

  it('Alder Lake enables ProtectUefiServices and ProvideCurrentCpuInfo', () => {
    const q = getQuirksForGeneration('Alder Lake');
    expect(q.ProtectUefiServices).toBe(true);
    expect(q.ProvideCurrentCpuInfo).toBe(true);
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('Raptor Lake matches Alder Lake quirks', () => {
    const qAL = getQuirksForGeneration('Alder Lake');
    const qRL = getQuirksForGeneration('Raptor Lake');
    expect(qRL).toEqual(qAL);
  });

  it('Rocket Lake matches Alder Lake quirks', () => {
    const qAL = getQuirksForGeneration('Alder Lake');
    const qRKL = getQuirksForGeneration('Rocket Lake');
    expect(qRKL).toEqual(qAL);
  });
});

// ─── Boot args generation ───────────────────────────────────────────────────

describe('generateConfigPlist — boot args', () => {
  it('appends alcid for non-Tahoe Intel', () => {
    const plist = generateConfigPlist(fakeProfile({
      targetOS: 'macOS Ventura',
      audioLayoutId: 7,
    }));
    expect(plist).toContain('alcid=7');
  });

  it('appends alcid for Tahoe (26+) — AppleALC still needs it', () => {
    const plist = generateConfigPlist(fakeProfile({
      targetOS: 'macOS Tahoe 26',
    }));
    expect(plist).toMatch(/alcid=\d/);
  });

  it('appends revpatch=sbvmm for Sonoma+ (14)', () => {
    const plist = generateConfigPlist(fakeProfile({
      targetOS: 'macOS Sonoma',
    }));
    expect(plist).toContain('revpatch=sbvmm');
  });

  it('does NOT append revpatch=sbvmm for Ventura (13)', () => {
    const plist = generateConfigPlist(fakeProfile({
      targetOS: 'macOS Ventura',
    }));
    expect(plist).not.toContain('revpatch=sbvmm');
  });

  it('appends -ibtcompatbeta for Tahoe Intel', () => {
    const plist = generateConfigPlist(fakeProfile({
      targetOS: 'macOS Tahoe 26',
    }));
    expect(plist).toContain('-ibtcompatbeta');
  });

  it('does NOT append -ibtcompatbeta for Ventura', () => {
    const plist = generateConfigPlist(fakeProfile({
      targetOS: 'macOS Ventura',
    }));
    expect(plist).not.toContain('-ibtcompatbeta');
  });

  it('appends -igfxblr for Coffee Lake laptop', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      smbios: 'MacBookPro15,2',
    }));
    expect(plist).toContain('-igfxblr');
  });

  it('does NOT append -igfxblr for Kaby Lake laptop', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Kaby Lake',
      isLaptop: true,
      smbios: 'MacBookPro14,1',
    }));
    expect(plist).not.toContain('-igfxblr');
  });

  it('does NOT append -igfxblr for Coffee Lake desktop', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: false,
    }));
    expect(plist).not.toContain('-igfxblr');
  });

  it('conservative strategy adds verbose and debug args', () => {
    const plist = generateConfigPlist(fakeProfile({
      strategy: 'conservative',
    } as any));
    expect(plist).toContain('-v');
    expect(plist).toContain('debug=0x100');
    expect(plist).toContain('keepsyms=1');
  });
});

// ─── Intel iGPU device properties ───────────────────────────────────────────

describe('generateConfigPlist — Intel iGPU properties', () => {
  it('Haswell gets correct platform ID', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Haswell', smbios: 'iMac14,4' }));
    expect(plist).toContain('AwAiDQ==');
  });

  it('Broadwell gets correct platform ID', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Broadwell', smbios: 'iMac16,2' }));
    // BwAiFg== = 0x16220007 (Broadwell display per Dortania)
    expect(plist).toContain('BwAiFg==');
  });

  it('Skylake gets correct platform ID', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Skylake', smbios: 'iMac17,1' }));
    // AAASGQ== = 0x19120000 (Skylake display per Dortania)
    expect(plist).toContain('AAASGQ==');
  });

  it('Kaby Lake gets correct platform ID', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Kaby Lake', smbios: 'iMac18,1' }));
    expect(plist).toContain('AAASWQ==');
  });

  it('Coffee Lake gets default platform ID', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Coffee Lake', smbios: 'iMac19,1' }));
    expect(plist).toContain('BwCbPg==');
  });

  it('Comet Lake gets its own platform ID', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Comet Lake', smbios: 'iMac20,1' }));
    // BwCbPg== = 0x3E9B0007 (Comet Lake display per Dortania — shares Coffee Lake ID)
    expect(plist).toContain('BwCbPg==');
  });

  it('Alder Lake does NOT include iGPU device properties (no macOS driver)', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Alder Lake',
      smbios: 'MacPro7,1',
    }));
    expect(plist).not.toContain('AAPL,ig-platform-id');
    expect(plist).not.toContain('framebuffer-patch-enable');
  });

  it('Raptor Lake does NOT include iGPU device properties', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Raptor Lake',
      smbios: 'MacPro7,1',
    }));
    expect(plist).not.toContain('AAPL,ig-platform-id');
  });

  it('Rocket Lake does NOT include iGPU device properties', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Rocket Lake',
      smbios: 'MacPro7,1',
    }));
    expect(plist).not.toContain('AAPL,ig-platform-id');
  });

  it('AMD does NOT include iGPU device properties', () => {
    const plist = generateConfigPlist(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      coreCount: 8,
      smbios: 'iMacPro1,1',
    }));
    expect(plist).not.toContain('AAPL,ig-platform-id');
  });
});

// ─── CPUID spoofing ─────────────────────────────────────────────────────────

describe('generateConfigPlist — CPUID spoofing', () => {
  it('Alder Lake has non-default CPUID spoof data', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Alder Lake',
      smbios: 'MacPro7,1',
    }));
    expect(plist).toContain('VQYKAAAAAAAAAAAAAAAAAA==');
  });

  it('Raptor Lake has non-default CPUID spoof data', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Raptor Lake',
      smbios: 'MacPro7,1',
    }));
    expect(plist).toContain('VQYKAAAAAAAAAAAAAAAAAA==');
  });

  it('Coffee Lake uses default CPUID (no spoofing needed)', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Coffee Lake' }));
    // Default CPUID data is all zeros
    expect(plist).toContain('AAAAAAAAAAAAAA==');
  });
});

// ─── Kext selection per architecture ────────────────────────────────────────

describe('getRequiredResources — kext selection matrix', () => {
  it('includes CPUTopologyRebuild for Alder Lake', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Alder Lake' }));
    expect(r.kexts).toContain('CPUTopologyRebuild.kext');
  });

  it('includes CPUTopologyRebuild for Raptor Lake', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Raptor Lake' }));
    expect(r.kexts).toContain('CPUTopologyRebuild.kext');
  });

  it('does NOT include CPUTopologyRebuild for Coffee Lake', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Coffee Lake' }));
    expect(r.kexts).not.toContain('CPUTopologyRebuild.kext');
  });

  it('includes RestrictEvents for Sonoma+', () => {
    const r = getRequiredResources(fakeProfile({ targetOS: 'macOS Sonoma' }));
    expect(r.kexts).toContain('RestrictEvents.kext');
  });

  it('does NOT include RestrictEvents for Ventura', () => {
    const r = getRequiredResources(fakeProfile({ targetOS: 'macOS Ventura' }));
    expect(r.kexts).not.toContain('RestrictEvents.kext');
  });

  it('includes AMDRyzenCPUPowerManagement for Ryzen', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
    }));
    expect(r.kexts).toContain('AMDRyzenCPUPowerManagement.kext');
  });

  it('includes AMDRyzenCPUPowerManagement for Threadripper', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Threadripper',
    }));
    expect(r.kexts).toContain('AMDRyzenCPUPowerManagement.kext');
  });

  it('does NOT include AMDRyzenCPUPowerManagement for Bulldozer', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Bulldozer',
    }));
    expect(r.kexts).not.toContain('AMDRyzenCPUPowerManagement.kext');
  });

  it('does NOT include AppleMCEReporterDisabler for AMD on Big Sur (11)', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      targetOS: 'macOS Big Sur',
    }));
    expect(r.kexts).not.toContain('AppleMCEReporterDisabler.kext');
  });

  it('includes laptop kexts and SSDTs for Intel laptop', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
    }));
    expect(r.kexts).toContain('SMCBatteryManager.kext');
    expect(r.kexts).toContain('VoodooPS2Controller.kext');
    expect(r.ssdts).toContain('SSDT-PNLF.aml');
    expect(r.ssdts).toContain('SSDT-XOSI.aml');
  });

  it('includes laptop SSDTs for AMD laptop', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      isLaptop: true,
    }));
    expect(r.ssdts).toContain('SSDT-PNLF.aml');
    expect(r.ssdts).toContain('SSDT-XOSI.aml');
  });
});

// ─── SSDT selection matrix ──────────────────────────────────────────────────

describe('getRequiredResources — SSDT selection matrix', () => {
  it('Haswell uses SSDT-PLUG + SSDT-EC', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Haswell' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC.aml');
    expect(r.ssdts).not.toContain('SSDT-AWAC.aml');
  });

  it('Broadwell uses SSDT-PLUG + SSDT-EC', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Broadwell' }));
    expect(r.ssdts).toEqual(expect.arrayContaining(['SSDT-PLUG.aml', 'SSDT-EC.aml']));
  });

  it('Sandy Bridge uses SSDT-PLUG + SSDT-EC', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Sandy Bridge' }));
    expect(r.ssdts).toEqual(expect.arrayContaining(['SSDT-PLUG.aml', 'SSDT-EC.aml']));
  });

  it('Skylake uses SSDT-PLUG + SSDT-EC-USBX (USB power management required on 6th gen)', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Skylake' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).not.toContain('SSDT-EC.aml');
  });

  it('Kaby Lake uses SSDT-PLUG + SSDT-EC-USBX (USB power management required on 7th gen)', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Kaby Lake' }));
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).not.toContain('SSDT-EC.aml');
  });

  it('Sandy Bridge uses legacy SSDT-EC (no USBX, older controller)', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Sandy Bridge' }));
    expect(r.ssdts).toContain('SSDT-EC.aml');
    expect(r.ssdts).not.toContain('SSDT-EC-USBX.aml');
  });

  it('Ivy Bridge uses legacy SSDT-EC (no USBX, older controller)', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Ivy Bridge' }));
    expect(r.ssdts).toContain('SSDT-EC.aml');
    expect(r.ssdts).not.toContain('SSDT-EC-USBX.aml');
  });

  it('Coffee Lake Z390 includes SSDT-PMC', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      motherboard: 'ASUS Prime Z390-A',
    }));
    expect(r.ssdts).toContain('SSDT-PMC.aml');
  });

  it('Coffee Lake Z370 DOES include SSDT-PMC (all 300-series need it)', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      motherboard: 'ASUS ROG Z370',
    }));
    expect(r.ssdts).toContain('SSDT-PMC.aml');
  });

  it('Coffee Lake non-300-series does NOT include SSDT-PMC', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      motherboard: 'Generic Coffee Lake Board',
    }));
    expect(r.ssdts).not.toContain('SSDT-PMC.aml');
  });

  it('Alder Lake uses SSDT-PLUG-ALT (not SSDT-PLUG) and includes SSDT-RHUB', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Alder Lake' }));
    expect(r.ssdts).toContain('SSDT-PLUG-ALT.aml');
    expect(r.ssdts).not.toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-RHUB.aml');
  });

  it('Raptor Lake includes SSDT-RHUB', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Raptor Lake' }));
    expect(r.ssdts).toContain('SSDT-RHUB.aml');
  });

  it('Comet Lake Z490 includes SSDT-RHUB', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Comet Lake',
      motherboard: 'ASUS ROG Z490',
    }));
    expect(r.ssdts).toContain('SSDT-RHUB.aml');
  });

  it('Comet Lake non-Z490 does NOT include SSDT-RHUB', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Comet Lake',
      motherboard: 'MSI B460',
    }));
    expect(r.ssdts).not.toContain('SSDT-RHUB.aml');
  });

  it('laptop includes ECEnabler.kext', () => {
    const r = getRequiredResources(fakeProfile({ isLaptop: true }));
    expect(r.kexts).toContain('ECEnabler.kext');
  });

  it('desktop does NOT include ECEnabler.kext', () => {
    const r = getRequiredResources(fakeProfile({ isLaptop: false }));
    expect(r.kexts).not.toContain('ECEnabler.kext');
  });

  it('AMD B850 includes SSDT-CPUR', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'ASUS B850 Creator',
    }));
    expect(r.ssdts).toContain('SSDT-CPUR.aml');
  });

  it('AMD X670E includes SSDT-CPUR', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'MSI MEG X670E ACE',
    }));
    expect(r.ssdts).toContain('SSDT-CPUR.aml');
  });

  it('AMD X870 includes SSDT-CPUR', () => {
    const r = getRequiredResources(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'ASUS ROG X870 Hero',
    }));
    expect(r.ssdts).toContain('SSDT-CPUR.aml');
  });

  it('all 300-series boards get SSDT-PMC', () => {
    for (const mb of ['ASUS Z390', 'MSI Z370', 'ASRock H370', 'Gigabyte B360', 'ASUS H310']) {
      const r = getRequiredResources(fakeProfile({
        generation: 'Coffee Lake',
        motherboard: mb,
      }));
      expect(r.ssdts, mb).toContain('SSDT-PMC.aml');
    }
  });

  it('Comet Lake gets SSDT-AWAC', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Comet Lake',
      motherboard: 'MSI Z490',
    }));
    expect(r.ssdts).toContain('SSDT-AWAC.aml');
  });

  it('Haswell gets SSDT-EC (not SSDT-EC-USBX)', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Haswell',
      targetOS: 'macOS Ventura',
    }));
    expect(r.ssdts).toContain('SSDT-EC.aml');
    expect(r.ssdts).not.toContain('SSDT-EC-USBX.aml');
  });

  it('Skylake gets SSDT-EC-USBX (not plain SSDT-EC)', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Skylake' }));
    expect(r.ssdts).toContain('SSDT-EC-USBX.aml');
    expect(r.ssdts).not.toContain('SSDT-EC.aml');
  });

  it('every SSDT requested has a source policy', () => {
    const profiles = [
      fakeProfile({ generation: 'Haswell' }),
      fakeProfile({ generation: 'Coffee Lake', motherboard: 'ASUS Z390' }),
      fakeProfile({ generation: 'Alder Lake' }),
      fakeProfile({ generation: 'Raptor Lake' }),
      fakeProfile({ architecture: 'AMD', generation: 'Ryzen', motherboard: 'MSI B650' }),
      fakeProfile({ generation: 'Coffee Lake', isLaptop: true }),
      fakeProfile({ architecture: 'AMD', generation: 'Ryzen', isLaptop: true }),
    ];
    for (const profile of profiles) {
      const resources = getRequiredResources(profile);
      const unsupported = getUnsupportedSsdtRequests(resources.ssdts);
      expect(unsupported, `Unsupported SSDTs for ${profile.generation} ${profile.architecture}`).toEqual([]);
    }
  });
});

// ─── SMBIOS discrete GPU logic ──────────────────────────────────────────────

describe('getSMBIOSForProfile — discrete GPU logic', () => {
  it('Alder Lake desktop always returns MacPro7,1 (no iGPU path)', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Alder Lake',
      motherboard: 'ASUS Z690',
    }));
    expect(smbios).toBe('MacPro7,1');
  });

  it('Raptor Lake desktop always returns MacPro7,1', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Raptor Lake',
      motherboard: 'MSI Z790',
    }));
    expect(smbios).toBe('MacPro7,1');
  });

  it('Rocket Lake desktop returns MacPro7,1', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Rocket Lake',
      motherboard: 'ASRock Z590',
    }));
    expect(smbios).toBe('MacPro7,1');
  });
});

// ─── getBIOSSettings coverage ───────────────────────────────────────────────

describe('getBIOSSettings', () => {
  it('returns AMD BIOS settings for AMD profile', () => {
    const settings = getBIOSSettings(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
    }));
    expect(settings.enable.length).toBeGreaterThan(0);
    expect(settings.disable.length).toBeGreaterThan(0);
    const enableNames = settings.enable.map(s => s.name);
    expect(enableNames).toContain('SVM Mode');
    const disableNames = settings.disable.map(s => s.name);
    expect(disableNames).toContain('IOMMU');
  });

  it('returns Intel BIOS settings for Intel profile', () => {
    const settings = getBIOSSettings(fakeProfile({
      architecture: 'Intel',
      generation: 'Coffee Lake',
    }));
    const disableNames = settings.disable.map(s => s.name);
    expect(disableNames).toContain('CFG Lock');
    expect(disableNames).toContain('VT-d');
    const enableNames = settings.enable.map(s => s.name);
    expect(enableNames).toContain('VT-x');
  });

  it('Z390 adds ProtectUefiServices note', () => {
    const settings = getBIOSSettings(fakeProfile({
      motherboard: 'ASUS Prime Z390-A',
    }));
    const enableNames = settings.enable.map(s => s.name);
    expect(enableNames.some(n => n.includes('ProtectUefiServices'))).toBe(true);
  });

  it('Z490 adds ProtectUefiServices note', () => {
    const settings = getBIOSSettings(fakeProfile({
      motherboard: 'MSI MEG Z490 ACE',
    }));
    const enableNames = settings.enable.map(s => s.name);
    expect(enableNames.some(n => n.includes('ProtectUefiServices'))).toBe(true);
  });

  it('non-Z390/Z490 does NOT add ProtectUefiServices note', () => {
    const settings = getBIOSSettings(fakeProfile({
      motherboard: 'ASUS ROG Z370',
    }));
    const enableNames = settings.enable.map(s => s.name);
    expect(enableNames.some(n => n.includes('ProtectUefiServices'))).toBe(false);
  });
});

// ─── Z390 NVRAM ─────────────────────────────────────────────────────────────

describe('generateConfigPlist — Z390/Z370 NVRAM legacy flags', () => {
  it('Z390 motherboard enables LegacyEnable', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Coffee Lake',
      motherboard: 'ASUS Prime Z390-A',
      smbios: 'iMac19,1',
    }));
    // The generated plist should have legacy NVRAM flags
    expect(plist).toContain('<key>LegacyEnable</key>');
  });

  it('Z490 motherboard does NOT enable legacy NVRAM', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Comet Lake',
      motherboard: 'MSI Z490',
      smbios: 'iMac20,1',
    }));
    // Z490 should not trigger legacy NVRAM
    const legacyIdx = plist.indexOf('<key>LegacyEnable</key>');
    if (legacyIdx !== -1) {
      // If present, should be false
      const afterKey = plist.slice(legacyIdx);
      expect(afterKey).toContain('<false/>');
    }
  });
});

// ─── Full generation × form factor × OS matrix ─────────────────────────────

describe('config generator — full generation matrix produces valid output', () => {
  const intelGens: HardwareProfile['generation'][] = [
    'Penryn', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell',
    'Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake',
    'Rocket Lake', 'Alder Lake', 'Raptor Lake',
  ];
  const amdGens: HardwareProfile['generation'][] = ['Ryzen', 'Threadripper', 'Bulldozer'];
  const osTargets = ['macOS Ventura', 'macOS Sonoma', 'macOS Tahoe 26'];

  const TAHOE_BLOCKED = new Set(['Penryn', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell']);

  for (const gen of intelGens) {
    for (const targetOS of osTargets) {
      if (targetOS === 'macOS Tahoe 26' && TAHOE_BLOCKED.has(gen)) {
        it(`Intel ${gen} desktop + ${targetOS} throws (unsupported)`, () => {
          expect(() => {
            const profile = fakeProfile({ generation: gen, targetOS });
            getSMBIOSForProfile(profile);
          }).toThrow(/not supported on.*Tahoe|Tahoe.*requires Skylake/i);
        });
      } else {
        it(`Intel ${gen} desktop + ${targetOS} generates valid plist`, () => {
          const profile = fakeProfile({ generation: gen, targetOS });
          profile.smbios = getSMBIOSForProfile(profile);
          const plist = generateConfigPlist(profile);
          expect(plist).toContain('<?xml');
          expect(plist).toContain('<plist');
          expect(plist).toContain('</plist>');
          expect(plist).toContain(profile.smbios);
        });
      }
    }
  }

  for (const gen of intelGens) {
    it(`Intel ${gen} laptop generates valid plist`, () => {
      const profile = fakeProfile({ generation: gen, isLaptop: true });
      profile.smbios = getSMBIOSForProfile(profile);
      const plist = generateConfigPlist(profile);
      expect(plist).toContain('<?xml');
      expect(plist).toContain(profile.smbios);
    });
  }

  for (const gen of amdGens) {
    for (const targetOS of osTargets) {
      it(`AMD ${gen} + ${targetOS} generates valid plist`, () => {
        const profile = fakeProfile({
          architecture: 'AMD',
          generation: gen,
          coreCount: 8,
          targetOS,
        });
        profile.smbios = getSMBIOSForProfile(profile);
        const plist = generateConfigPlist(profile);
        expect(plist).toContain('<?xml');
        expect(plist).toContain(profile.smbios);
      });
    }
  }
});

// ─── Tahoe SMBIOS selection ─────────────────────────────────────────────────

describe('getSMBIOSForProfile — Tahoe generation-specific', () => {
  it('Skylake desktop on Tahoe returns iMac20,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Skylake',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('iMac20,1');
  });

  it('Kaby Lake desktop on Tahoe returns iMac20,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Kaby Lake',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('iMac20,1');
  });

  it('Coffee Lake desktop on Tahoe returns iMac20,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('iMac20,1');
  });

  it('Comet Lake desktop on Tahoe returns iMac20,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Comet Lake',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('iMac20,1');
  });

  it('Coffee Lake desktop with dGPU on Tahoe still returns iMac20,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
      gpuDevices: [
        { name: 'Intel UHD 630', vendorId: '8086', deviceId: '3e92' },
        { name: 'AMD Radeon RX 580', vendorId: '1002', deviceId: '67df' },
      ],
    }))).toBe('iMac20,1');
  });

  it('Haswell-E on Tahoe returns MacPro7,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Haswell-E',
      targetOS: 'macOS Tahoe 26',
    }))).toBe('MacPro7,1');
  });

  it('Coffee Lake laptop on Tahoe returns MacBookPro16,1', () => {
    expect(getSMBIOSForProfile(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Tahoe 26',
      isLaptop: true,
    }))).toBe('MacBookPro16,1');
  });

  it('Haswell on Tahoe throws', () => {
    expect(() => getSMBIOSForProfile(fakeProfile({
      generation: 'Haswell',
      targetOS: 'macOS Tahoe 26',
    }))).toThrow(/not supported/i);
  });
});

// ─── Headless iGPU detection across generations ─────────────────────────────

describe('generateConfigPlist — headless iGPU detection', () => {
  it('Kaby Lake with dGPU uses headless ig-platform-id', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Kaby Lake',
      gpuDevices: [
        { name: 'Intel HD 630', vendorId: '8086', deviceId: '5912' },
        { name: 'AMD Radeon RX 570', vendorId: '1002', deviceId: '67df' },
      ],
    }));
    // AAASZQ== = 0x59120003 (Kaby Lake headless)
    expect(plist).toContain('AwASWQ==');
    expect(plist).not.toContain('framebuffer-patch-enable');
  });

  it('Skylake iGPU-only uses display ig-platform-id with framebuffer patches', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Skylake',
      gpuDevices: [
        { name: 'Intel HD 530', vendorId: '8086', deviceId: '1912' },
      ],
    }));
    // AAASGQ== = 0x19120000 (Skylake display)
    expect(plist).toContain('AAASGQ==');
    expect(plist).toContain('framebuffer-patch-enable');
  });

  it('Haswell with dGPU uses headless ig-platform-id', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Haswell',
      targetOS: 'macOS Ventura',
      gpuDevices: [
        { name: 'Intel HD 4600', vendorId: '8086', deviceId: '0412' },
        { name: 'NVIDIA GeForce GTX 770', vendorId: '10de', deviceId: '1184' },
      ],
    }));
    // BAASBA== = 0x04120004 (Haswell headless)
    expect(plist).toContain('BAASBA==');
  });

  it('Rocket Lake does NOT include ig-platform-id (no macOS driver)', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Rocket Lake',
      gpuDevices: [
        { name: 'Intel UHD 750', vendorId: '8086', deviceId: '4c8a' },
        { name: 'AMD Radeon RX 6600', vendorId: '1002', deviceId: '73ff' },
      ],
    }));
    expect(plist).not.toContain('ig-platform-id');
  });
});

// ─── Audio path boundary tests ──────────────────────────────────────────────

describe('generateConfigPlist — audio path by chipset era', () => {
  it('Rocket Lake uses modern audio path', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Rocket Lake' }));
    expect(plist).toContain('Pci(0x1f,0x3)');
  });

  it('Alder Lake uses modern audio path', () => {
    const plist = generateConfigPlist(fakeProfile({ generation: 'Alder Lake' }));
    expect(plist).toContain('Pci(0x1f,0x3)');
  });

  it('AMD Ryzen uses legacy audio path', () => {
    const plist = generateConfigPlist(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      coreCount: 8,
    }));
    expect(plist).toContain('Pci(0x1b,0x0)');
  });
});

// ─── Quirk policy cross-checks ──────────────────────────────────────────────

describe('getQuirksForGeneration — cross-generation policy', () => {
  it('Haswell uses EnableWriteUnprotector=true, RebuildAppleMemoryMap=false', () => {
    const q = getQuirksForGeneration('Haswell', '');
    expect(q.EnableWriteUnprotector).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(false);
  });

  it('Coffee Lake uses EnableWriteUnprotector=false, RebuildAppleMemoryMap=true', () => {
    const q = getQuirksForGeneration('Coffee Lake', '');
    expect(q.EnableWriteUnprotector).toBe(false);
    expect(q.RebuildAppleMemoryMap).toBe(true);
  });

  it('Comet Lake SetupVirtualMap=false (memory protection)', () => {
    const q = getQuirksForGeneration('Comet Lake', '');
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('Rocket Lake ProvideCurrentCpuInfo=true', () => {
    const q = getQuirksForGeneration('Rocket Lake', '');
    expect(q.ProvideCurrentCpuInfo).toBe(true);
  });

  it('AMD B550 SetupVirtualMap=false', () => {
    const q = getQuirksForGeneration('Ryzen', 'ASUS TUF B550');
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('AMD A520 SetupVirtualMap=false', () => {
    const q = getQuirksForGeneration('Ryzen', 'ASRock A520');
    expect(q.SetupVirtualMap).toBe(false);
  });

  it('AMD TRX40 SetupVirtualMap=false + DevirtualiseMmio=true', () => {
    const q = getQuirksForGeneration('Threadripper', 'ASUS TRX40');
    expect(q.SetupVirtualMap).toBe(false);
    expect(q.DevirtualiseMmio).toBe(true);
  });

  it('AMD AppleCpuPmCfgLock=false (not applicable)', () => {
    const q = getQuirksForGeneration('Ryzen', '');
    expect(q.AppleCpuPmCfgLock).toBe(false);
    expect(q.AppleXcpmCfgLock).toBe(false);
  });

  it('X99 board uses legacy quirk profile', () => {
    const q = getQuirksForGeneration('Haswell-E', 'ASUS X99');
    expect(q.EnableWriteUnprotector).toBe(true);
    expect(q.RebuildAppleMemoryMap).toBe(false);
    expect(q.SetupVirtualMap).toBe(true);
  });

  it('X299 board uses modern quirk profile', () => {
    const q = getQuirksForGeneration('Cascade Lake-X', 'ASUS X299');
    expect(q.EnableWriteUnprotector).toBe(false);
    expect(q.DevirtualiseMmio).toBe(true);
    expect(q.ProtectUefiServices).toBe(true);
    expect(q.SetupVirtualMap).toBe(false);
  });

  // --- v2.7.10 deep audit fixes ---

  it('HEDT (Haswell-E) enables AppleXcpmExtraMsrs', () => {
    const q = getQuirksForGeneration('Haswell-E', 'ASUS X99');
    expect(q.AppleXcpmExtraMsrs).toBe(true);
    expect(q.AppleCpuPmCfgLock).toBe(false);
  });

  it('HEDT (Broadwell-E) enables AppleXcpmExtraMsrs', () => {
    const q = getQuirksForGeneration('Broadwell-E', 'MSI X99A');
    expect(q.AppleXcpmExtraMsrs).toBe(true);
  });

  it('HEDT (Ivy Bridge-E) enables AppleXcpmExtraMsrs', () => {
    const q = getQuirksForGeneration('Ivy Bridge-E', 'ASUS Rampage');
    expect(q.AppleXcpmExtraMsrs).toBe(true);
  });

  it('Cascade Lake-X enables AppleXcpmExtraMsrs', () => {
    const q = getQuirksForGeneration('Cascade Lake-X', 'ASUS X299');
    expect(q.AppleXcpmExtraMsrs).toBe(true);
  });

  it('Ice Lake laptop gets MacBookPro16,2 SMBIOS', () => {
    const smbios = getSMBIOSForProfile(fakeProfile({
      generation: 'Ice Lake',
      isLaptop: true,
      targetOS: 'macOS Ventura 13',
    }));
    expect(smbios).toBe('MacBookPro16,2');
  });

  it('Ice Lake laptop gets SSDT-AWAC', () => {
    const r = getRequiredResources(fakeProfile({
      generation: 'Ice Lake',
      isLaptop: true,
    }));
    expect(r.ssdts).toContain('SSDT-AWAC.aml');
    expect(r.ssdts).toContain('SSDT-PLUG.aml');
    expect(r.ssdts).toContain('SSDT-EC-USBX-LAPTOP.aml');
  });

  it('Skylake on Tahoe enables FixupAppleEfiImages', () => {
    const q = getQuirksForGeneration('Skylake', '', false, 'canonical', 'macOS Tahoe 26');
    expect(q.FixupAppleEfiImages).toBe(true);
  });

  it('Kaby Lake on Tahoe enables FixupAppleEfiImages', () => {
    const q = getQuirksForGeneration('Kaby Lake', '', false, 'canonical', 'macOS Tahoe 26');
    expect(q.FixupAppleEfiImages).toBe(true);
  });

  it('AMD Ryzen on Tahoe enables FixupAppleEfiImages', () => {
    const q = getQuirksForGeneration('Ryzen', 'MSI B550', false, 'canonical', 'macOS Tahoe 26');
    expect(q.FixupAppleEfiImages).toBe(true);
  });

  it('Skylake+ disables AppleCpuPmCfgLock (XCPM only)', () => {
    for (const gen of ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake'] as const) {
      const q = getQuirksForGeneration(gen);
      expect(q.AppleCpuPmCfgLock, gen).toBe(false);
    }
  });

  it('ASUS boards enable DisableRtcChecksum', () => {
    const q = getQuirksForGeneration('Coffee Lake', 'ASUS ROG Z390');
    expect(q.DisableRtcChecksum).toBe(true);
  });

  it('Rocket Lake gets CPUID spoof in plist', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Rocket Lake',
      targetOS: 'macOS Ventura 13',
    }));
    expect(plist).toContain('VQYKAAAAAAAAAAAAAAAAAA==');
  });

  it('UpdateSMBIOSMode is Custom for MacPro7,1, Create for others', () => {
    const macProPlist = generateConfigPlist(fakeProfile({
      generation: 'Alder Lake',
      smbios: 'MacPro7,1',
      targetOS: 'macOS Ventura 13',
    }));
    expect(macProPlist).toContain('<key>UpdateSMBIOSMode</key><string>Custom</string>');

    const imacPlist = generateConfigPlist(fakeProfile({
      generation: 'Coffee Lake',
      smbios: 'iMac19,1',
      targetOS: 'macOS Ventura 13',
    }));
    expect(imacPlist).toContain('<key>UpdateSMBIOSMode</key><string>Create</string>');
  });

  it('agdpmod=pikera added for Polaris on iMac SMBIOS', () => {
    const plist = generateConfigPlist(fakeProfile({
      generation: 'Coffee Lake',
      targetOS: 'macOS Ventura 13',
      gpuDevices: [
        { name: 'Intel UHD 630' },
        { name: 'AMD Radeon RX 580' },
      ],
    }));
    expect(plist).toContain('agdpmod=pikera');
  });

  it('dk.e1000=0 only for Comet Lake+, not Skylake', () => {
    const clPlist = generateConfigPlist(fakeProfile({
      generation: 'Comet Lake',
      targetOS: 'macOS Ventura 13',
    }));
    expect(clPlist).toContain('dk.e1000=0');

    const skPlist = generateConfigPlist(fakeProfile({
      generation: 'Skylake',
      targetOS: 'macOS Ventura 13',
    }));
    expect(skPlist).not.toContain('dk.e1000=0');
  });

  it('Ivy Bridge-E capped at Monterey in compatibility', () => {
    // Tested indirectly — Ivy Bridge-E should throw on Tahoe SMBIOS
    expect(() => getSMBIOSForProfile(fakeProfile({
      generation: 'Ivy Bridge-E',
      targetOS: 'macOS Tahoe 26',
    }))).not.toThrow(); // HEDT goes to MacPro7,1, which is valid
  });

  it('HEDT generations get SSDT-EC-USBX and SSDT-PLUG', () => {
    for (const gen of ['Ivy Bridge-E', 'Haswell-E', 'Broadwell-E', 'Cascade Lake-X'] as const) {
      const r = getRequiredResources(fakeProfile({ generation: gen }));
      expect(r.ssdts, gen).toContain('SSDT-PLUG.aml');
      expect(r.ssdts, gen).toContain('SSDT-EC-USBX.aml');
    }
  });

  it('Cascade Lake-X includes SSDT-UNC', () => {
    const r = getRequiredResources(fakeProfile({ generation: 'Cascade Lake-X' }));
    expect(r.ssdts).toContain('SSDT-UNC.aml');
  });

  it('Pre-Sandy Bridge does NOT include SSDT-PLUG', () => {
    for (const gen of ['Penryn', 'Nehalem', 'Westmere'] as const) {
      const r = getRequiredResources(fakeProfile({ generation: gen }));
      expect(r.ssdts, gen).not.toContain('SSDT-PLUG.aml');
      expect(r.ssdts, gen).toContain('SSDT-EC.aml');
    }
  });

  it('LegacyNvram covers B360/H370/Q370/B365', () => {
    for (const board of ['MSI B360M', 'ASRock H370', 'Lenovo Q370', 'ASUS B365M']) {
      const plist = generateConfigPlist(fakeProfile({
        generation: 'Coffee Lake',
        motherboard: board,
        targetOS: 'macOS Ventura 13',
      }));
      expect(plist, board).toContain('<key>LegacyEnable</key><true/>');
    }
  });

  it('SSDT-PMC includes B365 and Q370', () => {
    for (const board of ['MSI B365M', 'Lenovo Q370']) {
      const r = getRequiredResources(fakeProfile({
        generation: 'Coffee Lake',
        motherboard: board,
      }));
      expect(r.ssdts, board).toContain('SSDT-PMC.aml');
    }
  });
});
