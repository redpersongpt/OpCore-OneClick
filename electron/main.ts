import { app, BrowserWindow, dialog, ipcMain, shell } from 'electron';
import path from 'path';
import crypto from 'node:crypto';
import { fileURLToPath, pathToFileURL } from 'url';
import { exec } from 'child_process';
import util from 'util';
import os from 'os';
import fs from 'fs';
import https from 'https';
import http from 'http';
import net from 'net';
import tls from 'tls';
import { generateConfigPlist, getBIOSSettings, getRequiredResources, getSMBIOSForProfile, type HardwareProfile } from './configGenerator.js';
import { validateEfi, type ValidationResult } from './configValidator.js';
import { checkCompatibility } from './compatibility.js';
import { buildCompatibilityMatrix } from './compatibilityMatrix.js';
import { createEfiBackupManager, type EfiBackupPolicy } from './efiBackup.js';
import { buildResourcePlan, type ResourcePlan } from './resourcePlanner.js';
import {
  createHardwareProfileArtifact,
  extractHardwareProfileInterpretationMetadata,
  parseHardwareProfileArtifact,
  type HardwareProfileArtifact,
  type HardwareProfileInterpretationMetadata,
} from './hardwareProfileArtifact.js';
import { createHardwareProfileStore } from './hardwareProfileStore.js';
import { parseMacOSVersion } from './hackintoshRules.js';
import { buildBiosOrchestratorState } from './bios/orchestrator.js';
import { persistBiosOrchestratorState } from './bios/statePersistence.js';
import { buildHardwareFingerprint, clearBiosSession, loadBiosSession, saveBiosSession, updateBiosSessionStage } from './bios/sessionState.js';
import type { BiosOrchestratorState, BiosSessionState, BiosSettingSelection } from './bios/types.js';
import { createLogger } from './logger.js';
import {
  createClassifiedIpcError,
  type ClassifiedError,
} from './errorMessaging.js';
import {
  buildIssueReportDraft,
  buildSavedSupportLog,
  createDiagnosticsSnapshot,
  openIssueReportUrl,
  type IssueReportTrigger,
  type ReleaseFailureContext,
} from './releaseDiagnostics.js';
import { createDiskOps, getFreeSpaceMB, type DiskInfo } from './diskOps.js';
import {
  buildFlashConfirmationValidationFromSnapshotComparison,
  buildDiskIdentityFingerprint,
  canProceedWithFlash,
  compareFlashAuthorizationSnapshots,
  computeEfiStateHash,
  computeInstallerPayloadHash,
  createFlashAuthorizationSnapshot,
  createFlashAuthorizationSnapshotFromRecord,
  createFlashConfirmationStore,
  findDiskIdentityCollisions,
  resolveFlashPreparationIdentity,
  type DiskIdentityFingerprint,
  type FlashAuthorizationMismatchField,
  type FlashAuthorizationSnapshot,
  type FlashConfirmationRecord,
  validateFlashConfirmation,
  validateFlashConfirmationRecord,
} from './flashSafety.js';
import { createTaskRegistry, type OpToken } from './taskManager.js';
import type { TaskUpdatePayload } from './taskManager.js';
import { detectHardware } from './hardwareDetect.js';
import { interpretHardware, type HardwareInterpretation } from './hardwareInterpret.js';
import { probeFirmware } from './firmwarePreflight.js';
import { runPreflightChecks, recordFailure, getFailureCount, shouldSkipRetry, getFailureMemory, clearFailureMemory, type PreflightReport, type ConfidenceLevel } from './preventionLayer.js';
import { simulateBuild, dryRunRecovery, verifyBuildState, verifyEfiBuildSuccess, verifyRecoverySuccess, type BuildPlan, type RecoveryDryRun, type StateVerification, type SuccessContract, type Certainty } from './deterministicLayer.js';
import { runSafeSimulation, type SafeSimulationResult } from './safeSimulation.js';
import { sim } from './simulation.js';
import { getCompatModeConfigPath, getPackagedRendererEntryPath, getPreloadScriptPath } from './runtimePaths.js';
import { runEfiBuildFlow } from './efiBuildFlow.js';
import { inferLaptopFormFactor } from './formFactor.js';
import { generateFolderManifest, verifyFolderIntegrity } from './resourceIntegrity.js';
import {
  APPLE_RECOVERY_MLB_ZERO,
  buildAppleRecoveryDownloadHeaders,
  queryAppleRecoveryAssets,
} from './appleRecovery.js';
import {
  buildStartupFailurePageUrl,
  determineDidFailLoadAction,
  describeStartupFailure,
  MAX_MAIN_FRAME_LOAD_RETRIES,
  RENDERER_READY_TIMEOUT_MS,
  type StartupFailureEventInput,
} from './startupRecovery.js';
import {
  deriveBiosFlowState,
  deriveReleaseFlowState,
  evaluateBuildGuard,
  evaluateDeployGuard,
  type FlowGuardResult,
} from '../src/lib/stateMachine.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const execPromise = util.promisify(exec);

/**
 * Robust command execution with optional OpToken support for cancellation.
 */
async function runCommand(cmd: string, options: any = {}, token?: OpToken): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = exec(cmd, options, (error, stdout, stderr) => {
      if (error) reject(error);
      else resolve({ stdout: String(stdout), stderr: String(stderr) });
    });
    if (token) token.registerProcess(child);
  });
}

// ── Recovery Source Abstraction ──────────────────────────────────────────────

export type RecoveryTrustLevel = 'official' | 'local_cached_official' | 'manual_user_provided' | 'custom_unverified';
export type RecoverySourceId = 'apple_primary' | 'local_cache' | 'manual_import' | 'custom_source';

export interface RecoveryAssetMetadata {
  version: string;
  size: number;
  sourceId: RecoverySourceId;
  trustLevel: RecoveryTrustLevel;
  timestamp: number;
  dmgPath: string;
  chunklistPath?: string;
  isPartial: boolean;
}

class RecoveryCacheManager {
  private cacheRoot: string;

  constructor(userDataPath: string) {
    this.cacheRoot = path.resolve(userDataPath, 'Recovery_Cache');
    if (!fs.existsSync(this.cacheRoot)) fs.mkdirSync(this.cacheRoot, { recursive: true });
  }

  /**
   * Generates a unique cache key based on version and source.
   * Prevents collisions between official and manual imports.
   */
  getCacheKey(version: string, sourceId: string): string {
    const safeVersion = version.replace(/[^a-zA-Z0-9._-]/g, '_');
    const safeSource = sourceId.replace(/[^a-zA-Z0-9._-]/g, '_');
    return `${safeVersion}_${safeSource}`;
  }

  getVersionDir(version: string, sourceId: string): string {
    const key = this.getCacheKey(version, sourceId);
    const dir = path.join(this.cacheRoot, key);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    return dir;
  }

  getMetadataPath(version: string, sourceId: string): string {
    return path.join(this.getVersionDir(version, sourceId), 'metadata.json');
  }

  async getCachedAsset(version: string): Promise<RecoveryAssetMetadata | null> {
    // Check local cache priority: apple_primary, then manual_import
    const priorities: RecoverySourceId[] = ['apple_primary', 'manual_import'];
    
    for (const sourceId of priorities) {
      const metaPath = this.getMetadataPath(version, sourceId);
      if (!fs.existsSync(metaPath)) continue;

      try {
        const meta: RecoveryAssetMetadata = JSON.parse(fs.readFileSync(metaPath, 'utf8'));
        
        // 1. Verify file exists
        if (!fs.existsSync(meta.dmgPath)) continue;
        
        const stat = fs.statSync(meta.dmgPath);
        const actualSize = stat.size;
        
        // 2. Strict size range validation (macOS recovery images are 350MB - 1GB)
        if (actualSize < 350 * 1024 * 1024 || actualSize > 1024 * 1024 * 1024) {
          log('WARN', 'cache', 'Cached asset failed size validation', { actualSize, version });
          continue; 
        }

        // 3. Metadata consistency check
        if (!meta.isPartial && Math.abs(actualSize - meta.size) > 1024) {
          log('WARN', 'cache', 'Cached asset size mismatch with metadata', { actualSize, expected: meta.size });
          meta.isPartial = true; // Downgrade to partial for safety
        }

        return meta;
      } catch (e) {
        log('ERROR', 'cache', 'Failed to read cache metadata', { error: String(e) });
      }
    }
    return null;
  }

  saveMetadata(meta: RecoveryAssetMetadata) {
    try {
      const metaPath = this.getMetadataPath(meta.version, meta.sourceId);
      fs.writeFileSync(metaPath, JSON.stringify(meta, null, 2));
    } catch (e) {
      log('WARN', 'cache', 'Failed to save cache metadata', { error: String(e) });
    }
  }

  async clearCache(version?: string) {
    try {
      if (version) {
        const safeV = version.replace(/[^a-zA-Z0-9._-]/g, '_');
        const keys = fs.readdirSync(this.cacheRoot).filter(k => k.startsWith(safeV));
        for (const key of keys) {
          fs.rmSync(path.join(this.cacheRoot, key), { recursive: true, force: true });
        }
      } else {
        if (fs.existsSync(this.cacheRoot)) fs.rmSync(this.cacheRoot, { recursive: true, force: true });
        fs.mkdirSync(this.cacheRoot, { recursive: true });
      }
    } catch (e) {
      log('WARN', 'cache', 'Failed to clear cache', { error: String(e) });
    }
  }
}

const cacheManager = new RecoveryCacheManager(app.getPath('userData'));

// ── Download Helpers ──────────────────────────────────────────────

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

async function downloadFileWithProgress(
  url: string,
  dest: string,
  onProgress: (downloaded: number, total: number) => void,
  startOffset = 0,
  checkAborted?: () => void,
  extraHeaders?: Record<string, string>,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const CONNECT_TIMEOUT_MS = 30_000;
    const INACTIVITY_TIMEOUT_MS = 20_000;

    function fetchUrl(urlStr: string, redirects = 0): void {
      if (redirects > 10) { reject(new Error('Too many redirects')); return; }
      const parsedUrl = new URL(urlStr);
      const lib = parsedUrl.protocol === 'https:' ? https : http;
      const headers: Record<string, string | number> = {
        'User-Agent': 'InternetRecovery/1.0',
        ...(extraHeaders ?? {}),
      };
      if (startOffset > 0) {
        headers['Range'] = `bytes=${startOffset}-`;
      }
      const options = {
        hostname: parsedUrl.hostname,
        path: parsedUrl.pathname + parsedUrl.search,
        headers
      };
      const req = lib.get(options as any, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302 || res.statusCode === 307 || res.statusCode === 308) {
          clearTimeout(connectTimer);
          fetchUrl(res.headers.location!, redirects + 1);
          return;
        }
        const isResume = res.statusCode === 206;
        const isFull   = res.statusCode === 200;
        if (!isResume && !isFull) {
          clearTimeout(connectTimer);
          reject(new Error(`HTTP ${res.statusCode} downloading ${urlStr}`));
          return;
        }
        const effectiveOffset = isResume ? startOffset : 0;
        const contentLength = parseInt(res.headers['content-length'] || '0', 10);
        const total = contentLength + effectiveOffset;
        let downloaded = effectiveOffset;
        let rejected = false;
        const file = fs.createWriteStream(dest, isResume ? { flags: 'a' } : { flags: 'w' });
        let inactivityTimer: NodeJS.Timeout | null = null;

        const clearInactivityTimer = () => {
          if (inactivityTimer) {
            clearTimeout(inactivityTimer);
            inactivityTimer = null;
          }
        };

        const armInactivityTimer = () => {
          clearInactivityTimer();
          inactivityTimer = setTimeout(() => {
            if (rejected) return;
            rejected = true;
            res.destroy(new Error(`Download stalled after ${INACTIVITY_TIMEOUT_MS / 1000}s with no progress: ${urlStr}`));
            file.destroy(new Error(`Download stalled after ${INACTIVITY_TIMEOUT_MS / 1000}s with no progress: ${urlStr}`));
            reject(new Error(`Download stalled after ${INACTIVITY_TIMEOUT_MS / 1000}s with no progress: ${urlStr}`));
          }, INACTIVITY_TIMEOUT_MS);
        };

        clearTimeout(connectTimer);
        armInactivityTimer();
        res.on('data', (chunk: Buffer) => {
          if (rejected) return;
          try { checkAborted?.(); } catch (abortErr) {
            rejected = true;
            clearInactivityTimer();
            res.destroy();
            file.destroy();
            try { fs.truncateSync(dest, downloaded); } catch {}
            reject(abortErr);
            return;
          }
          downloaded += chunk.length;
          armInactivityTimer();
          onProgress(downloaded, total);
        });
        res.pipe(file);
        file.on('finish', () => {
          if (rejected) return;
          clearInactivityTimer();
          file.close(() => resolve());
        });
        res.on('error', (e) => {
          clearInactivityTimer();
          if (!rejected) { rejected = true; reject(e); }
        });
        file.on('error', (e) => {
          clearInactivityTimer();
          if (!rejected) { rejected = true; reject(e); }
        });
      });
      const connectTimer = setTimeout(() => {
        req.destroy(new Error(`Timed out after ${CONNECT_TIMEOUT_MS / 1000}s connecting to ${urlStr}`));
      }, CONNECT_TIMEOUT_MS);
      req.on('error', (error) => {
        clearTimeout(connectTimer);
        reject(error);
      });
    }
    fetchUrl(url);
  });
}

// Logger is created after app is ready — see app.whenReady() block
let logger: ReturnType<typeof createLogger>;
// Track diagnostics state for snapshot
let lastHardwareProfile: any = null;
let lastScannedProfile: HardwareProfile | null = null;
let lastBuildProfile: HardwareProfile | null = null;
let lastHardwareInterpretation: HardwareInterpretation | null = null;
let lastLiveHardwareProfileArtifact: HardwareProfileArtifact | null = null;
let lastSelectedDisk: DiskInfo | null = null;
let failedKexts: Array<{ name: string; repo: string; error: string }> = [];
let kextSources: Record<string, 'github' | 'embedded' | 'failed'> = {};
let lastValidationResult: ValidationResult | null = null;
let lastFailureContext: ReleaseFailureContext | null = null;
let lastFirmwareSummary: string | null = null;
let recoveryStats = { attempts: 0, lastError: null as string | null, lastHttpCode: null as number | null, finalDecision: null as string | null };
let diskOps: ReturnType<typeof createDiskOps>;
let registry!: ReturnType<typeof createTaskRegistry>;
let flashConfirmationStore!: ReturnType<typeof createFlashConfirmationStore>;
let hardwareProfileStore!: ReturnType<typeof createHardwareProfileStore>;
let efiBackupManager!: ReturnType<typeof createEfiBackupManager>;

// ── Embedded Kext Fallback ───────────────────────────────────────────────────

function getEmbeddedKextsDir(): string {
  // Packaged app: extraResources → resources/kexts
  const prodPath = path.join(process.resourcesPath ?? '', 'kexts');
  if (fs.existsSync(prodPath)) return prodPath;
  // Dev: electron/assets/kexts relative to project root
  const devPath = path.join(__dirname, '..', 'electron', 'assets', 'kexts');
  if (fs.existsSync(devPath)) return devPath;
  return devPath;
}

function hasEmbeddedKext(kextName: string): boolean {
  const kextPath = path.join(getEmbeddedKextsDir(), kextName);
  return fs.existsSync(kextPath) && fs.existsSync(path.join(kextPath, 'Contents', 'MacOS'));
}

function installEmbeddedKext(kextName: string, targetDir: string): { name: string; version: string } {
  const kextsDir = getEmbeddedKextsDir();
  const src = path.join(kextsDir, kextName);
  const dest = path.join(targetDir, kextName);
  if (fs.existsSync(dest)) fs.rmSync(dest, { recursive: true, force: true });
  copyDirSync(src, dest);
  // Read version from companion .version file (outside bundle to avoid codesign issues)
  const baseName = kextName.replace(/\.kext$/, '');
  const versionFile = path.join(kextsDir, `${baseName}.version`);
  const version = fs.existsSync(versionFile) ? fs.readFileSync(versionFile, 'utf-8').trim() : 'embedded';
  fs.writeFileSync(path.join(dest, '.version'), `${version}-embedded`);
  return { name: kextName, version: `${version}-embedded` };
}

function validateInstalledKext(kextName: string, targetDir: string): boolean {
  const kextPath = path.join(targetDir, kextName);
  if (!fs.existsSync(kextPath)) return false;
  if (!fs.existsSync(path.join(kextPath, 'Contents', 'MacOS'))) return false;
  // Size sanity: Contents/MacOS should have at least one file > 1KB
  try {
    const macosDir = path.join(kextPath, 'Contents', 'MacOS');
    const files = fs.readdirSync(macosDir);
    return files.some(f => fs.statSync(path.join(macosDir, f)).size > 1024);
  } catch { return false; }
}

// Compat shim — used by existing log() call sites until migrated

function log(level: string, ctx: string, msg: string, data?: Record<string, unknown>) {
  if (!logger) return;
  const fn = (logger as unknown as Record<string, unknown>)[level.toLowerCase()];
  if (typeof fn === 'function') (fn as (c: string, m: string, d?: Record<string, unknown>) => void)(ctx, msg, data);
}

function inferFailureTriggerFromChannel(channel: string): IssueReportTrigger {
  switch (channel) {
    case 'build-efi':
      return 'efi_build_failure';
    case 'validate-efi':
      return 'efi_validation_failure';
    case 'download-recovery':
    case 'recovery:import':
      return 'recovery_failure';
    case 'safe-simulation:run':
      return 'simulation_failure';
    case 'list-usb-devices':
    case 'get-hard-drives':
    case 'get-disk-info':
      return 'disk_read_failure';
    default:
      return 'ipc_failure';
  }
}

function rememberFailureContext(context: Omit<ReleaseFailureContext, 'occurredAt'> & { occurredAt?: string }): void {
  lastFailureContext = {
    ...context,
    occurredAt: context.occurredAt ?? new Date().toISOString(),
  };
}

function getCurrentCompatibilityReport(): ReturnType<typeof checkCompatibility> | null {
  const profile = getCurrentBuildProfile();
  return profile ? checkCompatibility(profile) : null;
}

// Crash-safe ipcMain.handle wrapper.
// Every handler is wrapped in try/catch — unhandled throws are logged and
// re-thrown so the renderer receives a proper IPC rejection instead of a
// silent hang or unhandled-rejection crash in the main process.
function ipcHandle(
  channel: string,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  handler: (event: Electron.IpcMainInvokeEvent, ...args: any[]) => Promise<any> | any,
): void {
  ipcMain.handle(channel, async (event, ...args) => {
    try {
      return await handler(event, ...args);
    } catch (err: any) {
      rememberFailureContext({
        trigger: inferFailureTriggerFromChannel(channel),
        message: err?.message ?? `IPC handler '${channel}' failed`,
        detail: err?.stack ?? null,
        channel,
        code: err?.code ?? null,
      });
      log('ERROR', 'ipc', `Handler '${channel}' threw`, {
        error: err?.message,
        code: err?.code,
      });
      if (logger) logger.flush();
      const sanitized = new Error(err?.message ?? `IPC handler '${channel}' failed`);
      if (err?.code) {
        Object.assign(sanitized, { code: err.code });
      }
      Object.defineProperty(sanitized, 'stack', {
        value: undefined,
        configurable: true,
        enumerable: false,
        writable: true,
      });
      throw sanitized;
    }
  });
}

// ── Startup crash guards ─────────────────────────────────────────────────────
const EARLY_CRASH_FILE = path.join(app.getPath('userData'), 'startup-crash.log');
function writeEarlyCrash(tag: string, err: unknown) {
  const line = `[${new Date().toISOString()}] ${tag}: ${(err as Error)?.stack ?? String(err)}\n`;
  try { fs.appendFileSync(EARLY_CRASH_FILE, line); } catch {}
  if (logger) logger.fatal('startup', tag, { error: String(err) });
}
process.on('uncaughtException',   (err) => writeEarlyCrash('uncaughtException', err));
process.on('unhandledRejection',  (err) => writeEarlyCrash('unhandledRejection', err));

// ── Compatibility mode (read before app.ready — must apply flags here) ────────
// CI writes dist-electron/compat.json before packaging to select the mode.
// Modes: "none" | "gpu-disabled" | "legacy"
type CompatMode = 'none' | 'gpu-disabled' | 'legacy';
let compatMode: CompatMode = 'none';
try {
  const compatFile = getCompatModeConfigPath(__dirname);
  const parsed = JSON.parse(fs.readFileSync(compatFile, 'utf8'));
  compatMode = parsed.mode ?? 'none';
} catch { /* no compat.json — standard build */ }

if (compatMode === 'gpu-disabled' || compatMode === 'legacy') {
  // Must be called before app.whenReady()
  app.disableHardwareAcceleration();
}

// ── Linux Sandbox / Root Hardening ───────────────────────────────────────────
if (process.platform === 'linux') {
  const isRoot = process.getuid?.() === 0;
  if (isRoot) {
    // Electron/Chromium sandbox is strictly incompatible with root on Linux.
    // Automatically apply --no-sandbox only when necessary to ensure the app opens.
    app.commandLine.appendSwitch('no-sandbox');
    app.commandLine.appendSwitch('disable-setuid-sandbox');
  }
}

if (compatMode === 'legacy') {
  // Maximum compatibility for Windows Server 2016 / headless / no-GPU environments.
  // Chromium 130 (Electron 41) tries D3D11/DXGI 1.4 which is absent on Server 2016.
  app.commandLine.appendSwitch('disable-gpu');
  app.commandLine.appendSwitch('disable-gpu-compositing');
  app.commandLine.appendSwitch('disable-gpu-sandbox');
  app.commandLine.appendSwitch('disable-d3d11');
  app.commandLine.appendSwitch('disable-software-rasterizer');
  app.commandLine.appendSwitch('disable-features', 'VizDisplayCompositor,CalculateNativeWinOcclusion,WinUseBrowserSpellChecker');
  app.commandLine.appendSwitch('enable-features', 'MetalANGLE');
  app.commandLine.appendSwitch('use-angle', 'swiftshader');
  app.commandLine.appendSwitch('use-gl', 'swiftshader');
}

// Race any promise against a timeout
function withTimeout<T>(p: Promise<T>, ms: number, label: string): Promise<T> {
  return Promise.race([
    p,
    new Promise<never>((_, rej) => setTimeout(() => rej(new Error(`Timed out after ${ms / 1000}s: ${label}`)), ms))
  ]);
}

// Classify OS error codes into structured user-friendly messages
function classifyError(e: unknown): ClassifiedError {
  const err = e as NodeJS.ErrnoException;
  const message = err.message ?? String(e);

  // 1. Hardware Errors
  if (message.includes('lost') || message.includes('disconnected') || message.includes('rejected write block')) {
    return {
      category: 'hardware_error',
      message: 'USB device stopped responding',
      explanation: 'The target drive was disconnected or encountered a hardware-level failure during the operation.',
      suggestion: 'Try a different USB port (avoid hubs) or use a higher-quality USB drive.'
    };
  }

  // 2. Environment Errors (Permissions, Network, Space)
  if (err.code === 'EACCES' || err.code === 'EPERM' || message.includes('Permission denied')) {
    return {
      category: 'environment_error',
      message: 'Permission denied',
      explanation: 'The application lacks the necessary system privileges to write directly to the hardware.',
      suggestion: `Run the application ${process.platform === 'win32' ? 'as Administrator' : 'with sudo'}.`
    };
  }

  if (err.code === 'ENOSPC' || message.includes('disk space')) {
    return {
      category: 'environment_error',
      message: 'Not enough disk space',
      explanation: 'There is insufficient free space on your system drive or target drive to complete the operation.',
      suggestion: 'Free up some space and try again.'
    };
  }

  if (message.includes('APPLE_AUTH_REJECT') || message.includes('Apple rejected') || message.includes('401') || message.includes('403')) {
    return {
      category: 'environment_error',
      message: 'Apple recovery server rejected the request',
      explanation: 'Apple\'s recovery servers refused the download request. This is an external service limitation, not a problem with your machine or network.',
      suggestion: 'Use a cached recovery image, import one manually, or try a different macOS version.'
    };
  }

  if (message.includes('APPLE_RATE_LIMIT') || message.includes('rate-limited')) {
    return {
      category: 'environment_error',
      message: 'Apple rate limit',
      explanation: 'Apple\'s servers are temporarily limiting requests. This usually resolves within a few minutes.',
      suggestion: 'Wait a few minutes and retry, or import a recovery image manually.'
    };
  }

  // 3. App Errors / Timeouts
  if (err.code === 'ETIMEDOUT' || message.includes('Timed out')) {
    return {
      category: 'app_error',
      message: 'Operation timed out',
      explanation: 'The system or network took too long to respond to a critical request.',
      suggestion: 'Check your internet connection and try again.'
    };
  }

  return {
    category: 'app_error',
    message: 'Operation failed',
    explanation: message,
    suggestion: 'Restart the application and try again. If the issue persists, report it via the debug panel.'
  };
}

// Retry a fn up to maxAttempts times with exponential backoff + jitter
async function retryWithBackoff<T>(
  fn: () => Promise<T>,
  maxAttempts: number,
  label: string,
  checkAborted?: () => void
): Promise<T> {
  let lastErr: unknown;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    checkAborted?.();
    try { return await fn(); }
    catch (e: any) {
      lastErr = e;
      const fatal = e.message?.includes('cancelled') || e.message?.includes('rate limit') ||
                    e.message?.includes('SAFETY BLOCK') || e.message?.includes('corrupt');
      if (fatal || attempt === maxAttempts - 1) throw e;
      const delay = Math.min(1500 * Math.pow(2, attempt), 30000) + Math.random() * 500;
      log('WARN', 'retry', `${label} attempt ${attempt + 1}/${maxAttempts} failed — retrying in ${Math.round(delay / 1000)}s`, { error: e.message });
      await new Promise(r => setTimeout(r, delay));
    }
  }
  throw lastErr;
}

// Parse proxy from env vars
function parseProxy(): { host: string; port: number } | null {
  const raw = process.env.HTTPS_PROXY || process.env.https_proxy || process.env.HTTP_PROXY || process.env.http_proxy;
  if (!raw) return null;
  try {
    const u = new URL(raw.startsWith('http') ? raw : 'http://' + raw);
    return { host: u.hostname, port: parseInt(u.port) || 8080 };
  } catch { return null; }
}

// --- State Persistence ---

const STATE_FILE = path.join(app.getPath('userData'), 'app_state.json');

interface AppState {
  currentStep: string;
  profile: HardwareProfile | null;
  timestamp: number;
  planningProfileContext?: 'live_scan' | 'imported_artifact' | 'saved_artifact' | null;
  profileArtifactDigest?: string;
  // Download resume state
  recoveryDownloadOffset?: number;   // bytes already downloaded
  recoveryDmgDest?: string;          // absolute path to BaseSystem.dmg
  recoveryClDest?: string;           // absolute path to BaseSystem.chunklist
  efiPath?: string;                  // EFI build directory
  recoveryTargetOS?: string;         // macOS version being downloaded
}

function saveState(state: AppState | null) {
  try {
    if (state == null) {
      if (fs.existsSync(STATE_FILE)) fs.unlinkSync(STATE_FILE);
      return;
    }
    fs.writeFileSync(STATE_FILE, JSON.stringify(state));
  } catch (e) {
    log('WARN', 'state', 'Failed to persist state', { error: String(e) });
  }
}

function loadState(): AppState | null {
  if (fs.existsSync(STATE_FILE)) {
    try {
      return JSON.parse(fs.readFileSync(STATE_FILE, 'utf-8'));
    } catch (e) {
      return null;
    }
  }
  return null;
}

async function restartIntoFirmware(): Promise<{ supported: boolean; error?: string }> {
  if (process.platform !== 'win32') {
    return { supported: false };
  }
  try {
    await execPromise('bcdedit /set {fwbootmgr} bootsequence {fwbootmgr} /addfirst');
  } catch (e) {
    try {
      await execPromise('shutdown /r /fw /t 0');
      app.setLoginItemSettings({ openAtLogin: true, path: app.getPath('exe'), args: ['--resuming'] });
      return { supported: true };
    } catch (e2: any) {
      return { supported: false, error: (e2 as Error).message };
    }
  }

  app.setLoginItemSettings({ openAtLogin: true, path: app.getPath('exe'), args: ['--resuming'] });
  exec('shutdown /r /t 3', (err) => { if (err) log('ERROR', 'system', `BIOS restart failed: ${err.message}`); });
  return { supported: true };
}

// --- BIOS Probing ---
interface BIOSStatus {
  secureBootDisabled: boolean | 'unknown';
  virtualizationEnabled: boolean | 'unknown';
}

async function probeBiosSettings(): Promise<BIOSStatus> {
  const status: BIOSStatus = {
    secureBootDisabled: 'unknown',
    virtualizationEnabled: 'unknown'
  };

  if (process.platform === 'win32') {
    try {
      const sb = await execPromise('powershell -NoProfile -Command "Confirm-SecureBootUEFI"');
      status.secureBootDisabled = sb.stdout.trim().toLowerCase() === 'false';
    } catch (e) {}

    try {
      const vt = await execPromise('powershell -NoProfile -Command "(Get-CimInstance Win32_Processor).VirtualizationFirmwareEnabled"');
      status.virtualizationEnabled = vt.stdout.trim().toLowerCase() === 'true';
    } catch (e) {}
  } else if (process.platform === 'linux') {
    try {
      const sb = await execPromise('bootctl status 2>/dev/null | grep "Secure Boot"');
      status.secureBootDisabled = sb.stdout.toLowerCase().includes('disabled');
    } catch (e) {}
    
    try {
      const vt = await execPromise('lscpu | grep Virtualization');
      status.virtualizationEnabled = vt.stdout.includes('VT-x') || vt.stdout.includes('AMD-V');
    } catch (e) {}
  } else if (process.platform === 'darwin') {
    // macOS always has virtualization and no Secure Boot concern
    status.secureBootDisabled = true;
    status.virtualizationEnabled = true;
  }

  return status;
}

// --- EFI Structure ---

async function ensureOpenCoreBinaries(
  basePath: string,
  token?: OpToken,
  onPhase?: (phase: string, detail: string) => void,
) {
  const cacheDir = path.resolve(app.getPath('userData'), 'OpenCore_Cache');
  const ocVersion = '1.0.3';
  const ocUrl = `https://github.com/acidanthera/OpenCorePkg/releases/download/${ocVersion}/OpenCore-${ocVersion}-RELEASE.zip`;
  const ocZip = path.resolve(cacheDir, `OpenCore-${ocVersion}.zip`);
  const ocExtracted = path.resolve(cacheDir, ocVersion);
  const x64Efi = path.resolve(ocExtracted, 'X64/EFI');
  const coreFile = path.resolve(x64Efi, 'OC/OpenCore.efi');
  const manifestPath = path.resolve(ocExtracted, 'x64-efi-manifest.json');

  if (!fs.existsSync(cacheDir)) fs.mkdirSync(cacheDir, { recursive: true });

  if (fs.existsSync(coreFile)) {
    const integrity = verifyFolderIntegrity(x64Efi, manifestPath);
    if (!integrity.valid) {
      log('WARN', 'efi', 'OpenCore cache integrity check failed; rebuilding cache', {
        version: ocVersion,
        issues: integrity.issues,
      });
      try { fs.rmSync(ocExtracted, { recursive: true, force: true }); } catch {}
    }
  }

  if (!fs.existsSync(coreFile)) {
    const extractCachedZip = async () => {
      if (fs.existsSync(ocExtracted)) fs.rmSync(ocExtracted, { recursive: true, force: true });
      fs.mkdirSync(ocExtracted, { recursive: true });
      onPhase?.('Extracting OpenCore base files…', 'Preparing bootloader files for config generation.');
      if (process.platform === 'win32') {
        await runCommand(`powershell -Command "Expand-Archive -Path '${ocZip}' -DestinationPath '${ocExtracted}' -Force"`, {}, token);
      } else {
        await runCommand(`unzip -o "${ocZip}" -d "${ocExtracted}"`, {}, token);
      }
    };

    try {
      if (fs.existsSync(ocZip) && fs.statSync(ocZip).size > 0) {
        log('INFO', 'efi', 'Reusing cached OpenCore archive', { version: ocVersion, ocZip });
        await extractCachedZip();
      }
      if (!fs.existsSync(coreFile)) {
        log('INFO', 'efi', 'Downloading base OpenCore binaries...', { version: ocVersion });
        onPhase?.('Downloading OpenCore binaries…', `Caching OpenCore ${ocVersion} for the EFI build.`);
        await downloadFileWithProgress(ocUrl, ocZip, (downloaded, total) => {
          const detail = total > 0
            ? `${formatBytes(downloaded)} of ${formatBytes(total)}`
            : `${formatBytes(downloaded)} downloaded`;
          onPhase?.('Downloading OpenCore binaries…', detail);
        }, 0, () => token?.check());
        await extractCachedZip();
      }
    } catch (err) {
      throw new Error(`Failed to extract OpenCore binaries: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  if (!fs.existsSync(coreFile)) {
    throw new Error('OpenCore binaries could not be located after extraction. The download may be corrupt.');
  }

  if (fs.existsSync(x64Efi)) {
    generateFolderManifest(x64Efi, manifestPath);
    onPhase?.('Copying OpenCore base files…', 'Moving the base EFI structure into the build workspace.');
    copyDirSync(x64Efi, path.resolve(basePath, 'EFI'));
    const versionedFiles = [
      path.resolve(basePath, 'EFI/OC/OpenCore.efi'),
      path.resolve(basePath, 'EFI/BOOT/BOOTx64.efi'),
      path.resolve(basePath, 'EFI/OC/Drivers/OpenRuntime.efi'),
      path.resolve(basePath, 'EFI/OC/Drivers/OpenHfsPlus.efi'),
    ];
    for (const file of versionedFiles) {
      if (fs.existsSync(file)) {
        fs.writeFileSync(`${file}.version`, ocVersion);
      }
    }
  } else {
    throw new Error('Standard OpenCore EFI structure not found in extracted package.');
  }
}

async function createEfiStructure(
  basePath: string,
  profile: HardwareProfile,
  token?: OpToken,
  onPhase?: (phase: string, detail: string) => void,
) {
  await ensureOpenCoreBinaries(basePath, token, onPhase);

  const { kexts, ssdts } = getRequiredResources(profile);
  const dirs = [
    'EFI/BOOT',
    'EFI/OC/ACPI',
    'EFI/OC/Drivers',
    'EFI/OC/Kexts',
    'EFI/OC/Resources/Audio',
    'EFI/OC/Resources/Font',
    'EFI/OC/Resources/Image',
    'EFI/OC/Resources/Label',
    'EFI/OC/Tools'
  ];
  for (const dir of dirs) {
    const fullPath = path.resolve(basePath, dir);
    if (!fs.existsSync(fullPath)) fs.mkdirSync(fullPath, { recursive: true });
  }
  onPhase?.('Writing OpenCore configuration…', `Generating config.plist for ${profile.smbios}.`);
  const configContent = generateConfigPlist(profile);
  fs.writeFileSync(path.resolve(basePath, 'EFI/OC/config.plist'), configContent);
  onPhase?.('Preparing ACPI and kext placeholders…', `${ssdts.length} SSDT entries and ${kexts.length} kext folders queued.`);
  ssdts.forEach(s => {
    const p = path.resolve(basePath, 'EFI/OC/ACPI', s);
    if (!fs.existsSync(p)) fs.writeFileSync(p, '');
  });
  kexts.forEach(k => {
    const kextDir = path.resolve(basePath, 'EFI/OC/Kexts', k);
    if (!fs.existsSync(kextDir)) fs.mkdirSync(kextDir, { recursive: true });
  });
}

// --- CPU Generation Detection ---

function detectCpuGeneration(cpuModel: string): HardwareProfile['generation'] {
  const model = cpuModel.toLowerCase();
  if (model.includes('apple') || model.includes('m1') || model.includes('m2') || model.includes('m3') || model.includes('m4')) return 'Apple Silicon';
  
  // High-End Desktop / Servers
  if (model.includes('xeon')) {
    if (model.includes('w-') || model.includes('scalable')) return 'Cascade Lake-X'; // Approximation for modern Xeons
    if (model.includes('e5-v4') || model.includes('e5-v3')) return 'Broadwell-E';
    if (model.includes('e5-v2')) return 'Ivy Bridge-E';
    return 'Haswell-E'; // Safe middle ground for older Xeons
  }

  // Standard Core i series
  const match = model.match(/i\d-? ?(1?\d{4})/);
  if (match) {
    const num = parseInt(match[1]);
    if (num >= 10000 && num < 11000 && (/\bg[147]\b/.test(model) || model.includes('ice lake'))) return 'Ice Lake';
    if (num >= 14000) return 'Raptor Lake';
    if (num >= 13000) return 'Raptor Lake';
    if (num >= 12000) return 'Alder Lake';
    if (num >= 11000) return 'Rocket Lake';
    if (num >= 10000) return 'Comet Lake';
    if (num >= 8000) return 'Coffee Lake';
    if (num >= 7000) return 'Kaby Lake';
    if (num >= 6000) return 'Skylake';
    if (num >= 5000) return 'Broadwell';
    if (num >= 4000) return 'Haswell';
    if (num >= 3000) return 'Ivy Bridge';
    if (num >= 2000) return 'Sandy Bridge';
  }

  const legacyCoreMatch = model.match(/i[357]-?\s*(\d{3,4})([a-z]{0,2})/);
  if (legacyCoreMatch) {
    const num = parseInt(legacyCoreMatch[1], 10);
    const suffix = legacyCoreMatch[2] ?? '';
    if (num >= 900 && num < 1000) return 'Nehalem';
    if (num >= 800 && num < 900) return 'Nehalem';
    if (num >= 700 && num < 800) return 'Westmere';
    if (num >= 600 && num < 700) return 'Clarkdale';
    if (num >= 400 && num < 600) {
      if (/[lmqu]/.test(suffix)) return 'Arrandale';
      return 'Clarkdale';
    }
  }

  // Budget Intel Desktop
  if (model.includes('pentium') || model.includes('celeron')) {
    if (model.includes('gold')) return 'Coffee Lake'; 
    if (model.match(/g[45]\d{2}/)) return 'Skylake';
    if (model.match(/g3\d{2}/)) return 'Haswell';
    if (model.match(/g[2|1]\d{2}/) || model.match(/g[68]\d0/)) return 'Sandy Bridge';
    return 'Ivy Bridge';
  }
  
  // Legacy Intel Desktop
  if (model.includes('core 2 quad') || /\bq9\d{3}\b/.test(model)) return 'Yorkfield';
  if (model.includes('core 2 duo') || /\be8\d{3}\b/.test(model) || /\be7\d{3}\b/.test(model)) return 'Wolfdale';
  if (model.includes('core 2') || model.includes('quad') || model.includes('extreme')) return 'Penryn';

  // AMD Desktop
  if (model.includes('threadripper')) return 'Threadripper';
  if (model.includes('ryzen')) return 'Ryzen';
  if (model.includes('fx-') || model.includes('phenom') || model.includes('athlon')) return 'Bulldozer'; // Legacy AMD
  return 'Unknown';
}

function detectArchitecture(cpuModel: string): HardwareProfile['architecture'] {
  const model = cpuModel.toLowerCase();
  if (model.includes('apple') || model.includes('m1') || model.includes('m2') || model.includes('m3') || model.includes('m4')) return 'Apple Silicon';
  if (model.includes('ryzen') || model.includes('threadripper') || model.includes('amd')) return 'AMD';
  if (model.includes('intel') || model.match(/i\d-/)) return 'Intel';
  return 'Unknown';
}

// --- Hardware Detection per Platform ---

async function getWindowsHardwareInfo(): Promise<HardwareProfile> {
  const psCommand = (cmd: string) => `powershell -NoProfile -Command "${cmd}"`;
  const runWindowsProbe = (cmd: string, fallback = '') =>
    execPromise(psCommand(cmd), {
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRaw, gpuRaw, baseboardRaw, coresRaw, chassisRaw, manufRaw, modelRaw, batteryRaw] = await Promise.all([
    runWindowsProbe("Get-CimInstance CIM_Processor | Select-Object -ExpandProperty Name", 'Unknown CPU'),
    runWindowsProbe("Get-CimInstance CIM_VideoController | Select-Object -ExpandProperty Name", 'Unknown GPU'),
    runWindowsProbe("Get-CimInstance CIM_BaseBoard | Select-Object -ExpandProperty Product", 'Unknown Board'),
    runWindowsProbe("Get-CimInstance CIM_Processor | Select-Object -ExpandProperty NumberOfCores", '4'),
    runWindowsProbe("Get-CimInstance CIM_SystemEnclosure | Select-Object -ExpandProperty ChassisTypes", '3'),
    runWindowsProbe("Get-CimInstance CIM_ComputerSystem | Select-Object -ExpandProperty Manufacturer", 'Unknown'),
    runWindowsProbe("Get-CimInstance CIM_ComputerSystem | Select-Object -ExpandProperty Model"),
    runWindowsProbe("Get-CimInstance Win32_Battery | Select-Object -First 1 | ConvertTo-Json -Compress"),
  ]);

  const cpuModel = cpuRaw.stdout.trim().split('\n')[0];
  const gpuModel = gpuRaw.stdout.trim().split('\n').join(' / ');
  const motherboard = baseboardRaw.stdout.trim().split('\n')[0] || 'Unknown';
  const coreCount = parseInt(coresRaw.stdout.trim()) || 4;
  
  const chassisTypes = chassisRaw.stdout.trim().split('\n').map(c => parseInt(c.trim()));
  const isLaptop = inferLaptopFormFactor({
    cpuName: cpuModel,
    chassisTypes,
    modelName: modelRaw.stdout.trim(),
    batteryPresent: batteryRaw.stdout.trim().length > 0 && batteryRaw.stdout.trim() !== 'null',
  });

  const manuf = manufRaw.stdout.trim().toLowerCase();
  const isVM = manuf.includes('vmware') || manuf.includes('qemu') || manuf.includes('innotek') || manuf.includes('microsoft corporation') || manuf.includes('parallels');

  const generation = detectCpuGeneration(cpuModel);
  const architecture = detectArchitecture(cpuModel);

  const profile: HardwareProfile = {
    cpu: cpuModel,
    architecture,
    generation,
    coreCount,
    gpu: gpuModel,
    ram: (os.totalmem() / 1024 / 1024 / 1024).toFixed(0) + " GB",
    motherboard,
    targetOS: 'macOS Sequoia 15.x',
    smbios: '',
    kexts: [], ssdts: [], 
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop,
    isVM,
    audioLayoutId: 1
  };
  profile.smbios = getSMBIOSForProfile(profile);
  return profile;
}

async function getLinuxHardwareInfo(): Promise<HardwareProfile> {
  const [cpuRaw, gpuRaw, baseboardRaw, memRaw, chassisRaw, vendorRaw, batteryRaw] = await Promise.all([
    execPromise('lscpu'),
    execPromise('lspci | grep -i vga'),
    execPromise('cat /sys/class/dmi/id/board_name 2>/dev/null || cat /sys/class/dmi/id/product_name 2>/dev/null'),
    execPromise('free -b | grep Mem'),
    execPromise('cat /sys/class/dmi/id/chassis_type 2>/dev/null'),
    execPromise('cat /sys/class/dmi/id/sys_vendor 2>/dev/null'),
    execPromise('ls /sys/class/power_supply 2>/dev/null | grep -E "^BAT"')
  ]).catch(() => [{stdout: ''}, {stdout: ''}, {stdout: 'Unknown'}, {stdout: '0 0'}, {stdout: '3'}, {stdout: 'Unknown'}, {stdout: ''}]);

  const cpuLines = cpuRaw.stdout.split('\n');
  const cpuModel = (cpuLines.find(l => l.includes('Model name:')) || '').split(':')[1]?.trim() || 'Unknown CPU';
  const gpuModel = gpuRaw.stdout.split('\n').filter(Boolean).map(l => l.split(':')[2]?.trim()).join(' / ') || 'Generic GPU';
  const memTotal = parseInt(memRaw.stdout.split(/\s+/)[1]) || 4294967296; // Fallback 4GB
  
  const chassisType = parseInt(chassisRaw.stdout.trim());
  const isLaptop = inferLaptopFormFactor({
    cpuName: cpuModel,
    chassisTypes: Number.isFinite(chassisType) && chassisType > 0 ? [chassisType] : [],
    modelName: baseboardRaw.stdout.trim(),
    batteryPresent: batteryRaw.stdout.trim().length > 0,
  });

  const vendor = vendorRaw.stdout.trim().toLowerCase();
  const isVM = vendor.includes('vmware') || vendor.includes('qemu') || vendor.includes('innotek') || vendor.includes('microsoft') || vendor.includes('parallels');

  const generation = detectCpuGeneration(cpuModel);
  const architecture = detectArchitecture(cpuModel);

  const profile: HardwareProfile = {
    cpu: cpuModel, architecture,
    generation, coreCount: os.cpus().length, gpu: gpuModel,
    ram: (memTotal / 1024 / 1024 / 1024).toFixed(0) + " GB",
    motherboard: baseboardRaw.stdout.trim(),
    targetOS: 'macOS Sequoia 15.x', smbios: '',
    kexts: [], ssdts: [], bootArgs: '-v keepsyms=1 debug=0x100', isLaptop, isVM,
    audioLayoutId: 1
  };
  profile.smbios = getSMBIOSForProfile(profile);
  return profile;
}

async function getMacHardwareInfo(): Promise<HardwareProfile> {
  const [cpuRaw, gpuRaw, memRaw, modelRaw, hwModelRaw] = await Promise.all([
    execPromise('sysctl -n machdep.cpu.brand_string'),
    execPromise('system_profiler SPDisplaysDataType 2>/dev/null | grep "Chipset Model" | awk -F": " \'{print $2}\''),
    execPromise('sysctl -n hw.memsize'),
    execPromise('system_profiler SPHardwareDataType 2>/dev/null | grep "Model Identifier" | cut -d: -f2'),
    execPromise('sysctl -n hw.model 2>/dev/null')
  ]).catch(() => [{stdout: 'Unknown Mac CPU'}, {stdout: 'Unknown Mac GPU'}, {stdout: '8589934592'}, {stdout: 'Mac'}, {stdout: 'Unknown'}]);
  
  const boardName = modelRaw.stdout.trim() || 'Unknown Mac';
  const cpuModel = cpuRaw.stdout.trim();
  const gpuModel = gpuRaw.stdout.trim().split('\n').join(' / ') || 'Unknown GPU';
  const memBytes = parseInt(memRaw.stdout.trim()) || 8589934592;
  const generation = detectCpuGeneration(cpuModel);
  const architecture = detectArchitecture(cpuModel);
  const isLaptop = boardName.toLowerCase().includes('book');

  const hwModel = hwModelRaw.stdout.trim().toLowerCase();
  const isVM = hwModel.includes('vmware') || hwModel.includes('virtualbox') || hwModel.includes('parallels');

  const profile: HardwareProfile = {
    cpu: cpuModel,
    architecture,
    generation,
    coreCount: os.cpus().length,
    gpu: gpuModel,
    ram: (memBytes / 1024 / 1024 / 1024).toFixed(0) + " GB",
    motherboard: boardName,
    targetOS: 'macOS Sequoia 15.x',
    smbios: '',
    kexts: [], ssdts: [],
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop,
    isVM,
    audioLayoutId: 1
  };
  profile.smbios = getSMBIOSForProfile(profile);
  return profile;
}

// ── Map DetectedHardware → legacy HardwareProfile ────────────────────────────

function mapDetectedToProfile(hw: import('./hardwareDetect.js').DetectedHardware): HardwareProfile {
  const cpuModel = hw.cpu.name;
  const gpuModel = hw.gpus.map(g => g.name).join(' / ') || 'Unknown GPU';
  const generation = detectCpuGeneration(cpuModel);
  const architecture = detectArchitecture(cpuModel);
  const ramGB = (hw.ramBytes / 1024 / 1024 / 1024).toFixed(0) + ' GB';

  // Derive overall scan confidence from per-component confidence values
  const confidences = [hw.cpu.confidence, hw.primaryGpu.confidence];
  let scanConfidence: 'high' | 'medium' | 'low';
  if (confidences.every(c => c === 'detected')) {
    scanConfidence = 'high';
  } else if (confidences.some(c => c === 'unverified')) {
    scanConfidence = 'low';
  } else {
    scanConfidence = 'medium';
  }

  const profile: HardwareProfile = {
    cpu: cpuModel,
    architecture,
    generation,
    coreCount: hw.coreCount,
    gpu: gpuModel,
    gpuDevices: hw.gpus.map(gpu => ({
      name: gpu.name,
      vendorName: gpu.vendorName,
      vendorId: gpu.vendorId,
      deviceId: gpu.deviceId,
    })),
    ram: ramGB,
    motherboard: hw.motherboardModel || hw.motherboardVendor || 'Unknown',
    targetOS: 'macOS Sequoia 15.x',
    smbios: '',
    kexts: [], ssdts: [],
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop: hw.isLaptop,
    isVM: hw.isVM,
    audioLayoutId: 1,
    scanConfidence,
  };
  profile.smbios = getSMBIOSForProfile(profile);
  return profile;
}

// --- GitHub Kext Fetcher ---

interface KextRegistryEntry {
  repo: string;
  assetFilter?: string;
  directUrl?: string;
  staticVersion?: string;
}

const KEXT_REGISTRY: Record<string, KextRegistryEntry> = {
  'Lilu.kext':                          { repo: 'acidanthera/Lilu',                    assetFilter: 'RELEASE' },
  'VirtualSMC.kext':                    { repo: 'acidanthera/VirtualSMC',              assetFilter: 'RELEASE' },
  'SMCBatteryManager.kext':             { repo: 'acidanthera/VirtualSMC',              assetFilter: 'RELEASE' },
  'WhateverGreen.kext':                 { repo: 'acidanthera/WhateverGreen',           assetFilter: 'RELEASE' },
  'AppleALC.kext':                      { repo: 'acidanthera/AppleALC',                assetFilter: 'RELEASE' },
  'NootedRed.kext':                     { repo: 'ChefKissInc/NootedRed' },
  'NootRX.kext':                        { repo: 'ChefKissInc/NootRX',                  directUrl: 'https://nightly.link/ChefKissInc/NootRX/workflows/main/master/Artifacts.zip', staticVersion: 'nightly' },
  'RTCMemoryFixup.kext':                { repo: 'acidanthera/RTCMemoryFixup',          assetFilter: 'RELEASE' },
  'VoodooPS2Controller.kext':           { repo: 'acidanthera/VoodooPS2',              assetFilter: 'RELEASE' },
  'AMDRyzenCPUPowerManagement.kext':    { repo: 'trulyspinach/SMCAMDProcessor' },
  'SMCAMDProcessor.kext':               { repo: 'trulyspinach/SMCAMDProcessor' },
  'AppleMCEReporterDisabler.kext':      { repo: 'acidanthera/bugtracker',              directUrl: 'https://github.com/acidanthera/bugtracker/files/3703498/AppleMCEReporterDisabler.kext.zip', staticVersion: 'bugtracker' },
  'RestrictEvents.kext':                { repo: 'acidanthera/RestrictEvents',          assetFilter: 'RELEASE' },
  'NVMeFix.kext':                       { repo: 'acidanthera/NVMeFix',                assetFilter: 'RELEASE' },
  'CPUTopologyRebuild.kext':            { repo: 'b00t0x/CpuTopologyRebuild',           assetFilter: 'RELEASE' },
};

async function downloadToTemp(url: string, dest: string, timeoutMs = 120_000, checkAborted?: () => void): Promise<void> {
  return retryWithBackoff(
    () => withTimeout(new Promise<void>((resolve, reject) => {
      checkAborted?.();
      function fetchUrl(urlStr: string, redirects = 0): void {
        if (redirects > 10) { reject(new Error('Too many redirects')); return; }
        let parsedUrl: URL;
        try { parsedUrl = new URL(urlStr); } catch { reject(new Error(`Invalid URL: ${urlStr}`)); return; }

        function doRequest(socket?: net.Socket | tls.TLSSocket): void {
          const isHttps = parsedUrl.protocol === 'https:';
          const lib = isHttps ? https : http;
          const reqOptions: any = {
            hostname: parsedUrl.hostname,
            port: parsedUrl.port || (isHttps ? 443 : 80),
            path: parsedUrl.pathname + parsedUrl.search,
            headers: { 'User-Agent': 'macOS-One-Click/1.0' },
            timeout: timeoutMs,
            ...(socket ? { socket, agent: false } : {}),
          };
          const req = lib.request(reqOptions, (res: any) => {
            if ([301, 302, 307, 308].includes(res.statusCode)) {
              res.resume();
              fetchUrl(res.headers.location, redirects + 1); return;
            }
            if (res.statusCode === 403 || res.statusCode === 429) {
              res.resume();
              const remaining = res.headers['x-ratelimit-remaining'];
              const reset = res.headers['x-ratelimit-reset'];
              const resetAt = reset ? new Date(parseInt(reset) * 1000).toLocaleTimeString() : 'soon';
              reject(new Error(remaining === '0' || res.statusCode === 429
                ? `GitHub API rate limit exceeded — resets at ${resetAt}. Wait and retry.`
                : `GitHub returned 403 for ${urlStr}`));
              return;
            }
            if (res.statusCode !== 200) {
              res.resume();
              reject(new Error(`HTTP ${res.statusCode} fetching ${urlStr}`)); return;
            }
            const file = fs.createWriteStream(dest);
            res.pipe(file);
            file.on('finish', () => file.close(() => resolve()));
            file.on('error', (err: Error) => { try { fs.unlinkSync(dest); } catch(_){} reject(err); });
            res.on('error', (err: Error) => { try { fs.unlinkSync(dest); } catch(_){} reject(err); });
          });
          req.on('timeout', () => { req.destroy(); reject(new Error(`Connection timed out downloading ${urlStr}`)); });
          req.on('error', reject);
          req.end();
        }

        // Use proxy tunnel if available and target is HTTPS
        const proxy = parseProxy();
        if (proxy && parsedUrl.protocol === 'https:') {
          const targetPort = parseInt(parsedUrl.port) || 443;
          const proxySocket = net.createConnection(proxy.port, proxy.host);
          proxySocket.once('connect', () => {
            proxySocket.write(`CONNECT ${parsedUrl.hostname}:${targetPort} HTTP/1.1\r\nHost: ${parsedUrl.hostname}:${targetPort}\r\n\r\n`);
            proxySocket.once('data', (chunk: Buffer) => {
              if (!chunk.toString().startsWith('HTTP/1.1 200') && !chunk.toString().startsWith('HTTP/1.0 200')) {
                reject(new Error(`Proxy CONNECT rejected: ${chunk.toString().split('\r\n')[0]}`));
                proxySocket.destroy(); return;
              }
              const tlsSocket = tls.connect({ socket: proxySocket, servername: parsedUrl.hostname }, () => doRequest(tlsSocket));
              tlsSocket.on('error', reject);
            });
          });
          proxySocket.on('error', reject);
        } else {
          doRequest();
        }
      }
      fetchUrl(url);
    }), timeoutMs + 5_000, `downloadToTemp(${url})`),
    3, `downloadToTemp(${path.basename(dest)})`, checkAborted
  );
}

async function extractZip(zipPath: string, destDir: string): Promise<void> {
  // Verify the zip is a valid file before attempting extraction
  const stat = fs.statSync(zipPath);
  if (stat.size < 22) throw new Error(`Downloaded file is too small to be a valid zip (${stat.size} bytes) — download may be corrupted`);

  // Read the PK magic bytes to confirm it's actually a zip
  const fd = fs.openSync(zipPath, 'r');
  const magic = Buffer.alloc(4);
  fs.readSync(fd, magic, 0, 4, 0);
  fs.closeSync(fd);
  if (magic[0] !== 0x50 || magic[1] !== 0x4B) {
    throw new Error(`Downloaded file is not a valid zip archive (magic: ${magic.toString('hex')}) — possibly an HTML error page`);
  }

  if (process.platform === 'win32') {
    await withTimeout(
      execPromise(`powershell -NoProfile -Command "Expand-Archive -Path '${zipPath.replace(/'/g, "''")}' -DestinationPath '${destDir.replace(/'/g, "''")}' -Force"`),
      60_000, 'extractZip(win32)'
    );
  } else {
    await withTimeout(
      execPromise(`unzip -o "${zipPath}" -d "${destDir}"`),
      60_000, 'extractZip(unix)'
    );
  }
}

function findKextBundles(dir: string, targetName: string): string[] {
  const results: string[] = [];
  function walk(current: string, depth: number) {
    if (depth > 5) return;
    try {
      for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
        if (!entry.isDirectory()) continue;
        const fullPath = path.join(current, entry.name);
        if (entry.name.toLowerCase() === targetName.toLowerCase()) {
          results.push(fullPath);
        } else {
          walk(fullPath, depth + 1);
        }
      }
    } catch (_) {}
  }
  walk(dir, 0);
  return results;
}

function copyDirSync(src: string, dest: string): void {
  fs.mkdirSync(dest, { recursive: true });
  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDirSync(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

function formatPlatformLabel(platform: NodeJS.Platform): string {
  if (platform === 'win32') return 'Windows';
  if (platform === 'darwin') return 'macOS';
  if (platform === 'linux') return 'Linux';
  return platform;
}

function formatHardwareSummary(hardware: any): string {
  if (!hardware) return 'Hardware scan not completed';

  if (typeof hardware.cpu === 'string' && typeof hardware.gpu === 'string') {
    return `${hardware.cpu}, ${hardware.gpu}, ${formatPlatformLabel(process.platform)}`;
  }

  const cpu = typeof hardware.cpu?.name === 'string' ? hardware.cpu.name : null;
  const gpuNames = Array.isArray(hardware.gpus)
    ? hardware.gpus
        .map((gpu: any) => gpu?.name)
        .filter((name: unknown): name is string => typeof name === 'string' && name.length > 0)
    : [];
  const gpuSummary = gpuNames.length > 0
    ? Array.from(new Set(gpuNames)).join(' / ')
    : (typeof hardware.primaryGpu?.name === 'string' ? hardware.primaryGpu.name : null);

  if (!cpu || !gpuSummary) return 'Hardware scan not completed';
  return `${cpu}, ${gpuSummary}, ${formatPlatformLabel(process.platform)}`;
}

function buildCurrentDiagnosticsSnapshot() {
  const tasks = typeof registry?.list === 'function' ? registry.list() : [];
  const lastTask = tasks.length > 0 ? tasks[tasks.length - 1] : null;
  const appTail = logger?.readTail(200) ?? [];
  const relevantLogs = appTail.filter((entry) => entry.level === 'ERROR' || entry.level === 'FATAL' || entry.level === 'WARN');
  const lastErr = relevantLogs.length > 0 ? relevantLogs[relevantLogs.length - 1] : null;
  const compatibilityReport = getCurrentCompatibilityReport();

  let fwSummary = 'Not probed';
  const cachedFirmware = lastFirmwareSummary;
  if (cachedFirmware) {
    fwSummary = cachedFirmware;
  }

  const scanErrorFound = (logger?.readTail(100) ?? []).some((entry) => entry.ctx === 'scan' && entry.level === 'ERROR');
  const hwStatus = lastHardwareProfile
    ? formatHardwareSummary(lastHardwareProfile)
    : scanErrorFound
      ? 'Hardware scan failed — check logs for details'
      : 'Hardware scan not completed';

  const confidenceStatus = lastHardwareProfile
    ? (lastHardwareProfile.scanConfidence || 'unknown')
    : scanErrorFound
      ? 'Unavailable (scan failed)'
      : 'Not yet scanned';

  return createDiagnosticsSnapshot({
    version: app.getVersion(),
    platform: process.platform,
    arch: process.arch,
    compatMode,
    timestamp: new Date().toISOString(),
    sessionId: logger.sessionId,
    hardware: hwStatus,
    confidence: confidenceStatus,
    firmware: fwSummary,
    lastTaskKind: lastTask?.kind ?? null,
    lastTaskStatus: lastTask?.status ?? null,
    lastError: lastErr ? String((lastErr as any).msg ?? 'Unknown') : null,
    failedKexts: failedKexts.map((kext) => `${kext.name} (${kext.repo}: ${kext.error})`),
    kextSources,
    selectedDisk: lastSelectedDisk,
    diskIdentity: lastSelectedDisk ? buildDiskIdentityFingerprint(lastSelectedDisk) : null,
    compatibilityReport,
    validationResult: lastValidationResult,
    recoveryStats: {
      attempts: recoveryStats.attempts,
      lastHttpCode: recoveryStats.lastHttpCode ?? null,
      lastError: recoveryStats.lastError,
      decision: recoveryStats.finalDecision,
      source: (tasks.find((task) => task.kind === 'recovery-download')?.progress as any)?.sourceId ?? 'none',
    },
    recentLogs: relevantLogs,
    lastFailure: lastFailureContext,
  });
}

function saveSupportLogToDesktop(extraContext?: string | null): { fileName: string; savedTo: 'Desktop' } {
  const snapshot = buildCurrentDiagnosticsSnapshot();
  const logBody = buildSavedSupportLog(snapshot, logger?.readOpsTail(200) ?? [], extraContext ?? null);
  const timestamp = new Date().toISOString().replace(/[:]/g, '-');
  const fileName = `macos-one-click-support-log-${timestamp}.txt`;
  const destination = path.join(app.getPath('desktop'), fileName);

  fs.writeFileSync(destination, logBody, 'utf-8');
  logger?.timeline('diagnostics_export', undefined, { target: 'desktop', fileName, trigger: snapshot.trigger });
  log('INFO', 'diagnostics', 'Saved support log bundle', { fileName, destination });

  return { fileName, savedTo: 'Desktop' };
}

async function runEfiValidation(efiPath: string, profile: HardwareProfile | null): Promise<ValidationResult> {
  const result = await validateEfi(efiPath, profile, kextSources);
  lastValidationResult = result;
  return result;
}

function getCurrentBuildProfile(preferredProfile?: HardwareProfile | null): HardwareProfile | null {
  return preferredProfile ?? lastBuildProfile ?? lastScannedProfile;
}

function savePlanningHardwareProfileArtifact(input: {
  profile: unknown;
  interpretation?: unknown;
  source?: HardwareProfileArtifact['source'];
  capturedAt?: number;
}): HardwareProfileArtifact {
  return hardwareProfileStore.saveLatest(createHardwareProfileArtifact({
    profile: input.profile,
    interpretation: input.interpretation,
    source: input.source,
    capturedAt: input.capturedAt,
  }));
}

function requireFlashAuthorizationContext(): {
  buildProfile: HardwareProfile;
  hardwareFingerprint: string;
  hardwareProfileDigest: string;
} {
  const buildProfile = getCurrentBuildProfile();
  if (!buildProfile) {
    throw new Error('SAFETY BLOCK: Flash preparation requires a main-process build context. Rebuild the EFI or rescan hardware before flashing.');
  }
  if (!lastScannedProfile) {
    throw new Error('SAFETY BLOCK: Hardware must be scanned in this session before flashing. Run Scan Hardware again before any destructive action.');
  }
  if (!lastLiveHardwareProfileArtifact?.digest) {
    throw new Error('SAFETY BLOCK: Live hardware profile evidence is missing in the main process. Run Scan Hardware again before any destructive action.');
  }

  return {
    buildProfile,
    hardwareFingerprint: buildHardwareFingerprint(lastScannedProfile),
    hardwareProfileDigest: lastLiveHardwareProfileArtifact.digest,
  };
}

async function inspectEfiBackupPolicy(device: string): Promise<EfiBackupPolicy> {
  try {
    return await withTimeout(
      efiBackupManager.inspectPolicy(device, (targetDevice) => diskOps.inspectExistingEfi(targetDevice)),
      8_000,
      'inspectEfiBackupPolicy',
    );
  } catch (error: any) {
    log('WARN', 'efi-backup', 'EFI backup policy inspection timed out', {
      device,
      error: String(error?.message ?? error),
    });
    return {
      status: 'not_required',
      reason: 'EFI check timed out. Backup requirements will be checked again before writing.',
      existingEfiState: 'absent',
      latestBackup: null,
    };
  }
}

async function captureEfiBackupForFlash(input: {
  device: string;
  expectedIdentity: DiskIdentityFingerprint;
  hardwareProfileDigest: string;
}): Promise<EfiBackupPolicy> {
  return efiBackupManager.captureIfRequired({
    targetDevice: input.device,
    diskIdentity: input.expectedIdentity,
    hardwareProfileDigest: input.hardwareProfileDigest,
    inspectExistingEfi: (targetDevice) => diskOps.inspectExistingEfi(targetDevice),
    copyExistingEfi: (targetDevice, destinationPath) => diskOps.copyExistingEfi(targetDevice, destinationPath),
  });
}

function summarizeHash(value: string | null | undefined): string | null {
  if (!value) return null;
  return value.slice(0, 12);
}

function summarizeTokenId(token: string | null | undefined): string | null {
  if (!token) return null;
  return crypto.createHash('sha256').update(token).digest('hex').slice(0, 12);
}

function summarizeIdentityFingerprint(identity: Partial<DiskInfo> | DiskIdentityFingerprint | null | undefined): {
  fields: string[];
  serialPresent: boolean;
  transport: string | null;
  sizeBytes: number | null;
} {
  const fingerprint = buildDiskIdentityFingerprint(identity);
  return {
    fields: Object.keys(fingerprint).sort(),
    serialPresent: fingerprint.serialNumber !== undefined,
    transport: typeof fingerprint.transport === 'string' ? fingerprint.transport : null,
    sizeBytes: typeof fingerprint.sizeBytes === 'number' ? fingerprint.sizeBytes : null,
  };
}

function summarizeSnapshot(snapshot: FlashAuthorizationSnapshot): Record<string, unknown> {
  return {
    stage: snapshot.stage,
    sessionId: snapshot.sessionId,
    device: snapshot.device,
    efiHash: summarizeHash(snapshot.efiStateHash),
    payloadHash: summarizeHash(snapshot.payloadStateHash),
    hardwareHash: summarizeHash(snapshot.hardwareFingerprint),
    diskIdentity: summarizeIdentityFingerprint(snapshot.diskFingerprint),
  };
}

function buildCurrentFlashSnapshot(input: {
  stage: FlashAuthorizationSnapshot['stage'];
  device: string;
  efiPath: string;
  currentDisk: DiskInfo | null;
  hardwareFingerprint: string;
}): FlashAuthorizationSnapshot {
  const efiExists = fs.existsSync(input.efiPath);
  return createFlashAuthorizationSnapshot({
    stage: input.stage,
    sessionId: flashConfirmationStore.sessionId,
    device: input.device,
    diskFingerprint: input.currentDisk,
    efiStateHash: efiExists ? computeEfiStateHash(input.efiPath) : null,
    payloadStateHash: efiExists ? computeInstallerPayloadHash(input.efiPath) : null,
    hardwareFingerprint: input.hardwareFingerprint,
  });
}

function logFlashAuthorizationEvent(level: 'INFO' | 'WARN' | 'ERROR', message: string, data?: Record<string, unknown>) {
  log(level, 'flash-auth', message, data);
}

function logFlashDecision(input: {
  phase: 'prepare' | 'snapshot_b' | 'snapshot_c';
  device: string;
  decision: ReturnType<typeof canProceedWithFlash>;
  confirmationCode?: string | null;
  snapshot?: FlashAuthorizationSnapshot | null;
  record?: FlashConfirmationRecord | null;
  token?: string | null;
  mismatchFields?: FlashAuthorizationMismatchField[];
  collisionDevices?: string[];
  currentDisk?: DiskInfo | null;
  expectedIdentity?: Partial<DiskInfo> | DiskIdentityFingerprint | null;
}) {
  logFlashAuthorizationEvent('WARN', `Flash authorization blocked at ${input.phase}`, {
    device: input.device,
    code: input.decision.code ?? 'unknown',
    confirmationCode: input.confirmationCode ?? null,
    tokenId: summarizeTokenId(input.token),
    mismatchFields: input.mismatchFields ?? [],
    collisionDevices: input.collisionDevices ?? [],
    snapshot: input.snapshot ? summarizeSnapshot(input.snapshot) : null,
    confirmedSnapshot: input.record ? summarizeSnapshot(createFlashAuthorizationSnapshotFromRecord(input.record, 'snapshot_a')) : null,
    currentDisk: input.currentDisk ? {
      identityConfidence: input.currentDisk.identityConfidence ?? 'unknown',
      identityFieldsUsed: input.currentDisk.identityFieldsUsed ?? [],
      partitionTable: input.currentDisk.partitionTable,
      isSystemDisk: input.currentDisk.isSystemDisk,
      mountedPartitions: input.currentDisk.mountedPartitions,
    } : null,
    expectedIdentity: summarizeIdentityFingerprint(input.expectedIdentity),
  });
}

async function getLiveFirmwareInfo() {
  try {
    return await withTimeout(probeFirmware(), 30_000, 'probeFirmwareForBiosState');
  } catch {
    return null;
  }
}

async function getBiosStateForProfile(profile: HardwareProfile): Promise<BiosOrchestratorState> {
  const firmwareInfo = await getLiveFirmwareInfo();
  const session = loadBiosSession(app.getPath('userData'));
  return buildBiosOrchestratorState({
    profile,
    biosConfig: getBIOSSettings(profile),
    firmwareInfo,
    platform: process.platform,
    safeMode: true,
    session,
  });
}

async function verifyAndPersistBiosState(
  profile: HardwareProfile,
  selectedChanges?: Record<string, BiosSettingSelection>,
  stage: BiosSessionState['stage'] = 'planned',
): Promise<BiosOrchestratorState> {
  const firmwareInfo = await getLiveFirmwareInfo();
  return persistBiosOrchestratorState({
    userDataPath: app.getPath('userData'),
    profile,
    biosConfig: getBIOSSettings(profile),
    firmwareInfo,
    platform: process.platform,
    safeMode: true,
    selectedChanges,
    stageWhenBlocked: stage,
  });
}

async function continueWithCurrentBiosState(
  profile: HardwareProfile,
  selectedChanges?: Record<string, BiosSettingSelection>,
): Promise<BiosOrchestratorState> {
  return persistBiosOrchestratorState({
    userDataPath: app.getPath('userData'),
    profile,
    biosConfig: getBIOSSettings(profile),
    firmwareInfo: null,
    platform: process.platform,
    safeMode: true,
    selectedChanges,
    stageWhenBlocked: 'partially_verified',
  });
}

async function ensureBiosReady(
  profile: HardwareProfile,
  options?: { allowAcceptedSession?: boolean },
): Promise<void> {
  if (options?.allowAcceptedSession) {
    return;
  }
  const state = await getBiosStateForProfile(profile);
  if (!state.readyToBuild || state.stage !== 'complete') {
    throw new Error(`BIOS step incomplete: ${state.blockingIssues[0] ?? 'Required firmware settings are not verified.'}`);
  }
}

async function getBuildFlowGuard(profile: HardwareProfile, allowAcceptedSession = false): Promise<FlowGuardResult> {
  const compatibility = checkCompatibility(profile);
  const compatibilityBlocked = !compatibility.isCompatible || compatibility.errors.length > 0;
  const biosState = await getBiosStateForProfile(profile);
  const biosFlowState = deriveBiosFlowState({
    stage: biosState.stage,
    readyToBuild: biosState.readyToBuild,
  });

  return evaluateBuildGuard({
    compatibilityBlocked,
    biosFlowState,
    biosAccepted: allowAcceptedSession,
    releaseFlowState: deriveReleaseFlowState({
      step: 'building',
      hasProfile: true,
      compatibilityBlocked,
      biosFlowState,
      buildReady: false,
      hasEfi: false,
      validationBlocked: false,
    }),
  });
}

async function getDeployFlowGuard(profile: HardwareProfile, efiPath: string): Promise<FlowGuardResult> {
  const compatibility = checkCompatibility(profile);
  const compatibilityBlocked = !compatibility.isCompatible || compatibility.errors.length > 0;
  const biosState = await getBiosStateForProfile(profile);
  const biosFlowState = deriveBiosFlowState({
    stage: biosState.stage,
    readyToBuild: biosState.readyToBuild,
  });
  const hasEfi = Boolean(efiPath) && fs.existsSync(path.join(efiPath, 'EFI/OC/OpenCore.efi'));
  let validationBlocked = !hasEfi;

  if (hasEfi) {
    const validation = await runEfiValidation(efiPath, profile);
    validationBlocked = validation.overall === 'blocked';
  }

  return evaluateDeployGuard({
    compatibilityBlocked,
    biosFlowState,
    releaseFlowState: deriveReleaseFlowState({
      step: 'usb-select',
      hasProfile: true,
      compatibilityBlocked,
      biosFlowState,
      buildReady: hasEfi && !validationBlocked,
      hasEfi,
      validationBlocked,
    }),
    validationBlocked,
    hasEfi,
  });
}

async function getFlashCollisionDevices(expectedIdentity: Partial<DiskInfo> | DiskIdentityFingerprint, device: string): Promise<string[]> {
  const peers = await diskOps.listUsbDevices().catch(() => [] as Array<{ device: string }>);
  const peerInfos = await Promise.all(
    peers
      .filter((peer) => peer.device !== device)
      .map((peer) => diskOps.getDiskInfo(peer.device).catch(() => null)),
  );
  const collisions = findDiskIdentityCollisions(expectedIdentity, device, peerInfos);
  if (collisions.length > 0) {
    logFlashAuthorizationEvent('WARN', 'Disk identity collision detected', {
      device,
      collisions,
      expectedIdentity: summarizeIdentityFingerprint(expectedIdentity),
      peerCount: peerInfos.filter(Boolean).length,
    });
  }
  return collisions;
}

async function validateFlashExecutionContext(input: {
  device: string;
  efiPath: string;
  buildProfile: HardwareProfile;
  hardwareFingerprint: string;
  currentDisk: DiskInfo | null;
  explicitUserConfirmation: boolean;
  confirmationToken: string | null;
  consumeConfirmation: boolean;
}) {
  const snapshot = buildCurrentFlashSnapshot({
    stage: 'snapshot_b',
    device: input.device,
    efiPath: input.efiPath,
    currentDisk: input.currentDisk,
    hardwareFingerprint: input.hardwareFingerprint,
  });
  const lookup = input.consumeConfirmation
    ? flashConfirmationStore.consume(input.confirmationToken)
    : flashConfirmationStore.peek(input.confirmationToken);
  const confirmation = validateFlashConfirmation({
    lookup,
    snapshot,
  });
  const deployGuard = await getDeployFlowGuard(input.buildProfile, input.efiPath);
  const biosState = await getBiosStateForProfile(input.buildProfile);
  const validation = await runEfiValidation(input.efiPath, input.buildProfile);
  const effectiveIdentity = lookup.record?.diskFingerprint ?? null;
  const collisionDevices = effectiveIdentity
    ? await getFlashCollisionDevices(effectiveIdentity, input.device)
    : [];

  return {
    decision: canProceedWithFlash({
      selectedDevice: input.device,
      currentDisk: input.currentDisk,
      expectedIdentity: effectiveIdentity,
      collisionDevices,
      deployGuardAllowed: deployGuard.allowed,
      deployGuardReason: deployGuard.reason,
      biosReady: biosState.readyToBuild && biosState.stage === 'complete',
      efiValidationClean: validation.overall !== 'blocked',
      explicitUserConfirmation: input.explicitUserConfirmation,
      confirmationValidated: confirmation,
    }),
    confirmation,
    lookup,
    record: lookup.record,
    snapshot,
    collisionDevices,
    currentDisk: input.currentDisk,
  };
}

async function validateFlashPreWriteContext(input: {
  device: string;
  efiPath: string;
  buildProfile: HardwareProfile;
  hardwareFingerprint: string;
  explicitUserConfirmation: boolean;
  record: FlashConfirmationRecord;
  verificationSnapshot: FlashAuthorizationSnapshot;
}) {
  const currentDisk = await diskOps.getDiskInfo(input.device).catch(() => null);
  const snapshot = buildCurrentFlashSnapshot({
    stage: 'snapshot_c',
    device: input.device,
    efiPath: input.efiPath,
    currentDisk,
    hardwareFingerprint: input.hardwareFingerprint,
  });
  let confirmation = validateFlashConfirmationRecord({
    record: input.record,
    snapshot,
  });

  if (confirmation.valid) {
    const verificationComparison = compareFlashAuthorizationSnapshots(input.verificationSnapshot, snapshot);
    if (!verificationComparison.ok) {
      confirmation = buildFlashConfirmationValidationFromSnapshotComparison({
        comparison: verificationComparison,
      });
    }
  }

  const deployGuard = await getDeployFlowGuard(input.buildProfile, input.efiPath);
  const biosState = await getBiosStateForProfile(input.buildProfile);
  const validation = await runEfiValidation(input.efiPath, input.buildProfile);
  const collisionDevices = await getFlashCollisionDevices(input.record.diskFingerprint, input.device);

  return {
    decision: canProceedWithFlash({
      selectedDevice: input.device,
      currentDisk,
      expectedIdentity: input.record.diskFingerprint,
      collisionDevices,
      deployGuardAllowed: deployGuard.allowed,
      deployGuardReason: deployGuard.reason,
      biosReady: biosState.readyToBuild && biosState.stage === 'complete',
      efiValidationClean: validation.overall !== 'blocked',
      explicitUserConfirmation: input.explicitUserConfirmation,
      confirmationValidated: confirmation,
    }),
    confirmation,
    snapshot,
    collisionDevices,
    currentDisk,
  };
}

async function fetchKextFromGitHub(kextName: string, targetDir: string, checkAborted?: () => void): Promise<{ name: string; version: string }> {
  const entry = KEXT_REGISTRY[kextName];
  if (!entry) return { name: kextName, version: 'bundled' };

  const proxy = parseProxy();

  async function queryGitHubRelease(): Promise<{ version: string; assetUrl: string | null; assetName: string | null }> {
    if (entry.directUrl) {
      return {
        version: entry.staticVersion ?? 'direct',
        assetUrl: entry.directUrl,
        assetName: path.basename(new URL(entry.directUrl).pathname) || null,
      };
    }
    return new Promise((resolve, reject) => {
      const reqOptions: any = {
        hostname: 'api.github.com',
        port: 443,
        path: `/repos/${entry.repo}/releases/latest`,
        headers: { 'User-Agent': 'macOS-One-Click/1.0' },
        timeout: 30_000,
      };
      function doQuery(socket?: net.Socket | tls.TLSSocket): void {
        if (socket) { reqOptions.socket = socket; reqOptions.agent = false; }
        const req = https.request(reqOptions, (res) => {
          if (res.statusCode === 403 || res.statusCode === 429) {
            res.resume();
            const reset = res.headers['x-ratelimit-reset'] as string | undefined;
            const resetAt = reset ? new Date(parseInt(reset) * 1000).toLocaleTimeString() : 'soon';
            reject(new Error(`GitHub API rate limit exceeded — resets at ${resetAt}`)); return;
          }
          if (res.statusCode && res.statusCode >= 400) {
            res.resume(); reject(new Error(`GitHub API HTTP ${res.statusCode} for ${entry.repo}`)); return;
          }
          let data = '';
          res.on('data', (chunk: Buffer) => data += chunk);
          res.on('end', () => {
            try {
              const release = JSON.parse(data);
              const version: string = release.tag_name || 'unknown';
              const assets: Array<{ name: string; browser_download_url: string }> = release.assets || [];
              let asset = assets.find(a => a.name.endsWith('.zip') && (!entry.assetFilter || a.name.toUpperCase().includes(entry.assetFilter.toUpperCase())));
              if (!asset) asset = assets.find(a => a.name.endsWith('.zip'));
              resolve({ version, assetUrl: asset?.browser_download_url ?? null, assetName: asset?.name ?? null });
            } catch(e) { reject(e); }
          });
          res.on('error', reject);
        });
        req.on('error', reject);
        req.on('timeout', () => { req.destroy(); reject(new Error('GitHub API request timed out')); });
        req.end();
      }
      if (proxy) {
        const sock = net.createConnection(proxy.port, proxy.host);
        sock.once('connect', () => {
          sock.write(`CONNECT api.github.com:443 HTTP/1.1\r\nHost: api.github.com:443\r\n\r\n`);
          sock.once('data', (c: Buffer) => {
            if (!c.toString().startsWith('HTTP/1.1 200') && !c.toString().startsWith('HTTP/1.0 200')) {
              reject(new Error(`Proxy CONNECT failed: ${c.toString().split('\r\n')[0]}`)); sock.destroy(); return;
            }
            const tlsSock = tls.connect({ socket: sock, servername: 'api.github.com' }, () => doQuery(tlsSock));
            tlsSock.on('error', reject);
          });
        });
        sock.on('error', reject);
      } else { doQuery(); }
    });
  }

  try {
    const { version, assetUrl } = await retryWithBackoff(queryGitHubRelease, 3, `github-api(${kextName})`, checkAborted);
    const dest = path.join(targetDir, kextName);
    const versionFile = path.join(dest, '.version');
    const manifestFile = path.join(dest, 'manifest.json');

    if (fs.existsSync(dest) && fs.existsSync(versionFile)) {
      const installedVersion = fs.readFileSync(versionFile, 'utf-8').trim();
      if (installedVersion === version) {
        const integrity = verifyFolderIntegrity(dest, manifestFile);
        if (integrity.valid) {
          log('INFO', 'kext', `Reusing validated ${kextName} ${version}`, { dest });
          return { name: kextName, version };
        }
      }
    }

    if (!assetUrl) {
      const kextDir = path.join(targetDir, kextName);
      if (!fs.existsSync(kextDir)) fs.mkdirSync(kextDir, { recursive: true });
      fs.writeFileSync(path.join(kextDir, '.version'), version);
      generateFolderManifest(kextDir, path.join(kextDir, 'manifest.json'));
      return { name: kextName, version };
    }

    const tmpZip = path.join(os.tmpdir(), `oc_${kextName.replace(/[^a-zA-Z0-9]/g, '_')}_${Date.now()}.zip`);
    const tmpExtract = path.join(os.tmpdir(), `oc_extract_${Date.now()}`);
    log('INFO', 'kext', `Downloading ${kextName} ${version}`, { url: assetUrl });

    try {
      checkAborted?.();
      await downloadToTemp(assetUrl, tmpZip, 120_000, checkAborted);
      log('DEBUG', 'kext', `Download complete, extracting`, { kextName, tmpZip });
      checkAborted?.();
      fs.mkdirSync(tmpExtract, { recursive: true });
      await extractZip(tmpZip, tmpExtract);

      const bundles = findKextBundles(tmpExtract, kextName);
      if (bundles.length > 0) {
        const staging = dest + '.staging';
        try {
          if (fs.existsSync(staging)) fs.rmSync(staging, { recursive: true, force: true });
          copyDirSync(bundles[0], staging);
          fs.writeFileSync(path.join(staging, '.version'), version);
          generateFolderManifest(staging, path.join(staging, 'manifest.json'));
          if (fs.existsSync(dest)) fs.rmSync(dest, { recursive: true, force: true });
          fs.renameSync(staging, dest);
          log('INFO', 'kext', `Installed ${kextName} ${version}`, { dest });
        } catch (renameErr) {
          try { if (fs.existsSync(staging)) fs.rmSync(staging, { recursive: true, force: true }); } catch (_) {}
          if (fs.existsSync(dest)) fs.rmSync(dest, { recursive: true, force: true });
          copyDirSync(bundles[0], dest);
          fs.writeFileSync(path.join(dest, '.version'), version);
          generateFolderManifest(dest, path.join(dest, 'manifest.json'));
          log('WARN', 'kext', `Atomic rename failed, used copy fallback`, { kextName });
        }
      } else {
        log('WARN', 'kext', `Could not find ${kextName} in extracted archive — writing stub`, { tmpExtract });
        if (!fs.existsSync(dest)) fs.mkdirSync(dest, { recursive: true });
        fs.writeFileSync(path.join(dest, '.version'), version);
        generateFolderManifest(dest, path.join(dest, 'manifest.json'));
      }
    } finally {
      try { if (fs.existsSync(tmpZip)) fs.unlinkSync(tmpZip); } catch (_) {}
      try { if (fs.existsSync(tmpExtract)) fs.rmSync(tmpExtract, { recursive: true, force: true }); } catch (_) {}
    }
    return { name: kextName, version };
  } catch (err) {
    const msg = classifyError(err);
    log('ERROR', 'kext', `Failed to fetch ${kextName} — using offline stub`, { error: msg });
    const kextDir = path.join(targetDir, kextName);
    if (!fs.existsSync(kextDir)) fs.mkdirSync(kextDir, { recursive: true });
    fs.writeFileSync(path.join(kextDir, '.version'), 'offline');
    return { name: kextName, version: 'offline' };
  }
}

// --- USB Drive Listing (moved to diskOps.ts) ---

// --- Production Lock ---

function enableProductionLock(efiPath: string, targetOS?: string): boolean {
  const configPath = path.join(efiPath, 'EFI/OC/config.plist');
  if (!fs.existsSync(configPath)) return false;

  const secureBootModel = targetOS && parseMacOSVersion(targetOS) >= 26 ? 'Disabled' : 'Default';
  let content = fs.readFileSync(configPath, 'utf-8');
  // Set SecureBootModel to the documented release-appropriate value.
  content = content.replace(
    /<key>SecureBootModel<\/key>\s*<string>[^<]*<\/string>/,
    `<key>SecureBootModel</key><string>${secureBootModel}</string>`
  );
  // Set DisableSecurityPolicy to false
  content = content.replace(
    /<key>DisableSecurityPolicy<\/key>\s*<true\/>/,
    '<key>DisableSecurityPolicy</key><false/>'
  );
  // Set Vault to Secure
  content = content.replace(
    /<key>Vault<\/key>\s*<string>Optional<\/string>/,
    '<key>Vault</key><string>Secure</string>'
  );
  fs.writeFileSync(configPath, content);
  return true;
}

// --- Partitioning Logic (moved to diskOps.ts) ---

// --- Safety Helpers (moved to diskOps.ts) ---
// getSystemDiskId, getDiskPartitionTable, getDirSizeSync, getFreeSpaceMB are now in diskOps.ts

// --- Preflight ---

interface PreflightIssue { severity: 'error' | 'warn'; message: string; }
interface PreflightResult {
  ok: boolean;
  issues: PreflightIssue[];
  adminPrivileges: boolean;
  binaries: Record<string, boolean>;
  freeSpaceMB: number;
}

async function runPreflight(): Promise<PreflightResult> {
  const issues: PreflightIssue[] = [];

  // Admin privileges check
  let adminPrivileges = false;
  if (process.platform === 'win32') {
    try { await execPromise('net session >nul 2>&1'); adminPrivileges = true; } catch { adminPrivileges = false; }
  } else {
    adminPrivileges = (process.getuid?.() ?? 1) === 0;
  }
  if (!adminPrivileges) {
    issues.push({ severity: 'error', message: process.platform === 'win32' ? 'Run as Administrator for disk operations' : 'Root/sudo required for disk and USB operations' });
  }

  // Required binaries
  const platformBins: Record<string, string[]> = {
    darwin:  ['diskutil', 'cp', 'unzip'],
    linux:   ['parted', 'mkfs.fat', 'unzip', 'lsblk', 'mount'],
    win32:   ['diskpart', 'xcopy'],
  };
  const bins = platformBins[process.platform] ?? [];
  const binaries: Record<string, boolean> = {};
  await Promise.all(bins.map(async bin => {
    try {
      await execPromise(process.platform === 'win32' ? `where ${bin}` : `which ${bin}`);
      binaries[bin] = true;
    } catch {
      binaries[bin] = false;
      issues.push({ severity: 'error', message: `Required binary not found: ${bin} — install it before continuing` });
    }
  }));

  // Disk space in userData
  const freeSpaceMB = await getFreeSpaceMB(app.getPath('userData'));
  if (freeSpaceMB < 5000 && freeSpaceMB !== Infinity) {
    issues.push({ severity: 'error', message: `Only ${freeSpaceMB} MB free in user data dir — need at least 5 GB for recovery download + EFI build` });
  } else if (freeSpaceMB < 8000 && freeSpaceMB !== Infinity) {
    issues.push({ severity: 'warn', message: `Low disk space: ${freeSpaceMB} MB free — recommend at least 8 GB` });
  }

  return { ok: issues.filter(i => i.severity === 'error').length === 0, issues, adminPrivileges, binaries, freeSpaceMB };
}

// --- Main Window ---

let mainWindow: BrowserWindow | null = null;
const startupLifecycle = {
  preloadReady: false,
  rendererReady: false,
  recoveryShown: false,
  readyTimer: null as NodeJS.Timeout | null,
  mainFrameLoadRetries: 0,
  appEntryUrl: null as string | null,
  safeEntryUrl: null as string | null,
  packagedDistRoot: null as string | null,
};

function clearStartupReadyTimer(): void {
  if (startupLifecycle.readyTimer) {
    clearTimeout(startupLifecycle.readyTimer);
    startupLifecycle.readyTimer = null;
  }
}

function resetStartupLifecycle(appEntryUrl: string | null, packagedDistRoot: string | null): void {
  clearStartupReadyTimer();
  startupLifecycle.preloadReady = false;
  startupLifecycle.rendererReady = false;
  startupLifecycle.recoveryShown = false;
  startupLifecycle.mainFrameLoadRetries = 0;
  startupLifecycle.appEntryUrl = appEntryUrl;
  startupLifecycle.safeEntryUrl = appEntryUrl
    ? (() => {
        const safeUrl = new URL(appEntryUrl);
        safeUrl.searchParams.set('safe-recovery', '1');
        return safeUrl.toString();
      })()
    : null;
  startupLifecycle.packagedDistRoot = packagedDistRoot;
}

function showMainWindowIfNeeded(): void {
  if (mainWindow && !mainWindow.isDestroyed() && !mainWindow.isVisible()) {
    mainWindow.show();
  }
}

function isTrustedRendererNavigation(targetUrl: string): boolean {
  if (targetUrl.startsWith('data:text/html')) return true;
  if (targetUrl.startsWith('about:blank')) return true;
  if (targetUrl.startsWith('devtools://')) return true;

  const appEntryUrl = startupLifecycle.appEntryUrl;
  if (appEntryUrl) {
    const appEntry = new URL(appEntryUrl);
    const target = new URL(targetUrl);
    if (appEntry.protocol === 'http:' || appEntry.protocol === 'https:') {
      return target.origin === appEntry.origin;
    }
    if (target.protocol === 'file:' && startupLifecycle.packagedDistRoot) {
      try {
        return fileURLToPath(target).startsWith(startupLifecycle.packagedDistRoot);
      } catch {
        return false;
      }
    }
  }

  return false;
}

function isSafeExternalTarget(targetUrl: string): boolean {
  return targetUrl.startsWith('https://') || targetUrl.startsWith('http://') || targetUrl.startsWith('mailto:');
}

async function showStartupRecovery(input: StartupFailureEventInput): Promise<void> {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  if (startupLifecycle.recoveryShown && input.kind !== 'renderer_process_gone') return;

  startupLifecycle.recoveryShown = true;
  clearStartupReadyTimer();
  const preliminarySnapshot = buildCurrentDiagnosticsSnapshot();
  const preliminaryDraft = buildIssueReportDraft(preliminarySnapshot);
  const descriptor = describeStartupFailure({
    ...input,
    diagnostics: preliminarySnapshot,
    issueDraft: preliminaryDraft,
  });

  rememberFailureContext({
    trigger: 'startup_failure',
    message: descriptor.failureMessage,
    detail: descriptor.technicalSummary,
    channel: 'startup',
    code: input.kind,
  });

  const snapshot = buildCurrentDiagnosticsSnapshot();
  const draft = buildIssueReportDraft(snapshot);
  log('WARN', 'startup', 'startup recovery shown', {
    kind: input.kind,
    retryAvailable: Boolean(startupLifecycle.appEntryUrl),
    safeRecoveryAvailable: Boolean(startupLifecycle.safeEntryUrl),
  });
  await mainWindow.loadURL(buildStartupFailurePageUrl({
    ...input,
    diagnostics: snapshot,
    issueDraft: draft,
    retryTargetUrl: startupLifecycle.appEntryUrl,
    safeTargetUrl: startupLifecycle.safeEntryUrl,
  }));
  showMainWindowIfNeeded();
}

function armRendererReadyWatchdog(): void {
  clearStartupReadyTimer();
  if (startupLifecycle.rendererReady || startupLifecycle.recoveryShown) return;

  startupLifecycle.readyTimer = setTimeout(() => {
    if (startupLifecycle.rendererReady || startupLifecycle.recoveryShown) return;
    void showStartupRecovery({
      kind: 'renderer_boot_timeout',
      detail: startupLifecycle.preloadReady
        ? 'The renderer HTML loaded, but the application UI never signaled readiness.'
        : 'The renderer HTML loaded, but the preload bridge never signaled readiness.',
    });
  }, RENDERER_READY_TIMEOUT_MS);
}

function createWindow() {
  const preloadPath = getPreloadScriptPath(__dirname);
  const preloadExists = fs.existsSync(preloadPath);
  const appEntryUrl = app.isPackaged
    ? pathToFileURL(getPackagedRendererEntryPath(__dirname)).toString()
    : 'http://localhost:5173';
  const packagedDistRoot = app.isPackaged
    ? path.dirname(getPackagedRendererEntryPath(__dirname))
    : null;

  resetStartupLifecycle(appEntryUrl, packagedDistRoot);
  log('INFO', 'startup', 'createWindow — begin', {
    preloadPath,
    preloadExists,
    platform: process.platform,
    packaged: app.isPackaged,
    __dirname,
    compatMode,
  });

  if (!preloadExists) {
    log('ERROR', 'startup', 'Preload script missing — renderer will have no window.electron API', { preloadPath });
  }

  mainWindow = new BrowserWindow({
    width: 1200, height: 800,
    minWidth: 960, minHeight: 650,
    show: false,
    // hiddenInset is macOS-only; use default on Windows/Linux to avoid init crash
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
    backgroundColor: '#050505',
    webPreferences: {
      preload: preloadPath,
      contextIsolation: true, nodeIntegration: false, sandbox: false
    }
  });

  log('INFO', 'startup', 'BrowserWindow constructed');

  // ── Renderer lifecycle events ────────────────────────────────────────────
  mainWindow.webContents.on('did-finish-load', () => {
    log('INFO', 'startup', 'did-finish-load — renderer fully loaded');
    showMainWindowIfNeeded();
    if (!startupLifecycle.recoveryShown) {
      armRendererReadyWatchdog();
    }
  });

  mainWindow.webContents.on('did-fail-load', (_e, errorCode, errorDescription, validatedURL, isMainFrame) => {
    const action = determineDidFailLoadAction(
      { errorCode, errorDescription, validatedURL, isMainFrame },
      startupLifecycle.mainFrameLoadRetries,
    );

    if (action === 'ignore') {
      log('INFO', 'startup', 'did-fail-load ignored', { errorCode, errorDescription, validatedURL, isMainFrame });
      return;
    }

    if (action === 'retry' && startupLifecycle.appEntryUrl && !startupLifecycle.recoveryShown) {
      const retryWindow = mainWindow;
      if (!retryWindow) {
        void showStartupRecovery({
          kind: 'did_fail_load',
          detail: `Main window was unavailable during startup retry: ${validatedURL}`,
        });
        return;
      }
      startupLifecycle.mainFrameLoadRetries += 1;
      startupLifecycle.rendererReady = false;
      startupLifecycle.preloadReady = false;
      clearStartupReadyTimer();
      log('WARN', 'startup', 'did-fail-load main-frame failure — retrying once', {
        errorCode,
        errorDescription,
        validatedURL,
        isMainFrame,
        retry: startupLifecycle.mainFrameLoadRetries,
        maxRetries: MAX_MAIN_FRAME_LOAD_RETRIES,
      });
      void retryWindow.loadURL(startupLifecycle.appEntryUrl).catch((error) => {
        log('FATAL', 'startup', 'automatic startup reload failed', { error: String(error) });
        void showStartupRecovery({
          kind: 'load_rejected',
          detail: `Automatic startup retry failed: ${String(error)}`,
        });
      });
      return;
    }

    log('ERROR', 'startup', 'did-fail-load', { errorCode, errorDescription, validatedURL, isMainFrame });
    void showStartupRecovery({
      kind: 'did_fail_load',
      errorCode,
      errorDescription,
      validatedURL,
      detail: `Navigation failed with ${errorDescription} (${errorCode}).`,
    });
  });

  mainWindow.webContents.on('render-process-gone' as any, (_e: unknown, details: { reason: string; exitCode: number }) => {
    log('FATAL', 'startup', 'renderer-process-gone', {
      reason: details.reason,
      exitCode: details.exitCode,
    });
    if (logger) logger.flush();
    void showStartupRecovery({
      kind: 'renderer_process_gone',
      reason: details.reason,
      exitCode: details.exitCode,
      detail: `Renderer process exited with reason=${details.reason} exitCode=${details.exitCode}.`,
    });
  });

  mainWindow.once('ready-to-show', () => {
    log('INFO', 'startup', 'ready-to-show — window painted, about to become visible');
    showMainWindowIfNeeded();
  });

  mainWindow.on('show',  () => log('INFO', 'startup', 'window show event'));
  mainWindow.on('focus', () => log('INFO', 'startup', 'window focus event'));

  mainWindow.on('unresponsive', () => {
    log('WARN', 'startup', 'renderer unresponsive');
    if (logger) logger.flush();
  });

  mainWindow.on('responsive', () => {
    log('INFO', 'startup', 'renderer responsive again');
  });

  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (!isTrustedRendererNavigation(url) && isSafeExternalTarget(url)) {
      void shell.openExternal(url);
    }
    return { action: 'deny' };
  });

  mainWindow.webContents.on('will-navigate', (event, url) => {
    if (isTrustedRendererNavigation(url)) return;
    event.preventDefault();
    if (isSafeExternalTarget(url)) {
      void shell.openExternal(url);
    }
  });

  if (app.isPackaged) {
    const indexPath = getPackagedRendererEntryPath(__dirname);
    const indexExists = fs.existsSync(indexPath);
    log('INFO', 'startup', 'loadFile — packaged', { indexPath, exists: indexExists });

    if (!preloadExists || !indexExists) {
      log('FATAL', 'startup', 'Packaged startup asset missing', {
        preloadExists,
        indexExists,
      });
      void showStartupRecovery({
        kind: 'missing_assets',
        preloadExists,
        indexExists,
      });
      return;
    }

    void mainWindow.loadFile(indexPath).catch((error) => {
      log('FATAL', 'startup', 'loadFile failed', { error: String(error) });
      void showStartupRecovery({
        kind: 'load_rejected',
        detail: String(error),
      });
    });
  } else {
    log('INFO', 'startup', 'loadURL — dev server');
    void mainWindow.loadURL('http://localhost:5173').catch((error) => {
      log('FATAL', 'startup', 'loadURL failed', { error: String(error) });
      void showStartupRecovery({
        kind: 'load_rejected',
        detail: `Renderer dev server could not be reached: ${String(error)}`,
      });
    });
  }

  log('INFO', 'startup', 'createWindow — complete (loadFile/loadURL queued)');
}

app.whenReady().then(async () => {
  // Initialise logger
  logger = createLogger({
    logFile:          path.join(app.getPath('userData'), 'app.log'),
    opsFile:          path.join(app.getPath('userData'), 'operations.log'),
    minLevel:         app.isPackaged ? 'INFO' : 'DEBUG',
    maxFileSizeBytes: 2 * 1024 * 1024,
    rotationCount:    2,
    flushIntervalMs:  200,
    crashSafeSync:    true,
    isPackaged:       app.isPackaged,
  });
  app.on('before-quit', () => {
    logger.timeline('app_quit', undefined, {});
    logger.flush();
    const lockFile = path.join(app.getPath('userData'), 'session.lock');
    try { if (fs.existsSync(lockFile)) fs.unlinkSync(lockFile); } catch {}
  });
  hardwareProfileStore = createHardwareProfileStore(app.getPath('userData'));
  efiBackupManager = createEfiBackupManager(app.getPath('userData'));
  flashConfirmationStore = createFlashConfirmationStore(() => Date.now(), logger.sessionId);
  logFlashAuthorizationEvent('INFO', 'Flash authorization session started', {
    sessionId: flashConfirmationStore.sessionId,
  });

  // ── GPU / child-process crash capture ──────────────────────────────────────
  // child-process-gone covers GPU process, network service, renderer, utility.
  // This is the primary signal for Windows Server 2016 GPU driver crashes.
  app.on('child-process-gone' as any, (_e: unknown, details: {
    type: string; reason: string; exitCode: number; name?: string; serviceName?: string;
  }) => {
    log('FATAL', 'gpu', 'child-process-gone', {
      type: details.type,
      reason: details.reason,
      exitCode: details.exitCode,
      name: details.name,
      serviceName: details.serviceName,
    });
    if (logger) logger.flush();
  });

  // Legacy GPU crash event — still emitted by Electron 41 for backwards compat
  app.on('gpu-process-crashed' as any, (_e: unknown, killed: boolean) => {
    log('FATAL', 'gpu', 'gpu-process-crashed', { killed, compatMode });
    if (logger) logger.flush();
  });

  // Log GPU info once — identifies the adapter and driver on the test machine
  try {
    const gpuInfo = await (app as any).getGPUInfo('basic');
    log('INFO', 'gpu', 'GPU info', {
      gpuDevice: (gpuInfo as any)?.gpuDevice,
      driverVersion: (gpuInfo as any)?.driverVersion,
      auxAttributes: (gpuInfo as any)?.auxAttributes,
    });
  } catch (e) {
    log('WARN', 'gpu', 'getGPUInfo failed', { error: String(e) });
  }

  // Initialise disk ops
  diskOps = createDiskOps(log);

  createWindow();

  // Initialise task registry
  registry = createTaskRegistry(
    (p: TaskUpdatePayload) => mainWindow?.webContents.send('task:update', p),
    logger
  );

  const sendAlert = (message: string) => {
    mainWindow?.webContents.send('task:update', {
      task: {
        taskId: 'alert-' + Date.now(),
        kind: 'system-alert' as any,
        status: 'failed',
        error: message,
        progress: null,
        startedAt: Date.now(),
        endedAt: Date.now()
      }
    });
  };

  // State persistence
  ipcHandle('get-persisted-state', () => loadState());
  ipcHandle('save-state', (_event, state: AppState) => saveState(state));
  ipcHandle('clear-state', () => { if (fs.existsSync(STATE_FILE)) fs.unlinkSync(STATE_FILE); });

  // Hardware profile artifacts — advisory planning inputs only
  ipcHandle('hardware-profile:get-latest', () => hardwareProfileStore.loadLatest());
  ipcHandle('hardware-profile:save', (_event: Electron.IpcMainInvokeEvent, payload: {
    profile: HardwareProfile;
    interpretation?: HardwareProfileInterpretationMetadata | null;
    source?: HardwareProfileArtifact['source'];
  }) => {
    return savePlanningHardwareProfileArtifact({
      profile: payload.profile,
      interpretation: payload.interpretation ?? null,
      source: payload.source ?? 'manual_planning',
    });
  });
  ipcHandle('hardware-profile:export', async (_event: Electron.IpcMainInvokeEvent, artifactPayload?: HardwareProfileArtifact | null) => {
    const artifact = artifactPayload
      ? parseHardwareProfileArtifact(artifactPayload)
      : hardwareProfileStore.loadLatest();
    if (!artifact) {
      throw new Error('No validated hardware profile artifact is available to export.');
    }

    const result = await dialog.showSaveDialog({
      title: 'Export Hardware Profile',
      defaultPath: path.resolve(app.getPath('documents'), `macos-one-click-hardware-profile-${artifact.digest.slice(0, 12)}.json`),
      filters: [{ name: 'Hardware Profile Artifact', extensions: ['json'] }],
    });
    if (result.canceled || !result.filePath) return null;

    hardwareProfileStore.exportArtifact(artifact, result.filePath);
    return {
      filePath: result.filePath,
      artifact,
    };
  });
  ipcHandle('hardware-profile:import', async () => {
    const result = await dialog.showOpenDialog({
      title: 'Import Hardware Profile Artifact',
      filters: [{ name: 'Hardware Profile Artifact', extensions: ['json'] }],
      properties: ['openFile'],
    });
    if (result.canceled || result.filePaths.length === 0) return null;

    const importedArtifact = hardwareProfileStore.importFromFile(result.filePaths[0]);
    const artifact = savePlanningHardwareProfileArtifact({
      profile: importedArtifact.profile,
      interpretation: importedArtifact.interpretation,
      source: 'imported_artifact',
      capturedAt: importedArtifact.capturedAt,
    });
    log('INFO', 'hardware-profile', 'Imported hardware profile artifact', {
      digest: artifact.digest,
      source: artifact.source,
      filePath: result.filePaths[0],
    });
    return artifact;
  });

  ipcHandle('efi-backup:inspect-policy', async (_event: Electron.IpcMainInvokeEvent, device: string) => {
    return inspectEfiBackupPolicy(device);
  });

  // BIOS orchestration state
  ipcHandle('bios:get-state', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile) => {
    const current = loadBiosSession(app.getPath('userData'));
    if (current?.hardwareFingerprint === buildHardwareFingerprint(profile) && (current.stage === 'awaiting_return' || current.stage === 'rebooting_to_firmware')) {
      updateBiosSessionStage(app.getPath('userData'), 'resumed_from_firmware');
    }
    const state = await getBiosStateForProfile(profile);
    if (state.readyToBuild && state.session && state.session.stage !== 'complete') {
      saveBiosSession(app.getPath('userData'), {
        ...state.session,
        stage: 'complete',
        rebootRequested: false,
        timestamp: Date.now(),
      });
      return getBiosStateForProfile(profile);
    }
    return state;
  });

  ipcHandle('bios:apply-supported', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => {
    const state = await verifyAndPersistBiosState(profile, selectedChanges, 'planned');
    return {
      state,
      appliedCount: 0,
      message: 'Safe mode is active. No BIOS settings were changed automatically; use Restart into BIOS or verify them manually.',
    };
  });

  ipcHandle('bios:verify-manual', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => {
    return verifyAndPersistBiosState(profile, selectedChanges, 'verifying');
  });

  ipcHandle('bios:continue-current-state', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => {
    return continueWithCurrentBiosState(profile, selectedChanges);
  });

  ipcHandle('bios:restart-to-firmware', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => {
    const state = await verifyAndPersistBiosState(profile, selectedChanges, 'planned');
    const restart = await restartIntoFirmware();
    if (!restart.supported) {
      return { supported: false, error: restart.error, state };
    }
    const resumedState = await verifyAndPersistBiosState(profile, selectedChanges, 'awaiting_return');
    updateBiosSessionStage(app.getPath('userData'), 'awaiting_return');
    return { supported: true, state: resumedState };
  });

  ipcHandle('bios:clear-session', async () => {
    clearBiosSession(app.getPath('userData'));
    return true;
  });

  ipcHandle('bios:resume-state', async () => {
    const session = loadBiosSession(app.getPath('userData'));
    if (!session) {
      return { hasSession: false, stage: null, fingerprint: null, stale: false, message: 'No BIOS session found.' };
    }
    const stale = Date.now() - session.timestamp > 7 * 24 * 60 * 60 * 1000;
    return {
      hasSession: true,
      stage: session.stage,
      fingerprint: session.hardwareFingerprint,
      stale,
      message: stale
        ? 'BIOS session is older than 7 days and may be stale.'
        : `BIOS session active — stage: ${session.stage}`,
    };
  });

  ipcHandle('bios:restart-capability', async () => {
    const isWin = process.platform === 'win32';
    return {
      supported: isWin,
      method: isWin ? 'uefi_firmware_command' : 'none',
      requiresAdmin: isWin,
      platform: process.platform,
    };
  });

  ipcHandle('flow:guard-build', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, allowAcceptedSession?: boolean) => {
    return getBuildFlowGuard(profile, allowAcceptedSession === true);
  });

  ipcHandle('flow:guard-deploy', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, efiPath: string) => {
    return getDeployFlowGuard(profile, efiPath);
  });

  // BIOS probing (legacy simple result — kept for backward-compat)
  ipcHandle('probe-bios', () => probeBiosSettings());

  // Full firmware preflight — UEFI mode, Secure Boot, VT, vendor/version, per-requirement checklist
  ipcHandle('probe-firmware', async () => {
    try {
      const info = await withTimeout(probeFirmware(), 30_000, 'probeFirmware');
      lastFirmwareSummary = `SB:${info.secureBoot ?? '?'}, VT:${info.vtEnabled ?? '?'}, VT-d:${info.vtdEnabled ?? '?'}`;
      log('INFO', 'firmware', 'Firmware preflight complete', {
        vendor: info.vendor,
        version: info.version,
        firmwareMode: info.firmwareMode,
        secureBoot: info.secureBoot,
        vtEnabled: info.vtEnabled,
        confidence: info.confidence,
        failingCount: info.requirements.filter(r => r.status === 'failing').length,
      });
      return { ok: true, data: info };
    } catch (err: any) {
      lastFirmwareSummary = 'Probe failed';
      log('ERROR', 'firmware', 'Firmware preflight failed', { error: err?.message });
      return { ok: false, error: err?.message ?? 'Unknown error' };
    }
  });
  
  // System restart
  ipcHandle('restart-computer', () => {
    app.setLoginItemSettings({ openAtLogin: true, path: app.getPath('exe'), args: ['--resuming'] });
    const cmd = process.platform === 'win32' ? 'shutdown /r /t 0' : 'reboot';
    exec(cmd, (err) => { if (err) log('ERROR', 'system', `Restart command failed: ${err.message}`); });
  });
  ipcHandle('disable-autostart', () => app.setLoginItemSettings({ openAtLogin: false }));

  // Restart directly into UEFI firmware (Windows-only)
  // Uses bcdedit to set the firmware as the one-time boot target, then reboots.
  ipcHandle('restart-to-bios', async () => {
    return restartIntoFirmware();
  });

  // Get the persisted download resume state specifically
  ipcHandle('get-download-resume-state', () => {
    const s = loadState();
    if (!s) return null;
    if (!s.timestamp || Date.now() - s.timestamp >= 4 * 3600 * 1000) return null;
    if (!s.recoveryDownloadOffset || s.recoveryDownloadOffset <= 0) return null;
    if (!s.recoveryDmgDest || !s.efiPath || !s.recoveryTargetOS) return null;
    if (!s.profile) return null;
    // Validate paths still exist on disk
    if (!fs.existsSync(s.efiPath)) return null;
    if (!fs.existsSync(path.join(s.efiPath, 'EFI', 'OC', 'config.plist'))) return null;
    if (!fs.existsSync(s.recoveryDmgDest)) return null;
    return {
      offset: s.recoveryDownloadOffset,
      dmgDest: s.recoveryDmgDest,
      clDest: s.recoveryClDest || null,
      efiPath: s.efiPath,
      targetOS: s.recoveryTargetOS || 'macOS Sequoia 15',
    };
  });
  
  // Hardware scan — uses hardwareDetect.ts for rich PCI-ID-based detection;
  // result is normalised into HardwareProfile for the rest of the app
  ipcHandle('scan-hardware', async () => {
    try {
      const hw = await withTimeout(detectHardware(), 18_000, 'detectHardware');
      lastHardwareProfile = hw;

      // Build interpretation layer — separates facts from inferences
      const interpretation = interpretHardware(hw);
      lastHardwareInterpretation = interpretation;

      log('INFO', 'scan', 'Hardware detected', {
        cpu: hw.cpu.name,
        cpuVendor: hw.cpu.vendorName,
        cpuConfidence: hw.cpu.confidence,
        primaryGpu: hw.primaryGpu.name,
        gpuVendor: hw.primaryGpu.vendorName,
        gpuConfidence: hw.primaryGpu.confidence,
        isLaptop: hw.isLaptop,
        isVM: hw.isVM,
        interpretationConfidence: interpretation.overallConfidence,
        manualVerifyCount: interpretation.manualVerificationNeeded.length,
      });

      // Map DetectedHardware → HardwareProfile (legacy shape used by configGenerator)
      const legacyResult = mapDetectedToProfile(hw);
      lastScannedProfile = legacyResult;
      const artifact = savePlanningHardwareProfileArtifact({
        profile: legacyResult,
        interpretation: extractHardwareProfileInterpretationMetadata(interpretation),
        source: 'live_scan',
      });
      lastLiveHardwareProfileArtifact = artifact;
      return { profile: legacyResult, interpretation, artifact };
    } catch (err: any) {
      log('ERROR', 'scan', 'Hardware detection failed, falling back to legacy scanner', { error: err?.message });
      lastHardwareProfile = null;
      lastHardwareInterpretation = null;
      // Fallback to old per-platform functions
      let profile: HardwareProfile;
      if (process.platform === 'darwin') profile = await getMacHardwareInfo();
      else if (process.platform === 'win32') profile = await getWindowsHardwareInfo();
      else profile = await getLinuxHardwareInfo();
      lastScannedProfile = profile;
      const artifact = savePlanningHardwareProfileArtifact({
        profile,
        interpretation: null,
        source: 'legacy_scan',
      });
      lastLiveHardwareProfileArtifact = artifact;
      return { profile, interpretation: null, artifact };
    }
  });

  // EFI build
  ipcHandle('build-efi', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, allowAcceptedSession?: boolean) => {
    lastBuildProfile = profile;
    return runEfiBuildFlow(
      { profile, allowAcceptedSession },
      {
        registry,
        getUserDataPath: () => app.getPath('userData'),
        log,
        rememberFailureContext,
        checkCompatibility,
        ensureBiosReady,
        createEfiStructure,
        withTimeout,
        classifyError,
        createClassifiedIpcError,
        removeDir: (targetPath) => {
          try { fs.rmSync(targetPath, { recursive: true, force: true }); } catch (_) {}
        },
      },
    );
  });

  // Kext fetcher — hybrid: GitHub (latest) → embedded fallback → hard fail
  ipcHandle('fetch-latest-kexts', async (_event: Electron.IpcMainInvokeEvent, efiPath: string, kextNames: string[]) => {
    const kextsDir = path.resolve(efiPath, 'EFI/OC/Kexts');
    if (!fs.existsSync(kextsDir)) fs.mkdirSync(kextsDir, { recursive: true });
    const token = registry.create('kext-fetch');
    log('INFO', 'kext', 'Starting hybrid kext fetch', { count: kextNames.length, taskId: token.taskId });

    const results: { name: string; version: string; source: 'github' | 'embedded' | 'failed' }[] = [];
    failedKexts = [];
    kextSources = {};

    try {
      for (const kextName of kextNames) {
        token.check();
        let result: { name: string; version: string } | null = null;
        let source: 'github' | 'embedded' | 'failed' = 'failed';
        let githubFailureReason: string | null = null;
        let embeddedFailureReason: string | null = null;

        // Layer 1: Try GitHub (latest version)
        try {
          result = await fetchKextFromGitHub(kextName, kextsDir, () => token.check());
          // Validate the GitHub result landed correctly
          if (result.version !== 'offline' && validateInstalledKext(kextName, kextsDir)) {
            source = 'github';
            log('INFO', 'kext', `${kextName} — GitHub OK`, { version: result.version });
          } else {
            log('WARN', 'kext', `${kextName} — GitHub download incomplete, trying embedded`, { version: result?.version });
            result = null; // force fallback
          }
        } catch (err: any) {
          githubFailureReason = err?.message ?? String(err);
          log('WARN', 'kext', `${kextName} — GitHub failed: ${err.message}, trying embedded`);
          result = null;
        }

        // Layer 2: Fallback to embedded kext
        if (!result && hasEmbeddedKext(kextName)) {
          try {
            result = installEmbeddedKext(kextName, kextsDir);
            if (validateInstalledKext(kextName, kextsDir)) {
              source = 'embedded';
              log('INFO', 'kext', `${kextName} — embedded fallback OK`, { version: result.version });
            } else {
              log('ERROR', 'kext', `${kextName} — embedded kext failed validation`);
              embeddedFailureReason = 'Embedded fallback failed post-install validation';
              result = null;
            }
          } catch (embErr: any) {
            embeddedFailureReason = embErr?.message ?? String(embErr);
            log('ERROR', 'kext', `${kextName} — embedded install failed: ${embErr.message}`);
            result = null;
          }
        } else {
          embeddedFailureReason = 'No embedded fallback is bundled for this kext';
        }

        // Hard fail — neither source worked
        if (!result) {
          const entry = KEXT_REGISTRY[kextName];
          const failureParts = [githubFailureReason, embeddedFailureReason]
            .filter((value): value is string => Boolean(value))
            .map((value) => value.trim());
          failedKexts.push({
            name: kextName,
            repo: entry?.repo || 'unknown',
            error: failureParts.length > 0 ? failureParts.join(' | ') : 'Both GitHub and embedded fallback failed',
          });
          result = { name: kextName, version: 'offline' };
          source = 'failed';
          log('ERROR', 'kext', `${kextName} — HARD FAIL: no source available`);
        }

        kextSources[kextName] = source;
        results.push({ ...result, source });
        registry.updateProgress(token.taskId, {
          kind: 'kext-fetch', kextName: result.name, version: result.version,
          index: results.length, total: kextNames.length, source,
        });
      }
      registry.complete(token.taskId);
      return results;
    } catch (e) {
      const classified = classifyError(e);
      if (!token.aborted) registry.fail(token.taskId, classified.message);
      else registry.cancel(token.taskId);
      throw createClassifiedIpcError(classified, e);
    }
  });

  // USB device listing
  ipcHandle('list-usb-devices', () => diskOps.listUsbDevices());

  // Local Partitioning
  ipcHandle('get-hard-drives',   () => withTimeout(diskOps.getHardDrives(), 30_000, 'getHardDrives'));
  ipcHandle('shrink-partition', async (_e: Electron.IpcMainInvokeEvent, disk: string, size: number, confirmed?: boolean) => {
    if (!confirmed) throw new Error('Partition shrink requires explicit user confirmation');
    // Safety: block system disk and verify partition table before destructive op
    const info = await diskOps.getDiskInfo(disk);
    if (info.isSystemDisk) {
      throw new Error(`SAFETY BLOCK: ${disk} is the system boot disk — cannot shrink`);
    }
    if (info.partitionTable === 'unknown') {
      throw new Error(`SAFETY BLOCK: Cannot read partition table for ${disk} — refusing to shrink an unidentified disk`);
    }
    return withTimeout(diskOps.shrinkPartition(disk, size, confirmed), 60_000, 'shrinkPartition');
  });
  ipcHandle('create-boot-partition', async (_e: Electron.IpcMainInvokeEvent, disk: string, efi: string, confirmed: boolean, profileData?: HardwareProfile | null) => {
    if (!confirmed) throw new Error('Partition creation requires explicit user confirmation');
    const effectiveProfile = getCurrentBuildProfile(profileData ?? null);
    if (!effectiveProfile) throw new Error('Hardware profile is required before preparing a boot partition.');
    await ensureBiosReady(effectiveProfile);
    const info = await diskOps.getDiskInfo(disk);
    if (info.isSystemDisk) {
      throw new Error(`SAFETY BLOCK: ${disk} is the system boot disk — cannot create boot partition`);
    }
    if (info.partitionTable === 'unknown') {
      throw new Error(`SAFETY BLOCK: Cannot read partition table for ${disk} — refusing to modify an unidentified disk`);
    }
    if (info.partitionTable === 'mbr') {
      throw new Error(`SAFETY BLOCK: ${disk} uses MBR partition table — OpenCore requires GPT`);
    }
    const token = registry.create('partition-prep');
    const resolvedEfi = path.resolve(efi);
    try {
      const validation = await runEfiValidation(resolvedEfi, getCurrentBuildProfile(profileData ?? null));
      if (validation.overall === 'blocked') {
        const blockers = validation.issues.filter(i => i.severity === 'blocked');
        const msg = `EFI validation failed: ${blockers.map(b => `${b.component} @ ${b.expectedPath}`).join('; ')}`;
        registry.fail(token.taskId, msg);
        throw new Error(msg);
      }
      await diskOps.createBootPartition({
        disk, efiPath: resolvedEfi, confirmed,
        onPhase: (phase, detail) => {
          registry.updateProgress(token.taskId, { kind: 'partition-prep', phase, detail });
        },
        registerProcess: (p) => token.registerProcess(p),
      });
      registry.complete(token.taskId);
    } catch (e) {
      const classified = classifyError(e);
      if (!token.aborted) registry.fail(token.taskId, classified.message);
      else registry.cancel(token.taskId);
      throw createClassifiedIpcError(classified, e);
    }
  });

  // EFI validation — runs pre-flash checks against the built EFI directory
  ipcHandle('validate-efi', async (_event: Electron.IpcMainInvokeEvent, efiPath: string, profileData?: HardwareProfile | null) => {
    log('INFO', 'validate-efi', 'Running EFI validation', { efiPath });
    const profile = getCurrentBuildProfile(profileData ?? null);
    const result = await withTimeout(runEfiValidation(efiPath, profile), 30_000, 'validateEfi');
    log('INFO', 'validate-efi', `Validation complete: ${result.overall}`, { issueCount: result.issues.length });
    if (result.overall === 'blocked') {
      const firstIssue = result.issues[0] ?? null;
      rememberFailureContext({
        trigger: 'efi_validation_failure',
        message: firstIssue?.message ?? 'EFI validation blocked the current build.',
        detail: result.firstFailureTrace
          ? `${result.firstFailureTrace.code} ${result.firstFailureTrace.component} @ ${result.firstFailureTrace.expectedPath}`
          : null,
        code: firstIssue?.code ?? null,
      });
    }
    return result;
  });

  ipcHandle('flash:prepare-confirmation', async (
    _event: Electron.IpcMainInvokeEvent,
    device: string,
    efiPath: string,
    expectedIdentity: Partial<DiskInfo> | undefined,
  ) => {
    const { buildProfile, hardwareFingerprint, hardwareProfileDigest } = requireFlashAuthorizationContext();
    const resolvedEfiPath = path.resolve(efiPath);
    const currentDisk = await diskOps.getDiskInfo(device);
    const capturedIdentity = resolveFlashPreparationIdentity(expectedIdentity ?? null, currentDisk);

    if (!capturedIdentity) {
      throw new Error('SAFETY BLOCK: Disk identity could not be confirmed for this target. Re-select the drive and wait for its details to load before flashing.');
    }
    const deployGuard = await getDeployFlowGuard(buildProfile, resolvedEfiPath);
    const biosState = await getBiosStateForProfile(buildProfile);
    const validation = await runEfiValidation(resolvedEfiPath, buildProfile);
    const collisionDevices = await getFlashCollisionDevices(capturedIdentity, device);
    const decision = canProceedWithFlash({
      selectedDevice: device,
      currentDisk,
      expectedIdentity: capturedIdentity,
      collisionDevices,
      deployGuardAllowed: deployGuard.allowed,
      deployGuardReason: deployGuard.reason,
      biosReady: biosState.readyToBuild && biosState.stage === 'complete',
      efiValidationClean: validation.overall !== 'blocked',
      explicitUserConfirmation: true,
      confirmationValidated: { valid: true, reason: null, code: null },
    });
    if (!decision.allowed) {
      logFlashDecision({
        phase: 'prepare',
        device,
        decision,
        collisionDevices,
        currentDisk,
        expectedIdentity: capturedIdentity,
      });
      throw new Error(decision.reason ?? 'SAFETY BLOCK: Flash preparation failed.');
    }

    const backupPolicy = await withTimeout(
      captureEfiBackupForFlash({
        device,
        expectedIdentity: capturedIdentity,
        hardwareProfileDigest,
      }),
      15_000,
      'captureEfiBackupForFlash',
    );
    if (backupPolicy.status === 'blocked') {
      throw new Error(backupPolicy.reason);
    }

    const snapshot = buildCurrentFlashSnapshot({
      stage: 'snapshot_a',
      device,
      efiPath: resolvedEfiPath,
      currentDisk,
      hardwareFingerprint,
    });
    if (!snapshot.efiStateHash || !snapshot.hardwareFingerprint || !snapshot.diskFingerprint) {
      const reason = !snapshot.efiStateHash
        ? 'SAFETY BLOCK: EFI state could not be captured for confirmation. Rebuild or revalidate the EFI and try again.'
        : !snapshot.diskFingerprint
          ? 'SAFETY BLOCK: Disk identity could not be captured for confirmation. Re-select the drive and try again.'
          : 'SAFETY BLOCK: Hardware fingerprint could not be captured for confirmation. Re-scan hardware and try again.';
      logFlashAuthorizationEvent('WARN', 'Flash confirmation snapshot capture failed', {
        device,
        snapshot: summarizeSnapshot(snapshot),
      });
      throw new Error(reason);
    }

    const record = flashConfirmationStore.issue({
      device,
      expectedIdentity: snapshot.diskFingerprint,
      efiStateHash: snapshot.efiStateHash,
      payloadStateHash: snapshot.payloadStateHash,
      hardwareFingerprint,
    });
    logFlashAuthorizationEvent('INFO', 'Issued flash confirmation token', {
      device,
      tokenId: summarizeTokenId(record.token),
      expiresAt: record.expiresAt,
      sessionId: record.sessionId,
      snapshot: summarizeSnapshot(snapshot),
    });

    return {
      token: record.token,
      expiresAt: record.expiresAt,
      diskInfo: currentDisk,
      backupPolicy,
    };
  });

  // USB flashing — destructive, requires confirmation flag + disk identity
  ipcHandle('flash-usb', async (_event: Electron.IpcMainInvokeEvent, device: string, efiPath: string, confirmed: boolean, confirmationToken?: string | null) => {
    if (!confirmed) throw new Error('USB flash requires explicit confirmation');
    const { buildProfile, hardwareFingerprint } = requireFlashAuthorizationContext();
    const resolvedEfiPath = path.resolve(efiPath);
    const currentDisk = await diskOps.getDiskInfo(device).catch(() => null);
    const verification = await validateFlashExecutionContext({
      device,
      efiPath: resolvedEfiPath,
      buildProfile,
      hardwareFingerprint,
      currentDisk,
      explicitUserConfirmation: confirmed,
      confirmationToken: confirmationToken ?? null,
      consumeConfirmation: true,
    });
    if (!verification.decision.allowed) {
      logFlashDecision({
        phase: 'snapshot_b',
        device,
        decision: verification.decision,
        confirmationCode: verification.confirmation.code,
        snapshot: verification.snapshot,
        record: verification.record,
        token: confirmationToken ?? null,
        mismatchFields: verification.confirmation.mismatchFields,
        collisionDevices: verification.collisionDevices,
        currentDisk: verification.currentDisk,
        expectedIdentity: verification.record?.diskFingerprint ?? null,
      });
      throw new Error(verification.decision.reason ?? 'SAFETY BLOCK: Flash blocked by safety policy.');
    }
    if (!verification.record) {
      logFlashAuthorizationEvent('ERROR', 'Flash verification succeeded without a confirmation record', {
        device,
        tokenId: summarizeTokenId(confirmationToken ?? null),
      });
      throw new Error('SAFETY BLOCK: Flash confirmation record is unavailable. Re-open the confirmation dialog and try again.');
    }

    const token = registry.create('usb-flash');
    logger.timeline('flash_start', token.taskId, { device });
    log('INFO', 'usb-flash', 'Starting USB flash', { device, efiPath: resolvedEfiPath, taskId: token.taskId });
    try {
      lastSelectedDisk = currentDisk;

      // Determine required space: Full recovery (14.5GB) vs EFI only (0.5GB)
      const hasRecovery = fs.existsSync(path.resolve(resolvedEfiPath, 'com.apple.recovery.boot'));
      const requiredSpaceBytes = hasRecovery ? 14.5 * 1e9 : 0.5 * 1e9;

      const safety = await diskOps.runSafetyChecks(device, resolvedEfiPath, requiredSpaceBytes);
      const fatals = safety.violations.filter(v => v.severity === 'fatal');
      if (fatals.length > 0) {
        registry.fail(token.taskId, fatals[0].message);
        throw new Error(fatals[0].message);
      }
      const preWriteVerification = await validateFlashPreWriteContext({
        device,
        efiPath: resolvedEfiPath,
        buildProfile,
        hardwareFingerprint,
        explicitUserConfirmation: confirmed,
        record: verification.record,
        verificationSnapshot: verification.snapshot,
      });
      if (!preWriteVerification.decision.allowed) {
        logFlashDecision({
          phase: 'snapshot_c',
          device,
          decision: preWriteVerification.decision,
          confirmationCode: preWriteVerification.confirmation.code,
          snapshot: preWriteVerification.snapshot,
          record: verification.record,
          token: confirmationToken ?? null,
          mismatchFields: preWriteVerification.confirmation.mismatchFields,
          collisionDevices: preWriteVerification.collisionDevices,
          currentDisk: preWriteVerification.currentDisk,
          expectedIdentity: verification.record.diskFingerprint,
        });
        throw new Error(preWriteVerification.decision.reason ?? 'SAFETY BLOCK: Flash blocked by safety policy.');
      }
      await diskOps.flashUsb({
        device, efiPath: resolvedEfiPath, confirmed,
        onPhase: (phase, detail) => {
          registry.updateProgress(token.taskId, { kind: 'usb-flash', phase, detail });
          token.check();
        },
        checkAborted: () => token.check(),
        registerProcess: (p) => token.registerProcess(p),
      });
      registry.complete(token.taskId);
      log('INFO', 'usb-flash', 'USB flash complete', { device });
      return true;
    } catch (e: any) {
      const classified = classifyError(e);
      logger.timeline('flash_fail', token.taskId, { category: classified.category, error: e.message });
      if (!token.aborted) registry.fail(token.taskId, classified.message);
      else registry.cancel(token.taskId);
      if (logger) logger.flush();
      throw createClassifiedIpcError(classified, e);
    }
  });

  // Production lock
  ipcHandle('enable-production-lock', (_event: Electron.IpcMainInvokeEvent, efiPath: string, targetOS?: string) => enableProductionLock(efiPath, targetOS));

  // ─────────────────────────────────────────────────────────────────
  // Real macOS Recovery Download — Apple osrecovery.apple.com protocol
  // Equivalent to Dortania macrecovery.py but implemented in pure Node.js
  // ─────────────────────────────────────────────────────────────────

  async function appleRecoveryQuery(boardId: string): Promise<{ dmgUrl: string; dmgToken: string; chunklistUrl: string; chunklistToken: string }> {
    const MAX_ATTEMPTS = 3;
    // Escalating delays: immediate, 3s, 8s
    const RETRY_DELAYS = [0, 3000, 8000];
    recoveryStats.attempts = 0;
    recoveryStats.finalDecision = null;

    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
      recoveryStats.attempts = attempt;

      try {
        log('INFO', 'recovery', `Apple recovery query attempt ${attempt}/${MAX_ATTEMPTS}`, { boardId, mlb: APPLE_RECOVERY_MLB_ZERO });

        // Escalating delay before retries
        const delay = RETRY_DELAYS[attempt - 1] || 0;
        if (delay > 0) {
          log('INFO', 'recovery', `Retry delay: ${delay}ms`);
          await new Promise(r => setTimeout(r, delay));
        }

        const result = await queryAppleRecoveryAssets({
          boardId,
          mlb: APPLE_RECOVERY_MLB_ZERO,
          osType: 'default',
        });

        log('INFO', 'recovery', `Apple recovery query succeeded on attempt ${attempt}`);
        recoveryStats.finalDecision = 'apple_official';
        recoveryStats.lastHttpCode = 200;
        return result;

      } catch (e: any) {
        recoveryStats.lastError = e.message;
        const match = e.message.match(/:(\d{3})$/);
        recoveryStats.lastHttpCode = match ? Number(match[1]) : recoveryStats.lastHttpCode;
        const isRetryable = e.message.startsWith('APPLE_AUTH_REJECT') ||
                            e.message.startsWith('APPLE_RATE_LIMIT') ||
                            e.message.startsWith('APPLE_SERVER_ERROR') ||
                            e.message.startsWith('APPLE_HTTP') ||
                            e.message.startsWith('APPLE_EMPTY_RESPONSE') ||
                            e.message.startsWith('CONN_ERR');

        if (attempt < MAX_ATTEMPTS && isRetryable) {
          log('WARN', 'recovery', `Attempt ${attempt} failed (${e.message}). Retrying with fresh serial...`);
          continue;
        }

        // All attempts exhausted or non-retryable error
        recoveryStats.finalDecision = 'apple_rejected';
        // Produce a clean, user-facing error message
        const httpCode = recoveryStats.lastHttpCode;
        if (e.message.startsWith('APPLE_AUTH_REJECT')) {
          throw new Error(`Apple rejected the recovery request after ${attempt} attempt(s) (HTTP ${httpCode}). This is an external service limitation — not a problem with your machine. Use a cached recovery, import manually, or try a different macOS version.`);
        }
        if (e.message.startsWith('APPLE_RATE_LIMIT')) {
          throw new Error(`Apple rate-limited the request after ${attempt} attempt(s). Wait a few minutes and retry, or import a recovery image manually.`);
        }
        if (e.message.startsWith('CONN_ERR')) {
          throw new Error(`Could not reach Apple recovery servers after ${attempt} attempt(s). Check your internet connection, or import a recovery image manually.`);
        }
        throw new Error(`Apple recovery request failed after ${attempt} attempt(s) (HTTP ${httpCode ?? 'unknown'}). Import a recovery image manually or try a different macOS version.`);
      }
    }

    // Should not reach here, but safety net
    recoveryStats.finalDecision = 'apple_rejected';
    throw new Error(`Apple recovery request failed after ${MAX_ATTEMPTS} attempts (last HTTP ${recoveryStats.lastHttpCode ?? 'unknown'}). Import a recovery image manually or try a different macOS version.`);
  }

  ipcHandle('download-recovery', async (_event: Electron.IpcMainInvokeEvent, targetPath: string, macOSVersion: string, startOffset = 0) => {
    const token = registry.create('recovery-download');
    log('INFO', 'recovery', 'Starting recovery acquisition', { macOSVersion, startOffset, taskId: token.taskId });
    const versionDir = cacheManager.getVersionDir(macOSVersion, 'apple_primary');
    const dmgDest = path.join(versionDir, 'BaseSystem.dmg');
    const clDest  = path.join(versionDir, 'BaseSystem.chunklist');

    try {
      // 1. Check local cache first
      const cached = await cacheManager.getCachedAsset(macOSVersion);

      let effectiveOffset = 0;

      if (cached && !cached.isPartial) {
        log('INFO', 'recovery', 'Found valid cached recovery', { version: macOSVersion });
        registry.updateProgress(token.taskId, {
          kind: 'recovery-download',
          percent: 100,
          status: `Using cached recovery (${formatBytes(cached.size)})`,
          sourceId: 'local_cache',
          trustLevel: cached.trustLevel,
          bytesDownloaded: cached.size,
          dmgDest: cached.dmgPath,
          clDest: cached.chunklistPath ?? ''
        });
        
        // Copy from cache to EFI build target
        const finalRecoveryDir = path.resolve(targetPath, 'com.apple.recovery.boot');
        if (!fs.existsSync(finalRecoveryDir)) fs.mkdirSync(finalRecoveryDir, { recursive: true });
        fs.copyFileSync(cached.dmgPath, path.join(finalRecoveryDir, 'BaseSystem.dmg'));
        if (cached.chunklistPath && fs.existsSync(cached.chunklistPath)) {
          fs.copyFileSync(cached.chunklistPath, path.join(finalRecoveryDir, 'BaseSystem.chunklist'));
        }
        
        registry.complete(token.taskId);
        return { dmgPath: path.join(finalRecoveryDir, 'BaseSystem.dmg'), recoveryDir: finalRecoveryDir, cached: true };
      }

      if (cached && cached.isPartial) {
        try {
          effectiveOffset = fs.statSync(cached.dmgPath).size;
          log('INFO', 'recovery', 'Found partial cached recovery', { offset: effectiveOffset });
        } catch {
          log('WARN', 'recovery', 'Partial cache file missing on disk, starting fresh');
          effectiveOffset = 0;
        }
      } else if (startOffset > 0 && fs.existsSync(dmgDest)) {
        try {
          effectiveOffset = fs.statSync(dmgDest).size;
          log('INFO', 'recovery', 'Renderer requested resume; using cached file size', {
            requestedOffset: startOffset,
            actualOffset: effectiveOffset,
          });
        } catch {
          effectiveOffset = 0;
        }
      }

      // 2. Resolve download URLs from Apple
      // Prepare output directory (com.apple.recovery.boot is what OpenCore expects)
      const BOARD_IDS: Record<string, { boardId: string; mlb: string }> = {
        '16':    { boardId: 'Mac-827FAC58A8FDFA22', mlb: '00000000000000000' },
        '15':    { boardId: 'Mac-827FAC58A8FDFA22', mlb: '00000000000000000' },
        '14':    { boardId: 'Mac-827FAC58A8FDFA22', mlb: '00000000000000000' },
        '13':    { boardId: 'Mac-4B682C642B45593E', mlb: '00000000000000000' },
        '12':    { boardId: 'Mac-FFE5EF870D7BA81A', mlb: '00000000000000000' },
        '11':    { boardId: 'Mac-42FD25EABCABB274', mlb: '00000000000000000' },
        '10.15': { boardId: 'Mac-00BE6ED71E35EB86', mlb: '00000000000000000' },
        '10.14': { boardId: 'Mac-7BA5B2D9BE2258A1', mlb: 'F4K10270Q2J3WLVAD' },
        '10.13': { boardId: 'Mac-BE088AF8C5EB4FA2', mlb: 'F17M0XA0H7G3F91AD' },
      };

      let versionKey = '15';
      for (const key of Object.keys(BOARD_IDS)) {
        if (macOSVersion.includes(key)) { versionKey = key; break; }
        const major = key.split('.')[0];
        if (macOSVersion.includes(` ${major}`)) { versionKey = key; break; }
      }
      const { boardId } = BOARD_IDS[versionKey];

      registry.updateProgress(token.taskId, {
        kind: 'recovery-download',
        percent: effectiveOffset > 0 ? 5 : 2,
        status: effectiveOffset > 0 ? 'Resuming cached download...' : `Requesting official recovery for ${macOSVersion}...`,
        sourceId: 'apple_primary',
        trustLevel: 'official',
        bytesDownloaded: effectiveOffset,
        dmgDest,
        clDest
      });

      const urls = await withTimeout(appleRecoveryQuery(boardId), 30_000, 'appleRecoveryQuery');

      // 3. Download to cache
      await downloadFileWithProgress(urls.dmgUrl, dmgDest, (downloaded, total) => {
        const pct = total > 0 ? 8 + Math.round((downloaded / total) * 82) : 8;
        registry.updateProgress(token.taskId, {
          kind: 'recovery-download',
          percent: pct,
          bytesDownloaded: downloaded,
          status: `Downloading BaseSystem.dmg — ${formatBytes(downloaded)} / ${total > 0 ? formatBytes(total) : '...'}`,
          sourceId: 'apple_primary',
          trustLevel: 'official',
          dmgDest,
          clDest
        });
      }, effectiveOffset, () => token.check(), buildAppleRecoveryDownloadHeaders(urls.dmgToken));

      if (urls.chunklistUrl && !fs.existsSync(clDest)) {
        await downloadFileWithProgress(urls.chunklistUrl, clDest, () => {}, 0, () => token.check(), buildAppleRecoveryDownloadHeaders(urls.chunklistToken));
      }

      // 4. Finalise cache and copy to EFI
      const finalSize = fs.statSync(dmgDest).size;
      cacheManager.saveMetadata({
        version: macOSVersion,
        size: finalSize,
        sourceId: 'apple_primary',
        trustLevel: 'local_cached_official',
        timestamp: Date.now(),
        dmgPath: dmgDest,
        chunklistPath: clDest,
        isPartial: false
      });

      const finalRecoveryDir = path.resolve(targetPath, 'com.apple.recovery.boot');
      if (!fs.existsSync(finalRecoveryDir)) fs.mkdirSync(finalRecoveryDir, { recursive: true });
      fs.copyFileSync(dmgDest, path.join(finalRecoveryDir, 'BaseSystem.dmg'));
      if (fs.existsSync(clDest)) fs.copyFileSync(clDest, path.join(finalRecoveryDir, 'BaseSystem.chunklist'));

      registry.complete(token.taskId);
      const finalClDest = path.join(finalRecoveryDir, 'BaseSystem.chunklist');
      return { dmgPath: path.join(finalRecoveryDir, 'BaseSystem.dmg'), recoveryDir: finalRecoveryDir, clDest: fs.existsSync(finalClDest) ? finalClDest : undefined };

    } catch (e: any) {
      try {
        if (fs.existsSync(dmgDest)) {
          const partialSize = fs.statSync(dmgDest).size;
          if (partialSize > 0) {
            cacheManager.saveMetadata({
              version: macOSVersion,
              size: partialSize,
              sourceId: 'apple_primary',
              trustLevel: 'official',
              timestamp: Date.now(),
              dmgPath: dmgDest,
              chunklistPath: fs.existsSync(clDest) ? clDest : undefined,
              isPartial: true,
            });
          }
        }
      } catch {}
      const classified = classifyError(e);
      rememberFailureContext({
        trigger: 'recovery_failure',
        message: classified.message,
        detail: classified.explanation,
        code: classified.category,
      });
      log('ERROR', 'recovery', 'Recovery acquisition failed', { error: classified.explanation });
      if (!token.aborted) registry.fail(token.taskId, classified.message);
      else registry.cancel(token.taskId);
      throw createClassifiedIpcError(classified, e);
    }
  });

  // Manual recovery import
  ipcHandle('recovery:import', async (_event: Electron.IpcMainInvokeEvent, targetPath: string, macOSVersion: string) => {
    const { dialog } = require('electron');
    const result = await dialog.showOpenDialog({
      title: `Select BaseSystem.dmg for ${macOSVersion}`,
      filters: [{ name: 'Apple Recovery Image', extensions: ['dmg'] }],
      properties: ['openFile']
    });

    if (result.canceled || result.filePaths.length === 0) return null;

    const importedPath = result.filePaths[0];
    const filename = path.basename(importedPath);
    
    // 1. Filename validation (must be BaseSystem.dmg)
    if (!filename.toLowerCase().includes('basesystem') || !filename.toLowerCase().endsWith('.dmg')) {
      throw new Error(`Invalid filename: "${filename}". macOS recovery images must be named "BaseSystem.dmg".`);
    }

    const stat = fs.statSync(importedPath);
    
    // 2. Strict size check (Recovery images are typically 450MB - 900MB)
    if (stat.size < 350 * 1024 * 1024 || stat.size > 1024 * 1024 * 1024) {
      throw new Error(`The selected file does not appear to be a valid macOS recovery image (detected size: ${formatBytes(stat.size)}). A valid image should be between 400MB and 950MB.`);
    }

    const versionDir = cacheManager.getVersionDir(macOSVersion, 'manual_import');
    const dmgDest = path.join(versionDir, 'BaseSystem.dmg');
    
    log('INFO', 'recovery', 'Importing manual recovery image', { from: importedPath, to: dmgDest });
    
    fs.copyFileSync(importedPath, dmgDest);
    
    cacheManager.saveMetadata({
      version: macOSVersion,
      size: stat.size,
      sourceId: 'manual_import',
      trustLevel: 'manual_user_provided',
      timestamp: Date.now(),
      dmgPath: dmgDest,
      isPartial: false
    });

    const finalRecoveryDir = path.resolve(targetPath, 'com.apple.recovery.boot');
    if (!fs.existsSync(finalRecoveryDir)) fs.mkdirSync(finalRecoveryDir, { recursive: true });
    fs.copyFileSync(dmgDest, path.join(finalRecoveryDir, 'BaseSystem.dmg'));

    return { dmgPath: path.join(finalRecoveryDir, 'BaseSystem.dmg'), recoveryDir: finalRecoveryDir };
  });

  // Get cached recovery info
  ipcHandle('recovery:get-cached-info', async (_event: Electron.IpcMainInvokeEvent, version: string) => {
    return await cacheManager.getCachedAsset(version);
  });

  // Clear cache for a version
  ipcHandle('recovery:clear-cache', async (_event: Electron.IpcMainInvokeEvent, version: string) => {
    const { dialog } = require('electron');
    const choice = await dialog.showMessageBox({
      type: 'warning',
      buttons: ['Cancel', 'Clear Current Version', 'Clear All Recovery Cache'],
      defaultId: 0,
      title: 'Clear Recovery Cache',
      message: 'Are you sure you want to clear the recovery cache?',
      detail: 'This will delete local recovery images. You will need to re-download or re-import them.'
    });

    if (choice.response === 1) {
      await cacheManager.clearCache(version);
      return true;
    } else if (choice.response === 2) {
      await cacheManager.clearCache();
      return true;
    }
    return false;
  });

  // Preflight check (legacy — used by existing PrereqStep banner)
  ipcHandle('run-preflight', () => runPreflight());

  // Extended prechecks — used by PrecheckStep (Phase 3)
  ipcHandle('run-prechecks', async () => {
    // Run all precheck items concurrently, each isolated from failures.
    const [preflight, usbList] = await Promise.all([
      runPreflight().catch(() => null),
      diskOps.listUsbDevices().catch(() => [] as { name: string; device: string; size: string }[]),
    ]);

    // Network check: lightweight attempt to reach Apple CDN
    let networkOk = false;
    try {
      await new Promise<void>((resolve, reject) => {
        const req = https.request({ hostname: 'osrecovery.apple.com', path: '/', method: 'HEAD', timeout: 5000 }, res => {
          res.destroy();
          resolve();
        });
        req.on('error', reject);
        req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
        req.end();
      });
      networkOk = true;
    } catch { networkOk = false; }

    const freeSpaceMB = preflight?.freeSpaceMB ?? Infinity;

    // Platform-aware admin check:
    // macOS grants disk access per-operation via system prompts (osascript/diskutil
    // elevation) — the app does not need to run as root. Reporting adminPrivileges=false
    // on a normal macOS launch would produce a misleading block state.
    let adminPrivileges: boolean;
    let adminNote: string | null = null;
    if (process.platform === 'darwin') {
      adminPrivileges = true;
      adminNote = 'macOS grants disk access per-operation via system prompts — no persistent admin session required';
    } else {
      adminPrivileges = preflight?.adminPrivileges ?? false;
    }

    // Determine firmware detection availability per platform
    const firmwareDetectionAvailable = process.platform !== 'darwin';

    // Collect blocking binary failures from preflight
    const missingBinaries = preflight
      ? Object.entries(preflight.binaries).filter(([, ok]) => !ok).map(([bin]) => bin)
      : [];

    return {
      platform: process.platform,
      adminPrivileges,
      adminNote,
      freeSpaceMB,
      networkOk,
      usbDetected: usbList.length > 0,
      firmwareDetectionAvailable,
      missingBinaries,
    };
  });

  // ── Prevention Layer IPC ──────────────────────────────────────────────────

  ipcHandle('preflight:run', async (_event: Electron.IpcMainInvokeEvent, kextNames: string[]) => {
    log('INFO', 'preflight', `Running preflight checks for ${kextNames.length} kexts`);
    const report = await runPreflightChecks(kextNames, KEXT_REGISTRY);
    log('INFO', 'preflight', `Preflight complete: ${report.confidence}`, {
      warnings: report.warnings.length,
      blockers: report.blockers.length,
      kextsChecked: report.kextAvailability.length,
      networkOverall: report.network.overall,
    });
    return report;
  });

  ipcHandle('preflight:record-failure', async (_event: Electron.IpcMainInvokeEvent, code: string, message: string) => {
    const entry = recordFailure(code, message);
    log('INFO', 'preflight', `Failure recorded: ${code} (count: ${entry.count})`);
    return entry;
  });

  ipcHandle('preflight:should-skip-retry', async (_event: Electron.IpcMainInvokeEvent, code: string) => {
    return shouldSkipRetry(code);
  });

  ipcHandle('preflight:failure-memory', async () => {
    return getFailureMemory();
  });

  ipcHandle('preflight:clear-memory', async () => {
    clearFailureMemory();
    return true;
  });

  // ── Deterministic Layer ─────────────────────────────────────────────────────

  ipcHandle('deterministic:simulate-build', async (_event: Electron.IpcMainInvokeEvent, kextNames: string[], ssdtNames: string[], smbios: string) => {
    return simulateBuild(kextNames, ssdtNames, KEXT_REGISTRY, smbios);
  });

  ipcHandle('deterministic:dry-run-recovery', async (_event: Electron.IpcMainInvokeEvent, targetOS: string, smbios: string) => {
    return dryRunRecovery(targetOS, smbios);
  });

  ipcHandle('deterministic:verify-build-state', async (_event: Electron.IpcMainInvokeEvent, efiPath: string, requiredKexts: string[]) => {
    return verifyBuildState(efiPath, requiredKexts);
  });

  ipcHandle('deterministic:verify-efi-success', async (_event: Electron.IpcMainInvokeEvent, efiPath: string, requiredKexts: string[]) => {
    return verifyEfiBuildSuccess(efiPath, requiredKexts);
  });

  ipcHandle('deterministic:verify-recovery-success', async (_event: Electron.IpcMainInvokeEvent, recoveryDir: string) => {
    return verifyRecoverySuccess(recoveryDir);
  });

  ipcHandle('safe-simulation:run', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile): Promise<SafeSimulationResult> => {
    try {
      const result = await runSafeSimulation(profile, {
        userDataPath: app.getPath('userData'),
        createEfiWorkspace: async (workspacePath, simulationProfile) => {
          await createEfiStructure(workspacePath, simulationProfile);
        },
        validateEfi: async (efiPath, simulationProfile) => validateEfi(efiPath, simulationProfile, kextSources),
        buildCompatibilityMatrix,
        simulateBuild: async (simulationProfile) => {
          const { kexts, ssdts } = getRequiredResources(simulationProfile);
          return simulateBuild(kexts, ssdts, KEXT_REGISTRY, simulationProfile.smbios);
        },
        dryRunRecovery,
        kextRegistry: KEXT_REGISTRY,
      });

      log('INFO', 'safe-simulation', 'Safe simulation completed', {
        targetOS: profile.targetOS,
        workspacePath: result.workspacePath,
        validationOverall: result.validationSummary.overall,
        blockerCount: result.blockers.length,
        warningCount: result.warnings.length,
      });

      return result;
    } catch (error: any) {
      rememberFailureContext({
        trigger: 'simulation_failure',
        message: error?.message ?? 'Safe simulation failed.',
        detail: error?.stack ?? null,
      });
      throw error;
    }
  });

  ipcHandle('resource-plan:get', async (_event: Electron.IpcMainInvokeEvent, profile: HardwareProfile, efiPath?: string | null): Promise<ResourcePlan> => {
    const resolvedEfiPath = efiPath ? path.resolve(efiPath) : null;
    const validationResult = resolvedEfiPath && fs.existsSync(resolvedEfiPath)
      ? await validateEfi(resolvedEfiPath, profile, kextSources).catch(() => null)
      : null;

    return buildResourcePlan({
      profile,
      validationResult,
      kextRegistry: KEXT_REGISTRY,
      kextSources,
    });
  });

  // Diagnostics snapshot — used by CopyDiagnosticsButton
  ipcHandle('get-diagnostics', async () => {
    return buildCurrentDiagnosticsSnapshot();
  });

  ipcHandle('log:save-support-log', async (_event: Electron.IpcMainInvokeEvent, extraContext?: string | null) => {
    return saveSupportLogToDesktop(extraContext ?? null);
  });

  ipcHandle('log:ui-event', async (_event: Electron.IpcMainInvokeEvent, eventName: string, detail?: Record<string, unknown> | null) => {
    logger?.timeline('ui_event', undefined, {
      event: eventName,
      ...(detail ?? {}),
    });
    return true;
  });

  ipcHandle('task:cancel', (_e: Electron.IpcMainInvokeEvent, taskId: string) => registry.cancel(taskId));
  ipcHandle('task:list',   () => registry.list());

  // Disk info / safety query — does NOT set lastSelectedDisk; that is only
  // set in flash-usb after the user has committed to a specific drive.
  // get-disk-info is also called during drive list enrichment for every drive,
  // so setting lastSelectedDisk here would overwrite it with the wrong device.
  ipcHandle('get-disk-info', async (_event: Electron.IpcMainInvokeEvent, device: string) => {
    return withTimeout(diskOps.getDiskInfo(device), 30_000, 'getDiskInfo');
  });

  // Open folder
  ipcHandle('open-folder', (_event: Electron.IpcMainInvokeEvent, folderPath: string) => shell.openPath(folderPath));

  // Log file path + tail + session id
  ipcHandle('get-log-path',       () => path.join(app.getPath('userData'), 'app.log'));
  ipcHandle('log:get-tail',       (_e: Electron.IpcMainInvokeEvent, n: number) => logger.readTail(n));
  ipcHandle('log:get-ops-tail',   (_e: Electron.IpcMainInvokeEvent, n: number) => logger.readOpsTail(n));
  ipcHandle('log:get-session-id', () => logger.sessionId);
  ipcHandle('log:clear', () => {
    try {
      if (fs.existsSync(logger.logPath)) fs.writeFileSync(logger.logPath, '', 'utf-8');
      return true;
    } catch { return false; }
  });

  // Renderer crash / error reporting — called from preload's window.onerror
  ipcMain.on('renderer-error', (_e, payload: { type: string; message: string; stack?: string; source?: string; line?: number }) => {
    if (payload.type === 'preload-ping') {
      startupLifecycle.preloadReady = true;
      log('INFO', 'renderer', payload.message);
    } else {
      log('ERROR', 'renderer', `[${payload.type}] ${payload.message}`, {
        stack: payload.stack,
        source: payload.source,
        line: payload.line,
      });
      rememberFailureContext({
        trigger: 'unexpected_runtime_error',
        message: payload.message,
        detail: payload.stack ?? payload.source ?? null,
        channel: 'renderer-error',
        code: payload.type,
      });
    }
    logger?.flush();
  });

  ipcHandle('renderer:ready', async () => {
    startupLifecycle.rendererReady = true;
    startupLifecycle.mainFrameLoadRetries = 0;
    clearStartupReadyTimer();
    log('INFO', 'startup', 'renderer-ready handshake received', {
      preloadReady: startupLifecycle.preloadReady,
    });
    return true;
  });

  // Issue reporter
  ipcHandle('report-issue', async (_event: Electron.IpcMainInvokeEvent, extraContext?: string | null) => {
    const snapshot = buildCurrentDiagnosticsSnapshot();
    const draft = buildIssueReportDraft(snapshot, extraContext ?? null);
    const baseUrl = 'https://github.com/redpersongpt/macOS-One-Click/issues/new';
    // Only put the short title in the URL — body goes to clipboard to avoid
    // URL-length truncation issues across platforms/browsers.
    const url = `${baseUrl}?title=${encodeURIComponent(draft.title)}&labels=bug`;
    const success = await openIssueReportUrl(url, (targetUrl) => shell.openExternal(targetUrl));
    if (!success) {
      log('WARN', 'app', 'Failed to open browser automatically', { issueTrigger: draft.trigger });
    }
    return { success, body: draft.body, baseUrl };
  });

  ipcHandle('app:open-latest-release', async () => {
    const url = 'https://github.com/redpersongpt/macOS-One-Click/releases/latest';
    if (!isSafeExternalTarget(url)) {
      throw new Error('Latest release URL is not a safe external target.');
    }
    await shell.openExternal(url);
    return true;
  });

  log('INFO', 'app', 'App ready', { version: app.getVersion(), platform: process.platform, packaged: app.isPackaged });

  mainWindow?.on('close', (e) => {
    const activeTasks = registry.list().filter(t => t.status === 'running');
    const dangerous = activeTasks.some(t => t.kind === 'usb-flash' || t.kind === 'partition-prep');

    if (dangerous) {
      e.preventDefault();
      log('WARN', 'app', 'App close prevented: destructive task in progress');
      sendAlert('CRITICAL: Writing to disk in progress. Closing now could corrupt your drive. Wait for completion or an error to occur.');
    }
  });

  mainWindow?.on('closed', () => {
    clearStartupReadyTimer();
    mainWindow = null;
  });

});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});

app.on('activate', () => {
  if (mainWindow === null) createWindow();
});
