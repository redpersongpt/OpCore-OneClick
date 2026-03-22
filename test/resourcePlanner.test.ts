import { describe, it, expect } from 'vitest';
import { buildResourcePlan, type ResourcePlan, type ResourcePlanEntry } from '../electron/resourcePlanner.js';
import { getRequiredResources, getSMBIOSForProfile } from '../electron/configGenerator.js';
import type { HardwareProfile } from '../electron/configGenerator.js';
import { getSsdtSourcePolicy } from '../electron/ssdtSourcePolicy.js';
import type { KextRegistryEntry } from '../electron/kextSourcePolicy.js';

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

// Minimal registry covering all kexts the generator can request
const KEXT_REGISTRY: Record<string, KextRegistryEntry> = {
  'Lilu.kext':                       { repo: 'acidanthera/Lilu', assetFilter: 'RELEASE' },
  'VirtualSMC.kext':                 { repo: 'acidanthera/VirtualSMC', assetFilter: 'RELEASE' },
  'SMCBatteryManager.kext':          { repo: 'acidanthera/VirtualSMC', assetFilter: 'RELEASE' },
  'WhateverGreen.kext':              { repo: 'acidanthera/WhateverGreen', assetFilter: 'RELEASE' },
  'AppleALC.kext':                   { repo: 'acidanthera/AppleALC', assetFilter: 'RELEASE' },
  'NootedRed.kext':                  { repo: 'ChefKissInc/NootedRed', assetFilter: 'RELEASE' },
  'NootRX.kext':                     { repo: 'ChefKissInc/NootRX', directUrl: 'https://nightly.link/ChefKissInc/NootRX/workflows/main/master/Artifacts.zip', staticVersion: 'nightly' },
  'VoodooPS2Controller.kext':        { repo: 'acidanthera/VoodooPS2', assetFilter: 'RELEASE' },
  'AMDRyzenCPUPowerManagement.kext': { repo: 'trulyspinach/SMCAMDProcessor', directUrl: 'https://github.com/trulyspinach/SMCAMDProcessor/releases/latest/download/AMDRyzenCPUPowerManagement.kext.zip', staticVersion: 'latest' },
  'AppleMCEReporterDisabler.kext':   { repo: 'acidanthera/bugtracker', directUrl: 'https://github.com/acidanthera/bugtracker/files/3703498/AppleMCEReporterDisabler.kext.zip', staticVersion: 'bugtracker' },
  'RestrictEvents.kext':             { repo: 'acidanthera/RestrictEvents', assetFilter: 'RELEASE' },
  'NVMeFix.kext':                    { repo: 'acidanthera/NVMeFix', assetFilter: 'RELEASE' },
  'CPUTopologyRebuild.kext':         { repo: 'b00t0x/CpuTopologyRebuild', assetFilter: 'RELEASE' },
};

// ─── Resource plan completeness ─────────────────────────────────────────────

describe('buildResourcePlan — completeness', () => {
  it('includes all required kexts for Intel Comet Lake', () => {
    const profile = fakeProfile();
    const plan = buildResourcePlan({ profile, kextRegistry: KEXT_REGISTRY });
    const resources = getRequiredResources(profile);
    for (const kext of resources.kexts) {
      expect(plan.resources.some(r => r.name === kext && r.kind === 'kext'),
        `Missing kext in plan: ${kext}`).toBe(true);
    }
  });

  it('includes all required SSDTs for Intel Comet Lake', () => {
    const profile = fakeProfile();
    const plan = buildResourcePlan({ profile, kextRegistry: KEXT_REGISTRY });
    const resources = getRequiredResources(profile);
    for (const ssdt of resources.ssdts) {
      expect(plan.resources.some(r => r.name === ssdt && r.kind === 'ssdt'),
        `Missing SSDT in plan: ${ssdt}`).toBe(true);
    }
  });

  it('always includes OpenRuntime and OpenHfsPlus drivers', () => {
    const plan = buildResourcePlan({ profile: fakeProfile(), kextRegistry: KEXT_REGISTRY });
    expect(plan.resources.some(r => r.name === 'OpenRuntime.efi' && r.kind === 'driver')).toBe(true);
    expect(plan.resources.some(r => r.name === 'OpenHfsPlus.efi' && r.kind === 'driver')).toBe(true);
  });

  it('includes a recovery payload entry', () => {
    const plan = buildResourcePlan({ profile: fakeProfile(), kextRegistry: KEXT_REGISTRY });
    expect(plan.resources.some(r => r.kind === 'payload')).toBe(true);
  });
});

// ─── Source class correctness ───────────────────────────────────────────────

describe('buildResourcePlan — source class correctness', () => {
  it('kexts with registry entries are classified as downloaded', () => {
    const plan = buildResourcePlan({ profile: fakeProfile(), kextRegistry: KEXT_REGISTRY });
    const liluEntry = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(liluEntry).toBeTruthy();
    expect(liluEntry!.sourceClass).toBe('downloaded');
  });

  it('kexts with embedded source are classified as embedded (not bundled)', () => {
    const plan = buildResourcePlan({
      profile: fakeProfile(),
      kextRegistry: KEXT_REGISTRY,
      kextSources: { 'Lilu.kext': 'embedded' },
    });
    const liluEntry = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(liluEntry!.sourceClass).toBe('embedded');
  });

  it('embedded kext source string names the upstream repo', () => {
    const plan = buildResourcePlan({
      profile: fakeProfile(),
      kextRegistry: KEXT_REGISTRY,
      kextSources: { 'Lilu.kext': 'embedded' },
    });
    const liluEntry = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(liluEntry!.source).toContain('Embedded');
    expect(liluEntry!.source).toContain('acidanthera/Lilu');
  });

  it('kexts with github source are classified as downloaded', () => {
    const plan = buildResourcePlan({
      profile: fakeProfile(),
      kextRegistry: KEXT_REGISTRY,
      kextSources: { 'Lilu.kext': 'github' },
    });
    const liluEntry = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(liluEntry!.sourceClass).toBe('downloaded');
  });

  it('failed kexts are marked blocked', () => {
    const plan = buildResourcePlan({
      profile: fakeProfile(),
      kextRegistry: KEXT_REGISTRY,
      kextSources: { 'Lilu.kext': 'failed' },
    });
    const liluEntry = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(liluEntry!.validationOutcome).toBe('blocked');
  });

  it('drivers are always downloaded sourceClass', () => {
    const plan = buildResourcePlan({ profile: fakeProfile(), kextRegistry: KEXT_REGISTRY });
    const drivers = plan.resources.filter(r => r.kind === 'driver');
    for (const d of drivers) {
      expect(d.sourceClass).toBe('downloaded');
    }
  });

  it('recovery payload is downloaded sourceClass', () => {
    const plan = buildResourcePlan({ profile: fakeProfile(), kextRegistry: KEXT_REGISTRY });
    const payload = plan.resources.find(r => r.kind === 'payload');
    expect(payload!.sourceClass).toBe('downloaded');
  });
});

// ─── SSDT source policy integration ─────────────────────────────────────────

describe('buildResourcePlan — SSDT source policy', () => {
  it('SSDTs with source policy have correct validation outcome', () => {
    const profile = fakeProfile();
    const plan = buildResourcePlan({ profile, kextRegistry: KEXT_REGISTRY });
    const ssdtEntries = plan.resources.filter(r => r.kind === 'ssdt');
    for (const ssdt of ssdtEntries) {
      const policy = getSsdtSourcePolicy(ssdt.name);
      if (policy) {
        // Should be verified or pending_manual, not blocked
        expect(ssdt.validationOutcome, `${ssdt.name} should not be blocked`).not.toBe('blocked');
      }
    }
  });

  it('SSDT with supplemental download shows correct source description', () => {
    const profile = fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      motherboard: 'MSI B650',
    });
    const plan = buildResourcePlan({ profile, kextRegistry: KEXT_REGISTRY });
    const ecUsbx = plan.resources.find(r => r.name === 'SSDT-EC-USBX-DESKTOP.aml');
    expect(ecUsbx).toBeTruthy();
    expect(ecUsbx!.source).toContain('supplemental');
  });

  it('SSDT without supplemental shows OpenCore package source', () => {
    const profile = fakeProfile({ generation: 'Haswell' });
    const plan = buildResourcePlan({ profile, kextRegistry: KEXT_REGISTRY });
    const plug = plan.resources.find(r => r.name === 'SSDT-PLUG.aml');
    expect(plug).toBeTruthy();
    expect(plug!.source).toContain('OpenCore package');
  });
});

// ─── Cross-architecture plan correctness ────────────────────────────────────

describe('buildResourcePlan — architecture matrix', () => {
  const scenarios = [
    {
      name: 'Intel Comet Lake desktop',
      profile: fakeProfile(),
      expectedKexts: ['Lilu.kext', 'VirtualSMC.kext', 'WhateverGreen.kext', 'AppleALC.kext'],
      expectedSsdts: ['SSDT-PLUG.aml', 'SSDT-AWAC.aml', 'SSDT-EC-USBX.aml'],
    },
    {
      name: 'AMD Ryzen B650',
      profile: fakeProfile({ architecture: 'AMD', generation: 'Ryzen', motherboard: 'MSI B650', targetOS: 'macOS Monterey' }),
      expectedKexts: ['Lilu.kext', 'VirtualSMC.kext', 'AppleALC.kext', 'AMDRyzenCPUPowerManagement.kext', 'AppleMCEReporterDisabler.kext'],
      expectedSsdts: ['SSDT-EC-USBX-DESKTOP.aml', 'SSDT-CPUR.aml'],
    },
    {
      name: 'Intel Alder Lake desktop',
      profile: fakeProfile({ generation: 'Alder Lake' }),
      expectedKexts: ['Lilu.kext', 'VirtualSMC.kext', 'WhateverGreen.kext', 'AppleALC.kext', 'CPUTopologyRebuild.kext'],
      expectedSsdts: ['SSDT-PLUG-ALT.aml', 'SSDT-AWAC.aml', 'SSDT-EC-USBX.aml'],
    },
    {
      name: 'Intel Coffee Lake laptop',
      profile: fakeProfile({ generation: 'Coffee Lake', isLaptop: true }),
      expectedKexts: ['Lilu.kext', 'VirtualSMC.kext', 'SMCBatteryManager.kext', 'VoodooPS2Controller.kext'],
      expectedSsdts: ['SSDT-PNLF.aml', 'SSDT-XOSI.aml'],
    },
  ];

  for (const scenario of scenarios) {
    it(`${scenario.name}: plan includes all expected resources`, () => {
      const plan = buildResourcePlan({
        profile: scenario.profile,
        kextRegistry: KEXT_REGISTRY,
      });
      for (const kext of scenario.expectedKexts) {
        expect(plan.resources.some(r => r.name === kext),
          `${scenario.name}: missing kext ${kext}`).toBe(true);
      }
      for (const ssdt of scenario.expectedSsdts) {
        expect(plan.resources.some(r => r.name === ssdt),
          `${scenario.name}: missing SSDT ${ssdt}`).toBe(true);
      }
    });
  }
});

// ─── Kext identity/version display ──────────────────────────────────────────

describe('buildResourcePlan — kext identity display', () => {
  it('kexts with directUrl show direct asset description', () => {
    const plan = buildResourcePlan({ profile: fakeProfile({ architecture: 'AMD', generation: 'Ryzen', targetOS: 'macOS Monterey' }), kextRegistry: KEXT_REGISTRY });
    const amdPower = plan.resources.find(r => r.name === 'AMDRyzenCPUPowerManagement.kext');
    expect(amdPower).toBeTruthy();
    expect(amdPower!.expectedIdentityOrVersion).toContain('Direct asset');
  });

  it('kexts with assetFilter show repo and filter', () => {
    const plan = buildResourcePlan({ profile: fakeProfile(), kextRegistry: KEXT_REGISTRY });
    const lilu = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(lilu).toBeTruthy();
    expect(lilu!.expectedIdentityOrVersion).toContain('acidanthera/Lilu');
    expect(lilu!.expectedIdentityOrVersion).toContain('RELEASE');
  });
});

// ─── Network-exclusive kext annotation ──────────────────────────────────────

describe('buildResourcePlan — network-exclusive kext annotation', () => {
  // A registry entry without embeddedFallback is network-exclusive.
  const REGISTRY_WITH_FALLBACK: Record<string, KextRegistryEntry> = {
    ...KEXT_REGISTRY,
    'Lilu.kext':             { ...KEXT_REGISTRY['Lilu.kext'], embeddedFallback: true },
    'CPUTopologyRebuild.kext': { ...KEXT_REGISTRY['CPUTopologyRebuild.kext'], embeddedFallback: false },
  };

  it('pre-build plan annotates network-exclusive kexts in source description', () => {
    const plan = buildResourcePlan({
      profile: fakeProfile({ generation: 'Alder Lake' }),
      kextRegistry: REGISTRY_WITH_FALLBACK,
    });
    const cpuTopo = plan.resources.find(r => r.name === 'CPUTopologyRebuild.kext');
    expect(cpuTopo).toBeTruthy();
    expect(cpuTopo!.source).toContain('network required');
  });

  it('pre-build plan does NOT annotate kexts that have embedded fallback', () => {
    const plan = buildResourcePlan({
      profile: fakeProfile(),
      kextRegistry: REGISTRY_WITH_FALLBACK,
    });
    const lilu = plan.resources.find(r => r.name === 'Lilu.kext');
    expect(lilu).toBeTruthy();
    expect(lilu!.source).not.toContain('network required');
  });

  it('post-build plan does NOT annotate even network-exclusive kexts once source is known', () => {
    // Once kextSources is set (post-fetch), the annotation is suppressed — source is already resolved.
    const plan = buildResourcePlan({
      profile: fakeProfile({ generation: 'Alder Lake' }),
      kextRegistry: REGISTRY_WITH_FALLBACK,
      kextSources: { 'CPUTopologyRebuild.kext': 'github' },
    });
    const cpuTopo = plan.resources.find(r => r.name === 'CPUTopologyRebuild.kext');
    expect(cpuTopo!.source).not.toContain('network required');
  });
});
