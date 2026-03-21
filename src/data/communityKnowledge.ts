// ── Community Knowledge Base ────────────────────────────────────────────────
// Curated dataset of known issues and fixes for specific hardware combinations.
// This is NOT fetched from the internet — it's a static, vetted dataset based on
// Dortania guides, community reports, and documented hardware quirks.

export interface CommunityIssue {
  id: string;
  hardware: {
    cpuGen?: string[];
    gpuVendor?: string[];
    gpuPattern?: string;
    architecture?: string[];
    formFactor?: 'laptop' | 'desktop' | 'any';
  };
  title: string;
  description: string;
  fix: string;
  source: string;
  severity: 'tip' | 'common_fix' | 'critical';
}

export const COMMUNITY_KNOWLEDGE: CommunityIssue[] = [
  // ── AMD-specific ──────────────────────────────────────────────────────────
  {
    id: 'amd-sleep',
    hardware: { architecture: ['AMD'], formFactor: 'any' },
    title: 'Sleep/Wake issues on AMD systems',
    description: 'AMD Hackintosh systems frequently have broken sleep. The system may not wake, or USB devices disconnect after wake.',
    fix: 'Disable sleep entirely in System Settings > Energy. Use a screen saver instead. If you need sleep, try mapping USB ports with USBToolBox and setting darkwake=0.',
    source: 'Dortania Post-Install Guide',
    severity: 'common_fix',
  },
  {
    id: 'amd-adobe',
    hardware: { architecture: ['AMD'], formFactor: 'any' },
    title: 'Adobe apps crash on AMD',
    description: 'Adobe Photoshop, Lightroom, and Premiere Pro may crash due to Intel-specific CPU instructions (MKL/Intel Performance Libraries).',
    fix: 'Install the AMD Vanilla patches from the AMD-OSX GitHub and use the adobe_fixup branch of AMDFriend, or use Rosetta-compatible versions where available.',
    source: 'AMD-OSX Community',
    severity: 'common_fix',
  },
  {
    id: 'amd-iservices',
    hardware: { architecture: ['AMD'], formFactor: 'any' },
    title: 'iMessage/FaceTime not working on AMD',
    description: 'iServices require a valid SMBIOS serial number configuration. AMD systems using MacPro7,1 or iMacPro1,1 need specific serial generation.',
    fix: 'Use GenSMBIOS to generate a valid serial, MLB, and UUID. Verify the serial is NOT in use at checkcoverage.apple.com — it should return "Unable to check coverage".',
    source: 'Dortania iServices Guide',
    severity: 'common_fix',
  },

  // ── Intel iGPU ────────────────────────────────────────────────────────────
  {
    id: 'igpu-hdmi-black',
    hardware: { cpuGen: ['Coffee Lake', 'Comet Lake'], formFactor: 'desktop' },
    title: 'HDMI black screen on Coffee Lake/Comet Lake desktop',
    description: 'Intel UHD 630 iGPU HDMI output often shows a black screen after boot. This is a known framebuffer connector issue.',
    fix: 'Patch the framebuffer connectors in config.plist. Use Hackintool to identify correct connector types and apply device-properties patches for AAPL,ig-platform-id and framebuffer-conX-type.',
    source: 'Dortania GPU Patching Guide',
    severity: 'common_fix',
  },
  {
    id: 'igpu-4k',
    hardware: { cpuGen: ['Skylake', 'Kaby Lake', 'Coffee Lake', 'Comet Lake'], formFactor: 'any' },
    title: '4K display issues on Intel iGPU',
    description: '4K displays may run at low refresh rate or flicker on Intel integrated graphics due to DVMT pre-alloc or DPCD link rate issues.',
    fix: 'Set DVMT pre-alloc to 64MB or higher in BIOS. Add framebuffer-stolenmem and framebuffer-fbmem patches in device properties if BIOS doesn\'t expose the option.',
    source: 'Dortania GPU Patching Guide',
    severity: 'tip',
  },

  // ── Alder Lake / Raptor Lake ──────────────────────────────────────────────
  {
    id: 'alderlake-ecore',
    hardware: { cpuGen: ['Alder Lake', 'Raptor Lake'], formFactor: 'any' },
    title: 'E-cores cause instability',
    description: 'Alder Lake and Raptor Lake hybrid architecture can cause kernel panics or performance issues if E-cores are not properly handled.',
    fix: 'CPUTopologyRebuild kext is included in your build. If you experience instability, try disabling E-cores in BIOS as a test. The ProvideCurrentCpuInfo quirk should also be enabled.',
    source: 'Dortania Alder Lake Guide',
    severity: 'common_fix',
  },

  // ── NVIDIA ────────────────────────────────────────────────────────────────
  {
    id: 'nvidia-modern',
    hardware: { gpuVendor: ['NVIDIA'], gpuPattern: 'RTX|GTX 16|RTX 20|RTX 30|RTX 40' },
    title: 'Modern NVIDIA GPUs have no macOS support',
    description: 'NVIDIA Turing (GTX 16xx, RTX 20xx) and newer GPUs have no macOS drivers. Apple and NVIDIA have not collaborated on drivers since High Sierra.',
    fix: 'There is no fix. Use -wegnoegpu to disable the dGPU and rely on Intel iGPU, or replace the GPU with a supported AMD card (RX 580, RX 5700 XT, RX 6600 XT, etc.).',
    source: 'Dortania GPU Buyers Guide',
    severity: 'critical',
  },
  {
    id: 'nvidia-kepler',
    hardware: { gpuVendor: ['NVIDIA'], gpuPattern: 'GT 7[1-9]0|GTX 7[0-8]0|GTX TITAN' },
    title: 'Kepler GPUs limited to Big Sur',
    description: 'NVIDIA Kepler GPUs (GT 710, GTX 780, etc.) have native macOS drivers but Apple dropped support after Big Sur 11.',
    fix: 'Use macOS Big Sur 11 or older. OpenCore Legacy Patcher can re-enable Kepler drivers on Monterey/Ventura but this is unsupported.',
    source: 'Dortania GPU Buyers Guide',
    severity: 'common_fix',
  },

  // ── Audio ─────────────────────────────────────────────────────────────────
  {
    id: 'audio-layout',
    hardware: { formFactor: 'any' },
    title: 'No audio output after installation',
    description: 'Audio not working is usually caused by an incorrect layout-id for your motherboard\'s audio codec.',
    fix: 'Find your audio codec model (usually ALC897, ALC1220, etc.) and try different alcid values. Check the AppleALC wiki for supported layout IDs for your specific codec.',
    source: 'AppleALC Supported Codecs',
    severity: 'common_fix',
  },

  // ── USB ───────────────────────────────────────────────────────────────────
  {
    id: 'usb-15port',
    hardware: { formFactor: 'any' },
    title: 'USB ports not working (15-port limit)',
    description: 'macOS has a 15-port USB limit. If your motherboard has more than 15 USB ports (counting USB 2.0 and 3.0 personalities separately), some will not work.',
    fix: 'Create a USB map using USBToolBox or Hackintool. This creates a kext that tells macOS which ports to use. Must be done post-install.',
    source: 'Dortania USB Mapping Guide',
    severity: 'common_fix',
  },

  // ── Networking ────────────────────────────────────────────────────────────
  {
    id: 'wifi-intel-airdrop',
    hardware: { formFactor: 'any' },
    title: 'No AirDrop or Handoff with Intel Wi-Fi',
    description: 'Intel Wi-Fi cards work for basic Wi-Fi connectivity but cannot support AirDrop, Handoff, Universal Clipboard, or Continuity Camera.',
    fix: 'This is a fundamental limitation of the Intel Wi-Fi kext. For full Apple wireless ecosystem features, replace with a Broadcom BCM94360NG (M.2) or BCM94360CS2 (with adapter).',
    source: 'OpenIntelWireless Documentation',
    severity: 'tip',
  },

  // ── Boot issues ───────────────────────────────────────────────────────────
  {
    id: 'boot-stuck-apfs',
    hardware: { formFactor: 'any' },
    title: 'Boot stuck at [EB|#LOG:EXITBS:START]',
    description: 'The most common OpenCore boot failure. Usually caused by incorrect GPU configuration, missing kexts, or wrong SMBIOS.',
    fix: 'Boot with -v (verbose) to see the actual error. Common fixes: verify WhateverGreen is loaded, check AAPL,ig-platform-id for iGPU, ensure all kexts are properly signed and loaded.',
    source: 'Dortania Troubleshooting Guide',
    severity: 'common_fix',
  },
  {
    id: 'boot-kernel-panic-appleintelcpu',
    hardware: { architecture: ['AMD'], formFactor: 'any' },
    title: 'Kernel panic: AppleIntelCPUPowerManagement',
    description: 'This panic occurs because macOS tries to use Intel-specific CPU power management on an AMD system.',
    fix: 'Ensure the AMD kernel patches are applied in config.plist. The NullCPUPowerManagement approach is outdated — use the proper kernel patches from the AMD-OSX patches repository.',
    source: 'Dortania AMD Guide',
    severity: 'critical',
  },

  // ── Post-install ──────────────────────────────────────────────────────────
  {
    id: 'postinstall-updates',
    hardware: { formFactor: 'any' },
    title: 'macOS updates may break your setup',
    description: 'macOS updates can break kext compatibility or change driver behavior. Always have a backup EFI and test updates on a secondary partition first.',
    fix: 'Before updating: 1) Backup your working EFI folder, 2) Check acidanthera releases for kext updates, 3) Update OpenCore and all kexts BEFORE updating macOS, 4) Test on a clone if possible.',
    source: 'Dortania Post-Install Guide',
    severity: 'tip',
  },
  {
    id: 'postinstall-oc-from-disk',
    hardware: { formFactor: 'any' },
    title: 'Copy OpenCore to internal drive after install',
    description: 'After installing macOS, you should copy the OpenCore EFI from the USB drive to your internal drive\'s EFI partition so you can boot without the USB.',
    fix: 'Mount both EFI partitions (USB and internal) using MountEFI or diskutil. Copy the entire EFI folder from the USB to the internal drive\'s EFI partition.',
    source: 'Dortania Post-Install Guide',
    severity: 'tip',
  },

  // ── Storage ───────────────────────────────────────────────────────────────
  {
    id: 'nvme-samsung',
    hardware: { formFactor: 'any' },
    title: 'Samsung PM981/PM991 NVMe Kernel Panics',
    description: 'Samsung PM981, PM991, and some other OEM NVMe drives are known to cause random kernel panics in macOS.',
    fix: 'Use NVMeFix.kext. If issues persist, the drive may need to be replaced with a macOS-compatible one (e.g., WD SN850X). Avoid Samsung OEM drives if possible.',
    source: 'Dortania NVMe Guide',
    severity: 'critical',
  },
  {
    id: 'nvme-micron',
    hardware: { formFactor: 'any' },
    title: 'Micron/Crucial NVMe slow boot times',
    description: 'Some Micron 2200S and Crucial P2 drives experience extremely slow boot times or controller freezes.',
    fix: 'Ensure the NVMeFix kext is loaded. If the drive continues to stall or freeze during boot, it may require replacement.',
    source: 'Dortania NVMe Guide',
    severity: 'common_fix',
  },

  // ── Network ───────────────────────────────────────────────────────────────
  {
    id: 'ethernet-i225',
    hardware: { formFactor: 'desktop', cpuGen: ['Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake'] },
    title: 'Intel I225-V Ethernet not working',
    description: 'The Intel I225-V 2.5GbE controller needs specific boot arguments or kexts to work in macOS Monterey and later.',
    fix: 'Use AppleIGC.kext for macOS Monterey and newer versions to ensure reliable connectivity.',
    source: 'Dortania Ethernet Guide',
    severity: 'common_fix',
  },
  {
    id: 'wifi-broadcom-sonoma',
    hardware: { formFactor: 'any' },
    title: 'Broadcom Wi-Fi broken in macOS Sonoma',
    description: 'Apple removed support for older Broadcom Wi-Fi cards (like BCM94360NG) in macOS Sonoma 14.',
    fix: 'Use OpenCore Legacy Patcher (OCLP) to apply root patches for modern Broadcom Wi-Fi, or switch to Intel Wi-Fi via AirportItlwm if you prefer an unpatched system.',
    source: 'OpenCore Legacy Patcher Documentation',
    severity: 'critical',
  }
];

/** Find relevant community issues for a given hardware profile. */
export function getRelevantIssues(profile: {
  architecture: string;
  generation: string;
  gpu: string;
  isLaptop: boolean;
  kexts?: string[];
}): CommunityIssue[] {
  return COMMUNITY_KNOWLEDGE.filter(issue => {
    const hw = issue.hardware;

    if (hw.architecture && !hw.architecture.includes(profile.architecture)) return false;
    if (hw.cpuGen && !hw.cpuGen.includes(profile.generation)) return false;
    if (hw.formFactor && hw.formFactor !== 'any') {
      if (hw.formFactor === 'laptop' && !profile.isLaptop) return false;
      if (hw.formFactor === 'desktop' && profile.isLaptop) return false;
    }
    if (hw.gpuVendor) {
      const gpuLower = profile.gpu.toLowerCase();
      const match = hw.gpuVendor.some(v => gpuLower.includes(v.toLowerCase()));
      if (!match) return false;
    }
    if (hw.gpuPattern) {
      const re = new RegExp(hw.gpuPattern, 'i');
      if (!re.test(profile.gpu)) return false;
    }

    return true;
  });
}
