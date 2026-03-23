// ── Deterministic Validation Layer ───────────────────────────────────────────
// Ensures the app KNOWS whether an operation will succeed BEFORE executing it.
// Every critical operation has a dry-run that verifies all dependencies,
// resolves all URLs, and checks all preconditions. If the dry-run passes,
// the real operation is guaranteed to succeed (within network stability).

import * as https from 'https';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { queryAppleRecoveryAssets } from './appleRecovery.js';
import {
  resolveKextSourcePlan,
  type KextRegistryEntry,
} from './kextSourcePolicy.js';

// ─── Types ──────────────────────────────────────────────────────────────────

export type Certainty = 'will_succeed' | 'may_fail' | 'will_fail';

export interface ComponentVerification {
  name: string;
  type: 'opencore' | 'kext' | 'ssdt' | 'recovery' | 'disk';
  verified: boolean;
  certainty: Certainty;
  detail: string;
  url?: string;
  expectedSizeRange?: [number, number]; // [min, max] in bytes
  actualSize?: number;
}

export interface BuildPlan {
  timestamp: string;
  profile: string;                       // smbios
  certainty: Certainty;                  // overall
  components: ComponentVerification[];
  blockers: string[];                    // human-readable blockers
  totalComponents: number;
  verifiedComponents: number;
  failedComponents: number;
  estimatedBuildTimeMs: number;
}

export interface RecoveryDryRun {
  timestamp: string;
  targetOS: string;
  boardId: string;
  endpointReachable: boolean;
  testRequestResult: 'success' | 'auth_rejected' | 'server_error' | 'timeout' | 'unreachable';
  httpCode: number | null;
  certainty: Certainty;
  recommendation: string;
}

export interface SuccessContract {
  passed: boolean;
  checks: { name: string; passed: boolean; detail: string }[];
  certainty: Certainty;
}

export interface StateVerification {
  buildReady: boolean;
  efiPathValid: boolean;
  efiPathExists: boolean;
  openCoreExists: boolean;
  openCoreSize: number;
  configPlistValid: boolean;
  kextsFolderPopulated: boolean;
  missingKexts: string[];
}

// ─── Build Input Contract ────────────────────────────────────────────────────

export interface BuildInputContractResult {
  valid: boolean;
  violations: string[];
}

/**
 * Validates that a hardware profile has all the fields required to produce
 * a deterministic EFI build.  Call BEFORE starting any build or dry-run.
 */
export function validateBuildInputContract(profile: {
  architecture?: string;
  generation?: string;
  targetOS?: string;
  smbios?: string;
  motherboard?: string;
  coreCount?: number;
}): BuildInputContractResult {
  const violations: string[] = [];
  if (!profile.architecture) violations.push('architecture is required');
  if (!profile.generation) violations.push('generation is required');
  if (!profile.targetOS) violations.push('targetOS is required');
  if (!profile.smbios) violations.push('smbios is required');
  if (!profile.motherboard) violations.push('motherboard is required');
  if (profile.architecture === 'AMD' && (!profile.coreCount || profile.coreCount < 1)) {
    violations.push('AMD profiles require coreCount ≥ 1');
  }
  return { valid: violations.length === 0, violations };
}

// ─── HTTP Verification Helpers ──────────────────────────────────────────────

function verifyUrl(url: string, timeoutMs = 10000): Promise<{ reachable: boolean; httpCode: number; contentLength: number; error?: string }> {
  return new Promise((resolve) => {
    try {
      const parsedUrl = new URL(url);
      const lib = parsedUrl.protocol === 'https:' ? https : require('http');
      const req = lib.request({
        hostname: parsedUrl.hostname,
        port: parsedUrl.port || (parsedUrl.protocol === 'https:' ? 443 : 80),
        path: parsedUrl.pathname + parsedUrl.search,
        method: 'HEAD',
        headers: { 'User-Agent': 'OpCore-OneClick/1.0' },
        timeout: timeoutMs,
      }, (res: any) => {
        // Follow redirects
        if ([301, 302, 307, 308].includes(res.statusCode) && res.headers.location) {
          res.destroy();
          verifyUrl(res.headers.location, timeoutMs).then(resolve);
          return;
        }
        const contentLength = parseInt(res.headers['content-length'] || '0', 10);
        res.destroy();
        resolve({
          reachable: res.statusCode >= 200 && res.statusCode < 400,
          httpCode: res.statusCode,
          contentLength,
        });
      });
      req.on('error', (e: Error) => resolve({ reachable: false, httpCode: 0, contentLength: 0, error: e.message }));
      req.on('timeout', () => { req.destroy(); resolve({ reachable: false, httpCode: 0, contentLength: 0, error: 'timeout' }); });
      req.end();
    } catch (e: any) {
      resolve({ reachable: false, httpCode: 0, contentLength: 0, error: e.message });
    }
  });
}

function queryGitHubRelease(repo: string, assetFilter?: string): Promise<{ version: string; assetUrl: string | null; assetName: string | null; error?: string }> {
  return new Promise((resolve) => {
    const req = https.request({
      hostname: 'api.github.com',
      path: `/repos/${repo}/releases/latest`,
      headers: { 'User-Agent': 'OpCore-OneClick/1.0' },
      timeout: 10000,
    }, (res) => {
      if (res.statusCode === 403 || res.statusCode === 429) {
        res.resume();
        resolve({ version: '', assetUrl: null, assetName: null, error: 'GitHub rate limited' });
        return;
      }
      if ((res.statusCode ?? 0) >= 400) {
        res.resume();
        resolve({ version: '', assetUrl: null, assetName: null, error: `HTTP ${res.statusCode}` });
        return;
      }
      let data = '';
      res.on('data', (c: Buffer) => data += c);
      res.on('end', () => {
        try {
          const release = JSON.parse(data);
          const version = release.tag_name || 'unknown';
          const assets: { name: string; browser_download_url: string; size: number }[] = release.assets || [];
          let asset = assets.find(a => a.name.endsWith('.zip') && (!assetFilter || a.name.toUpperCase().includes(assetFilter.toUpperCase())));
          if (!asset) asset = assets.find(a => a.name.endsWith('.zip'));
          resolve({
            version,
            assetUrl: asset?.browser_download_url ?? null,
            assetName: asset?.name ?? null,
          });
        } catch (e) {
          resolve({ version: '', assetUrl: null, assetName: null, error: 'Failed to parse release' });
        }
      });
      res.on('error', (e) => resolve({ version: '', assetUrl: null, assetName: null, error: e.message }));
    });
    req.on('error', (e) => resolve({ version: '', assetUrl: null, assetName: null, error: e.message }));
    req.on('timeout', () => { req.destroy(); resolve({ version: '', assetUrl: null, assetName: null, error: 'timeout' }); });
    req.end();
  });
}

// ─── Phase 1: Build Dry-Run Simulation ──────────────────────────────────────

export async function simulateBuild(
  kextNames: string[],
  ssdtNames: string[],
  kextRegistry: Record<string, KextRegistryEntry>,
  smbios: string,
  ocVersion = '1.0.3',
): Promise<BuildPlan> {
  const components: ComponentVerification[] = [];
  const blockers: string[] = [];

  // 1. Verify OpenCore binaries URL
  const ocUrl = `https://github.com/acidanthera/OpenCorePkg/releases/download/${ocVersion}/OpenCore-${ocVersion}-RELEASE.zip`;
  const ocCheck = await verifyUrl(ocUrl);
  components.push({
    name: `OpenCore ${ocVersion}`,
    type: 'opencore',
    verified: ocCheck.reachable,
    certainty: ocCheck.reachable ? 'will_succeed' : 'will_fail',
    detail: ocCheck.reachable
      ? `Verified: ${Math.round(ocCheck.contentLength / 1024)} KB`
      : `Unreachable: ${ocCheck.error || `HTTP ${ocCheck.httpCode}`}`,
    url: ocUrl,
    actualSize: ocCheck.contentLength,
  });
  if (!ocCheck.reachable) blockers.push(`OpenCore ${ocVersion} binary is unreachable`);

  // 2. Verify each kext — resolve GitHub release and verify asset URL
  // Deduplicate kexts (some share repos)
  const checkedRepos = new Map<string, { version: string; assetUrl: string | null; error?: string }>();

  // Batch kexts in groups of 3 to avoid rate limiting
  const kextBatches: string[][] = [];
  for (let i = 0; i < kextNames.length; i += 3) {
    kextBatches.push(kextNames.slice(i, i + 3));
  }

  for (const batch of kextBatches) {
    const results = await Promise.all(batch.map(async (kextName) => {
      const entry = kextRegistry[kextName];
      if (!entry) {
        return {
          name: kextName,
          type: 'kext' as const,
          verified: true,
          certainty: 'will_succeed' as Certainty,
          detail: 'Bundled — no download needed',
        };
      }

      if (entry.directUrl) {
        const directCheck = await verifyUrl(entry.directUrl);
        const resolution = resolveKextSourcePlan(kextName, entry, null, {
          directUrlReachable: directCheck.reachable,
          directUrlError: directCheck.error || (directCheck.httpCode ? `HTTP ${directCheck.httpCode}` : null),
        });

        // If the direct URL probe failed, treat as may_fail — the actual download
        // at build time uses different networking and may still succeed.
        // Only will_fail is used for hard blockers (no source at all).
        return {
          name: kextName,
          type: 'kext' as const,
          verified: resolution.available,
          certainty: resolution.available ? 'will_succeed' as Certainty : 'may_fail' as Certainty,
          detail: resolution.available
            ? `${resolution.version ?? 'direct'} direct download verified`
            : `Direct URL unreachable in pre-check — will attempt at build time. ${resolution.message}`,
          url: entry.directUrl,
          actualSize: directCheck.contentLength,
        };
      }

      // Check if we already queried this repo
      let releaseInfo = checkedRepos.get(entry.repo);
      if (!releaseInfo) {
        releaseInfo = await queryGitHubRelease(entry.repo, entry.assetFilter);
        checkedRepos.set(entry.repo, releaseInfo);
      }

      const releaseResolution = resolveKextSourcePlan(kextName, entry, releaseInfo);

      if (releaseResolution.route === 'embedded') {
        return {
          name: kextName,
          type: 'kext' as const,
          verified: true,
          certainty: 'will_succeed' as Certainty,
          detail: releaseResolution.message,
          url: `https://github.com/${entry.repo}/releases/latest`,
        };
      }

      if (!releaseResolution.available || !releaseResolution.assetUrl) {
        return {
          name: kextName,
          type: 'kext' as const,
          verified: false,
          certainty: 'will_fail' as Certainty,
          detail: releaseResolution.message,
          url: `https://github.com/${entry.repo}/releases/latest`,
        };
      }

      // Verify the actual download URL resolves
      const assetCheck = await verifyUrl(releaseResolution.assetUrl);
      if (!assetCheck.reachable && entry.embeddedFallback) {
        return {
          name: kextName,
          type: 'kext' as const,
          verified: true,
          certainty: 'will_succeed' as Certainty,
          detail: `${kextName} asset URL failed, but a bundled fallback is ready.`,
          url: `https://github.com/${entry.repo}/releases/latest`,
        };
      }
      return {
        name: kextName,
        type: 'kext' as const,
        verified: assetCheck.reachable,
        certainty: assetCheck.reachable ? 'will_succeed' : 'may_fail' as Certainty,
        detail: assetCheck.reachable
          ? `${releaseResolution.version ?? 'unknown'} verified (${Math.round(assetCheck.contentLength / 1024)} KB)`
          : `Asset URL failed: ${assetCheck.error || `HTTP ${assetCheck.httpCode}`}`,
        url: releaseResolution.assetUrl,
        actualSize: assetCheck.contentLength,
      };
    }));
    components.push(...results);
  }

  // 3. SSDTs — these are generated/bundled, always succeed
  for (const ssdt of ssdtNames) {
    components.push({
      name: ssdt,
      type: 'ssdt',
      verified: true,
      certainty: 'will_succeed',
      detail: 'Generated locally — no download needed',
    });
  }

  // 4. Disk write check
  const tmpDir = os.tmpdir();
  let diskOk = false;
  try {
    const testFile = path.join(tmpDir, `.oc_dry_run_${Date.now()}`);
    fs.writeFileSync(testFile, 'test');
    fs.unlinkSync(testFile);
    diskOk = true;
  } catch { /* disk not writable */ }

  components.push({
    name: 'Disk write',
    type: 'disk',
    verified: diskOk,
    certainty: diskOk ? 'will_succeed' : 'will_fail',
    detail: diskOk ? 'Build directory writable' : 'Cannot write to build directory',
  });
  if (!diskOk) blockers.push('Build directory is not writable');

  // Generate failed kext blockers — only hard-block on will_fail certainty.
  // may_fail means the pre-check URL probe failed but actual download may still work.
  const failedKexts = components.filter(c => c.type === 'kext' && !c.verified && c.certainty === 'will_fail');
  if (failedKexts.length > 0) {
    blockers.push(`${failedKexts.length} kext(s) unavailable: ${failedKexts.map(k => k.name).join(', ')}`);
  }

  const verified = components.filter(c => c.verified).length;
  const failed = components.filter(c => !c.verified).length;

  const overall: Certainty = blockers.length > 0
    ? 'will_fail'
    : failed > 0
      ? 'may_fail'
      : 'will_succeed';

  return {
    timestamp: new Date().toISOString(),
    profile: smbios,
    certainty: overall,
    components,
    blockers,
    totalComponents: components.length,
    verifiedComponents: verified,
    failedComponents: failed,
    estimatedBuildTimeMs: 15000 + (kextNames.length * 5000),
  };
}

// ─── Phase 2: Recovery Dry-Run ──────────────────────────────────────────────

const BOARD_IDS: Record<string, string> = {
  '16': 'Mac-827FAC58A8FDFA22',
  '15': 'Mac-827FAC58A8FDFA22',
  '14': 'Mac-827FAC58A8FDFA22',
  '13': 'Mac-4B682C642B45593E',
  '12': 'Mac-FFE5EF870D7BA81A',
  '11': 'Mac-42FD25EABCABB274',
  '10.15': 'Mac-00BE6ED71E35EB86',
};

function extractOSVersionKey(targetOS: string): string {
  const m = targetOS.match(/(\d+(?:\.\d+)?)/);
  return m ? m[1] : '15';
}

export async function dryRunRecovery(
  targetOS: string,
  _smbios: string,
): Promise<RecoveryDryRun> {
  const versionKey = extractOSVersionKey(targetOS);
  const boardId = BOARD_IDS[versionKey] || BOARD_IDS['15'];
  try {
    await queryAppleRecoveryAssets({ boardId, osType: 'default' });
    return {
      timestamp: new Date().toISOString(),
      targetOS,
      boardId,
      endpointReachable: true,
      testRequestResult: 'success',
      httpCode: 200,
      certainty: 'will_succeed',
      recommendation: 'Recovery download is expected to succeed.',
    };
  } catch (error: any) {
    const message = error?.message ?? 'unknown';
    if (message.startsWith('APPLE_AUTH_REJECT')) {
      const match = message.match(/:(\d{3})$/);
      return {
        timestamp: new Date().toISOString(),
        targetOS,
        boardId,
        endpointReachable: true,
        testRequestResult: 'auth_rejected',
        httpCode: match ? Number(match[1]) : null,
        certainty: 'will_fail',
        recommendation: `Apple rejected the recovery request (HTTP ${match ? Number(match[1]) : 'unknown'}). This target will likely fail until the request shape is corrected or Apple changes policy.`,
      };
    }
    if (message.startsWith('APPLE_SERVER_ERROR') || message.startsWith('APPLE_HTTP')) {
      const match = message.match(/:(\d{3})$/);
      return {
        timestamp: new Date().toISOString(),
        targetOS,
        boardId,
        endpointReachable: true,
        testRequestResult: 'server_error',
        httpCode: match ? Number(match[1]) : null,
        certainty: 'will_fail',
        recommendation: 'Apple recovery servers returned a non-success response. Retry later if the service is degraded.',
      };
    }
    if (message.includes('timeout')) {
      return {
        timestamp: new Date().toISOString(),
        targetOS,
        boardId,
        endpointReachable: false,
        testRequestResult: 'timeout',
        httpCode: null,
        certainty: 'may_fail',
        recommendation: 'Apple server timed out. Network may be slow, but the request path is otherwise valid.',
      };
    }
    return {
      timestamp: new Date().toISOString(),
      targetOS,
      boardId,
      endpointReachable: false,
      testRequestResult: 'unreachable',
      httpCode: null,
      certainty: 'will_fail',
      recommendation: 'Apple recovery server is unreachable. Check network or use manual import.',
    };
  }
}

// ─── Phase 3: Guaranteed State Verification ─────────────────────────────────
// Always verifies from disk, never trusts boolean flags alone.

export function verifyBuildState(efiPath: string | null, requiredKexts: string[]): StateVerification {
  if (!efiPath) {
    return {
      buildReady: false,
      efiPathValid: false,
      efiPathExists: false,
      openCoreExists: false,
      openCoreSize: 0,
      configPlistValid: false,
      kextsFolderPopulated: false,
      missingKexts: requiredKexts,
    };
  }

  const efiPathExists = fs.existsSync(efiPath);
  if (!efiPathExists) {
    return {
      buildReady: false,
      efiPathValid: true,
      efiPathExists: false,
      openCoreExists: false,
      openCoreSize: 0,
      configPlistValid: false,
      kextsFolderPopulated: false,
      missingKexts: requiredKexts,
    };
  }

  // Check OpenCore.efi
  const ocPath = path.join(efiPath, 'EFI/OC/OpenCore.efi');
  const ocExists = fs.existsSync(ocPath);
  let ocSize = 0;
  if (ocExists) {
    try { ocSize = fs.statSync(ocPath).size; } catch { /* */ }
  }

  // Check config.plist
  const plistPath = path.join(efiPath, 'EFI/OC/config.plist');
  let plistValid = false;
  if (fs.existsSync(plistPath)) {
    try {
      const content = fs.readFileSync(plistPath, 'utf-8');
      plistValid = content.includes('<plist') && content.includes('</plist>') && content.includes('<dict>');
    } catch { /* */ }
  }

  // Check kexts — deep validation: Contents/MacOS must exist with binary > 1KB
  const kextsDir = path.join(efiPath, 'EFI/OC/Kexts');
  const kextsExist = fs.existsSync(kextsDir);
  const missingKexts: string[] = [];
  if (kextsExist) {
    for (const k of requiredKexts) {
      const kextPath = path.join(kextsDir, k);
      if (!fs.existsSync(kextPath)) {
        missingKexts.push(k);
        continue;
      }
      const macosDir = path.join(kextPath, 'Contents', 'MacOS');
      if (!fs.existsSync(macosDir)) {
        missingKexts.push(k);
        continue;
      }
      try {
        const files = fs.readdirSync(macosDir);
        const hasBinary = files.some(f => fs.statSync(path.join(macosDir, f)).size > 1024);
        if (!hasBinary) missingKexts.push(k);
      } catch {
        missingKexts.push(k);
      }
    }
  } else {
    missingKexts.push(...requiredKexts);
  }

  const buildReady = ocExists && ocSize > 100 * 1024 && plistValid && missingKexts.length === 0;

  return {
    buildReady,
    efiPathValid: true,
    efiPathExists,
    openCoreExists: ocExists,
    openCoreSize: ocSize,
    configPlistValid: plistValid,
    kextsFolderPopulated: kextsExist && missingKexts.length === 0,
    missingKexts,
  };
}

// ─── Phase 4: Hard Success Contracts ────────────────────────────────────────

export function verifyEfiBuildSuccess(efiPath: string, requiredKexts: string[], requiredSsdts?: string[]): SuccessContract {
  const checks: { name: string; passed: boolean; detail: string }[] = [];

  // Check 1: OpenCore.efi exists and is > 100KB
  const ocPath = path.join(efiPath, 'EFI/OC/OpenCore.efi');
  let ocOk = false;
  let ocDetail = '';
  try {
    const stat = fs.statSync(ocPath);
    ocOk = stat.size > 100 * 1024;
    ocDetail = ocOk ? `${Math.round(stat.size / 1024)} KB` : `Too small: ${stat.size} bytes`;
  } catch {
    ocDetail = 'File not found';
  }
  checks.push({ name: 'OpenCore.efi', passed: ocOk, detail: ocDetail });

  // Check 2: BOOTx64.efi exists
  const bootPath = path.join(efiPath, 'EFI/BOOT/BOOTx64.efi');
  let bootOk = false;
  try {
    const stat = fs.statSync(bootPath);
    bootOk = stat.size > 20 * 1024;
    checks.push({ name: 'BOOTx64.efi', passed: bootOk, detail: `${Math.round(stat.size / 1024)} KB` });
  } catch {
    checks.push({ name: 'BOOTx64.efi', passed: false, detail: 'File not found' });
  }

  // Check 3: config.plist valid
  const plistPath = path.join(efiPath, 'EFI/OC/config.plist');
  let plistOk = false;
  try {
    const content = fs.readFileSync(plistPath, 'utf-8');
    plistOk = content.length > 500 && content.includes('<plist') && content.includes('</plist>');
    checks.push({ name: 'config.plist', passed: plistOk, detail: plistOk ? `${Math.round(content.length / 1024)} KB, valid structure` : 'Invalid or too small' });
  } catch {
    checks.push({ name: 'config.plist', passed: false, detail: 'File not found or unreadable' });
  }

  // Check 4: All required kexts present with valid binaries (or valid codeless kexts)
  const kextsDir = path.join(efiPath, 'EFI/OC/Kexts');
  const missingKexts: string[] = [];
  const stubKexts: string[] = [];
  for (const k of requiredKexts) {
    const kextPath = path.join(kextsDir, k);
    if (!fs.existsSync(kextPath)) { missingKexts.push(k); continue; }
    const macosDir = path.join(kextPath, 'Contents', 'MacOS');
    if (!fs.existsSync(macosDir)) {
      // Codeless kexts (e.g. AppleMCEReporterDisabler) have Info.plist but no binary
      const plistFile = path.join(kextPath, 'Contents', 'Info.plist');
      let validCodeless = false;
      try {
        const plistContent = fs.readFileSync(plistFile, 'utf-8');
        validCodeless = plistContent.includes('<plist') && plistContent.includes('CFBundleIdentifier');
      } catch { /* missing or unreadable */ }
      if (!validCodeless) stubKexts.push(k);
      continue;
    }
    try {
      const files = fs.readdirSync(macosDir);
      const hasBinary = files.some(f => fs.statSync(path.join(macosDir, f)).size > 1024);
      if (!hasBinary) stubKexts.push(k);
    } catch {
      stubKexts.push(k);
    }
  }
  const kextsOk = missingKexts.length === 0 && stubKexts.length === 0;
  checks.push({
    name: 'Required kexts',
    passed: kextsOk,
    detail: kextsOk
      ? `All ${requiredKexts.length} kexts present with valid binaries`
      : missingKexts.length > 0 && stubKexts.length > 0
        ? `Missing: ${missingKexts.join(', ')}; Stub (no binary): ${stubKexts.join(', ')}`
        : missingKexts.length > 0
          ? `Missing: ${missingKexts.join(', ')}`
          : `Stub kexts (no valid binary in Contents/MacOS): ${stubKexts.join(', ')}`,
  });

  // Check 5: No nested EFI structure
  const nestedOk = !fs.existsSync(path.join(efiPath, 'EFI/EFI'));
  checks.push({ name: 'Structure integrity', passed: nestedOk, detail: nestedOk ? 'Clean' : 'Nested EFI/EFI detected' });

  // Check 6: ACPI directory exists (SSDTs cannot be loaded without it)
  const acpiDir = path.join(efiPath, 'EFI/OC/ACPI');
  const acpiOk = fs.existsSync(acpiDir) && fs.statSync(acpiDir).isDirectory();
  checks.push({ name: 'ACPI directory', passed: acpiOk, detail: acpiOk ? 'Present' : 'Missing — SSDTs cannot be loaded by OpenCore' });

  // Check 7: Required SSDTs present (when caller provides the list)
  if (requiredSsdts && requiredSsdts.length > 0) {
    const missingSsdts = requiredSsdts.filter(s => !fs.existsSync(path.join(acpiDir, s)));
    const ssdtsOk = missingSsdts.length === 0;
    checks.push({
      name: 'Required SSDTs',
      passed: ssdtsOk,
      detail: ssdtsOk
        ? `All ${requiredSsdts.length} required SSDTs present`
        : `Missing from EFI/OC/ACPI: ${missingSsdts.join(', ')}`,
    });
  }

  const allPassed = checks.every(c => c.passed);

  return {
    passed: allPassed,
    checks,
    certainty: allPassed ? 'will_succeed' : 'will_fail',
  };
}

export function verifyRecoverySuccess(recoveryDir: string): SuccessContract {
  const checks: { name: string; passed: boolean; detail: string }[] = [];

  // Check 1: BaseSystem.dmg exists
  const dmgPath = path.join(recoveryDir, 'com.apple.recovery.boot', 'BaseSystem.dmg');
  let dmgOk = false;
  let dmgSize = 0;
  try {
    const stat = fs.statSync(dmgPath);
    dmgSize = stat.size;
    // Valid range: 350MB to 1GB
    dmgOk = dmgSize > 350 * 1024 * 1024 && dmgSize < 1024 * 1024 * 1024;
    checks.push({
      name: 'BaseSystem.dmg',
      passed: dmgOk,
      detail: dmgOk
        ? `${Math.round(dmgSize / (1024 * 1024))} MB — valid range`
        : dmgSize > 0
          ? `${Math.round(dmgSize / (1024 * 1024))} MB — outside expected range (350MB-1GB)`
          : 'File is empty',
    });
  } catch {
    checks.push({ name: 'BaseSystem.dmg', passed: false, detail: 'File not found' });
  }

  // Check 2: Not a partial download (check for consistent size — at least > 350MB)
  const notPartial = dmgSize > 350 * 1024 * 1024;
  checks.push({ name: 'Download completeness', passed: notPartial, detail: notPartial ? 'File size consistent with complete download' : 'File appears incomplete or partial' });

  // Check 3: Recovery boot directory exists
  const recovBootDir = path.join(recoveryDir, 'com.apple.recovery.boot');
  const dirOk = fs.existsSync(recovBootDir);
  checks.push({ name: 'Recovery directory', passed: dirOk, detail: dirOk ? 'Present' : 'Missing' });

  const allPassed = checks.every(c => c.passed);
  return {
    passed: allPassed,
    checks,
    certainty: allPassed ? 'will_succeed' : 'will_fail',
  };
}
