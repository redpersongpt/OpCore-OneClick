import fs from 'fs/promises';
import path from 'path';
import { getRequiredResources, type HardwareProfile } from './configGenerator.js';
import { parseMacOSVersion } from './hackintoshRules.js';

// ── Types ────────────────────────────────────────────────────────────────────

export type ValidationSeverity = 'pass' | 'warning' | 'blocked';

export interface ValidationIssue {
  code: string;
  severity: 'warning' | 'blocked';
  message: string;
  detail: string | null;
  component: string;
  expectedPath: string;
  actualCondition: string;
}

export type ValidationTraceSource = 'github' | 'embedded' | 'generated' | 'unknown';

export interface ValidationTrace {
  code: string;
  component: string;
  expectedPath: string;
  source: ValidationTraceSource;
  detail: string;
}

export interface ValidationResult {
  overall: ValidationSeverity;
  issues: ValidationIssue[];
  checkedAt: string;
  firstFailureTrace: ValidationTrace | null;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function blocked(
  code: string,
  message: string,
  component: string,
  expectedPath: string,
  actualCondition: string,
  detail: string | null = null,
): ValidationIssue {
  return { code, severity: 'blocked', message, detail, component, expectedPath, actualCondition };
}

function warning(
  code: string,
  message: string,
  component: string,
  expectedPath: string,
  actualCondition: string,
  detail: string | null = null,
): ValidationIssue {
  return { code, severity: 'warning', message, detail, component, expectedPath, actualCondition };
}

function inferIssueSource(
  issue: ValidationIssue,
  kextSources: Record<string, 'github' | 'embedded' | 'direct' | 'failed'>,
): ValidationTraceSource {
  if (issue.component.endsWith('.kext')) {
    const source = kextSources[issue.component];
    if (source === 'github' || source === 'embedded') return source;
    if (source === 'direct') return 'github';
    return 'unknown';
  }
  if (
    issue.component === 'OpenCore.efi' ||
    issue.component === 'BOOTx64.efi' ||
    issue.component === 'OpenRuntime.efi' ||
    issue.component === 'OpenHfsPlus.efi' ||
    issue.component === 'config.plist' ||
    issue.component === 'Drivers directory' ||
    issue.component === 'Kexts directory' ||
    issue.component === 'EFI folder' ||
    issue.component.endsWith('.aml')
  ) {
    return 'generated';
  }
  return 'unknown';
}

function buildValidationTrace(
  issues: ValidationIssue[],
  kextSources: Record<string, 'github' | 'embedded' | 'direct' | 'failed'>,
): ValidationTrace | null {
  const first = issues.find(issue => issue.severity === 'blocked') ?? issues[0] ?? null;
  if (!first) return null;
  return {
    code: first.code,
    component: first.component,
    expectedPath: first.expectedPath,
    source: inferIssueSource(first, kextSources),
    detail: first.detail ?? first.actualCondition,
  };
}

async function exists(p: string): Promise<boolean> {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

async function isDir(p: string): Promise<boolean> {
  try {
    const stat = await fs.stat(p);
    return stat.isDirectory();
  } catch {
    return false;
  }
}

/**
 * Extract array entries from a plist section using simple string matching.
 * Looks for `<key>{sectionKey}</key>` followed by `<array>...</array>` and
 * pulls out `<string>` values nested under `<key>{entryKey}</key>`.
 */
function extractPlistArrayEntries(
  plistContent: string,
  sectionPath: string[],
  entryKey: string,
): string[] {
  let cursor = plistContent;

  // Walk down the section path (e.g. ['Kernel', 'Add'] or ['ACPI', 'Add'])
  for (const key of sectionPath) {
    const keyTag = `<key>${key}</key>`;
    const idx = cursor.indexOf(keyTag);
    if (idx === -1) return [];
    cursor = cursor.slice(idx + keyTag.length);
  }

  // Find the array that follows
  const arrayStart = cursor.indexOf('<array>');
  if (arrayStart === -1) return [];
  const arrayEnd = cursor.indexOf('</array>', arrayStart);
  if (arrayEnd === -1) return [];
  const arrayContent = cursor.slice(arrayStart, arrayEnd);

  // Extract all values for the given entryKey
  const results: string[] = [];
  const keyTag = `<key>${entryKey}</key>`;
  let searchFrom = 0;
  while (true) {
    const keyIdx = arrayContent.indexOf(keyTag, searchFrom);
    if (keyIdx === -1) break;
    const afterKey = keyIdx + keyTag.length;
    // Next <string>...</string> after the key
    const strStart = arrayContent.indexOf('<string>', afterKey);
    const strEnd = arrayContent.indexOf('</string>', afterKey);
    if (strStart === -1 || strEnd === -1) {
      searchFrom = afterKey;
      continue;
    }
    results.push(arrayContent.slice(strStart + '<string>'.length, strEnd));
    searchFrom = strEnd;
  }
  return results;
}

function extractSimplePlistValue(plistContent: string, key: string): string | null {
  const match = plistContent.match(new RegExp(`<key>${key}</key>\\s*<string>([^<]*)</string>`, 'i'));
  return match?.[1]?.trim() ?? null;
}

function extractKextExecutable(infoPlistContent: string): string | null {
  return extractSimplePlistValue(infoPlistContent, 'CFBundleExecutable');
}

/**
 * Check whether AMD kernel patches are present in the plist content.
 * Returns true if any Kernel > Patch entry has a comment containing "AMD"
 * or known AMD patch identifiers.
 */
function hasAMDPatches(plistContent: string): boolean {
  const comments = extractPlistArrayEntries(plistContent, ['Kernel', 'Patch'], 'Comment');
  return comments.some(c =>
    /amd|ryzen|threadripper|genuineintel.*bypass/i.test(c),
  );
}

/**
 * Extract Find and Replace values from Kernel > Patch entries.
 * Returns pairs so we can check for empties.
 */
function extractPatchFindReplace(plistContent: string): { find: string; replace: string; comment: string }[] {
  const results: { find: string; replace: string; comment: string }[] = [];

  // Locate Kernel > Patch > <array>
  let cursor = plistContent;
  const kernelIdx = cursor.indexOf('<key>Kernel</key>');
  if (kernelIdx === -1) return results;
  cursor = cursor.slice(kernelIdx);

  const patchIdx = cursor.indexOf('<key>Patch</key>');
  if (patchIdx === -1) return results;
  cursor = cursor.slice(patchIdx);

  const arrayStart = cursor.indexOf('<array>');
  if (arrayStart === -1) return results;
  const arrayEnd = cursor.indexOf('</array>', arrayStart);
  if (arrayEnd === -1) return results;
  const arrayContent = cursor.slice(arrayStart, arrayEnd);

  // Split by <dict> to get individual patch entries
  const dicts = arrayContent.split('<dict>').slice(1); // skip content before first <dict>
  for (const dict of dicts) {
    const getValue = (key: string): string => {
      const keyTag = `<key>${key}</key>`;
      const idx = dict.indexOf(keyTag);
      if (idx === -1) return '';
      const after = idx + keyTag.length;
      // Look for <data> or <string> next
      const dataStart = dict.indexOf('<data>', after);
      const strStart = dict.indexOf('<string>', after);
      // Use whichever comes first (and exists)
      if (dataStart !== -1 && (strStart === -1 || dataStart < strStart)) {
        const dataEnd = dict.indexOf('</data>', dataStart);
        if (dataEnd === -1) return '';
        return dict.slice(dataStart + '<data>'.length, dataEnd).trim();
      }
      if (strStart !== -1) {
        const strEnd = dict.indexOf('</string>', strStart);
        if (strEnd === -1) return '';
        return dict.slice(strStart + '<string>'.length, strEnd).trim();
      }
      return '';
    };

    const find = getValue('Find');
    const replace = getValue('Replace');
    const comment = getValue('Comment');

    // Only include entries that have at least a comment or either field
    if (comment || find || replace) {
      results.push({ find, replace, comment });
    }
  }

  return results;
}

async function readVersionSidecar(targetPath: string): Promise<string | null> {
  try {
    const sidecar = await fs.readFile(`${targetPath}.version`, 'utf-8');
    return sidecar.trim() || null;
  } catch {
    return null;
  }
}

const LILU_PLUGIN_KEXTS = new Set([
  'WhateverGreen.kext',
  'AppleALC.kext',
  'RestrictEvents.kext',
  'CPUTopologyRebuild.kext',
  'NootRX.kext',
  'NootedRed.kext',
  'NVMeFix.kext',
]);

const VIRTUALSMC_PLUGIN_KEXTS = new Set([
  'SMCProcessor.kext',
  'SMCSuperIO.kext',
  'SMCBatteryManager.kext',
  'SMCLightSensor.kext',
  'SMCDellSensors.kext',
]);

// VoodooI2C plugin kexts — ship inside VoodooI2C.kext/Contents/PlugIns/
// and require VoodooI2C as the parent kext
const VOODOOI2C_PLUGIN_KEXTS = new Set([
  'VoodooI2CHID.kext',
]);

// ── Main Validator ───────────────────────────────────────────────────────────

export async function validateEfi(
  efiPath: string,
  profile: HardwareProfile | null,
  kextSourceHints: Record<string, 'github' | 'embedded' | 'direct' | 'failed'> = {},
): Promise<ValidationResult> {
  const issues: ValidationIssue[] = [];

  // ── 1. File existence and size checks (blocked) ─────────────────────────

  const requiredFiles = [
    { rel: 'EFI/OC/config.plist',  label: 'config.plist', minSize: 500 },
    { rel: 'EFI/OC/OpenCore.efi',  label: 'OpenCore.efi', minSize: 100 * 1024 },
    // OpenCore 1.0.3 ships BOOTx64.efi and OpenRuntime.efi at 24 KB.
    { rel: 'EFI/BOOT/BOOTx64.efi', label: 'BOOTx64.efi', minSize: 20 * 1024 },
    { rel: 'EFI/OC/Drivers/OpenRuntime.efi', label: 'OpenRuntime.efi', minSize: 20 * 1024 },
    { rel: 'EFI/OC/Drivers/OpenHfsPlus.efi', label: 'OpenHfsPlus.efi', minSize: 30 * 1024 },
  ];

  for (const { rel, label, minSize } of requiredFiles) {
    const fullPath = path.join(efiPath, rel);
    if (!(await exists(fullPath))) {
      issues.push(blocked('MISSING_FILE', `Required file missing: ${label}`, label, rel, 'File is missing on disk', rel));
    } else {
      const stat = await fs.stat(fullPath);
      if (stat.size < minSize) {
        issues.push(blocked(
          'FILE_TOO_SMALL',
          `Required file is too small: ${label}`,
          label,
          rel,
          `File size ${stat.size} bytes is below required minimum ${minSize} bytes`,
          `Size: ${Math.round(stat.size / 1024)} KB (Expected ≥ ${Math.round(minSize / 1024)} KB)`,
        ));
      }
    }
  }

  // Check for nested EFI structure (common extraction bug)
  if (await isDir(path.join(efiPath, 'EFI/EFI'))) {
    issues.push(blocked(
      'NESTED_STRUCTURE',
      'Invalid nested folder structure detected',
      'EFI folder',
      'EFI',
      'Found nested EFI/EFI directory',
      'Found EFI/EFI — structure must start with a single EFI folder.',
    ));
  }

  const requiredDirs = [
    { rel: 'EFI/OC/Drivers', label: 'Drivers directory' },
    { rel: 'EFI/OC/Kexts',  label: 'Kexts directory' },
    { rel: 'EFI/OC/ACPI',   label: 'ACPI directory' },
  ];

  for (const { rel, label } of requiredDirs) {
    if (!(await isDir(path.join(efiPath, rel)))) {
      issues.push(blocked('MISSING_DIR', `Required directory missing: ${label}`, label, rel, 'Directory is missing on disk', rel));
    }
  }

  // ── 2. Plist integrity (blocked) ────────────────────────────────────────

  const plistPath = path.join(efiPath, 'EFI/OC/config.plist');
  let plistContent = '';

  if (await exists(plistPath)) {
    try {
      const stat = await fs.stat(plistPath);
      if (stat.size < 100) {
        issues.push(blocked(
          'PLIST_TOO_SMALL',
          'config.plist is under 100 bytes — likely empty or corrupt',
          'config.plist',
          'EFI/OC/config.plist',
          `config.plist is only ${stat.size} bytes`,
          `Size: ${stat.size} bytes`,
        ));
      } else {
        plistContent = await fs.readFile(plistPath, 'utf-8');

        const hasPlistOpen = /<plist/i.test(plistContent);
        const hasPlistClose = /<\/plist>/i.test(plistContent);
        const hasDict = /<dict>/i.test(plistContent);

        if (!hasPlistOpen || !hasPlistClose || !hasDict) {
          issues.push(blocked(
            'PLIST_INVALID',
            'config.plist is not a valid XML plist',
            'config.plist',
            'EFI/OC/config.plist',
            'Missing required plist XML structure',
            'Missing <plist>, </plist>, or <dict> structure',
          ));
        } else {
          // Verify required OpenCore config sections exist
          const requiredSections = ['ACPI', 'Booter', 'DeviceProperties', 'Kernel', 'Misc', 'NVRAM', 'PlatformInfo', 'UEFI'];
          const missingSections = requiredSections.filter(section => {
            const keyTag = `<key>${section}</key>`;
            return !plistContent.includes(keyTag);
          });
          if (missingSections.length > 0) {
            issues.push(blocked(
              'PLIST_SECTIONS_MISSING',
              `config.plist is missing required OpenCore sections: ${missingSections.join(', ')}`,
              'config.plist',
              'EFI/OC/config.plist',
              `Missing top-level config sections: ${missingSections.join(', ')}`,
              `Required: ${requiredSections.join(', ')}`,
            ));
          }
        }
      }
    } catch (e) {
      issues.push(blocked(
        'PLIST_READ_ERROR',
        'Failed to read config.plist',
        'config.plist',
        'EFI/OC/config.plist',
        'Read operation failed',
        String(e),
      ));
    }
  }

  // ── 3. Kext consistency (blocked for missing, warning for extras) ───────

  if (plistContent) {
    const plistKexts = extractPlistArrayEntries(plistContent, ['Kernel', 'Add'], 'BundlePath');
    const plistDrivers = extractPlistArrayEntries(plistContent, ['UEFI', 'Drivers'], 'Path');
    const kextsDir = path.join(efiPath, 'EFI/OC/Kexts');
    const driversDir = path.join(efiPath, 'EFI/OC/Drivers');
    const acpiDir = path.join(efiPath, 'EFI/OC/ACPI');

    if (await isDir(kextsDir)) {
      const topLevelKexts = new Set(plistKexts.map(bundlePath => bundlePath.split('/')[0]));

      for (const kextBundle of plistKexts) {
        // BundlePath may be e.g. "Lilu.kext" or "VoodooI2C.kext/Contents/Plugins/VoodooI2CHID.kext"
        const topLevel = kextBundle.split('/')[0];
        const topLevelPath = path.join(kextsDir, topLevel);
        if (!(await exists(topLevelPath))) {
          issues.push(blocked(
            'KEXT_MISSING',
            `Kext referenced in config but not on disk: ${topLevel}`,
            topLevel,
            `EFI/OC/Kexts/${topLevel}`,
            `config.plist references ${kextBundle} but the bundle is missing on disk`,
            `BundlePath: ${kextBundle}`,
          ));
          continue;
        }

        const exactBundlePath = path.join(kextsDir, ...kextBundle.split('/'));
        if (!(await exists(exactBundlePath))) {
          issues.push(blocked(
            'KEXT_BUNDLE_PATH_MISSING',
            `Configured kext bundle path is missing: ${kextBundle}`,
            topLevel,
            `EFI/OC/Kexts/${kextBundle}`,
            'config.plist references a nested kext bundle path that does not exist on disk',
            `BundlePath: ${kextBundle}`,
          ));
          continue;
        }

        const plistOnDisk = path.join(exactBundlePath, 'Contents/Info.plist');
        if (!(await exists(plistOnDisk))) {
          issues.push(blocked(
            'KEXT_INFO_PLIST_MISSING',
            `Kext bundle is incomplete: ${kextBundle}`,
            topLevel,
            `EFI/OC/Kexts/${kextBundle}/Contents/Info.plist`,
            'Kext bundle exists but Contents/Info.plist is missing',
            `BundlePath: ${kextBundle}`,
          ));
          continue;
        }

        const infoPlistContent = await fs.readFile(plistOnDisk, 'utf-8').catch(() => '');
        const executableName = extractKextExecutable(infoPlistContent);
        if (executableName) {
          const executableRel = `Contents/MacOS/${executableName}`;
          if (!(await exists(path.join(exactBundlePath, executableRel)))) {
            issues.push(blocked(
              'KEXT_EXECUTABLE_MISSING',
              `Kext executable missing: ${kextBundle}`,
              topLevel,
              `EFI/OC/Kexts/${kextBundle}/${executableRel}`,
              `The bundle declares ${executableName} as its executable but that file is missing on disk`,
              `BundlePath: ${kextBundle}`,
            ));
          }
        }
      }

      if (topLevelKexts.size > 0 && !topLevelKexts.has('Lilu.kext') && [...topLevelKexts].some(kext => LILU_PLUGIN_KEXTS.has(kext))) {
        issues.push(blocked(
          'KEXT_LILU_DEPENDENCY',
          'Lilu plugin selected without Lilu.kext',
          'Lilu.kext',
          'EFI/OC/Kexts/Lilu.kext',
          'One or more Lilu plugins are present but Lilu.kext is missing from config.plist',
          [...topLevelKexts].filter(kext => LILU_PLUGIN_KEXTS.has(kext)).join(', '),
        ));
      }

      if (topLevelKexts.size > 0 && !topLevelKexts.has('VirtualSMC.kext') && [...topLevelKexts].some(kext => VIRTUALSMC_PLUGIN_KEXTS.has(kext))) {
        issues.push(blocked(
          'KEXT_VIRTUALSMC_DEPENDENCY',
          'VirtualSMC plugin selected without VirtualSMC.kext',
          'VirtualSMC.kext',
          'EFI/OC/Kexts/VirtualSMC.kext',
          'One or more VirtualSMC plugins are present but VirtualSMC.kext is missing from config.plist',
          [...topLevelKexts].filter(kext => VIRTUALSMC_PLUGIN_KEXTS.has(kext)).join(', '),
        ));
      }

      // VoodooI2C plugin kexts referenced via nested BundlePath require VoodooI2C.kext as parent
      const hasVoodooI2CPlugin = plistKexts.some(bp => {
        const leaf = bp.split('/').pop() ?? '';
        return VOODOOI2C_PLUGIN_KEXTS.has(leaf);
      });
      if (hasVoodooI2CPlugin && !topLevelKexts.has('VoodooI2C.kext')) {
        issues.push(blocked(
          'KEXT_VOODOOI2C_DEPENDENCY',
          'VoodooI2C plugin selected without VoodooI2C.kext',
          'VoodooI2C.kext',
          'EFI/OC/Kexts/VoodooI2C.kext',
          'VoodooI2CHID is configured as a plugin but VoodooI2C.kext is missing from config.plist',
          'VoodooI2CHID.kext',
        ));
      }
    }

    if (await isDir(driversDir)) {
      for (const driverPath of plistDrivers) {
        const fullDriverPath = path.join(driversDir, driverPath);
        if (!(await exists(fullDriverPath))) {
          issues.push(blocked(
            'DRIVER_MISSING',
            `Driver referenced in config but not on disk: ${driverPath}`,
            driverPath,
            `EFI/OC/Drivers/${driverPath}`,
            'config.plist references this driver but the file is missing on disk',
            null,
          ));
        }
      }
    }

    const secureBootModel = extractSimplePlistValue(plistContent, 'SecureBootModel');
    const kextNamesInConfig = Array.from(new Set(plistKexts.map(bundlePath => bundlePath.split('/')[0].replace(/\.kext$/, ''))));
    if (kextNamesInConfig.includes('AirportItlwm') && secureBootModel?.toLowerCase() === 'disabled') {
      issues.push(blocked(
        'AIRPORTITLWM_SECUREBOOT_REQUIRED',
        'AirportItlwm requires SecureBootModel-enabled behavior',
        'AirportItlwm.kext',
        'EFI/OC/config.plist',
        'SecureBootModel is Disabled while AirportItlwm is selected',
        'Use AirportItlwm only when SecureBootModel is enabled, or switch to Itlwm and accept Recovery limitations.',
      ));
    }

    if (kextNamesInConfig.includes('Itlwm')) {
      issues.push(warning(
        'ITLWM_RECOVERY_LIMITATION',
        'Itlwm does not provide Recovery Wi-Fi',
        'Itlwm.kext',
        'EFI/OC/Kexts/Itlwm.kext',
        'Intel Wi-Fi fallback path selected',
        'Use AirportItlwm with SecureBootModel for native-style and Recovery networking.',
      ));
    }

    if (kextNamesInConfig.includes('AppleMCEReporterDisabler') && profile && profile.architecture !== 'AMD' && parseMacOSVersion(profile.targetOS) < 10.15) {
      issues.push(warning(
        'APPLEMCE_POSSIBLY_UNNEEDED',
        'AppleMCEReporterDisabler appears unnecessary for this profile',
        'AppleMCEReporterDisabler.kext',
        'EFI/OC/Kexts/AppleMCEReporterDisabler.kext',
        'The selected CPU/OS combination is not one of the canonical documented cases',
        null,
      ));
    }

    // Cross-check with profile's required kexts
    if (profile) {
      const required = getRequiredResources(profile);
      const kextsOnDisk = await isDir(kextsDir)
        ? (await fs.readdir(kextsDir)).filter(e => e.endsWith('.kext'))
        : [];

      for (const reqKext of required.kexts) {
        if (!kextsOnDisk.includes(reqKext)) {
          issues.push(warning(
            'KEXT_EXPECTED_MISSING',
            `Expected kext not found on disk: ${reqKext}`,
            reqKext,
            `EFI/OC/Kexts/${reqKext}`,
            'Required by the selected hardware profile but missing on disk',
            'Based on hardware profile requirements',
          ));
        }
      }
    }
  }

  // ── 4. Patch sanity (warning) ───────────────────────────────────────────

  if (plistContent) {
    // AMD patch on non-AMD system
    if (profile && profile.architecture !== 'AMD' && hasAMDPatches(plistContent)) {
      issues.push(warning(
        'AMD_PATCHES_ON_INTEL',
        'AMD kernel patches detected on a non-AMD system',
        'Kernel patches',
        'EFI/OC/config.plist',
        `AMD patch entries are present for architecture ${profile.architecture}`,
        `Architecture: ${profile.architecture}`,
      ));
    }

    // AMD patch completeness — cpuid_cores_per_package patches are now auto-generated.
    // Warn only if core count is missing (scan failure).
    if (profile && profile.architecture === 'AMD' && hasAMDPatches(plistContent) && !profile.coreCount) {
      issues.push(warning(
        'AMD_CORE_COUNT_MISSING',
        'AMD core count was not detected — cpuid_cores_per_package patches may use wrong value',
        'Kernel patches',
        'EFI/OC/config.plist',
        'The hardware scan did not detect a core count. Re-run the scan or set the core count manually.',
        'Core count: unknown',
      ));
    }

    // Empty Find/Replace values
    const patches = extractPatchFindReplace(plistContent);
    for (const patch of patches) {
      if (patch.find === '' || patch.replace === '') {
        issues.push(warning(
          'PATCH_EMPTY_DATA',
          'Kernel patch has empty Find or Replace data',
          patch.comment || 'Kernel patch',
          'EFI/OC/config.plist',
          'Kernel patch entry has empty Find or Replace payload',
          patch.comment ? `Patch: ${patch.comment}` : null,
        ));
      }
    }
  }

  // ── 4b. PlatformInfo honesty (warning) ────────────────────────────────

  if (plistContent) {
    const serial = extractSimplePlistValue(plistContent, 'SystemSerialNumber');
    const uuid = extractSimplePlistValue(plistContent, 'SystemUUID');
    const mlb = extractSimplePlistValue(plistContent, 'MLB');

    const PLACEHOLDER_PATTERNS = [
      /^W0+1$/,                                    // W0000000001
      /^M0+1$/,                                    // M000000000001
      /^0{8}(-0{4}){3}-0{12}$/,                   // 00000000-0000-0000-0000-000000000000
      /^[A-Z]0{5,}/,                               // Any letter + many zeros
    ];

    const isPlaceholder = (v: string | null) => v ? PLACEHOLDER_PATTERNS.some(p => p.test(v)) : false;

    if (isPlaceholder(serial) || isPlaceholder(mlb) || isPlaceholder(uuid)) {
      issues.push(warning(
        'PLATFORMINFO_PLACEHOLDER_SERIALS',
        'PlatformInfo contains placeholder serial numbers',
        'PlatformInfo',
        'EFI/OC/config.plist',
        'MLB, SystemSerialNumber, or SystemUUID contain obvious placeholder values. iMessage, FaceTime, and Apple ID sign-in will fail. Generate valid serials with GenSMBIOS before booting.',
        `MLB: ${mlb ?? 'missing'}, Serial: ${serial ?? 'missing'}, UUID: ${uuid ?? 'missing'}`,
      ));
    }
  }

  // ── 5. SSDT check (warning) ─────────────────────────────────────────────

  if (plistContent) {
    const ssdtEntries = extractPlistArrayEntries(plistContent, ['ACPI', 'Add'], 'Path');
    const acpiDir = path.join(efiPath, 'EFI/OC/ACPI');

    for (const ssdtFile of ssdtEntries) {
      if (!(await exists(path.join(acpiDir, ssdtFile)))) {
        issues.push(warning(
          'SSDT_MISSING',
          `SSDT referenced in config but not found: ${ssdtFile}`,
          ssdtFile,
          `EFI/OC/ACPI/${ssdtFile}`,
          'config.plist references this SSDT but the file is missing on disk',
          `Expected at EFI/OC/ACPI/${ssdtFile}`,
        ));
      }
    }

    if (await isDir(acpiDir)) {
      const acpiEntries = await fs.readdir(acpiDir);
      for (const entry of acpiEntries) {
        if (entry.endsWith('.dsl')) {
          issues.push(blocked(
            'ACPI_DSL_PRESENT',
            'Uncompiled ACPI source file found in EFI',
            entry,
            `EFI/OC/ACPI/${entry}`,
            'Raw .dsl source file is present in the runtime EFI',
            'Compile AML files before shipping the EFI.',
          ));
        }
        if (entry.toLowerCase().startsWith('dsdt')) {
          issues.push(warning(
            'ACPI_DSDT_PRESENT',
            'DSDT dump found in ACPI folder',
            entry,
            `EFI/OC/ACPI/${entry}`,
            'A full DSDT dump was copied into the runtime EFI',
            'Canonical OpenCore builds normally ship targeted SSDTs, not raw DSDT dumps.',
          ));
        }
      }
    }
  }

  // ── 6. OpenCore version markers (warning/blocked) ──────────────────────

  const openCoreVersion = await readVersionSidecar(path.join(efiPath, 'EFI/OC/OpenCore.efi'));
  const bootVersion = await readVersionSidecar(path.join(efiPath, 'EFI/BOOT/BOOTx64.efi'));
  const openRuntimeVersion = await readVersionSidecar(path.join(efiPath, 'EFI/OC/Drivers/OpenRuntime.efi'));
  const hfsPlusVersion = await readVersionSidecar(path.join(efiPath, 'EFI/OC/Drivers/OpenHfsPlus.efi'));

  if (openCoreVersion && bootVersion && openCoreVersion !== bootVersion) {
    issues.push(blocked(
      'OPENCORE_VERSION_MISMATCH',
      'OpenCore.efi and BOOTx64.efi do not come from the same release set',
      'BOOTx64.efi',
      'EFI/BOOT/BOOTx64.efi',
      `Version marker mismatch: OpenCore ${openCoreVersion} vs BOOT ${bootVersion}`,
      null,
    ));
  }

  if (openCoreVersion && openRuntimeVersion && openCoreVersion !== openRuntimeVersion) {
    issues.push(blocked(
      'OPENRUNTIME_VERSION_MISMATCH',
      'OpenRuntime.efi does not match OpenCore.efi',
      'OpenRuntime.efi',
      'EFI/OC/Drivers/OpenRuntime.efi',
      `Version marker mismatch: OpenCore ${openCoreVersion} vs OpenRuntime ${openRuntimeVersion}`,
      null,
    ));
  }

  if (openCoreVersion && hfsPlusVersion && openCoreVersion !== hfsPlusVersion) {
    issues.push(warning(
      'HFSPPLUS_VERSION_MISMATCH',
      'OpenHfsPlus.efi does not match the OpenCore marker set',
      'OpenHfsPlus.efi',
      'EFI/OC/Drivers/OpenHfsPlus.efi',
      `Version marker mismatch: OpenCore ${openCoreVersion} vs HfsPlus ${hfsPlusVersion}`,
      null,
    ));
  }

  // ── Compute overall severity ────────────────────────────────────────────

  let overall: ValidationSeverity = 'pass';
  if (issues.some(i => i.severity === 'warning')) overall = 'warning';
  if (issues.some(i => i.severity === 'blocked')) overall = 'blocked';

  return {
    overall,
    issues,
    checkedAt: new Date().toISOString(),
    firstFailureTrace: buildValidationTrace(issues, kextSourceHints),
  };
}
