import type { AppVersionInfo, DetectedHardware, HardwareProfile } from '../bridge/types';

/**
 * Things the user must know about where the scan ran:
 * - `apple_silicon`: the scan describes an Apple silicon Mac, which cannot use an OpenCore EFI.
 * - `real_mac`: an Intel Mac booted natively; it runs macOS without an EFI from this app.
 * - `hackintosh`: scanned on macOS through OpenCore; ids and SMBIOS come from the running config.
 * - `linux_acpi`: Linux without root; the ACPI tables could not be read.
 * - `apple_profile`: an imported (or edited) profile describes an Apple silicon Mac.
 */
export type ScanNotice = 'apple_silicon' | 'real_mac' | 'hackintosh' | 'linux_acpi' | 'apple_profile';

/** The app itself runs natively on an Apple silicon Mac. */
export function isAppleSiliconHost(info: AppVersionInfo | null): boolean {
  return info?.hostOs === 'macos' && info.arch === 'aarch64';
}

/** The profile describes a Mac with an Apple CPU (no OpenCore EFI can be built for it). */
export function isAppleProfile(profile: HardwareProfile | null): boolean {
  return profile !== null && (profile.cpu.platform === 'apple_silicon' || profile.cpu.vendor === 'apple');
}

export function scanNotices(detected: DetectedHardware | null, profile: HardwareProfile | null): ScanNotice[] {
  const notices: ScanNotice[] = [];
  // Only a scan of this computer says anything about the host.
  if (profile && profile.source !== 'scan') return notices;
  if (isAppleProfile(profile)) return ['apple_silicon'];
  if (!detected) return notices;
  if (detected.hostOs === 'macos') {
    const throughOpenCore = detected.warnings.some((w) => /opencore/i.test(w));
    const genuine = !throughOpenCore && detected.firmware.biosVendor?.trim().toLowerCase() === 'apple';
    notices.push(genuine ? 'real_mac' : 'hackintosh');
  }
  if (detected.hostOs === 'linux' && !detected.acpiTablesDir) notices.push('linux_acpi');
  return notices;
}

/**
 * Notices for the profile in use: what the scan says about this host, or for a
 * profile that did not come from a scan only whether it describes a Mac no EFI
 * can be built for.
 */
export function profileNotices(detected: DetectedHardware | null, profile: HardwareProfile | null): ScanNotice[] {
  if (profile && profile.source !== 'scan') return isAppleProfile(profile) ? ['apple_profile'] : [];
  return scanNotices(detected, profile);
}

/** Notices that need the user's decision first: they keep the app on the scan step instead of moving on. */
export function holdsScanStep(notices: readonly ScanNotice[]): boolean {
  // A Hackintosh scanning itself (to rebuild its own EFI) moves on; the hardware step repeats the warning.
  return notices.some((n) => n !== 'hackintosh');
}

/** The profile cannot be used to build an EFI at all. */
export function blocksBuild(notices: readonly ScanNotice[]): boolean {
  return notices.includes('apple_silicon') || notices.includes('apple_profile');
}

/**
 * Commands that start the app as root on Linux so the ACPI tables can be read.
 * `-E` keeps the display variables; `-H` points HOME at /root, otherwise the
 * root session would leave root-owned logs, state and scan folders in the
 * user's own app data folder (sudo keeps HOME with -E unless always_set_home
 * is set), which the next normal start can no longer write.
 */
export const LINUX_ROOT_COMMANDS = {
  wayland: 'xhost +si:localuser:root',
  deb: 'sudo -EH opcore-oneclick',
  appImage: 'sudo -EH ./OpCore-OneClick_*.AppImage',
} as const;
