/**
 * Hardware Interpretation Layer
 *
 * Sits between raw DetectedHardware and HardwareProfile.
 * Separates detected facts from derived/inferred interpretations,
 * explains every mapping decision, and flags what the user
 * should verify manually.
 *
 * This module does NOT invent data. It only works with what
 * hardwareDetect.ts returns.
 *
 * Basis semantics:
 *   detected — value read directly from an authoritative OS or hardware source
 *              (PCI ID, vendor register, DMI data, OS memory API)
 *   derived  — computed from detected values via deterministic mapping logic
 *              (e.g. CPU generation from model number, GPU support from vendor+model)
 *              The input is authoritative but the mapping table may be imperfect.
 *   inferred — heuristic guess with weak evidence
 *              (e.g. form factor from chassis type, legacy AMD gen from brand name)
 *   unknown  — no usable signal found
 */

import type { DetectedHardware, GpuDevice, CpuInfo, DetectionConfidence } from './hardwareDetect.js';
import { classifyGpu, type HardwareGpuDeviceSummary } from './hackintoshRules.js';

// ── Types ─────────────────────────────────────────────────────────────────────

export type InterpretationBasis = 'detected' | 'derived' | 'inferred' | 'unknown';

export interface InterpretedFact {
  /** What was determined */
  label: string;
  /** The value */
  value: string;
  /** How it was determined */
  basis: InterpretationBasis;
  /** Short explanation of why this value was chosen */
  reasoning: string;
  /** What the user should check manually if basis is not 'detected' */
  verifyHint: string | null;
}

export interface GpuInterpretation {
  name: string;
  vendor: InterpretedFact;
  pciIds: InterpretedFact;
  macosSupport: InterpretedFact;
  driverNote: string | null;
}

export interface CpuInterpretation {
  name: string;
  vendor: InterpretedFact;
  architecture: InterpretedFact;
  generation: InterpretedFact;
  coreCount: InterpretedFact;
}

export interface BoardInterpretation {
  vendor: InterpretedFact;
  model: InterpretedFact;
  formFactor: InterpretedFact;
}

export interface AudioInterpretation {
  codec: InterpretedFact;
  layoutId: InterpretedFact;
}

export interface NetworkInterpretation {
  ethernet: InterpretedFact;
  wifi: InterpretedFact;
}

export interface HardwareInterpretation {
  /** Overall interpretation confidence */
  overallConfidence: 'high' | 'medium' | 'low';
  /** Plain-language summary of what was found */
  summary: string;
  /** Things the user should manually verify before trusting the config */
  manualVerificationNeeded: string[];
  /** Detailed per-component interpretations */
  cpu: CpuInterpretation;
  primaryGpu: GpuInterpretation;
  allGpus: GpuInterpretation[];
  board: BoardInterpretation;
  audio: AudioInterpretation;
  network: NetworkInterpretation;
  ram: InterpretedFact;
  vmDetected: InterpretedFact;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Map raw DetectionConfidence (from hardwareDetect.ts) to InterpretationBasis.
 * This is for values that hardwareDetect reads directly — PCI IDs, vendor strings.
 * 'detected' in hardwareDetect means authoritative OS/hardware data.
 * 'partially-detected' means name-string fallback (no PCI ID) — that's inferred.
 */
function basisFromConfidence(c: DetectionConfidence): InterpretationBasis {
  if (c === 'detected') return 'detected';
  if (c === 'partially-detected') return 'inferred';
  return 'unknown';
}

// ── CPU generation derivation ─────────────────────────────────────────────────
// ALL generation results are 'derived' or 'inferred', NEVER 'detected'.
// The CPU model name is detected; the generation is derived from it via a
// naming-convention lookup table that may be imperfect for OEM/edge-case SKUs.

function cpuGenFromName(name: string): { generation: string; basis: InterpretationBasis; reasoning: string } {
  const model = name.toLowerCase();

  // Apple Silicon — brand name in model string. Derivation is reliable but it's
  // still name-string matching, not a CPUID register read.
  if (model.includes('apple') || /\bm[1-4]\b/.test(model)) {
    return {
      generation: 'Apple Silicon',
      basis: 'derived',
      reasoning: 'Detected CPU name contains Apple Silicon identifier. Generation derived from brand name — reliable for Apple hardware.',
    };
  }

  // Xeon — naming convention is less predictable; some Xeon models span
  // multiple microarchitectures within a product family.
  if (model.includes('xeon')) {
    if (model.includes('w-') || model.includes('scalable')) return { generation: 'Cascade Lake-X', basis: 'inferred', reasoning: 'Detected Xeon W or Scalable series. Mapped to Cascade Lake-X — this is approximate; Xeon W spans multiple generations.' };
    if (model.includes('e5-v4')) return { generation: 'Broadwell-E', basis: 'derived', reasoning: 'Detected Xeon E5 v4 series. Derived generation: Broadwell-E (v4 suffix maps reliably to this gen).' };
    if (model.includes('e5-v3')) return { generation: 'Haswell-E', basis: 'derived', reasoning: 'Detected Xeon E5 v3 series. Derived generation: Haswell-E (v3 suffix maps reliably to this gen).' };
    if (model.includes('e5-v2')) return { generation: 'Ivy Bridge-E', basis: 'derived', reasoning: 'Detected Xeon E5 v2 series. Derived generation: Ivy Bridge-E (v2 suffix maps reliably to this gen).' };
    return { generation: 'Haswell-E', basis: 'inferred', reasoning: 'Detected Xeon model but could not extract version suffix. Defaulted to Haswell-E as a safe middle ground — verify your exact Xeon model.' };
  }

  // Standard Intel Core i-series — model number extraction via regex.
  // The number range → generation mapping follows Intel's public naming scheme
  // and is reliable for standard retail SKUs. OEM-modified SKUs or engineering
  // samples may not follow this pattern.
  const match = model.match(/i\d-?\s?(1?\d{4})/);
  if (match) {
    const num = parseInt(match[1]);
    const map: [number, string][] = [
      [14000, 'Raptor Lake'], [13000, 'Raptor Lake'], [12000, 'Alder Lake'],
      [11000, 'Rocket Lake'], [10000, 'Comet Lake'], [8000, 'Coffee Lake'],
      [7000, 'Kaby Lake'], [6000, 'Skylake'], [5000, 'Broadwell'],
      [4000, 'Haswell'], [3000, 'Ivy Bridge'], [2000, 'Sandy Bridge'],
    ];
    for (const [threshold, gen] of map) {
      if (num >= threshold) {
        return {
          generation: gen,
          basis: 'derived',
          reasoning: `Detected CPU model number ${num}. Derived generation: ${gen} (Intel naming convention: ${threshold}–${threshold + 999} series). Note: OEM-rebadged SKUs may not follow this pattern.`,
        };
      }
    }
  }

  // Budget Intel — Pentium/Celeron naming is less consistent than Core i-series.
  // Model suffixes vary between generations and OEMs repackage them.
  if (model.includes('pentium') || model.includes('celeron')) {
    if (model.includes('gold')) return { generation: 'Coffee Lake', basis: 'inferred', reasoning: 'Detected Pentium Gold branding. Inferred generation: Coffee Lake — Pentium Gold is typically Coffee Lake era, but verify at ark.intel.com.' };
    if (model.match(/g[45]\d{2}/)) return { generation: 'Skylake', basis: 'inferred', reasoning: 'Detected Pentium/Celeron G4xxx/G5xxx pattern. Inferred generation: Skylake era — this pattern spans multiple generations; verify at ark.intel.com.' };
    return { generation: 'Ivy Bridge', basis: 'inferred', reasoning: 'Detected Pentium/Celeron but could not match to a specific generation. Defaulted to Ivy Bridge — verify your exact model at ark.intel.com.' };
  }

  // Core 2 era — "Core 2" branding is unambiguous within the Penryn family,
  // but the actual microarch could be Conroe, Merom, Wolfdale, or Penryn.
  // We map to Penryn as the latest/safest in this family.
  if (model.includes('core 2') || model.includes('quad') || model.includes('extreme')) {
    return {
      generation: 'Penryn',
      basis: 'derived',
      reasoning: 'Detected Core 2 / Quad / Extreme branding. Derived generation: Penryn (this covers the Core 2 family — actual microarch may be Conroe through Penryn).',
    };
  }

  // AMD — Threadripper branding is unambiguous. Ryzen branding spans Zen 1–5
  // but the OpenCore config treats all Ryzen gens the same way.
  if (model.includes('threadripper')) return {
    generation: 'Threadripper',
    basis: 'derived',
    reasoning: 'Detected Threadripper branding. Derived generation: Threadripper. OpenCore config uses the same settings for all Threadripper generations.',
  };
  if (model.includes('ryzen')) return {
    generation: 'Ryzen',
    basis: 'derived',
    reasoning: 'Detected Ryzen branding. Derived generation: Ryzen. Note: this covers Zen 1 through Zen 5 — OpenCore uses the same base settings for all, but core count matters for AMD kernel patches.',
  };

  // Legacy AMD — FX/Phenom/Athlon branding spans Bulldozer, Piledriver,
  // Steamroller, and Excavator. We group them as "Bulldozer family" because
  // OpenCore config treats them identically.
  if (model.includes('fx-') || model.includes('phenom') || model.includes('athlon')) {
    return {
      generation: 'Bulldozer',
      basis: 'inferred',
      reasoning: 'Detected legacy AMD branding (FX/Phenom/Athlon). Inferred generation: Bulldozer family — actual microarch may be Piledriver, Steamroller, or Excavator. OpenCore config is the same for all.',
    };
  }

  return { generation: 'Unknown', basis: 'unknown', reasoning: 'CPU model name could not be matched to any known generation.' };
}

// ── CPU architecture derivation ───────────────────────────────────────────────
// Architecture (Intel/AMD/Apple Silicon) is derived from brand names in the
// model string. This is reliable when the brand name is present, but it's
// still a string match — not a CPUID register read.

function cpuArchFromName(name: string): { arch: string; basis: InterpretationBasis; reasoning: string } {
  const model = name.toLowerCase();

  if (model.includes('apple') || /\bm[1-4]\b/.test(model)) return {
    arch: 'Apple Silicon',
    basis: 'derived',
    reasoning: 'Detected Apple Silicon identifier in CPU name. Architecture derived from brand name.',
  };

  if (model.includes('ryzen') || model.includes('threadripper') || model.includes('fx-') || model.includes('phenom') || model.includes('athlon')) {
    return {
      arch: 'AMD',
      basis: 'derived',
      reasoning: 'Detected AMD product name in CPU string. Architecture derived from brand name.',
    };
  }

  if (model.includes('intel') || model.match(/i\d-/) || model.includes('xeon') || model.includes('core 2') || model.includes('pentium') || model.includes('celeron')) {
    return {
      arch: 'Intel',
      basis: 'derived',
      reasoning: 'Detected Intel product name in CPU string. Architecture derived from brand name.',
    };
  }

  return { arch: 'Unknown', basis: 'unknown', reasoning: 'CPU vendor could not be determined from the model name.' };
}

// ── GPU macOS support assessment ──────────────────────────────────────────────
// macOS support is NEVER directly detected from the system — it is always
// derived by matching the GPU vendor + model name against a static support
// database. The database may be outdated if Apple adds or drops GPU support.

function interpretGpuSupport(gpu: GpuDevice): { support: string; basis: InterpretationBasis; reasoning: string; driverNote: string | null } {
  const vendor = gpu.vendorName;

  if (vendor === 'Unknown') {
    return {
      support: 'Cannot determine',
      basis: 'unknown',
      reasoning: 'GPU vendor is unknown — macOS support cannot be assessed.',
      driverNote: null,
    };
  }

  // VM GPUs — vendor ID is authoritative (from PCI ID or manufacturer string),
  // and the conclusion ("no Metal") is a known fact about virtual GPUs.
  if (vendor === 'VMware' || vendor === 'Microsoft' || vendor === 'QEMU') {
    return {
      support: 'Virtual (no Metal)',
      basis: 'derived',
      reasoning: `Detected ${vendor} virtual GPU (from PCI vendor ID or system manufacturer). Derived support status: macOS runs in VESA mode without Metal acceleration.`,
      driverNote: 'GPU passthrough required for Metal support in a VM.',
    };
  }

  const assessment = classifyGpu({
    name: gpu.name,
    vendorName: gpu.vendorName,
    vendorId: gpu.vendorId,
    deviceId: gpu.deviceId,
  } satisfies HardwareGpuDeviceSummary);

  let support = 'Cannot determine';
  if (assessment.tier === 'supported') {
    support = 'Supported';
  } else if (assessment.tier === 'supported_with_limit') {
    support = assessment.maxMacOSVersion
      ? `Supported up to macOS ${assessment.maxMacOSVersion}`
      : 'Supported with version limit';
  } else if (assessment.tier === 'partial_support') {
    support = 'Partial support';
  } else if (assessment.tier === 'unsupported') {
    support = 'Unsupported';
  }

  const driverNote = assessment.requiresPikera
    ? 'Boot-arg agdpmod=pikera is required on this GPU path.'
    : assessment.requiresNootRX
      ? 'This GPU path requires NootRX instead of WhateverGreen.'
      : assessment.requiresNootedRed
        ? 'This GPU path requires NootedRed and remains lower-confidence than native display paths.'
        : assessment.requiresDisable
          ? 'This GPU must be disabled or removed from the active display path for macOS.'
          : assessment.vendor === 'Intel'
            ? 'Intel iGPU support still depends on framebuffer configuration and the active output path.'
            : null;

  return {
    support,
    basis: assessment.tier === 'unknown' ? 'unknown' : 'derived',
    reasoning: assessment.notes[0] ?? 'GPU support was derived from the shared Hackintosh compatibility rules.',
    driverNote,
  };
}

// ── Main interpretation function ──────────────────────────────────────────────

export function interpretHardware(hw: DetectedHardware): HardwareInterpretation {
  const manualVerify: string[] = [];

  // ── CPU ──
  // CPU vendor string is detected (authoritative from OS/CPUID).
  // vendorName ("Intel"/"AMD") is a trivial lookup from that — keep as detected.
  const cpuBasis = basisFromConfidence(hw.cpu.confidence);
  const genResult = cpuGenFromName(hw.cpu.name);
  const archResult = cpuArchFromName(hw.cpu.name);

  if (genResult.basis !== 'detected') {
    // 'derived' and 'inferred' both need manual verification — different severity
    const qualifier = genResult.basis === 'derived'
      ? 'was derived from the model name — likely correct but verify for OEM or engineering-sample CPUs'
      : 'was inferred with low confidence — verify your exact CPU model at ark.intel.com or amd.com/en/products';
    manualVerify.push(`CPU generation (${genResult.generation}) ${qualifier}`);
  }
  if (archResult.basis === 'unknown') {
    manualVerify.push('CPU vendor could not be determined — the generated config may use incorrect settings');
  }

  const cpuInterp: CpuInterpretation = {
    name: hw.cpu.name,
    vendor: {
      label: 'CPU Vendor',
      value: hw.cpu.vendorName,
      basis: cpuBasis,
      reasoning: cpuBasis === 'detected'
        ? `Vendor string "${hw.cpu.vendor}" read directly from system CPUID/WMI`
        : cpuBasis === 'inferred'
        ? `Vendor guessed from CPU name — not read from hardware`
        : `Vendor could not be determined`,
      verifyHint: cpuBasis !== 'detected' ? 'Check your CPU vendor on the manufacturer website' : null,
    },
    architecture: {
      label: 'Architecture',
      value: archResult.arch,
      basis: archResult.basis,
      reasoning: archResult.reasoning,
      verifyHint: archResult.basis === 'unknown' ? 'Confirm whether your CPU is Intel, AMD, or Apple Silicon' : null,
    },
    generation: {
      label: 'CPU Generation',
      value: genResult.generation,
      basis: genResult.basis,
      reasoning: genResult.reasoning,
      verifyHint: genResult.basis !== 'detected'
        ? genResult.basis === 'derived'
          ? 'Generation was derived from model number — likely correct, but verify if you have an OEM or unusual SKU'
          : 'Look up your exact CPU model to confirm the microarchitecture generation'
        : null,
    },
    coreCount: {
      label: 'Core Count',
      value: `${hw.coreCount}`,
      basis: hw.coreCount > 0 ? 'detected' : 'unknown',
      reasoning: hw.coreCount > 0
        ? 'Core count read from OS scheduler (logical cores)'
        : 'Core count could not be determined',
      verifyHint: null,
    },
  };

  // ── GPU ──
  function interpretOneGpu(gpu: GpuDevice): GpuInterpretation {
    const gpuBasis = basisFromConfidence(gpu.confidence);
    const support = interpretGpuSupport(gpu);

    if (gpuBasis !== 'detected') {
      manualVerify.push(`GPU "${gpu.name}" vendor was inferred from name (no PCI ID) — verify the exact model in your system's device manager`);
    }
    if (support.basis === 'inferred' || support.basis === 'unknown') {
      manualVerify.push(`macOS support for "${gpu.name}" is uncertain — check the Dortania GPU Buyers Guide`);
    }

    return {
      name: gpu.name,
      vendor: {
        label: 'GPU Vendor',
        value: gpu.vendorName,
        basis: gpuBasis,
        reasoning: gpu.vendorId
          ? `PCI vendor ID 0x${gpu.vendorId} read from hardware. Resolved to ${gpu.vendorName} via standard PCI ID database.`
          : `No PCI vendor ID available. Vendor inferred from GPU name "${gpu.name}" via string matching.`,
        verifyHint: gpuBasis !== 'detected' ? 'Check the GPU vendor in Device Manager (Windows) or lspci (Linux)' : null,
      },
      pciIds: {
        label: 'PCI IDs',
        value: gpu.vendorId && gpu.deviceId ? `${gpu.vendorId}:${gpu.deviceId}` : 'Not available',
        basis: gpu.vendorId ? 'detected' : 'unknown',
        reasoning: gpu.vendorId
          ? `PCI IDs read from system hardware enumeration`
          : `PCI IDs not available — vendor was inferred from name only`,
        verifyHint: !gpu.vendorId ? 'PCI IDs are important for accurate config generation — try running from a different OS if possible' : null,
      },
      macosSupport: {
        label: 'macOS Support',
        value: support.support,
        basis: support.basis,
        reasoning: support.reasoning,
        verifyHint: support.basis === 'inferred' || support.basis === 'unknown'
          ? 'Check the Dortania GPU Buyers Guide for your specific GPU model'
          : support.basis === 'derived'
          ? 'Derived from model name against a known GPU support database — verify if your exact SKU has known issues'
          : null,
      },
      driverNote: support.driverNote,
    };
  }

  const allGpuInterps = hw.gpus.map(interpretOneGpu);
  const primaryGpuInterp = allGpuInterps[0];

  // ── Board ──
  const boardVendorKnown = hw.motherboardVendor !== 'Unknown' && hw.motherboardVendor !== '';
  const boardModelKnown = hw.motherboardModel !== 'Unknown' && hw.motherboardModel !== '';

  if (!boardModelKnown) {
    manualVerify.push('Motherboard model was not detected — quirk selection and SMBIOS mapping may be inaccurate');
  }

  const boardInterp: BoardInterpretation = {
    vendor: {
      label: 'Board Manufacturer',
      value: hw.motherboardVendor || 'Unknown',
      basis: boardVendorKnown ? 'detected' : 'unknown',
      reasoning: boardVendorKnown
        ? 'Motherboard vendor read from system DMI/SMBIOS data'
        : 'Motherboard vendor could not be read — board-specific quirks may be wrong',
      verifyHint: !boardVendorKnown ? 'Check the manufacturer name on your motherboard or in BIOS setup' : null,
    },
    model: {
      label: 'Board Model',
      value: hw.motherboardModel || 'Unknown',
      basis: boardModelKnown ? 'detected' : 'unknown',
      reasoning: boardModelKnown
        ? 'Motherboard model read from system DMI/SMBIOS data'
        : 'Motherboard model could not be read — chipset-specific quirks (e.g. Z390, B550) cannot be applied',
      verifyHint: !boardModelKnown ? 'The board model determines important config.plist quirks — check your motherboard box or BIOS for the exact model' : null,
    },
    formFactor: {
      label: 'Form Factor',
      value: hw.isLaptop ? 'Laptop' : 'Desktop',
      basis: 'inferred',
      reasoning: hw.isLaptop
        ? 'Inferred as laptop from SMBIOS chassis type or mobile CPU suffix (e.g. -U, -H, -HQ). Neither is guaranteed — some OEMs set chassis type incorrectly.'
        : 'No laptop indicators found in chassis type or CPU suffix — assumed desktop. This could be wrong for all-in-ones or non-standard form factors.',
      verifyHint: 'Confirm whether this is a laptop or desktop — laptop configs include different kexts (battery, trackpad, backlight)',
    },
  };

  // ── Audio ──
  const primaryAudio = hw.audioDevices?.find(a => a.codecName !== null);
  const audioCodecValue = primaryAudio?.codecName ?? 'Not detected';
  const audioCodecBasis: InterpretationBasis = primaryAudio
    ? (primaryAudio.vendorId ? 'detected' : 'inferred')
    : 'unknown';

  if (audioCodecBasis === 'unknown') {
    manualVerify.push('Audio codec was not detected — layout-id defaults to 1 (may not match your hardware)');
  } else if (audioCodecBasis === 'inferred') {
    manualVerify.push(`Audio codec (${audioCodecValue}) was inferred — verify in AppleALC supported codecs list`);
  }

  const audioInterp: AudioInterpretation = {
    codec: {
      label: 'Audio Codec',
      value: audioCodecValue,
      basis: audioCodecBasis,
      reasoning: primaryAudio
        ? primaryAudio.vendorId
          ? `Detected HDA vendor ${primaryAudio.vendorId}:${primaryAudio.deviceId} from PnP enumeration. Resolved to ${audioCodecValue}.`
          : `Audio device "${primaryAudio.name}" found but vendor/device IDs not available.`
        : 'No audio devices detected by hardware scan.',
      verifyHint: audioCodecBasis !== 'detected'
        ? 'Check your audio codec in Device Manager (Windows) or codec dump tools. The layout-id in config.plist depends on this.'
        : null,
    },
    layoutId: {
      label: 'Audio Layout ID',
      value: primaryAudio?.codecName
        ? `Mapped from ${primaryAudio.codecName}`
        : '1 (universal fallback)',
      basis: primaryAudio?.codecName ? 'derived' : 'inferred',
      reasoning: primaryAudio?.codecName
        ? `Layout-ID derived from codec ${primaryAudio.codecName} using AppleALC supported codecs table.`
        : 'Using universal fallback layout-id=1 — works on most hardware but may not enable all outputs.',
      verifyHint: 'If audio does not work after install, try different layout-id values from the AppleALC wiki',
    },
  };

  // ── Network ──
  const ethernetDevices = hw.networkDevices?.filter(n => n.type === 'ethernet') ?? [];
  const wifiDevices = hw.networkDevices?.filter(n => n.type === 'wifi') ?? [];
  const primaryEthernet = ethernetDevices.find(n => n.adapterFamily) ?? ethernetDevices[0];
  const primaryWifi = wifiDevices.find(n => n.adapterFamily) ?? wifiDevices[0];

  const networkInterp: NetworkInterpretation = {
    ethernet: {
      label: 'Ethernet Adapter',
      value: primaryEthernet?.adapterFamily ?? (primaryEthernet ? `${primaryEthernet.vendorName} (unresolved)` : 'Not detected'),
      basis: primaryEthernet?.adapterFamily ? (primaryEthernet.vendorId ? 'detected' : 'inferred') : 'unknown',
      reasoning: primaryEthernet?.adapterFamily
        ? `Ethernet adapter resolved from PCI vendor:device ${primaryEthernet.vendorId}:${primaryEthernet.deviceId}`
        : primaryEthernet
          ? `Ethernet device found ("${primaryEthernet.name}") but adapter family could not be resolved`
          : 'No Ethernet adapter detected by hardware scan.',
      verifyHint: !primaryEthernet?.adapterFamily
        ? 'Check your Ethernet adapter in Device Manager — the correct kext depends on the chipset (Intel/Realtek/Atheros)'
        : null,
    },
    wifi: {
      label: 'Wi-Fi Adapter',
      value: primaryWifi?.adapterFamily ?? (primaryWifi ? `${primaryWifi.vendorName} (unresolved)` : 'Not detected'),
      basis: primaryWifi?.adapterFamily ? (primaryWifi.vendorId ? 'detected' : 'inferred') : 'unknown',
      reasoning: primaryWifi?.adapterFamily
        ? `Wi-Fi adapter resolved from PCI vendor:device ${primaryWifi.vendorId}:${primaryWifi.deviceId}`
        : primaryWifi
          ? `Wi-Fi device found ("${primaryWifi.name}") but adapter family could not be resolved`
          : 'No Wi-Fi adapter detected by hardware scan.',
      verifyHint: !primaryWifi?.adapterFamily
        ? 'Check your Wi-Fi adapter — Intel Wi-Fi requires itlwm/AirportItlwm, Broadcom may need AirportBrcmFixup'
        : null,
    },
  };

  // ── RAM ──
  const ramGB = Math.round(hw.ramBytes / 1024 / 1024 / 1024);
  const ramInterp: InterpretedFact = {
    label: 'System Memory',
    value: `${ramGB} GB`,
    basis: ramGB > 0 ? 'detected' : 'unknown',
    reasoning: ramGB > 0 ? 'Total RAM read from OS memory API' : 'RAM size could not be determined',
    verifyHint: null,
  };

  // ── VM ──
  const vmInterp: InterpretedFact = {
    label: 'Virtual Machine',
    value: hw.isVM ? 'Yes' : 'No',
    basis: 'inferred',
    reasoning: hw.isVM
      ? 'System manufacturer string matches known hypervisor vendor names (VMware, QEMU, Hyper-V, Parallels, etc.). This is a heuristic — a bare-metal system with a spoofed manufacturer string would also match.'
      : 'No hypervisor indicators found in system manufacturer string. This could be wrong if the hypervisor masks its identity.',
    verifyHint: hw.isVM ? null : 'If you are running in a VM, make sure the hypervisor is correctly identified — config will differ',
  };

  // ── Overall confidence ──────────────────────────────────────────────────────
  // Confidence scoring:
  //   detected = 3, derived = 2, inferred = 1, unknown = 0
  //
  // We score the 5 most safety-critical facts. Total possible = 15.
  //   high   → 13–15 (all detected, or detected + some derived)
  //   medium → 8–12  (mostly derived, maybe one inferred)
  //   low    → 0–7   (any unknown, or multiple inferred)

  const basisScore: Record<InterpretationBasis, number> = {
    detected: 3,
    derived: 2,
    inferred: 1,
    unknown: 0,
  };

  const criticalFacts: InterpretationBasis[] = [
    cpuInterp.vendor.basis,       // How we knew it was Intel/AMD
    cpuInterp.generation.basis,   // How we chose the config generation
    primaryGpuInterp.vendor.basis,     // How we identified the GPU vendor
    primaryGpuInterp.macosSupport.basis,  // How we assessed macOS support
    boardInterp.model.basis,      // How we applied board-specific quirks
  ];

  const totalScore = criticalFacts.reduce((sum, b) => sum + basisScore[b], 0);
  const hasUnknown = criticalFacts.some(b => b === 'unknown');

  let overallConfidence: 'high' | 'medium' | 'low';
  if (hasUnknown) {
    overallConfidence = 'low';
  } else if (totalScore >= 13) {
    overallConfidence = 'high';
  } else if (totalScore >= 8) {
    overallConfidence = 'medium';
  } else {
    overallConfidence = 'low';
  }

  // ── Summary ──
  const parts: string[] = [];
  parts.push(`${cpuInterp.architecture.value} ${cpuInterp.generation.value} system`);
  if (hw.isLaptop) parts.push('(laptop)');
  if (hw.isVM) parts.push('running in a virtual machine');
  parts.push(`with ${primaryGpuInterp.vendor.value} graphics`);
  if (primaryGpuInterp.macosSupport.value !== 'Unknown' && primaryGpuInterp.macosSupport.value !== 'Cannot determine') {
    parts.push(`— GPU status: ${primaryGpuInterp.macosSupport.value}`);
  }

  // De-duplicate manual verify items
  const uniqueVerify = [...new Set(manualVerify)];

  return {
    overallConfidence,
    summary: parts.join(' '),
    manualVerificationNeeded: uniqueVerify,
    cpu: cpuInterp,
    primaryGpu: primaryGpuInterp,
    allGpus: allGpuInterps,
    board: boardInterp,
    audio: audioInterp,
    network: networkInterp,
    ram: ramInterp,
    vmDetected: vmInterp,
  };
}
