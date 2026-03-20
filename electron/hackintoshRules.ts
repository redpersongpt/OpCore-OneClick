export interface HardwareGpuDeviceSummary {
  name: string;
  vendorName?: string;
  vendorId?: string | null;
  deviceId?: string | null;
}

export interface MacOSVersionOption {
  id: string;
  name: string;
  icon: string;
  numeric: number;
}

export type GpuSupportTier = 'supported' | 'supported_with_limit' | 'partial_support' | 'unsupported' | 'unknown';

export interface GpuAssessment {
  name: string;
  vendor: 'Intel' | 'AMD' | 'NVIDIA' | 'Unknown';
  tier: GpuSupportTier;
  maxMacOSVersion: number | null;
  notes: string[];
  requiresDisable: boolean;
  requiresPikera: boolean;
  requiresNootRX: boolean;
  requiresNootedRed: boolean;
  isLikelyDiscrete: boolean;
}

export const MACOS_VERSIONS: MacOSVersionOption[] = [
  { id: '26', name: 'macOS Tahoe 26', icon: 'tahoe', numeric: 26 },
  { id: '15', name: 'macOS Sequoia 15', icon: 'sequoia', numeric: 15 },
  { id: '14', name: 'macOS Sonoma 14', icon: 'sonoma', numeric: 14 },
  { id: '13', name: 'macOS Ventura 13', icon: 'ventura', numeric: 13 },
  { id: '12', name: 'macOS Monterey 12', icon: 'monterey', numeric: 12 },
  { id: '11', name: 'macOS Big Sur 11', icon: 'big-sur', numeric: 11 },
  { id: '10.15', name: 'macOS Catalina 10.15', icon: 'catalina', numeric: 10.15 },
  { id: '10.14', name: 'macOS Mojave 10.14', icon: 'mojave', numeric: 10.14 },
  { id: '10.13', name: 'macOS High Sierra 10.13', icon: 'high-sierra', numeric: 10.13 },
];

export function parseMacOSVersion(os: string): number {
  const aliases: Record<string, number> = {
    'high sierra': 10.13,
    'mojave': 10.14,
    'catalina': 10.15,
    'big sur': 11,
    'monterey': 12,
    'ventura': 13,
    'sonoma': 14,
    'sequoia': 15,
    'tahoe': 26,
  };

  const lower = os.toLowerCase();
  for (const [label, numeric] of Object.entries(aliases)) {
    if (lower.includes(label)) return numeric;
  }

  const versionMatch = lower.match(/(\d+(?:\.\d+)?)/);
  return versionMatch ? parseFloat(versionMatch[1]) : 15;
}

export function getEligibleMacOSVersions(maxVersion: number): MacOSVersionOption[] {
  return MACOS_VERSIONS.filter(version => version.numeric <= maxVersion);
}

export function resolveGpuVendor(name: string, vendorHint?: string): GpuAssessment['vendor'] {
  const normalized = `${vendorHint ?? ''} ${name}`.toLowerCase();
  if (
    normalized.includes('nvidia') ||
    normalized.includes('geforce') ||
    normalized.includes('quadro') ||
    normalized.includes('rtx') ||
    normalized.includes('gtx')
  ) {
    return 'NVIDIA';
  }
  if (
    normalized.includes('amd') ||
    normalized.includes('radeon') ||
    normalized.includes(' rx ') ||
    normalized.includes('vega') ||
    normalized.includes('firepro') ||
    normalized.includes('navi')
  ) {
    return 'AMD';
  }
  if (
    normalized.includes('intel') ||
    normalized.includes('iris') ||
    normalized.includes('uhd') ||
    normalized.includes('hd graphics') ||
    normalized.includes('arc')
  ) {
    return 'Intel';
  }
  return 'Unknown';
}

export function splitGpuSummary(gpuSummary: string): HardwareGpuDeviceSummary[] {
  return gpuSummary
    .split(/\s+\/\s+/)
    .map(name => name.trim())
    .filter(Boolean)
    .map(name => ({ name }));
}

export function getProfileGpuDevices(profile: { gpu: string; gpuDevices?: HardwareGpuDeviceSummary[] }): HardwareGpuDeviceSummary[] {
  if (profile.gpuDevices && profile.gpuDevices.length > 0) return profile.gpuDevices;
  return splitGpuSummary(profile.gpu);
}

function assessment(
  name: string,
  vendor: GpuAssessment['vendor'],
  tier: GpuSupportTier,
  maxMacOSVersion: number | null,
  notes: string[] = [],
  overrides: Partial<Pick<GpuAssessment, 'requiresDisable' | 'requiresPikera' | 'requiresNootRX' | 'requiresNootedRed' | 'isLikelyDiscrete'>> = {},
): GpuAssessment {
  return {
    name,
    vendor,
    tier,
    maxMacOSVersion,
    notes,
    requiresDisable: overrides.requiresDisable ?? false,
    requiresPikera: overrides.requiresPikera ?? false,
    requiresNootRX: overrides.requiresNootRX ?? false,
    requiresNootedRed: overrides.requiresNootedRed ?? false,
    isLikelyDiscrete: overrides.isLikelyDiscrete ?? vendor !== 'Intel',
  };
}

export function classifyGpu(device: HardwareGpuDeviceSummary): GpuAssessment {
  const name = device.name.trim() || 'Unknown GPU';
  const lower = name.toLowerCase();
  const vendor = resolveGpuVendor(name, device.vendorName);

  if (vendor === 'NVIDIA') {
    if (
      lower.includes('rtx') ||
      lower.includes('1650') ||
      lower.includes('1660') ||
      lower.includes('turing') ||
      lower.includes('ampere') ||
      lower.includes('ada') ||
      /\b(?:20|30|40)\d{2}\b/.test(lower)
    ) {
      return assessment(name, vendor, 'unsupported', null, [
        'Modern NVIDIA architectures do not have macOS drivers.',
      ], { requiresDisable: true });
    }

    if (
      lower.includes('maxwell') ||
      lower.includes('pascal') ||
      /\b(?:750 ti|950|960|970|980|1050|1060|1070|1080)\b/.test(lower)
    ) {
      return assessment(name, vendor, 'supported_with_limit', 10.13, [
        'Maxwell/Pascal require NVIDIA Web Drivers and are limited to High Sierra.',
      ]);
    }

    if (
      lower.includes('kepler') ||
      /\b(?:710|720|730|740|760|770|780)\b/.test(lower) ||
      lower.includes('quadro k')
    ) {
      return assessment(name, vendor, 'supported_with_limit', 11, [
        'Kepler is capped at Big Sur.',
      ]);
    }

    return assessment(name, vendor, 'unknown', null, [
      'NVIDIA GPU detected but the exact macOS support ceiling could not be classified from the model string.',
    ]);
  }

  if (vendor === 'Intel') {
    if (
      lower.includes('arc') ||
      lower.includes('iris xe') ||
      lower.includes(' xe ') ||
      lower.includes('ice lake g1') ||
      /\b(?:uhd|hd)(?:\s+graphics)?\s*(?:510|610|600|605)\b/.test(lower) ||
      lower.includes('hd 2500')
    ) {
      return assessment(name, vendor, 'unsupported', null, [
        'This Intel graphics class is not supported by macOS.',
      ], { isLikelyDiscrete: false });
    }

    if (lower.includes('hd 4000') || lower.includes('hd graphics 4000')) {
      return assessment(name, vendor, 'supported_with_limit', 11, [
        'HD 4000 is capped at Big Sur.',
      ], { isLikelyDiscrete: false });
    }

    if (
      lower.includes('hd graphics 4400') ||
      lower.includes('hd graphics 4600') ||
      lower.includes('hd graphics 5000') ||
      lower.includes('hd graphics 5500') ||
      lower.includes('hd graphics 6000') ||
      lower.includes('hd 4400') ||
      lower.includes('hd 4600') ||
      lower.includes('hd 5000') ||
      lower.includes('hd 5500') ||
      lower.includes('hd 6000') ||
      lower.includes('iris 5100') ||
      lower.includes('iris 6100') ||
      lower.includes('iris 6200') ||
      lower.includes('iris pro')
    ) {
      return assessment(name, vendor, 'supported_with_limit', 12, [
        'This older Intel iGPU generation should be capped at Monterey for deterministic automation.',
      ], { isLikelyDiscrete: false });
    }

    if (
      lower.includes('hd 520') ||
      lower.includes('hd 530') ||
      lower.includes('uhd 620') ||
      lower.includes('uhd 630') ||
      lower.includes('iris 540') ||
      lower.includes('iris 550') ||
      lower.includes('iris plus') ||
      lower.includes('uhd')
    ) {
      return assessment(name, vendor, 'supported', 26, [], { isLikelyDiscrete: false });
    }

    return assessment(name, vendor, 'unknown', null, [
      'Intel graphics detected but the exact support ceiling needs manual verification.',
    ], { isLikelyDiscrete: false });
  }

  if (vendor === 'AMD') {
    if (
      lower.includes('rx 7600') ||
      lower.includes('rx 7700') ||
      lower.includes('rx 7800') ||
      lower.includes('rx 7900') ||
      lower.includes('w7500') ||
      lower.includes('w7600') ||
      lower.includes('w7700') ||
      lower.includes('w7800') ||
      lower.includes('w7900') ||
      lower.includes('navi 3')
    ) {
      return assessment(name, vendor, 'unsupported', null, [
        'RDNA 3 / Navi 3x remains unsupported in macOS.',
      ]);
    }

    if (
      lower.includes('rx 6300') ||
      lower.includes('rx 6400') ||
      lower.includes('rx 6500') ||
      lower.includes('w6300') ||
      lower.includes('w6400') ||
      lower.includes('navi 24')
    ) {
      return assessment(name, vendor, 'unsupported', null, [
        'Navi 24 remains unsupported in macOS.',
      ]);
    }

    if (
      lower.includes('rx 6700') ||
      lower.includes('rx 6750') ||
      lower.includes('6750 gre') ||
      lower.includes('navi 22')
    ) {
      return assessment(name, vendor, 'partial_support', 15, [
        'Navi 22 requires NootRX and is not a native WhateverGreen path.',
      ], { requiresNootRX: true });
    }

    if (
      lower.includes('rx 5500') ||
      lower.includes('rx 5600') ||
      lower.includes('rx 5700') ||
      lower.includes('rx 6600') ||
      lower.includes('rx 6650') ||
      lower.includes('rx 6800') ||
      lower.includes('rx 6900') ||
      lower.includes('radeon vii') ||
      lower.includes('w5500') ||
      lower.includes('w5700') ||
      lower.includes('w6600') ||
      lower.includes('w6800') ||
      lower.includes('vega') ||
      lower.includes('polaris') ||
      lower.includes('rx 460') ||
      lower.includes('rx 470') ||
      lower.includes('rx 480') ||
      lower.includes('rx 550') ||
      lower.includes('rx 560') ||
      lower.includes('rx 570') ||
      lower.includes('rx 580') ||
      lower.includes('rx 590')
    ) {
      const olderPolarisOrVega =
        lower.includes('rx 460') ||
        lower.includes('rx 470') ||
        lower.includes('rx 480') ||
        lower.includes('rx 550') ||
        lower.includes('rx 560') ||
        lower.includes('rx 570') ||
        lower.includes('rx 580') ||
        lower.includes('rx 590') ||
        lower.includes('vega') ||
        lower.includes('radeon vii');

      return assessment(
        name,
        vendor,
        'supported',
        olderPolarisOrVega ? 26 : 26,
        [],
        { requiresPikera: /rx (?:5500|5600|5700|6600|6650|6800|6900)|w5500|w5700|w6600|w6800/.test(lower) },
      );
    }

    if (
      lower.includes('r9 ') ||
      lower.includes('r7 ') ||
      lower.includes('hd 7') ||
      lower.includes('firepro d') ||
      lower.includes('firepro w')
    ) {
      return assessment(name, vendor, 'supported_with_limit', 12, [
        'Older AMD GCN paths should be capped at Monterey.',
      ]);
    }

    if (
      lower.includes('vega 3') ||
      lower.includes('vega 6') ||
      lower.includes('vega 8') ||
      lower.includes('vega 9') ||
      lower.includes('vega 10') ||
      lower.includes('vega 11') ||
      lower.includes('radeon graphics')
    ) {
      return assessment(name, vendor, 'partial_support', 15, [
        'AMD Vega APUs require NootedRed and remain lower-confidence than native display paths.',
      ], { requiresNootedRed: true, isLikelyDiscrete: false });
    }

    return assessment(name, vendor, 'unknown', null, [
      'AMD GPU detected but the exact macOS support ceiling could not be classified from the model string.',
    ]);
  }

  return assessment(name, 'Unknown', 'unknown', null, [
    'GPU vendor could not be determined.',
  ]);
}

export function getBestSupportedGpuPath(
  devices: HardwareGpuDeviceSummary[],
  targetVersion?: number,
): GpuAssessment | null {
  const assessments = devices.map(classifyGpu);
  const candidates = assessments.filter(assessment => {
    if (assessment.tier === 'unsupported' || assessment.tier === 'unknown') return false;
    if (targetVersion == null || assessment.maxMacOSVersion == null) return true;
    return assessment.maxMacOSVersion >= targetVersion;
  });

  if (candidates.length === 0) return null;

  const scored = [...candidates].sort((left, right) => {
    const rank = (assessment: GpuAssessment): number => {
      if (assessment.tier === 'supported') return 3;
      if (assessment.tier === 'supported_with_limit') return 2;
      if (assessment.tier === 'partial_support') return 1;
      return 0;
    };
    const rankDiff = rank(right) - rank(left);
    if (rankDiff !== 0) return rankDiff;
    const leftMax = left.maxMacOSVersion ?? 0;
    const rightMax = right.maxMacOSVersion ?? 0;
    return rightMax - leftMax;
  });

  return scored[0] ?? null;
}

export function hasUnsupportedDiscreteGpu(devices: HardwareGpuDeviceSummary[]): boolean {
  return devices
    .map(classifyGpu)
    .some(assessment => assessment.isLikelyDiscrete && assessment.tier === 'unsupported');
}

export function hasSupportedDisplayPath(devices: HardwareGpuDeviceSummary[], targetVersion?: number): boolean {
  return getBestSupportedGpuPath(devices, targetVersion) !== null;
}

export function hasMacProEraAmdGpu(gpus: Array<string | HardwareGpuDeviceSummary>): boolean {
  return gpus.some(gpu => {
    const name = typeof gpu === 'string' ? gpu : gpu.name;
    const lower = name.toLowerCase();
    return (
      lower.includes('polaris') ||
      lower.includes('vega') ||
      lower.includes('radeon vii') ||
      lower.includes('rx 460') ||
      lower.includes('rx 470') ||
      lower.includes('rx 480') ||
      lower.includes('rx 550') ||
      lower.includes('rx 560') ||
      lower.includes('rx 570') ||
      lower.includes('rx 580') ||
      lower.includes('rx 590') ||
      lower.includes('rx 5500') ||
      lower.includes('rx 5600') ||
      lower.includes('rx 5700') ||
      lower.includes('rx 6600') ||
      lower.includes('rx 6800') ||
      lower.includes('rx 6900')
    );
  });
}

export function needsNaviPikera(devices: HardwareGpuDeviceSummary[]): boolean {
  return devices.some(device => classifyGpu(device).requiresPikera);
}

export function hasUnsupportedModernNvidia(devices: HardwareGpuDeviceSummary[]): boolean {
  return devices
    .map(classifyGpu)
    .some(assessment => assessment.vendor === 'NVIDIA' && assessment.tier === 'unsupported');
}

export function getGpuCeiling(devices: HardwareGpuDeviceSummary[], targetVersion?: number): number | null {
  const best = getBestSupportedGpuPath(devices, targetVersion);
  return best?.maxMacOSVersion ?? null;
}
