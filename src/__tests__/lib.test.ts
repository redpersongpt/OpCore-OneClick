import { describe, expect, it } from 'vitest';
import type { DiskInfo, FirmwareCheck } from '../bridge/types';
import { compatGate, defaultTarget, reachOf, sortedVersions } from '../lib/compat';
import { confirmPhrase, deviceName, diskBlock, minimumDiskBytes, MIN_BYTES_WITH_RECOVERY, phraseMatches } from '../lib/disk';
import { checkState, probeFor } from '../lib/firmware';
import { formatBytes, formatCodecId, formatPercent, formatVram, maskSecret, normalizePciId, parseCodecId, parseCount } from '../lib/format';
import { buildIssueUrl, MAX_DIAGNOSTICS, redact } from '../lib/issue';
import { compareMacos, macosLabel } from '../lib/macos';
import { stableStringify } from '../lib/stable';
import { blocksBuild, holdsScanStep, isAppleSiliconHost, profileNotices, scanNotices } from '../lib/host';
import { validationVerdict } from '../lib/verdict';
import { demoScanResult } from '../lib/demo';
import { option, report } from './fixtures';

describe('compatibility gate', () => {
  it('allows a supported version', () => {
    expect(compatGate(report('15', 'supported'), '15')).toBe('ok');
    expect(compatGate(report('15', 'partial'), '15')).toBe('ok');
  });

  it('asks for the expert confirmation for workaround releases and doubtful verdicts', () => {
    // CryptexFixup / telemetrap past the CPU ceiling: the backend rates the release "partial".
    const workaround = report('26', 'partial', [option('15'), option('26', false, { notes: ['No AVX2: macOS 13+ installs only with CryptexFixup.'] })]);
    expect(compatGate(workaround, '26')).toBe('expert');
    const blockingNote = report('15', 'supported', undefined, [{ level: 'blocking', component: 'gpu', title: 't', detail: 'd' }]);
    expect(compatGate(blockingNote, '15')).toBe('expert');
    // A supported release with a contradictory overall verdict is never silently allowed.
    expect(compatGate(report('15', 'unsupported'), '15')).toBe('expert');
  });

  it('blocks unsupported hardware and unknown versions', () => {
    expect(compatGate(report('26', 'unsupported', [option('26', false)]), '26')).toBe('blocked');
    // Unknown CPU or core count: nothing to override, the profile must be completed.
    expect(compatGate(report('15', 'unknown', [option('15', false)]), '15')).toBe('blocked');
    expect(reachOf(report('26', 'partial', [option('26', false)]))).toBe('expert');
    expect(
      reachOf(report('26', 'partial', [option('26', false)], [{ level: 'blocking', component: 'cpu', title: 't', detail: 'd' }])),
    ).toBe('blocked');
    expect(compatGate(report('15'), '10.13')).toBe('blocked');
    expect(compatGate(null, '15')).toBe('blocked');
  });

  it('picks the default target', () => {
    expect(defaultTarget(report('14'))).toBe('14');
    expect(defaultTarget(report(null))).toBe('15');
    expect(defaultTarget(report(null, 'partial', [option('12'), option('13'), option('14', false)]))).toBe('13');
    expect(sortedVersions(report(null)).map((v) => v.version)).toEqual(['26', '15', '14']);
  });

  it('labels and orders macOS versions', () => {
    expect(macosLabel('26')).toBe('macOS Tahoe 26');
    expect(macosLabel('10.15')).toBe('macOS Catalina 10.15');
    expect(compareMacos('10.15', '11')).toBeLessThan(0);
  });
});

describe('formatting', () => {
  it('never prints NaN percentages', () => {
    expect(formatPercent(NaN)).toBeNull();
    expect(formatPercent(undefined)).toBeNull();
    expect(formatPercent(0.426)).toBe('43%');
    expect(formatPercent(3)).toBe('100%');
  });

  it('formats bytes, codec ids and PCI ids', () => {
    expect(formatBytes(15_931_539_456)).toBe('15.9 GB');
    expect(formatBytes(null)).toBe('?');
    expect(formatVram(8192)).toBe('8 GB');
    expect(formatVram(1536)).toBe('1.5 GB');
    expect(formatVram(512)).toBe('512 MB');
    expect(formatVram(null)).toBeNull();
    expect(formatCodecId(0x10ec0897)).toBe('0x10EC0897');
    expect(parseCodecId('10ec:0897')).toBe(0x10ec0897);
    expect(parseCodecId('0x10ec')).toBeNull();
    expect(normalizePciId('0x8086')).toBe('8086');
    expect(normalizePciId('80861')).toBeNull();
    expect(parseCount('12')).toBe(12);
    expect(parseCount('1.5')).toBeNull();
    // Counts go into Rust u32 fields; larger values would be rejected by the backend.
    expect(parseCount('4294967295')).toBe(4_294_967_295);
    expect(parseCount('4294967296')).toBeNull();
  });

  it('masks secrets', () => {
    expect(maskSecret('C02ABCDEF123')).toBe('•••••••••123');
    expect(maskSecret('ab')).toBe('••');
  });

  it('stableStringify sorts keys and drops undefined', () => {
    expect(stableStringify({ b: 1, a: { d: 2, c: undefined } })).toBe('{"a":{"d":2},"b":1}');
  });
});

describe('disks', () => {
  const disk = (patch: Partial<DiskInfo>): DiskInfo => ({
    devicePath: '/dev/sdb',
    model: 'Ultra',
    vendor: 'SanDisk',
    serialNumber: null,
    sizeBytes: 32_000_000_000,
    sizeDisplay: '32.0 GB',
    transport: 'usb',
    removable: true,
    partitionTable: 'gpt',
    partitions: [],
    isSystemDisk: false,
    blockedReason: null,
    ...patch,
  });

  it('blocks system disks, backend-blocked disks and small disks', () => {
    expect(diskBlock(disk({}), MIN_BYTES_WITH_RECOVERY)).toBeNull();
    expect(diskBlock(disk({ isSystemDisk: true }), 0)).toEqual({ kind: 'system' });
    expect(diskBlock(disk({ blockedReason: 'Holds the page file' }), 0)).toEqual({ kind: 'backend', reason: 'Holds the page file' });
    expect(diskBlock(disk({ sizeBytes: 2_000_000_000 }), MIN_BYTES_WITH_RECOVERY)?.kind).toBe('too_small');
  });

  it('accepts a 4 GB stick for recovery but grows with a large recovery image', () => {
    expect(minimumDiskBytes(true, null)).toBeLessThan(4_000_000_000);
    expect(minimumDiskBytes(true, 5_000_000_000)).toBeGreaterThan(5_000_000_000);
    expect(minimumDiskBytes(false, null)).toBeLessThan(minimumDiskBytes(true, null));
  });

  it('confirmation phrase is the size, compared leniently', () => {
    expect(confirmPhrase(disk({}))).toBe('32.0 GB');
    expect(phraseMatches(' 32.0  gb ', '32.0 GB')).toBe(true);
    expect(phraseMatches('32 GB', '32.0 GB')).toBe(false);
    expect(phraseMatches('', '')).toBe(false);
  });

  it('adds the device name when another drive has the same size', () => {
    const a = disk({});
    const b = disk({ devicePath: '/dev/sdc', model: 'Cruzer' });
    const win = disk({ devicePath: '\\\\.\\PhysicalDrive3' });
    expect(deviceName(a)).toBe('sdb');
    expect(deviceName(win)).toBe('PhysicalDrive3');
    expect(deviceName(disk({ devicePath: '/dev/disk4' }))).toBe('disk4');
    expect(confirmPhrase(a, [a])).toBe('32.0 GB');
    expect(confirmPhrase(a, [a, disk({ devicePath: '/dev/sdc', sizeDisplay: '64.0 GB' })])).toBe('32.0 GB');
    expect(confirmPhrase(a, [a, b])).toBe('32.0 GB sdb');
    expect(confirmPhrase(b, [a, b])).toBe('32.0 GB sdc');
    expect(phraseMatches('32.0 GB', confirmPhrase(a, [a, b]))).toBe(false);
    expect(phraseMatches('32.0 gb  SDB', confirmPhrase(a, [a, b]))).toBe(true);
  });
});

describe('firmware checks', () => {
  const check = (status: string, name = 'Secure Boot'): FirmwareCheck => ({ name, status, evidence: '', required: true });

  it('understands both status vocabularies', () => {
    expect(checkState(check('ok'))).toBe('ok');
    expect(checkState(check('confirmed'))).toBe('ok');
    expect(checkState(check('failing'))).toBe('action');
    expect(checkState(check('action'))).toBe('action');
    expect(checkState(check('inferred'))).toBe('inferred');
    expect(checkState(check('unverified'))).toBe('unknown');
    expect(checkState(check('not_applicable'))).toBe('na');
  });

  it('matches checklist items to probe results', () => {
    const r = {
      uefiMode: check('ok', 'UEFI'),
      secureBoot: check('action', 'SB'),
      vtX: check('ok', 'VTX'),
      vtD: check('unknown', 'VTD'),
      above4g: check('unknown', '4G'),
      biosVendor: null,
      biosVersion: null,
      confidence: 'high',
    };
    const setting = (name: string) => ({ name, value: 'disable', required: true, reason: '', locationHint: null });
    expect(probeFor(setting('Secure Boot'), r)?.name).toBe('SB');
    expect(probeFor(setting('VT-d'), r)?.name).toBe('VTD');
    expect(probeFor(setting('Intel Virtualization Technology (VT-x)'), r)?.name).toBe('VTX');
    expect(probeFor(setting('Above 4G Decoding'), r)?.name).toBe('4G');
    expect(probeFor(setting('CFG Lock'), r)).toBeNull();
    expect(probeFor(setting('CSM'), r)?.name).toBe('UEFI');
    expect(probeFor(setting('Boot Mode: UEFI only'), r)?.name).toBe('UEFI');
    // Legacy USB support is about keyboards in the firmware, not about the boot mode.
    expect(probeFor(setting('Legacy USB Support'), r)).toBeNull();
  });
});

describe('validation verdict', () => {
  it('distinguishes blocking issues from warnings', () => {
    const base = { valid: true, ocvalidateRan: true, ocvalidateOutput: null };
    expect(validationVerdict({ ...base, issues: [] })).toBe('passed');
    expect(validationVerdict({ ...base, issues: [{ level: 'warning', source: 'kext', message: 'm', path: null }] })).toBe('warnings');
    expect(validationVerdict({ ...base, issues: [{ level: 'blocking', source: 'ocvalidate', message: 'm', path: null }] })).toBe('failed');
    expect(validationVerdict({ ...base, valid: false, issues: [] })).toBe('failed');
  });
});

describe('bug report URL', () => {
  it('redacts user names, MAC addresses and serials', () => {
    const text = redact('C:\\Users\\Ata\\AppData /home/ata/.cache /Users/ata/x mac 3C:7C:3F:12:34:56 serial: C02XK1ZZJHD5');
    expect(text).not.toMatch(/Ata|ata\//);
    expect(text).toContain('<user>');
    expect(text).toContain('<mac>');
    expect(text).toContain('<redacted>');
  });

  it('caps the diagnostics size', () => {
    const url = buildIssueUrl({ title: 'x', description: 'y', diagnostics: 'a'.repeat(10_000) });
    const body = new URL(url).searchParams.get('body') ?? '';
    expect(body.length).toBeLessThan(MAX_DIAGNOSTICS + 300);
    expect(body).toContain('(truncated)');
    expect(url.startsWith('https://github.com/redpersongpt/OpCore-OneClick/issues/new?')).toBe(true);
  });
});

describe('scan host notices', () => {
  const scan = (hostOs: string, patch: { biosVendor?: string | null; warnings?: string[]; acpi?: string | null } = {}) => {
    const demo = demoScanResult();
    return {
      detected: {
        ...demo.detected,
        hostOs,
        warnings: patch.warnings ?? [],
        acpiTablesDir: patch.acpi ?? null,
        firmware: { ...demo.detected.firmware, biosVendor: patch.biosVendor ?? null },
      },
      profile: { ...demo.profile, source: 'scan' },
    };
  };

  it('recognises Apple silicon, real Macs, Hackintoshes and Linux without ACPI access', () => {
    const apple = scan('macos');
    apple.profile.cpu = { ...apple.profile.cpu, vendor: 'apple', platform: 'apple_silicon' };
    expect(scanNotices(apple.detected, apple.profile)).toEqual(['apple_silicon']);

    const mac = scan('macos', { biosVendor: 'Apple' });
    expect(scanNotices(mac.detected, mac.profile)).toEqual(['real_mac']);
    const hack = scan('macos', { biosVendor: 'Apple', warnings: ['Booted through OpenCore (1.0.8): device ids and SMBIOS reflect the running configuration'] });
    expect(scanNotices(hack.detected, hack.profile)).toEqual(['hackintosh']);

    const linux = scan('linux');
    expect(scanNotices(linux.detected, linux.profile)).toEqual(['linux_acpi']);
    const root = scan('linux', { acpi: '/tmp/acpi/scan-1' });
    expect(scanNotices(root.detected, root.profile)).toEqual([]);
    expect(scanNotices(scan('windows').detected, scan('windows').profile)).toEqual([]);

    // An imported profile says nothing about this computer.
    expect(scanNotices(linux.detected, { ...linux.profile, source: 'imported' })).toEqual([]);
  });

  it('flags an imported Apple silicon profile, which no EFI can be built for', () => {
    const apple = scan('macos');
    const imported = { ...apple.profile, source: 'imported', cpu: { ...apple.profile.cpu, vendor: 'apple' as const, platform: 'apple_silicon' as const } };
    expect(profileNotices(null, imported)).toEqual(['apple_profile']);
    expect(blocksBuild(profileNotices(null, imported))).toBe(true);
    // A scan of this Mac is described as such.
    expect(profileNotices(apple.detected, { ...imported, source: 'scan' })).toEqual(['apple_silicon']);
    expect(blocksBuild(['apple_silicon'])).toBe(true);
    expect(blocksBuild(['real_mac', 'linux_acpi', 'hackintosh'])).toBe(false);
    const pc = scan('windows');
    expect(profileNotices(null, { ...pc.profile, source: 'manual' })).toEqual([]);
  });

  it('holds the scan step only for notices that need a decision', () => {
    expect(holdsScanStep(['apple_silicon'])).toBe(true);
    expect(holdsScanStep(['real_mac'])).toBe(true);
    expect(holdsScanStep(['linux_acpi'])).toBe(true);
    // A Hackintosh rebuilding its own EFI moves on; the hardware step repeats the warning.
    expect(holdsScanStep(['hackintosh'])).toBe(false);
    expect(holdsScanStep([])).toBe(false);
    expect(isAppleSiliconHost({ version: '5', opencoreVersion: '1.0.8', hostOs: 'macos', arch: 'aarch64' })).toBe(true);
    expect(isAppleSiliconHost({ version: '5', opencoreVersion: '1.0.8', hostOs: 'macos', arch: 'x86_64' })).toBe(false);
    expect(isAppleSiliconHost(null)).toBe(false);
  });
});
