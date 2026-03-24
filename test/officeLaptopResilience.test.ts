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
  type GpuDevice,
  type AudioDevice,
  type NetworkDevice,
} from '../electron/hardwareDetect.js';
import { detectCpuGeneration, detectArchitecture, mapDetectedToProfile } from '../electron/hardwareMapper.js';
import { classifyGpu, type HardwareGpuDeviceSummary } from '../electron/hackintoshRules.js';
import { interpretHardware } from '../electron/hardwareInterpret.js';
import { inferLaptopFormFactor } from '../electron/formFactor.js';
import { resolveAudioLayoutId } from '../electron/configGenerator.js';
import type { DetectedHardware } from '../electron/hardwareDetect.js';

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

  it('mobile CPU suffix without battery does NOT assume laptop', () => {
    expect(inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-4510U CPU @ 2.00GHz',
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
