import { parseMacOSVersion } from '../../electron/hackintoshRules.js';

export interface KextEntry {
  name: string;
  repo: string;
  required: 'always' | 'conditional' | 'optional';
  category: 'must-have' | 'gpu' | 'audio' | 'ethernet' | 'wifi' | 'usb' | 'laptop' | 'extras' | 'amd';
  description: string;
  conditions?: string[];
  exclusions?: string[];
  minOS?: string;
  maxOS?: string;
  dependsOn?: string[];
  orderingAfter?: string[];
  canonicality?: 'canonical' | 'canonical_with_caveats' | 'legacy' | 'temporary_mapping_only';
  validationRule?: string;
}

export interface KextSelectionOptions {
  isAMD: boolean;
  isLaptop: boolean;
  isHEDT: boolean;
  isAlderLake: boolean;
  nicChipset: string;
  nvmeModel: string;
  macOSVersion: string;
  wifiChipset?: string;
  wifiMode?: 'airportitlwm' | 'itlwm' | 'none';
  secureBootModelEnabled?: boolean;
  isDualSocketIntel?: boolean;
  motherboard?: string;
}

export interface KextDecisionIssue {
  code: string;
  severity: 'warning' | 'blocked';
  message: string;
}

export function isMontereyOrNewer(macOSVersion: string): boolean {
  return parseMacOSVersion(macOSVersion) >= 12;
}

export function needsAppleMceReporterDisabler(opts: {
  isAMD: boolean;
  macOSVersion: string;
  isDualSocketIntel?: boolean;
}): boolean {
  if (opts.isAMD) return parseMacOSVersion(opts.macOSVersion) >= 12;
  return Boolean(opts.isDualSocketIntel) && parseMacOSVersion(opts.macOSVersion) >= 10.15;
}

export function requiresSecureBootModelForAirportItlwm(selectedKexts: string[]): boolean {
  return selectedKexts.includes('AirportItlwm');
}

export function shouldSelectXhciUnsupported(opts: {
  isAMD: boolean;
  isHEDT: boolean;
  motherboard?: string;
  macOSVersion: string;
}): boolean {
  if (opts.isAMD) return false;
  const board = (opts.motherboard ?? '').toLowerCase();
  if (opts.isHEDT && (board.includes('x79') || board.includes('x99'))) return true;
  if (board.includes('h370') || board.includes('b360') || board.includes('h310')) return true;
  if (board.includes('z390') && parseMacOSVersion(opts.macOSVersion) <= 11) return true;
  return false;
}

export const KEXT_REGISTRY: KextEntry[] = [
  {
    name: 'Lilu',
    repo: 'acidanthera/Lilu',
    required: 'always',
    category: 'must-have',
    description: 'Core patching framework required by most plugin kexts.',
    minOS: '10.4',
    canonicality: 'canonical',
    validationRule: 'Required whenever a Lilu plugin is selected.',
  },
  {
    name: 'VirtualSMC',
    repo: 'acidanthera/VirtualSMC',
    required: 'always',
    category: 'must-have',
    description: 'SMC emulation required for macOS to boot.',
    minOS: '10.4',
    canonicality: 'canonical',
    validationRule: 'Always required for standard OpenCore systems.',
  },
  {
    name: 'WhateverGreen',
    repo: 'acidanthera/WhateverGreen',
    required: 'always',
    category: 'gpu',
    description: 'Graphics fixups for Intel iGPU, AMD dGPU, HDMI audio, and display routing.',
    minOS: '10.6',
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    canonicality: 'canonical',
    validationRule: 'Requires Lilu.',
  },
  {
    name: 'NootRX',
    repo: 'ChefKissInc/NootRX',
    required: 'conditional',
    category: 'gpu',
    description: 'Navi 22 GPU patch path for RX 6700/6750-class cards.',
    conditions: ['AMD Navi 22 only'],
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    canonicality: 'canonical_with_caveats',
    validationRule: 'Use instead of WhateverGreen on Navi 22.',
  },
  {
    name: 'NootedRed',
    repo: 'ChefKissInc/NootedRed',
    required: 'conditional',
    category: 'gpu',
    description: 'AMD Vega APU graphics patch path.',
    conditions: ['AMD Vega APU only'],
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    canonicality: 'canonical_with_caveats',
    validationRule: 'Use for documented Vega APU paths instead of WhateverGreen.',
  },
  {
    name: 'AppleALC',
    repo: 'acidanthera/AppleALC',
    required: 'always',
    category: 'audio',
    description: 'Native AppleHDA audio path for supported codecs via layout-id/alcid.',
    minOS: '10.8',
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    exclusions: ['VoodooHDA'],
    canonicality: 'canonical',
    validationRule: 'Requires Lilu and should not be paired with VoodooHDA.',
  },
  {
    name: 'IntelMausi',
    repo: 'acidanthera/IntelMausi',
    required: 'conditional',
    category: 'ethernet',
    description: 'Intel Ethernet driver for common I217/I218/I219-class controllers.',
    conditions: ['Intel NIC except I211 on Monterey+'],
    minOS: '10.9',
    canonicality: 'canonical',
  },
  {
    name: 'AppleIGB',
    repo: 'donatengit/AppleIGB',
    required: 'conditional',
    category: 'ethernet',
    description: 'Preferred driver for Intel I211 on Monterey and newer.',
    conditions: ['Intel I211', 'macOS 12+'],
    minOS: '12.0',
    exclusions: ['IntelMausi when I211 is detected'],
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'RealtekRTL8111',
    repo: 'Mieze/RTL8111_driver_for_OS_X',
    required: 'conditional',
    category: 'ethernet',
    description: 'Driver for standard Realtek gigabit Ethernet controllers.',
    conditions: ['Realtek 1G NIC'],
    minOS: '10.8',
    canonicality: 'canonical',
  },
  {
    name: 'RTL812xLucy',
    repo: 'Mieze/RTL812xLucy',
    required: 'conditional',
    category: 'ethernet',
    description: 'Driver for Realtek 2.5G/5G RTL8125/RTL8126-class Ethernet controllers.',
    conditions: ['Realtek 2.5G/5G NIC'],
    minOS: '10.15',
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'AtherosE2200Ethernet',
    repo: 'Mieze/AtherosE2200Ethernet',
    required: 'conditional',
    category: 'ethernet',
    description: 'Ethernet driver for Atheros and Killer NICs except Realtek-based variants.',
    conditions: ['Atheros NIC', 'Killer non-E2500'],
    minOS: '10.8',
    canonicality: 'canonical',
  },
  {
    name: 'AirportItlwm',
    repo: 'OpenIntelWireless/itlwm',
    required: 'optional',
    category: 'wifi',
    description: 'Intel Wi-Fi driver with native AirPort integration.',
    conditions: ['Intel Wi-Fi', 'SecureBootModel enabled path'],
    minOS: '10.13',
    dependsOn: [],
    canonicality: 'canonical_with_caveats',
    validationRule: 'Blocked if SecureBootModel is not enabled.',
  },
  {
    name: 'Itlwm',
    repo: 'OpenIntelWireless/itlwm',
    required: 'optional',
    category: 'wifi',
    description: 'Intel Wi-Fi fallback driver that uses HeliPort instead of native AirPort integration.',
    conditions: ['Intel Wi-Fi', 'No SecureBootModel-enabled path'],
    minOS: '10.13',
    canonicality: 'canonical_with_caveats',
    validationRule: 'Warn that Recovery Wi-Fi is unavailable and HeliPort is required.',
  },
  {
    name: 'IntelBluetoothFirmware',
    repo: 'OpenIntelWireless/IntelBluetoothFirmware',
    required: 'optional',
    category: 'wifi',
    description: 'Bluetooth firmware uploader for Intel cards.',
    conditions: ['Intel Bluetooth'],
    minOS: '10.13',
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'AirportBrcmFixup',
    repo: 'acidanthera/AirportBrcmFixup',
    required: 'conditional',
    category: 'wifi',
    description: 'Broadcom Wi-Fi patch set for supported non-native card families.',
    conditions: ['Broadcom BCM4352/BCM43602 or legacy Broadcom Wi-Fi path'],
    minOS: '10.10',
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'BrcmPatchRAM',
    repo: 'acidanthera/BrcmPatchRAM',
    required: 'optional',
    category: 'wifi',
    description: 'Broadcom Bluetooth firmware uploader and patch set.',
    conditions: ['Broadcom Bluetooth path'],
    minOS: '10.10',
    canonicality: 'canonical',
  },
  {
    name: 'BlueToolFixup',
    repo: 'acidanthera/BrcmPatchRAM',
    required: 'conditional',
    category: 'wifi',
    description: 'Monterey-and-newer Bluetooth fixup for third-party Bluetooth paths.',
    conditions: ['macOS 12+', 'Third-party Bluetooth'],
    minOS: '12.0',
    canonicality: 'canonical',
  },
  {
    name: 'USBInjectAll',
    repo: 'RehabMan/OS-X-USB-Inject-All',
    required: 'conditional',
    category: 'usb',
    description: 'Temporary USB mapping helper for legacy Intel systems only.',
    conditions: ['Temporary Intel USB mapping only'],
    exclusions: ['General AMD usage'],
    canonicality: 'legacy',
  },
  {
    name: 'XHCI-unsupported',
    repo: 'RehabMan/OS-X-USB-Inject-All',
    required: 'conditional',
    category: 'usb',
    description: 'Controller workaround for documented Intel/HEDT USB controllers.',
    conditions: ['X79/X99 HEDT', 'H370/B360/H310', 'Some older Z390 cases'],
    exclusions: ['Generic AMD systems'],
    canonicality: 'canonical_with_caveats',
    validationRule: 'Do not auto-select merely because the platform is AMD.',
  },
  {
    name: 'NVMeFix',
    repo: 'acidanthera/NVMeFix',
    required: 'conditional',
    category: 'extras',
    description: 'NVMe power-management and compatibility fixes for problem drives.',
    conditions: ['Problem NVMe drive such as PM981/PM991/2200S/600p'],
    minOS: '10.14',
    canonicality: 'canonical',
  },
  {
    name: 'AppleMCEReporterDisabler',
    repo: 'acidanthera/AppleMCEReporterDisabler',
    required: 'conditional',
    category: 'extras',
    description: 'Kernel panic avoidance for AMD on Monterey+ and dual-socket Intel on Catalina+.',
    conditions: ['AMD + Monterey+', 'Dual-socket Intel + Catalina+'],
    minOS: '10.15',
    canonicality: 'canonical',
    validationRule: 'Should only be selected on documented affected CPU/OS combinations.',
  },
  {
    name: 'CpuTscSync',
    repo: 'lvs1974/CpuTscSync',
    required: 'conditional',
    category: 'extras',
    description: 'TSC sync for Intel HEDT/server platforms.',
    conditions: ['Intel HEDT/server only'],
    minOS: '10.8',
    exclusions: ['AMD'],
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'RestrictEvents',
    repo: 'acidanthera/RestrictEvents',
    required: 'optional',
    category: 'extras',
    description: 'Assorted macOS behavior patches including update/workaround paths on newer releases.',
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    canonicality: 'canonical',
  },
  {
    name: 'CPUTopologyRebuild',
    repo: 'b00t0x/CpuTopologyRebuild',
    required: 'conditional',
    category: 'extras',
    description: 'Alder Lake-only heterogeneous-core rebuild helper.',
    conditions: ['Alder Lake only'],
    dependsOn: ['Lilu'],
    orderingAfter: ['Lilu'],
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'SMCProcessor',
    repo: 'acidanthera/VirtualSMC',
    required: 'conditional',
    category: 'extras',
    description: 'VirtualSMC sensor plugin for Intel CPU temperatures.',
    conditions: ['Intel CPU'],
    dependsOn: ['VirtualSMC'],
    canonicality: 'canonical',
  },
  {
    name: 'SMCSuperIO',
    repo: 'acidanthera/VirtualSMC',
    required: 'conditional',
    category: 'extras',
    description: 'VirtualSMC Super I/O sensor plugin for fan and board sensors.',
    conditions: ['Intel desktop board sensors'],
    dependsOn: ['VirtualSMC'],
    canonicality: 'canonical',
  },
  {
    name: 'SMCAMDProcessor',
    repo: 'trulyspinach/SMCAMDProcessor',
    required: 'conditional',
    category: 'amd',
    description: 'AMD sensor plugin and companion monitoring path.',
    conditions: ['AMD Ryzen/Threadripper'],
    dependsOn: ['VirtualSMC'],
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'VoodooPS2Controller',
    repo: 'acidanthera/VoodooPS2',
    required: 'conditional',
    category: 'laptop',
    description: 'PS/2 keyboard and trackpad driver for laptops that still use PS/2 input devices.',
    conditions: ['Laptop with PS/2 keyboard or trackpad'],
    canonicality: 'canonical',
  },
  {
    name: 'VoodooI2C',
    repo: 'VoodooI2C/VoodooI2C',
    required: 'conditional',
    category: 'laptop',
    description: 'I2C input driver for supported touchpads and touchscreens.',
    conditions: ['Laptop with I2C input devices'],
    canonicality: 'canonical',
  },
  {
    name: 'VoodooSMBus',
    repo: 'VoodooSMBus/VoodooSMBus',
    required: 'conditional',
    category: 'laptop',
    description: 'SMBus laptop input support for a smaller subset of laptops.',
    conditions: ['Laptop with SMBus touchpad'],
    canonicality: 'canonical_with_caveats',
  },
  {
    name: 'AMDRyzenCPUPowerManagement',
    repo: 'trulyspinach/SMCAMDProcessor',
    required: 'conditional',
    category: 'amd',
    description: 'AMD CPU power-management driver for Ryzen/Threadripper.',
    conditions: ['Ryzen/Threadripper desktop'],
    canonicality: 'canonical',
  },
];

export function getKextsForProfile(opts: KextSelectionOptions): KextEntry[] {
  return KEXT_REGISTRY.filter(kext => {
    if (kext.required === 'always') return true;

    if (kext.category === 'amd' && !opts.isAMD) return false;
    if (kext.category === 'laptop' && !opts.isLaptop) return false;
    if (kext.name === 'CpuTscSync' && !opts.isHEDT) return false;
    if (kext.name === 'CPUTopologyRebuild' && !opts.isAlderLake) return false;
    if (kext.name === 'SMCProcessor' && opts.isAMD) return false;
    if (kext.name === 'SMCSuperIO' && opts.isAMD) return false;
    if (kext.name === 'NVMeFix' && !opts.nvmeModel) return false;
    if (kext.name === 'AppleMCEReporterDisabler' && !needsAppleMceReporterDisabler(opts)) return false;
    if (kext.name === 'AirportItlwm' && (opts.wifiMode !== 'airportitlwm' || !opts.secureBootModelEnabled)) return false;
    if (kext.name === 'Itlwm' && opts.wifiMode !== 'itlwm') return false;
    if (kext.name === 'BlueToolFixup' && !isMontereyOrNewer(opts.macOSVersion)) return false;
    if (kext.name === 'XHCI-unsupported' && !shouldSelectXhciUnsupported(opts)) return false;
    if (kext.name === 'USBInjectAll' && opts.isAMD) return false;

    if (kext.category === 'ethernet') {
      const nic = opts.nicChipset.toLowerCase();
      if (kext.name === 'AppleIGB') return nic.includes('i211') && isMontereyOrNewer(opts.macOSVersion);
      if (kext.name === 'IntelMausi') return nic.includes('intel') && !nic.includes('i211');
      if (kext.name === 'RTL812xLucy') return nic.includes('8125') || nic.includes('8126') || nic.includes('2.5g');
      if (kext.name === 'RealtekRTL8111') return nic.includes('realtek') && !nic.includes('8125') && !nic.includes('8126') && !nic.includes('2.5g');
      if (kext.name === 'AtherosE2200Ethernet') return nic.includes('atheros') || (nic.includes('killer') && !nic.includes('e2500'));
    }

    return kext.required === 'conditional' || kext.required === 'optional';
  });
}

export function validateKextSelection(selectedKexts: string[], opts: {
  macOSVersion: string;
  isAMD: boolean;
  secureBootModelEnabled?: boolean;
  isDualSocketIntel?: boolean;
  motherboard?: string;
}): KextDecisionIssue[] {
  const issues: KextDecisionIssue[] = [];
  const selected = new Set(selectedKexts);

  const liluPlugins = ['WhateverGreen', 'AppleALC', 'RestrictEvents', 'CPUTopologyRebuild', 'NootRX', 'NootedRed'];
  if (liluPlugins.some(plugin => selected.has(plugin)) && !selected.has('Lilu')) {
    issues.push({
      code: 'KEXT_LILU_DEPENDENCY',
      severity: 'blocked',
      message: 'Lilu plugin selected without Lilu.',
    });
  }

  if (selected.has('AirportItlwm') && !opts.secureBootModelEnabled) {
    issues.push({
      code: 'KEXT_AIRPORTITLWM_SECUREBOOT_REQUIRED',
      severity: 'blocked',
      message: 'AirportItlwm requires SecureBootModel-enabled behavior for native and Recovery networking.',
    });
  }

  if (selected.has('Itlwm')) {
    issues.push({
      code: 'KEXT_ITLWM_RECOVERY_LIMITATION',
      severity: 'warning',
      message: 'Itlwm is not a native AirPort path and does not provide Recovery Wi-Fi.',
    });
  }

  if (selected.has('BlueToolFixup') && !isMontereyOrNewer(opts.macOSVersion)) {
    issues.push({
      code: 'KEXT_BLUETOOLFIXUP_UNNEEDED',
      severity: 'warning',
      message: 'BlueToolFixup should only be used on Monterey and newer.',
    });
  }

  const appleMceNeeded = needsAppleMceReporterDisabler(opts);
  if (selected.has('AppleMCEReporterDisabler') && !appleMceNeeded) {
    issues.push({
      code: 'KEXT_APPLEMCE_MISMATCH',
      severity: 'warning',
      message: 'AppleMCEReporterDisabler is selected on a CPU/OS combination that is not documented as requiring it.',
    });
  }
  if (!selected.has('AppleMCEReporterDisabler') && appleMceNeeded) {
    issues.push({
      code: 'KEXT_APPLEMCE_REQUIRED',
      severity: 'blocked',
      message: 'AppleMCEReporterDisabler is required for this documented CPU/OS combination.',
    });
  }

  if (selected.has('XHCI-unsupported') && !shouldSelectXhciUnsupported({
    isAMD: opts.isAMD,
    isHEDT: false,
    motherboard: opts.motherboard,
    macOSVersion: opts.macOSVersion,
  })) {
    issues.push({
      code: 'KEXT_XHCI_UNSUPPORTED_OVERSELECTED',
      severity: 'warning',
      message: 'XHCI-unsupported is selected outside the documented Intel/HEDT controller cases.',
    });
  }

  return issues;
}
