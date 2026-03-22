import { checkCompatibility } from '../../electron/compatibility.js';
import type { HardwareProfile } from '../../electron/configGenerator.js';
import { isCompatibilityBlocked } from './releaseFlow.js';
import { normalizeErrorMessage } from './errorMessage.js';
import type { StepId } from './installStepGuards.js';

export function getFlashFailureTargetStep(
  errorMessage: string,
  profile: HardwareProfile | null | undefined,
): StepId {
  const normalized = normalizeErrorMessage(errorMessage).toLowerCase();
  const activeCompat = profile ? checkCompatibility(profile) : null;

  if (
    normalized.includes('compatibility is blocked')
    || normalized.includes('no supported display path')
    || (profile ? isCompatibilityBlocked(activeCompat) : false)
  ) {
    return 'report';
  }

  if (
    normalized.includes('bios readiness is no longer satisfied')
    || normalized.includes('bios step incomplete')
    || normalized.includes('required bios setting')
    || normalized.includes('firmware settings before flashing')
  ) {
    return 'bios';
  }

  if (
    normalized.includes('efi validation is no longer clean')
    || normalized.includes('validated efi is required before deployment')
    || normalized.includes('efi validation is blocked')
  ) {
    return 'report';
  }

  if (
    normalized.includes('no target disk is selected for flashing')
    || normalized.includes('disk identity could not be confirmed')
    || normalized.includes('no disk identity fingerprint was captured')
    || normalized.includes('target disk') && normalized.includes('no longer available')
    || normalized.includes('re-select the drive')
    || normalized.includes('confirmation token')
    || normalized.includes('flash confirmation is stale or missing')
  ) {
    return 'usb-select';
  }

  return 'usb-select';
}
