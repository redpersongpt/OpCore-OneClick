import { HardwareProfile } from './configGenerator.js';
import {
  classifyGpu,
  getBestSupportedGpuPath,
  getEligibleMacOSVersions,
  getGpuCeiling,
  getProfileGpuDevices,
  hasSupportedDisplayPath,
  hasUnsupportedDiscreteGpu,
  parseMacOSVersion,
} from './hackintoshRules.js';

export type CompatibilityLevel =
  | 'supported'
  | 'supported_with_warnings'
  | 'partial_support'
  | 'low_confidence'
  | 'blocked';

export type ConfigStrategy =
  | 'canonical'
  | 'conservative'
  | 'blocked';

export interface CompatibilityReport {
  level: CompatibilityLevel;
  strategy: ConfigStrategy;
  confidence: 'high' | 'medium' | 'low';
  explanation: string;
  manualVerificationRequired: boolean;
  isCompatible: boolean;
  maxOSVersion: string;
  eligibleVersions: { id: string; name: string; icon: string }[];
  recommendedVersion: string;
  warnings: string[];
  errors: string[];
  minReqMet: boolean;
}

function capFromCpu(profile: HardwareProfile): number | null {
  const cpu = profile.cpu.toLowerCase();

  if (profile.architecture === 'Intel') {
    if (profile.generation === 'Penryn') return 10.13;
    if (['Sandy Bridge', 'Ivy Bridge', 'Unknown'].includes(profile.generation)) return 12;
    if (['Haswell', 'Broadwell', 'Haswell-E', 'Broadwell-E'].includes(profile.generation)) return 12;
    if (cpu.includes('pentium') || cpu.includes('celeron') || cpu.includes('atom')) return 12;
  }

  return null;
}

function setBlocked(report: CompatibilityReport, explanation: string): CompatibilityReport {
  report.level = 'blocked';
  report.strategy = 'blocked';
  report.isCompatible = false;
  report.explanation = explanation;
  report.eligibleVersions = [];
  report.recommendedVersion = '';
  report.maxOSVersion = 'Blocked';
  return report;
}

export function checkCompatibility(profile: HardwareProfile): CompatibilityReport {
  const report: CompatibilityReport = {
    level: 'supported',
    strategy: 'canonical',
    confidence: profile.scanConfidence || 'low',
    explanation: 'System appears to be a valid OpenCore target.',
    manualVerificationRequired: false,
    isCompatible: true,
    maxOSVersion: 'macOS Tahoe 26',
    eligibleVersions: getEligibleMacOSVersions(26),
    recommendedVersion: 'macOS Sequoia 15',
    warnings: [],
    errors: [],
    minReqMet: true,
  };

  const targetVersion = parseMacOSVersion(profile.targetOS);
  const gpuDevices = getProfileGpuDevices(profile);
  const gpuAssessments = gpuDevices.map(classifyGpu);
  const bestAnyDisplayPath = getBestSupportedGpuPath(gpuDevices);
  const bestSelectedDisplayPath = getBestSupportedGpuPath(gpuDevices, targetVersion);
  const bestDisplayCeiling = getGpuCeiling(gpuDevices);
  const cpuCeiling = capFromCpu(profile);
  const motherboard = profile.motherboard.toLowerCase();
  const cpu = profile.cpu.toLowerCase();

  if (report.confidence === 'low') {
    report.level = 'low_confidence';
    report.strategy = 'conservative';
    report.explanation = 'Hardware detection was incomplete. Manual verification is required before trusting this build.';
    report.manualVerificationRequired = true;
  } else if (report.confidence === 'medium') {
    report.level = 'supported_with_warnings';
    report.strategy = 'conservative';
    report.explanation = 'Some hardware values were inferred. The build uses conservative defaults and still needs manual review.';
    report.manualVerificationRequired = true;
  }

  const ramGB = parseInt(profile.ram, 10) || 0;
  if (ramGB > 0 && ramGB < 4) {
    report.warnings.push('RAM is below 4 GB. The system may boot, but installer reliability and performance will be poor.');
    report.minReqMet = false;
    if (report.level === 'supported') report.level = 'supported_with_warnings';
  }

  if (profile.architecture === 'Apple Silicon') {
    report.errors.push('Apple Silicon systems already run macOS natively and are not Hackintosh targets.');
    return setBlocked(report, 'Apple Silicon hardware is not a valid OpenCore/Hackintosh target.');
  }

  if (profile.isVM) {
    report.warnings.push('Virtual machines need PCIe GPU passthrough for usable acceleration. Without passthrough, macOS will run in VESA mode only.');
    report.level = 'partial_support';
    report.strategy = 'conservative';
    report.explanation = 'Virtual machine target detected. Treat this as a low-performance lab path, not a normal desktop build.';
  }

  if (cpu.includes('pentium') || cpu.includes('celeron') || cpu.includes('atom')) {
    if (profile.isLaptop) {
      report.errors.push('Mobile Pentium, Celeron, and Atom systems are not valid Hackintosh targets.');
      return setBlocked(report, 'Unsupported mobile Intel CPU family.');
    }
    if (!hasSupportedDisplayPath(gpuDevices)) {
      report.errors.push('Desktop Pentium/Celeron requires a separate supported display GPU. No supported output path was detected.');
      return setBlocked(report, 'No supported display path remains on this low-end Intel system.');
    }
    report.warnings.push('Low-end Intel desktop CPUs still need careful SMBIOS and display-path validation.');
    if (report.level === 'supported') report.level = 'partial_support';
  }

  if (profile.architecture === 'AMD' && profile.isLaptop) {
    const hasLimitedAmdLaptopPath = gpuAssessments.some(assessment =>
      assessment.requiresNootedRed ||
      assessment.name.toLowerCase().includes('5300m') ||
      assessment.name.toLowerCase().includes('5500m') ||
      assessment.name.toLowerCase().includes('5600m') ||
      assessment.name.toLowerCase().includes('5700m'),
    );

    if (!hasLimitedAmdLaptopPath) {
      report.errors.push('AMD laptops are not generally supported by canonical OpenCore automation. No documented supported display path was detected.');
      return setBlocked(report, 'Unsupported AMD laptop path.');
    }

    report.warnings.push('AMD laptop support is limited and lower-confidence even on the few documented working GPU paths.');
    report.level = 'partial_support';
    report.strategy = 'conservative';
    report.manualVerificationRequired = true;
  }

  const unsupportedGpuNames = gpuAssessments
    .filter(assessment => assessment.tier === 'unsupported')
    .map(assessment => assessment.name);

  if (!profile.isVM && !hasSupportedDisplayPath(gpuDevices)) {
    if (unsupportedGpuNames.length > 0) {
      report.errors.push(`No supported display path remains. Unsupported GPU(s): ${unsupportedGpuNames.join(', ')}.`);
    } else {
      report.errors.push('No supported display path remains. The detected GPU path is unknown or unsupported for the selected target.');
    }
    return setBlocked(report, 'OpenCore build blocked because no supported display path remains.');
  }

  if (profile.isLaptop && hasUnsupportedDiscreteGpu(gpuDevices)) {
    report.warnings.push('Laptop discrete GPU must be disabled unless you have confirmed the internal panel and required outputs stay on a supported path.');
    report.level = report.level === 'supported' ? 'partial_support' : report.level;
    report.strategy = 'conservative';
    report.manualVerificationRequired = true;
  } else if (!profile.isLaptop && hasUnsupportedDiscreteGpu(gpuDevices)) {
    report.warnings.push('Unsupported discrete GPU detected. Make sure the monitor is connected to a supported iGPU or supported AMD GPU before booting macOS.');
    if (report.level === 'supported') report.level = 'supported_with_warnings';
  }

  for (const assessment of gpuAssessments) {
    for (const note of assessment.notes) {
      if (!report.warnings.includes(note) && assessment.tier !== 'unsupported') {
        report.warnings.push(note);
      }
    }
  }

  if (bestAnyDisplayPath?.requiresNootRX) {
    report.warnings.push('Detected AMD Navi 22 GPU requires NootRX and is not a native WhateverGreen path.');
    report.level = 'partial_support';
    report.strategy = 'conservative';
  }

  if (bestAnyDisplayPath?.requiresNootedRed) {
    report.warnings.push('Detected AMD Vega APU path requires NootedRed and remains lower-confidence than native Intel or AMD dGPU paths.');
    report.level = 'partial_support';
    report.strategy = 'conservative';
  }

  if (motherboard.includes('pm981') || motherboard.includes('pm991') || motherboard.includes('2200s')) {
    report.warnings.push('Known-problem NVMe drive detected. NVMeFix helps, but this storage path is still lower-confidence.');
    report.strategy = 'conservative';
  }
  if (motherboard.includes('600p')) {
    report.warnings.push('Intel 600p NVMe is known to cause boot instability in macOS.');
    report.strategy = 'conservative';
  }

  if (profile.architecture === 'Intel' && ['Coffee Lake', 'Comet Lake', 'Rocket Lake'].includes(profile.generation) && report.level === 'supported') {
    report.explanation = `Intel ${profile.generation} system with a supported display path. This is a strong canonical OpenCore target.`;
  } else if (profile.architecture === 'AMD' && !profile.isLaptop && bestAnyDisplayPath) {
    report.explanation = `AMD desktop path detected. CPU support is viable, but success still depends on kernel patches, GPU class, and SMBIOS correctness.`;
  } else if (bestAnyDisplayPath) {
    report.explanation = `Supported display path detected through ${bestAnyDisplayPath.name}.`;
  }

  let maxVersion = 26;
  if (cpuCeiling != null) maxVersion = Math.min(maxVersion, cpuCeiling);
  if (bestDisplayCeiling != null) maxVersion = Math.min(maxVersion, bestDisplayCeiling);

  const eligibleVersions = getEligibleMacOSVersions(maxVersion);
  report.eligibleVersions = eligibleVersions;
  report.maxOSVersion = eligibleVersions[0]?.name ?? 'Blocked';
  report.recommendedVersion = eligibleVersions[0]?.name ?? '';

  if (!profile.isVM && !bestSelectedDisplayPath) {
    const highestSupported = eligibleVersions[0]?.name ?? 'an older supported version';
    report.level = 'blocked';
    report.strategy = 'blocked';
    report.isCompatible = false;
    report.errors.push(`Selected target ${profile.targetOS} exceeds the supported GPU or display-path ceiling. Choose ${highestSupported} or older.`);
    report.explanation = `The current target macOS version is above the supported GPU/display-path ceiling. Select ${highestSupported} or older.`;
  }

  if (report.level === 'supported' && report.manualVerificationRequired) {
    report.level = 'supported_with_warnings';
  }

  if (report.errors.length > 0) {
    report.isCompatible = false;
    report.level = 'blocked';
    report.strategy = 'blocked';
  }

  return report;
}
