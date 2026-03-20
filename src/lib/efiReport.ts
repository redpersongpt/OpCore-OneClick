// ── EFI Intelligence Report ──────────────────────────────────────────────────
// Generates a comprehensive, human-readable report explaining every decision
// made during EFI generation. This is the "senior Hackintosh expert" feature.

import type { HardwareProfile } from '../../electron/configGenerator';
import { getSMBIOSForProfile, getRequiredResources } from '../../electron/configGenerator';
import {
  classifyGpu,
  getBestSupportedGpuPath,
  getProfileGpuDevices,
  parseMacOSVersion,
  type GpuAssessment,
} from '../../electron/hackintoshRules.js';
import { KEXT_REGISTRY, type KextEntry } from '../data/kextRegistry';
import type { CompatibilityReport } from '../../electron/compatibility';
import { computeConfidenceScore } from './confidenceScore';

// ── Types ────────────────────────────────────────────────────────────────────

export interface EfiReportSection {
  title: string;
  items: EfiReportItem[];
}

export interface EfiReportItem {
  label: string;
  value: string;
  detail?: string;
  severity?: 'info' | 'success' | 'warning' | 'danger';
}

export interface KextExplanation {
  name: string;
  version?: string;
  category: string;
  reason: string;
  canonicality: string;
  dependencies: string[];
}

export interface BootArgExplanation {
  arg: string;
  purpose: string;
  impact: 'cosmetic' | 'functional' | 'critical';
}

export interface KnownLimitation {
  area: string;
  description: string;
  severity: 'low' | 'medium' | 'high';
  workaround?: string;
}

export interface EfiReport {
  hardware: EfiReportSection;
  smbios: { selected: string; reasoning: string; alternatives: string[] };
  kexts: KextExplanation[];
  bootArgs: BootArgExplanation[];
  limitations: KnownLimitation[];
  confidenceScore: number;
  confidenceExplanation: string;
  macOSCeiling: { version: string; reason: string } | null;
  generatedAt: string;
}

// ── Boot arg dictionary ──────────────────────────────────────────────────────

const BOOT_ARG_DICTIONARY: Record<string, { purpose: string; impact: BootArgExplanation['impact'] }> = {
  '-v': { purpose: 'Verbose boot — shows kernel messages during startup instead of the Apple logo. Useful for diagnosing boot failures.', impact: 'cosmetic' },
  'keepsyms=1': { purpose: 'Preserves kernel symbols during panics, making crash logs readable for debugging.', impact: 'cosmetic' },
  'debug=0x100': { purpose: 'Prevents automatic reboot on kernel panic. Keeps the panic screen visible so you can read the error.', impact: 'functional' },
  '-wegnoegpu': { purpose: 'Tells WhateverGreen to disable all discrete GPUs. Used when the dGPU is unsupported and the system should use the iGPU only.', impact: 'critical' },
  'agdpmod=pikera': { purpose: 'Fixes black screen on AMD Navi GPUs (RX 5000/6000/7000) by patching the board-id check in AppleGraphicsDevicePolicy.', impact: 'critical' },
  'unfairgva=1': { purpose: 'Enables DRM support (Netflix, Apple TV+) on systems that would otherwise fail hardware DRM checks.', impact: 'functional' },
  '-igfxnotelemetryload': { purpose: 'Prevents loading Intel GPU telemetry driver, which can cause freezes on some iGPU configurations.', impact: 'functional' },
  'npci=0x2000': { purpose: 'Fixes PCI configuration issues on older systems. Required for some legacy AMD and Intel HEDT boards.', impact: 'critical' },
  'npci=0x3000': { purpose: 'Alternative PCI configuration fix. Try this if npci=0x2000 does not resolve PCI-related boot failures.', impact: 'critical' },
  'e1000=0': { purpose: 'Disables Apple\'s built-in Intel e1000 driver to prevent conflicts with IntelMausi on Catalina and older.', impact: 'functional' },
  '-lilubetaall': { purpose: 'Allows Lilu and all its plugins to load on beta/unsupported macOS versions.', impact: 'functional' },
  '-no_compat_check': { purpose: 'Bypasses macOS hardware compatibility check. Required when using an SMBIOS that the target macOS version would otherwise reject.', impact: 'critical' },
  'revpatch=sbvmm': { purpose: 'RestrictEvents patch that enables software-update support on unsupported SMBIOS models.', impact: 'functional' },
  'alcboot': { purpose: 'Fixes audio codec initialization at boot time for AppleALC.', impact: 'functional' },
};

// ── SMBIOS reasoning ─────────────────────────────────────────────────────────

function getSMBIOSReasoning(profile: HardwareProfile): string {
  const osVer = parseMacOSVersion(profile.targetOS);
  const smbios = profile.smbios;

  if (profile.isVM) {
    return profile.architecture === 'AMD'
      ? 'Virtual machine detected with AMD host — MacPro7,1 provides the widest driver compatibility for VMs.'
      : 'Virtual machine detected with Intel host — iMacPro1,1 is the standard VM SMBIOS for Intel guests.';
  }

  if (profile.architecture === 'AMD') {
    if (profile.generation === 'Threadripper') return 'Threadripper systems map to MacPro7,1 — the only Mac with a comparable core/thread count and memory architecture.';
    if (smbios === 'MacPro7,1') return 'AMD Ryzen with a supported discrete AMD GPU. MacPro7,1 provides native AMD GPU acceleration paths.';
    return 'AMD Ryzen system. iMacPro1,1 is the standard SMBIOS for AMD Hackintosh systems without MacPro-era GPUs.';
  }

  if (profile.isLaptop) {
    const gen = profile.generation;
    if (gen === 'Coffee Lake' || gen === 'Comet Lake') return `${gen} laptop — MacBookPro15,2 or MacBookPro16,1 provides correct iGPU framebuffer and power management for this generation.`;
    if (gen === 'Kaby Lake') return 'Kaby Lake laptop — MacBookPro14,1 matches this generation\'s iGPU and power characteristics.';
    if (osVer >= 13) return `${gen} laptop targeting ${profile.targetOS} — older SMBIOS models are dropped in Ventura+, so a newer model is used to maintain update support.`;
    return `${gen} laptop — SMBIOS chosen to match the nearest real MacBook with this processor generation.`;
  }

  if (profile.generation.includes('-E') || profile.generation.includes('-X')) {
    return osVer >= 13
      ? 'HEDT/Server processor — MacPro7,1 is required for Ventura+ as older Mac Pro models were dropped.'
      : 'HEDT/Server processor — MacPro6,1 matches the original Mac Pro with similar Xeon-class hardware.';
  }

  return `${profile.generation} desktop — ${smbios} matches the nearest iMac generation with this processor family, providing correct power management and iGPU framebuffer paths.`;
}

function getSMBIOSAlternatives(profile: HardwareProfile): string[] {
  const alts: string[] = [];
  if (profile.architecture === 'AMD' && profile.smbios === 'iMacPro1,1') {
    alts.push('MacPro7,1 — use if you have a supported AMD discrete GPU (Polaris/Navi)');
  }
  if (profile.isLaptop && profile.smbios === 'MacBookPro14,1') {
    alts.push('MacBookPro15,2 — try if iGPU acceleration is not working correctly');
  }
  if (!profile.isLaptop && profile.smbios.startsWith('iMac')) {
    alts.push('MacPro7,1 — use if you have a dedicated AMD GPU and want GPU-first acceleration');
  }
  return alts;
}

// ── Kext explanations ────────────────────────────────────────────────────────

function explainKexts(kextNames: string[], profile: HardwareProfile): KextExplanation[] {
  return kextNames.map(name => {
    const entry = KEXT_REGISTRY.find(k => k.name === name);
    if (!entry) {
      return {
        name,
        category: 'unknown',
        reason: 'This kext was selected but is not in the known registry. It may be a custom addition.',
        canonicality: 'unknown',
        dependencies: [],
      };
    }

    let reason = entry.description;

    // Add context-specific reasoning
    if (name === 'AppleMCEReporterDisabler' && profile.architecture === 'AMD') {
      reason = 'Required on AMD systems running Monterey or newer to prevent kernel panics from Apple\'s MCE (Machine Check Exception) reporter, which expects Intel-specific MSRs.';
    } else if (name === 'NootRX') {
      reason = 'Patches AMD Navi 22 GPUs (RX 6700/6750 XT) for macOS. This GPU family needs special handling because Apple only natively supports Navi 21/23.';
    } else if (name === 'NootedRed') {
      reason = 'Enables graphics acceleration for AMD Vega integrated GPUs (APUs). Without this, the system would fall back to software rendering.';
    } else if (name === 'CPUTopologyRebuild' && profile.generation === 'Alder Lake') {
      reason = 'Alder Lake uses a hybrid P-core/E-core design that macOS doesn\'t understand natively. This kext rebuilds the CPU topology so macOS sees cores correctly.';
    } else if (name === 'WhateverGreen') {
      reason = 'Comprehensive GPU fixup framework. Handles framebuffer patching for Intel iGPUs, HDMI/DP output fixes, and AMD GPU compatibility patches.';
    } else if (name === 'VirtualSMC') {
      reason = 'Emulates Apple\'s System Management Controller (SMC). macOS refuses to boot without SMC hardware — this makes your PC look like a real Mac at the firmware level.';
    } else if (name === 'Lilu') {
      reason = 'Core kernel patching framework. Most other kexts (WhateverGreen, AppleALC, etc.) are Lilu plugins and will not function without it loaded first.';
    }

    return {
      name,
      version: undefined,
      category: entry.category,
      reason,
      canonicality: entry.canonicality ?? 'canonical',
      dependencies: entry.dependsOn ?? [],
    };
  });
}

// ── Boot arg explanations ────────────────────────────────────────────────────

function explainBootArgs(bootArgs: string): BootArgExplanation[] {
  const args = bootArgs.split(/\s+/).filter(Boolean);
  const explanations: BootArgExplanation[] = [];

  for (const arg of args) {
    // Handle alcid=XX specially
    if (arg.startsWith('alcid=')) {
      const id = arg.split('=')[1];
      explanations.push({
        arg,
        purpose: `Sets the AppleALC audio layout ID to ${id}. This tells the audio kext which codec pin configuration to use for your motherboard's audio chip.`,
        impact: 'functional',
      });
      continue;
    }

    const known = BOOT_ARG_DICTIONARY[arg];
    if (known) {
      explanations.push({ arg, ...known });
    } else {
      explanations.push({
        arg,
        purpose: 'Custom or unrecognized boot argument. Check Dortania\'s guide for details.',
        impact: 'functional',
      });
    }
  }

  return explanations;
}

// ── Known limitations ────────────────────────────────────────────────────────

function detectLimitations(profile: HardwareProfile, gpuAssessments: GpuAssessment[]): KnownLimitation[] {
  const limits: KnownLimitation[] = [];

  // GPU limitations
  const hasDisabledDgpu = profile.bootArgs.includes('-wegnoegpu');
  const hasUnsupportedNvidia = gpuAssessments.some(g => g.vendor === 'NVIDIA' && g.tier === 'unsupported');
  const hasPartialGpu = gpuAssessments.some(g => g.tier === 'partial_support');

  if (hasDisabledDgpu) {
    limits.push({
      area: 'Discrete GPU',
      description: 'Your discrete GPU is disabled via -wegnoegpu. The system will use the integrated GPU only. External displays may be limited.',
      severity: 'medium',
      workaround: 'This is intentional — your dGPU is not supported by macOS. Use monitor outputs connected to the motherboard (iGPU).',
    });
  }

  if (hasUnsupportedNvidia) {
    limits.push({
      area: 'NVIDIA GPU',
      description: 'NVIDIA GPUs from the Turing generation (RTX 20xx) and newer have no macOS drivers and never will. Maxwell and Pascal GPUs are limited to High Sierra (10.13).',
      severity: 'high',
    });
  }

  // DRM
  const isDrmCapable = profile.smbios === 'MacPro7,1' || profile.smbios === 'iMacPro1,1';
  if (!isDrmCapable) {
    limits.push({
      area: 'DRM Content',
      description: 'Hardware DRM (Netflix in Safari, Apple TV+ 4K, iTunes movies) may not work. Only MacPro7,1 and iMacPro1,1 SMBIOS with AMD dGPUs have full DRM support.',
      severity: 'low',
      workaround: 'Use Chrome for Netflix. For Apple TV+, the unfairgva=1 boot arg may help on some configurations.',
    });
  }

  // Sleep
  if (profile.architecture === 'AMD') {
    limits.push({
      area: 'Sleep/Wake',
      description: 'Sleep on AMD Hackintosh is unreliable. Wake from sleep frequently fails or causes USB disconnections.',
      severity: 'medium',
      workaround: 'Disable sleep in System Settings > Energy and use screen saver instead. Some users report success with specific USB mapping.',
    });
  }
  if (profile.isLaptop && hasDisabledDgpu) {
    limits.push({
      area: 'Sleep/Wake',
      description: 'Laptop sleep with disabled dGPU often causes wake failures. The GPU may not power down correctly in S3 state.',
      severity: 'medium',
      workaround: 'Test sleep behavior. If wake fails, consider using hibernatemode=25 or disabling sleep entirely.',
    });
  }

  // Wi-Fi
  const hasIntelWifi = profile.kexts.some(k => k === 'AirportItlwm' || k === 'Itlwm');
  if (hasIntelWifi) {
    limits.push({
      area: 'Wi-Fi',
      description: 'Intel Wi-Fi works via third-party kext but lacks some features: no AirDrop, no Continuity/Handoff, potentially slower speeds.',
      severity: 'low',
      workaround: 'For full Apple wireless features, replace with a Broadcom BCM94360NG or similar natively supported card.',
    });
  }

  // Itlwm-specific
  if (profile.kexts.includes('Itlwm')) {
    limits.push({
      area: 'Wi-Fi (Recovery)',
      description: 'Itlwm does not work during macOS Recovery or installation. You will need Ethernet or a compatible USB Wi-Fi adapter during setup.',
      severity: 'medium',
      workaround: 'Use AirportItlwm with SecureBootModel enabled for Recovery Wi-Fi, or use a wired Ethernet connection during installation.',
    });
  }

  // Bluetooth
  if (profile.kexts.some(k => k === 'IntelBluetoothFirmware' || k === 'BlueToolFixup')) {
    limits.push({
      area: 'Bluetooth',
      description: 'Intel Bluetooth works for audio and basic peripherals, but AirDrop, Handoff, and Universal Clipboard will not function.',
      severity: 'low',
    });
  }

  // iGPU-only on desktop
  if (!profile.isLaptop && !hasDisabledDgpu && gpuAssessments.every(g => !g.isLikelyDiscrete || g.tier === 'unsupported')) {
    limits.push({
      area: 'Graphics Performance',
      description: 'Running on integrated graphics only. GPU-intensive tasks (video editing, 3D) will be significantly slower than with a supported discrete GPU.',
      severity: 'low',
    });
  }

  // macOS version ceiling
  const ceilings = gpuAssessments
    .filter(g => g.maxMacOSVersion !== null)
    .map(g => g.maxMacOSVersion!);
  if (ceilings.length > 0) {
    const lowest = Math.min(...ceilings);
    const targetVer = parseMacOSVersion(profile.targetOS);
    if (targetVer > lowest) {
      limits.push({
        area: 'macOS Version',
        description: `Your GPU limits macOS support to version ${lowest}. Running a newer version may result in no graphics acceleration.`,
        severity: 'high',
      });
    }
  }

  return limits;
}

// ── macOS ceiling ────────────────────────────────────────────────────────────

function getMacOSCeiling(gpuAssessments: GpuAssessment[]): { version: string; reason: string } | null {
  const ceilings = gpuAssessments
    .filter(g => g.maxMacOSVersion !== null && g.tier !== 'unsupported')
    .map(g => ({ ver: g.maxMacOSVersion!, name: g.name }));

  if (ceilings.length === 0) return null;

  const lowest = ceilings.reduce((a, b) => a.ver < b.ver ? a : b);
  const versionName =
    lowest.ver >= 26 ? 'macOS Tahoe' :
    lowest.ver >= 15 ? 'macOS Sequoia' :
    lowest.ver >= 14 ? 'macOS Sonoma' :
    lowest.ver >= 13 ? 'macOS Ventura' :
    lowest.ver >= 12 ? 'macOS Monterey' :
    lowest.ver >= 11 ? 'macOS Big Sur' :
    `macOS ${lowest.ver}`;

  return {
    version: versionName,
    reason: `${lowest.name} has native driver support up to ${versionName}. Running a newer version will result in no hardware acceleration for this GPU.`,
  };
}

// ── Main report generator ────────────────────────────────────────────────────

export function generateEfiReport(
  profile: HardwareProfile,
  compat: CompatibilityReport | null,
  kextResults?: Array<{ name: string; version: string; source?: string }>,
): EfiReport {
  const gpuDevices = getProfileGpuDevices(profile);
  const gpuAssessments = gpuDevices.map(classifyGpu);
  const bestPath = getBestSupportedGpuPath(gpuDevices, parseMacOSVersion(profile.targetOS));

  // Hardware section
  const hardware: EfiReportSection = {
    title: 'Hardware Summary',
    items: [
      { label: 'Processor', value: profile.cpu, detail: `${profile.architecture} ${profile.generation} · ${profile.coreCount ?? '?'} cores`, severity: 'info' },
      { label: 'Graphics', value: profile.gpu, detail: bestPath ? `Active path: ${bestPath.name} (${bestPath.tier})` : 'No supported display path', severity: bestPath ? 'success' : 'danger' },
      { label: 'Memory', value: profile.ram, severity: parseInt(profile.ram) >= 8 ? 'success' : 'warning' },
      { label: 'Board', value: profile.motherboard, severity: 'info' },
      { label: 'Form Factor', value: profile.isLaptop ? 'Laptop' : 'Desktop', severity: 'info' },
      { label: 'Target macOS', value: profile.targetOS, severity: 'info' },
    ],
  };

  // SMBIOS
  const smbios = {
    selected: profile.smbios,
    reasoning: getSMBIOSReasoning(profile),
    alternatives: getSMBIOSAlternatives(profile),
  };

  // Kexts with versions from results
  const kextExplanations = explainKexts(profile.kexts, profile);
  if (kextResults) {
    for (const exp of kextExplanations) {
      const result = kextResults.find(r => r.name === exp.name);
      if (result) {
        exp.version = result.version;
      }
    }
  }

  // Boot args
  const bootArgs = explainBootArgs(profile.bootArgs);

  // Limitations
  const limitations = detectLimitations(profile, gpuAssessments);

  // Confidence
  const { score, explanation } = computeConfidenceScore(profile, compat, gpuAssessments);

  // macOS ceiling
  const macOSCeiling = getMacOSCeiling(gpuAssessments);

  return {
    hardware,
    smbios,
    kexts: kextExplanations,
    bootArgs,
    limitations,
    confidenceScore: score,
    confidenceExplanation: explanation,
    macOSCeiling,
    generatedAt: new Date().toISOString(),
  };
}
