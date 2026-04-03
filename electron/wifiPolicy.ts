import { parseMacOSVersion } from './hackintoshRules.js';

export type WifiChipsetFamily = 'intel' | 'broadcom' | 'other' | 'unknown';

export type BroadcomWifiSupportClass =
  | 'native_pre_sonoma'
  | 'fixup_pre_sonoma'
  | 'legacy_fixup'
  | 'legacy_unsupported_on_target'
  | 'sonoma_root_patch'
  | 'unknown_broadcom';

export interface BroadcomWifiPolicy {
  chipset: string;
  supportClass: BroadcomWifiSupportClass;
  autoKexts: string[];
  summary: string;
}

function normalizeChipset(chipset?: string): string {
  return (chipset ?? '').trim().toLowerCase();
}

export function classifyWifiChipsetFamily(chipset?: string): WifiChipsetFamily {
  const normalized = normalizeChipset(chipset);
  if (!normalized) return 'unknown';
  if (normalized.includes('intel')) return 'intel';
  if (normalized.includes('broadcom')) return 'broadcom';
  return 'other';
}

export function getBroadcomWifiPolicy(chipset: string | undefined, targetOS: string): BroadcomWifiPolicy | null {
  const normalized = normalizeChipset(chipset);
  if (!normalized.includes('broadcom')) return null;

  const osVer = parseMacOSVersion(targetOS);
  const policyBase = { chipset: chipset?.trim() || 'Broadcom Wi-Fi' };

  if (normalized.includes('bcm4352') || normalized.includes('bcm43602')) {
    if (osVer >= 14) {
      return {
        ...policyBase,
        supportClass: 'sonoma_root_patch',
        autoKexts: [],
        summary: 'Broadcom BCM4352/BCM43602-class Wi-Fi loses clean native support on Sonoma and newer, so it now needs OCLP/root patches or a card swap.',
      };
    }
    return {
      ...policyBase,
      supportClass: 'fixup_pre_sonoma',
      autoKexts: ['AirportBrcmFixup.kext'],
      summary: 'Broadcom BCM4352/BCM43602-class Wi-Fi is supported on Ventura and older with AirportBrcmFixup.',
    };
  }

  if (normalized.includes('bcm4360')) {
    if (osVer >= 14) {
      return {
        ...policyBase,
        supportClass: 'sonoma_root_patch',
        autoKexts: [],
        summary: 'Broadcom BCM4360-class Wi-Fi loses clean native support on Sonoma and newer, so it now needs OCLP/root patches or a card swap.',
      };
    }
    return {
      ...policyBase,
      supportClass: 'native_pre_sonoma',
      autoKexts: [],
      summary: 'Broadcom BCM4360-class Wi-Fi is a native-style path on Ventura and older and does not need an injected Wi-Fi kext by default.',
    };
  }

  if (normalized.includes('bcm4331') || normalized.includes('bcm43224')) {
    if (osVer <= 10.15) {
      return {
        ...policyBase,
        supportClass: 'legacy_fixup',
        autoKexts: ['AirportBrcmFixup.kext'],
        summary: 'Broadcom BCM4331/BCM43224-class Wi-Fi is a Catalina-and-older legacy path that still needs AirportBrcmFixup.',
      };
    }
    return {
      ...policyBase,
      supportClass: 'legacy_unsupported_on_target',
      autoKexts: [],
      summary: 'Broadcom BCM4331/BCM43224-class Wi-Fi tops out around Catalina and should not be treated as a normal Big Sur-or-newer path.',
    };
  }

  return {
    ...policyBase,
    supportClass: 'unknown_broadcom',
    autoKexts: [],
    summary: 'Detected a Broadcom Wi-Fi chipset that this generator does not classify cleanly yet. Do not assume a working native wireless path without manual verification.',
  };
}
