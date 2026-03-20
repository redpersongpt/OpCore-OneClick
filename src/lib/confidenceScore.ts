// ── Confidence Score System ──────────────────────────────────────────────────
// Computes a 0–100 confidence score for the current build, based on GPU support,
// CPU generation maturity, kext complexity, and known problem setups.

import type { HardwareProfile } from '../../electron/configGenerator';
import type { CompatibilityReport } from '../../electron/compatibility';
import type { GpuAssessment } from '../../electron/hackintoshRules';

export interface ConfidenceResult {
  score: number;
  explanation: string;
  factors: ConfidenceFactor[];
}

export interface ConfidenceFactor {
  name: string;
  impact: number; // -30 to +20
  detail: string;
}

// ── CPU maturity scores ─────────────────────────────────────────────────────

const CPU_MATURITY: Record<string, number> = {
  'Coffee Lake': 20,
  'Kaby Lake': 18,
  'Comet Lake': 17,
  'Skylake': 16,
  'Haswell': 14,
  'Broadwell': 14,
  'Ivy Bridge': 10,
  'Sandy Bridge': 8,
  'Ryzen': 12,
  'Threadripper': 10,
  'Alder Lake': 8,
  'Raptor Lake': 6,
  'Rocket Lake': 6,
  'Penryn': 5,
  'Bulldozer': 4,
  'Cascade Lake-X': 10,
  'Haswell-E': 12,
  'Broadwell-E': 12,
  'Ivy Bridge-E': 8,
};

export function computeConfidenceScore(
  profile: HardwareProfile,
  compat: CompatibilityReport | null,
  gpuAssessments: GpuAssessment[],
): ConfidenceResult {
  const factors: ConfidenceFactor[] = [];
  let base = 50; // Start at 50

  // ── Factor 1: CPU generation maturity
  const cpuScore = CPU_MATURITY[profile.generation] ?? 0;
  factors.push({
    name: 'CPU Generation',
    impact: cpuScore,
    detail: cpuScore >= 15
      ? `${profile.generation} is a well-supported generation with mature kext and power management support.`
      : cpuScore >= 8
      ? `${profile.generation} is supported but may require additional configuration or has known edge cases.`
      : `${profile.generation} has limited community testing or requires significant workarounds.`,
  });
  base += cpuScore;

  // ── Factor 2: GPU support tier
  const bestGpu = gpuAssessments.find(g => g.tier === 'supported')
    ?? gpuAssessments.find(g => g.tier === 'supported_with_limit')
    ?? gpuAssessments.find(g => g.tier === 'partial_support')
    ?? gpuAssessments[0];

  if (bestGpu) {
    if (bestGpu.tier === 'supported') {
      factors.push({ name: 'GPU Support', impact: 15, detail: `${bestGpu.name} has native macOS driver support.` });
      base += 15;
    } else if (bestGpu.tier === 'supported_with_limit') {
      factors.push({ name: 'GPU Support', impact: 5, detail: `${bestGpu.name} is supported but limited to an older macOS version.` });
      base += 5;
    } else if (bestGpu.tier === 'partial_support') {
      factors.push({ name: 'GPU Support', impact: -5, detail: `${bestGpu.name} has partial support — some features may not work.` });
      base -= 5;
    } else {
      factors.push({ name: 'GPU Support', impact: -20, detail: `${bestGpu.name} is not supported by macOS. The system will use iGPU or software rendering.` });
      base -= 20;
    }
  }

  // ── Factor 3: Kext complexity
  const conditionalKexts = profile.kexts.filter(k => {
    const known = ['Lilu', 'VirtualSMC', 'WhateverGreen', 'AppleALC'];
    return !known.includes(k);
  });
  if (conditionalKexts.length <= 2) {
    factors.push({ name: 'Kext Complexity', impact: 5, detail: 'Minimal kext footprint — fewer moving parts means fewer potential issues.' });
    base += 5;
  } else if (conditionalKexts.length <= 5) {
    factors.push({ name: 'Kext Complexity', impact: 0, detail: `${conditionalKexts.length} conditional kexts — reasonable complexity for this hardware.` });
  } else {
    factors.push({ name: 'Kext Complexity', impact: -5, detail: `${conditionalKexts.length} conditional kexts — higher complexity increases the chance of kext conflicts.` });
    base -= 5;
  }

  // ── Factor 4: Known problem setups
  if (profile.architecture === 'AMD' && profile.isLaptop) {
    factors.push({ name: 'AMD Laptop', impact: -25, detail: 'AMD laptops have very limited macOS support. iGPU acceleration, trackpad, and power management are major challenges.' });
    base -= 25;
  }
  if (profile.isVM) {
    factors.push({ name: 'Virtual Machine', impact: -10, detail: 'VMs have limited GPU passthrough and some kexts behave differently.' });
    base -= 10;
  }
  if (profile.generation === 'Alder Lake' || profile.generation === 'Raptor Lake') {
    factors.push({ name: 'Hybrid Architecture', impact: -5, detail: 'P-core/E-core hybrid CPUs require CPUTopologyRebuild and have less testing than traditional architectures.' });
    base -= 5;
  }

  // ── Factor 5: Scan confidence
  if (profile.scanConfidence === 'high') {
    factors.push({ name: 'Hardware Detection', impact: 5, detail: 'All hardware was detected with high confidence using vendor IDs.' });
    base += 5;
  } else if (profile.scanConfidence === 'low') {
    factors.push({ name: 'Hardware Detection', impact: -10, detail: 'Some hardware was inferred from name patterns. Kext selection may not be perfectly matched.' });
    base -= 10;
  }

  // ── Factor 6: Compatibility report
  if (compat) {
    if (compat.level === 'supported') {
      factors.push({ name: 'Compatibility', impact: 5, detail: 'Compatibility analysis found no issues.' });
      base += 5;
    } else if (compat.level === 'blocked') {
      factors.push({ name: 'Compatibility', impact: -30, detail: 'Compatibility analysis found blocking issues.' });
      base -= 30;
    } else if (compat.warnings.length > 0) {
      const penalty = Math.min(compat.warnings.length * 3, 15);
      factors.push({ name: 'Compatibility Warnings', impact: -penalty, detail: `${compat.warnings.length} warning(s) from compatibility analysis.` });
      base -= penalty;
    }
  }

  // Clamp to 0–100
  const score = Math.max(0, Math.min(100, base));

  // Build explanation
  const explanation = score >= 80
    ? 'High confidence — well-supported hardware with a proven configuration path.'
    : score >= 60
    ? 'Moderate confidence — supported hardware with some areas that may need manual attention.'
    : score >= 40
    ? 'Low-moderate confidence — this setup will likely work but has known limitations or unusual hardware.'
    : score >= 20
    ? 'Low confidence — significant challenges expected. Prepare for troubleshooting.'
    : 'Very low confidence — this hardware combination has fundamental compatibility issues.';

  return { score, explanation, factors };
}
