import { describe, expect, it } from 'vitest';
import { getSuggestionPayload } from '../src/lib/suggestionEngine.js';

describe('getSuggestionPayload — flash safety and write classification', () => {
  it('classifies prepare-confirmation BIOS blocks without USB/admin advice', () => {
    const payload = getSuggestionPayload({
      errorMessage: "Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: BIOS readiness is no longer satisfied. Re-verify firmware settings before flashing.",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).toBe('bios_readiness_blocked');
    expect(payload.message).toContain('BIOS readiness');
    expect(payload.suggestion).toContain('BIOS');
    expect(payload.suggestion).not.toContain('Administrator');
  });

  it('classifies compatibility/display-path blockers without USB write fallback', () => {
    const payload = getSuggestionPayload({
      errorMessage: "Error invoking remote method 'flash:prepare-confirmation': Error: Compatibility is blocked. Fix the compatibility report before deployment.",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).toBe('compatibility_blocked');
    expect(payload.message).toContain('deployable');
    expect(payload.suggestion).toContain('report step');
  });

  it('classifies missing selected disk explicitly', () => {
    const payload = getSuggestionPayload({
      errorMessage: "Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: No target disk is selected for flashing.",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).toBe('selected_disk_missing');
    expect(payload.message).toContain('Target drive');
    expect(payload.suggestion).toContain('USB selection');
  });

  it('classifies missing disk identity explicitly', () => {
    const payload = getSuggestionPayload({
      errorMessage: "Error invoking remote method 'flash:prepare-confirmation': Error: SAFETY BLOCK: No disk identity fingerprint was captured for this selection. Re-select the drive before flashing.",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).toBe('disk_identity_missing');
    expect(payload.message).toContain('identity');
    expect(payload.suggestion).not.toContain('Administrator');
  });

  it('keeps real write failures on the flash write path', () => {
    const payload = getSuggestionPayload({
      errorMessage: "Error invoking remote method 'flash-usb': Error: USB flash write failed with a generic I/O error during copy",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).toBe('flash_write_error');
    expect(payload.message).toContain('write');
  });

  it('classifies stalled flash tasks without admin/device fallback', () => {
    const payload = getSuggestionPayload({
      errorMessage: "Error invoking remote method 'flash-usb': Error: Operation stalled: no progress received for 900 seconds. Please check the current step and try again.",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).toBe('watchdog_trigger');
    expect(payload.message).toContain('stalled');
    expect(payload.suggestion).not.toContain('Administrator');
  });
});

describe('getSuggestionPayload — #38 diskpart format must not match recovery_download_failed', () => {
  it('compound diskpart + Format-Volume failure is NOT classified as recovery download', () => {
    const payload = getSuggestionPayload({
      errorMessage:
        "Error invoking remote method 'flash-usb': Error: diskpart created a partition on disk 2, " +
        "but failed to format it as FAT32 OPENCORE. Both diskpart inline format and PowerShell " +
        "Format-Volume fallback failed. Stage: partition exists → format failed → Format-Volume fallback also failed.",
      platform: 'win32',
      step: 'usb-select',
    });

    expect(payload.code).not.toBe('recovery_download_failed');
    expect(payload.message).not.toContain('Recovery download');
  });

  it('actual recovery download failure still classifies correctly', () => {
    const payload = getSuggestionPayload({
      errorMessage: 'Apple recovery download failed: connection reset by peer',
      platform: 'win32',
      step: 'recovery-download',
    });

    expect(payload.code).toBe('recovery_download_failed');
  });
});
