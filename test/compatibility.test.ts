import { describe, it, expect } from 'vitest';
import { checkCompatibility, type CompatibilityReport } from '../electron/compatibility.js';
import type { HardwareProfile } from '../electron/configGenerator.js';

function fakeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i7-10700K',
    architecture: 'Intel',
    generation: 'Comet Lake',
    motherboard: 'ASUS ROG Z490',
    gpu: 'Intel UHD 630',
    ram: '16 GB',
    coreCount: 8,
    targetOS: 'macOS Ventura',
    smbios: 'iMac20,1',
    kexts: [],
    ssdts: [],
    bootArgs: '',
    isLaptop: false,
    scanConfidence: 'high',
    ...overrides,
  } as HardwareProfile;
}

// ─── Basic compatibility checks ─────────────────────────────────────────────

describe('checkCompatibility — basic hardware', () => {
  it('Intel Comet Lake with Intel UHD 630 is supported', () => {
    const report = checkCompatibility(fakeProfile());
    expect(report.isCompatible).toBe(true);
    expect(['supported', 'experimental']).toContain(report.level);
  });

  it('returns valid structure', () => {
    const report = checkCompatibility(fakeProfile());
    expect(report.level).toBeTruthy();
    expect(report.strategy).toBeTruthy();
    expect(report.confidence).toBeTruthy();
    expect(report.maxOSVersion).toBeTruthy();
    expect(report.eligibleVersions.length).toBeGreaterThan(0);
    expect(report.recommendedVersion).toBeTruthy();
  });

  it('AMD Ryzen with RX 580 is compatible', () => {
    const report = checkCompatibility(fakeProfile({
      architecture: 'AMD',
      generation: 'Ryzen',
      cpu: 'AMD Ryzen 7 5800X',
      gpu: 'AMD Radeon RX 580',
      coreCount: 8,
    }));
    expect(report.isCompatible).toBe(true);
  });

  it('Intel Haswell desktop has compatibility report', () => {
    const report = checkCompatibility(fakeProfile({
      generation: 'Haswell',
      cpu: 'Intel Core i7-4770K',
      gpu: 'Intel HD Graphics 4600',
    }));
    // Haswell is older — may be experimental or risky but should produce a valid report
    expect(report.level).toBeTruthy();
    expect(report.eligibleVersions.length).toBeGreaterThan(0);
  });

  it('Intel Coffee Lake laptop is compatible', () => {
    const report = checkCompatibility(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      cpu: 'Intel Core i7-8750H',
      gpu: 'Intel UHD Graphics 630',
    }));
    expect(report.isCompatible).toBe(true);
  });
});

// ─── Scan confidence effects ────────────────────────────────────────────────

describe('checkCompatibility — scan confidence', () => {
  it('high confidence does not downgrade level', () => {
    const report = checkCompatibility(fakeProfile({ scanConfidence: 'high' }));
    expect(report.level).not.toBe('risky');
  });

  it('low confidence downgrades to at least experimental', () => {
    const report = checkCompatibility(fakeProfile({ scanConfidence: 'low' }));
    expect(['experimental', 'risky']).toContain(report.level);
  });
});

// ─── Advisory confidence scoring ────────────────────────────────────────────

describe('checkCompatibility — advisory confidence', () => {
  it('returns advisory confidence with score and label', () => {
    const report = checkCompatibility(fakeProfile());
    expect(report.advisoryConfidence).toBeTruthy();
    expect(typeof report.advisoryConfidence.score).toBe('number');
    expect(['High confidence', 'Medium confidence', 'Low confidence']).toContain(report.advisoryConfidence.label);
  });

  it('high scan confidence boosts advisory score', () => {
    const highConf = checkCompatibility(fakeProfile({ scanConfidence: 'high' }));
    const lowConf = checkCompatibility(fakeProfile({ scanConfidence: 'low' }));
    expect(highConf.advisoryConfidence.score).toBeGreaterThan(lowConf.advisoryConfidence.score);
  });
});

// ─── Failure point analysis ─────────────────────────────────────────────────

describe('checkCompatibility — failure points', () => {
  it('returns failure points array', () => {
    const report = checkCompatibility(fakeProfile());
    expect(Array.isArray(report.mostLikelyFailurePoints)).toBe(true);
  });

  it('failure points have required structure', () => {
    const report = checkCompatibility(fakeProfile({
      isLaptop: true,
      generation: 'Coffee Lake',
    }));
    for (const fp of report.mostLikelyFailurePoints) {
      expect(fp.title).toBeTruthy();
      expect(fp.detail).toBeTruthy();
      expect(['very likely', 'likely', 'possible']).toContain(fp.likelihood);
      expect(['rule', 'community', 'fallback']).toContain(fp.source);
    }
  });
});

// ─── Eligible versions ──────────────────────────────────────────────────────

describe('checkCompatibility — eligible versions', () => {
  it('eligible versions list is non-empty', () => {
    const report = checkCompatibility(fakeProfile());
    expect(report.eligibleVersions.length).toBeGreaterThan(0);
  });

  it('recommended version is in eligible versions list', () => {
    const report = checkCompatibility(fakeProfile());
    const names = report.eligibleVersions.map(v => v.name);
    // recommendedVersion should match one of the eligible version names
    expect(names.some(n => report.recommendedVersion.includes(n) || n.includes(report.recommendedVersion))).toBe(true);
  });

  it('max OS version is reasonable', () => {
    const report = checkCompatibility(fakeProfile());
    expect(report.maxOSVersion).toMatch(/macOS/);
  });
});

// ─── Warnings and errors ────────────────────────────────────────────────────

describe('checkCompatibility — warnings', () => {
  it('supported hardware has no errors', () => {
    const report = checkCompatibility(fakeProfile());
    expect(report.errors.length).toBe(0);
  });

  it('returns strategy based on level', () => {
    const report = checkCompatibility(fakeProfile());
    if (report.level === 'supported') {
      expect(report.strategy).toBe('canonical');
    }
  });
});

describe('checkCompatibility — Broadcom Wi-Fi policy', () => {
  it('marks Sonoma-era Broadcom Wi-Fi as risky instead of pretending it is a clean native path', () => {
    const report = checkCompatibility(fakeProfile({
      generation: 'Coffee Lake',
      isLaptop: true,
      targetOS: 'macOS Sonoma',
      wifiChipset: 'Broadcom BCM4352',
      gpu: 'Intel UHD Graphics 630',
    }));

    expect(['experimental', 'risky']).toContain(report.level);
    expect(report.warnings.some((warning) => /broadcom wi-fi/i.test(warning))).toBe(true);
    expect(report.warnings.some((warning) => /sonoma/i.test(warning))).toBe(true);
  });

  it('marks legacy Broadcom Wi-Fi as risky on Big Sur and newer', () => {
    const report = checkCompatibility(fakeProfile({
      generation: 'Haswell',
      isLaptop: true,
      targetOS: 'macOS Big Sur 11',
      wifiChipset: 'Broadcom BCM4331',
      gpu: 'Intel HD Graphics 4600',
    }));

    expect(report.level).toBe('risky');
    expect(report.warnings.some((warning) => /catalina/i.test(warning))).toBe(true);
  });
});

// ─── Next actions ───────────────────────────────────────────────────────────

describe('checkCompatibility — next actions', () => {
  it('returns next actions array', () => {
    const report = checkCompatibility(fakeProfile());
    expect(Array.isArray(report.nextActions)).toBe(true);
  });

  it('next actions have required structure', () => {
    const report = checkCompatibility(fakeProfile({
      isLaptop: true,
      scanConfidence: 'low',
    }));
    for (const action of report.nextActions) {
      expect(action.title).toBeTruthy();
      expect(action.detail).toBeTruthy();
      expect(['rule', 'community', 'fallback']).toContain(action.source);
      expect(['high', 'medium', 'low']).toContain(action.confidence);
    }
  });
});
