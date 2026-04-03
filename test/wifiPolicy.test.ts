import { describe, expect, it } from 'vitest';
import { classifyWifiChipsetFamily, getBroadcomWifiPolicy } from '../electron/wifiPolicy.js';

describe('wifiPolicy', () => {
  it('classifies Intel and Broadcom chipset families separately', () => {
    expect(classifyWifiChipsetFamily('Intel Wi-Fi 6 AX200')).toBe('intel');
    expect(classifyWifiChipsetFamily('Broadcom BCM4352')).toBe('broadcom');
    expect(classifyWifiChipsetFamily('Qualcomm Atheros AR9485')).toBe('other');
    expect(classifyWifiChipsetFamily(undefined)).toBe('unknown');
  });

  it('treats BCM4352 as AirportBrcmFixup on Ventura and older', () => {
    const policy = getBroadcomWifiPolicy('Broadcom BCM4352', 'macOS Ventura');
    expect(policy?.supportClass).toBe('fixup_pre_sonoma');
    expect(policy?.autoKexts).toEqual(['AirportBrcmFixup.kext']);
  });

  it('treats BCM4360 as a native-style pre-Sonoma path', () => {
    const policy = getBroadcomWifiPolicy('Broadcom BCM4360', 'macOS Ventura');
    expect(policy?.supportClass).toBe('native_pre_sonoma');
    expect(policy?.autoKexts).toEqual([]);
  });

  it('marks BCM4352 as patch-only on Sonoma and newer', () => {
    const policy = getBroadcomWifiPolicy('Broadcom BCM4352', 'macOS Sonoma');
    expect(policy?.supportClass).toBe('sonoma_root_patch');
    expect(policy?.autoKexts).toEqual([]);
  });

  it('marks BCM4331 as a Catalina-era legacy path', () => {
    const modern = getBroadcomWifiPolicy('Broadcom BCM4331', 'macOS Big Sur');
    const legacy = getBroadcomWifiPolicy('Broadcom BCM4331', 'macOS Catalina');
    expect(modern?.supportClass).toBe('legacy_unsupported_on_target');
    expect(legacy?.supportClass).toBe('legacy_fixup');
    expect(legacy?.autoKexts).toEqual(['AirportBrcmFixup.kext']);
  });
});
