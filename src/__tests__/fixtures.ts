import type {
  BuildPlan,
  BuildResult,
  CompatibilityReport,
  MacOsOption,
  MacOsVersion,
  PlanNote,
  SupportLevel,
} from '../bridge/types';
import { demoScanResult } from '../lib/demo';

export const profile = () => demoScanResult().profile;

export function option(version: MacOsVersion, supported = true, extra: Partial<MacOsOption> = {}): MacOsOption {
  return { version, name: `macOS ${version}`, supported, recommended: false, notes: [], needsRootPatch: false, ...extra };
}

export function report(
  target: MacOsVersion | null,
  level: SupportLevel = 'supported',
  versions: MacOsOption[] = [option('14'), option('15', true, { recommended: true }), option('26')],
  notes: PlanNote[] = [],
): CompatibilityReport {
  return {
    level,
    summary: 'summary',
    target,
    recommended: versions.find((v) => v.recommended)?.version ?? null,
    versions,
    components: [],
    notes,
    confidence: 0.9,
  };
}

export function plan(target: MacOsVersion = '15'): BuildPlan {
  return {
    target,
    smbios: { model: 'iMac19,1', reason: 'Coffee Lake desktop', secureBootModel: 'Default', boardIdSkip: false, alternatives: ['iMac20,1'] },
    ssdts: [],
    acpiPatches: [],
    acpiDeletes: [],
    acpiQuirks: {},
    booterQuirks: {},
    booterPatches: [],
    deviceProperties: [],
    kexts: [],
    kernelPatches: [],
    amdCoreCount: null,
    kernelBlocks: [],
    kernelQuirks: {},
    kernelEmulate: {},
    miscBoot: {},
    miscDebug: {},
    miscSecurity: {},
    tools: [],
    bootArgs: ['-v'],
    csrActiveConfig: 0,
    nvramAdd: [],
    nvramDelete: [],
    nvramSettings: {},
    platformInfo: {},
    drivers: [],
    uefiQuirks: {},
    uefiApfs: {},
    uefiOutput: {},
    uefiInput: {},
    biosSettings: [],
    notes: [],
    postInstall: [],
  };
}

export function buildResult(target: MacOsVersion = '15'): BuildResult {
  return {
    buildId: 'b1',
    efiPath: '/tmp/build/b1',
    configPlistPath: '/tmp/build/b1/EFI/OC/config.plist',
    target,
    opencoreVersion: '1.0.8',
    identity: { model: 'iMac19,1', serial: 'C02XXXXXXXXX', mlb: 'C02XXXXXXXXXXXXXX', systemUuid: 'UUID', rom: '112233445566' },
    plan: plan(target),
    kexts: [],
    ssdts: [],
    validation: { valid: true, ocvalidateRan: true, ocvalidateOutput: null, issues: [] },
    warnings: [],
  };
}
