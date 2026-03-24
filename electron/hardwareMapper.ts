/**
 * Shared hardware-to-profile mapping functions.
 *
 * Extracted from main.ts so they can be used by both the Electron app
 * and the headless CLI without pulling in Electron dependencies.
 */

import os from 'os';
import type { DetectedHardware } from './hardwareDetect.js';
import type { HardwareProfile } from './configGenerator.js';
import { getSMBIOSForProfile, resolveAudioLayoutId } from './configGenerator.js';

export function detectCpuGeneration(cpuModel: string): HardwareProfile['generation'] {
  const model = cpuModel.toLowerCase();
  if (model.includes('apple') || model.includes('m1') || model.includes('m2') || model.includes('m3') || model.includes('m4')) return 'Apple Silicon';

  // High-End Desktop / Servers
  if (model.includes('xeon')) {
    if (model.includes('w-') || model.includes('scalable')) return 'Cascade Lake-X';
    if (model.includes('e5-v4') || model.includes('e5-v3')) return 'Broadwell-E';
    if (model.includes('e5-v2')) return 'Ivy Bridge-E';
    return 'Haswell-E';
  }

  // Standard Core i series
  const match = model.match(/i\d-? ?(1?\d{4})/);
  if (match) {
    const num = parseInt(match[1]);
    if (num >= 10000 && num < 11000 && (/\bg[147]\b/.test(model) || model.includes('ice lake'))) return 'Ice Lake';
    if (num >= 14000) return 'Raptor Lake';
    if (num >= 13000) return 'Raptor Lake';
    if (num >= 12000) return 'Alder Lake';
    if (num >= 11000) return 'Rocket Lake';
    if (num >= 10000) return 'Comet Lake';
    if (num >= 8000) return 'Coffee Lake';
    if (num >= 7000) return 'Kaby Lake';
    if (num >= 6000) return 'Skylake';
    if (num >= 5000) return 'Broadwell';
    if (num >= 4000) return 'Haswell';
    if (num >= 3000) return 'Ivy Bridge';
    if (num >= 2000) return 'Sandy Bridge';
  }

  const legacyCoreMatch = model.match(/i[357]-?\s*(\d{3,4})([a-z]{0,2})/);
  if (legacyCoreMatch) {
    const num = parseInt(legacyCoreMatch[1], 10);
    const suffix = legacyCoreMatch[2] ?? '';
    if (num >= 900 && num < 1000) return 'Nehalem';
    if (num >= 800 && num < 900) return 'Nehalem';
    if (num >= 700 && num < 800) return 'Westmere';
    if (num >= 600 && num < 700) return 'Clarkdale';
    if (num >= 400 && num < 600) {
      if (/[lmqu]/.test(suffix)) return 'Arrandale';
      return 'Clarkdale';
    }
  }

  // Budget Intel Desktop
  if (model.includes('pentium') || model.includes('celeron')) {
    if (model.includes('gold')) return 'Coffee Lake';
    if (model.match(/g[45]\d{2}/)) return 'Skylake';
    if (model.match(/g3\d{2}/)) return 'Haswell';
    if (model.match(/g[2|1]\d{2}/) || model.match(/g[68]\d0/)) return 'Sandy Bridge';
    return 'Ivy Bridge';
  }

  // Legacy Intel Desktop
  if (model.includes('core 2 quad') || /\bq9\d{3}\b/.test(model)) return 'Yorkfield';
  if (model.includes('core 2 duo') || /\be8\d{3}\b/.test(model) || /\be7\d{3}\b/.test(model)) return 'Wolfdale';
  if (model.includes('core 2') || model.includes('quad') || model.includes('extreme')) return 'Penryn';

  // AMD Desktop
  if (model.includes('threadripper')) return 'Threadripper';
  if (model.includes('ryzen')) return 'Ryzen';
  if (model.includes('fx-') || model.includes('phenom') || model.includes('athlon')) return 'Bulldozer';
  return 'Unknown';
}

export function detectArchitecture(cpuModel: string): HardwareProfile['architecture'] {
  const model = cpuModel.toLowerCase();
  if (model.includes('apple') || model.includes('m1') || model.includes('m2') || model.includes('m3') || model.includes('m4')) return 'Apple Silicon';
  if (model.includes('ryzen') || model.includes('threadripper') || model.includes('amd')) return 'AMD';
  if (model.includes('intel') || model.match(/i\d-/)) return 'Intel';
  return 'Unknown';
}

export function mapDetectedToProfile(hw: DetectedHardware): HardwareProfile {
  const cpuModel = hw.cpu.name;
  const gpuModel = hw.gpus.map(g => g.name).join(' / ') || 'Unknown GPU';
  const generation = detectCpuGeneration(cpuModel);
  const architecture = detectArchitecture(cpuModel);
  const ramGB = (hw.ramBytes / 1024 / 1024 / 1024).toFixed(0) + ' GB';

  const confidences = [hw.cpu.confidence, hw.primaryGpu.confidence];
  let scanConfidence: 'high' | 'medium' | 'low';
  if (confidences.every(c => c === 'detected')) {
    scanConfidence = 'high';
  } else if (confidences.some(c => c === 'unverified')) {
    scanConfidence = 'low';
  } else {
    scanConfidence = 'medium';
  }

  // Resolve audio codec from detected audio devices
  const primaryAudio = hw.audioDevices?.find(a => a.codecName !== null);
  const audioCodec = primaryAudio?.codecName ?? undefined;
  const audioLayoutId = resolveAudioLayoutId(audioCodec);

  // Resolve network adapters
  const primaryEthernet = hw.networkDevices?.find(n => n.type === 'ethernet' && n.adapterFamily);
  const primaryWifi = hw.networkDevices?.find(n => n.type === 'wifi' && n.adapterFamily);
  const nicChipset = primaryEthernet?.adapterFamily ?? undefined;
  const wifiChipset = primaryWifi?.adapterFamily ?? undefined;

  const profile: HardwareProfile = {
    cpu: cpuModel,
    architecture,
    generation,
    coreCount: hw.coreCount,
    gpu: gpuModel,
    gpuDevices: hw.gpus.map(gpu => ({
      name: gpu.name,
      vendorName: gpu.vendorName,
      vendorId: gpu.vendorId,
      deviceId: gpu.deviceId,
    })),
    ram: ramGB,
    motherboard: hw.motherboardModel || hw.motherboardVendor || 'Unknown',
    targetOS: 'macOS Sequoia 15.x',
    smbios: '',
    kexts: [], ssdts: [],
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop: hw.isLaptop,
    isVM: hw.isVM,
    audioCodec,
    audioLayoutId,
    nicChipset,
    wifiChipset,
    scanConfidence,
  };
  profile.smbios = getSMBIOSForProfile(profile);
  return profile;
}
