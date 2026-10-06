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

/** Message key of a `recovery:progress` phase; unknown phases read as "contacting Apple". */
export function recoveryPhaseLabel(phase: string | null | undefined): MessageKey {
  switch (phase) {
    case 'downloading':
    case 'verifying':
    case 'complete':
    case 'failed':
      return `recovery.phase.${phase}`;
    default:
      return 'recovery.phase.resolving';
  }
}

const ERROR_HINTS: Record<string, MessageKey> = {
  NETWORK_ERROR: 'error.hint.network',
  RATE_LIMITED: 'error.hint.rateLimited',
  BUSY: 'error.hint.busy',
  BUILD_IN_PROGRESS: 'error.hint.busy',
  FLASH_IN_PROGRESS: 'error.hint.busy',
  RECOVERY_IN_PROGRESS: 'error.hint.busy',
  ADMIN_REQUIRED: 'error.hint.admin',
  ELEVATION_CANCELLED: 'error.hint.elevationCancelled',
  TARGET_ABOVE_CPU_LIMIT: 'error.hint.cpuLimit',
  DISK_IDENTITY_CHANGED: 'error.hint.diskChanged',
  DISK_AMBIGUOUS: 'error.hint.diskAmbiguous',
  RECOVERY_NOT_READY: 'error.hint.recoveryNotReady',
};

/** Translated advice for a well-known backend error code, or null. */
export function errorHint(code: string): MessageKey | null {
  return ERROR_HINTS[code] ?? null;
}

export const ERROR_HINT_CODES = Object.keys(ERROR_HINTS);
