/**
 * Office Laptop Resilience Tests
 *
 * Regression and coverage tests for old office laptop hardware detection:
 * - HP ProBook / EliteBook
 * - Dell Latitude
 * - Lenovo ThinkPad T/X/L
 * - Generic Haswell/Broadwell/Skylake/Kaby Lake office machines
 *
 * Tests cover:
 * 1. CPU generation parsing for mobile U-series CPUs
 * 2. GPU fallback when vendor driver is missing (Microsoft Basic Display Adapter)
 * 3. Audio codec detection from HDA vendor/device IDs
 * 4. OEM business-laptop model classification
 * 5. Confidence model behavior with partial detection
 */

import { describe, it, expect } from 'vitest';
import {
  resolveGpuVendor,
  isSoftwareDisplayAdapter,
  isGenericGpuName,
  normalizeWindowsGpuList,
  inferIntelIgpuName,
  enhanceGenericGpuNames,
  resolveAudioCodec,
  resolveNetworkAdapter,
  classifyNetworkType,
  isI2CDeviceId,
  deriveInputStack,
  buildPartialDetectedHardware,
  type GpuDevice,
  type AudioDevice,
  type NetworkDevice,
  type InputDevice,
} from '../electron/hardwareDetect.js';
import { detectCpuGeneration, detectArchitecture, mapDetectedToProfile } from '../electron/hardwareMapper.js';
import { classifyGpu, type HardwareGpuDeviceSummary } from '../electron/hackintoshRules.js';
import { interpretHardware } from '../electron/hardwareInterpret.js';
import { inferLaptopFormFactor } from '../electron/formFactor.js';
import {
  resolveAudioLayoutId,
  getRequiredResources,
  generateConfigPlist,
  resolveKextBundlePath,
  resolveKextExecutablePath,
  PLUGIN_KEXT_PARENTS,
} from '../electron/configGenerator.js';
import type { HardwareProfile } from '../electron/configGenerator.js';
import type { DetectedHardware } from '../electron/hardwareDetect.js';

function fakeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i7-10700K',
    architecture: 'Intel',
    generation: 'Comet Lake',
    gpu: 'Intel UHD 630',
    ram: '16 GB',
    motherboard: 'Generic',
    targetOS: 'macOS Sequoia 15.x',
    smbios: 'iMac20,1',
    kexts: [],
    ssdts: [],
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop: false,
    ...overrides,
  };
}

// ─── HP ProBook 450 G2 fixture ──────────────────────────────────────────────
// Real repro: Haswell i7-4510U, missing Intel GPU driver, Realtek ALC282 audio.
// Windows shows "Microsoft Basic Display Adapter" but PNPDeviceID has VEN_8086.

function makeHPProBook450G2(): DetectedHardware {
  return {
    cpu: {
      name: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      vendor: 'GenuineIntel',
      vendorName: 'Intel',
      confidence: 'detected',
    },
    gpus: [{
      name: 'Microsoft Basic Display Adapter',
      vendorId: '8086',
      deviceId: '0a16',
      vendorName: 'Intel',
      confidence: 'detected',
    }],
    primaryGpu: {
      name: 'Microsoft Basic Display Adapter',
      vendorId: '8086',
      deviceId: '0a16',
      vendorName: 'Intel',
      confidence: 'detected',
    },
    motherboardVendor: 'Hewlett-Packard',
    motherboardModel: 'HP ProBook 450 G2',
    ramBytes: 8 * 1024 * 1024 * 1024,
    coreCount: 2,
    isLaptop: true,
    isVM: false,
    audioDevices: [{
      name: 'Realtek High Definition Audio',
      vendorId: '10ec',
      deviceId: '0282',
      codecName: 'ALC282',
      confidence: 'detected',
    }],
    networkDevices: [
      {
        name: 'Intel(R) Ethernet Connection I218-LM',
        vendorId: '8086',
        deviceId: '155a',
        vendorName: 'Intel',
        adapterFamily: 'Intel I218-LM',
        type: 'ethernet',
        confidence: 'detected',
      },
      {
        name: 'Intel(R) Wireless 7260',
        vendorId: '8086',
        deviceId: '08b1',
        vendorName: 'Intel',
        adapterFamily: 'Intel Wireless 7260',
        type: 'wifi',
        confidence: 'detected',
      },
    ],
    inputDevices: [], // Old office laptop — PS2 trackpad, no I2C HID devices detected
  };
}

// ─── 1. CPU PARSING: Old office laptop mobile CPUs ──────────────────────────

describe('CPU generation parsing — old office laptop CPUs', () => {
  const cases: [string, string][] = [
    // HP ProBook 450 G2 repro
    ['Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz', 'Haswell'],
    // Other Haswell mobile U-series
    ['Intel(R) Core(TM) i5-4200U CPU @ 1.60GHz', 'Haswell'],
    ['Intel(R) Core(TM) i3-4005U CPU @ 1.70GHz', 'Haswell'],
    ['Intel(R) Core(TM) i5-4300U CPU @ 1.90GHz', 'Haswell'],
    ['Intel(R) Core(TM) i7-4600U CPU @ 2.10GHz', 'Haswell'],
    // Broadwell mobile (ThinkPad T450s era)
    ['Intel(R) Core(TM) i5-5200U CPU @ 2.20GHz', 'Broadwell'],
    ['Intel(R) Core(TM) i7-5500U CPU @ 2.40GHz', 'Broadwell'],
    ['Intel(R) Core(TM) i5-5300U CPU @ 2.30GHz', 'Broadwell'],
    // Skylake mobile (ThinkPad T460 era)
    ['Intel(R) Core(TM) i5-6200U CPU @ 2.30GHz', 'Skylake'],
    ['Intel(R) Core(TM) i7-6500U CPU @ 2.50GHz', 'Skylake'],
    ['Intel(R) Core(TM) i5-6300U CPU @ 2.40GHz', 'Skylake'],
    // Kaby Lake mobile (ThinkPad T470 era)
    ['Intel(R) Core(TM) i5-7200U CPU @ 2.50GHz', 'Kaby Lake'],
    ['Intel(R) Core(TM) i7-7500U CPU @ 2.70GHz', 'Kaby Lake'],
    // Coffee Lake mobile (ThinkPad T480 era)
    ['Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz', 'Coffee Lake'],
    ['Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz', 'Coffee Lake'],
    // Ivy Bridge mobile (old ThinkPad T430 era)
    ['Intel(R) Core(TM) i5-3320M CPU @ 2.60GHz', 'Ivy Bridge'],
    ['Intel(R) Core(TM) i7-3520M CPU @ 2.90GHz', 'Ivy Bridge'],
    // Sandy Bridge mobile
    ['Intel(R) Core(TM) i5-2520M CPU @ 2.50GHz', 'Sandy Bridge'],
    ['Intel(R) Core(TM) i7-2620M CPU @ 2.70GHz', 'Sandy Bridge'],
  ];

  for (const [cpu, expected] of cases) {
    it(`${cpu} → ${expected}`, () => {
      expect(detectCpuGeneration(cpu)).toBe(expected);
    });
  }

  it('all old mobile CPUs detected as Intel architecture', () => {
    for (const [cpu] of cases) {
      expect(detectArchitecture(cpu)).toBe('Intel');
    }
  });
});

// ─── 2. GPU FALLBACK: Missing driver detection ─────────────────────────────

describe('GPU fallback — Microsoft Basic Display Adapter with real Intel PCI ID', () => {
  const basicAdapterWithIntelPci: GpuDevice = {
    name: 'Microsoft Basic Display Adapter',
    vendorId: '8086',
    deviceId: '0a16',
    vendorName: 'Intel',
    confidence: 'detected',
  };

  const pureBasicAdapter: GpuDevice = {
    name: 'Microsoft Basic Display Adapter',
    vendorId: '1414',
    deviceId: null,
    vendorName: 'Microsoft',
    confidence: 'detected',
  };

  const pureBasicAdapterNoVendor: GpuDevice = {
    name: 'Microsoft Basic Display Adapter',
    vendorId: null,
    deviceId: null,
    vendorName: 'Microsoft',
    confidence: 'partially-detected',
  };

  it('Intel PCI vendor ID prevents software adapter classification', () => {
    expect(isSoftwareDisplayAdapter(basicAdapterWithIntelPci)).toBe(false);
  });

  it('Microsoft vendor ID 1414 IS classified as software adapter', () => {
    expect(isSoftwareDisplayAdapter(pureBasicAdapter)).toBe(true);
  });

  it('null vendor ID + basic adapter name IS classified as software adapter', () => {
    expect(isSoftwareDisplayAdapter(pureBasicAdapterNoVendor)).toBe(true);
  });

  it('NVIDIA PCI vendor ID prevents software adapter classification', () => {
    const nvidiaGeneric: GpuDevice = {
      name: 'Microsoft Basic Display Adapter',
      vendorId: '10de',
      deviceId: '1234',
      vendorName: 'NVIDIA',
      confidence: 'detected',
    };
    expect(isSoftwareDisplayAdapter(nvidiaGeneric)).toBe(false);
  });

  it('AMD PCI vendor ID prevents software adapter classification', () => {
    const amdGeneric: GpuDevice = {
      name: 'Microsoft Basic Display Adapter',
      vendorId: '1002',
      deviceId: '5678',
      vendorName: 'AMD',
      confidence: 'detected',
    };
    expect(isSoftwareDisplayAdapter(amdGeneric)).toBe(false);
  });

  it('normalizeWindowsGpuList keeps Intel-backed basic adapter', () => {
    const result = normalizeWindowsGpuList([basicAdapterWithIntelPci]);
    expect(result).toHaveLength(1);
    expect(result[0].vendorId).toBe('8086');
  });

  it('normalizeWindowsGpuList filters pure software adapters', () => {
    const realGpu: GpuDevice = {
      name: 'Intel HD Graphics 4600',
      vendorId: '8086',
      deviceId: '0412',
      vendorName: 'Intel',
      confidence: 'detected',
    };
    const result = normalizeWindowsGpuList([realGpu, pureBasicAdapter]);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('Intel HD Graphics 4600');
  });
});

describe('GPU name enhancement — inferring Intel iGPU from CPU', () => {
  it('Haswell i7-4510U → Intel HD Graphics 4400/4600', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz'))
      .toBe('Intel HD Graphics 4400/4600');
  });

  it('Broadwell i5-5200U → Intel HD Graphics 5500/6000', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-5200U CPU @ 2.20GHz'))
      .toBe('Intel HD Graphics 5500/6000');
  });

  it('Skylake i5-6200U → Intel HD Graphics 520/530', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-6200U CPU @ 2.30GHz'))
      .toBe('Intel HD Graphics 520/530');
  });

  it('Kaby Lake i5-7200U → Intel HD Graphics 620/630', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-7200U CPU @ 2.50GHz'))
      .toBe('Intel HD Graphics 620/630');
  });

  it('Coffee Lake i5-8250U → Intel UHD Graphics 630', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz'))
      .toBe('Intel UHD Graphics 630');
  });

  it('Comet Lake i5-10210U → Intel UHD Graphics 630', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-10210U CPU @ 1.60GHz'))
      .toBe('Intel UHD Graphics 630');
  });

  it('Ivy Bridge i5-3320M → Intel HD Graphics 4000', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-3320M CPU @ 2.60GHz'))
      .toBe('Intel HD Graphics 4000');
  });

  it('Sandy Bridge i5-2520M → Intel HD Graphics 2000/3000', () => {
    expect(inferIntelIgpuName('Intel(R) Core(TM) i5-2520M CPU @ 2.50GHz'))
      .toBe('Intel HD Graphics 2000/3000');
  });

  it('non-Intel CPU returns null', () => {
    expect(inferIntelIgpuName('AMD Ryzen 5 5600X')).toBeNull();
  });

  it('enhanceGenericGpuNames replaces basic adapter name with inferred iGPU', () => {
    const gpus: GpuDevice[] = [{
      name: 'Microsoft Basic Display Adapter',
      vendorId: '8086',
      deviceId: '0a16',
      vendorName: 'Intel',
      confidence: 'detected',
    }];
    const result = enhanceGenericGpuNames(gpus, 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz');
    expect(result[0].name).toContain('Intel HD Graphics 4400/4600');
    expect(result[0].name).toContain('driver not installed');
    expect(result[0].vendorId).toBe('8086'); // PCI ID preserved
  });

  it('enhanceGenericGpuNames does NOT touch non-generic GPU names', () => {
    const gpus: GpuDevice[] = [{
      name: 'Intel HD Graphics 4600',
      vendorId: '8086',
      deviceId: '0412',
      vendorName: 'Intel',
      confidence: 'detected',
    }];
    const result = enhanceGenericGpuNames(gpus, 'Intel(R) Core(TM) i7-4770K CPU @ 3.50GHz');
    expect(result[0].name).toBe('Intel HD Graphics 4600');
  });

  it('enhanceGenericGpuNames does NOT touch basic adapter with non-Intel vendor', () => {
    const gpus: GpuDevice[] = [{
      name: 'Microsoft Basic Display Adapter',
      vendorId: null,
      deviceId: null,
      vendorName: 'Microsoft',
      confidence: 'partially-detected',
    }];
    const result = enhanceGenericGpuNames(gpus, 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz');
    expect(result[0].name).toBe('Microsoft Basic Display Adapter');
  });
});

describe('classifyGpu — Intel iGPU with generic adapter name', () => {
  it('basic adapter with Intel vendorId is NOT classified as unsupported software', () => {
    const device: HardwareGpuDeviceSummary = {
      name: 'Microsoft Basic Display Adapter',
      vendorName: 'Intel',
      vendorId: '8086',
      deviceId: '0a16',
    };
    const result = classifyGpu(device);
    // Should NOT be classified as "software adapter, safely ignored"
    expect(result.notes.join(' ')).not.toContain('software or remote display adapter');
  });

  it('basic adapter with Intel vendorId gets Intel vendor classification', () => {
    const device: HardwareGpuDeviceSummary = {
      name: 'Microsoft Basic Display Adapter',
      vendorName: 'Intel',
      vendorId: '8086',
      deviceId: '0a16',
    };
    const result = classifyGpu(device);
    expect(result.vendor).toBe('Intel');
  });

  it('enhanced Haswell iGPU name classifies as supported with limit', () => {
    const device: HardwareGpuDeviceSummary = {
      name: 'Intel HD Graphics 4400/4600 (driver not installed)',
      vendorName: 'Intel',
      vendorId: '8086',
      deviceId: '0a16',
    };
    const result = classifyGpu(device);
    // Should match the HD 4400/4600 pattern
    expect(result.vendor).toBe('Intel');
    expect(result.tier).not.toBe('unsupported');
  });

  it('pure software basic adapter with no real vendor IS unsupported', () => {
    const device: HardwareGpuDeviceSummary = {
      name: 'Microsoft Basic Display Adapter',
      vendorName: 'Unknown',
      vendorId: null,
      deviceId: null,
    };
    const result = classifyGpu(device);
    expect(result.notes.join(' ')).toContain('software or remote display adapter');
  });
});

// ─── 3. AUDIO: HDA vendor/device ID detection ──────────────────────────────

describe('Audio codec detection from HDA device IDs', () => {
  it('VEN_10EC&DEV_0282 → ALC282', () => {
    expect(resolveAudioCodec('10ec', '0282')).toBe('ALC282');
  });

  it('VEN_10EC&DEV_0892 → ALC892', () => {
    expect(resolveAudioCodec('10ec', '0892')).toBe('ALC892');
  });

  it('VEN_10EC&DEV_0887 → ALC887', () => {
    expect(resolveAudioCodec('10ec', '0887')).toBe('ALC887');
  });

  it('VEN_10EC&DEV_0256 → ALC256', () => {
    expect(resolveAudioCodec('10ec', '0256')).toBe('ALC256');
  });

  it('VEN_10EC&DEV_0255 → ALC255', () => {
    expect(resolveAudioCodec('10ec', '0255')).toBe('ALC255');
  });

  it('VEN_10EC&DEV_0269 → ALC269', () => {
    expect(resolveAudioCodec('10ec', '0269')).toBe('ALC269');
  });

  it('VEN_10EC&DEV_0b50 → ALC1220', () => {
    expect(resolveAudioCodec('10ec', '0b50')).toBe('ALC1220');
  });

  it('unknown Realtek device ID returns partial info', () => {
    const result = resolveAudioCodec('10ec', 'ffff');
    expect(result).toContain('Realtek');
    expect(result).toContain('DEV_FFFF');
  });

  it('Conexant vendor → Conexant codec string', () => {
    const result = resolveAudioCodec('14f1', '5066');
    expect(result).toContain('Conexant');
  });

  it('IDT vendor → IDT codec string', () => {
    const result = resolveAudioCodec('111d', '7675');
    expect(result).toContain('IDT');
  });

  it('null vendor returns null', () => {
    expect(resolveAudioCodec(null, '0282')).toBeNull();
  });

  it('null device returns null', () => {
    expect(resolveAudioCodec('10ec', null)).toBeNull();
  });

  it('unknown vendor ID returns null', () => {
    expect(resolveAudioCodec('dead', 'beef')).toBeNull();
  });
});

describe('Audio codec → layout-id mapping', () => {
  it('ALC282 → layout-id 3', () => {
    expect(resolveAudioLayoutId('ALC282')).toBe(3);
  });

  it('ALC887 → layout-id 1', () => {
    expect(resolveAudioLayoutId('ALC887')).toBe(1);
  });

  it('ALC255 → layout-id 3', () => {
    expect(resolveAudioLayoutId('ALC255')).toBe(3);
  });

  it('ALC1220 → layout-id 1', () => {
    expect(resolveAudioLayoutId('ALC1220')).toBe(1);
  });

  it('unknown codec → fallback layout-id 1', () => {
    expect(resolveAudioLayoutId('Unknown Codec')).toBe(1);
  });

  it('undefined codec → fallback layout-id 1', () => {
    expect(resolveAudioLayoutId(undefined)).toBe(1);
  });
});

// ─── 4. OEM LAPTOP MODEL DETECTION ─────────────────────────────────────────

describe('OEM laptop model detection — form factor', () => {
  const laptopModels = [
    'HP ProBook 450 G2',
    'HP ProBook 440 G3',
    'HP ProBook 640 G1',
    'HP EliteBook 840 G5',
    'HP EliteBook 850 G3',
    'HP EliteBook Folio 9480m',
    'Dell Latitude E7450',
    'Dell Latitude 5480',
    'Dell Latitude E6430',
    'Dell Latitude 3340',
    'Lenovo ThinkPad T450s',
    'Lenovo ThinkPad T460',
    'Lenovo ThinkPad X230',
    'Lenovo ThinkPad X1 Carbon',
    'Lenovo ThinkPad L440',
    'Lenovo IdeaPad 330',
    'ASUS ZenBook UX430',
    'Acer TravelMate P2510',
    'Fujitsu LifeBook E734',
    'Microsoft Surface Pro 4',
  ];

  for (const model of laptopModels) {
    it(`${model} detected as laptop from model name`, () => {
      expect(inferLaptopFormFactor({
        cpuName: 'Intel(R) Core(TM) i5-5200U CPU @ 2.20GHz',
        modelName: model,
      })).toBe(true);
    });
  }

  it('chassis type 9 (laptop) overrides desktop model name', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i5-6200U CPU @ 2.30GHz',
      chassisTypes: [9],
      modelName: 'Generic Desktop',
    })).toBe(true);
  });

  it('mobile CPU suffix + battery detects laptop without model name', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      batteryPresent: true,
    })).toBe(true);
  });

  it('U-suffix CPU alone IS enough for laptop (U is never desktop)', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      batteryPresent: false,
    })).toBe(true);
  });

  it('H-suffix CPU without battery or model does NOT assume laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz',
      batteryPresent: false,
    })).toBe(false);
  });
});

// ─── 5. HP ProBook 450 G2 FULL PIPELINE TEST ───────────────────────────────

describe('HP ProBook 450 G2 — full pipeline integration', () => {
  const hw = makeHPProBook450G2();

  it('CPU generation is Haswell, not Unknown', () => {
    expect(detectCpuGeneration(hw.cpu.name)).toBe('Haswell');
  });

  it('architecture is Intel', () => {
    expect(detectArchitecture(hw.cpu.name)).toBe('Intel');
  });

  it('GPU with Intel vendorId is NOT filtered as software adapter', () => {
    expect(isSoftwareDisplayAdapter(hw.gpus[0])).toBe(false);
  });

  it('GPU vendor resolves to Intel from PCI vendor ID', () => {
    expect(resolveGpuVendor('8086', 'Microsoft Basic Display Adapter')).toBe('Intel');
  });

  it('audio codec is ALC282', () => {
    expect(hw.audioDevices[0].codecName).toBe('ALC282');
  });

  it('mapDetectedToProfile produces valid Haswell laptop profile', () => {
    // Apply GPU name enhancement (normally done in detectWindowsHardware)
    const enhanced: DetectedHardware = {
      ...hw,
      gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
    };
    enhanced.primaryGpu = enhanced.gpus[0];

    const profile = mapDetectedToProfile(enhanced);
    expect(profile.generation).toBe('Haswell');
    expect(profile.architecture).toBe('Intel');
    expect(profile.isLaptop).toBe(true);
    expect(profile.audioCodec).toBe('ALC282');
    expect(profile.audioLayoutId).toBe(3); // ALC282 → layout-id 3
    expect(profile.gpu).not.toBe('Unknown GPU');
    expect(profile.gpu).toContain('Intel');
    expect(profile.scanConfidence).not.toBe('low');
  });

  it('interpretHardware does NOT produce low confidence for this machine', () => {
    const enhanced: DetectedHardware = {
      ...hw,
      gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
    };
    enhanced.primaryGpu = enhanced.gpus[0];

    const interp = interpretHardware(enhanced);
    // Should not be low confidence — we have CPU, GPU (Intel), board, etc.
    expect(interp.overallConfidence).not.toBe('low');
    expect(interp.cpu.generation.value).toBe('Haswell');
    expect(interp.primaryGpu.vendor.value).toBe('Intel');
    expect(interp.audio.codec.value).toBe('ALC282');
    expect(interp.audio.codec.basis).toBe('detected');
  });

  it('summary mentions Intel Haswell laptop', () => {
    const enhanced: DetectedHardware = {
      ...hw,
      gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
    };
    enhanced.primaryGpu = enhanced.gpus[0];

    const interp = interpretHardware(enhanced);
    expect(interp.summary).toContain('Intel');
    expect(interp.summary).toContain('Haswell');
    expect(interp.summary).toContain('laptop');
  });
});

// ─── 6. CONFIDENCE MODEL: Partial but useful detection ──────────────────────

describe('Confidence model — partial detection behavior', () => {
  it('detected CPU + inferred GPU is medium confidence, not low', () => {
    const hw = makeHPProBook450G2();
    const enhanced: DetectedHardware = {
      ...hw,
      gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
    };
    enhanced.primaryGpu = enhanced.gpus[0];
    const interp = interpretHardware(enhanced);
    // Should be medium (not low) because CPU and GPU vendor are known
    expect(['high', 'medium']).toContain(interp.overallConfidence);
  });

  it('completely unknown GPU still produces low confidence', () => {
    const hw: DetectedHardware = {
      cpu: {
        name: 'Some Random CPU',
        vendor: 'Unknown',
        vendorName: 'Unknown',
        confidence: 'unverified',
      },
      gpus: [{ name: 'Unknown GPU', vendorId: null, deviceId: null, vendorName: 'Unknown', confidence: 'unverified' }],
      primaryGpu: { name: 'Unknown GPU', vendorId: null, deviceId: null, vendorName: 'Unknown', confidence: 'unverified' },
      motherboardVendor: 'Unknown',
      motherboardModel: 'Unknown',
      ramBytes: 8 * 1024 * 1024 * 1024,
      coreCount: 4,
      isLaptop: false,
      isVM: false,
      audioDevices: [],
      networkDevices: [],
      inputDevices: [],
    };
    const interp = interpretHardware(hw);
    expect(interp.overallConfidence).toBe('low');
  });

  it('audio interpretation shows codec when detected', () => {
    const hw = makeHPProBook450G2();
    const interp = interpretHardware(hw);
    expect(interp.audio.codec.value).toBe('ALC282');
    expect(interp.audio.codec.basis).toBe('detected');
    expect(interp.audio.layoutId.basis).toBe('derived');
  });

  it('audio interpretation shows fallback when no audio detected', () => {
    const hw: DetectedHardware = {
      ...makeHPProBook450G2(),
      audioDevices: [],
    };
    const interp = interpretHardware(hw);
    expect(interp.audio.codec.value).toBe('Not detected');
    expect(interp.audio.codec.basis).toBe('unknown');
    expect(interp.audio.layoutId.value).toContain('fallback');
  });
});

// ─── 7. CROSS-MODEL OLD OFFICE LAPTOP MATRIX ───────────────────────────────

describe('Old office laptop detection matrix', () => {
  const scenarios = [
    {
      name: 'ThinkPad T440 (Haswell)',
      cpu: 'Intel(R) Core(TM) i5-4300U CPU @ 1.90GHz',
      expectedGen: 'Haswell',
      model: 'Lenovo ThinkPad T440',
    },
    {
      name: 'ThinkPad T450s (Broadwell)',
      cpu: 'Intel(R) Core(TM) i5-5300U CPU @ 2.30GHz',
      expectedGen: 'Broadwell',
      model: 'Lenovo ThinkPad T450s',
    },
    {
      name: 'Dell Latitude E7450 (Broadwell)',
      cpu: 'Intel(R) Core(TM) i7-5600U CPU @ 2.60GHz',
      expectedGen: 'Broadwell',
      model: 'Dell Latitude E7450',
    },
    {
      name: 'Dell Latitude 5480 (Kaby Lake)',
      cpu: 'Intel(R) Core(TM) i5-7300U CPU @ 2.60GHz',
      expectedGen: 'Kaby Lake',
      model: 'Dell Latitude 5480',
    },
    {
      name: 'HP EliteBook 840 G3 (Skylake)',
      cpu: 'Intel(R) Core(TM) i5-6300U CPU @ 2.40GHz',
      expectedGen: 'Skylake',
      model: 'HP EliteBook 840 G3',
    },
    {
      name: 'ThinkPad X230 (Ivy Bridge)',
      cpu: 'Intel(R) Core(TM) i5-3320M CPU @ 2.60GHz',
      expectedGen: 'Ivy Bridge',
      model: 'Lenovo ThinkPad X230',
    },
    {
      name: 'HP ProBook 640 G1 (Haswell)',
      cpu: 'Intel(R) Core(TM) i5-4200M CPU @ 2.50GHz',
      expectedGen: 'Haswell',
      model: 'HP ProBook 640 G1',
    },
  ];

  for (const s of scenarios) {
    it(`${s.name}: generation=${s.expectedGen}, detected as laptop`, () => {
      expect(detectCpuGeneration(s.cpu)).toBe(s.expectedGen);
      expect(detectArchitecture(s.cpu)).toBe('Intel');
      expect(inferLaptopFormFactor({ cpuName: s.cpu, modelName: s.model })).toBe(true);
    });
  }

  it('all old office laptops get iGPU inference when driver is missing', () => {
    for (const s of scenarios) {
      const igpu = inferIntelIgpuName(s.cpu);
      expect(igpu, `${s.name} should produce iGPU name`).not.toBeNull();
      expect(igpu).toContain('Intel');
    }
  });
});

// ─── 8. DXDIAG / CHIP TYPE FALLBACK ────────────────────────────────────────

describe('DxDiag chip type fallback — VideoProcessor field', () => {
  it('isGenericGpuName identifies generic names', () => {
    expect(isGenericGpuName('Microsoft Basic Display Adapter')).toBe(true);
    expect(isGenericGpuName('Standard VGA Graphics Adapter')).toBe(true);
    expect(isGenericGpuName('Unknown GPU')).toBe(true);
    expect(isGenericGpuName('')).toBe(true);
  });

  it('isGenericGpuName passes real GPU names', () => {
    expect(isGenericGpuName('Intel HD Graphics 4600')).toBe(false);
    expect(isGenericGpuName('NVIDIA GeForce GTX 1080')).toBe(false);
    expect(isGenericGpuName('Intel(R) HSW Mobile/Desktop Graphics Controller')).toBe(false);
    expect(isGenericGpuName('AMD Radeon RX 580')).toBe(false);
  });

  // VideoProcessor (chip type) is the DxDiag equivalent — it contains the real
  // GPU description even when Windows Name is generic. The GPU parsing code
  // uses VideoProcessor as a fallback when Name is generic.
  it('chip type string "Intel(R) HSW Mobile/Desktop..." is not generic', () => {
    expect(isGenericGpuName('Intel(R) HSW Mobile/Desktop Graphics Controller')).toBe(false);
  });

  it('chip type string with Intel keyword resolves to Intel vendor', () => {
    expect(resolveGpuVendor(null, 'Intel(R) HSW Mobile/Desktop Graphics Controller')).toBe('Intel');
  });

  it('chip type string with HD Graphics resolves to Intel vendor', () => {
    expect(resolveGpuVendor(null, 'Intel(R) HD Graphics Family')).toBe('Intel');
  });
});

// ─── 9. WI-FI / ETHERNET HARDWARE DETECTION ────────────────────────────────

describe('Network adapter detection — Intel Ethernet', () => {
  const intelEthernetCases: [string, string, string][] = [
    // [deviceId, expectedFamily, description]
    ['155a', 'Intel I218-LM', 'I218-LM (Haswell laptop)'],
    ['1559', 'Intel I218-V', 'I218-V (Haswell desktop)'],
    ['153a', 'Intel I217-LM', 'I217-LM (Haswell desktop)'],
    ['153b', 'Intel I217-V', 'I217-V (Haswell desktop)'],
    ['156f', 'Intel I219-LM', 'I219-LM v1 (Skylake laptop)'],
    ['1570', 'Intel I219-V', 'I219-V v1 (Skylake desktop)'],
    ['15b7', 'Intel I219-LM', 'I219-LM v4 (Kaby Lake)'],
    ['15b8', 'Intel I219-V', 'I219-V v4 (Kaby Lake)'],
    ['15bb', 'Intel I219-LM', 'I219-LM v5 (Coffee Lake)'],
    ['1539', 'Intel I211-AT', 'I211-AT (desktop, needs AppleIGB)'],
    ['1502', 'Intel 82579LM', '82579LM (Sandy/Ivy Bridge)'],
    ['1503', 'Intel 82579V', '82579V (Sandy Bridge desktop)'],
    ['10d3', 'Intel 82574L', '82574L (server/workstation)'],
    ['15f2', 'Intel I225-V', 'I225-V (Alder Lake desktop)'],
    ['125b', 'Intel I226-V', 'I226-V (Raptor Lake desktop)'],
  ];

  for (const [did, expected, desc] of intelEthernetCases) {
    it(`Intel 8086:${did} → ${desc}`, () => {
      const result = resolveNetworkAdapter('8086', did, 'Ethernet Controller');
      expect(result.vendorName).toBe('Intel');
      expect(result.adapterFamily).toBe(expected);
      expect(result.type).toBe('ethernet');
    });
  }
});

describe('Network adapter detection — Realtek Ethernet', () => {
  it('RTL8111 (10ec:8168)', () => {
    const result = resolveNetworkAdapter('10ec', '8168', 'Realtek PCIe GbE');
    expect(result.vendorName).toBe('Realtek');
    expect(result.adapterFamily).toBe('Realtek RTL8111');
    expect(result.type).toBe('ethernet');
  });

  it('RTL8125 2.5G (10ec:8125)', () => {
    const result = resolveNetworkAdapter('10ec', '8125', 'Realtek Gaming Ethernet');
    expect(result.vendorName).toBe('Realtek');
    expect(result.adapterFamily).toBe('Realtek RTL8125');
    expect(result.type).toBe('ethernet');
  });

  it('RTL8101/8102 (10ec:8136)', () => {
    const result = resolveNetworkAdapter('10ec', '8136', 'Realtek FE NIC');
    expect(result.vendorName).toBe('Realtek');
    expect(result.adapterFamily).toBe('Realtek RTL8101/8102');
    expect(result.type).toBe('ethernet');
  });

  it('unknown Realtek device ID → partial info', () => {
    const result = resolveNetworkAdapter('10ec', 'ffff', 'Some Realtek');
    expect(result.vendorName).toBe('Realtek');
    expect(result.adapterFamily).toBe('Realtek (unknown model)');
  });
});

describe('Network adapter detection — Intel Wi-Fi', () => {
  const intelWifiCases: [string, string, string][] = [
    ['08b1', 'Intel Wireless 7260', 'Wireless 7260 (Haswell era)'],
    ['095a', 'Intel Wireless 7265', 'Wireless 7265 (Broadwell era)'],
    ['24f3', 'Intel Wireless 8260', 'Wireless 8260 (Skylake era)'],
    ['24fd', 'Intel Wireless 8265', 'Wireless 8265 (Kaby Lake era)'],
    ['2526', 'Intel Wireless 9260', 'Wireless 9260 (Coffee Lake era)'],
    ['9df0', 'Intel Wireless 9560', 'Wireless 9560 (Coffee Lake)'],
    ['2723', 'Intel Wi-Fi 6 AX200', 'Wi-Fi 6 AX200'],
    ['02f0', 'Intel Wi-Fi 6 AX201', 'Wi-Fi 6 AX201'],
    ['2725', 'Intel Wi-Fi 6E AX210', 'Wi-Fi 6E AX210'],
    ['3165', 'Intel Wireless 3165', 'Wireless 3165 (budget)'],
  ];

  for (const [did, expected, desc] of intelWifiCases) {
    it(`Intel 8086:${did} → ${desc}`, () => {
      const result = resolveNetworkAdapter('8086', did, 'Wi-Fi Adapter');
      expect(result.vendorName).toBe('Intel');
      expect(result.adapterFamily).toBe(expected);
      expect(result.type).toBe('wifi');
    });
  }
});

describe('Network adapter detection — Broadcom Wi-Fi', () => {
  it('BCM4360 (14e4:43a0)', () => {
    const result = resolveNetworkAdapter('14e4', '43a0', 'Wireless');
    expect(result.vendorName).toBe('Broadcom');
    expect(result.adapterFamily).toBe('Broadcom BCM4360');
    expect(result.type).toBe('wifi');
  });

  it('BCM4352 (14e4:43b1)', () => {
    const result = resolveNetworkAdapter('14e4', '43b1', 'Wireless');
    expect(result.vendorName).toBe('Broadcom');
    expect(result.adapterFamily).toBe('Broadcom BCM4352');
  });

  it('BCM43602 (14e4:43ba)', () => {
    const result = resolveNetworkAdapter('14e4', '43ba', 'AirPort');
    expect(result.vendorName).toBe('Broadcom');
    expect(result.adapterFamily).toBe('Broadcom BCM43602');
  });

  it('unknown Broadcom → partial info with wifi type', () => {
    const result = resolveNetworkAdapter('14e4', 'ffff', 'Wireless Network');
    expect(result.vendorName).toBe('Broadcom');
    expect(result.adapterFamily).toBe('Broadcom Wi-Fi (unknown model)');
    expect(result.type).toBe('wifi');
  });
});

describe('Network adapter detection — Atheros/Killer', () => {
  it('Killer E2200 (1969:e091)', () => {
    const result = resolveNetworkAdapter('1969', 'e091', 'Killer E2200');
    expect(result.vendorName).toBe('Atheros');
    expect(result.adapterFamily).toBe('Killer E2200');
    expect(result.type).toBe('ethernet');
  });

  it('Atheros AR8161 (1969:1091)', () => {
    const result = resolveNetworkAdapter('1969', '1091', 'Atheros Ethernet');
    expect(result.vendorName).toBe('Atheros');
    expect(result.adapterFamily).toBe('Atheros AR8161');
  });
});

describe('Network device type classification', () => {
  it('wireless keywords → wifi', () => {
    expect(classifyNetworkType('Intel(R) Wireless-AC 8265')).toBe('wifi');
    expect(classifyNetworkType('Intel(R) Wi-Fi 6 AX200 160MHz')).toBe('wifi');
    expect(classifyNetworkType('Broadcom 802.11ac Wireless')).toBe('wifi');
    expect(classifyNetworkType('Intel Centrino Advanced-N 6205')).toBe('wifi');
    expect(classifyNetworkType('Intel Dual Band Wireless-AC 7260')).toBe('wifi');
  });

  it('ethernet keywords → ethernet', () => {
    expect(classifyNetworkType('Intel(R) Ethernet Connection I219-V')).toBe('ethernet');
    expect(classifyNetworkType('Realtek PCIe GbE Family Controller')).toBe('ethernet');
    expect(classifyNetworkType('Intel(R) 82579LM Gigabit Network Connection')).toBe('ethernet');
    expect(classifyNetworkType('Killer E2200 Gigabit Ethernet Controller')).toBe('ethernet');
  });

  it('ambiguous → unknown', () => {
    expect(classifyNetworkType('Some Random Network Device')).toBe('unknown');
  });
});

describe('Network adapter — null/unknown vendor handling', () => {
  it('null vendor returns Unknown', () => {
    const result = resolveNetworkAdapter(null, null, 'Something');
    expect(result.vendorName).toBe('Unknown');
    expect(result.adapterFamily).toBeNull();
  });

  it('unknown vendor ID returns Unknown', () => {
    const result = resolveNetworkAdapter('dead', 'beef', 'Mystery NIC');
    expect(result.vendorName).toBe('Unknown');
    expect(result.adapterFamily).toBeNull();
  });
});

// ─── 10. EXPANDED OLD OFFICE LAPTOP FULL PIPELINE MATRIX ───────────────────

describe('Old office laptop full pipeline matrix — with network + audio', () => {
  interface OfficeLaptopFixture {
    name: string;
    cpu: string;
    expectedGen: string;
    model: string;
    gpu: { name: string; vendorId: string; deviceId: string };
    audio: { vendorId: string; deviceId: string; expectedCodec: string };
    ethernet: { vendorId: string; deviceId: string; expectedFamily: string };
    wifi: { vendorId: string; deviceId: string; expectedFamily: string };
  }

  const fixtures: OfficeLaptopFixture[] = [
    {
      name: 'HP ProBook 450 G2 (Haswell)',
      cpu: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      expectedGen: 'Haswell',
      model: 'HP ProBook 450 G2',
      gpu: { name: 'Microsoft Basic Display Adapter', vendorId: '8086', deviceId: '0a16' },
      audio: { vendorId: '10ec', deviceId: '0282', expectedCodec: 'ALC282' },
      ethernet: { vendorId: '8086', deviceId: '155a', expectedFamily: 'Intel I218-LM' },
      wifi: { vendorId: '8086', deviceId: '08b1', expectedFamily: 'Intel Wireless 7260' },
    },
    {
      name: 'ThinkPad T440 (Haswell)',
      cpu: 'Intel(R) Core(TM) i5-4300U CPU @ 1.90GHz',
      expectedGen: 'Haswell',
      model: 'Lenovo ThinkPad T440',
      gpu: { name: 'Intel HD Graphics 4400', vendorId: '8086', deviceId: '0a16' },
      audio: { vendorId: '10ec', deviceId: '0292', expectedCodec: 'ALC292' },
      ethernet: { vendorId: '8086', deviceId: '155a', expectedFamily: 'Intel I218-LM' },
      wifi: { vendorId: '8086', deviceId: '08b1', expectedFamily: 'Intel Wireless 7260' },
    },
    {
      name: 'ThinkPad T450s (Broadwell)',
      cpu: 'Intel(R) Core(TM) i5-5300U CPU @ 2.30GHz',
      expectedGen: 'Broadwell',
      model: 'Lenovo ThinkPad T450s',
      gpu: { name: 'Intel HD Graphics 5500', vendorId: '8086', deviceId: '1616' },
      audio: { vendorId: '10ec', deviceId: '0292', expectedCodec: 'ALC292' },
      ethernet: { vendorId: '8086', deviceId: '15a2', expectedFamily: 'Intel I218-LM' },
      wifi: { vendorId: '8086', deviceId: '095a', expectedFamily: 'Intel Wireless 7265' },
    },
    {
      name: 'Dell Latitude E7450 (Broadwell)',
      cpu: 'Intel(R) Core(TM) i7-5600U CPU @ 2.60GHz',
      expectedGen: 'Broadwell',
      model: 'Dell Latitude E7450',
      gpu: { name: 'Intel HD Graphics 5500', vendorId: '8086', deviceId: '1616' },
      audio: { vendorId: '10ec', deviceId: '0255', expectedCodec: 'ALC255' },
      ethernet: { vendorId: '8086', deviceId: '15a2', expectedFamily: 'Intel I218-LM' },
      wifi: { vendorId: '8086', deviceId: '095a', expectedFamily: 'Intel Wireless 7265' },
    },
    {
      name: 'HP EliteBook 840 G3 (Skylake)',
      cpu: 'Intel(R) Core(TM) i5-6300U CPU @ 2.40GHz',
      expectedGen: 'Skylake',
      model: 'HP EliteBook 840 G3',
      gpu: { name: 'Intel HD Graphics 520', vendorId: '8086', deviceId: '1916' },
      audio: { vendorId: '14f1', deviceId: '5098', expectedCodec: 'Conexant' },
      ethernet: { vendorId: '8086', deviceId: '156f', expectedFamily: 'Intel I219-LM' },
      wifi: { vendorId: '8086', deviceId: '24f3', expectedFamily: 'Intel Wireless 8260' },
    },
    {
      name: 'Dell Latitude 5480 (Kaby Lake)',
      cpu: 'Intel(R) Core(TM) i5-7300U CPU @ 2.60GHz',
      expectedGen: 'Kaby Lake',
      model: 'Dell Latitude 5480',
      gpu: { name: 'Intel HD Graphics 620', vendorId: '8086', deviceId: '5916' },
      audio: { vendorId: '10ec', deviceId: '0256', expectedCodec: 'ALC256' },
      ethernet: { vendorId: '8086', deviceId: '15b7', expectedFamily: 'Intel I219-LM' },
      wifi: { vendorId: '8086', deviceId: '24fd', expectedFamily: 'Intel Wireless 8265' },
    },
    {
      name: 'ThinkPad T480 (Coffee Lake)',
      cpu: 'Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz',
      expectedGen: 'Coffee Lake',
      model: 'Lenovo ThinkPad T480',
      gpu: { name: 'Intel UHD Graphics 620', vendorId: '8086', deviceId: '5917' },
      audio: { vendorId: '10ec', deviceId: '0285', expectedCodec: 'ALC285' },
      ethernet: { vendorId: '8086', deviceId: '15bb', expectedFamily: 'Intel I219-LM' },
      wifi: { vendorId: '8086', deviceId: '2526', expectedFamily: 'Intel Wireless 9260' },
    },
    {
      name: 'ThinkPad X230 (Ivy Bridge)',
      cpu: 'Intel(R) Core(TM) i5-3320M CPU @ 2.60GHz',
      expectedGen: 'Ivy Bridge',
      model: 'Lenovo ThinkPad X230',
      gpu: { name: 'Intel HD Graphics 4000', vendorId: '8086', deviceId: '0166' },
      audio: { vendorId: '10ec', deviceId: '0269', expectedCodec: 'ALC269' },
      ethernet: { vendorId: '8086', deviceId: '1502', expectedFamily: 'Intel 82579LM' },
      wifi: { vendorId: '8086', deviceId: '08b1', expectedFamily: 'Intel Wireless 7260' },
    },
    {
      name: 'HP ProBook 640 G1 (Haswell, generic driver)',
      cpu: 'Intel(R) Core(TM) i5-4200M CPU @ 2.50GHz',
      expectedGen: 'Haswell',
      model: 'HP ProBook 640 G1',
      gpu: { name: 'Microsoft Basic Display Adapter', vendorId: '8086', deviceId: '0416' },
      audio: { vendorId: '10ec', deviceId: '0282', expectedCodec: 'ALC282' },
      ethernet: { vendorId: '8086', deviceId: '153a', expectedFamily: 'Intel I217-LM' },
      wifi: { vendorId: '8086', deviceId: '08b1', expectedFamily: 'Intel Wireless 7260' },
    },
    {
      name: 'Dell Latitude E6430 (Ivy Bridge)',
      cpu: 'Intel(R) Core(TM) i5-3340M CPU @ 2.70GHz',
      expectedGen: 'Ivy Bridge',
      model: 'Dell Latitude E6430',
      gpu: { name: 'Intel HD Graphics 4000', vendorId: '8086', deviceId: '0166' },
      audio: { vendorId: '10ec', deviceId: '0269', expectedCodec: 'ALC269' },
      ethernet: { vendorId: '8086', deviceId: '1502', expectedFamily: 'Intel 82579LM' },
      wifi: { vendorId: '8086', deviceId: '08b1', expectedFamily: 'Intel Wireless 7260' },
    },
    {
      name: 'HP EliteBook 850 G5 (Coffee Lake)',
      cpu: 'Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz',
      expectedGen: 'Coffee Lake',
      model: 'HP EliteBook 850 G5',
      gpu: { name: 'Intel UHD Graphics 620', vendorId: '8086', deviceId: '5917' },
      audio: { vendorId: '14f1', deviceId: '510f', expectedCodec: 'Conexant' },
      ethernet: { vendorId: '8086', deviceId: '15bb', expectedFamily: 'Intel I219-LM' },
      wifi: { vendorId: '8086', deviceId: '24fd', expectedFamily: 'Intel Wireless 8265' },
    },
    {
      name: 'ThinkPad L440 (Haswell, Realtek Ethernet)',
      cpu: 'Intel(R) Core(TM) i5-4300M CPU @ 2.60GHz',
      expectedGen: 'Haswell',
      model: 'Lenovo ThinkPad L440',
      gpu: { name: 'Intel HD Graphics 4600', vendorId: '8086', deviceId: '0416' },
      audio: { vendorId: '10ec', deviceId: '0292', expectedCodec: 'ALC292' },
      ethernet: { vendorId: '10ec', deviceId: '8168', expectedFamily: 'Realtek RTL8111' },
      wifi: { vendorId: '8086', deviceId: '08b1', expectedFamily: 'Intel Wireless 7260' },
    },
  ];

  for (const f of fixtures) {
    describe(f.name, () => {
      it('CPU generation', () => {
        expect(detectCpuGeneration(f.cpu)).toBe(f.expectedGen);
      });

      it('detected as laptop', () => {
        expect(inferLaptopFormFactor({ cpuName: f.cpu, modelName: f.model })).toBe(true);
      });

      it('audio codec detection', () => {
        const codec = resolveAudioCodec(f.audio.vendorId, f.audio.deviceId);
        expect(codec).not.toBeNull();
        expect(codec).toContain(f.audio.expectedCodec);
      });

      it('Ethernet adapter detection', () => {
        const result = resolveNetworkAdapter(f.ethernet.vendorId, f.ethernet.deviceId, 'Ethernet');
        expect(result.adapterFamily).toBe(f.ethernet.expectedFamily);
        expect(result.type).toBe('ethernet');
      });

      it('Wi-Fi adapter detection', () => {
        const result = resolveNetworkAdapter(f.wifi.vendorId, f.wifi.deviceId, 'Wireless');
        expect(result.adapterFamily).toBe(f.wifi.expectedFamily);
        expect(result.type).toBe('wifi');
      });

      it('full profile mapping produces valid profile', () => {
        const hw: DetectedHardware = {
          cpu: { name: f.cpu, vendor: 'GenuineIntel', vendorName: 'Intel', confidence: 'detected' },
          gpus: [{ ...f.gpu, vendorName: 'Intel', confidence: 'detected' as const }],
          primaryGpu: { ...f.gpu, vendorName: 'Intel', confidence: 'detected' as const },
          motherboardVendor: f.model.split(' ')[0],
          motherboardModel: f.model,
          ramBytes: 8 * 1024 * 1024 * 1024,
          coreCount: 2,
          isLaptop: true,
          isVM: false,
          audioDevices: [{
            name: 'Audio Device',
            vendorId: f.audio.vendorId,
            deviceId: f.audio.deviceId,
            codecName: resolveAudioCodec(f.audio.vendorId, f.audio.deviceId),
            confidence: 'detected',
          }],
          networkDevices: [
            {
              name: 'Ethernet',
              vendorId: f.ethernet.vendorId,
              deviceId: f.ethernet.deviceId,
              ...resolveNetworkAdapter(f.ethernet.vendorId, f.ethernet.deviceId, 'Ethernet'),
              confidence: 'detected' as const,
            },
            {
              name: 'Wi-Fi',
              vendorId: f.wifi.vendorId,
              deviceId: f.wifi.deviceId,
              ...resolveNetworkAdapter(f.wifi.vendorId, f.wifi.deviceId, 'Wireless'),
              confidence: 'detected' as const,
            },
          ],
          inputDevices: [],
        };

        // Apply GPU name enhancement for generic adapter names
        const enhanced: DetectedHardware = {
          ...hw,
          gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
        };
        enhanced.primaryGpu = enhanced.gpus[0];

        const profile = mapDetectedToProfile(enhanced);
        expect(profile.generation).toBe(f.expectedGen);
        expect(profile.architecture).toBe('Intel');
        expect(profile.isLaptop).toBe(true);
        expect(profile.gpu).toContain('Intel');
        expect(profile.nicChipset).toBe(f.ethernet.expectedFamily);
        expect(profile.wifiChipset).toBe(f.wifi.expectedFamily);
      });
    });
  }
});

// ─── 11. NETWORK INTERPRETATION IN FULL PIPELINE ───────────────────────────

describe('Network interpretation in hardware interpretation pipeline', () => {
  it('detected ethernet and wifi show in interpretation', () => {
    const hw = makeHPProBook450G2();
    const enhanced: DetectedHardware = {
      ...hw,
      gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
    };
    enhanced.primaryGpu = enhanced.gpus[0];
    const interp = interpretHardware(enhanced);
    expect(interp.network.ethernet.value).toBe('Intel I218-LM');
    expect(interp.network.ethernet.basis).toBe('detected');
    expect(interp.network.wifi.value).toBe('Intel Wireless 7260');
    expect(interp.network.wifi.basis).toBe('detected');
  });

  it('no network devices show as not detected', () => {
    const hw: DetectedHardware = {
      ...makeHPProBook450G2(),
      networkDevices: [],
    };
    const interp = interpretHardware(hw);
    expect(interp.network.ethernet.value).toBe('Not detected');
    expect(interp.network.ethernet.basis).toBe('unknown');
    expect(interp.network.wifi.value).toBe('Not detected');
    expect(interp.network.wifi.basis).toBe('unknown');
  });

  it('ethernet only (no wifi) shows partial detection', () => {
    const hw: DetectedHardware = {
      ...makeHPProBook450G2(),
      networkDevices: [{
        name: 'Intel Ethernet',
        vendorId: '8086',
        deviceId: '155a',
        vendorName: 'Intel',
        adapterFamily: 'Intel I218-LM',
        type: 'ethernet',
        confidence: 'detected',
      }],
    };
    const interp = interpretHardware(hw);
    expect(interp.network.ethernet.value).toBe('Intel I218-LM');
    expect(interp.network.wifi.value).toBe('Not detected');
  });
});

// ─── 12. I2C VS PS2 DETECTION ──────────────────────────────────────────────

describe('I2C device ID detection', () => {
  it('PNP0C50 (HID-over-I2C standard) → I2C', () => {
    expect(isI2CDeviceId('ACPI\\PNP0C50\\1')).toBe(true);
  });

  it('INT33C3 (Intel I2C controller, Haswell) → I2C', () => {
    expect(isI2CDeviceId('ACPI\\INT33C3\\0')).toBe(true);
  });

  it('INT3432 (Intel I2C controller, Skylake) → I2C', () => {
    expect(isI2CDeviceId('ACPI\\INT3432\\0')).toBe(true);
  });

  it('INT3433 (Intel I2C controller, Skylake) → I2C', () => {
    expect(isI2CDeviceId('ACPI\\INT3433\\0')).toBe(true);
  });

  it('MSFT0001 (Microsoft I2C HID minidriver) → I2C', () => {
    expect(isI2CDeviceId('ACPI\\MSFT0001\\0')).toBe(true);
  });

  it('path containing I2C in double-backslash PnP format → I2C', () => {
    // Windows PnP IDs use double-backslash in raw PowerShell output
    expect(isI2CDeviceId('ACPI\\\\ELAN_I2C_DEVICE\\\\0')).toBe(true);
  });

  it('regular HID device (no I2C signature) → not I2C', () => {
    expect(isI2CDeviceId('HID\\VID_046D&PID_C52B\\1')).toBe(false);
  });

  it('PS2 keyboard device → not I2C', () => {
    expect(isI2CDeviceId('ACPI\\PNP0303\\0')).toBe(false);
  });

  it('USB mouse device → not I2C', () => {
    expect(isI2CDeviceId('USB\\VID_0461&PID_4D64\\5')).toBe(false);
  });
});

describe('Input stack derivation', () => {
  it('laptop with I2C device → i2c', () => {
    const devices: InputDevice[] = [
      { name: 'I2C HID Device', pnpDeviceId: 'ACPI\\PNP0C50\\1', isI2C: true, confidence: 'detected' },
      { name: 'HID Keyboard', pnpDeviceId: 'ACPI\\PNP0303\\0', isI2C: false, confidence: 'detected' },
    ];
    expect(deriveInputStack(devices, true)).toBe('i2c');
  });

  it('laptop with only PS2/USB HID devices → ps2', () => {
    const devices: InputDevice[] = [
      { name: 'HID Keyboard', pnpDeviceId: 'ACPI\\PNP0303\\0', isI2C: false, confidence: 'detected' },
      { name: 'USB Mouse', pnpDeviceId: 'USB\\VID_0461\\0', isI2C: false, confidence: 'detected' },
    ];
    expect(deriveInputStack(devices, true)).toBe('ps2');
  });

  it('laptop with no HID devices → unknown (conservative)', () => {
    expect(deriveInputStack([], true)).toBe('unknown');
  });

  it('desktop → always unknown regardless of devices', () => {
    const devices: InputDevice[] = [
      { name: 'I2C HID Device', pnpDeviceId: 'ACPI\\PNP0C50\\1', isI2C: true, confidence: 'detected' },
    ];
    expect(deriveInputStack(devices, false)).toBe('unknown');
  });
});

// ─── 13. VOODOOI2C-AWARE LAPTOP PATH ───────────────────────────────────────

describe('VoodooI2C-aware laptop path in config generator', () => {
  it('I2C laptop gets VoodooI2C + VoodooI2CHID + SSDT-GPIO', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Skylake',
      isLaptop: true,
      inputStack: 'i2c',
    }));
    expect(kexts).toContain('VoodooI2C.kext');
    expect(kexts).toContain('VoodooI2CHID.kext');
    expect(kexts).toContain('VoodooPS2Controller.kext'); // keyboard still PS2
    expect(ssdts).toContain('SSDT-GPIO.aml');
  });

  it('PS2 laptop does NOT get VoodooI2C or SSDT-GPIO', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Skylake',
      isLaptop: true,
      inputStack: 'ps2',
    }));
    expect(kexts).toContain('VoodooPS2Controller.kext');
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(kexts).not.toContain('VoodooI2CHID.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
  });

  it('unknown inputStack stays conservative PS2 path', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      inputStack: 'unknown',
    }));
    expect(kexts).toContain('VoodooPS2Controller.kext');
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
  });

  it('undefined inputStack (legacy profile) stays conservative PS2', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Haswell',
      isLaptop: true,
    }));
    expect(kexts).toContain('VoodooPS2Controller.kext');
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
  });

  it('I2C on Sandy Bridge stays PS2 (pre-I2C era)', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Sandy Bridge',
      isLaptop: true,
      inputStack: 'i2c',
    }));
    // Sandy Bridge does not support I2C path in generator
    expect(kexts).toContain('VoodooPS2Controller.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
    expect(ssdts).toContain('SSDT-XOSI.aml');
  });

  it('desktop never gets VoodooI2C regardless of inputStack', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: false,
      inputStack: 'i2c',
    }));
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(kexts).not.toContain('VoodooI2CHID.kext');
    expect(kexts).not.toContain('VoodooPS2Controller.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
  });

  it('old office laptop (HP ProBook 450 G2) stays PS2 with no SSDT-GPIO', () => {
    const hw = makeHPProBook450G2();
    const enhanced: DetectedHardware = {
      ...hw,
      gpus: enhanceGenericGpuNames(hw.gpus, hw.cpu.name),
    };
    enhanced.primaryGpu = enhanced.gpus[0];
    const profile = mapDetectedToProfile(enhanced);
    expect(profile.inputStack).toBe('unknown'); // no HID devices in fixture
    const { kexts, ssdts } = getRequiredResources(profile);
    expect(kexts).toContain('VoodooPS2Controller.kext');
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
  });
});

// ─── 14. VOODOOI2C SOURCE AND BUNDLE PATH COMPLETION ────────────────────────

describe('VoodooI2C BundlePath and plugin structure', () => {
  it('VoodooI2CHID resolves to nested plugin BundlePath', () => {
    expect(resolveKextBundlePath('VoodooI2CHID.kext'))
      .toBe('VoodooI2C.kext/Contents/PlugIns/VoodooI2CHID.kext');
  });

  it('VoodooI2C resolves to top-level BundlePath', () => {
    expect(resolveKextBundlePath('VoodooI2C.kext')).toBe('VoodooI2C.kext');
  });

  it('regular kexts are unaffected by plugin path resolution', () => {
    expect(resolveKextBundlePath('Lilu.kext')).toBe('Lilu.kext');
    expect(resolveKextBundlePath('VoodooPS2Controller.kext')).toBe('VoodooPS2Controller.kext');
    expect(resolveKextBundlePath('AppleALC.kext')).toBe('AppleALC.kext');
  });

  it('VoodooI2CHID has correct ExecutablePath', () => {
    expect(resolveKextExecutablePath('VoodooI2CHID.kext')).toBe('Contents/MacOS/VoodooI2CHID');
  });

  it('VoodooI2C has correct ExecutablePath', () => {
    expect(resolveKextExecutablePath('VoodooI2C.kext')).toBe('Contents/MacOS/VoodooI2C');
  });

  it('codeless kexts have empty ExecutablePath', () => {
    expect(resolveKextExecutablePath('AppleMCEReporterDisabler.kext')).toBe('');
  });

  it('PLUGIN_KEXT_PARENTS maps VoodooI2CHID → VoodooI2C', () => {
    expect(PLUGIN_KEXT_PARENTS['VoodooI2CHID.kext']).toBe('VoodooI2C.kext');
  });
});

describe('VoodooI2C config.plist generation correctness', () => {
  it('I2C laptop config.plist contains nested VoodooI2CHID BundlePath', () => {
    const profile = fakeProfile({
      generation: 'Skylake',
      isLaptop: true,
      inputStack: 'i2c',
    });
    const plist = generateConfigPlist(profile);
    // VoodooI2C as top-level
    expect(plist).toContain('<string>VoodooI2C.kext</string>');
    // VoodooI2CHID as nested plugin path
    expect(plist).toContain('<string>VoodooI2C.kext/Contents/PlugIns/VoodooI2CHID.kext</string>');
    // Correct executable for the plugin
    expect(plist).toContain('<string>Contents/MacOS/VoodooI2CHID</string>');
    // SSDT-GPIO must be present
    expect(plist).toContain('SSDT-GPIO.aml');
  });

  it('PS2 laptop config.plist does NOT contain VoodooI2C entries', () => {
    const profile = fakeProfile({
      generation: 'Skylake',
      isLaptop: true,
      inputStack: 'ps2',
    });
    const plist = generateConfigPlist(profile);
    expect(plist).toContain('<string>VoodooPS2Controller.kext</string>');
    expect(plist).not.toContain('VoodooI2C');
    expect(plist).not.toContain('SSDT-GPIO');
  });

  it('unknown inputStack laptop config.plist stays PS2-only', () => {
    const profile = fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      inputStack: 'unknown',
    });
    const plist = generateConfigPlist(profile);
    expect(plist).toContain('<string>VoodooPS2Controller.kext</string>');
    expect(plist).not.toContain('VoodooI2C');
  });
});

describe('VoodooI2C end-to-end agreement', () => {
  it('generator + source registry agree: I2C path kexts are all source-backed', () => {
    const { kexts } = getRequiredResources(fakeProfile({
      generation: 'Kaby Lake',
      isLaptop: true,
      inputStack: 'i2c',
    }));
    // VoodooI2C and VoodooI2CHID should both be in the kext list
    expect(kexts).toContain('VoodooI2C.kext');
    expect(kexts).toContain('VoodooI2CHID.kext');
    // VoodooI2CHID ships inside VoodooI2C release — only VoodooI2C needs a download entry
    // VoodooPS2Controller still present for keyboard
    expect(kexts).toContain('VoodooPS2Controller.kext');
  });

  it('PS2 path does NOT include VoodooI2C kexts', () => {
    const { kexts, ssdts } = getRequiredResources(fakeProfile({
      generation: 'Kaby Lake',
      isLaptop: true,
      inputStack: 'ps2',
    }));
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(kexts).not.toContain('VoodooI2CHID.kext');
    expect(ssdts).not.toContain('SSDT-GPIO.aml');
  });

  it('desktop never gets VoodooI2C regardless of inputStack', () => {
    const { kexts } = getRequiredResources(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: false,
      inputStack: 'i2c',
    }));
    expect(kexts).not.toContain('VoodooI2C.kext');
    expect(kexts).not.toContain('VoodooI2CHID.kext');
  });
});

// ─── 15. HEADLESS IGPU / LAPTOP DEFENSE-IN-DEPTH ───────────────────────────

describe('Headless iGPU defense — laptops never headless', () => {
  it('laptop with AMD dGPU still uses display ig-platform-id, not headless', () => {
    // HP ProBook 450 G2 scenario: Haswell laptop + AMD R5 M255 dGPU
    const profile = fakeProfile({
      generation: 'Haswell',
      isLaptop: true,
      gpu: 'Intel HD Graphics 4400 / AMD Radeon R5 M255',
      gpuDevices: [
        { name: 'Intel HD Graphics 4400', vendorName: 'Intel', vendorId: '8086', deviceId: '0a16' },
        { name: 'AMD Radeon R5 M255', vendorName: 'AMD', vendorId: '1002', deviceId: '6665' },
      ],
    });
    const plist = generateConfigPlist(profile);
    // Must use Haswell laptop ig-platform-id (0x0A260006 = BgAmCg==), NOT headless
    expect(plist).toContain('BgAmCg==');
    // Must NOT use Haswell headless ig-platform-id (0x04120004 = BAASBA==)
    expect(plist).not.toContain('BAASBA==');
  });

  it('laptop with unsupported NVIDIA dGPU still uses display ig-platform-id', () => {
    const profile = fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      gpu: 'Intel UHD 630 / NVIDIA GeForce GTX 1050',
      gpuDevices: [
        { name: 'Intel UHD 630' },
        { name: 'NVIDIA GeForce GTX 1050' },
      ],
    });
    const plist = generateConfigPlist(profile);
    // Must use Coffee Lake laptop ig-platform-id (0x3EA50009 = CQClPg==)
    expect(plist).toContain('CQClPg==');
  });

  it('desktop with supported AMD dGPU uses headless ig-platform-id', () => {
    const profile = fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: false,
      gpu: 'Intel UHD 630 / AMD Radeon RX 580',
      gpuDevices: [
        { name: 'Intel UHD 630' },
        { name: 'AMD Radeon RX 580' },
      ],
    });
    const plist = generateConfigPlist(profile);
    // Desktop with supported dGPU: headless Coffee Lake (0x3E910003 = AwCRPg==)
    expect(plist).toContain('AwCRPg==');
  });
});

describe('agdpmod=pikera correctness', () => {
  it('laptop with old AMD dGPU does NOT get agdpmod=pikera', () => {
    const profile = fakeProfile({
      generation: 'Haswell',
      isLaptop: true,
      smbios: 'MacBookPro11,4',
      gpu: 'Intel HD Graphics 4400 / AMD Radeon R5 M255',
      gpuDevices: [
        { name: 'Intel HD Graphics 4400' },
        { name: 'AMD Radeon R5 M255' },
      ],
    });
    const plist = generateConfigPlist(profile);
    expect(plist).not.toContain('agdpmod=pikera');
  });

  it('desktop iMac with supported AMD RX 580 DOES get agdpmod=pikera', () => {
    const profile = fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: false,
      smbios: 'iMac20,1',
      gpu: 'Intel UHD 630 / AMD Radeon RX 580',
      gpuDevices: [
        { name: 'Intel UHD 630' },
        { name: 'AMD Radeon RX 580' },
      ],
    });
    const plist = generateConfigPlist(profile);
    expect(plist).toContain('agdpmod=pikera');
  });

  it('desktop iMac with unsupported old AMD R7 does NOT get agdpmod=pikera', () => {
    const profile = fakeProfile({
      generation: 'Haswell',
      isLaptop: false,
      smbios: 'iMac14,2',
      gpu: 'Intel HD Graphics 4600 / AMD Radeon R7 250',
      gpuDevices: [
        { name: 'Intel HD Graphics 4600' },
        { name: 'AMD Radeon R7 250' },
      ],
    });
    const plist = generateConfigPlist(profile);
    expect(plist).not.toContain('agdpmod=pikera');
  });
});

// ─── 16. UPSTREAM LAPTOP CLASSIFICATION: SIGNAL FUSION ──────────────────────

describe('Laptop classification — signal fusion and resilience', () => {
  // HP ProBook 450 G2 repro: all fragile signals fail, but U-suffix alone saves it
  it('HP ProBook 450 G2: U-suffix alone classifies as laptop even with no chassis/model/battery', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      chassisTypes: [],        // query failed, empty
      modelName: '',           // query timed out
      batteryPresent: false,   // query timed out
    })).toBe(true);
  });

  it('U-suffix CPUs are always laptop', () => {
    const uCpus = [
      'Intel(R) Core(TM) i5-4200U CPU @ 1.60GHz',
      'Intel(R) Core(TM) i7-5500U CPU @ 2.40GHz',
      'Intel(R) Core(TM) i5-6200U CPU @ 2.30GHz',
      'Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz',
      'Intel(R) Core(TM) i5-10210U CPU @ 1.60GHz',
    ];
    for (const cpu of uCpus) {
      expect(inferLaptopFormFactor({ cpuName: cpu }), cpu).toBe(true);
    }
  });

  it('Y-suffix CPUs are always laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) m3-7Y30 CPU @ 1.00GHz',
    })).toBe(true);
  });

  it('chassis fallback of empty does not force desktop', () => {
    // Legacy scanner with failed chassis query (fallback empty instead of "3")
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      chassisTypes: [],
    })).toBe(true);
  });

  it('chassis type 3 (Desktop) with U-suffix CPU still classifies as laptop', () => {
    // Defense: even if chassis query wrongly returns desktop type,
    // U-suffix CPU is definitive evidence
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      chassisTypes: [3],  // Desktop chassis — wrong but U overrides
    })).toBe(true);
  });

  it('H-suffix CPU + battery = laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz',
      batteryPresent: true,
    })).toBe(true);
  });

  it('H-suffix CPU + OEM laptop model = laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz',
      modelName: 'Dell Latitude 5501',
    })).toBe(true);
  });

  it('H-suffix CPU alone is NOT laptop (could be NUC/mini)', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz',
    })).toBe(false);
  });

  it('model name "HP ProBook 450 G2" alone is laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Unknown CPU',
      modelName: 'HP ProBook 450 G2',
    })).toBe(true);
  });

  it('model name "Lenovo ThinkPad T440" alone is laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Unknown CPU',
      modelName: '20B7S0F200',  // ThinkPad model codes are opaque
    })).toBe(false); // opaque model code is NOT enough
    // But with manufacturer:
    expect(inferLaptopFormFactor({
      cpuName: 'Unknown CPU',
      modelName: 'ThinkPad T440',
    })).toBe(true);
  });

  it('mobile GPU + battery = laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Unknown CPU',
      batteryPresent: true,
      gpuName: 'AMD Radeon R5 M255',
    })).toBe(true);
  });

  it('mobile GPU + mobile CPU suffix = laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4710HQ CPU @ 2.50GHz',
      gpuName: 'NVIDIA GeForce GTX 850M',
    })).toBe(true);
  });

  it('real desktop CPU + no other signals = desktop (correct)', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-10700K CPU @ 3.80GHz',
    })).toBe(false);
  });

  it('real desktop CPU + desktop chassis = desktop (correct)', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-10700K CPU @ 3.80GHz',
      chassisTypes: [3],
    })).toBe(false);
  });

  it('AMD Ryzen desktop CPU = desktop (correct)', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'AMD Ryzen 7 5800X 8-Core Processor',
    })).toBe(false);
  });
});

describe('Legacy scanner HP ProBook 450 G2 worst-case', () => {
  it('with all queries failing (empty fallbacks), U-suffix CPU still saves laptop', () => {
    // Simulate: chassis='', model='', battery=false, manufacturer='Unknown'
    // Only CPU name survived: i7-4510U
    const result = inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      chassisTypes: [],
      modelName: '',
      batteryPresent: false,
      manufacturer: 'Unknown',
      gpuName: 'Unknown GPU',
    });
    expect(result).toBe(true);
  });

  it('with only model surviving, ProBook hint saves laptop', () => {
    const result = inferLaptopFormFactor({
      cpuName: 'Unknown CPU',
      chassisTypes: [],
      modelName: 'HP ProBook 450 G2',
      batteryPresent: false,
    });
    expect(result).toBe(true);
  });

  it('with all signals present, definitely laptop', () => {
    const result = inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
      chassisTypes: [10],
      modelName: 'HP ProBook 450 G2',
      batteryPresent: true,
      manufacturer: 'Hewlett-Packard',
      gpuName: 'Intel HD Graphics Family / AMD Radeon R5 M255',
    });
    expect(result).toBe(true);
  });
});

// ─── 17. NEW SCANNER PARTIAL RESULT PRESERVATION ────────────────────────────

describe('buildPartialDetectedHardware — fail-soft scanner', () => {
  it('builds valid DetectedHardware from just CPU data', () => {
    const result = buildPartialDetectedHardware({
      cpu: {
        name: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
        vendor: 'GenuineIntel',
        vendorName: 'Intel',
        confidence: 'partially-detected',
      },
    });
    expect(result.cpu.name).toBe('Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz');
    expect(result.cpu.vendorName).toBe('Intel');
    expect(result.gpus).toHaveLength(1);
    expect(result.gpus[0].name).toBe('Unknown GPU');
    expect(result.ramBytes).toBeGreaterThan(0);
    expect(result.audioDevices).toEqual([]);
    expect(result.networkDevices).toEqual([]);
    expect(result.inputDevices).toEqual([]);
  });

  it('preserves GPU data when provided', () => {
    const result = buildPartialDetectedHardware({
      gpus: [{
        name: 'Intel HD Graphics 4400',
        vendorId: '8086',
        deviceId: '0a16',
        vendorName: 'Intel',
        confidence: 'detected',
      }],
    });
    expect(result.gpus[0].name).toBe('Intel HD Graphics 4400');
    expect(result.primaryGpu.vendorName).toBe('Intel');
  });

  it('preserves isLaptop when provided', () => {
    const result = buildPartialDetectedHardware({ isLaptop: true });
    expect(result.isLaptop).toBe(true);
  });

  it('defaults isLaptop to false when not provided', () => {
    const result = buildPartialDetectedHardware({});
    expect(result.isLaptop).toBe(false);
  });

  it('builds complete enough profile for mapDetectedToProfile', () => {
    const result = buildPartialDetectedHardware({
      cpu: {
        name: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
        vendor: 'GenuineIntel',
        vendorName: 'Intel',
        confidence: 'partially-detected',
      },
      isLaptop: true,
    });
    // This should not throw
    const profile = mapDetectedToProfile(result);
    expect(profile.generation).toBe('Haswell');
    expect(profile.isLaptop).toBe(true);
    expect(profile.architecture).toBe('Intel');
  });

  it('HP ProBook 450 G2 partial recovery: CPU only yields Haswell laptop', () => {
    // Simulate: primary scanner timed out, only CPU name from os.cpus() survived
    const result = buildPartialDetectedHardware({
      cpu: {
        name: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
        vendor: 'GenuineIntel',
        vendorName: 'Intel',
        confidence: 'partially-detected',
      },
      isLaptop: true, // U-suffix → laptop via formFactor
    });
    const profile = mapDetectedToProfile(result);
    expect(profile.generation).toBe('Haswell');
    expect(profile.isLaptop).toBe(true);
    expect(profile.smbios).not.toBe('iMac20,1'); // Must NOT be desktop SMBIOS
    expect(profile.smbios).toContain('MacBook'); // Must be a MacBook variant
  });

  it('partial result with audio/network/input empty still produces valid profile', () => {
    const result = buildPartialDetectedHardware({
      cpu: {
        name: 'Intel(R) Core(TM) i5-6200U CPU @ 2.30GHz',
        vendor: 'GenuineIntel',
        vendorName: 'Intel',
        confidence: 'detected',
      },
      gpus: [{
        name: 'Intel HD Graphics 520',
        vendorId: '8086',
        deviceId: '1916',
        vendorName: 'Intel',
        confidence: 'detected',
      }],
      isLaptop: true,
    });
    const profile = mapDetectedToProfile(result);
    expect(profile.generation).toBe('Skylake');
    expect(profile.isLaptop).toBe(true);
    // CPU and GPU both detected → confidence is high despite missing audio/network
    // (confidence is based on CPU + GPU detection, not peripheral subsystems)
  });
});
