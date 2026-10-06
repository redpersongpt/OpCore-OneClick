import type { MessageKey, Translate } from '../i18n';

const COMPONENTS: Record<string, MessageKey> = {
  cpu: 'component.cpu',
  gpu: 'component.gpu',
  audio: 'component.audio',
  ethernet: 'component.ethernet',
  wifi: 'component.wifi',
  bluetooth: 'component.bluetooth',
  input: 'component.input',
  storage: 'component.storage',
  platform: 'component.platform',
  usb: 'component.usb',
  firmware: 'component.firmware',
  acpi: 'component.acpi',
  smbios: 'component.smbios',
};

/** Translated name of a backend component id ("gpu" → "Graphics"); unknown ids are shown as sent. */
export function componentLabel(t: Translate, component: string): string {
  const key = COMPONENTS[component.toLowerCase()];
  return key ? t(key) : component;
}

export const COMPONENT_IDS = Object.keys(COMPONENTS);
