import { describe, expect, it } from 'vitest';
import { getSsdtSourcePolicy, getUnsupportedSsdtRequests } from '../electron/ssdtSourcePolicy.js';

describe('getSsdtSourcePolicy', () => {
  it('maps SSDT-AWAC to the OpenCorePkg alias and Dortania fallback', () => {
    const policy = getSsdtSourcePolicy('SSDT-AWAC.aml');
    expect(policy).not.toBeNull();
    expect(policy.packageCandidates).toEqual(['SSDT-AWAC.aml', 'SSDT-AWAC-DISABLE.aml']);
    expect(policy.supplementalDownload?.url).toContain('/SSDT-AWAC.aml');
  });

  it('maps AMD desktop EC/USBX to Dortania compiled assets', () => {
    const policy = getSsdtSourcePolicy('SSDT-EC-USBX-DESKTOP.aml');
    expect(policy).not.toBeNull();
    expect(policy.packageCandidates).toEqual(['SSDT-EC-USBX-DESKTOP.aml']);
    expect(policy.supplementalDownload?.url).toContain('/SSDT-EC-USBX-DESKTOP.aml');
  });

  it('keeps OpenCorePkg-native Alder/Raptor SSDTs on package-only sourcing', () => {
    const plugAlt = getSsdtSourcePolicy('SSDT-PLUG-ALT.aml');
    expect(plugAlt).not.toBeNull();
    expect(plugAlt.packageCandidates).toEqual(['SSDT-PLUG-ALT.aml']);
    expect(plugAlt.supplementalDownload).toBeUndefined();

    const policy = getSsdtSourcePolicy('SSDT-PMC.aml');
    expect(policy).not.toBeNull();
    expect(policy.packageCandidates).toEqual(['SSDT-PMC.aml']);
    expect(policy.supplementalDownload).toBeUndefined();
  });

  it('reports unsupported SSDT requests explicitly', () => {
    expect(getSsdtSourcePolicy('SSDT-NOT-REAL.aml')).toBeNull();
    expect(getUnsupportedSsdtRequests(['SSDT-PMC.aml', 'SSDT-NOT-REAL.aml', 'SSDT-NOT-REAL.aml'])).toEqual([
      'SSDT-NOT-REAL.aml',
    ]);
  });
});
