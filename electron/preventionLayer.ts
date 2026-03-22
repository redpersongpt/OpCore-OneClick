// ── Prevention Layer ─────────────────────────────────────────────────────────
// Predicts and prevents failures BEFORE they happen.
// Advisory system that checks environment health, kext availability, Apple
// endpoint reachability, and tracks failure patterns to avoid blind retries.

import * as https from 'https';
import * as net from 'net';
import * as tls from 'tls';
import * as fs from 'fs';
import * as os from 'os';
import { probeAppleRecoveryEndpoint } from './appleRecovery.js';
import {
  resolveKextSourcePlan,
  type KextRegistryEntry,
} from './kextSourcePolicy.js';

// ── Types ────────────────────────────────────────────────────────────────────

export type ConfidenceLevel = 'green' | 'yellow' | 'red';

export interface EndpointCheck {
  name: string;
  reachable: boolean;
  latencyMs: number;
  httpCode?: number;
  error?: string;
}

export interface GitHubRateLimitInfo {
  remaining: number;
  limit: number;
  resetAt: string;        // ISO timestamp
  sufficient: boolean;    // remaining > needed
}

export interface KextAvailability {
  name: string;
  repo: string;
  available: boolean;
  version?: string;
  assetUrl?: string;
  error?: string;
  source?: 'bundled' | 'github' | 'direct' | 'embedded' | 'failed';
}

export interface DiskHealthCheck {
  writable: boolean;
  freeSpaceMB: number;
  sufficientSpace: boolean;   // > 2GB recommended
  error?: string;
}

export interface RecoveryEndpointCheck {
  reachable: boolean;
  latencyMs: number;
  httpCode?: number;
  error?: string;
}

export interface PreflightReport {
  timestamp: string;
  network: {
    githubApi: EndpointCheck;
    githubReleases: EndpointCheck;
    appleRecovery: EndpointCheck;
    overall: 'ok' | 'degraded' | 'down';
  };
  githubRateLimit: GitHubRateLimitInfo | null;
  kextAvailability: KextAvailability[];
  kextAllAvailable: boolean;
  diskHealth: DiskHealthCheck;
  recoveryEndpoint: RecoveryEndpointCheck;
  confidence: ConfidenceLevel;
  warnings: string[];
  blockers: string[];
}

export interface FailureMemoryEntry {
  code: string;         // e.g. 'kext_Lilu', 'recovery_auth', 'flash_write'
  count: number;
  lastMessage: string;
  lastTimestamp: number;
}

// ── Failure Memory ───────────────────────────────────────────────────────────
// Tracks recent failures across the session to avoid blind retries.

const failureMemory = new Map<string, FailureMemoryEntry>();

export function recordFailure(code: string, message: string): FailureMemoryEntry {
  const existing = failureMemory.get(code);
  const entry: FailureMemoryEntry = {
    code,
    count: (existing?.count ?? 0) + 1,
    lastMessage: message,
    lastTimestamp: Date.now(),
  };
  failureMemory.set(code, entry);
  return entry;
}

export function getFailureCount(code: string): number {
  return failureMemory.get(code)?.count ?? 0;
}

export function getFailureMemory(): FailureMemoryEntry[] {
  return Array.from(failureMemory.values());
}

export function shouldSkipRetry(code: string, threshold = 2): boolean {
  return getFailureCount(code) >= threshold;
}

export function clearFailureMemory(): void {
  failureMemory.clear();
}

// ── Network Probes ───────────────────────────────────────────────────────────

function probeEndpoint(hostname: string, path: string, method: 'HEAD' | 'GET' = 'HEAD', timeoutMs = 8000): Promise<EndpointCheck> {
  const start = Date.now();
  const name = hostname;
  return new Promise((resolve) => {
    const req = https.request({ hostname, path, method, timeout: timeoutMs }, (res) => {
      res.destroy();
      resolve({
        name,
        reachable: (res.statusCode ?? 0) < 500,
        latencyMs: Date.now() - start,
        httpCode: res.statusCode,
      });
    });
    req.on('error', (e) => resolve({ name, reachable: false, latencyMs: Date.now() - start, error: e.message }));
    req.on('timeout', () => { req.destroy(); resolve({ name, reachable: false, latencyMs: timeoutMs, error: 'timeout' }); });
    req.end();
  });
}

// ── GitHub Rate Limit Check ──────────────────────────────────────────────────

function checkGitHubRateLimit(): Promise<GitHubRateLimitInfo | null> {
  return new Promise((resolve) => {
    const req = https.request({
      hostname: 'api.github.com',
      path: '/rate_limit',
      headers: { 'User-Agent': 'OpCore-OneClick/1.0' },
      timeout: 8000,
    }, (res) => {
      let data = '';
      res.on('data', (c: Buffer) => data += c);
      res.on('end', () => {
        try {
          const json = JSON.parse(data);
          const core = json.resources?.core;
          if (!core) { resolve(null); return; }
          const resetAt = new Date(core.reset * 1000).toISOString();
          resolve({
            remaining: core.remaining,
            limit: core.limit,
            resetAt,
            sufficient: core.remaining > 15, // typical build needs ~10 API calls
          });
        } catch { resolve(null); }
      });
      res.on('error', () => resolve(null));
    });
    req.on('error', () => resolve(null));
    req.on('timeout', () => { req.destroy(); resolve(null); });
    req.end();
  });
}

// ── Kext Availability Check ──────────────────────────────────────────────────
// Uses HEAD requests against GitHub or direct asset URLs to verify each kext
// has a real delivery path before the build starts.

function probeUrl(urlString: string, timeoutMs = 10_000): Promise<{ reachable: boolean; httpCode?: number; error?: string }> {
  return new Promise((resolve) => {
    let parsed: URL;
    try {
      parsed = new URL(urlString);
    } catch {
      resolve({ reachable: false, error: `Invalid URL: ${urlString}` });
      return;
    }

    const isHttps = parsed.protocol === 'https:';
    const lib = isHttps ? https : require('http');
    const req = lib.request({
      hostname: parsed.hostname,
      port: parsed.port || (isHttps ? 443 : 80),
      path: parsed.pathname + parsed.search,
      method: 'HEAD',
      headers: { 'User-Agent': 'OpCore-OneClick/1.0' },
      timeout: timeoutMs,
    }, (res: any) => {
      if ([301, 302, 307, 308].includes(res.statusCode) && res.headers.location) {
        res.destroy();
        probeUrl(res.headers.location, timeoutMs).then(resolve);
        return;
      }
      res.destroy();
      resolve({
        reachable: (res.statusCode ?? 0) >= 200 && (res.statusCode ?? 0) < 400,
        httpCode: res.statusCode,
      });
    });
    req.on('error', (error: Error) => resolve({ reachable: false, error: error.message }));
    req.on('timeout', () => {
      req.destroy();
      resolve({ reachable: false, error: 'timeout' });
    });
    req.end();
  });
}

function checkKextAvailability(
  kextName: string,
  registry: Record<string, KextRegistryEntry>,
): Promise<KextAvailability> {
  const entry = registry[kextName];
  if (!entry) {
    return Promise.resolve({
      name: kextName,
      repo: 'bundled',
      available: true,
      version: 'bundled',
      source: 'bundled',
    });
  }

  if (entry.directUrl) {
    return probeUrl(entry.directUrl, 10_000).then((result) => {
      const resolution = resolveKextSourcePlan(kextName, entry, null, {
        directUrlReachable: result.reachable,
        directUrlError: result.error ?? (result.httpCode ? `HTTP ${result.httpCode}` : null),
      });
      return {
        name: kextName,
        repo: entry.repo,
        available: resolution.available,
        version: resolution.version ?? undefined,
        assetUrl: resolution.assetUrl ?? undefined,
        error: resolution.available ? undefined : resolution.message,
        source: resolution.route,
      };
    });
  }

  return new Promise((resolve) => {
    const req = https.request({
      hostname: 'api.github.com',
      path: `/repos/${entry.repo}/releases/latest`,
      headers: { 'User-Agent': 'OpCore-OneClick/1.0' },
      timeout: 10000,
    }, (res) => {
      if (res.statusCode === 403 || res.statusCode === 429) {
        res.resume();
        resolve({ name: kextName, repo: entry.repo, available: false, error: 'GitHub rate limited' });
        return;
      }
      if (res.statusCode !== 200) {
        res.resume();
        resolve({ name: kextName, repo: entry.repo, available: false, error: `HTTP ${res.statusCode}` });
        return;
      }
      let data = '';
      res.on('data', (c: Buffer) => data += c);
      res.on('end', () => {
        try {
          const release = JSON.parse(data);
          const resolution = resolveKextSourcePlan(kextName, entry, {
            version: release.tag_name || 'unknown',
            assetUrl: (() => {
              const assets: { name: string; browser_download_url: string }[] = release.assets || [];
              let asset = assets.find(a => a.name.endsWith('.zip') && (!entry.assetFilter || a.name.toUpperCase().includes(entry.assetFilter.toUpperCase())));
              if (!asset) asset = assets.find(a => a.name.endsWith('.zip'));
              return asset?.browser_download_url ?? null;
            })(),
            error: null,
          });
          resolve({
            name: kextName,
            repo: entry.repo,
            available: resolution.available,
            version: resolution.version ?? undefined,
            assetUrl: resolution.assetUrl ?? undefined,
            error: resolution.available ? undefined : resolution.message,
            source: resolution.route,
          });
        } catch {
          const resolution = resolveKextSourcePlan(kextName, entry, { error: 'Failed to parse release' });
          resolve({
            name: kextName,
            repo: entry.repo,
            available: resolution.available,
            version: resolution.version ?? undefined,
            assetUrl: resolution.assetUrl ?? undefined,
            error: resolution.available ? undefined : resolution.message,
            source: resolution.route,
          });
        }
      });
      res.on('error', (e) => {
        const resolution = resolveKextSourcePlan(kextName, entry, { error: e.message });
        resolve({
          name: kextName,
          repo: entry.repo,
          available: resolution.available,
          version: resolution.version ?? undefined,
          assetUrl: resolution.assetUrl ?? undefined,
          error: resolution.available ? undefined : resolution.message,
          source: resolution.route,
        });
      });
    });
    req.on('error', (e) => {
      const resolution = resolveKextSourcePlan(kextName, entry, { error: e.message });
      resolve({
        name: kextName,
        repo: entry.repo,
        available: resolution.available,
        version: resolution.version ?? undefined,
        assetUrl: resolution.assetUrl ?? undefined,
        error: resolution.available ? undefined : resolution.message,
        source: resolution.route,
      });
    });
    req.on('timeout', () => {
      req.destroy();
      const resolution = resolveKextSourcePlan(kextName, entry, { error: 'timeout' });
      resolve({
        name: kextName,
        repo: entry.repo,
        available: resolution.available,
        version: resolution.version ?? undefined,
        assetUrl: resolution.assetUrl ?? undefined,
        error: resolution.available ? undefined : resolution.message,
        source: resolution.route,
      });
    });
    req.end();
  });
}

// ── Disk Health ──────────────────────────────────────────────────────────────

function checkDiskHealth(targetDir?: string): DiskHealthCheck {
  const dir = targetDir || os.tmpdir();
  try {
    // Check writable
    const testFile = `${dir}/.oc_preflight_test_${Date.now()}`;
    fs.writeFileSync(testFile, 'test');
    fs.unlinkSync(testFile);

    // Check free space (simple heuristic via os.freemem — real disk space
    // requires platform-specific calls; we use the tmpdir as a proxy)
    let freeSpaceMB = Infinity;
    try {
      const stats = fs.statfsSync(dir);
      freeSpaceMB = Math.round((stats.bfree * stats.bsize) / (1024 * 1024));
    } catch {
      // statfsSync not available on all platforms
    }

    return {
      writable: true,
      freeSpaceMB,
      sufficientSpace: freeSpaceMB > 2048, // 2GB minimum recommended
    };
  } catch (e: any) {
    return {
      writable: false,
      freeSpaceMB: 0,
      sufficientSpace: false,
      error: e.message,
    };
  }
}

// ── Apple Recovery Pre-check ─────────────────────────────────────────────────

function probeAppleRecovery(): Promise<RecoveryEndpointCheck> {
  const start = Date.now();
  return probeAppleRecoveryEndpoint().then((result) => ({
    reachable: result.reachable && Boolean(result.sessionCookie),
    latencyMs: Date.now() - start,
    httpCode: result.httpCode ?? undefined,
    error: result.reachable && !result.sessionCookie ? 'missing_session_cookie' : undefined,
  })).catch((error: any) => ({
    reachable: false,
    latencyMs: Date.now() - start,
    error: error?.message ?? 'unknown_error',
  }));
}

// ── Confidence Computation ───────────────────────────────────────────────────

function computeConfidence(
  network: PreflightReport['network'],
  rateLimit: GitHubRateLimitInfo | null,
  kextAllAvailable: boolean,
  disk: DiskHealthCheck,
  recovery: RecoveryEndpointCheck,
  warnings: string[],
  blockers: string[],
): ConfidenceLevel {
  if (blockers.length > 0) return 'red';
  if (warnings.length > 2) return 'red';
  if (warnings.length > 0) return 'yellow';
  if (!network.githubApi.reachable || !network.githubReleases.reachable) return 'yellow';
  if (rateLimit && !rateLimit.sufficient) return 'yellow';
  if (!kextAllAvailable) return 'yellow';
  if (!disk.sufficientSpace) return 'yellow';
  if (!recovery.reachable) return 'yellow';
  return 'green';
}

// ── Main Preflight Runner ────────────────────────────────────────────────────

export async function runPreflightChecks(
  kextNames: string[],
  kextRegistry: Record<string, KextRegistryEntry>,
  targetDir?: string,
): Promise<PreflightReport> {
  const warnings: string[] = [];
  const blockers: string[] = [];

  // Run all network checks in parallel
  const [githubApi, githubReleases, appleRecovery, rateLimit] = await Promise.all([
    probeEndpoint('api.github.com', '/'),
    probeEndpoint('github.com', '/'),
    probeAppleRecovery(),
    checkGitHubRateLimit(),
  ]);

  const apiDependentKexts = kextNames.filter((name) => {
    const entry = kextRegistry[name];
    return !!entry && !entry.directUrl && !entry.embeddedFallback;
  });

  // Network overall assessment
  const networkOverall = (!githubApi.reachable && !appleRecovery.reachable)
    ? 'down' as const
    : (!githubApi.reachable || !githubReleases.reachable || !appleRecovery.reachable)
      ? 'degraded' as const
      : 'ok' as const;

  if (networkOverall === 'down') {
    blockers.push('No network connectivity — both GitHub and Apple servers are unreachable.');
  } else if (!githubApi.reachable) {
    warnings.push(
      apiDependentKexts.length > 0
        ? 'GitHub API is unreachable. Some kexts have no local fallback, so the build may stop before writing anything.'
        : 'GitHub API is unreachable. Bundled or direct-download fallbacks will be used where available.',
    );
  }

  // Rate limit
  if (rateLimit && !rateLimit.sufficient) {
    warnings.push(
      apiDependentKexts.length > 0
        ? `GitHub API rate limit is low (${rateLimit.remaining}/${rateLimit.limit} remaining). Some kexts may fail until the limit resets at ${new Date(rateLimit.resetAt).toLocaleTimeString()}.`
        : `GitHub API rate limit is low (${rateLimit.remaining}/${rateLimit.limit} remaining), but the required kexts have bundled or direct-download fallbacks.`,
    );
  }

  // Kext availability — still evaluate every kext even if GitHub is degraded,
  // because some entries can use direct URLs or bundled fallbacks.
  let kextResults: KextAvailability[] = [];
  let kextAllAvailable = true;
  if (kextNames.length > 0) {
    // Check kexts in batches of 4 to avoid hammering the API
    const batches: string[][] = [];
    for (let i = 0; i < kextNames.length; i += 4) {
      batches.push(kextNames.slice(i, i + 4));
    }
    for (const batch of batches) {
      const results = await Promise.all(
        batch.map(k => checkKextAvailability(k, kextRegistry))
      );
      kextResults.push(...results);
    }
    const unavailable = kextResults.filter(k => !k.available);
    kextAllAvailable = unavailable.length === 0;
    if (unavailable.length > 0) {
      const names = unavailable.map(k => `${k.name}${k.error ? ` (${k.error})` : ''}`).join(', ');
      warnings.push(`${unavailable.length} required kext(s) are not reachable: ${names}. The build will stop before writing anything. Check the Resource Plan for the exact source path, then retry once the network or GitHub limit recovers.`);
    }
  }

  // Disk health
  const diskHealth = checkDiskHealth(targetDir);
  if (!diskHealth.writable) {
    blockers.push('Build directory is not writable.');
  }
  if (!diskHealth.sufficientSpace && diskHealth.freeSpaceMB < 500) {
    blockers.push(`Critically low disk space: ${diskHealth.freeSpaceMB} MB free. At least 2 GB recommended.`);
  } else if (!diskHealth.sufficientSpace) {
    warnings.push(`Low disk space: ${diskHealth.freeSpaceMB} MB free. 2 GB+ recommended for a reliable build.`);
  }

  // Apple recovery
  if (!appleRecovery.reachable) {
    warnings.push('Apple recovery server is unreachable — recovery download will likely fail.');
  }

  // Failure memory integration
  for (const entry of failureMemory.values()) {
    if (entry.count >= 2 && Date.now() - entry.lastTimestamp < 30 * 60 * 1000) {
      if (entry.code.startsWith('kext_')) {
        warnings.push(`${entry.code.replace('kext_', '')} has failed ${entry.count} times this session. Manual download recommended.`);
      } else if (entry.code === 'recovery_auth') {
        warnings.push(`Apple recovery has failed ${entry.count} times. Consider switching macOS version or using manual import.`);
      }
    }
  }

  const appleEndpoint: EndpointCheck = { ...appleRecovery, name: 'osrecovery.apple.com' };
  const confidence = computeConfidence(
    { githubApi, githubReleases, appleRecovery: appleEndpoint, overall: networkOverall },
    rateLimit,
    kextAllAvailable,
    diskHealth,
    appleRecovery,
    warnings,
    blockers,
  );

  return {
    timestamp: new Date().toISOString(),
    network: { githubApi, githubReleases, appleRecovery: appleEndpoint, overall: networkOverall },
    githubRateLimit: rateLimit,
    kextAvailability: kextResults,
    kextAllAvailable,
    diskHealth,
    recoveryEndpoint: appleRecovery,
    confidence,
    warnings,
    blockers,
  };
}
