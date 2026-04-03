import { app, BrowserWindow, dialog, ipcMain, shell } from 'electron';
import path from 'path';
import crypto from 'node:crypto';
import { fileURLToPath, pathToFileURL } from 'url';
import { exec, spawn } from 'child_process';
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
import { type KextRegistryEntry } from './kextSourcePolicy.js';
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
import { detectCpuGeneration, detectArchitecture, mapDetectedToProfile } from './hardwareMapper.js';
import { detectElevationMethod, canElevate, elevationDescription } from './linuxElevate.js';
import { getSsdtSourcePolicy, getUnsupportedSsdtRequests } from './ssdtSourcePolicy.js';
import { createLogger } from './logger.js';
import {
  createClassifiedIpcError,
  type ClassifiedError,
} from './errorMessaging.js';
import { getRecoveryBoardId } from './recoveryBoardId.js';
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
import { detectHardware, buildPartialDetectedHardware } from './hardwareDetect.js';
import { interpretHardware, type HardwareInterpretation } from './hardwareInterpret.js';
import { canRestoreLatestScannedArtifact, reconcileHardwareScanProfile } from './hardwareProfileState.js';
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
  compareReleaseVersions,
  createBaseAppUpdateState,
  createPersistedAppUpdateSession,
  isInstallerResidueEntryName,
  normalizeReleaseVersion,
  pickReleaseAssetForPlatform,
  reconcilePersistedAppUpdateState,
  type AppUpdateResultMarker,
  type AppUpdateState,
  type LatestReleaseInfo,
  type PersistedAppUpdateSession,
  type ReleaseAssetInfo,
} from './appUpdater.js';
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
const PRODUCT_DISPLAY_NAME = 'OpCore-OneClick';
const PRODUCT_HTTP_NAME = 'OpCore-OneClick';
const LEGACY_USER_DATA_DIR_NAME = 'macOS-OneClick';
const REPO_SLUG = 'redpersongpt/OpCore-OneClick';
const REPO_WEB_BASE_URL = `https://github.com/${REPO_SLUG}`;

app.setName(PRODUCT_DISPLAY_NAME);
app.setPath('userData', path.resolve(app.getPath('appData'), LEGACY_USER_DATA_DIR_NAME));

const execPromise = util.promisify(exec);

const VALID_DEVICE_PATTERN = process.platform === 'win32'
  ? /^\\\\\.\\PhysicalDrive\d+$|^PHYSICALDRIVE\d+$/i
  : process.platform === 'darwin'
  ? /^\/dev\/disk\d+$/
  : /^\/dev\/(sd[a-z]|nvme\d+n\d+|loop\d+)$/;

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
        if (!meta.isPartial && (actualSize < 350 * 1024 * 1024 || actualSize > 1024 * 1024 * 1024)) {
          log('WARN', 'cache', 'Cached asset failed size validation, downgrading to partial', { actualSize, version });
          meta.isPartial = true; 
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
  if (bytes <= 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
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
    
    let currentOffset = startOffset;
    let retries = 0;
    const MAX_RETRIES = 5;

    function fetchUrl(urlStr: string, redirects = 0): void {
      if (redirects > 10) { reject(new Error('Too many redirects')); return; }
      
      const handleError = (e: Error) => {
        const msg = e.message.toLowerCase();
        const isNetworkError = msg.includes('econnreset') || msg.includes('socket hang up') || msg.includes('etimedout') || msg.includes('stalled') || msg.includes('timed out');
        if (isNetworkError && retries < MAX_RETRIES) {
          retries++;
          log('WARN', 'download', `Network drop (${e.message}), auto-retrying (${retries}/${MAX_RETRIES})...`, { url: urlStr, currentOffset });
          setTimeout(() => fetchUrl(urlStr, redirects), 2000);
          return;
        }
        reject(e);
      };

      const parsedUrl = new URL(urlStr);
      const lib = parsedUrl.protocol === 'https:' ? https : http;
      const headers: Record<string, string | number> = {
        'User-Agent': 'InternetRecovery/1.0',
        ...(extraHeaders ?? {}),
      };
      if (currentOffset > 0) {
        headers['Range'] = `bytes=${currentOffset}-`;
      }
      const options = {
        hostname: parsedUrl.hostname,
        path: parsedUrl.pathname + parsedUrl.search,
        headers
      };
      const req = lib.get(options as any, (res: any) => {
        if (res.statusCode === 301 || res.statusCode === 302 || res.statusCode === 307 || res.statusCode === 308) {
          clearTimeout(connectTimer);
          fetchUrl(res.headers.location!, redirects + 1);
          return;
        }
        const isResume = res.statusCode === 206;
        const isFull   = res.statusCode === 200;
        if (!isResume && !isFull) {
          clearTimeout(connectTimer);
          if (res.statusCode === 416 && currentOffset > 0) {
            log('WARN', 'download', 'HTTP 416 Requested Range Not Satisfiable during resume. Restarting from scratch.', { url: urlStr, offset: currentOffset });
            try { if (fs.existsSync(dest)) fs.unlinkSync(dest); } catch {}
            currentOffset = 0;
            fetchUrl(urlStr, redirects);
            return;
          }
          reject(new Error(`HTTP ${res.statusCode} downloading ${urlStr}`));
          return;
        }
        const effectiveOffset = isResume ? currentOffset : 0;
        const contentLength = parseInt(res.headers['content-length'] || '0', 10);
        const total = contentLength + effectiveOffset;
        let downloaded = effectiveOffset;
        let rejected = false;
        
        const writeFlags = (currentOffset > 0 && isResume) ? { flags: 'a' } : { flags: 'w' };
        const file = fs.createWriteStream(dest, writeFlags);
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
            handleError(new Error(`Download stalled after ${INACTIVITY_TIMEOUT_MS / 1000}s with no progress: ${urlStr}`));
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
          currentOffset = downloaded;
          armInactivityTimer();
          onProgress(downloaded, total);
        });
        res.pipe(file);
        file.on('finish', () => {
          if (rejected) return;
          clearInactivityTimer();
          file.close(() => resolve());
        });
        res.on('error', (e: Error) => {
          clearInactivityTimer();
          if (!rejected) { 
            rejected = true; 
            handleError(e); 
          }
        });
        file.on('error', (e: Error) => {
          clearInactivityTimer();
          if (!rejected) { 
            rejected = true; 
            handleError(e); 
          }
        });
      });
      const connectTimer = setTimeout(() => {
        req.destroy(new Error(`Timed out after ${CONNECT_TIMEOUT_MS / 1000}s connecting to ${urlStr}`));
      }, CONNECT_TIMEOUT_MS);
      req.on('error', (error: Error) => {
        clearTimeout(connectTimer);
        handleError(error);
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
type KextFetchSource = 'github' | 'embedded' | 'direct' | 'failed';

let failedKexts: Array<{ name: string; repo: string; error: string }> = [];
let kextSources: Record<string, KextFetchSource> = {};
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
  // Codeless kexts (e.g. AppleMCEReporterDisabler) have Info.plist but no Contents/MacOS
  const infoPlist = path.join(kextPath, 'Contents', 'Info.plist');
  const macosDir = path.join(kextPath, 'Contents', 'MacOS');
  if (!fs.existsSync(macosDir)) {
    try {
      const content = fs.readFileSync(infoPlist, 'utf-8');
      return content.includes('<plist') && content.includes('CFBundleIdentifier');
    } catch { return false; }
  }
  // Binary kexts: Contents/MacOS should have at least one file > 1KB
  try {
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

const REPO_RELEASE_API_PATH = `/repos/${REPO_SLUG}/releases/latest`;
const APP_UPDATE_RESULT_FILE = path.resolve(app.getPath('userData'), 'app-update-result.json');
const APP_UPDATE_SESSION_FILE = path.resolve(app.getPath('userData'), 'app-update-session.json');

function readAppUpdateResultMarker(): AppUpdateResultMarker | null {
  if (!fs.existsSync(APP_UPDATE_RESULT_FILE)) return null;
  try {
    return JSON.parse(fs.readFileSync(APP_UPDATE_RESULT_FILE, 'utf8')) as AppUpdateResultMarker;
  } catch {
    try { fs.rmSync(APP_UPDATE_RESULT_FILE, { force: true }); } catch {}
    return null;
  }
}

function consumeAppUpdateResultMarker(): AppUpdateResultMarker | null {
  const marker = readAppUpdateResultMarker();
  if (!marker) return null;
  try { fs.rmSync(APP_UPDATE_RESULT_FILE, { force: true }); } catch {}
  return marker;
}

function clearAppUpdateResultMarker(): void {
  try { fs.rmSync(APP_UPDATE_RESULT_FILE, { force: true }); } catch {}
}

function readPersistedAppUpdateSession(): PersistedAppUpdateSession | null {
  if (!fs.existsSync(APP_UPDATE_SESSION_FILE)) return null;
  try {
    return JSON.parse(fs.readFileSync(APP_UPDATE_SESSION_FILE, 'utf8')) as PersistedAppUpdateSession;
  } catch {
    try { fs.rmSync(APP_UPDATE_SESSION_FILE, { force: true }); } catch {}
    return null;
  }
}

function writePersistedAppUpdateSession(session: PersistedAppUpdateSession): void {
  fs.writeFileSync(APP_UPDATE_SESSION_FILE, JSON.stringify(session), 'utf8');
}

function clearPersistedAppUpdateSession(): void {
  try { fs.rmSync(APP_UPDATE_SESSION_FILE, { force: true }); } catch {}
}

function buildAppUpdateCleanupTargets(downloadedPath: string): string[] {
  const userDataPath = app.getPath('userData');
  const targets = new Set<string>([
    path.resolve(userDataPath, 'Recovery_Cache'),
    path.resolve(userDataPath, 'OpenCore_Cache'),
    path.resolve(userDataPath, 'safe-simulations'),
    path.resolve(userDataPath, 'hardware-profiles'),
    path.resolve(userDataPath, 'app_state.json'),
    path.resolve(userDataPath, 'bios_session.json'),
    path.resolve(userDataPath, 'startup-crash.log'),
    path.resolve(userDataPath, 'session.lock'),
    path.resolve(userDataPath, 'app.log'),
    path.resolve(userDataPath, 'operations.log'),
    path.resolve(downloadedPath),
    path.resolve(path.dirname(downloadedPath)),
  ]);

  try {
    for (const entry of fs.readdirSync(userDataPath, { withFileTypes: true })) {
      if (!isInstallerResidueEntryName(entry.name)) continue;
      targets.add(path.resolve(userDataPath, entry.name));
    }
  } catch {}

  return Array.from(targets);
}

function cleanupStaleUpdateWorkerScripts(): void {
  try {
    const workerDir = path.resolve(app.getPath('userData'), 'update-worker');
    if (!fs.existsSync(workerDir)) return;
    for (const entry of fs.readdirSync(workerDir)) {
      try { fs.rmSync(path.resolve(workerDir, entry), { force: true }); } catch {}
    }
    try { fs.rmdirSync(workerDir); } catch {}
  } catch {}
}

function createInitialAppUpdateState(): AppUpdateState {
  const supported = process.platform === 'win32' || process.platform === 'linux';
  const session = readPersistedAppUpdateSession();
  const marker = readAppUpdateResultMarker();
  const resolution = reconcilePersistedAppUpdateState({
    currentVersion: app.getVersion(),
    supported,
    session,
    resultMarker: marker,
    downloadedFileExists: session ? fs.existsSync(path.resolve(session.downloadedPath)) : false,
  });

  if (resolution.clearSession) clearPersistedAppUpdateSession();
  if (resolution.clearResultMarker) clearAppUpdateResultMarker();
  cleanupStaleUpdateWorkerScripts();

  return resolution.state;
}

let appUpdateState: AppUpdateState = createInitialAppUpdateState();

function reconcilePersistedUpdaterState(): AppUpdateState {
  const supported = process.platform === 'win32' || process.platform === 'linux';
  const session = readPersistedAppUpdateSession();
  const marker = consumeAppUpdateResultMarker();
  const resolution = reconcilePersistedAppUpdateState({
    currentVersion: app.getVersion(),
    supported,
    session,
    resultMarker: marker,
    downloadedFileExists: session ? fs.existsSync(path.resolve(session.downloadedPath)) : false,
  });

  if (resolution.clearSession) clearPersistedAppUpdateSession();
  if (resolution.clearResultMarker) clearAppUpdateResultMarker();

  appUpdateState = resolution.state;
  return appUpdateState;
}

function buildPersistedAppUpdateSession(phase: 'downloaded' | 'install-requested'): PersistedAppUpdateSession {
  if (!appUpdateState.downloadedPath || !appUpdateState.assetName || !appUpdateState.latestVersion) {
    throw new Error('No downloaded update is ready to persist.');
  }

  return createPersistedAppUpdateSession({
    phase,
    platform: process.platform,
    currentVersion: app.getVersion(),
    latestVersion: appUpdateState.latestVersion,
    releaseUrl: appUpdateState.releaseUrl,
    releaseNotes: appUpdateState.releaseNotes,
    assetName: appUpdateState.assetName,
    assetSize: appUpdateState.assetSize,
    downloadedPath: path.resolve(appUpdateState.downloadedPath),
    requestedAt: phase === 'install-requested' ? new Date().toISOString() : null,
  });
}

function setAppUpdateState(patch: Partial<AppUpdateState>): AppUpdateState {
  appUpdateState = {
    ...appUpdateState,
    ...patch,
    currentVersion: app.getVersion(),
    supported: process.platform === 'win32' || process.platform === 'linux',
  };
  return appUpdateState;
}

async function fetchLatestAppRelease(): Promise<LatestReleaseInfo> {
  return await retryWithBackoff(() => new Promise((resolve, reject) => {
    const req = https.request({
      hostname: 'api.github.com',
      path: REPO_RELEASE_API_PATH,
      method: 'GET',
      headers: {
        'User-Agent': `${PRODUCT_HTTP_NAME}-Updater/1.0`,
        Accept: 'application/vnd.github+json',
      },
      timeout: 15_000,
    }, (res) => {
      let data = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => { data += chunk; });
      res.on('end', () => {
        if (!res.statusCode || res.statusCode < 200 || res.statusCode >= 300) {
          reject(new Error(`Failed to query latest release (HTTP ${res.statusCode ?? 'unknown'})`));
          return;
        }
        try {
          resolve(JSON.parse(data));
        } catch {
          reject(new Error('Latest release response could not be parsed.'));
        }
      });
    });
    req.on('timeout', () => req.destroy(new Error('Latest release query timed out.')));
    req.on('error', reject);
    req.end();
  }), 3, 'fetchLatestAppRelease');
}

function selectLatestReleaseAsset(release: LatestReleaseInfo): ReleaseAssetInfo | null {
  return pickReleaseAssetForPlatform(process.platform, release.assets ?? []);
}

async function checkForAppUpdates(): Promise<AppUpdateState> {
  reconcilePersistedUpdaterState();
  if (appUpdateState.restartRequired) {
    return setAppUpdateState({ checking: false, available: false, error: null });
  }
  if (appUpdateState.readyToInstall && appUpdateState.downloadedPath && fs.existsSync(appUpdateState.downloadedPath)) {
    return setAppUpdateState({
      checking: false,
      installing: false,
      lastCheckedAt: Date.now(),
    });
  }
  setAppUpdateState({ checking: true, error: null });
  const checkedAt = Date.now();
  try {
    const release = await fetchLatestAppRelease();
    const asset = selectLatestReleaseAsset(release);
    const available = !!asset && compareReleaseVersions(release.tag_name, app.getVersion()) > 0;
    return setAppUpdateState({
      checking: false,
      installing: false,
      lastCheckedAt: checkedAt,
      available,
      latestVersion: release.tag_name ?? null,
      releaseUrl: release.html_url ?? `${REPO_WEB_BASE_URL}/releases/latest`,
      releaseNotes: release.body ?? null,
      assetName: asset?.name ?? null,
      assetSize: typeof asset?.size === 'number' ? asset.size : null,
      totalBytes: typeof asset?.size === 'number' ? asset.size : null,
      downloadedBytes: available && appUpdateState.readyToInstall ? appUpdateState.downloadedBytes : 0,
      downloadedPath: available && appUpdateState.readyToInstall ? appUpdateState.downloadedPath : null,
      readyToInstall: available && appUpdateState.readyToInstall && appUpdateState.assetName === asset?.name,
      restartRequired: appUpdateState.restartRequired,
      error: asset ? null : 'No installable update asset is published for this platform yet.',
    });
  } catch (error: any) {
    return setAppUpdateState({
      checking: false,
      installing: false,
      lastCheckedAt: checkedAt,
      available: false,
      latestVersion: null,
      releaseUrl: `${REPO_WEB_BASE_URL}/releases/latest`,
      releaseNotes: null,
      assetName: null,
      assetSize: null,
      downloadedBytes: 0,
      totalBytes: null,
      downloadedPath: null,
      readyToInstall: false,
      restartRequired: false,
      error: error?.message ?? 'Could not check for updates.',
    });
  }
}

async function downloadLatestAppUpdate(): Promise<AppUpdateState> {
  reconcilePersistedUpdaterState();
  if (appUpdateState.downloading) return appUpdateState;
  if (!appUpdateState.supported) {
    throw new Error('In-app updates are not supported on this platform.');
  }

  let release: LatestReleaseInfo;
  if (!appUpdateState.latestVersion || !appUpdateState.assetName || !appUpdateState.releaseUrl) {
    await checkForAppUpdates();
  }
  release = await fetchLatestAppRelease();
  const asset = selectLatestReleaseAsset(release);
  if (!asset?.browser_download_url || !asset.name) {
    throw new Error('No installable update asset is available for this platform.');
  }
  if (compareReleaseVersions(release.tag_name, app.getVersion()) <= 0) {
    return setAppUpdateState({
      available: false,
      installing: false,
      latestVersion: release.tag_name,
      releaseUrl: release.html_url,
      releaseNotes: release.body ?? null,
      assetName: asset.name,
      assetSize: asset.size ?? null,
      downloadedBytes: 0,
      totalBytes: asset.size ?? null,
      downloadedPath: null,
      readyToInstall: false,
      restartRequired: false,
      error: null,
    });
  }

  const downloadsDir = path.resolve(app.getPath('downloads'), 'OpCore-OneClick-Updates');
  fs.mkdirSync(downloadsDir, { recursive: true });
  const finalPath = path.resolve(downloadsDir, asset.name);
  const tempPath = `${finalPath}.download`;

  setAppUpdateState({
    checking: false,
    downloading: true,
    installing: false,
    available: true,
    latestVersion: release.tag_name,
    releaseUrl: release.html_url,
    releaseNotes: release.body ?? null,
    assetName: asset.name,
    assetSize: asset.size ?? null,
    downloadedBytes: 0,
    totalBytes: asset.size ?? null,
    downloadedPath: null,
    readyToInstall: false,
    restartRequired: false,
    error: null,
  });

  try {
    let startOffset = 0;
    if (fs.existsSync(tempPath)) {
      startOffset = fs.statSync(tempPath).size;
    }

    await downloadFileWithProgress(
      asset.browser_download_url,
      tempPath,
      (downloaded, total) => {
        setAppUpdateState({ 
          downloadedBytes: downloaded, 
          totalBytes: Number.isFinite(total) && total > 0 ? total : asset.size ?? null 
        });
      },
      startOffset,
      undefined,
      { 'User-Agent': `${PRODUCT_HTTP_NAME}-Updater/1.0` }
    );

    fs.rmSync(finalPath, { force: true });
    fs.renameSync(tempPath, finalPath);
    if (process.platform === 'linux' && finalPath.endsWith('.AppImage')) {
      fs.chmodSync(finalPath, 0o755);
    }
    const nextState = setAppUpdateState({
      downloading: false,
      installing: false,
      downloadedPath: finalPath,
      downloadedBytes: appUpdateState.totalBytes ?? appUpdateState.downloadedBytes,
      readyToInstall: true,
      restartRequired: false,
      error: null,
    });
    writePersistedAppUpdateSession(buildPersistedAppUpdateSession('downloaded'));
    return nextState;
  } catch (error: any) {
    try { fs.rmSync(tempPath, { force: true }); } catch {}
    return setAppUpdateState({
      downloading: false,
      installing: false,
      downloadedPath: null,
      readyToInstall: false,
      restartRequired: false,
      error: error?.message ?? 'Update download failed.',
    });
  }
}

async function installDownloadedUpdate(): Promise<AppUpdateState> {
  reconcilePersistedUpdaterState();
  if (!appUpdateState.downloadedPath || !fs.existsSync(appUpdateState.downloadedPath)) {
    throw new Error('No downloaded update is ready to install.');
  }
  if (process.platform === 'win32') {
    const installerPath = path.resolve(appUpdateState.downloadedPath);
    
    // We use cmd.exe's 'start' to invoke ShellExecute, which automatically
    // handles UAC elevation prompts. Node's native 'spawn' directly on the .exe
    // would fail with ERROR_ELEVATION_REQUIRED (code 740) without elevation.
    //
    // Passing --force-run leverages the NSIS installer's native electron-builder logic:
    // 1. It waits for the parent app process to exit.
    // 2. It performs a silent installation (/S).
    // 3. It automatically relaunches the application after success.
    const worker = spawn('cmd.exe', ['/c', 'start', '""', installerPath, '/S', '--force-run', '--updated'], {
      detached: true,
      stdio: 'ignore',
      windowsHide: true,
    });
    worker.unref();

    writePersistedAppUpdateSession(buildPersistedAppUpdateSession('install-requested'));
    const nextState = setAppUpdateState({
      checking: false,
      installing: true,
      available: false,
      downloading: false,
      readyToInstall: false,
      restartRequired: false,
      error: null,
    });
    setTimeout(() => {
      app.quit();
    }, 200);
    return nextState;
  }

  setAppUpdateState({ installing: true, error: null });
  const result = await shell.openPath(appUpdateState.downloadedPath);
  if (result) {
    throw new Error(result);
  }
  return setAppUpdateState({
    installing: false,
    error: null,
  });
}

function quitForScheduledAppUpdate(): boolean {
  const session = readPersistedAppUpdateSession();
  const hasPendingInstall = session
    && session.phase === 'install-requested'
    && fs.existsSync(path.resolve(session.downloadedPath));
  if (!hasPendingInstall && !appUpdateState.restartRequired) {
    throw new Error('No pending update install is waiting for restart.');
  }
  setTimeout(() => app.quit(), 100);
  return true;
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
  const rawMessage = err.message ?? String(e);
  const message = (() => {
    let normalized = rawMessage;
    let previous = '';
    while (normalized !== previous) {
      previous = normalized;
      normalized = normalized
        .replace(/^Error invoking remote method '[^']+':\s*/i, '')
        .replace(/^Error:\s*/i, '')
        .trim();
    }
    return normalized;
  })();

  if (message.includes('SAFETY BLOCK: BIOS readiness is no longer satisfied')) {
    return {
      category: 'app_error',
      message: 'Flash preparation is blocked by BIOS readiness',
      explanation: 'The firmware checklist no longer passes at the destructive flash boundary, so the USB write step was stopped before it started.',
      suggestion: 'Return to the BIOS step, recheck the required firmware settings, then reopen the flash confirmation dialog.'
    };
  }

  if (message.includes('Compatibility is blocked') || message.includes('No supported display path')) {
    return {
      category: 'app_error',
      message: 'Flash preparation is blocked by compatibility',
      explanation: 'The selected macOS target is no longer deployable for the current hardware path, so retrying the USB write step will not fix it.',
      suggestion: 'Return to the report step, fix the compatibility blocker, then reopen the flash confirmation dialog.'
    };
  }

  if (message.includes('No target disk is selected for flashing')) {
    return {
      category: 'app_error',
      message: 'Flash preparation is blocked by a missing selected drive',
      explanation: 'The destructive flash step was reached without a live selected target disk.',
      suggestion: 'Go back to USB selection, refresh the drive list, and select the target drive again before flashing.'
    };
  }

  if (message.includes('Disk identity could not be confirmed') || message.includes('No disk identity fingerprint was captured')) {
    return {
      category: 'app_error',
      message: 'Flash preparation is blocked by missing disk identity',
      explanation: 'The app could not confirm the physical identity of the target drive immediately before the destructive write step.',
      suggestion: 'Reconnect the drive, re-select it, wait for its details to load, then reopen the flash confirmation dialog.'
    };
  }

  if (message.includes('Operation stalled') || message.includes('no progress received for 60 seconds')) {
    return {
      category: 'app_error',
      message: 'Operation stalled',
      explanation: 'A background flash step stopped reporting progress before the app could confirm the final state. This is not enough evidence to blame permissions or the USB drive.',
      suggestion: 'Retry the interrupted step once. If it stalls again, copy the diagnostics instead of retrying blindly.'
    };
  }

  if (/^Task .* was cancelled$/i.test(message)) {
    return {
      category: 'app_error',
      message: 'Operation was cancelled before completion',
      explanation: 'The flash task ended before the app could confirm the final state on disk. This is not a proven permission or hardware failure.',
      suggestion: 'Retry the interrupted step once. If the same cancellation repeats, copy the diagnostics and report it.'
    };
  }

  // Recovery download resume failure (#42)
  if (message.includes('HTTP 416')) {
    return {
      category: 'environment_error',
      message: 'Download cache mismatch',
      explanation: 'The local recovery download cache does not match the file on Apple\'s server. This often happens if the download was interrupted and the file version changed.',
      suggestion: 'Retry the download. The app will automatically clear the mismatch and start fresh.'
    };
  }

  // Disk format failures (#38, #41) — must be classified before generic hardware errors
  if (message.includes('failed to format it as FAT32') || message.includes('Format-Volume fallback') || message.includes('diskpart format')) {
    return {
      category: 'hardware_error',
      message: 'Disk format failed',
      explanation: 'Windows could not format the partition as FAT32. Another process may be locking the drive, or the drive controller is rejecting the format command.',
      suggestion: 'Close Explorer and any programs accessing the drive, then retry. If repeated failures occur, try a different USB port (rear motherboard port preferred).'
    };
  }

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
  if (maxAttempts <= 0) throw new Error('maxAttempts must be >= 1');
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
  const runProbe = (cmd: string) => execPromise(cmd, {
    timeout: 5_000,
    maxBuffer: 1024 * 1024,
  });

  if (process.platform === 'win32') {
    try {
      const sb = await runProbe('powershell -NoProfile -Command "Confirm-SecureBootUEFI"');
      status.secureBootDisabled = sb.stdout.trim().toLowerCase() === 'false';
    } catch (e) {}

    try {
      const vt = await runProbe('powershell -NoProfile -Command "(Get-CimInstance Win32_Processor).VirtualizationFirmwareEnabled"');
      status.virtualizationEnabled = vt.stdout.trim().toLowerCase() === 'true';
    } catch (e) {}
  } else if (process.platform === 'linux') {
    try {
      const sb = await runProbe('bootctl status 2>/dev/null | grep "Secure Boot"');
      status.secureBootDisabled = sb.stdout.toLowerCase().includes('disabled');
    } catch (e) {}
    
    try {
      const vt = await runProbe('LANG=C lscpu | grep Virtualization');
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
): Promise<{ ocExtractedDir: string }> {
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
  return { ocExtractedDir: ocExtracted };
}

async function createEfiStructure(
  basePath: string,
  profile: HardwareProfile,
  token?: OpToken,
  onPhase?: (phase: string, detail: string) => void,
) {
  const { ocExtractedDir } = await ensureOpenCoreBinaries(basePath, token, onPhase);
  const openCoreCacheDir = path.resolve(app.getPath('userData'), 'OpenCore_Cache');

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

  // Source SSDTs from the OpenCore package's pre-compiled samples
  const ssdtSampleDirs = [
    path.resolve(ocExtractedDir, 'Docs', 'AcpiSamples', 'Binaries'),
    path.resolve(ocExtractedDir, 'Docs', 'AcpiSamples'),
  ];
  onPhase?.('Placing ACPI tables…', `${ssdts.length} SSDT files required.`);
  const unsupportedSsdts = getUnsupportedSsdtRequests(ssdts);
  if (unsupportedSsdts.length > 0) {
    log('ERROR', 'efi', 'Required SSDT has no supported source policy', {
      unsupportedSsdts,
      requestedSsdts: ssdts,
    });
    throw new Error(
      `EFI build failed: required SSDT files have no supported source policy for this hardware profile: ${unsupportedSsdts.join(', ')}.`
    );
  }
  const missingSsdts: string[] = [];
  for (const s of ssdts) {
    const destPath = path.resolve(basePath, 'EFI/OC/ACPI', s);
    if (fs.existsSync(destPath) && fs.statSync(destPath).size > 0) continue;
    const policy = getSsdtSourcePolicy(s);
    if (!policy) {
      missingSsdts.push(s);
      log('ERROR', 'efi', `Required SSDT has no supported source policy: ${s}`);
      continue;
    }
    let found = false;
    for (const sampleDir of ssdtSampleDirs) {
      for (const candidate of policy.packageCandidates) {
        const src = path.resolve(sampleDir, candidate);
        if (fs.existsSync(src) && fs.statSync(src).size > 0) {
          fs.copyFileSync(src, destPath);
          found = true;
          break;
        }
      }
      if (found) break;
    }
    if (!found && policy.supplementalDownload) {
      const supplementalDir = path.resolve(openCoreCacheDir, 'Supplemental_ACPI');
      const cachedSupplementalPath = path.resolve(supplementalDir, policy.supplementalDownload.fileName);
      if (!fs.existsSync(cachedSupplementalPath) || fs.statSync(cachedSupplementalPath).size === 0) {
        fs.mkdirSync(supplementalDir, { recursive: true });
        onPhase?.('Fetching supplemental ACPI tables…', policy.supplementalDownload.fileName);
        try {
          await downloadFileWithProgress(
            policy.supplementalDownload.url,
            cachedSupplementalPath,
            () => {},
            0,
            () => token?.check(),
          );
        } catch (dlErr) {
          log('ERROR', 'efi', `Failed to download supplemental SSDT: ${s}`, {
            url: policy.supplementalDownload.url,
            error: dlErr instanceof Error ? dlErr.message : String(dlErr),
          });
        }
      }
      if (fs.existsSync(cachedSupplementalPath) && fs.statSync(cachedSupplementalPath).size > 0) {
        fs.copyFileSync(cachedSupplementalPath, destPath);
        found = true;
      }
    }
    if (!found) {
      missingSsdts.push(s);
      log('ERROR', 'efi', `Required SSDT not found in package or supplemental sources: ${s}`, {
        searchedDirs: ssdtSampleDirs,
        candidateNames: policy.packageCandidates,
        supplementalUrl: policy.supplementalDownload?.url ?? null,
      });
    }
  }
  // SSDT-GPIO is optional: it's only needed for VoodooI2C (I2C trackpad path).
  // If it can't be sourced, remove it from config.plist and fall back to PS2.
  const OPTIONAL_SSDTS = new Set(['SSDT-GPIO.aml']);
  const requiredMissing = missingSsdts.filter(s => !OPTIONAL_SSDTS.has(s));
  const optionalMissing = missingSsdts.filter(s => OPTIONAL_SSDTS.has(s));

  if (optionalMissing.length > 0) {
    const configPath = path.resolve(basePath, 'EFI/OC/config.plist');
    if (fs.existsSync(configPath)) {
      let configContent = fs.readFileSync(configPath, 'utf-8');
      for (const ssdt of optionalMissing) {
        const escaped = ssdt.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        const ssdtBlock = new RegExp(`\\s*<dict>[\\s\\S]*?<string>${escaped}</string>[\\s\\S]*?</dict>`, 'g');
        configContent = configContent.replace(ssdtBlock, '');
        log('WARN', 'efi', `Removed optional SSDT ${ssdt} from config.plist (download failed, falling back to PS2 path)`, { ssdt });
      }
      // Also remove VoodooI2C kexts if SSDT-GPIO was dropped (they need GPIO to work)
      if (optionalMissing.includes('SSDT-GPIO.aml')) {
        for (const kext of ['VoodooI2C.kext', 'VoodooI2CHID.kext', 'VoodooI2C.kext/Contents/PlugIns/VoodooI2CHID.kext']) {
          const escapedKext = kext.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
          const kextBlock = new RegExp(`\\s*<dict>[\\s\\S]*?<string>${escapedKext}</string>[\\s\\S]*?</dict>`, 'g');
          configContent = configContent.replace(kextBlock, '');
        }
        log('WARN', 'efi', 'Removed VoodooI2C kexts from config.plist (SSDT-GPIO unavailable, falling back to PS2)', {});
      }
      fs.writeFileSync(configPath, configContent);
    }
  }

  if (requiredMissing.length > 0) {
    throw new Error(
      `EFI build failed: required SSDT files could not be sourced for this hardware profile: ${requiredMissing.join(', ')}. ` +
      'The OpenCore cache or supplemental ACPI cache may be incomplete — rebuild after clearing OpenCore_Cache.'
    );
  }

  // Kext directories are created by fetch-latest-kexts when content is ready.
  // Pre-creating empty stubs causes broken-bundle validation failures
  // when a kext fetch fails (e.g. NootedRed with no embedded fallback).
}

// --- Hardware Detection per Platform ---

async function getWindowsHardwareInfo(): Promise<HardwareProfile> {
  const psCommand = (cmd: string) => `powershell -NoProfile -Command "${cmd}"`;
  const runWindowsProbe = (cmd: string, fallback = '', timeout = 8_000) =>
    execPromise(psCommand(cmd), {
      timeout,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRaw, gpuRaw, baseboardRaw, coresRaw, chassisRaw, manufRaw] = await Promise.all([
    runWindowsProbe("Get-CimInstance CIM_Processor | Select-Object -ExpandProperty Name", 'Unknown CPU', 10_000),
    runWindowsProbe("Get-CimInstance CIM_VideoController | Select-Object -ExpandProperty Name", 'Unknown GPU', 10_000),
    runWindowsProbe("Get-CimInstance CIM_BaseBoard | Select-Object -ExpandProperty Product", 'Unknown Board', 8_000),
    runWindowsProbe("Get-CimInstance CIM_Processor | Select-Object -ExpandProperty NumberOfCores", '4', 8_000),
    runWindowsProbe("Get-CimInstance CIM_SystemEnclosure | Select-Object -ExpandProperty ChassisTypes", '', 6_000),
    runWindowsProbe("Get-CimInstance CIM_ComputerSystem | Select-Object -ExpandProperty Manufacturer", 'Unknown', 6_000),
  ]);
  const [modelRaw, batteryRaw] = await Promise.all([
    runWindowsProbe("Get-CimInstance CIM_ComputerSystem | Select-Object -ExpandProperty Model", '', 6_000),
    runWindowsProbe("Get-CimInstance Win32_Battery | Select-Object -First 1 | ConvertTo-Json -Compress", '', 6_000),
  ]);

  const cpuModel = cpuRaw.stdout.trim().split('\n')[0] || os.cpus()[0]?.model || 'Unknown CPU';
  const gpuLines = gpuRaw.stdout
    .trim()
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  const filteredGpuLines = gpuLines.filter((line) => !/remote display adapter|basic display adapter|render only|indirect display/i.test(line));
  const gpuModel = (filteredGpuLines.length > 0 ? filteredGpuLines : gpuLines).join(' / ') || 'Unknown GPU';
  const motherboard = baseboardRaw.stdout.trim().split('\n')[0] || 'Unknown';
  const coreCount = parseInt(coresRaw.stdout.trim()) || 4;
  
  const chassisTypes = chassisRaw.stdout.trim().split('\n').map(c => parseInt(c.trim())).filter(n => !isNaN(n) && n > 0);
  const manufacturerStr = manufRaw.stdout.trim();
  const isLaptop = inferLaptopFormFactor({
    cpuName: cpuModel,
    chassisTypes,
    modelName: modelRaw.stdout.trim(),
    batteryPresent: batteryRaw.stdout.trim().length > 0 && batteryRaw.stdout.trim() !== 'null',
    manufacturer: manufacturerStr,
    gpuName: gpuModel,
  });

  const manuf = manufacturerStr.toLowerCase();
  const isVM = manuf.includes('vmware') || manuf.includes('qemu') || manuf.includes('innotek') || manuf.includes('microsoft corporation') || manuf.includes('parallels');

  const generation = detectCpuGeneration(cpuModel);
  const architecture = detectArchitecture(cpuModel);

  // Build gpuDevices list from parsed GPU names for proper classification downstream
  const gpuNames = gpuModel.split(' / ').map(name => name.trim()).filter(Boolean);
  const gpuDevices = gpuNames.map(name => ({ name }));

  const profile: HardwareProfile = {
    cpu: cpuModel,
    architecture,
    generation,
    coreCount,
    gpu: gpuModel,
    gpuDevices,
    ram: (os.totalmem() / 1024 / 1024 / 1024).toFixed(0) + " GB",
    motherboard,
    targetOS: 'macOS Sequoia 15.x',
    smbios: '',
    kexts: [], ssdts: [],
    bootArgs: '-v keepsyms=1 debug=0x100',
    isLaptop,
    isVM,
    audioLayoutId: 1,
    inputStack: 'unknown', // Legacy scanner cannot detect I2C — stays conservative PS2
  };
  profile.smbios = getSMBIOSForProfile(profile);
  return profile;
}

async function getLinuxHardwareInfo(): Promise<HardwareProfile> {
  const runLinuxProbe = (cmd: string, fallback = '') =>
    execPromise(cmd, {
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRaw, gpuRaw, baseboardRaw, memRaw, chassisRaw, vendorRaw, batteryRaw] = await Promise.all([
    runLinuxProbe('lscpu'),
    runLinuxProbe('lspci | grep -i vga'),
    runLinuxProbe('cat /sys/class/dmi/id/board_name 2>/dev/null || cat /sys/class/dmi/id/product_name 2>/dev/null', 'Unknown'),
    runLinuxProbe('free -b | grep Mem', '0 0'),
    runLinuxProbe('cat /sys/class/dmi/id/chassis_type 2>/dev/null', '3'),
    runLinuxProbe('cat /sys/class/dmi/id/sys_vendor 2>/dev/null', 'Unknown'),
    runLinuxProbe('ls /sys/class/power_supply 2>/dev/null | grep -E "^BAT"'),
  ]);

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
  const runMacProbe = (cmd: string, fallback = '') =>
    execPromise(cmd, {
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRaw, gpuRaw, memRaw, modelRaw, hwModelRaw] = await Promise.all([
    runMacProbe('sysctl -n machdep.cpu.brand_string', 'Unknown Mac CPU'),
    runMacProbe('system_profiler SPDisplaysDataType 2>/dev/null | grep "Chipset Model" | awk -F": " \'{print $2}\'', 'Unknown Mac GPU'),
    runMacProbe('sysctl -n hw.memsize', '8589934592'),
    runMacProbe('system_profiler SPHardwareDataType 2>/dev/null | grep "Model Identifier" | cut -d: -f2', 'Mac'),
    runMacProbe('sysctl -n hw.model 2>/dev/null', 'Unknown'),
  ]);
  
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

// --- GitHub Kext Fetcher ---

const KEXT_REGISTRY: Record<string, KextRegistryEntry> = {
  'Lilu.kext':                          { repo: 'acidanthera/Lilu',                    assetFilter: 'RELEASE' },
  'VirtualSMC.kext':                    { repo: 'acidanthera/VirtualSMC',              assetFilter: 'RELEASE' },
  'SMCBatteryManager.kext':             { repo: 'acidanthera/VirtualSMC',              assetFilter: 'RELEASE' },
  'WhateverGreen.kext':                 { repo: 'acidanthera/WhateverGreen',           assetFilter: 'RELEASE' },
  'AppleALC.kext':                      { repo: 'acidanthera/AppleALC',                assetFilter: 'RELEASE' },
  'NootedRed.kext':                     { repo: 'ChefKissInc/NootedRed',               directUrl: 'https://nightly.link/ChefKissInc/NootedRed/workflows/main/master/Artifacts.zip', staticVersion: 'nightly' },
  'NootRX.kext':                        { repo: 'ChefKissInc/NootRX',                  directUrl: 'https://nightly.link/ChefKissInc/NootRX/workflows/main/master/Artifacts.zip', staticVersion: 'nightly' },
  'RTCMemoryFixup.kext':                { repo: 'acidanthera/RTCMemoryFixup',          assetFilter: 'RELEASE' },
  'VoodooPS2Controller.kext':           { repo: 'acidanthera/VoodooPS2',              assetFilter: 'RELEASE' },
  'VoodooI2C.kext':                     { repo: 'VoodooI2C/VoodooI2C',               assetFilter: 'VoodooI2C' },
  'AMDRyzenCPUPowerManagement.kext':    { repo: 'trulyspinach/SMCAMDProcessor',        directUrl: 'https://github.com/trulyspinach/SMCAMDProcessor/releases/latest/download/AMDRyzenCPUPowerManagement.kext.zip', staticVersion: 'latest' },
  'SMCAMDProcessor.kext':               { repo: 'trulyspinach/SMCAMDProcessor',        directUrl: 'https://github.com/trulyspinach/SMCAMDProcessor/releases/latest/download/SMCAMDProcessor.kext.zip', staticVersion: 'latest' },
  'AppleMCEReporterDisabler.kext':      { repo: 'acidanthera/bugtracker',              directUrl: 'https://github.com/acidanthera/bugtracker/files/3703498/AppleMCEReporterDisabler.kext.zip', staticVersion: 'bugtracker' },
  'RestrictEvents.kext':                { repo: 'acidanthera/RestrictEvents',          assetFilter: 'RELEASE' },
  'NVMeFix.kext':                       { repo: 'acidanthera/NVMeFix',                assetFilter: 'RELEASE' },
  'CPUTopologyRebuild.kext':            { repo: 'b00t0x/CpuTopologyRebuild',           assetFilter: 'RELEASE' },
  'ECEnabler.kext':                     { repo: 'averycblack/ECEnabler',               assetFilter: 'RELEASE' },
  'SMCProcessor.kext':                  { repo: 'acidanthera/VirtualSMC',              assetFilter: 'RELEASE' },
  'SMCSuperIO.kext':                    { repo: 'acidanthera/VirtualSMC',              assetFilter: 'RELEASE' },
  'IntelMausi.kext':                    { repo: 'acidanthera/IntelMausi',              assetFilter: 'RELEASE' },
  'RealtekRTL8111.kext':                { repo: 'Mieze/RTL8111_driver_for_OS_X',      assetFilter: 'RealtekRTL8111' },
  'itlwm.kext':                          { repo: 'OpenIntelWireless/itlwm',            assetFilter: 'itlwm' },
  'AirportBrcmFixup.kext':              { repo: 'acidanthera/AirportBrcmFixup',       assetFilter: 'RELEASE' },
  'USBInjectAll.kext':                  { repo: 'Sniki/OS-X-USB-Inject-All',          assetFilter: 'RELEASE' },
};

for (const [kextName, entry] of Object.entries(KEXT_REGISTRY)) {
  entry.embeddedFallback = hasEmbeddedKext(kextName);
}

function readValidatedInstalledKext(kextName: string, targetDir: string): { name: string; version: string } | null {
  if (!validateInstalledKext(kextName, targetDir)) return null;
  const versionFile = path.join(targetDir, kextName, '.version');
  const version = fs.existsSync(versionFile)
    ? fs.readFileSync(versionFile, 'utf-8').trim()
    : 'installed';
  return { name: kextName, version };
}

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
            headers: { 'User-Agent': `${PRODUCT_HTTP_NAME}/1.0` },
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
  const hwStatus = lastScannedProfile
    ? formatHardwareSummary(lastScannedProfile)
    : scanErrorFound
      ? 'Hardware scan failed — check logs for details'
      : 'Hardware scan not completed';

  const confidenceStatus = lastScannedProfile
    ? (lastScannedProfile.scanConfidence || 'unknown')
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
  const fileName = `opcore-oneclick-support-log-${timestamp}.txt`;
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
      biosReady: biosState.readyToBuild || biosState.stage === 'complete',
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
      biosReady: biosState.readyToBuild || biosState.stage === 'complete',
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

async function fetchKextFromGitHub(
  kextName: string,
  targetDir: string,
  releaseCache?: Map<string, Promise<{ version: string; assetUrl: string | null; assetName: string | null }>>,
  checkAborted?: () => void,
): Promise<{ name: string; version: string }> {
  const entry = KEXT_REGISTRY[kextName];
  if (!entry) return { name: kextName, version: 'bundled' };

  const proxy = parseProxy();

  function loadGitHubRelease(): Promise<{ version: string; assetUrl: string | null; assetName: string | null }> {
    if (entry.directUrl) {
      return Promise.resolve({
        version: entry.staticVersion ?? 'direct',
        assetUrl: entry.directUrl,
        assetName: path.basename(new URL(entry.directUrl).pathname) || null,
      });
    }
    const cached = releaseCache?.get(entry.repo);
    if (cached) return cached;

    const request = new Promise<{ version: string; assetUrl: string | null; assetName: string | null }>((resolve, reject) => {
      const reqOptions: any = {
        hostname: 'api.github.com',
        port: 443,
        path: `/repos/${entry.repo}/releases/latest`,
        headers: { 'User-Agent': `${PRODUCT_HTTP_NAME}/1.0` },
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
    const cachedRequest = request.catch((error) => {
      releaseCache?.delete(entry.repo);
      throw error;
    });
    releaseCache?.set(entry.repo, cachedRequest);
    return cachedRequest;
  }

  try {
    const { version, assetUrl } = await retryWithBackoff(loadGitHubRelease, 3, `github-api(${kextName})`, checkAborted);
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
      const installed = readValidatedInstalledKext(kextName, targetDir);
      if (installed) {
        log('WARN', 'kext', `Release metadata for ${kextName} had no matching archive; reusing validated installed copy`, { version: installed.version });
        return installed;
      }
      throw new Error(`No matching archive asset found for ${kextName}`);
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
        const installed = readValidatedInstalledKext(kextName, targetDir);
        if (installed) {
          log('WARN', 'kext', `Archive for ${kextName} did not contain the expected bundle; reusing validated installed copy`, { version: installed.version });
          return installed;
        }
        throw new Error(`Downloaded archive did not contain ${kextName}`);
      }
    } finally {
      try { if (fs.existsSync(tmpZip)) fs.unlinkSync(tmpZip); } catch (_) {}
      try { if (fs.existsSync(tmpExtract)) fs.rmSync(tmpExtract, { recursive: true, force: true }); } catch (_) {}
    }
    return { name: kextName, version };
  } catch (err) {
    const msg = classifyError(err);
    log('ERROR', 'kext', `Failed to fetch ${kextName}`, { error: msg.message });
    throw new Error(`Failed to fetch ${kextName}: ${msg.message}`);
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
  } else if (process.platform === 'linux') {
    // Linux: check per-command elevation (pkexec/sudo) instead of requiring root
    const method = await detectElevationMethod();
    adminPrivileges = method !== 'none';
    if (!adminPrivileges) {
      issues.push({ severity: 'error', message: 'Install polkit (pkexec) or sudo for disk operations — do not run the entire app as root' });
    }
  } else {
    adminPrivileges = (process.getuid?.() ?? 1) === 0;
  }
  if (!adminPrivileges && process.platform === 'win32') {
    issues.push({ severity: 'error', message: 'Run as Administrator for disk operations' });
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
  return targetUrl.startsWith('https://') || targetUrl.startsWith('mailto:');
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
    title: PRODUCT_DISPLAY_NAME,
    // hiddenInset is macOS-only; use default on Windows/Linux to avoid init crash
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
    backgroundColor: '#050505',
    webPreferences: {
      preload: preloadPath,
      contextIsolation: true, nodeIntegration: false,
      // sandbox: false is required because the preload script uses ipcRenderer
      // for IPC communication. With sandbox: true + contextIsolation: true,
      // the preload cannot access electron APIs. This is the standard Electron
      // pattern for apps that use contextBridge + ipcRenderer.
      sandbox: false
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
    if (process.platform === 'win32') {
      try { require('child_process').execSync('echo automount enable | diskpart', { timeout: 5000 }); } catch {}
    }
  });
  // Clear stale OpenCore cache on startup — prevents corrupted/partial downloads from
  // blocking subsequent builds. Supplemental ACPI files and extracted kext archives
  // are re-downloaded as needed. (#51)
  const openCoreCachePath = path.resolve(app.getPath('userData'), 'OpenCore_Cache');
  if (fs.existsSync(openCoreCachePath)) {
    try {
      fs.rmSync(openCoreCachePath, { recursive: true, force: true });
      log('INFO', 'startup', 'Cleared OpenCore_Cache on startup');
    } catch (e: any) {
      log('WARN', 'startup', 'Failed to clear OpenCore_Cache', { error: e?.message });
    }
  }

  hardwareProfileStore = createHardwareProfileStore(app.getPath('userData'));
  const latestHardwareArtifact = hardwareProfileStore.loadLatest();
  if (latestHardwareArtifact && canRestoreLatestScannedArtifact(latestHardwareArtifact)) {
    lastScannedProfile = latestHardwareArtifact.profile;
    lastLiveHardwareProfileArtifact = latestHardwareArtifact;
  }
  efiBackupManager = createEfiBackupManager(app.getPath('userData'));
  // Detect Linux elevation method early so it's cached for preflight and disk ops
  if (process.platform === 'linux') {
    detectElevationMethod().then(method => {
      log('INFO', 'startup', `Linux elevation method: ${method}`);
    }).catch(() => {});
  }
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
  ipcHandle('get-persisted-state', () => {
    const state = loadState();
    // Auto-clear stale app state (>24 hours) to prevent stuck recheck loops
    if (state && Date.now() - state.timestamp > 24 * 60 * 60 * 1000) {
      log('INFO', 'state', 'Auto-cleared stale app state (>24 hours old)', { step: state.currentStep });
      saveState(null);
      return null;
    }
    return state;
  });
  ipcHandle('save-state', (_event, state: AppState) => {
    if (!state || typeof state.currentStep !== 'string' || typeof state.timestamp !== 'number') {
      log('WARN', 'state', 'save-state rejected: invalid shape', { state });
      return;
    }
    saveState(state);
  });
  ipcHandle('clear-state', () => { if (fs.existsSync(STATE_FILE)) fs.unlinkSync(STATE_FILE); });

  // Hardware profile artifacts — advisory planning inputs only
  ipcHandle('hardware-profile:get-latest', () => {
    const artifact = hardwareProfileStore.loadLatest();
    if (artifact && canRestoreLatestScannedArtifact(artifact)) {
      lastScannedProfile = artifact.profile;
      lastLiveHardwareProfileArtifact = artifact;
    }
    return artifact;
  });
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
      defaultPath: path.resolve(app.getPath('documents'), `opcore-oneclick-hardware-profile-${artifact.digest.slice(0, 12)}.json`),
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
    // Auto-clear stale sessions (>7 days) — prevents stuck recheck loops from old state
    if (stale) {
      clearBiosSession(app.getPath('userData'));
      log('INFO', 'bios', 'Auto-cleared stale BIOS session (>7 days old)', { stage: session.stage });
      return { hasSession: false, stage: null, fingerprint: null, stale: false, message: 'Stale BIOS session was automatically cleared.' };
    }
    return {
      hasSession: true,
      stage: session.stage,
      fingerprint: session.hardwareFingerprint,
      stale: false,
      message: `BIOS session active — stage: ${session.stage}`,
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
      let hw: import('./hardwareDetect.js').DetectedHardware;
      let scanSource: 'live_scan' | 'partial_scan' = 'live_scan';
      try {
        hw = await withTimeout(detectHardware(), 60_000, 'detectHardware');
      } catch (primaryErr: any) {
        // Primary scan failed (likely timeout on slow hardware).
        // Before falling to legacy, try to build a partial result from whatever
        // CPU/model data the host OS can provide through Node.js APIs.
        log('WARN', 'scan', 'Primary scanner failed, attempting partial recovery', { error: primaryErr?.message });
        const fallbackCpuName = os.cpus()[0]?.model?.trim() || 'Unknown CPU';
        hw = buildPartialDetectedHardware({
          cpu: {
            name: fallbackCpuName,
            vendor: fallbackCpuName.toLowerCase().includes('intel') ? 'GenuineIntel' : 'Unknown',
            vendorName: fallbackCpuName.toLowerCase().includes('intel') ? 'Intel' : 'Unknown',
            confidence: 'partially-detected',
          },
          ramBytes: os.totalmem(),
          coreCount: os.cpus().length,
          isLaptop: inferLaptopFormFactor({ cpuName: fallbackCpuName }),
        });
        scanSource = 'partial_scan';
      }

      // Build interpretation layer — separates facts from inferences
      const interpretation = interpretHardware(hw);

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
        scanSource,
      });

      // Map DetectedHardware → HardwareProfile (legacy shape used by configGenerator)
      const legacyResult = mapDetectedToProfile(hw);
      const reconciled = reconcileHardwareScanProfile(lastScannedProfile, legacyResult);
      const effectiveProfile = reconciled.profile;
      lastHardwareProfile = hw;
      lastHardwareInterpretation = interpretation;
      lastScannedProfile = effectiveProfile;
      const artifact = savePlanningHardwareProfileArtifact({
        profile: effectiveProfile,
        interpretation: extractHardwareProfileInterpretationMetadata(interpretation),
        source: scanSource === 'partial_scan' ? 'live_scan' : 'live_scan',
      });
      lastLiveHardwareProfileArtifact = artifact;
      return { profile: effectiveProfile, interpretation, artifact };
    } catch (err: any) {
      log('ERROR', 'scan', 'Hardware detection failed, falling back to legacy scanner', { error: err?.message });
      // Fallback to old per-platform functions
      let profile: HardwareProfile;
      if (process.platform === 'darwin') profile = await getMacHardwareInfo();
      else if (process.platform === 'win32') profile = await getWindowsHardwareInfo();
      else profile = await getLinuxHardwareInfo();
      const reconciled = reconcileHardwareScanProfile(lastScannedProfile, profile);
      lastScannedProfile = reconciled.profile;
      if (!reconciled.usedExistingFields) {
        lastHardwareProfile = null;
        lastHardwareInterpretation = null;
      }
      const artifact = savePlanningHardwareProfileArtifact({
        profile: reconciled.profile,
        interpretation: null,
        source: 'legacy_scan',
      });
      lastLiveHardwareProfileArtifact = artifact;
      return { profile: reconciled.profile, interpretation: null, artifact };
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

  // Kexts that are non-fatal when download fails — the system boots without them.
  // These get removed from config.plist instead of blocking the entire EFI build.
  // Wi-Fi and Bluetooth kexts are optional: the user can install them later after
  // getting macOS running. Ethernet kexts without embedded fallbacks are also optional.
  const OPTIONAL_KEXTS = new Set([
    'itlwm.kext',              // Intel Wi-Fi (needs HeliPort companion app anyway)
    'AirportItlwm.kext',       // Intel Wi-Fi (native AirPort, needs SecureBootModel)
    'IntelBluetoothFirmware.kext', // Intel Bluetooth firmware
    'BlueToolFixup.kext',      // Bluetooth fix for Monterey+
    'BrcmPatchRAM3.kext',      // Broadcom Bluetooth
    'AirportBrcmFixup.kext',   // Broadcom Wi-Fi
    'VoodooI2C.kext',          // I2C trackpad (PS2 fallback still works)
    'VoodooI2CHID.kext',       // I2C HID satellite
  ]);

  // Kext fetcher — primary network source (GitHub release or direct asset) → embedded fallback → hard fail
  ipcHandle('fetch-latest-kexts', async (_event: Electron.IpcMainInvokeEvent, efiPath: string, kextNames: string[]) => {
    const kextsDir = path.resolve(efiPath, 'EFI/OC/Kexts');
    if (!fs.existsSync(kextsDir)) fs.mkdirSync(kextsDir, { recursive: true });
    const token = registry.create('kext-fetch');
    log('INFO', 'kext', 'Starting hybrid kext fetch', { count: kextNames.length, taskId: token.taskId });

    const results: { name: string; version: string; source: KextFetchSource }[] = [];
    failedKexts = [];
    kextSources = {};
    const releaseCache = new Map<string, Promise<{ version: string; assetUrl: string | null; assetName: string | null }>>();

    try {
      for (const kextName of kextNames) {
        token.check();
        let result: { name: string; version: string } | null = null;
        let source: KextFetchSource = 'failed';
        let githubFailureReason: string | null = null;
        let embeddedFailureReason: string | null = null;
        const entry = KEXT_REGISTRY[kextName];
        const primarySource: KextFetchSource = entry?.directUrl ? 'direct' : 'github';

        // Layer 1: Try the primary network source (GitHub release or direct asset)
        try {
          result = await fetchKextFromGitHub(kextName, kextsDir, releaseCache, () => token.check());
          // Validate the GitHub result landed correctly
          if (result.version !== 'unavailable' && validateInstalledKext(kextName, kextsDir)) {
            source = primarySource;
            log('INFO', 'kext', `${kextName} — ${primarySource} OK`, { version: result.version });
          } else {
            log('WARN', 'kext', `${kextName} — ${primarySource} download incomplete, trying embedded`, { version: result?.version });
            result = null; // force fallback
          }
        } catch (err: any) {
          githubFailureReason = err?.message ?? String(err);
          log('WARN', 'kext', `${kextName} — ${primarySource} failed: ${err.message}, trying embedded`);
          result = null;
        }

        if (!result) {
          const installed = readValidatedInstalledKext(kextName, kextsDir);
          if (installed) {
            result = installed;
            source = primarySource;
            log('WARN', 'kext', `${kextName} — reusing validated local copy after ${primarySource} failure`, { version: installed.version });
          }
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
        } else if (!result) {
          embeddedFailureReason = `No embedded fallback for ${kextName} — internet access required (${entry?.repo ?? 'unknown upstream'})`;
        }

        // Hard fail — neither source worked
        if (!result) {
          const failureParts = [githubFailureReason, embeddedFailureReason]
            .filter((value): value is string => Boolean(value))
            .map((value) => value.trim());
          failedKexts.push({
            name: kextName,
            repo: entry?.repo || 'unknown',
            error: failureParts.length > 0 ? failureParts.join(' | ') : 'Both GitHub and embedded fallback failed',
          });
          result = { name: kextName, version: 'unavailable' };
          source = 'failed';
          log('ERROR', 'kext', `${kextName} — HARD FAIL: no source available`);

          // Clean up any stub directory so the validator never sees a partial bundle
          const stubDir = path.resolve(kextsDir, `${kextName}.kext`);
          if (fs.existsSync(stubDir)) {
            try { fs.rmSync(stubDir, { recursive: true, force: true }); } catch (_) {}
            log('INFO', 'kext', `${kextName} — removed stub directory to prevent misleading validation`, { stubDir });
          }
        }

        kextSources[kextName] = source;
        results.push({ ...result, source });
        registry.updateProgress(token.taskId, {
          kind: 'kext-fetch', kextName: result.name, version: result.version,
          index: results.length, total: kextNames.length, source,
        });
      }
      // Separate required failures (block build) from optional failures (warn but continue).
      const requiredFailures = failedKexts.filter(fk => !OPTIONAL_KEXTS.has(fk.name));
      const optionalFailures = failedKexts.filter(fk => OPTIONAL_KEXTS.has(fk.name));

      // Remove optional failed kexts from config.plist so validation doesn't block on them
      if (optionalFailures.length > 0) {
        const configPath = path.resolve(efiPath, 'EFI/OC/config.plist');
        if (fs.existsSync(configPath)) {
          let configContent = fs.readFileSync(configPath, 'utf-8');
          for (const fk of optionalFailures) {
            // Remove the entire <dict>...</dict> block for this kext from Kernel/Add
            const bundlePath = fk.name.endsWith('.kext') ? fk.name : `${fk.name}.kext`;
            const escapedName = bundlePath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
            const kextBlockPattern = new RegExp(
              `\\s*<dict>[\\s\\S]*?<string>${escapedName}</string>[\\s\\S]*?</dict>`,
              'g'
            );
            configContent = configContent.replace(kextBlockPattern, '');
            log('WARN', 'kext', `Removed optional kext ${fk.name} from config.plist (download failed, non-fatal)`, {
              kext: fk.name, repo: fk.repo, error: fk.error,
            });
          }
          fs.writeFileSync(configPath, configContent);
        }
      }

      // Fail-fast only on REQUIRED kext failures
      if (requiredFailures.length > 0) {
        const summary = requiredFailures
          .map(fk => `${fk.name} (${fk.repo}): ${fk.error}`)
          .join('; ');
        const errorMessage =
          requiredFailures.length === 1
            ? `Required kext unavailable: ${requiredFailures[0].name} — ${requiredFailures[0].error}. ` +
              `No embedded fallback exists for ${requiredFailures[0].name}. ` +
              'Internet access required to download this kext from GitHub.'
            : `${requiredFailures.length} required kexts unavailable: ${summary}. ` +
              'Internet access is required for kexts that have no embedded fallback.';
        log('ERROR', 'kext', 'Kext fetch completed with hard failures', { requiredFailures, optionalFailures });
        registry.fail(token.taskId, errorMessage);
        throw new Error(errorMessage);
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
  ipcHandle('convert-disk-to-gpt', async (_e: Electron.IpcMainInvokeEvent, device: string, confirmed?: boolean) => {
    if (!VALID_DEVICE_PATTERN.test(device)) throw new Error('Invalid device path');
    if (!confirmed) throw new Error('Disk conversion requires explicit user confirmation');
    const info = await diskOps.getDiskInfo(device);
    if (info.isSystemDisk) {
      throw new Error(`SAFETY BLOCK: ${device} is the system boot disk — cannot erase and convert it to GPT`);
    }
    if (info.partitionTable === 'unknown') {
      throw new Error(`SAFETY BLOCK: Cannot read partition table for ${device} — refusing to erase an unidentified disk`);
    }
    if (info.partitionTable === 'gpt') {
      return info;
    }
    const token = registry.create('partition-prep');
    try {
      const converted = await diskOps.convertDiskToGpt({
        device,
        confirmed,
        onPhase: (phase, detail) => {
          registry.updateProgress(token.taskId, { kind: 'partition-prep', phase, detail });
        },
        registerProcess: (p) => token.registerProcess(p),
      });
      registry.complete(token.taskId);
      return converted;
    } catch (e) {
      const classified = classifyError(e);
      if (!token.aborted) registry.fail(token.taskId, classified.message);
      else registry.cancel(token.taskId);
      throw createClassifiedIpcError(classified, e);
    }
  });
  ipcHandle('shrink-partition', async (_e: Electron.IpcMainInvokeEvent, disk: string, size: number, confirmed?: boolean) => {
    if (!VALID_DEVICE_PATTERN.test(disk)) throw new Error('Invalid device path');
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
    if (!VALID_DEVICE_PATTERN.test(device)) throw new Error('Invalid device path');
    const throwClassified = (message: string): never => {
      const error = new Error(message);
      throw createClassifiedIpcError(classifyError(error), error);
    };

    const { buildProfile, hardwareFingerprint, hardwareProfileDigest } = requireFlashAuthorizationContext();
    const resolvedEfiPath = path.resolve(efiPath);
    const currentDisk = await diskOps.getDiskInfo(device);
    lastSelectedDisk = currentDisk;
    const capturedIdentity = resolveFlashPreparationIdentity(expectedIdentity ?? null, currentDisk);

    if (!capturedIdentity) {
      throwClassified('SAFETY BLOCK: Disk identity could not be confirmed for this target. Re-select the drive and wait for its details to load before flashing.');
    }
    const confirmedIdentity = capturedIdentity as NonNullable<typeof capturedIdentity>;
    const deployGuard = await getDeployFlowGuard(buildProfile, resolvedEfiPath);
    const biosState = await getBiosStateForProfile(buildProfile);
    const validation = await runEfiValidation(resolvedEfiPath, buildProfile);
    const collisionDevices = await getFlashCollisionDevices(confirmedIdentity, device);
    // BIOS readiness: accept if the session is complete OR if readyToBuild is true
    // regardless of stage. The stage can be 'idle' when the session fingerprint changed
    // between scan retries but the underlying settings are still verified.
    // A successful EFI build (which got us here) already validated BIOS readiness.
    const biosReady = biosState.readyToBuild || biosState.stage === 'complete';
    const decision = canProceedWithFlash({
      selectedDevice: device,
      currentDisk,
      expectedIdentity: confirmedIdentity,
      collisionDevices,
      deployGuardAllowed: deployGuard.allowed,
      deployGuardReason: deployGuard.reason,
      biosReady,
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
        expectedIdentity: confirmedIdentity,
      });
      throwClassified(decision.reason ?? 'SAFETY BLOCK: Flash preparation failed.');
    }

    const backupPolicy = await withTimeout(
      captureEfiBackupForFlash({
        device,
        expectedIdentity: confirmedIdentity,
        hardwareProfileDigest,
      }),
      15_000,
      'captureEfiBackupForFlash',
    );
    if (backupPolicy.status === 'blocked') {
      throwClassified(backupPolicy.reason ?? 'SAFETY BLOCK: Existing EFI backup policy blocked the flash confirmation step.');
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
      throwClassified(reason);
    }
    const snapshotEfiStateHash = snapshot.efiStateHash as string;
    const snapshotDiskFingerprint = snapshot.diskFingerprint as NonNullable<typeof snapshot.diskFingerprint>;
    const snapshotHardwareFingerprint = snapshot.hardwareFingerprint as string;

    const record = flashConfirmationStore.issue({
      device,
      expectedIdentity: snapshotDiskFingerprint,
      efiStateHash: snapshotEfiStateHash,
      payloadStateHash: snapshot.payloadStateHash ?? undefined,
      hardwareFingerprint: snapshotHardwareFingerprint,
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
    if (!VALID_DEVICE_PATTERN.test(device)) throw new Error('Invalid device path');
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
      const taskState = registry.get(token.taskId);
      const surfacedError = token.aborted && taskState?.error
        ? new Error(taskState.error)
        : e;
      const classified = classifyError(surfacedError);
      logger.timeline('flash_fail', token.taskId, { category: classified.category, error: surfacedError.message });
      if (!token.aborted) registry.fail(token.taskId, classified.message);
      else if (taskState?.status !== 'failed' && taskState?.status !== 'cancelled') registry.cancel(token.taskId);
      if (logger) logger.flush();
      throw createClassifiedIpcError(classified, surfacedError);
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
      const { boardId } = getRecoveryBoardId(macOSVersion);

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
    // macOS and Linux use per-operation elevation — the app does not need to run as root.
    let adminPrivileges: boolean;
    let adminNote: string | null = null;
    if (process.platform === 'darwin') {
      adminPrivileges = true;
      adminNote = 'macOS grants disk access per-operation via system prompts — no persistent admin session required';
    } else if (process.platform === 'linux') {
      const method = await detectElevationMethod();
      adminPrivileges = method !== 'none';
      adminNote = elevationDescription();
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

  ipcHandle('deterministic:verify-efi-success', async (_event: Electron.IpcMainInvokeEvent, efiPath: string, requiredKexts: string[], requiredSsdts?: string[]) => {
    return verifyEfiBuildSuccess(efiPath, requiredKexts, requiredSsdts);
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
  ipcHandle('open-folder', (_event: Electron.IpcMainInvokeEvent, folderPath: string) => shell.showItemInFolder(folderPath));

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
    const baseUrl = `${REPO_WEB_BASE_URL}/issues/new`;
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
    const url = `${REPO_WEB_BASE_URL}/releases/latest`;
    if (!isSafeExternalTarget(url)) {
      throw new Error('Latest release URL is not a safe external target.');
    }
    await shell.openExternal(url);
    return true;
  });

  ipcHandle('app:update-state', async () => {
    // Return in-memory state directly. Disk reconciliation only happens at
    // startup (createInitialAppUpdateState) and at the start of each action
    // function (checkForAppUpdates, downloadLatestAppUpdate,
    // installDownloadedUpdate). Calling reconcilePersistedUpdaterState here
    // would clobber active in-memory state (e.g. downloading:true) with stale
    // disk state, wiping download progress and allowing double-downloads.
    return appUpdateState;
  });

  ipcHandle('app:check-for-updates', async () => {
    return await checkForAppUpdates();
  });

  ipcHandle('app:download-update', async () => {
    return await downloadLatestAppUpdate();
  });

  ipcHandle('app:install-update', async () => {
    return await installDownloadedUpdate();
  });

  ipcHandle('app:quit-for-update', async () => {
    return quitForScheduledAppUpdate();
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
