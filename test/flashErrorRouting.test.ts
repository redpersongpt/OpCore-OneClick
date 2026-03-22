import { describe, expect, it } from 'vitest';
import { getFlashFailureTargetStep } from '../src/lib/flashErrorRouting.js';

describe('getFlashFailureTargetStep', () => {
  it('routes BIOS readiness blockers back to bios', () => {
    expect(getFlashFailureTargetStep(
      "Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: BIOS readiness is no longer satisfied. Re-verify firmware settings before flashing.",
      null,
    )).toBe('bios');
  });

  it('routes compatibility blockers back to report', () => {
    expect(getFlashFailureTargetStep(
      "Error invoking remote method 'flash:prepare-confirmation': Error: Compatibility is blocked. Fix the compatibility report before deployment.",
      null,
    )).toBe('report');
  });

  it('routes missing disk identity back to usb-select', () => {
    expect(getFlashFailureTargetStep(
      "Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: No disk identity fingerprint was captured for this selection. Re-select the drive before flashing.",
      null,
    )).toBe('usb-select');
  });

  it('routes stalled flash tasks back to usb-select', () => {
    expect(getFlashFailureTargetStep(
      "Error invoking remote method 'flash-usb': Error: Operation stalled: no progress received for 900 seconds. Please check the current step and try again.",
      null,
    )).toBe('usb-select');
  });
});
