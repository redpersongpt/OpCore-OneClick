import React, { useState, useEffect, useMemo, useRef } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import {
  CheckCircle, Usb, Search, Box, Download, Settings, ChevronRight,
  HardDrive, ShieldCheck, ShieldAlert, Check, Info, AlertTriangle,
  X, HelpCircle, Package, RefreshCcw, ChevronDown, ChevronLeft
} from 'lucide-react';
import BrandIcon from './components/BrandIcon';
import { getBIOSSettings, getRequiredResources, getSMBIOSForProfile, type HardwareProfile, type BIOSConfig } from '../electron/configGenerator';
import {
  checkCompatibility,
  type CompatibilityReport,
} from '../electron/compatibility';
import { buildCompatibilityMatrix } from '../electron/compatibilityMatrix';
import type {
  BiosOrchestratorState,
  BiosResumeStateResponse,
  BiosSettingSelection,
  FirmwareRestartCapability,
} from '../electron/bios/types';
import { troubleshootingData, type Severity } from './data/troubleshooting';

import ScanStep      from './components/steps/ScanStep';
import BiosStep      from './components/steps/BiosStep';
import VersionStep   from './components/steps/VersionStep';
import ProgressStep  from './components/steps/ProgressStep';
import UsbStep       from './components/steps/UsbStep';
import CompleteStep  from './components/steps/CompleteStep';
import PrereqStep    from './components/steps/PrereqStep';
import ReportStep    from './components/steps/ReportStep';
import MethodStep    from './components/steps/MethodStep';
import PrecheckStep  from './components/steps/PrecheckStep';
import WelcomeStep   from './components/steps/WelcomeStep';
import FailureRecoveryPanel from './components/FailureRecoveryPanel';
import CopyDiagnosticsButton from './components/CopyDiagnosticsButton';
import DebugOverlay from './components/DebugOverlay';
import ValidationSummary from './components/ValidationSummary';
import EfiReportPanel from './components/EfiReport';
import CommunityPanel from './components/CommunityPanel';
import type { ValidationResult, ValidationTrace } from '../electron/configValidator';
import { generateEfiReport, type EfiReport } from './lib/efiReport';
import { getRelevantIssues, type CommunityIssue } from './data/communityKnowledge';
import { useTaskManager } from './hooks/useTaskManager';
import type { TaskKind, TaskState } from '../electron/taskManager';
import { BEGINNER_SAFETY_MODE } from './config';
import { getSuggestionPayload, type Suggestion } from './lib/suggestionEngine';
import { buildFailureRecoveryViewModel, parseFailureRecoveryPayload } from './lib/failureRecovery.js';
import {
  buildBiosRecoveryPayload,
  performBiosContinue,
  performBiosRecheck,
  type BiosRecoveryCode,
} from './lib/biosStepFlow.js';
import { resolveBackToSafetyStep, resolveRecoveryRetryAction } from './lib/recoveryRouting.js';
import {
  isCompatibilityBlocked,
  recoveryResumeDecision,
  isValidationBlockingDeployment,
  restoreFlowDecision,
  targetSelectionDecision,
} from './lib/releaseFlow.js';
import {
  deriveBiosFlowState,
  deriveReleaseFlowState,
  evaluateBuildGuard,
  evaluateDeployGuard,
  type FlowGuardResult,
} from './lib/stateMachine.js';
import type { StepGuardState, StepId } from './lib/installStepGuards.js';
import {
  buildBuildFlowContext,
  evaluateBuildFlowStall,
  evaluateStepTransitionWithOverrides,
  latestTaskByKind,
  latestTaskByKindSince,
  type BuildFlowSnapshot,
} from './lib/buildFlowMonitor.js';
import {
  canStartBuildRun,
  createBuildEntryUiState,
  taskBelongsToRun,
} from './lib/buildRuntime.js';
import type { PreflightReport, ConfidenceLevel } from '../electron/preventionLayer';
import type { BuildPlan, RecoveryDryRun, Certainty } from '../electron/deterministicLayer';
import type {
  HardwareProfileArtifact,
  HardwareProfileInterpretationMetadata,
} from '../electron/hardwareProfileArtifact';
import type { EfiBackupPolicy } from '../electron/efiBackup';
import type { ResourcePlan } from '../electron/resourcePlanner';
import type { AppUpdateState as ElectronAppUpdateState } from '../electron/appUpdater';
import {
  canRestoreLatestScannedArtifact,
  reconcileHardwareScanProfile,
} from '../electron/hardwareProfileState';
import {
  pickSelectedDiskInfo,
  shouldRetryDiskInfoLookup,
  toExpectedDiskIdentity,
  type RendererDiskInfo,
} from './lib/diskIdentityState.js';
import type { SafeSimulationResult } from '../electron/safeSimulation';
import type { PublicDiagnosticsSnapshot } from '../electron/releaseDiagnostics';
import UpdaterPanel from './components/UpdaterPanel';
import { resolveScanSuccessStep, type ScanSuccessStep } from './lib/scanFlow';
import { getSidebarStatus } from './lib/sidebarState.js';
import { buildResourcePlanOwnerKey, resolveVisibleResourcePlan } from './lib/resourcePlanState.js';
import { APP_UPDATE_REFRESH_INTERVAL_MS, shouldRefreshAppUpdateState } from './lib/updateState.js';
type KextFetchResult = { name: string; version: string; source?: 'github' | 'embedded' | 'direct' | 'failed' };

declare global {
  interface Window {
    electron: {
      scanHardware: () => Promise<{ profile: HardwareProfile; interpretation: import('../electron/hardwareInterpret').HardwareInterpretation | null; artifact: HardwareProfileArtifact }>;
      getLatestHardwareProfile: () => Promise<HardwareProfileArtifact | null>;
      saveHardwareProfile: (payload: { profile: HardwareProfile; interpretation?: HardwareProfileInterpretationMetadata | null; source?: HardwareProfileArtifact['source'] }) => Promise<HardwareProfileArtifact>;
      exportHardwareProfile: (artifact?: HardwareProfileArtifact | null) => Promise<{ filePath: string; artifact: HardwareProfileArtifact } | null>;
      importHardwareProfile: () => Promise<HardwareProfileArtifact | null>;
      inspectEfiBackupPolicy: (device: string) => Promise<EfiBackupPolicy>;
      buildEFI: (p: HardwareProfile, allowAcceptedSession?: boolean) => Promise<string>;
      fetchLatestKexts: (efi: string, ks: string[]) => Promise<KextFetchResult[]>;
      downloadRecovery: (dir: string, osv: string, startOffset?: number) => Promise<{ dmgPath: string; recoveryDir: string }>;
      listUsbDevices: () => Promise<{ name: string; device: string; size: string }[]>;
      prepareFlashConfirmation: (dev: string, efi: string, expectedIdentity?: { devicePath?: string; sizeBytes?: number; model?: string; vendor?: string; serialNumber?: string; transport?: string; removable?: boolean; partitionTable?: string }) => Promise<{ token: string; expiresAt: number; diskInfo: RendererDiskInfo; backupPolicy: EfiBackupPolicy }>;
      flashUsb: (dev: string, efi: string, ok: boolean, confirmationToken?: string | null) => Promise<boolean>;
      validateEfi: (efiPath: string, profile?: import('../electron/configGenerator').HardwareProfile | null) => Promise<import('../electron/configValidator').ValidationResult>;
      enableProductionLock: (efi: string, targetOS?: string) => Promise<boolean>;
      getBiosState: (profile: import('../electron/configGenerator').HardwareProfile) => Promise<BiosOrchestratorState>;
      applySupportedBiosChanges: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<{ state: BiosOrchestratorState; appliedCount: number; message: string }>;
      verifyManualBiosChanges: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<BiosOrchestratorState>;
      continueBiosWithCurrentState: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<BiosOrchestratorState>;
      restartToFirmwareWithSession: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<{ supported: boolean; error?: string; state: BiosOrchestratorState }>;
      clearBiosSession: () => Promise<boolean>;
      getBiosResumeState: () => Promise<import('../electron/bios/types').BiosResumeStateResponse>;
      getBiosRestartCapability: () => Promise<import('../electron/bios/types').FirmwareRestartCapability>;
      guardBuild: (profile: import('../electron/configGenerator').HardwareProfile, allowAcceptedSession?: boolean) => Promise<import('./lib/stateMachine').FlowGuardResult>;
      guardDeploy: (profile: import('../electron/configGenerator').HardwareProfile, efiPath: string) => Promise<import('./lib/stateMachine').FlowGuardResult>;
      openFolder: (p: string) => Promise<void>;
      getLogPath: () => Promise<string>;
      getPersistedState: () => Promise<any>;
      saveState: (s: object) => Promise<void>;
      clearState: () => Promise<void>;
      probeBios: () => Promise<{ secureBootDisabled: boolean | 'unknown'; virtualizationEnabled: boolean | 'unknown' }>;
      probeFirmware: () => Promise<{ ok: boolean; data?: import('../electron/firmwarePreflight').FirmwareInfo; error?: string }>;
      restartComputer: () => Promise<void>;
      restartToBios: () => Promise<{ supported: boolean; error?: string }>;
      disableAutostart: () => Promise<void>;
      getHardDrives: () => Promise<{ name: string; device: string; size: string; type: string }[]>;
      shrinkPartition: (disk: string, sizeGB: number, confirmed: boolean) => Promise<void>;
      createBootPartition: (disk: string, efiPath: string, confirmed: boolean, profile?: import('../electron/configGenerator').HardwareProfile | null) => Promise<void>;
      getDownloadResumeState: () => Promise<{ offset: number; dmgDest: string; clDest: string | null; efiPath: string; targetOS: string } | null>;
      getDiskInfo: (device: string) => Promise<RendererDiskInfo>;
      runPreflight: () => Promise<{ ok: boolean; issues: Array<{ severity: string; message: string }>; adminPrivileges: boolean; binaries: Record<string, boolean>; freeSpaceMB: number }>;
      // Task manager
      onTaskUpdate: (cb: (payload: { task: import('../electron/taskManager').TaskState }) => void) => () => void;
      taskList: () => Promise<import('../electron/taskManager').TaskState[]>;
      taskCancel: (taskId: string) => Promise<boolean>;
      // Enhanced logging
      getLogTail: (n: number) => Promise<Record<string, unknown>[]>;
      getOpsTail: (n: number) => Promise<Record<string, unknown>[]>;
      logClear: () => Promise<boolean>;
      getSessionId: () => Promise<string>;
      // Issue reporter
      reportIssue: (extraContext?: string | null) => Promise<{ success: boolean; body: string; baseUrl: string }>;
      openLatestReleasePage: () => Promise<boolean>;
      getAppUpdateState: () => Promise<ElectronAppUpdateState>;
      checkForUpdates: () => Promise<ElectronAppUpdateState>;
      downloadLatestUpdate: () => Promise<ElectronAppUpdateState>;
      installLatestUpdate: () => Promise<ElectronAppUpdateState>;
      quitForUpdate: () => Promise<boolean>;
      // Recovery Cache & Import
      importRecovery: (targetPath: string, macOSVersion: string) => Promise<{ dmgPath: string; recoveryDir: string } | null>;
      getCachedRecoveryInfo: (version: string) => Promise<any>;
      clearRecoveryCache: (version: string) => Promise<boolean>;
      // Extended prechecks
      runPrechecks: () => Promise<{
        platform: string;
        adminPrivileges: boolean;
        adminNote?: string | null;
        freeSpaceMB: number;
        networkOk: boolean;
        usbDetected: boolean;
        firmwareDetectionAvailable: boolean;
        missingBinaries: string[];
      }>;
      runSafeSimulation: (profile: import('../electron/configGenerator').HardwareProfile) => Promise<SafeSimulationResult>;
      getResourcePlan: (profile: import('../electron/configGenerator').HardwareProfile, efiPath?: string | null) => Promise<ResourcePlan>;
      // Diagnostics snapshot
      getDiagnostics: () => Promise<PublicDiagnosticsSnapshot>;
      saveSupportLog: (extraContext?: string | null) => Promise<{ fileName: string; savedTo: 'Desktop' }>;
      logUiEvent: (eventName: string, detail?: Record<string, unknown> | null) => Promise<boolean>;
      notifyRendererReady: () => Promise<boolean>;
    };
  }
}

const STEP_ORDER: StepId[] = ['welcome','prereq','precheck','scanning','version-select','report','method-select','bios','building','kext-fetch','recovery-download','usb-select','part-prep','flashing','complete'];

/** Shared step transition — consistent, subtle, fast. */
const STEP_TRANSITION = { duration: 0.2, ease: [0.25, 0.1, 0.25, 1.0] as const };
const stepEnter  = { opacity: 0, y: 8 };
const stepActive = { opacity: 1, y: 0 };
const stepExit   = { opacity: 0, y: -4 };

const SIDEBAR_STEPS = [
  { id: 'welcome',           label: 'Welcome',          icon: BrandIcon },
  { id: 'prereq',            label: 'Prerequisites',    icon: CheckCircle },
  { id: 'precheck',          label: 'System Check',     icon: ShieldCheck },
  { id: 'scanning',          label: 'Hardware Scan',    icon: Search },
  { id: 'version-select',    label: 'macOS Version',    icon: BrandIcon },
  { id: 'report',            label: 'Compatibility',    icon: ShieldCheck },
  { id: 'bios',              label: 'BIOS Settings',    icon: Settings },
  { id: 'building',          label: 'Build EFI',        icon: Box },
  { id: 'kext-fetch',        label: 'Fetch Kexts',      icon: Download },
  { id: 'recovery-download', label: 'Download macOS',   icon: BrandIcon },
  { id: 'usb-select',        label: 'USB Drive',        icon: Usb },
  { id: 'complete',          label: 'Complete',          icon: CheckCircle },
] as const;


const CATS = ['All', 'OpenCore Boot Issues', 'Kernel Issues', 'Userspace Issues', 'Post-Install Issues', 'Miscellaneous Issues'] as const;
type PlanningProfileContext = 'live_scan' | 'imported_artifact' | 'saved_artifact' | null;

const debugWarn = (...args: unknown[]) => {
  if (import.meta.env.DEV) console.warn(...args);
};

const debugError = (...args: unknown[]) => {
  if (import.meta.env.DEV) console.error(...args);
};

export default function App() {
  const [step, _setStepRaw] = useState<StepId>('landing');
  const [progress, setProgress] = useState(0);
  const [statusText, setStatus] = useState('');
  const [profile, setProfile] = useState<HardwareProfile | null>(null);
  const [profileArtifact, setProfileArtifact] = useState<HardwareProfileArtifact | null>(null);
  const [planningProfileContext, setPlanningProfileContext] = useState<PlanningProfileContext>(null);
  const [compat, setCompat] = useState<CompatibilityReport | null>(null);
  const [biosConf, setBiosConf] = useState<BIOSConfig | null>(null);
  const [biosStatus, setBiosStatus] = useState<any>(null);
  const [firmwareInfo, setFirmwareInfo] = useState<import('../electron/firmwarePreflight').FirmwareInfo | null>(null);
  const [biosState, setBiosState] = useState<BiosOrchestratorState | null>(null);
  const [biosResumeState, setBiosResumeState] = useState<BiosResumeStateResponse | null>(null);
  const [restartCapability, setRestartCapability] = useState<FirmwareRestartCapability | null>(null);
  const [hwInterpretation, setHwInterpretation] = useState<import('../electron/hardwareInterpret').HardwareInterpretation | null>(null);
  const [efiPath, setEfiPath] = useState<string | null>(null);
  const [buildReady, setBuildReady] = useState(false);
  const [kextResults, setKextResults] = useState<KextFetchResult[]>([]);
  const [recovPct, setRecovPct] = useState(0);
  const [recovStatus, setRecovStatus] = useState('');
  const [recovError, setRecovError] = useState<string | null>(null);
  const [recovOffset, setRecovOffset] = useState(0);
  const [recovDmgDest, setRecovDmgDest] = useState<string | null>(null);
  const [recovClDest, setRecovClDest] = useState<string | null>(null);
  const [cachedRecovInfo, setCachedRecovInfo] = useState<any>(null);
  const [platform, setPlatform] = useState<string>('unknown');
  const [adminPrivileges, setAdminPrivileges] = useState<boolean | null>(null);
  const [appUpdateState, setAppUpdateState] = useState<ElectronAppUpdateState | null>(null);
  const [usbDevices, setUsbDevices] = useState<import('./components/steps/UsbStep').DriveInfo[]>([]);
  const [usbRefreshBusy, setUsbRefreshBusy] = useState(false);
  const [selectedUsb, setSelectedUsb] = useState<string | null>(null);
  const [efiBackupPolicy, setEfiBackupPolicy] = useState<EfiBackupPolicy | null>(null);
  const [productionLocked, setProdLock] = useState(false);
  const [showLockModal, setLockModal] = useState(false);
  const [search, setSearch] = useState('');
  const [cat, setCat] = useState<string>('All');
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [method, setMethod] = useState<'usb' | 'partition'>('usb');
  const [selectedDisk, setSelectedDisk] = useState<string | null>(null);
  // ── Disclaimer / Safety gates ──────────────────────────────────
  const [showDisclaimer, setShowDisclaimer] = useState(false);
  const [disclaimerAccepted, setDisclaimerAccepted] = useState(false);
  const [showRecoveryPrompt, setShowRecoveryPrompt] = useState(false);
  const [globalError, setGlobalError] = useState<string | null>(null);
  const [globalNotice, setGlobalNotice] = useState<string | null>(null);
  const lastRecovSaveRef = useRef(0);
  const latestProfileRef = useRef<HardwareProfile | null>(null);
  const latestEfiPathRef = useRef<string | null>(null);
  const latestPlanningContextRef = useRef<PlanningProfileContext>(null);
  const isDeployingRef = useRef(false);
  const isScanningRef = useRef(false);
  const scanRequestIdRef = useRef(0);
  const usbRefreshRequestRef = useRef(0);
  const selectedUsbInfoRequestRef = useRef(0);
  const selectedUsbRef = useRef<string | null>(null);
  const [diskInfo, setDiskInfo] = useState<RendererDiskInfo | null>(null);
  const [showDiskWarning, setShowDiskWarning] = useState(false);
  const [showUnknownPartitionWarning, setShowUnknownPartitionWarning] = useState(false);
  const [showFlashConfirm, setShowFlashConfirm] = useState(false);
  const [showPartitionConfirm, setShowPartitionConfirm] = useState(false);
  const [flashConfirmText, setFlashConfirmText] = useState('');
  const [flashConfirmationToken, setFlashConfirmationToken] = useState<string | null>(null);
  const [flashConfirmationExpiresAt, setFlashConfirmationExpiresAt] = useState<number | null>(null);
  // Phase 7b: flash milestones
  const [flashMilestones, setFlashMilestones] = useState<string[]>([]);
  // Flash confirmation: 4-checkbox gate
  const [flashChecks, setFlashChecks] = useState<Set<string>>(new Set());
  // EFI validation state
  const [validationResult, setValidationResult] = useState<ValidationResult | null>(null);
  const [validationRunning, setValidationRunning] = useState(false);
  // EFI Intelligence Report + Community Knowledge
  const [efiReport, setEfiReport] = useState<EfiReport | null>(null);
  const [communityIssues, setCommunityIssues] = useState<CommunityIssue[]>([]);
  const [showEfiReport, setShowEfiReport] = useState(false);

  const buildKextSourceMap = (results: KextFetchResult[] = kextResults): Record<string, 'github' | 'embedded' | 'direct' | 'failed'> =>
    Object.fromEntries(
      results
        .filter((result): result is KextFetchResult & { source: 'github' | 'embedded' | 'direct' | 'failed' } => !!result.source)
        .map(result => [result.name, result.source]),
    );

  const describeValidationFailure = (result: ValidationResult): string => {
    const first = result.issues.find(issue => issue.severity === 'blocked') ?? result.issues[0];
    if (!first) return 'EFI validation failed.';
    return `EFI validation failed: ${first.component} at ${first.expectedPath} — ${first.actualCondition}.`;
  };

  const buildInterpretationMetadata = (
    interpretation: import('../electron/hardwareInterpret').HardwareInterpretation | null,
  ): HardwareProfileInterpretationMetadata | null => {
    if (!interpretation) return null;
    return {
      overallConfidence: interpretation.overallConfidence,
      summary: interpretation.summary,
      manualVerificationNeeded: interpretation.manualVerificationNeeded,
    };
  };

  useEffect(() => {
    latestProfileRef.current = profile;
  }, [profile]);

  useEffect(() => {
    latestPlanningContextRef.current = planningProfileContext;
  }, [planningProfileContext]);

  useEffect(() => {
    latestEfiPathRef.current = efiPath;
  }, [efiPath]);

  useEffect(() => {
    let cancelled = false;
    const initUpdates = async () => {
      await refreshAppUpdateState();
      if (!cancelled) {
        await checkForAppUpdates();
      }
    };
    void initUpdates();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!shouldRefreshAppUpdateState(appUpdateState)) return;
    const interval = setInterval(() => {
      void refreshAppUpdateState();
    }, APP_UPDATE_REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [appUpdateState]);

  // ── Suggestion Engine State ──────────────────────────────────
  const [lastSuggestion, setLastSuggestion] = useState<{ code: string; category: string; title: string } | null>(null);
  const errorCountRef = useRef<Record<string, number>>({});

  // ── Prevention Layer State ─────────────────────────────────────
  const [preflightReport, setPreflightReport] = useState<PreflightReport | null>(null);
  const [preflightRunning, setPreflightRunning] = useState(false);
  const [confidence, setConfidence] = useState<ConfidenceLevel>('green');

  // ── Deterministic Layer State ─────────────────────────────────
  const [buildPlan, setBuildPlan] = useState<BuildPlan | null>(null);
  const [recoveryDryRun, setRecoveryDryRun] = useState<RecoveryDryRun | null>(null);
  const [certainty, setCertainty] = useState<Certainty>('may_fail');
  const [resourcePlan, setResourcePlan] = useState<ResourcePlan | null>(null);
  const [safeSimulationResult, setSafeSimulationResult] = useState<SafeSimulationResult | null>(null);
  const [simulationRunning, setSimulationRunning] = useState(false);
  const [buildFlow, setBuildFlow] = useState<BuildFlowSnapshot | null>(null);
  const [buildFlowAlert, setBuildFlowAlert] = useState<{
    level: 'taking_longer' | 'stalled';
    reason: string;
    pendingCondition: string | null;
  } | null>(null);
  const buildFlowRef = useRef<BuildFlowSnapshot | null>(null);
  const buildRunIdRef = useRef(0);
  const buildStartRequestedRef = useRef(false);
  const biosAcceptedRef = useRef(false);
  const biosRefreshRequestIdRef = useRef(0);
  const buildAutoStartRef = useRef(false);
  const lastStableResourcePlanRef = useRef<ResourcePlan | null>(null);
  const lastStableResourcePlanOwnerRef = useRef<string | null>(null);

  // ── Debug Overlay ──────────────────────────────────────────────
  const [debugOpen, setDebugOpen] = useState(false);
  const [debugSessionId, setDebugSessionId] = useState('');
  const [recentEvents, setRecentEvents] = useState<any[]>([]);
  const [watchdogCount, setWatchdogCount] = useState(0);
  const [recoveryTryCount, setRecoveryTryCount] = useState(0);
  const [biosAccepted, setBiosAccepted] = useState(false);
  const [flashConfirmBusy, setFlashConfirmBusy] = useState(false);
  const hasLiveHardwareContext = planningProfileContext === 'live_scan';
  const biosReady = biosState?.readyToBuild === true && biosState?.stage === 'complete';
  const compatibilityBlocked = isCompatibilityBlocked(compat);
  const validationBlocked = isValidationBlockingDeployment(validationResult);
  const biosFlowState = useMemo(
    () => deriveBiosFlowState({
      stage: biosState?.stage,
      readyToBuild: biosState?.readyToBuild,
    }),
    [biosState?.readyToBuild, biosState?.stage],
  );
  const releaseFlowState = useMemo(
    () => deriveReleaseFlowState({
      step,
      hasProfile: !!profile,
      compatibilityBlocked,
      biosFlowState,
      buildReady,
      hasEfi: !!efiPath,
      validationBlocked,
    }),
    [biosFlowState, buildReady, compatibilityBlocked, efiPath, profile, step, validationBlocked],
  );
  const compatibilityMatrix = useMemo(
    () => (profile ? buildCompatibilityMatrix(profile) : null),
    [profile],
  );
  const resourcePlanOwnerKey = useMemo(
    () => buildResourcePlanOwnerKey(profile),
    [profile],
  );
  const visibleResourcePlan = useMemo(
    () => resolveVisibleResourcePlan(
      resourcePlan,
      resourcePlanOwnerKey,
      lastStableResourcePlanRef.current,
      lastStableResourcePlanOwnerRef.current,
    ),
    [resourcePlan, resourcePlanOwnerKey],
  );
  const localBuildGuard = useMemo(
    () => evaluateBuildGuard({
      compatibilityBlocked,
      biosFlowState,
      biosAccepted,
      releaseFlowState,
    }),
    [biosAccepted, biosFlowState, compatibilityBlocked, releaseFlowState],
  );
  const localDeployGuard = useMemo(
    () => evaluateDeployGuard({
      compatibilityBlocked,
      biosFlowState,
      releaseFlowState,
      validationBlocked,
      hasEfi: !!efiPath,
    }),
    [biosFlowState, compatibilityBlocked, efiPath, releaseFlowState, validationBlocked],
  );
  const postBuildReady = !compatibilityBlocked && (biosReady || biosAccepted) && buildReady && !!efiPath && !validationBlocked;

  const setBiosAcceptedRuntime = (next: boolean) => {
    biosAcceptedRef.current = next;
    setBiosAccepted(next);
  };

  useEffect(() => {
    setBiosAcceptedRuntime(false);
  }, [biosState?.hardwareFingerprint]);

  useEffect(() => {
    if (!profile) return;
    const nextCompat = checkCompatibility(profile);
    setCompat(nextCompat);
    setProfile((currentProfile) => {
      if (!currentProfile || currentProfile.strategy === nextCompat.strategy) return currentProfile;
      return { ...currentProfile, strategy: nextCompat.strategy };
    });
  }, [profile]);

  useEffect(() => {
    if (!profile || !compat || !validationResult || !buildReady) return;
    try {
      setEfiReport(generateEfiReport(profile, compat, kextResults, validationResult));
    } catch (error) {
      debugWarn('[efi-report] Failed to refresh report after compatibility refresh:', error);
    }
  }, [buildReady, compat, kextResults, profile, validationResult]);

  // ── Task Manager ────────────────────────────────────────────────
  const { tasks, activeTask, cancelTask } = useTaskManager();
  const activeBuildRunStartedAt = buildFlow?.startedAt ?? null;
  const latestTasks = useMemo(() => {
    const latest = new Map<TaskKind, TaskState>();
    for (const task of tasks.values()) {
      const current = latest.get(task.kind);
      if (!current || task.startedAt > current.startedAt || task.lastUpdateAt > current.lastUpdateAt) {
        latest.set(task.kind, task);
      }
    }
    return latest;
  }, [tasks]);
  const recovTask = activeTask('recovery-download');
  const kextTask  = activeTask('kext-fetch');
  const efiTask   = activeTask('efi-build');
  const flashTask = activeTask('usb-flash');
  const latestEfiTask = activeBuildRunStartedAt == null
    ? latestTaskByKind(tasks.values(), 'efi-build') ?? latestTasks.get('efi-build')
    : latestTaskByKindSince(tasks.values(), 'efi-build', activeBuildRunStartedAt);
  const latestKextTask = activeBuildRunStartedAt == null
    ? latestTaskByKind(tasks.values(), 'kext-fetch') ?? latestTasks.get('kext-fetch')
    : latestTaskByKindSince(tasks.values(), 'kext-fetch', activeBuildRunStartedAt);
  const latestRecoveryTask = activeBuildRunStartedAt == null
    ? latestTaskByKind(tasks.values(), 'recovery-download') ?? latestTasks.get('recovery-download')
    : latestTaskByKindSince(tasks.values(), 'recovery-download', activeBuildRunStartedAt);

  const isImportingRef = useRef(false);
  const isRetryingRecovRef = useRef(false);
  const isFlashingRef = useRef(false);
  const lastRuntimeErrorRef = useRef<string | null>(null);
  const logUiEvent = (eventName: string, detail?: Record<string, unknown> | null) => {
    window.electron?.logUiEvent?.(eventName, detail ?? null).catch(() => {});
  };
  const updateBuildFlow = (
    updater: BuildFlowSnapshot | null | ((current: BuildFlowSnapshot | null) => BuildFlowSnapshot | null),
  ) => {
    setBuildFlow((current) => {
      const next = typeof updater === 'function'
        ? updater(current)
        : updater;
      buildFlowRef.current = next;
      return next;
    });
  };
  const buildStepGuardState = (overrides?: Partial<StepGuardState>): StepGuardState => ({
    profile,
    compat,
    hasLiveHardwareContext,
    biosReady,
    biosAccepted,
    buildReady,
    efiPath,
    biosConf,
    selectedUsb,
    compatibilityBlocked,
    validationBlocked,
    postBuildReady,
    localBuildGuard,
    localDeployGuard,
    ...(overrides ?? {}),
  });
  const describeBuildFlowFailure = (reason: string, targetStep?: StepId) => {
    const snapshot = buildFlowRef.current;
    const pending = snapshot?.pendingRendererExpectation
      ? `The app was still waiting for ${snapshot.pendingRendererExpectation}.`
      : 'The app did not receive the terminal build signal it expected.';
    return JSON.stringify({
      message: snapshot?.phase === 'stalled' ? 'Build stalled' : 'Build interrupted',
      explanation: reason,
      decisionSummary: pending,
      suggestion: 'Retry the build. If it stalls again, copy the report and include the saved support log.',
      contextNote: buildBuildFlowContext(snapshot),
      code: 'build_flow_stalled',
      severity: 'error',
      targetStep: targetStep ?? snapshot?.uiStep ?? step,
      rawMessage: reason,
    });
  };
  const triggerBuildFlowRecovery = (reason: string, pendingCondition?: string | null, targetStep?: StepId) => {
    const snapshot = buildFlowRef.current;
    logUiEvent('build_flow_stalled', {
      phase: snapshot?.phase ?? 'unknown',
      uiStep: snapshot?.uiStep ?? step,
      activeTaskKind: snapshot?.activeTaskKind ?? null,
      activeTaskStatus: snapshot?.activeTaskStatus ?? null,
      pendingRendererExpectation: pendingCondition ?? snapshot?.pendingRendererExpectation ?? null,
      reason,
    });
    buildRunIdRef.current += 1;
    isDeployingRef.current = false;
    buildStartRequestedRef.current = false;
    updateBuildFlow((current) => current ? {
      ...current,
      active: false,
      phase: 'stalled',
      stalledReason: reason,
    } : current);
    setBuildFlowAlert({
      level: 'stalled',
      reason,
      pendingCondition: pendingCondition ?? snapshot?.pendingRendererExpectation ?? null,
    });
    setGlobalError(describeBuildFlowFailure(reason, targetStep));
  };
  const advanceToMethodSelect = (resolvedEfiPath?: string | null) => {
    const nextEfiPath = resolvedEfiPath ?? efiPath;
    const transition = attemptStepTransition('method-select', {
      buildReady: true,
      efiPath: nextEfiPath,
      validationBlocked: false,
      postBuildReady: Boolean(nextEfiPath) && !compatibilityBlocked && (biosReady || biosAcceptedRef.current),
    });
    if (!transition?.ok) {
      setGlobalError(describeBuildFlowFailure(
        transition?.reason ?? 'The build completed, but the next installation step could not open.',
        transition?.redirect ?? 'report',
      ));
      return false;
    }
    return true;
  };
  const handleImportRecovery = async () => {
    if (!efiPath || !profile?.targetOS || isImportingRef.current) return;
    isImportingRef.current = true;
    setRecovError(null);
    try {
      const res = await window.electron.importRecovery(efiPath, profile.targetOS);
      if (res) {
        setRecovPct(100);
        setRecovStatus('Recovery imported manually.');
        advanceToMethodSelect(efiPath);
      } else {
        setRecovError('Import returned no result — the file may be invalid or missing.');
      }
    } catch (e: any) {
      const msg = e.message || 'Recovery import failed';
      setRecovError(msg);
      setErrorWithSuggestion(msg, 'recovery-download');
    } finally { isImportingRef.current = false; }
  };

  const openLatestReleasePage = async () => {
    try {
      await window.electron.openLatestReleasePage();
    } catch (e: any) {
      setErrorWithSuggestion(e?.message || 'Could not open the latest release page.', step);
    }
  };

  const refreshAppUpdateState = async () => {
    try {
      setAppUpdateState(await window.electron.getAppUpdateState());
    } catch {
      // keep updater non-fatal to the main flow
    }
  };

  const withOptimisticAppUpdateState = (
    updater: (current: ElectronAppUpdateState) => ElectronAppUpdateState,
  ) => {
    setAppUpdateState((current) => updater(current ?? {
      currentVersion: 'unknown',
      checking: false,
      downloading: false,
      installing: false,
      lastCheckedAt: null,
      available: false,
      supported: platform === 'win32' || platform === 'linux',
      latestVersion: null,
      releaseUrl: null,
      releaseNotes: null,
      assetName: null,
      assetSize: null,
      downloadedBytes: 0,
      totalBytes: null,
      downloadedPath: null,
      readyToInstall: false,
      restartRequired: false,
      error: null,
    }));
  };

  const checkForAppUpdates = async () => {
    try {
      withOptimisticAppUpdateState((current) => ({
        ...current,
        checking: true,
        error: null,
      }));
      setAppUpdateState(await window.electron.checkForUpdates());
    } catch (e: any) {
      setErrorWithSuggestion(e?.message || 'Could not check for updates.', step);
    }
  };

  const downloadLatestUpdate = async () => {
    try {
      withOptimisticAppUpdateState((current) => ({
        ...current,
        checking: false,
        downloading: true,
        installing: false,
        readyToInstall: false,
        restartRequired: false,
        error: null,
      }));
      setAppUpdateState(await window.electron.downloadLatestUpdate());
    } catch (e: any) {
      setErrorWithSuggestion(e?.message || 'Could not download the latest update.', step);
    }
  };

  const installLatestUpdate = async () => {
    try {
      withOptimisticAppUpdateState((current) => ({
        ...current,
        checking: false,
        downloading: false,
        installing: true,
        error: null,
      }));
      setAppUpdateState(await window.electron.installLatestUpdate());
    } catch (e: any) {
      setErrorWithSuggestion(e?.message || 'Could not install the downloaded update.', step);
    }
  };

  const quitForUpdate = async () => {
    try {
      await window.electron.quitForUpdate();
    } catch (e: any) {
      setErrorWithSuggestion(e?.message || 'Could not restart the app to finish the update.', step);
    }
  };

  const handlePrimaryUpdateAction = () => {
    if (appUpdateState?.restartRequired) {
      void quitForUpdate();
      return;
    }
    if (appUpdateState?.readyToInstall) {
      void installLatestUpdate();
      return;
    }
    if (appUpdateState?.available) {
      void downloadLatestUpdate();
      return;
    }
    void checkForAppUpdates();
  };

  const classifyRetryBucket = (errorMessage: string, trace?: ValidationTrace | null) => {
    const msgLower = errorMessage.toLowerCase();
    if (trace?.code) return trace.code;
    if (msgLower.includes('401') || msgLower.includes('403')) return 'recovery_auth';
    if (msgLower.includes('recovery')) return 'recovery_dl';
    if (msgLower.includes('efi') && msgLower.includes('valid')) return 'efi_val';
    if (msgLower.includes('permission denied') || msgLower.includes('eacces') || msgLower.includes('eperm') || msgLower.includes('administrator') || msgLower.includes('sudo')) {
      return 'flash_permission';
    }
    if (msgLower.includes('timed out') || msgLower.includes('timeout')) return 'flash_timeout';
    if (msgLower.includes('verification failed') || msgLower.includes('not found on usb after copy') || msgLower.includes('not found on usb after copy')) {
      return 'flash_verify';
    }
    if (msgLower.includes('write-protect') || msgLower.includes('write protect') || msgLower.includes('i/o error') || msgLower.includes('input/output error') || msgLower.includes('media is write protected') || msgLower.includes('device rejected write')) {
      return 'flash_media';
    }
    if (msgLower.includes('flash') || msgLower.includes('write') || msgLower.includes('diskpart') || msgLower.includes('xcopy') || msgLower.includes('mkfs') || msgLower.includes('mount') || msgLower.includes('umount')) {
      return 'flash_general';
    }
    return 'other';
  };

  /** Set a global error with context-aware suggestion.
   *  Tracks retry counts per error code so suggestions evolve on repeated failures. */
  const setErrorWithSuggestion = (
    errorMessage: string,
    overrideStep?: string,
    options?: {
      validationResult?: ValidationResult | null;
      kextSources?: Record<string, 'github' | 'embedded' | 'direct' | 'failed'>;
    },
  ) => {
    setGlobalNotice(null);
    const trace = options?.validationResult?.firstFailureTrace ?? validationResult?.firstFailureTrace ?? null;
    // Pre-compute a rough code for retry counting (before full payload build)
    const msgLower = errorMessage.toLowerCase();
    const roughCode = classifyRetryBucket(errorMessage, trace);
    errorCountRef.current[roughCode] = (errorCountRef.current[roughCode] ?? 0) + 1;

    const payload = getSuggestionPayload({
      errorMessage,
      profile,
      platform,
      step: overrideStep ?? step,
      diskInfo: diskInfo ?? null,
      retryCount: errorCountRef.current[roughCode] - 1,
      validationIssues: options?.validationResult?.issues ?? validationResult?.issues,
      validationTrace: trace,
      kextSources: options?.kextSources ?? buildKextSourceMap(),
    });
    if (payload.code) {
      setLastSuggestion({ code: payload.code, category: payload.category ?? 'unknown', title: payload.message });
    }
    const serialized = JSON.stringify({
      ...payload,
      targetStep: overrideStep ?? step,
      rawMessage: errorMessage,
    });
    setGlobalError(serialized);
    logUiEvent('error_surface_opened', {
      step: overrideStep ?? step,
      code: payload.code ?? roughCode,
      message: payload.message,
      rawMessage: errorMessage,
    });
  };

  useEffect(() => {
    errorCountRef.current.flash_general = 0;
    errorCountRef.current.flash_media = 0;
    errorCountRef.current.flash_permission = 0;
    errorCountRef.current.flash_timeout = 0;
    errorCountRef.current.flash_verify = 0;
  }, [selectedUsb]);

  const openBiosRecoverySurface = (
    code: BiosRecoveryCode,
    options?: {
      detail?: string | null;
      state?: Pick<BiosOrchestratorState, 'blockingIssues' | 'settings'> | null;
    },
  ) => {
    const payload = buildBiosRecoveryPayload({
      code,
      detail: options?.detail ?? null,
      state: options?.state ?? null,
    });
    setGlobalNotice(null);
    setGlobalError(JSON.stringify(payload));
    logUiEvent('error_surface_opened', {
      step: 'bios',
      code: payload.code ?? code,
      message: payload.message,
      rawMessage: options?.detail ?? payload.explanation ?? payload.message,
    });
  };

  useEffect(() => {
    const onRuntimeError = (event: Event) => {
      const detail = (event as CustomEvent<{ type?: string; message?: string }>).detail ?? {};
      const message = detail.message?.trim() || 'Unexpected runtime error in the application UI.';
      const key = `${detail.type ?? 'runtime'}:${message}`;
      if (lastRuntimeErrorRef.current === key) return;
      lastRuntimeErrorRef.current = key;
      setErrorWithSuggestion(`Unexpected runtime error: ${message}`, step);
    };

    window.addEventListener('moc:runtime-error', onRuntimeError as EventListener);
    return () => window.removeEventListener('moc:runtime-error', onRuntimeError as EventListener);
  }, [setErrorWithSuggestion, step]);

  useEffect(() => {
    if (step === 'landing') return;
    logUiEvent('step_changed', { step });
  }, [step]);

  useEffect(() => {
    if (step !== 'building' && step !== 'kext-fetch' && step !== 'recovery-download') {
      setBuildFlowAlert((current) => current?.level === 'taking_longer' ? null : current);
    }
  }, [step]);

  const invalidateGeneratedBuild = () => {
    buildRunIdRef.current += 1;
    buildFlowRef.current = null;
    setBuildFlow(null);
    setBuildFlowAlert(null);
    setEfiPath(null);
    setBuildReady(false);
    setValidationResult(null);
    setKextResults([]);
    setRecovPct(0);
    setRecovStatus('');
    setRecovError(null);
    setRecovOffset(0);
    setRecovDmgDest(null);
    setRecovClDest(null);
    setCachedRecovInfo(null);
    clearSelectedUsbState();
    setFlashMilestones([]);
    setRecoveryDryRun(null);
    setShowFlashConfirm(false);
    setShowPartitionConfirm(false);
    setFlashConfirmationToken(null);
    setFlashConfirmationExpiresAt(null);
    setFlashConfirmText('');
    setFlashChecks(new Set());
    setEfiReport(null);
    setCommunityIssues([]);
    setShowEfiReport(false);
    setResourcePlan(null);
    setSafeSimulationResult(null);
    setEfiBackupPolicy(null);
  };

  const clearFlashConfirmationState = () => {
    setShowFlashConfirm(false);
    setFlashConfirmationToken(null);
    setFlashConfirmationExpiresAt(null);
    setFlashConfirmText('');
    setFlashChecks(new Set());
    setFlashConfirmBusy(false);
  };

  const clearSelectedUsbState = () => {
    selectedUsbRef.current = null;
    setSelectedUsb(null);
    setDiskInfo(null);
    setEfiBackupPolicy(null);
    clearFlashConfirmationState();
  };

  const handleUsbSelection = (device: string | null) => {
    selectedUsbInfoRequestRef.current += 1;
    if (!device) {
      clearSelectedUsbState();
      return;
    }
    selectedUsbRef.current = device;
    setSelectedUsb(device);
    setDiskInfo(null);
    setEfiBackupPolicy(null);
    clearFlashConfirmationState();
  };

  const setDiskInfoIfCurrent = (device: string, info: RendererDiskInfo) => {
    if (selectedUsbRef.current === device) {
      setDiskInfo(info);
    }
  };

  const resolveDiskInfoForDevice = async (
    device: string,
    options?: { retries?: number; preferCaptured?: boolean },
  ): Promise<RendererDiskInfo | null> => {
    const retries = options?.retries ?? 2;
    const captured = pickSelectedDiskInfo(device, diskInfo, null);
    for (let attempt = 0; attempt < retries; attempt += 1) {
      try {
        const info = await window.electron.getDiskInfo(device);
        setDiskInfoIfCurrent(device, info);
        return info;
      } catch (error) {
        if (!shouldRetryDiskInfoLookup(error, attempt, retries)) break;
        await new Promise((resolve) => setTimeout(resolve, 350));
      }
    }
    if (options?.preferCaptured !== false && captured) {
      return captured;
    }
    return null;
  };

  const getBuildGuardRedirect = (activeCompat: CompatibilityReport | null | undefined): StepId =>
    isCompatibilityBlocked(activeCompat) ? 'report' : 'bios';

  const getDeployGuardRedirect = (
    activeCompat: CompatibilityReport | null | undefined,
    validation?: ValidationResult | null,
  ): StepId => {
    if (isCompatibilityBlocked(activeCompat) || isValidationBlockingDeployment(validation) || !efiPath || !buildReady) {
      return 'report';
    }
    return 'bios';
  };

  const ensureBuildGuard = async (
    activeProfile: HardwareProfile,
    options?: { surfaceError?: boolean },
  ): Promise<FlowGuardResult> => {
    const activeCompat = checkCompatibility(activeProfile);
    setCompat(activeCompat);

    if (!hasLiveHardwareContext) {
      const reason = 'Imported or restored hardware profiles are planning inputs only. Run a live hardware scan in this session before BIOS, build, or deployment actions.';
      if (options?.surfaceError !== false) {
        setErrorWithSuggestion(reason, 'report');
        _setStepRaw('report');
      }
      return {
        allowed: false,
        reason,
        currentState: 'report',
        biosState: 'idle',
      };
    }

    const guard = await window.electron.guardBuild(activeProfile, biosAcceptedRef.current);
    if (!guard.allowed && options?.surfaceError !== false) {
      const redirect = getBuildGuardRedirect(activeCompat);
      setGlobalNotice(null);
      setGlobalError(JSON.stringify({
        code: 'build_blocked_by_guard',
        message: 'EFI build is blocked',
        explanation: guard.reason ?? 'Build is blocked by the current firmware or compatibility state.',
        decisionSummary: guard.reason ?? 'The EFI build cannot start from the current release state.',
        suggestion: redirect === 'bios'
          ? 'Return to the BIOS step and use Continue or Recheck BIOS before building again.'
          : 'Return to the report step and fix the blocking prerequisite before rebuilding.',
        category: 'build_error',
        severity: 'warning',
        targetStep: redirect,
        rawMessage: guard.reason ?? 'Build is blocked by the current firmware or compatibility state.',
      }));
      logUiEvent('error_surface_opened', {
        step: redirect,
        code: 'build_blocked_by_guard',
        message: 'EFI build is blocked',
        rawMessage: guard.reason ?? 'Build is blocked by the current firmware or compatibility state.',
      });
      _setStepRaw(redirect);
    }
    return guard;
  };

  const ensureDeployGuard = async (
    activeProfile: HardwareProfile,
    activeEfiPath: string,
    options?: { surfaceError?: boolean; reasonSuffix?: string },
  ): Promise<{ guard: FlowGuardResult; validation: ValidationResult | null }> => {
    const activeCompat = checkCompatibility(activeProfile);
    setCompat(activeCompat);

    if (!hasLiveHardwareContext) {
      const reason = 'Imported or restored hardware profiles are planning inputs only. Run a live hardware scan in this session before any deploy or write action.';
      if (options?.surfaceError !== false) {
        setErrorWithSuggestion(reason, 'report');
        _setStepRaw('report');
      }
      return {
        guard: {
          allowed: false,
          reason,
          currentState: 'report',
          biosState: 'idle',
        },
        validation: null,
      };
    }

    const guard = await window.electron.guardDeploy(activeProfile, activeEfiPath);
    let validation: ValidationResult | null = null;

    if (!guard.allowed) {
      try {
        validation = await window.electron.validateEfi(activeEfiPath, activeProfile);
        setValidationResult(validation);
        if (isValidationBlockingDeployment(validation)) {
          setBuildReady(false);
        }
      } catch {
        validation = null;
      }

      if (options?.surfaceError !== false) {
        const redirect = getDeployGuardRedirect(activeCompat, validation);
        const reason = guard.reason ?? 'Deployment is blocked by the current build state.';
        setErrorWithSuggestion(`${reason}${options?.reasonSuffix ?? ''}`, redirect, {
          validationResult: validation,
        });
        _setStepRaw(redirect);
      }
    }

    return { guard, validation };
  };

  // ── Guarded step transition ─────────────────────────────────────
  // Validates prerequisites before allowing navigation to a new step.
  // If prerequisites fail, shows an error and redirects to a safe earlier step.
  const attemptStepTransition = (target: StepId, overrides?: Partial<StepGuardState>) => {
    const result = evaluateStepTransitionWithOverrides(target, buildStepGuardState(), overrides);
    if (!result.ok) {
      debugWarn(`[guard] Blocked transition to "${target}": ${result.reason}`);
      logUiEvent('step_transition_blocked', { target, redirect: result.redirect ?? null, reason: result.reason ?? 'blocked' });
      if (result.redirect) _setStepRaw(result.redirect);
      return result;
    }
    logUiEvent('step_transition_allowed', { target });
    _setStepRaw(target);
    return result;
  };

  const setStep = (target: StepId) => {
    const result = attemptStepTransition(target);
    if (!result?.ok) {
      return;
    }
  };

  const refreshBiosState = async (
    activeProfile: HardwareProfile,
    options?: { redirectIfBlocked?: boolean; assumeLiveHardwareContext?: boolean; surfaceError?: boolean },
  ) => {
    const requestId = biosRefreshRequestIdRef.current + 1;
    biosRefreshRequestIdRef.current = requestId;
    const liveContext = options?.assumeLiveHardwareContext === true
      || latestPlanningContextRef.current === 'live_scan';
    if (!liveContext) {
      setBiosState(null);
      setBiosAcceptedRuntime(false);
      return null;
    }
    try {
      const nextState = await window.electron.getBiosState(activeProfile);
      if (biosRefreshRequestIdRef.current !== requestId) {
        return nextState;
      }
      setBiosState(nextState);
      setBiosAcceptedRuntime(false);
      if (options?.redirectIfBlocked && (!(nextState.readyToBuild && nextState.stage === 'complete')) && STEP_ORDER.indexOf(step) > STEP_ORDER.indexOf('bios')) {
        _setStepRaw('bios');
      }
      return nextState;
    } catch (e: any) {
      if (options?.surfaceError !== false) {
        openBiosRecoverySurface('bios_recheck_failed', {
          detail: e?.message || 'Failed to evaluate BIOS preparation state.',
        });
      }
      return null;
    }
  };

  // ── Debug Overlay: keyboard shortcut + session ID ──────────────
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === 'D') {
        e.preventDefault();
        setDebugOpen(x => !x);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);

  useEffect(() => {
    if (window.electron?.getSessionId) {
      window.electron.getSessionId().then(setDebugSessionId).catch(() => {});
    }
  }, []);

  useEffect(() => {
    if (step === 'recovery-download' && profile?.targetOS) {
      window.electron.getCachedRecoveryInfo(profile.targetOS)
        .then(setCachedRecovInfo)
        .catch(() => setCachedRecovInfo(null));
    }
  }, [step, profile?.targetOS]);

  useEffect(() => {
    if (!profile) {
      setResourcePlan(null);
      lastStableResourcePlanRef.current = null;
      lastStableResourcePlanOwnerRef.current = null;
      return;
    }

    let cancelled = false;
    // Keep showing the previous plan while the new one loads (avoid blank flash)
    window.electron.getResourcePlan(profile, efiPath)
      .then((plan) => {
        if (!cancelled) {
          setResourcePlan(plan);
          lastStableResourcePlanRef.current = plan;
          lastStableResourcePlanOwnerRef.current = resourcePlanOwnerKey;
        }
      })
      .catch(() => {
        if (!cancelled) setResourcePlan(null);
      });

    return () => {
      cancelled = true;
    };
  }, [efiPath, kextResults, profile, resourcePlanOwnerKey, validationResult?.checkedAt]);

  useEffect(() => {
    selectedUsbRef.current = selectedUsb;
    if (!selectedUsb) {
      setDiskInfo(null);
      return;
    }

    let cancelled = false;
    const requestId = selectedUsbInfoRequestRef.current + 1;
    selectedUsbInfoRequestRef.current = requestId;

    const captureIdentity = async () => {
      const info = await resolveDiskInfoForDevice(selectedUsb, { retries: 2, preferCaptured: false }).catch(() => null);
      if (!cancelled && selectedUsbInfoRequestRef.current === requestId && info) {
        setDiskInfoIfCurrent(selectedUsb, info);
      }
    };

    void captureIdentity();

    return () => {
      cancelled = true;
    };
  }, [selectedUsb]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (step !== 'usb-select' || !selectedUsb) {
      setEfiBackupPolicy(null);
      return;
    }

    let cancelled = false;
    window.electron.inspectEfiBackupPolicy(selectedUsb)
      .then((policy) => {
        if (!cancelled) setEfiBackupPolicy(policy);
      })
      .catch((error: any) => {
        if (!cancelled) {
          setEfiBackupPolicy({
            status: 'blocked',
            reason: error?.message || 'Existing EFI could not be inspected safely on the selected target.',
            existingEfiState: 'unreadable',
            latestBackup: null,
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [selectedUsb, step]);

  useEffect(() => {
    if (step === 'bios' && profile) {
      refreshBiosState(profile).catch(() => {});
      window.electron.getBiosRestartCapability()
        .then(setRestartCapability)
        .catch(() => setRestartCapability(null));
      window.electron.getBiosResumeState()
        .then(setBiosResumeState)
        .catch(() => setBiosResumeState(null));
    }
  }, [step, profile]); // eslint-disable-line react-hooks/exhaustive-deps

  // Handle telemetry updates for debug panel
  useEffect(() => {
    if (!debugOpen) return;
    
    const update = async () => {
      try {
        const ops = await window.electron.getOpsTail(50);
        setRecentEvents(ops.slice(-10).reverse());
        setWatchdogCount(ops.filter((e: any) => e.kind === 'watchdog_trigger').length);
        setRecoveryTryCount(ops.filter((e: any) => e.kind === 'recovery_attempt').length);
      } catch {}
    };

    update();
    const interval = setInterval(update, 2000);
    return () => clearInterval(interval);
  }, [debugOpen]);

  // Handle system alerts (Phase 5 hardening)
  useEffect(() => {
    const alerts = Array.from(tasks.values()).filter(t => (t.kind as string) === 'system-alert' && t.status === 'failed');
    if (alerts.length > 0) {
      const latest = alerts[alerts.length - 1];
      if (latest.error) setErrorWithSuggestion(latest.error);
    }
  }, [tasks]);

  // Drive recovPct / recovStatus / recovOffset from the live task state
  useEffect(() => {
    const p = recovTask?.progress as { kind: string; percent?: number; status?: string; bytesDownloaded?: number; dmgDest?: string; clDest?: string } | null | undefined;
    if (!p) return;
    if (p.percent !== undefined) setRecovPct(p.percent);
    if (p.status)                setRecovStatus(p.status);
    if (p.dmgDest) setRecovDmgDest(p.dmgDest);
    if (p.clDest !== undefined) setRecovClDest(p.clDest || null);
    if (p.bytesDownloaded !== undefined) setRecovOffset(p.bytesDownloaded);

    const persistedDmgDest = p.dmgDest ?? recovDmgDest;
    const persistedClDest = p.clDest !== undefined ? (p.clDest || null) : recovClDest;
    if ((p.bytesDownloaded && p.bytesDownloaded > 0) || persistedDmgDest) {
      const now = Date.now();
      if (now - lastRecovSaveRef.current > 5000) {
        lastRecovSaveRef.current = now;
        const latestProfile = latestProfileRef.current;
        const latestEfiPath = latestEfiPathRef.current;
        try {
          window.electron.saveState({
            currentStep: 'recovery-download',
            profile: latestProfile,
            timestamp: now,
            efiPath: latestEfiPath ?? undefined,
            recoveryDownloadOffset: p.bytesDownloaded ?? recovOffset,
            recoveryDmgDest: persistedDmgDest ?? undefined,
            recoveryClDest: persistedClDest ?? undefined,
            recoveryTargetOS: latestProfile?.targetOS || 'macOS Sequoia 15',
          });
        } catch {}
      }
    }
    updateBuildFlow((current) => {
      if (!current?.active || current.activeTaskKind !== 'recovery-download') return current;
      return {
        ...current,
        activeTaskStatus: recovTask?.status ?? current.activeTaskStatus,
        lastProgressAt: Date.now(),
        lastTaskPhase: p.status ?? current.lastTaskPhase,
        taskCompleteEventFired: recovTask?.status === 'complete',
        stalledReason: null,
      };
    });
  }, [recovTask?.progress, recovClDest, recovDmgDest, recovOffset]); // eslint-disable-line react-hooks/exhaustive-deps

  // Drive kextResults / progress from the live kext task state
  useEffect(() => {
    const p = kextTask?.progress as { kind: string; kextName?: string; version?: string; index?: number; total?: number } | null | undefined;
    const snapshot = buildFlowRef.current;
    if (!p || p.kind !== 'kext-fetch' || !taskBelongsToRun(kextTask, snapshot?.startedAt)) return;
    if (p.kextName) {
      setKextResults(prev => prev.find(k => k.name === p.kextName) ? prev : [...prev, { name: p.kextName!, version: p.version ?? '' }]);
    }
    if (p.index !== undefined && p.total) {
      setProgress(Math.round((p.index / p.total) * 100));
    }
    updateBuildFlow((current) => {
      if (!current?.active || current.activeTaskKind !== 'kext-fetch') return current;
      return {
        ...current,
        activeTaskStatus: kextTask?.status ?? current.activeTaskStatus,
        lastProgressAt: Date.now(),
        lastTaskPhase: p.kextName ? `${p.kextName} ${p.version ?? ''}`.trim() : current.lastTaskPhase,
        taskCompleteEventFired: kextTask?.status === 'complete',
        stalledReason: null,
      };
    });
  }, [kextTask?.progress]); // eslint-disable-line react-hooks/exhaustive-deps

  // Drive building progress from the live EFI build task state
  useEffect(() => {
    const p = efiTask?.progress as { kind: string; phase?: string; detail?: string } | null | undefined;
    const snapshot = buildFlowRef.current;
    if (!p || !taskBelongsToRun(efiTask, snapshot?.startedAt)) return;
    if (p.phase) setStatus(p.phase);
    updateBuildFlow((current) => {
      if (!current?.active || current.activeTaskKind !== 'efi-build') return current;
      return {
        ...current,
        activeTaskStatus: efiTask?.status ?? current.activeTaskStatus,
        lastProgressAt: Date.now(),
        lastTaskPhase: p.phase ?? current.lastTaskPhase,
        taskCompleteEventFired: efiTask?.status === 'complete',
        stalledReason: null,
      };
    });
  }, [efiTask?.progress, efiTask?.status]);

  // Drive flash progress from the live usb-flash task state
  useEffect(() => {
    const p = flashTask?.progress as { kind: string; phase?: string; detail?: string } | null | undefined;
    if (!p || p.kind !== 'usb-flash') return;
    const phase = p.phase;
    if (phase === 'erase' || phase === 'format') setProgress(20);
    else if (phase === 'copy') setProgress(60);
    else if (phase === 'verify') { setProgress(90); setStatus('flash_verify'); }
    else if (phase === 'eject') setProgress(95);
  }, [flashTask?.progress]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const snapshot = buildFlowRef.current;
    const task = snapshot?.activeTaskKind === 'efi-build'
      ? latestEfiTask
      : snapshot?.activeTaskKind === 'kext-fetch'
      ? latestKextTask
      : snapshot?.activeTaskKind === 'recovery-download'
      ? latestRecoveryTask
      : null;
    if (!task || !taskBelongsToRun(task, snapshot?.startedAt)) return;
    updateBuildFlow((current) => {
      if (!current?.active || current.activeTaskKind !== task.kind) return current;
      return {
        ...current,
        activeTaskStatus: task.status,
        lastProgressAt: task.lastUpdateAt,
        taskCompleteEventFired: task.status === 'complete',
        stalledReason: task.status === 'failed' ? task.error ?? current.stalledReason : current.stalledReason,
      };
    });
  }, [latestEfiTask, latestKextTask, latestRecoveryTask]);

  const filteredIssues = useMemo(() =>
    troubleshootingData.filter(it => {
      const matchCat = cat === 'All' || it.category === cat;
      const q = search.toLowerCase();
      const matchQ = !q || it.error.toLowerCase().includes(q) || it.fix.toLowerCase().includes(q) || it.category.toLowerCase().includes(q);
      return matchCat && matchQ;
    }), [search, cat]);

  useEffect(() => {
    const interval = window.setInterval(() => {
      const snapshot = buildFlowRef.current;
      const decision = evaluateBuildFlowStall(snapshot, Date.now());
      if (!snapshot?.active) {
        setBuildFlowAlert(null);
        return;
      }

      if (decision.level === 'healthy') {
        setBuildFlowAlert((current) => current?.level === 'taking_longer' ? null : current);
        return;
      }

      if (decision.level === 'taking_longer') {
        setBuildFlowAlert({
          level: 'taking_longer',
          reason: decision.reason ?? 'This step is taking longer than expected.',
          pendingCondition: decision.pendingCondition,
        });
        return;
      }

      if (decision.level === 'stalled' && buildFlowRef.current?.phase !== 'stalled') {
        triggerBuildFlowRecovery(
          decision.reason ?? 'The EFI build did not reach a terminal state.',
          decision.pendingCondition,
        );
      }
    }, 5_000);

    return () => window.clearInterval(interval);
  }, []);

  // Persist state
  useEffect(() => { (async () => { try {
    // Detect platform for BIOS step
    if (navigator.userAgent.includes('Windows')) setPlatform('win32');
    else if (navigator.userAgent.includes('Mac')) setPlatform('darwin');
    else setPlatform('linux');

    // Run early prechecks for admin rights
    if (window.electron && typeof window.electron.runPrechecks === 'function') {
      window.electron.runPrechecks().then(res => {
        setAdminPrivileges(res.adminPrivileges);
      }).catch(() => setAdminPrivileges(false));
    }

    if (window.electron?.getBiosRestartCapability) {
      window.electron.getBiosRestartCapability()
        .then(setRestartCapability)
        .catch(() => setRestartCapability(null));
    }

    if (window.electron?.getBiosResumeState) {
      window.electron.getBiosResumeState()
        .then(setBiosResumeState)
        .catch(() => setBiosResumeState(null));
    }

    // Guard: if the preload failed to wire window.electron, show a clear error
    // instead of crashing with "Cannot read properties of undefined".
    if (!window.electron || typeof window.electron.getPersistedState !== 'function') {
      setErrorWithSuggestion('Application failed to initialise — the preload script did not load. Restart the app. If the problem persists, check app.log in the data directory.');
      return;
    }

    await window.electron.notifyRendererReady().catch(() => false);

    if (window.location.search.includes('safe-recovery=1')) {
      await window.electron.clearState().catch(() => {});
      window.history.replaceState({}, document.title, window.location.pathname + window.location.hash);
      setProfile(null);
      setCompat(null);
      setBiosConf(null);
      setPlanningProfileContext(null);
      setProfileArtifact(null);
      invalidateGeneratedBuild();
      _setStepRaw('welcome');
      return;
    }

      const latestArtifact = await window.electron.getLatestHardwareProfile().catch(() => null);
      const s = await window.electron.getPersistedState();
      let restoredCompatibilityBlocked = false;
      let restoredBuildReady = false;
      if (s && s.profile && s.currentStep && Date.now() - s.timestamp < 4 * 3600 * 1000) {
        invalidateGeneratedBuild();
        const restore = restoreFlowDecision(s.profile, s.currentStep);
        const restoredPlanningContext = s.planningProfileContext ?? 'saved_artifact';
        restoredCompatibilityBlocked = isCompatibilityBlocked(restore.compatibility);

        setPlanningProfileContext(restoredPlanningContext);
        setProfileArtifact(latestArtifact && latestArtifact.digest === s.profileArtifactDigest ? latestArtifact : null);
      setProfile(restore.profile);
      setCompat(restore.compatibility);
      setBiosConf(restore.biosConfig);
      setBiosState(null);
      // Use _setStepRaw for restoration: state variables haven't flushed to the
      // render closure yet, so the centralized step guard would see stale nulls.
      const restoredStep = STEP_ORDER.indexOf(restore.restoredStep as StepId) > STEP_ORDER.indexOf('report')
        ? 'report'
        : (restore.restoredStep as StepId);
      _setStepRaw(restoredStep);
      if (restore.canReuseExistingEfi && s.efiPath) {
        try {
          const restoredValidation = await window.electron.validateEfi(s.efiPath, restore.profile);
          setValidationResult(restoredValidation);
          if (!isValidationBlockingDeployment(restoredValidation)) {
            setEfiPath(s.efiPath);
            setBuildReady(true);
            restoredBuildReady = true;
          }
        } catch {
            restoredBuildReady = false;
          }
        }
      } else if (canRestoreLatestScannedArtifact(latestArtifact)) {
        invalidateGeneratedBuild();
        const restore = restoreFlowDecision(latestArtifact.profile, 'report');
        restoredCompatibilityBlocked = isCompatibilityBlocked(restore.compatibility);
        setPlanningProfileContext('saved_artifact');
        setProfileArtifact(latestArtifact);
        setProfile(restore.profile);
        setCompat(restore.compatibility);
        setBiosConf(restore.biosConfig);
        setBiosState(null);
        _setStepRaw('report');
      }

    // Auto-resume an interrupted recovery download
    try {
      const resumeState = await window.electron.getDownloadResumeState();
      const resumeDecision = recoveryResumeDecision({
        compatibilityBlocked: restoredCompatibilityBlocked,
        efiReady: restoredBuildReady,
      });
      if (resumeState && resumeState.offset > 0 && resumeDecision.canResume) {
        setEfiPath(resumeState.efiPath);
        setRecovOffset(resumeState.offset);
        setRecovDmgDest(resumeState.dmgDest);
        setRecovClDest(resumeState.clDest);
        setRecovPct(8); // show that we're not at 0
        setRecovStatus(`Resuming from ${(resumeState.offset / 1024 / 1024).toFixed(0)} MB…`);
        _setStepRaw('recovery-download'); // bypass guard — efiPath hasn't flushed to closure yet
        // Kick off the resumed download — progress is driven by the recovTask useEffect above
        lastRecovSaveRef.current = 0;
        window.electron.downloadRecovery(resumeState.efiPath, resumeState.targetOS, resumeState.offset)
          .catch((e: any) => {
            const msg = e.message || 'Resume failed';
            setRecovError(msg);
            setErrorWithSuggestion(msg, 'recovery-download');
          });
      } else if (resumeState && resumeState.offset > 0) {
        invalidateGeneratedBuild();
        setErrorWithSuggestion(
          resumeDecision.message ?? 'Skipped recovery resume because the saved session is no longer valid.',
          resumeDecision.redirect ?? 'report',
        );
      }
    } catch(e) { /* no resume state */ }

    await window.electron.disableAutostart();
  } catch (e) { debugError('[init] Mount setup failed:', e); } })(); }, []);
  useEffect(() => {
    if (step !== 'landing' && profile) {
      try {
        window.electron.saveState({
          currentStep: step,
          profile,
          timestamp: Date.now(),
          efiPath: efiPath ?? undefined,
          planningProfileContext,
          profileArtifactDigest: profileArtifact?.digest,
        });
      } catch (e) {}
    }
  }, [step, profile, efiPath, planningProfileContext, profileArtifact]);

  // ── Workflows ──────────────────────────────────────────────

  const startScan = async (requestedSuccessStep?: ScanSuccessStep | unknown) => {
    if (isScanningRef.current) return;
    const successStep = resolveScanSuccessStep(requestedSuccessStep);
    const requestId = scanRequestIdRef.current + 1;
    scanRequestIdRef.current = requestId;
    isScanningRef.current = true;
    const previousProfile = latestProfileRef.current;
    const previousCompat = compat;
    const previousBiosConf = biosConf;
    const previousArtifact = profileArtifact;
    const previousPlanningContext = latestPlanningContextRef.current;
    const previousInterpretation = hwInterpretation;
    try {
      setStep('scanning'); setProgress(20);
      const scanResult = await window.electron.scanHardware();
      if (scanRequestIdRef.current !== requestId) return;
      const reconciled = reconcileHardwareScanProfile(previousProfile, scanResult.profile);
      const hw = {
        ...reconciled.profile,
        targetOS: reconciled.likelySameMachine && previousProfile?.targetOS
          ? previousProfile.targetOS
          : reconciled.profile.targetOS,
      };
      const shouldInvalidateExistingBuild = !previousProfile
        || reconciled.shouldInvalidateBuild
        || previousPlanningContext !== 'live_scan';
      if (shouldInvalidateExistingBuild) {
        invalidateGeneratedBuild();
      }
      setProfileArtifact(scanResult.artifact);
      setPlanningProfileContext('live_scan');
      setProfile(hw);
      setHwInterpretation(scanResult.interpretation ?? null);
      setProgress(60);
      const [bs, fw] = await Promise.all([
        window.electron.probeBios(),
        window.electron.probeFirmware(),
      ]);
      setBiosStatus(bs);
      if (fw.ok && fw.data) setFirmwareInfo(fw.data);
      setProgress(85);
      
      const report = checkCompatibility(hw);
      // Inject strategy into profile for config generation
      hw.strategy = report.strategy;
      hw.smbios = getSMBIOSForProfile(hw);
      
      setCompat(report);
      setProfile(hw); // Update with strategy
      setBiosConf(getBIOSSettings(hw));
      await refreshBiosState(hw, { assumeLiveHardwareContext: true, surfaceError: false });
      if (scanRequestIdRef.current !== requestId) return;
      setProgress(100);
      setTimeout(() => {
        if (scanRequestIdRef.current !== requestId) return;
        _setStepRaw(successStep);
      }, 700);
    } catch (e: any) {
      if (scanRequestIdRef.current !== requestId) return;
      if (previousProfile) {
        setProfile(previousProfile);
        setCompat(previousCompat);
        setBiosConf(previousBiosConf);
        setProfileArtifact(previousArtifact);
        setPlanningProfileContext(previousPlanningContext);
        setHwInterpretation(previousInterpretation);
        setErrorWithSuggestion(e.message || 'Hardware scan failed', 'report');
        _setStepRaw('report');
      } else {
        setErrorWithSuggestion(e.message || 'Hardware scan failed', 'precheck');
        _setStepRaw('precheck');
      }
    } finally {
      if (scanRequestIdRef.current === requestId) {
        isScanningRef.current = false;
      }
    }
  };

  const applyPlanningProfileArtifact = (
    artifact: HardwareProfileArtifact,
    context: Exclude<PlanningProfileContext, null>,
  ) => {
    invalidateGeneratedBuild();
    clearFlashConfirmationState();
    setProfileArtifact(artifact);
    setPlanningProfileContext(context);
    setHwInterpretation(null);
    setBiosState(null);
    setBiosResumeState(null);
    setFirmwareInfo(null);
    setRecovError(null);
    setRecovPct(0);
    setRecovStatus('');
    setRecovOffset(0);
    setRecovDmgDest(null);
    setRecovClDest(null);
    const nextProfile = { ...artifact.profile };
    const nextCompat = checkCompatibility(nextProfile);
    nextProfile.strategy = nextCompat.strategy;
    setCompat(nextCompat);
    setProfile(nextProfile);
    setBiosConf(getBIOSSettings(nextProfile));
    setStep('report');
  };

  const saveCurrentPlanningProfile = async () => {
    if (!profile) return;
    const artifact = await window.electron.saveHardwareProfile({
      profile,
      interpretation: buildInterpretationMetadata(hwInterpretation),
      source: hasLiveHardwareContext ? 'live_scan' : 'manual_planning',
    });
    setProfileArtifact(artifact);
  };

  const exportCurrentPlanningProfile = async () => {
    if (profileArtifact) {
      await window.electron.exportHardwareProfile(profileArtifact);
      return;
    }
    if (!profile) return;
    const artifact = await window.electron.saveHardwareProfile({
      profile,
      interpretation: buildInterpretationMetadata(hwInterpretation),
      source: hasLiveHardwareContext ? 'live_scan' : 'manual_planning',
    });
    setProfileArtifact(artifact);
    await window.electron.exportHardwareProfile(artifact);
  };

  const importPlanningProfile = async () => {
    const artifact = await window.electron.importHardwareProfile();
    if (!artifact) return;
    applyPlanningProfileArtifact(artifact, 'imported_artifact');
  };

  const runSafeSimulationPreview = async () => {
    if (!profile || simulationRunning) return;
    setSimulationRunning(true);
    try {
      const result = await window.electron.runSafeSimulation(profile);
      setSafeSimulationResult(result);
    } catch (e: any) {
      setErrorWithSuggestion(e.message || 'Safe simulation failed.', 'report');
    } finally {
      setSimulationRunning(false);
    }
  };

  const applySupportedBiosChanges = async (selectedChanges: Record<string, BiosSettingSelection>) => {
    if (!profile) throw new Error('Hardware profile missing for BIOS orchestration.');
    setBiosAcceptedRuntime(false);
    const result = await window.electron.applySupportedBiosChanges(profile, selectedChanges);
    setBiosState(result.state);
    return { message: result.message };
  };

  const recheckBiosState = async (selectedChanges: Record<string, BiosSettingSelection>) => {
    buildAutoStartRef.current = false;
    setBiosAcceptedRuntime(false);
    return performBiosRecheck({
    profile,
    currentState: biosState,
    applyVerifiedState: setBiosState,
    recheckManualChanges: (activeProfile, changes) => window.electron.verifyManualBiosChanges(activeProfile, changes),
    continueWithCurrentState: (activeProfile, changes) => window.electron.continueBiosWithCurrentState(activeProfile, changes),
    advanceToBuildStep: () => false,
    openRecoverySurface: (payload) => {
      setGlobalNotice(null);
      setGlobalError(JSON.stringify(payload));
      logUiEvent('error_surface_opened', {
        step: 'bios',
        code: payload.code ?? 'bios_recheck_failed',
        message: payload.message,
        rawMessage: payload.rawMessage ?? payload.explanation ?? payload.message,
      });
    },
    }, selectedChanges);
  };

  const continueFromCurrentBiosState = async (selectedChanges: Record<string, BiosSettingSelection>) => performBiosContinue({
    profile,
    currentState: biosState,
    applyVerifiedState: setBiosState,
    recheckManualChanges: (activeProfile, changes) => window.electron.verifyManualBiosChanges(activeProfile, changes),
    continueWithCurrentState: (activeProfile, changes) => window.electron.continueBiosWithCurrentState(activeProfile, changes),
    advanceToBuildStep: () => {
      buildAutoStartRef.current = true;
      setBiosAcceptedRuntime(true);
      const buildEntryUiState = createBuildEntryUiState();
      setProgress(buildEntryUiState.progress);
      setStatus(buildEntryUiState.statusText);
      setBuildFlow(null);
      setBuildFlowAlert(null);
      const nextBuildGuard = evaluateBuildGuard({
        compatibilityBlocked,
        biosFlowState: 'complete',
        biosAccepted: true,
        releaseFlowState,
      });
      const transition = attemptStepTransition('building', {
        biosAccepted: true,
        localBuildGuard: nextBuildGuard,
      });
      if (!transition?.ok) {
        buildAutoStartRef.current = false;
        return false;
      }
      return true;
    },
    openRecoverySurface: (payload) => {
      setGlobalNotice(null);
      setGlobalError(JSON.stringify(payload));
      logUiEvent('error_surface_opened', {
        step: 'bios',
        code: payload.code ?? 'bios_continue_blocked',
        message: payload.message,
        rawMessage: payload.rawMessage ?? payload.explanation ?? payload.message,
      });
    },
  }, selectedChanges);

  const restartToFirmwareWithSession = async (selectedChanges: Record<string, BiosSettingSelection>) => {
    if (!profile) return { supported: false, error: 'Hardware profile missing for BIOS reboot.' };
    buildAutoStartRef.current = false;
    setBiosAcceptedRuntime(false);
    const result = await window.electron.restartToFirmwareWithSession(profile, selectedChanges);
    setBiosState(result.state);
    window.electron.getBiosResumeState().then(setBiosResumeState).catch(() => {});
    if (!result.supported && result.error) {
      openBiosRecoverySurface('bios_restart_failed', { detail: result.error });
    }
    return { supported: result.supported, error: result.error };
  };

  const startDeploy = async () => {
    if (!canStartBuildRun({
      hasProfile: Boolean(profile),
      isDeploying: isDeployingRef.current,
      startRequested: buildStartRequestedRef.current,
    })) return;
    buildStartRequestedRef.current = true;
    if (!profile) {
      buildStartRequestedRef.current = false;
      return;
    }
    const buildEntryUiState = createBuildEntryUiState();
    setProgress(buildEntryUiState.progress);
    setStatus(buildEntryUiState.statusText);
    const allowAcceptedSession = biosAcceptedRef.current;
    const liveBiosState = (biosReady || allowAcceptedSession) ? biosState : await refreshBiosState(profile, { redirectIfBlocked: true });
    if (!liveBiosState) {
      buildStartRequestedRef.current = false;
      return;
    }
    const guard = await ensureBuildGuard(profile, { surfaceError: true });
    if (!guard.allowed) {
      buildStartRequestedRef.current = false;
      return;
    }
    isDeployingRef.current = true;
    const runId = buildRunIdRef.current + 1;
    buildRunIdRef.current = runId;
    buildFlowRef.current = null;
    setBuildFlow(null);
    setBuildFlowAlert(null);
    setEfiPath(null);
    setBuildReady(false);
    setValidationResult(null);
    setKextResults([]);
    setRecovPct(0);
    setRecovStatus('');
    setRecovError(null);
    setRecovOffset(0);
    setRecovDmgDest(null);
    setRecovClDest(null);
    setCachedRecovInfo(null);
    clearSelectedUsbState();
    setFlashMilestones([]);
    setRecoveryDryRun(null);
    setShowFlashConfirm(false);
    setShowPartitionConfirm(false);
    setFlashConfirmationToken(null);
    setFlashConfirmationExpiresAt(null);
    setFlashConfirmText('');
    setFlashChecks(new Set());
    setEfiReport(null);
    setCommunityIssues([]);
    setShowEfiReport(false);
    setResourcePlan(null);
    setSafeSimulationResult(null);
    setEfiBackupPolicy(null);
    setBuildPlan(null);
    const isCurrentRun = () => buildRunIdRef.current === runId;
    const applyBuildFlowSnapshot = (patch: Partial<BuildFlowSnapshot>) => {
      updateBuildFlow((current) => {
        const base: BuildFlowSnapshot = current && current.runId === runId
          ? current
          : {
              active: true,
              runId,
              phase: 'preflight',
              uiStep: 'building',
              startedAt: Date.now(),
              lastProgressAt: Date.now(),
              activeTaskKind: null,
              activeTaskStatus: null,
              lastTaskPhase: null,
              taskCompleteEventFired: false,
              validationStarted: false,
              validationFinished: false,
              pendingRendererExpectation: null,
              transitionGuardBlocked: null,
              stalledReason: null,
            };
        return {
          ...base,
          ...patch,
        };
      });
    };
    try {
      setStep('building');
      applyBuildFlowSnapshot({
        phase: 'preflight',
        uiStep: 'building',
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'the pre-build environment checks to finish',
        transitionGuardBlocked: null,
        stalledReason: null,
      });

      // Stage 0a: Prevention Layer — preflight environment check
      setStatus('Checking your build environment…');
      setProgress(1);
      try {
        const { kexts: requiredKexts } = getRequiredResources(profile);
        setPreflightRunning(true);
        const report = await (window.electron as any).runPreflightChecks(requiredKexts);
        if (!isCurrentRun()) return;
        setPreflightReport(report);
        setConfidence(report.confidence);
        setPreflightRunning(false);

        if (report.blockers.length > 0) {
          throw new Error(`Pre-build check failed: ${report.blockers[0]}`);
        }
        if (report.warnings.length > 0) {
          debugWarn('[preflight] Warnings:', report.warnings);
        }
        for (const k of report.kextAvailability.filter(k => !k.available)) {
          (window.electron as any).recordFailure(`kext_${k.name}`, k.error || 'unavailable');
        }
      } catch (preflightErr: any) {
        setPreflightRunning(false);
        if (preflightErr.message?.startsWith('Pre-build check failed:')) throw preflightErr;
        debugWarn('[preflight] Preflight check itself failed, proceeding:', preflightErr);
      }
      if (!isCurrentRun()) return;
      setProgress(3);

      // Stage 0b: Deterministic Layer — build dry-run simulation
      // Verifies every kext URL, OpenCore URL, and disk BEFORE real build
      applyBuildFlowSnapshot({
        phase: 'simulation',
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'the dependency simulation to finish',
      });
      setStatus('Verifying downloads and dependencies…');
      try {
        const { kexts: requiredKexts, ssdts: requiredSSDTs } = getRequiredResources(profile);
        const plan = await (window.electron as any).simulateBuild(requiredKexts, requiredSSDTs, profile.smbios);
        if (!isCurrentRun()) return;
        setBuildPlan(plan);
        setCertainty(plan.certainty);

        // Phase 5: Failure impossibility zone — block if build WILL fail
        if (plan.certainty === 'will_fail') {
          throw new Error(`Build will fail: ${plan.blockers[0] || 'One or more components are unreachable'}`);
        }
      } catch (simErr: any) {
        if (simErr.message?.startsWith('Build will fail:')) throw simErr;
        debugWarn('[deterministic] Build simulation failed, proceeding:', simErr);
      }
      if (!isCurrentRun()) return;
      setProgress(5);

      // Stage 1: build EFI
      applyBuildFlowSnapshot({
        phase: 'efi-build',
        activeTaskKind: 'efi-build',
        activeTaskStatus: 'running',
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'the EFI build task to complete',
        taskCompleteEventFired: false,
        lastTaskPhase: 'initialising',
      });
      setStatus('Generating OpenCore configuration…');
      setProgress(10);
      await new Promise(r => setTimeout(r, 800));
      const built = await window.electron.buildEFI(profile, allowAcceptedSession);
      if (!isCurrentRun()) return;
      setEfiPath(built);
      setProgress(55);
      setStatus('Validating EFI…');
      await new Promise(r => setTimeout(r, 600));
      if (!isCurrentRun()) return;
      setProgress(65);

      // Stage 2: kexts — progress is driven by kextTask useEffect above
      applyBuildFlowSnapshot({
        phase: 'kext-fetch',
        uiStep: 'kext-fetch',
        activeTaskKind: 'kext-fetch',
        activeTaskStatus: 'running',
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'the kext download task to complete',
        taskCompleteEventFired: false,
        lastTaskPhase: null,
      });
      _setStepRaw('kext-fetch');
      setProgress(0);
      setKextResults([]);
      const { kexts, ssdts: requiredSsdts } = getRequiredResources(profile);
      let fetchedKextResults: KextFetchResult[] = [];
      try {
        fetchedKextResults = await window.electron.fetchLatestKexts(built, kexts);
        if (!isCurrentRun()) return;
        setKextResults(fetchedKextResults);
        for (const k of fetchedKextResults.filter(k => k.source === 'failed')) {
          (window.electron as any).recordFailure(`kext_${k.name}`, 'Download failed').catch(() => {});
        }
      } catch (e) {
        if (!isCurrentRun()) return;
        fetchedKextResults = kexts.map(k => ({ name: k, version: 'unavailable', source: 'failed' }));
        setKextResults(fetchedKextResults);
        (window.electron as any).recordFailure('kext_batch', String((e as Error)?.message || 'Batch kext fetch failed')).catch(() => {});
      }
      if (!isCurrentRun()) return;
      setProgress(100);
      await new Promise(r => setTimeout(r, 400));
      if (!isCurrentRun()) return;

      // Stage 3: Build Integrity Check — existing configValidator + deterministic hard contract
      applyBuildFlowSnapshot({
        phase: 'validation',
        uiStep: 'building',
        activeTaskKind: null,
        activeTaskStatus: null,
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'EFI validation to finish',
        taskCompleteEventFired: false,
        validationStarted: true,
      });
      _setStepRaw('building');
      setStatus('Validating the generated EFI…');
      setProgress(78);
      const validation = await window.electron.validateEfi(built, profile);
      if (!isCurrentRun()) return;
      setValidationResult(validation);
      applyBuildFlowSnapshot({
        lastProgressAt: Date.now(),
        validationFinished: true,
      });
      if (validation.overall === 'blocked') {
        setBuildReady(false);
        applyBuildFlowSnapshot({
          active: false,
          phase: 'failed',
          stalledReason: describeValidationFailure(validation),
          pendingRendererExpectation: null,
        });
        setErrorWithSuggestion(describeValidationFailure(validation), 'building', {
          validationResult: validation,
          kextSources: buildKextSourceMap(fetchedKextResults),
        });
        setStep('report');
        return;
      }

      // Phase 4: Hard success contract — verify from disk, not flags
      try {
        const contract = await (window.electron as any).verifyEfiBuildSuccess(built, kexts, requiredSsdts);
        if (!isCurrentRun()) return;
        if (!contract.passed) {
          const failed = contract.checks.filter((c: any) => !c.passed);
          throw new Error(`EFI build contract failed: ${failed.map((c: any) => `${c.name}: ${c.detail}`).join('; ')}`);
        }
      } catch (contractErr: any) {
        if (contractErr.message?.startsWith('EFI build contract failed:')) throw contractErr;
        debugWarn('[deterministic] EFI contract check failed to execute:', contractErr);
      }

      setBuildReady(true); // BUILD IS NOW VERIFIED READY (by disk, not trust)
      applyBuildFlowSnapshot({
        phase: 'finalizing',
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'the recovery preparation step to open',
      });
      setProgress(92);
      setStatus('Finalizing the EFI build…');

      // Generate EFI Intelligence Report + Community Knowledge
      try {
        const report = generateEfiReport(profile, compat, fetchedKextResults, validation);
        setEfiReport(report);
        const issues = getRelevantIssues({
          architecture: profile.architecture,
          generation: profile.generation,
          gpu: profile.gpu,
          isLaptop: profile.isLaptop,
          kexts: profile.kexts,
        });
        setCommunityIssues(issues);
      } catch (e) {
        debugWarn('[efi-report] Failed to generate report:', e);
      }

      // Stage 4: recovery — progress is driven by recovTask useEffect above
      const recoveryTransition = attemptStepTransition('recovery-download', {
        buildReady: true,
        efiPath: built,
        validationBlocked: false,
        postBuildReady: true,
      });
      if (!recoveryTransition?.ok) {
        applyBuildFlowSnapshot({
          active: false,
          phase: 'failed',
          transitionGuardBlocked: recoveryTransition?.reason ?? 'the next build phase could not open',
          stalledReason: recoveryTransition?.reason ?? 'The next build phase could not open.',
          pendingRendererExpectation: 'the recovery preparation step to open',
        });
        triggerBuildFlowRecovery(
          recoveryTransition?.reason ?? 'The renderer could not leave the validated build phase.',
          'the recovery preparation step to open',
          recoveryTransition?.redirect ?? 'report',
        );
        return;
      }
      applyBuildFlowSnapshot({
        phase: 'recovery-dry-run',
        uiStep: 'recovery-download',
        activeTaskKind: null,
        activeTaskStatus: null,
        lastProgressAt: Date.now(),
        pendingRendererExpectation: 'the recovery source checks to finish',
        taskCompleteEventFired: false,
        transitionGuardBlocked: null,
      });
      setRecovPct(0); setRecovError(null); setRecovOffset(0); setRecovDmgDest(null); setRecovClDest(null);
      lastRecovSaveRef.current = 0;

      // Prevention: Check if Apple endpoint is known-bad before downloading
      if (preflightReport && !preflightReport.recoveryEndpoint.reachable) {
        setRecovError('Apple recovery server was unreachable during pre-build check. Recovery download will not be attempted.');
        setErrorWithSuggestion('Apple recovery server was unreachable during pre-build check. Use manual import or try a different macOS version.', 'recovery-download');
        return;
      }

      // Deterministic: Recovery dry-run — send real test request to Apple before download
      try {
        setStatus('Testing Apple recovery endpoint…');
        const dryRun = await (window.electron as any).dryRunRecovery(profile.targetOS || 'macOS Sequoia 15', profile.smbios);
        if (!isCurrentRun()) return;
        setRecoveryDryRun(dryRun);

        // Phase 5: Failure impossibility zone — block recovery if test request was rejected
        if (dryRun.certainty === 'will_fail') {
          setRecovError(`Recovery will fail: ${dryRun.recommendation}`);
          setErrorWithSuggestion(dryRun.recommendation, 'recovery-download');
          (window.electron as any).recordFailure('recovery_auth', dryRun.recommendation).catch(() => {});
          return; // Let user see error + use manual import
        }
      } catch (dryRunErr: any) {
        debugWarn('[deterministic] Recovery dry-run failed, proceeding:', dryRunErr);
      }

      try {
        applyBuildFlowSnapshot({
          phase: 'recovery-download',
          activeTaskKind: 'recovery-download',
          activeTaskStatus: 'running',
          lastProgressAt: Date.now(),
          pendingRendererExpectation: 'the recovery download task to complete',
          taskCompleteEventFired: false,
        });
        await window.electron.downloadRecovery(built, profile.targetOS || 'macOS Sequoia 15');
        if (!isCurrentRun()) return;
      } catch (e: any) {
        const msg = e.message || 'Unknown error';
        const failCode = msg.includes('401') || msg.includes('403') ? 'recovery_auth' : 'recovery_dl';
        (window.electron as any).recordFailure(failCode, msg).catch(() => {});
        setRecovError(msg);
        setErrorWithSuggestion(msg, 'recovery-download');
        return; /* wait for user action (Retry or Skip) */
      }

      // Phase 4: Hard success contract — verify recovery from disk
      try {
        applyBuildFlowSnapshot({
          phase: 'finalizing',
          activeTaskKind: null,
          activeTaskStatus: null,
          lastProgressAt: Date.now(),
          pendingRendererExpectation: 'the installer method screen to open',
          taskCompleteEventFired: false,
        });
        const recovContract = await (window.electron as any).verifyRecoverySuccess(built);
        if (!isCurrentRun()) return;
        if (!recovContract.passed) {
          const failed = recovContract.checks.filter((c: any) => !c.passed);
          setRecovError(`Recovery verification failed: ${failed.map((c: any) => `${c.name}: ${c.detail}`).join('; ')}`);
          setErrorWithSuggestion('Recovery download completed but verification failed. File may be incomplete.', 'recovery-download');
          return;
        }
      } catch (recovContractErr: any) {
        debugWarn('[deterministic] Recovery contract check failed to execute:', recovContractErr);
      }

      await new Promise(r => setTimeout(r, 1000));
      if (!isCurrentRun()) return;

      // Stage 5: Method select
      const methodTransition = attemptStepTransition('method-select', {
        buildReady: true,
        efiPath: built,
        validationBlocked: false,
        postBuildReady: true,
      });
      if (!methodTransition?.ok) {
        applyBuildFlowSnapshot({
          active: false,
          phase: 'failed',
          transitionGuardBlocked: methodTransition?.reason ?? 'the build did not advance to method selection',
          stalledReason: methodTransition?.reason ?? 'The build did not advance to method selection.',
          pendingRendererExpectation: 'the installer method screen to open',
        });
        triggerBuildFlowRecovery(
          methodTransition?.reason ?? 'The build completed, but the app could not advance to the next screen.',
          'the installer method screen to open',
          methodTransition?.redirect ?? 'report',
        );
        return;
      }
      applyBuildFlowSnapshot({
        active: false,
        phase: 'complete',
        uiStep: 'method-select',
        activeTaskKind: null,
        activeTaskStatus: null,
        lastProgressAt: Date.now(),
        pendingRendererExpectation: null,
        taskCompleteEventFired: false,
        stalledReason: null,
        transitionGuardBlocked: null,
      });
      setBuildFlowAlert(null);
    } catch (e: any) {
      if (!isCurrentRun()) {
        return;
      }
      setBuildReady(false);
      applyBuildFlowSnapshot({
        active: false,
        phase: 'failed',
        pendingRendererExpectation: null,
        stalledReason: e.message || 'Build failed',
      });
      const message = e.message || 'Build failed. Please check the hardware compatibility.';
      setGlobalNotice(null);
      setGlobalError(JSON.stringify({
        code: 'build_ipc_failed',
        message: 'EFI build failed',
        explanation: message,
        decisionSummary: message,
        suggestion: 'Return to the previous step, confirm the BIOS/build prerequisites, then retry the EFI build once.',
        category: 'build_error',
        severity: 'warning',
        targetStep: 'report',
        rawMessage: message,
      }));
      logUiEvent('error_surface_opened', {
        step: 'building',
        code: 'build_ipc_failed',
        message: 'EFI build failed',
        rawMessage: message,
      });
      setStep('report');
    } finally {
      if (isCurrentRun()) {
        isDeployingRef.current = false;
        buildStartRequestedRef.current = false;
      }
    }
  };

  useEffect(() => {
    if (
      step !== 'building'
      || !buildAutoStartRef.current
      || isDeployingRef.current
      || buildStartRequestedRef.current
    ) {
      return;
    }
    buildAutoStartRef.current = false;
    void startDeploy();
  }, [step]); // eslint-disable-line react-hooks/exhaustive-deps

  /** Enrich a raw drive list with disk info (isSystemDisk, partitionTable, etc.)
   *  Each drive info fetch is best-effort — failures leave the fields undefined. */
  const enrichDrives = async (
    rawDevs: { name: string; device: string; size: string; type?: string }[]
  ): Promise<import('./components/steps/UsbStep').DriveInfo[]> => {
    return Promise.all(
      rawDevs.map(async (d) => {
        try {
          const info = await window.electron.getDiskInfo(d.device);
          return {
            name: d.name,
            device: d.device,
            size: d.size,
            isSystemDisk: info.isSystemDisk,
            partitionTable: info.partitionTable as 'gpt' | 'mbr' | 'unknown',
            mountedPartitions: (info as any).mountedPartitions ?? [],
            removable: !info.isSystemDisk,
          };
        } catch {
          return { name: d.name, device: d.device, size: d.size };
        }
      })
    );
  };

  const toPendingDrives = (
    rawDevs: { name: string; device: string; size: string; type?: string }[]
  ): import('./components/steps/UsbStep').DriveInfo[] => rawDevs.map((d) => ({
    name: d.name,
    device: d.device,
    size: d.size,
  }));

  const loadUsbTargets = async (
    rawLoader: () => Promise<{ name: string; device: string; size: string; type?: string }[]>,
  ) => {
    const requestId = Date.now();
    usbRefreshRequestRef.current = requestId;
    setUsbRefreshBusy(true);
    const devices = await rawLoader();
    setUsbDevices(toPendingDrives(devices));
    try {
      const enriched = await enrichDrives(devices);
      if (usbRefreshRequestRef.current === requestId) {
        setUsbDevices(enriched);
      }
    } finally {
      if (usbRefreshRequestRef.current === requestId) {
        setUsbRefreshBusy(false);
      }
    }
  };

  const refreshUsbTargets = async () => {
    try {
      await loadUsbTargets(() => window.electron.listUsbDevices());
      clearSelectedUsbState();
    } catch (error: any) {
      setErrorWithSuggestion(error?.message || 'Failed to refresh removable drives. Reconnect the target USB and try again.', 'usb-select');
    }
  };

  const refreshPartitionTargets = async () => {
    try {
      setUsbRefreshBusy(true);
      const drives = await window.electron.getHardDrives();
      setUsbDevices(await enrichDrives(drives));
      clearSelectedUsbState();
    } catch (error: any) {
      setErrorWithSuggestion(error?.message || 'Failed to refresh disks. Retry or rescan before modifying a drive.', 'part-prep');
    } finally {
      setUsbRefreshBusy(false);
    }
  };

  const selectMethod = async (m: 'usb' | 'partition') => {
    if (!profile) return;
    if (!buildReady || !efiPath) {
      setErrorWithSuggestion('EFI validation failed — build integrity check failed or incomplete. Please go back and build the EFI first.');
      setStep('report');
      return;
    }

    try {
      const freshValidation = await window.electron.validateEfi(efiPath, profile);
      setValidationResult(freshValidation);
      if (isValidationBlockingDeployment(freshValidation)) {
        setBuildReady(false);
        setErrorWithSuggestion(`${describeValidationFailure(freshValidation)} Please rebuild.`, 'report', {
          validationResult: freshValidation,
        });
        setStep('report');
        return;
      }
    } catch {
      setBuildReady(false);
      setErrorWithSuggestion('EFI validation failed — failed to verify build integrity. Please rebuild.');
      setStep('report');
      return;
    }

    try {
      setMethod(m);
      // Always reset selection when loading new drive list
      clearSelectedUsbState();
      setUsbDevices([]);
      if (m === 'usb') {
        const transition = attemptStepTransition('usb-select');
        if (!transition?.ok) return;
        await loadUsbTargets(() => window.electron.listUsbDevices());
      } else {
        const transition = attemptStepTransition('part-prep');
        if (!transition?.ok) return;
        setUsbRefreshBusy(true);
        const drives = await window.electron.getHardDrives();
        const enriched = await enrichDrives(drives);
        setUsbDevices(enriched);
        setUsbRefreshBusy(false);
      }
    } catch (e: any) {
      setUsbRefreshBusy(false);
      setErrorWithSuggestion(e.message || 'Failed to list drives');
    }
  };

  const preparePartition = async (confirmed = false) => {
    if (!selectedUsb || !efiPath) return; // Re-use selectedUsb for disk name
    if (!profile) return;
    const { guard, validation } = await ensureDeployGuard(profile, efiPath, {
      surfaceError: true,
      reasonSuffix: ' Do not modify a disk until the BIOS and EFI are still valid.',
    });
    if (!guard.allowed) {
      return;
    }
    if (validation) {
      setValidationResult(validation);
      if (isValidationBlockingDeployment(validation)) {
        setBuildReady(false);
        return;
      }
    }
    try {
      const freshValidation = await window.electron.validateEfi(efiPath, profile ?? null);
      setValidationResult(freshValidation);
      if (isValidationBlockingDeployment(freshValidation)) {
        setBuildReady(false);
        setErrorWithSuggestion(`${describeValidationFailure(freshValidation)} Please rebuild before modifying a disk.`, 'report', {
          validationResult: freshValidation,
        });
        setStep('report');
        return;
      }
    } catch {
      setBuildReady(false);
      setErrorWithSuggestion('EFI validation failed — failed to verify build integrity before modifying a disk.', 'report');
      setStep('report');
      return;
    }
    if (!confirmed) {
      setShowPartitionConfirm(true);
      return;
    }

    // Final check: is the disk still there?
    try {
      await window.electron.getDiskInfo(selectedUsb);
    } catch (e) {
      setErrorWithSuggestion(`Target disk (${selectedUsb}) not found — device disconnected. Operation cancelled.`);
      setShowPartitionConfirm(false);
      return;
    }

    setShowPartitionConfirm(false);
    try {
      setStep('building'); setProgress(0);
      setStatus('Shrinking primary partition...');
      await window.electron.shrinkPartition(selectedUsb, 16, true);
      setProgress(50);
      setStatus('Creating bootstrap partition...');
      await window.electron.createBootPartition(selectedUsb, efiPath, true, profile ?? null);
      setProgress(100);
      setStep('complete');
    } catch (e: any) {
      setProgress(0);
      setErrorWithSuggestion(e.message || 'Partition prep failed');
      setStep('part-prep');
    }
  };

  // Phase 1: safety checks + show confirmation modal
  const initiateFlash = async () => {
    if (!selectedUsb || !efiPath) return;
    if (!profile) return;
    if (flashConfirmBusy) return;
    clearFlashConfirmationState();
    setFlashConfirmBusy(true);
    const { guard, validation } = await ensureDeployGuard(profile, efiPath, {
      surfaceError: true,
      reasonSuffix: ' Flashing is blocked until the BIOS and EFI are still valid.',
    });
    if (!guard.allowed) {
      setFlashConfirmBusy(false);
      return;
    }
    if (validation) {
      setValidationResult(validation);
      if (isValidationBlockingDeployment(validation)) {
        setBuildReady(false);
        setFlashConfirmBusy(false);
        return;
      }
    }
    const selectedDiskInfo = await resolveDiskInfoForDevice(selectedUsb, { retries: 2, preferCaptured: true });
    if (selectedDiskInfo) {
      if (selectedDiskInfo.isSystemDisk) {
        setErrorWithSuggestion(`SYSTEM_DISK: ${selectedUsb} is your system/boot disk. Select a different USB drive.`);
        setFlashConfirmBusy(false);
        return;
      }
      if (selectedDiskInfo.partitionTable === 'mbr') {
        setShowDiskWarning(true);
        setFlashConfirmBusy(false);
        return;
      }
      if (selectedDiskInfo.partitionTable === 'unknown') {
        setShowUnknownPartitionWarning(true);
        setFlashConfirmBusy(false);
        return;
      }
    }
    // Run EFI validation before showing confirmation
    setValidationResult(null);
    setValidationRunning(true);
    try {
      const vResult = await window.electron.validateEfi(efiPath, profile ?? null);
      setValidationResult(vResult);
      if (isValidationBlockingDeployment(vResult)) {
        setBuildReady(false);
        setErrorWithSuggestion(`${describeValidationFailure(vResult)} Flashing has been blocked. Rebuild before writing to disk.`, 'report', {
          validationResult: vResult,
        });
        setStep('report');
        setFlashConfirmBusy(false);
        return;
      }
    } catch (e) {
      debugWarn('[flash] Pre-flash EFI validation failed:', e);
      setValidationResult(null);
      setBuildReady(false);
      setErrorWithSuggestion('EFI validation failed — failed to verify build integrity before flashing.', 'report');
      setStep('report');
      setFlashConfirmBusy(false);
      return;
    } finally {
      setValidationRunning(false);
    }
    // Issue #31: ensure we always have disk identity before calling prepare.
    // If both selectedDiskInfo and diskInfo are null, the main process will
    // receive no identity, causing either a throw or a weak fallback that
    // can trigger false collision detection.
    const resolvedIdentity = pickSelectedDiskInfo(selectedUsb, selectedDiskInfo, diskInfo);
    if (!resolvedIdentity) {
      setErrorWithSuggestion(
        'Could not read disk identity for the selected drive. Unplug the drive, reconnect it, re-select it, and try again.',
        'usb-select',
      );
      setFlashConfirmBusy(false);
      return;
    }
    try {
      const prepared = await window.electron.prepareFlashConfirmation(
        selectedUsb,
        efiPath,
        toExpectedDiskIdentity(resolvedIdentity),
      );
      setDiskInfoIfCurrent(selectedUsb, prepared.diskInfo);
      setEfiBackupPolicy(prepared.backupPolicy);
      setFlashConfirmationToken(prepared.token);
      setFlashConfirmationExpiresAt(prepared.expiresAt);
    } catch (e: any) {
      setErrorWithSuggestion(e.message || 'Flash confirmation could not be prepared. Re-select the drive and try again.', 'usb-select');
      setFlashConfirmBusy(false);
      return;
    }
    setFlashConfirmBusy(false);
    setShowFlashConfirm(true);
  };

  // Phase 2: execute after explicit typed confirmation
  const executeFlash = async () => {
    if (!selectedUsb || !efiPath || isFlashingRef.current) return;
    if (!profile) return;
    if (!flashConfirmationToken) {
      setErrorWithSuggestion('SAFETY BLOCK: Flash confirmation is stale or missing. Re-open the confirmation dialog before writing to disk.', 'usb-select');
      return;
    }
    isFlashingRef.current = true;
    try {
      const { guard, validation } = await ensureDeployGuard(profile, efiPath, {
        surfaceError: true,
        reasonSuffix: ' Flashing has been stopped because the release state changed.',
      });
      if (!guard.allowed) {
        return;
      }
      if (validation) {
        setValidationResult(validation);
        if (isValidationBlockingDeployment(validation)) {
          setBuildReady(false);
          return;
        }
      }

      // Final check: did the drive disappear or change ID since selection?
      try {
        const info = await resolveDiskInfoForDevice(selectedUsb, { retries: 2, preferCaptured: true });
        if (!info) throw new Error('Device lost');
        setDiskInfoIfCurrent(selectedUsb, info);
      } catch (e) {
        setErrorWithSuggestion(`The selected drive (${selectedUsb}) is not found — device disconnected. Please re-select the drive and try again.`);
        clearFlashConfirmationState();
        return;
      }

      setShowFlashConfirm(false);
      setFlashMilestones([]);
      setStep('flashing'); setProgress(0);
      setStatus('flash_start');
      try {
        await window.electron.flashUsb(selectedUsb, efiPath, true, flashConfirmationToken);
      } catch (e: any) {
        (window.electron as any).recordFailure('flash_write', e.message || 'Flash failed').catch(() => {});
        clearFlashConfirmationState();
        setErrorWithSuggestion(e.message || 'USB flash write failed. Check that the drive is not write-protected and try a different USB drive.');
        setStep('usb-select');
        return;
      }
      // Flash succeeded — backend verified files on disk before returning
      clearFlashConfirmationState();
      setProgress(100);
      setStatus('flash_complete');
      setFlashMilestones(['USB partitioned', 'Files written', 'Verified on disk']);
      setStep('complete');
    } finally { isFlashingRef.current = false; }
  };

  // Keep flashUsb name for any remaining references
  const flashUsb = initiateFlash;

  const cancelCurrentOp = async (taskId: string) => {
    try {
      await cancelTask(taskId);
    } catch (e: any) {
      setErrorWithSuggestion(e.message || 'Cancel failed');
    }
    // Clear any error — cancellation is intentional, not an error
    setGlobalError(null);
    setStep('report');
  };

  const recoveryPayload = useMemo(() => parseFailureRecoveryPayload(globalError), [globalError]);
  const recoveryView = useMemo(() => buildFailureRecoveryViewModel(globalError), [globalError]);

  const handleRecoveryRetry = async () => {
    const target = recoveryPayload?.targetStep ?? step;
    logUiEvent('recovery_retry_clicked', { targetStep: target });
    setGlobalError(null);
    if (target === 'building' || target === 'kext-fetch' || target === 'recovery-download') {
      buildRunIdRef.current += 1;
      buildFlowRef.current = null;
      setBuildFlow(null);
      setBuildFlowAlert(null);
      isDeployingRef.current = false;
    }
    switch (resolveRecoveryRetryAction({
      targetStep: target,
      hasProfile: Boolean(profile),
      hasMethod: Boolean(method),
      buildReady,
    }).kind) {
      case 'scan':
        await startScan();
        return;
      case 'refresh_bios':
        if (profile) await refreshBiosState(profile, { redirectIfBlocked: false });
        return;
      case 'restart_build':
        if (profile) await startDeploy();
        return;
      case 'refresh_usb':
        await refreshUsbTargets();
        return;
      case 'refresh_partition':
        await refreshPartitionTargets();
        return;
      case 'reselect_method':
        if (method) await selectMethod(method);
        return;
      default:
        return;
    }
  };

  const handleBackToSafety = () => {
    const target = recoveryPayload?.targetStep;
    logUiEvent('recovery_back_to_safety_clicked', { targetStep: target ?? step });
    setGlobalError(null);
    buildRunIdRef.current += 1;
    buildFlowRef.current = null;
    setBuildFlow(null);
    setBuildFlowAlert(null);
    isDeployingRef.current = false;
    const destination = resolveBackToSafetyStep({ hasProfile: Boolean(profile) });
    setStep(destination);
  };

  const handleOpenIssueReport = async () => {
    try {
      logUiEvent('issue_report_open_requested', { targetStep: recoveryPayload?.targetStep ?? step });
      const reportContext = recoveryPayload?.contextNote ?? (typeof globalError === 'string' ? globalError : null);
      const res = await window.electron.reportIssue(reportContext);
      try {
        await navigator.clipboard.writeText(res.body);
      } catch {
        // Clipboard failure should not block the issue flow.
      }
      if (res.success) {
        setGlobalNotice('Issue report opened in your browser. The sanitized report was copied to the clipboard.');
      } else {
        setGlobalNotice(`Browser launch failed. The sanitized report was copied; paste it manually at ${res.baseUrl}.`);
      }
    } catch {
      setErrorWithSuggestion('Could not generate the issue report. Copy the report and file manually on GitHub.', recoveryPayload?.targetStep ?? step);
    }
  };

  const handleSaveSupportLog = async () => {
    try {
      logUiEvent('support_log_save_requested', { targetStep: recoveryPayload?.targetStep ?? step });
      const supportContext = recoveryPayload?.contextNote ?? (typeof globalError === 'string' ? globalError : null);
      const result = await window.electron.saveSupportLog(supportContext);
      setGlobalNotice(`Sanitized log saved to your ${result.savedTo} as ${result.fileName}.`);
    } catch (error: any) {
      setErrorWithSuggestion(error?.message || 'Could not save the support log to the Desktop.', recoveryPayload?.targetStep ?? step);
    }
  };

  // ── Sidebar helper ──────────────────────────────────────────

  const SidebarItem = ({ id, label, icon: Icon }: { id: string; label: string; icon: any }) => {
    const s = getSidebarStatus(step, id, STEP_ORDER);
    return (
      <div className={`flex items-center gap-3 px-4 py-3 rounded-xl transition-all ${s === 'active' ? 'bg-white/10 text-white' : s === 'complete' ? 'text-white/60' : 'text-[#555]'}`}>
        <div className={`w-6 h-6 rounded-md flex items-center justify-center text-xs ${s === 'complete' ? 'bg-green-500/20 text-green-400' : s === 'active' ? (Icon === BrandIcon ? 'bg-white/10 text-white' : 'bg-blue-500/20 text-blue-400') : 'bg-white/5 text-[#444]'}`}>
          {s === 'complete' ? <Check className="w-3.5 h-3.5" /> : <Icon className="w-3.5 h-3.5" />}
        </div>
        <span className="text-sm font-medium">{label}</span>
      </div>
    );
  };

  // ── Build ProgressStep stage arrays ────────────────────────

  const buildStages = [
    { label: 'Environment preflight', sublabel: preflightReport ? (preflightReport.confidence === 'green' ? 'All dependencies confirmed' : `${preflightReport.warnings.length} warning(s) — proceeding`) : 'Checking network, disk space, kext sources…', done: progress >= 3, active: progress > 0 && progress < 3 },
    { label: 'Dry-run simulation', sublabel: buildPlan ? (buildPlan.certainty === 'will_succeed' ? `${buildPlan.verifiedComponents}/${buildPlan.totalComponents} components reachable` : `${buildPlan.failedComponents} component(s) unverified`) : 'Testing every download URL before committing…', done: progress >= 5, active: progress >= 3 && progress < 5 },
    { label: 'Generate EFI structure', sublabel: profile ? `${profile.generation} · ${profile.smbios} · ${profile.kexts.length} kexts` : 'Generating OpenCore, ACPI, and boot files', done: progress >= 65, active: progress >= 5 && progress < 65 },
    { label: 'Validate generated EFI', sublabel: validationRunning ? 'Validation is running…' : 'Checking config, drivers, and required files on disk', done: progress >= 92, active: progress >= 65 && progress < 92 },
    { label: 'Start downloads', sublabel: 'Starting kext and recovery setup', done: progress >= 100, active: progress >= 92 && progress < 100 },
  ];
  const kextStages = kextResults.map((k: any) => {
    const src = k.source === 'embedded'
      ? 'bundled fallback'
      : k.source === 'direct'
      ? 'direct download'
      : k.source === 'failed'
      ? 'FAILED'
      : 'GitHub';
    return {
      label: k.name,
      sublabel: k.source === 'failed'
        ? 'FAILED — download and bundled fallback unavailable'
        : `v${k.version} — ${src}`,
      done: true, active: false,
    };
  });
  const recovStages = [
    { label: 'Contact Apple server', sublabel: 'osrecovery.apple.com', done: recovPct >= 8, active: recovPct > 0 && recovPct < 8 },
    { label: 'Download BaseSystem.dmg', sublabel: recovStatus, done: recovPct >= 92, active: recovPct >= 8 && recovPct < 92 },
    { label: 'Download BaseSystem.chunklist', sublabel: 'Integrity checksum file', done: recovPct >= 100, active: recovPct >= 92 && recovPct < 100 },
  ];
  const flashStages = [
    { label: 'Erase & format drive', sublabel: 'GPT partition table + FAT32', done: progress >= 40, active: progress > 0 && progress < 40 },
    { label: 'Copy OpenCore EFI', sublabel: 'bootloader + kexts + SSDTs', done: progress >= 80, active: progress >= 40 && progress < 80 },
    { label: 'Write recovery image', sublabel: 'com.apple.recovery.boot', done: progress >= 90, active: progress >= 80 && progress < 90 },
    { label: 'Verify write integrity', sublabel: 'Confirming all files were written correctly', done: statusText === 'flash_complete', active: statusText === 'flash_verify' },
  ];
  const buildProgressNotice = buildFlowAlert
    ? {
        tone: buildFlowAlert.level === 'stalled' ? 'critical' as const : 'warning' as const,
        title: buildFlowAlert.level === 'stalled' ? 'Build needs attention' : 'Taking longer than expected',
        message: buildFlowAlert.pendingCondition
          ? `${buildFlowAlert.reason} Still waiting for ${buildFlowAlert.pendingCondition}.`
          : buildFlowAlert.reason,
      }
    : null;
  // ── Render ──────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-[#050505] text-[#EDEDED] font-sans flex items-center justify-center p-4 overflow-hidden relative">
      <div className="bg-grain" />
      {/* Background glows */}
      <div className="absolute inset-0 z-0 pointer-events-none overflow-hidden">
        {step === 'landing' ? (
          <>
            <motion.div 
              animate={{ 
                scale: [1, 1.2, 1], 
                translateX: ['-10%', '10%', '-10%'],
                translateY: ['-10%', '5%', '-10%'],
                opacity: [0.15, 0.25, 0.15] 
              }} 
              transition={{ duration: 25, repeat: Infinity, ease: "linear" }} 
              className="absolute top-[-30%] left-[-20%] w-[100%] h-[100%] bg-blue-600/40 rounded-full blur-[200px]" 
            />
            <motion.div 
              animate={{ 
                scale: [1, 1.3, 1], 
                translateX: ['10%', '-10%', '10%'],
                translateY: ['10%', '-5%', '10%'],
                opacity: [0.1, 0.2, 0.1] 
              }} 
              transition={{ duration: 30, repeat: Infinity, ease: "linear", delay: 2 }} 
              className="absolute bottom-[-30%] right-[-20%] w-[90%] h-[90%] bg-purple-600/30 rounded-full blur-[200px]" 
            />
          </>
        ) : (
          <>
            <div className="absolute top-[-28%] left-[-18%] h-[95%] w-[95%] rounded-full bg-blue-600/22 blur-[200px]" />
            <div className="absolute bottom-[-28%] right-[-18%] h-[82%] w-[82%] rounded-full bg-purple-600/16 blur-[200px]" />
          </>
        )}
      </div>

      <AnimatePresence mode="wait">
        {/* ── LANDING ── */}
        {step === 'landing' && (
          <motion.div 
            key="landing" 
            initial={{ opacity: 0, scale: 0.98, y: 10 }} 
            animate={{ opacity: 1, scale: 1, y: 0 }} 
            exit={{ opacity: 0, scale: 1.02, y: -10 }}
            transition={{ type: "spring", stiffness: 100, damping: 20 }}
            className="z-10 flex max-h-[calc(100vh-2rem)] w-full max-w-2xl flex-col items-center overflow-y-auto px-6 py-4 text-center custom-scrollbar"
          >
            <motion.div 
              initial={{ opacity: 0, y: 20 }}
              animate={{ 
                opacity: 1, 
                y: [0, -10, 0],
                transition: {
                  opacity: { duration: 1 },
                  y: { duration: 6, repeat: Infinity, ease: "easeInOut" }
                }
              }}
              className="w-32 h-32 mb-8 flex items-center justify-center rounded-[2rem] bg-white/5 border border-white/10 backdrop-blur-xl shadow-2xl relative"
            >
              <div className="absolute inset-0 rounded-[2.5rem] bg-blue-500 blur-3xl opacity-20 pointer-events-none" />
              <BrandIcon className="w-20 h-20 text-white relative z-10" />
            </motion.div>
            
            <motion.div
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.2 }}
            >
              <h1 className="text-7xl font-black tracking-tighter mb-4 bg-clip-text text-transparent bg-gradient-to-b from-white to-white/40 leading-[0.9]">
                Install <br/> macOS
              </h1>
            </motion.div>

            <motion.p 
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.3 }}
              className="text-lg text-white/50 mb-8 leading-relaxed font-medium max-w-lg"
            >
              Scan your hardware. Build an OpenCore EFI. Write a bootable installer. Done.
            </motion.p>

            <motion.div 
              initial={{ opacity: 0, y: 20 }} 
              animate={{ opacity: 1, y: 0 }} 
              transition={{ delay: 0.4 }} 
              className="flex flex-col items-center gap-4"
            >
              <div className="flex w-full max-w-xl flex-col gap-4 sm:flex-row">
                <button
                  onClick={() => {
                    if (adminPrivileges === false && platform !== 'darwin') {
                      setErrorWithSuggestion(
                        platform === 'win32'
                          ? 'Please run the app as Administrator to continue. Right-click the .exe and select "Run as administrator".'
                          : 'Disk operations require privilege elevation. Install polkit (sudo apt install policykit-1) so the app can prompt for your password when needed. Do not run the entire app as root.'
                      );
                      return;
                    }
                    setStep('welcome');
                  }}
                  className="group flex min-w-0 flex-1 items-center justify-center gap-3 overflow-hidden rounded-2xl bg-white px-10 py-5 text-lg font-bold text-black shadow-[0_20px_50px_rgba(255,255,255,0.12)] transition-all hover:scale-105 active:scale-95 cursor-pointer relative">
                  <div className="absolute inset-0 bg-gradient-to-r from-transparent via-white/20 to-transparent translate-x-[-100%] group-hover:translate-x-[100%] transition-transform duration-700 ease-in-out" />
                  Begin Installation
                </button>
                <button onClick={() => setStep('troubleshooting')} className="flex min-w-0 flex-1 items-center justify-center gap-3 rounded-2xl border border-white/10 bg-white/5 px-10 py-5 text-lg font-bold text-white transition-all hover:bg-white/10 hover:scale-105 active:scale-95 cursor-pointer backdrop-blur-md">
                  <HelpCircle className="w-5 h-5 text-white/40" /> Troubleshoot
                </button>
              </div>
              <div className="w-full max-w-xl">
                <UpdaterPanel
                  state={appUpdateState}
                  onRefresh={() => { void checkForAppUpdates(); }}
                  onPrimaryAction={handlePrimaryUpdateAction}
                  onOpenRelease={() => { void openLatestReleasePage(); }}
                />
              </div>
            </motion.div>

            <motion.p 
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: 1 }}
              className="mt-8 text-[10px] text-white/20 font-mono uppercase tracking-[0.4em]"
            >
              OpCore-OneClick
            </motion.p>
          </motion.div>
        )}

        {/* ── WIZARD SHELL ── */}
        {step !== 'landing' && (
          <motion.div key="shell" initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
            className="z-10 flex h-[min(820px,calc(100vh-2rem))] w-full max-w-[min(1560px,calc(100vw-2rem))] glass-card overflow-hidden">

            {/* Sidebar */}
            <div className="w-64 border-r border-white/5 bg-black/20 flex flex-col p-6 gap-1 flex-shrink-0">
              <button 
                onClick={() => {
                  setStep('landing');
                  setProfile(null);
                  setEfiPath(null);
                  try { window.electron.clearState(); } catch(e) {}
                  setDisclaimerAccepted(false);
                }}
                className="flex items-center gap-2.5 mb-8 px-2 hover:opacity-75 transition-opacity cursor-pointer text-left"
              >
                <BrandIcon className="w-5 h-5 text-white" />
                <span className="font-bold text-sm tracking-wide text-white">OpCore-OneClick</span>
              </button>

              {SIDEBAR_STEPS.map(s => (React.createElement(SidebarItem as any, { key: s.id, id: s.id, label: s.label, icon: s.icon })))}

              <div className="mt-2 pt-4 border-t border-white/5">
                <button onClick={() => setStep('troubleshooting')} className={`w-full flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-medium transition-all cursor-pointer ${step === 'troubleshooting' ? 'bg-white/10 text-white' : 'text-[#555] hover:bg-white/5'}`}>
                  <HelpCircle className="w-3.5 h-3.5" /> Troubleshooting
                </button>
              </div>

              {/* Confidence indicator */}
              {preflightReport && (
                <div className="mt-2 px-3">
                  <div className={`flex items-center gap-2 px-3 py-2 rounded-lg text-xs font-bold ${
                    confidence === 'green' ? 'bg-emerald-500/10 text-emerald-400' :
                    confidence === 'yellow' ? 'bg-amber-500/10 text-amber-400' :
                    'bg-red-500/10 text-red-400'
                  }`}>
                    <div className={`w-2 h-2 rounded-full ${
                      confidence === 'green' ? 'bg-emerald-400' :
                      confidence === 'yellow' ? 'bg-amber-400' :
                      'bg-red-400'
                    }`} />
                    {confidence === 'green' ? 'All systems verified' :
                     confidence === 'yellow' ? `${preflightReport.warnings.length} warning${preflightReport.warnings.length !== 1 ? 's' : ''}` :
                     `${preflightReport.blockers.length} issue${preflightReport.blockers.length !== 1 ? 's' : ''} detected`}
                  </div>
                  {confidence !== 'green' && preflightReport.warnings.length > 0 && (
                    <div className="mt-1.5 space-y-1">
                      {preflightReport.warnings.slice(0, 3).map((w, i) => (
                        <p key={i} className="text-[9px] text-white/30 leading-tight">{w}</p>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* Trust mode indicator — deterministic layer */}
              {buildPlan && (
                <div className="mt-2 px-3">
                  <div className={`flex items-center gap-2 px-3 py-2 rounded-lg text-xs font-bold ${
                    certainty === 'will_succeed' ? 'bg-emerald-500/10 text-emerald-400' :
                    certainty === 'may_fail' ? 'bg-amber-500/10 text-amber-400' :
                    'bg-red-500/10 text-red-400'
                  }`}>
                    <div className={`w-2 h-2 rounded-full ${
                      certainty === 'will_succeed' ? 'bg-emerald-400' :
                      certainty === 'may_fail' ? 'bg-amber-400' :
                      'bg-red-400'
                    }`} />
                    {certainty === 'will_succeed' ? 'Build will succeed' :
                     certainty === 'may_fail' ? 'Build may fail' :
                     'Build will fail — blocked'}
                  </div>
                  {buildPlan.failedComponents > 0 && (
                    <p className="text-[9px] text-white/30 mt-1 leading-tight">
                      {buildPlan.verifiedComponents}/{buildPlan.totalComponents} components verified
                    </p>
                  )}
                </div>
              )}

              {/* Profile footer */}
              {profile && (
                <div className="mt-auto pt-6 border-t border-white/5 px-2 space-y-2">
                  <div className="text-[9px] text-[#444] font-bold uppercase tracking-widest">Target</div>
                  <div className="text-xs font-semibold text-[#777] truncate">{profile.targetOS}</div>
                  <div className="text-[9px] text-[#444] font-bold uppercase tracking-widest mt-1">SMBIOS</div>
                  <div className="text-xs font-semibold text-[#777]">{profile.smbios}</div>
                  <button
                    onClick={handlePrimaryUpdateAction}
                    disabled={appUpdateState?.checking || appUpdateState?.downloading || appUpdateState?.installing}
                    className="mt-3 flex w-full min-w-0 items-center gap-2 rounded-xl border border-white/10 bg-white/[0.04] px-3 py-2 text-left text-xs font-semibold text-white/65 transition-colors hover:bg-white/[0.08] hover:text-white cursor-pointer disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    <span className="flex min-w-0 flex-1 items-center gap-2">
                      {appUpdateState?.checking || (!appUpdateState?.restartRequired && !appUpdateState?.readyToInstall && !appUpdateState?.available) ? (
                        <RefreshCcw className={`w-3.5 h-3.5 flex-shrink-0 text-white/40 ${appUpdateState?.checking ? 'animate-spin' : ''}`} />
                      ) : (
                        <Download className="w-3.5 h-3.5 flex-shrink-0 text-white/40" />
                      )}
                      <span className="min-w-0 flex-1">
                        <span className="block break-words leading-snug">
                          {appUpdateState?.restartRequired
                            ? 'Restart to finish update'
                            : appUpdateState?.readyToInstall
                            ? 'Install downloaded update'
                            : appUpdateState?.available
                            ? 'Download latest update'
                            : appUpdateState?.checking
                            ? 'Refreshing update status'
                            : 'Check for updates'}
                        </span>
                        <span className="mt-0.5 block text-[10px] font-medium text-white/35">
                          {appUpdateState?.checking
                            ? 'Checking the latest release now'
                            : appUpdateState?.latestVersion && appUpdateState.available
                            ? `${appUpdateState.latestVersion} is available`
                            : appUpdateState?.lastCheckedAt
                            ? `Checked at ${new Date(appUpdateState.lastCheckedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
                            : 'See the latest published build'}
                        </span>
                      </span>
                    </span>
                    {appUpdateState?.restartRequired || appUpdateState?.available || appUpdateState?.readyToInstall ? (
                      <Download className="w-3.5 h-3.5 flex-shrink-0 text-white/30" />
                    ) : (
                      <RefreshCcw className={`w-3.5 h-3.5 flex-shrink-0 text-white/30 ${appUpdateState?.checking ? 'animate-spin' : ''}`} />
                    )}
                  </button>
                </div>
              )}
            </div>

            {/* Content */}
            <div className="flex-1 min-h-0 flex flex-col relative">
              <div className="relative flex-shrink-0 px-6 pt-8 pb-4 sm:px-8 xl:px-12">
                <div className="absolute top-4 right-8 opacity-10 pointer-events-none flex items-center gap-2">
                  <BrandIcon className="w-4 h-4 text-white" />
                  <span className="text-[10px] font-bold uppercase tracking-widest">OpCore-OneClick</span>
                </div>
                {(() => {
                  const backMap: Partial<Record<StepId, StepId>> = {
                    'welcome': 'landing',
                    'prereq': 'welcome',
                    'precheck': 'prereq',
                    'version-select': 'precheck',
                    'report': 'version-select',
                    'bios': 'report',
                    'recovery-download': 'bios',
                    'method-select': 'recovery-download',
                    'usb-select': 'method-select',
                    'part-prep': 'method-select',
                  };
                  const target = backMap[step];
                  if (!target) return null;
                  return (
                    <button
                      onClick={() => setStep(target)}
                      className="flex items-center gap-2 rounded-full border border-white/10 bg-white/[0.04] px-4 py-2 text-sm font-medium text-white/68 shadow-[0_16px_40px_rgba(0,0,0,0.18)] backdrop-blur-xl transition-colors hover:bg-white/[0.07] hover:text-white"
                    >
                      <ChevronLeft className="w-4 h-4" /> Back
                    </button>
                  );
                })()}
              </div>
              <div className="relative flex-1 overflow-y-auto custom-scrollbar px-6 pb-10 sm:px-8 xl:px-12">
              <AnimatePresence mode="wait">

                {/* WELCOME */}
                {step === 'welcome' && (
                  <motion.div key="welcome" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <WelcomeStep onContinue={() => setStep('prereq')} />
                  </motion.div>
                )}

                {/* PREREQUISITES */}
                {step === 'prereq' && (
                  <motion.div key="prereq" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <PrereqStep onContinue={() => setStep('precheck')} />
                  </motion.div>
                )}

                {/* SYSTEM PRECHECK */}
                {step === 'precheck' && (
                  <motion.div key="precheck" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <PrecheckStep onContinue={() => { void startScan(); }} />
                  </motion.div>
                )}

                {/* SCANNING */}
                {step === 'scanning' && (
                  <motion.div key="scan" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ScanStep progress={progress} profile={profile} />
                  </motion.div>
                )}


                {/* VERSION SELECT */}
                {step === 'version-select' && compat && profile && (
                  <motion.div key="ver" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION}>
                    <VersionStep
                      report={compat}
                      matrix={compatibilityMatrix ?? buildCompatibilityMatrix(profile)}
                      selectedVersion={profile?.targetOS ?? compat.recommendedVersion}
                      onUseRecommendedVersion={() => {
                        if (!profile || !compat.recommendedVersion) return;
                        const selection = targetSelectionDecision(profile, compat.recommendedVersion);
                        setProfile(selection.profile);
                        setCompat(selection.compatibility);
                        setBiosConf(selection.biosConfig);
                        invalidateGeneratedBuild();
                        refreshBiosState(selection.profile).catch(() => {});
                        setStep(selection.nextStep);
                      }}
                      onSelect={v => {
                        if (!profile) return;
                        const selection = targetSelectionDecision(profile, v);
                        setProfile(selection.profile);
                        setCompat(selection.compatibility);
                        setBiosConf(selection.biosConfig);
                        invalidateGeneratedBuild();
                        refreshBiosState(selection.profile).catch(() => {});
                        setStep(selection.nextStep);
                      }}
                    />
                  </motion.div>
                )}

                {/* REPORT */}
                {step === 'report' && compat && profile && (
                  <motion.div key="rep" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ReportStep
                      profile={profile}
                      report={compat}
                      matrix={compatibilityMatrix ?? buildCompatibilityMatrix(profile)}
                      interpretation={hwInterpretation}
                      profileArtifact={profileArtifact}
                      resourcePlan={visibleResourcePlan}
                      planningOnly={!hasLiveHardwareContext}
                      planningProfileContext={planningProfileContext}
                      simulationResult={safeSimulationResult}
                      simulationRunning={simulationRunning}
                      onSaveProfile={saveCurrentPlanningProfile}
                      onExportProfile={exportCurrentPlanningProfile}
                      onImportProfile={importPlanningProfile}
                      onRunSimulation={runSafeSimulationPreview}
                      onRunLiveScan={() => startScan('report')}
                      onContinue={() => setStep('bios')}
                    />
                  </motion.div>
                )}

                {/* BIOS */}
                {step === 'bios' && biosConf && (
                  <motion.div key="bios" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <BiosStep
                      biosConfig={biosConf}
                      biosStatus={biosStatus}
                      firmwareInfo={firmwareInfo}
                      orchestratorState={biosState}
                      resumeState={biosResumeState}
                      restartCapability={restartCapability}
                      onApplySupportedChanges={applySupportedBiosChanges}
                      onRecheckBios={recheckBiosState}
                      onContinueWithCurrentBiosState={continueFromCurrentBiosState}
                      onRestartToBios={restartToFirmwareWithSession}
                    />
                  </motion.div>
                )}

                {/* BUILDING */}
                {step === 'building' && (
                  <motion.div key="build" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ProgressStep
                      title="Building your EFI"
                      subtitle="Generating an OpenCore configuration for your hardware."
                      icon={Box}
                      progress={progress}
                      statusText={statusText}
                      notice={step === 'building' ? buildProgressNotice : null}
                      stages={buildStages}
                      onBegin={buildAutoStartRef.current || buildStartRequestedRef.current || buildFlow?.active ? undefined : startDeploy}
                      briefing={{
                        heading: 'What happens next',
                        bullets: [
                          'The app builds an OpenCore EFI for your hardware.',
                          'It downloads the required kexts.',
                          'It downloads Apple recovery files.',
                          'Then you choose where to write the installer.',
                        ],
                        estimatedMinutes: 15,
                        interruptionWarning: 'Downloads can resume later. Do not unplug the drive once writing starts.',
                      }}
                    />
                  </motion.div>
                )}

                {/* KEXT FETCH */}
                {step === 'kext-fetch' && (
                  <motion.div key="kext" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ProgressStep title="Downloading Kexts" subtitle="Downloading required kexts and validating bundled fallbacks." icon={Package} progress={progress} statusText={kextResults.length ? `${kextResults.length} ready` : 'Checking download sources…'} notice={step === 'kext-fetch' ? buildProgressNotice : null} stages={kextStages.length ? kextStages : [{ label: 'Checking download sources…', sublabel: '', done: false, active: true }]} />
                  </motion.div>
                )}

                {/* RECOVERY DOWNLOAD */}
                {step === 'recovery-download' && (
                  <motion.div key="recov" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full flex flex-col">
                    {/* Recovery Source Panel (Compact) */}
                    <div className="flex-shrink-0 mb-6 p-4 rounded-2xl bg-white/[0.03] border border-white/5 flex items-center justify-between gap-4">
                      <div className="flex items-center gap-3">
                        <div className="w-10 h-10 rounded-xl bg-blue-500/10 border border-blue-500/20 flex items-center justify-center">
                          <BrandIcon className="w-5 h-5 text-blue-400" />
                        </div>
                        <div>
                          <div className="text-[10px] font-bold text-white/30 uppercase tracking-widest">Acquisition Source</div>
                          <div className="text-sm font-bold text-white flex items-center gap-2">
                            {recovTask?.status === 'running' 
                              ? (recovTask.progress as any)?.sourceId === 'local_cache' ? 'Local Cache' : 'Apple Official'
                              : cachedRecovInfo ? (cachedRecovInfo.isPartial ? 'Resumable Cache' : 'Local Cache') : 'Apple CDN'}
                            {cachedRecovInfo && !cachedRecovInfo.isPartial && (
                              <span className="px-1.5 py-0.5 rounded-md bg-emerald-500/10 text-emerald-400 text-[9px] uppercase tracking-tighter border border-emerald-500/20">Verified</span>
                            )}
                          </div>
                        </div>
                      </div>

                      <div className="flex items-center gap-2">
                        {!recovTask && (
                          <button
                            onClick={handleImportRecovery}
                            className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] font-bold text-white/60 hover:bg-white/10 hover:text-white transition-all cursor-pointer flex items-center gap-1.5"
                          >
                            <Package className="w-3 h-3" /> Manual Import
                          </button>
                        )}
                        {cachedRecovInfo && (
                          <button
                            onClick={() => window.electron.clearRecoveryCache(profile?.targetOS || '')}
                            className="p-1.5 rounded-lg text-white/20 hover:text-red-400 hover:bg-red-500/10 transition-all cursor-pointer"
                            title="Clear recovery cache"
                          >
                            <RefreshCcw className="w-3 h-3" />
                          </button>
                        )}
                      </div>
                    </div>

                    {recovError ? (
                      <div className="space-y-5">
                        <div>
                          <h2 className="text-3xl font-bold text-white mb-2">Automatic recovery download is unavailable</h2>
                          <p className="text-[#888] text-sm">Apple's server rejected the request. This is an external service limitation, not a problem with your machine.</p>
                        </div>
                        <div className="p-4 rounded-2xl bg-red-500/8 border border-red-500/20">
                          <p className="text-xs text-red-300/80 font-mono">{recovError}</p>
                        </div>

                        <div className="space-y-2">
                          <p className="text-xs font-bold text-[#aaa]">What you can do:</p>

                          {/* Option 1: Retry */}
                          <button disabled={isRetryingRecovRef.current} onClick={async () => { if (isRetryingRecovRef.current) return; isRetryingRecovRef.current = true; setRecovError(null); setRecovPct(0); try { await window.electron.downloadRecovery(efiPath!, profile?.targetOS || 'macOS Sequoia 15'); try { const c = await (window.electron as any).verifyRecoverySuccess(efiPath!); if (!c.passed) { const failed = c.checks.filter((x: any) => !x.passed); setRecovError(`Recovery verification failed: ${failed.map((x: any) => `${x.name}: ${x.detail}`).join('; ')}`); setErrorWithSuggestion('Recovery download completed but verification failed. File may be incomplete.', 'recovery-download'); return; } } catch {} advanceToMethodSelect(efiPath!); } catch (e: any) { const msg = e.message || 'Retry failed'; const code = msg.includes('401') || msg.includes('403') || msg.includes('rejected') ? 'recovery_auth' : 'recovery_dl'; (window.electron as any).recordFailure(code, msg).catch(() => {}); setRecovError(msg); setErrorWithSuggestion(msg, 'recovery-download'); } finally { isRetryingRecovRef.current = false; } }}
                            className="w-full text-left p-4 bg-white/4 border border-white/8 rounded-xl hover:bg-white/8 transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-4">
                            <RefreshCcw className="w-5 h-5 text-white/60 shrink-0" />
                            <div>
                              <p className="text-sm font-bold text-white">Retry once more</p>
                              <p className="text-xs text-[#666]">Try Apple's servers again with a fresh request</p>
                            </div>
                          </button>

                          {/* Option 2: Use cached recovery */}
                          {cachedRecovInfo && !cachedRecovInfo.isPartial && (
                            <button onClick={async () => { setRecovError(null); setRecovPct(0); try { await window.electron.downloadRecovery(efiPath!, profile?.targetOS || 'macOS Sequoia 15'); try { const c = await (window.electron as any).verifyRecoverySuccess(efiPath!); if (!c.passed) { const failed = c.checks.filter((x: any) => !x.passed); setRecovError(`Cached recovery verification failed: ${failed.map((x: any) => `${x.name}: ${x.detail}`).join('; ')}`); return; } } catch {} advanceToMethodSelect(efiPath!); } catch (e: any) { setRecovError(e.message || 'Cache retrieval failed'); } }}
                              className="w-full text-left p-4 bg-emerald-500/8 border border-emerald-500/20 rounded-xl hover:bg-emerald-500/15 transition-all cursor-pointer flex items-center gap-4">
                              <HardDrive className="w-5 h-5 text-emerald-400 shrink-0" />
                              <div>
                                <p className="text-sm font-bold text-emerald-400">Use cached recovery</p>
                                <p className="text-xs text-emerald-400/60">A previously downloaded recovery image is available locally</p>
                              </div>
                              <span className="ml-auto text-xs bg-emerald-500/20 text-emerald-400 px-2 py-1 rounded-lg font-bold">Recommended</span>
                            </button>
                          )}

                          {/* Option 3: Import manually */}
                          <button onClick={handleImportRecovery}
                            className={`w-full text-left p-4 ${!cachedRecovInfo || cachedRecovInfo.isPartial ? 'bg-blue-500/8 border-blue-500/20' : 'bg-white/4 border-white/8'} border rounded-xl hover:bg-blue-500/12 transition-all cursor-pointer flex items-center gap-4`}>
                            <Download className="w-5 h-5 text-blue-400 shrink-0" />
                            <div>
                              <p className="text-sm font-bold text-blue-400">Import recovery image manually</p>
                              <p className="text-xs text-[#666]">Select a BaseSystem.dmg from another machine or download</p>
                            </div>
                            {(!cachedRecovInfo || cachedRecovInfo.isPartial) && (
                              <span className="ml-auto text-xs bg-blue-500/20 text-blue-400 px-2 py-1 rounded-lg font-bold">Recommended</span>
                            )}
                          </button>

                          {/* Option 4: Try different version */}
                          <button onClick={() => { setRecovError(null); setStep('version-select'); }}
                            className="w-full text-left p-4 bg-white/4 border border-white/8 rounded-xl hover:bg-white/8 transition-all cursor-pointer flex items-center gap-4">
                            <BrandIcon className="w-5 h-5 text-white/60 shrink-0" />
                            <div>
                              <p className="text-sm font-bold text-white">Try a different macOS version</p>
                              <p className="text-xs text-[#666]">Some versions have better availability than others</p>
                            </div>
                          </button>

                          {/* Option 5: EFI only */}
                          <button onClick={() => { advanceToMethodSelect(efiPath); }}
                            className="w-full text-left p-4 bg-white/4 border border-white/8 rounded-xl hover:bg-white/8 transition-all cursor-pointer flex items-center gap-4">
                            <Box className="w-5 h-5 text-amber-400/60 shrink-0" />
                            <div>
                              <p className="text-sm font-bold text-amber-400">Continue without macOS recovery</p>
                              <p className="text-xs text-[#666]">USB will have the bootloader only — you'll need to add a macOS installer separately before booting</p>
                            </div>
                          </button>
                        </div>
                      </div>
                    ) : (
                      <ProgressStep native title="Downloading Recovery Image" subtitle={`Fetching the ${profile?.targetOS || 'macOS'} recovery image (~700 MB). The full OS installs later from this image.`} icon={Package} progress={recovPct} statusText={recovStatus || 'Connecting…'} notice={step === 'recovery-download' ? buildProgressNotice : null} stages={recovStages} />
                    )}
                  </motion.div>
                )}

                {/* METHOD SELECT */}
                {step === 'method-select' && (
                  <motion.div key="meth" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <MethodStep onSelect={selectMethod} onBack={() => setStep('recovery-download')} platform={platform} />
                  </motion.div>
                )}

                {/* PARTITION PREP */}
                {step === 'part-prep' && (
                  <motion.div key="part" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <UsbStep devices={usbDevices} selected={selectedUsb} onSelect={handleUsbSelection} onRefresh={refreshPartitionTargets} requireFullSize={true} loading={usbRefreshBusy} />
                    {selectedUsb && (
                      <div className="pt-6 flex justify-end">
                        <button onClick={() => preparePartition(false)} className="px-8 py-3.5 bg-purple-600 text-white rounded-xl font-bold text-sm hover:bg-purple-500 transition-all cursor-pointer shadow-lg shadow-purple-600/20">
                          Deploy to Partition →
                        </button>
                      </div>
                    )}
                  </motion.div>
                )}

                {/* USB SELECT */}
                {step === 'usb-select' && (
                  <motion.div key="usb" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    {/* Beta label */}
                    <div className="flex items-center gap-2 mb-4">
                      <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-400 text-[10px] font-bold uppercase tracking-widest">
                        <ShieldAlert className="w-3 h-3" /> Experimental
                      </span>
                      <span className="text-[11px] text-white/30">USB flashing is in beta.</span>
                    </div>
                    <UsbStep
                      devices={usbDevices}
                      selected={selectedUsb}
                      backupPolicy={efiBackupPolicy}
                      onSelect={handleUsbSelection}
                      onDeselect={() => { handleUsbSelection(null); }}
                      onRefresh={refreshUsbTargets}
                      onConfirmDrive={initiateFlash}
                      confirmDriveBusy={flashConfirmBusy}
                      loading={usbRefreshBusy}
                      allowUnverifiedSelection={true}
                      requireFullSize={recovPct >= 100}
                    />
                  </motion.div>
                )}

                {/* FLASHING */}
                {step === 'flashing' && (
                  <motion.div key="flash" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ProgressStep title="Creating Bootable USB" subtitle="Writing OpenCore EFI and macOS recovery to your drive." icon={Usb} progress={progress} statusText={statusText} stages={flashStages} milestones={flashMilestones} onTroubleshoot={() => setStep('troubleshooting')} />
                  </motion.div>
                )}

                {/* COMPLETE */}
                {step === 'complete' && profile && (
                  <motion.div key="done" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full overflow-y-auto custom-scrollbar">
                    <CompleteStep profile={profile} efiPath={efiPath} productionLocked={productionLocked}
                      onOpenFolder={() => efiPath && window.electron.openFolder(efiPath)}
                      onProductionLock={async () => { if (efiPath) { try { await window.electron.enableProductionLock(efiPath, profile?.targetOS); setProdLock(true); } catch (e: any) { setErrorWithSuggestion(e.message || 'Failed to enable production lock'); } } }}
                      onRestart={() => window.electron.restartComputer().catch(() => {})}
                      onTroubleshoot={() => setStep('troubleshooting')} />

                    {/* System summary line */}
                    {efiReport && profile && (
                      <p className="mt-6 text-xs text-white/40 font-medium tracking-wide">
                        {profile.generation} {profile.cpu.split(/\s+/).find(w => /i[3579]|ryzen|xeon|threadripper/i.test(w)) ?? ''} · {efiReport.hardware.items.find(i => i.label === 'Graphics')?.value.split(/\s+/).slice(0, 3).join(' ') ?? profile.gpu.split(/\s+/).slice(0, 3).join(' ')} · {profile.targetOS.replace('macOS ', '')} target · {efiReport.confidenceLabel} build
                      </p>
                    )}

                    {/* EFI Intelligence Report toggle */}
                    {efiReport && (
                      <div className="mt-3 space-y-4">
                        <button
                          onClick={() => setShowEfiReport(v => !v)}
                          className="w-full flex items-center gap-4 px-5 py-4 rounded-2xl bg-white/[0.02] border border-white/6 hover:bg-white/[0.04] transition-colors cursor-pointer group"
                        >
                          {/* Mini confidence indicator */}
                          <div className="relative w-10 h-10 flex-shrink-0">
                            <svg className="w-10 h-10 -rotate-90" viewBox="0 0 40 40">
                              <circle cx="20" cy="20" r="16" fill="none" stroke="rgba(255,255,255,0.05)" strokeWidth="3" />
                              <circle cx="20" cy="20" r="16" fill="none"
                                stroke={efiReport.confidenceScore >= 75 ? '#34d399' : efiReport.confidenceScore >= 50 ? '#fbbf24' : '#ef4444'}
                                strokeWidth="3" strokeLinecap="round"
                                strokeDasharray={2 * Math.PI * 16}
                                strokeDashoffset={2 * Math.PI * 16 - (efiReport.confidenceScore / 100) * 2 * Math.PI * 16}
                              />
                            </svg>
                            <span className="absolute inset-0 flex items-center justify-center text-[10px] font-black text-white">{efiReport.confidenceScore}</span>
                          </div>
                          <div className="flex-1 text-left">
                            <div className="text-sm font-bold text-white/70 group-hover:text-white/90 transition-colors">EFI Intelligence Report</div>
                            <div className="text-[10px] text-white/30">{efiReport.kexts.length} kexts explained · {efiReport.limitations.length} known limitations · SMBIOS reasoning</div>
                          </div>
                          {showEfiReport ? <ChevronDown className="w-4 h-4 text-white/20" /> : <ChevronRight className="w-4 h-4 text-white/20" />}
                        </button>
                        {showEfiReport && <EfiReportPanel report={efiReport} />}
                      </div>
                    )}

                    {/* Community Knowledge */}
                    {communityIssues.length > 0 && (
                      <div className="mt-4">
                        <CommunityPanel issues={communityIssues} />
                      </div>
                    )}
                  </motion.div>
                )}

                {/* TROUBLESHOOTING */}
                {step === 'troubleshooting' && (
                  <motion.div key="help" initial={{ opacity: 0, x: 20 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0 }} className="space-y-5 h-full flex flex-col">
                    <div className="flex items-start justify-between flex-shrink-0">
                      <div>
                        <h2 className="text-4xl font-bold text-white mb-1">Troubleshooting</h2>
                        <p className="text-[#888] text-sm">{troubleshootingData.length} issues from Dortania's guides.</p>
                      </div>
                      <button onClick={() => setStep('landing')} className="p-2 hover:bg-white/5 rounded-full transition-colors cursor-pointer"><X className="w-5 h-5 text-[#444]" /></button>
                    </div>
                    <div className="relative flex-shrink-0">
                      <Search className="absolute left-4 top-1/2 -translate-y-1/2 w-4 h-4 text-[#555]" />
                      <input value={search} onChange={e => setSearch(e.target.value)} placeholder="Search errors, fixes, boot-args…" className="search-input" />
                    </div>
                    <div className="flex flex-wrap gap-2 flex-shrink-0">
                      {CATS.map(c => (
                        <button key={c} onClick={() => setCat(c)} className={`filter-chip ${cat === c ? 'active' : ''}`}>{c}</button>
                      ))}
                    </div>
                    <div className="flex-1 overflow-y-auto custom-scrollbar space-y-3 pr-2 -mr-2">
                      <AnimatePresence mode="popLayout">
                        {filteredIssues.map((it, i) => (
                          <motion.div 
                            layout
                            key={it.error} 
                            initial={{ opacity: 0, scale: 0.98, y: 10 }} 
                            animate={{ opacity: 1, scale: 1, y: 0 }} 
                            exit={{ opacity: 0, scale: 0.98, y: -10 }}
                            transition={{ 
                              type: "spring", 
                              stiffness: 300, 
                              damping: 30,
                              delay: i * 0.01 
                            }}
                            className={`magnetic-glow rounded-2xl border border-white/6 bg-white/3 severity-${it.severity} overflow-hidden`}
                          >
                          <div className="flex items-start gap-4 p-4 cursor-pointer hover:bg-white/2 transition-colors" onClick={() => setExpanded(p => { const n = new Set(p); n.has(i) ? n.delete(i) : n.add(i); return n; })}>
                            <div className="flex-1 min-w-0 space-y-1">
                              <div className={`text-[10px] font-bold uppercase tracking-widest ${it.severity === 'error' ? 'text-red-400' : it.severity === 'warning' ? 'text-amber-400' : 'text-blue-400'}`}>{it.category}</div>
                              <div className="text-sm font-bold text-white leading-snug break-words">{it.error}</div>
                            </div>
                            <motion.div animate={{ rotate: expanded.has(i) ? 180 : 0 }} className="flex-shrink-0"><ChevronDown className="w-4 h-4 text-[#444] mt-1" /></motion.div>
                          </div>
                          <AnimatePresence>
                            {expanded.has(i) && (
                              <motion.div 
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: 'auto', opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                className="overflow-hidden bg-white/[0.02]"
                              >
                                <div className="px-4 pb-4 pt-3 text-xs text-white/70 leading-relaxed border-t border-white/5 font-medium break-words whitespace-pre-wrap">{it.fix}</div>
                              </motion.div>
                            )}
                          </AnimatePresence>
                        </motion.div>
                      ))}
              </AnimatePresence>
              </div>
                  </motion.div>
                )}

	              </AnimatePresence>

	            </div>
	          </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── DISCLAIMER MODAL ─────────────────────────────────── */}
      <AnimatePresence>
        {showDisclaimer && (
          <motion.div
            key="disclaimer"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/80 backdrop-blur-md"
          >
            <motion.div
              initial={{ scale: 0.95, y: 20 }}
              animate={{ scale: 1, y: 0 }}
              exit={{ scale: 0.95, y: 20 }}
              className="bg-[#0d0d0f] border border-white/10 rounded-3xl max-w-lg w-full p-8 shadow-2xl"
            >
              <div className="flex items-center gap-3 mb-5">
                <div className="w-10 h-10 rounded-xl bg-amber-500/10 border border-amber-500/20 flex items-center justify-center">
                  <Info className="w-5 h-5 text-amber-400" />
                </div>
                <h2 className="text-xl font-bold text-white">Heads Up</h2>
              </div>
              <div className="space-y-3 text-sm text-white/60 leading-relaxed">
                <p>
                  <span className="text-white font-semibold">OpCore-OneClick</span> is a community project and is not affiliated with Apple.
                </p>
                <p>
                  Continue only if you are okay changing boot settings.
                </p>
              </div>
              <div className="flex gap-3 mt-6">
                <button
                  onClick={() => { setShowDisclaimer(false); }}
                  className="flex-1 py-3 rounded-xl bg-white/5 border border-white/10 text-white/50 text-sm font-medium hover:bg-white/8 transition-all cursor-pointer"
                >
                  Cancel
                </button>
                <button
                  onClick={() => { setDisclaimerAccepted(true); setShowDisclaimer(false); setShowRecoveryPrompt(true); }}
                  className="flex-1 py-3 rounded-xl bg-white text-black text-sm font-bold hover:bg-white/90 transition-all cursor-pointer"
                >
                  Continue
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── RECOVERY POINT PROMPT ────────────────────────────────── */}
      <AnimatePresence>
        {showRecoveryPrompt && (
          <motion.div
            key="recovery-prompt"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/80 backdrop-blur-md"
          >
            <motion.div
              initial={{ scale: 0.95, y: 20 }}
              animate={{ scale: 1, y: 0 }}
              exit={{ scale: 0.95, y: 20 }}
              className="bg-[#0d0d0f] border border-white/10 rounded-3xl max-w-lg w-full p-8 shadow-2xl"
            >
              <div className="flex items-center gap-3 mb-5">
                <div className="w-10 h-10 rounded-xl bg-rose-500/10 border border-rose-500/20 flex items-center justify-center">
                  <ShieldCheck className="w-5 h-5 text-rose-400" />
                </div>
                <h2 className="text-xl font-bold text-white">Create a Recovery Point First</h2>
              </div>
              <div className="space-y-3 text-sm text-white/60 leading-relaxed">
                <p>
                  Create a recovery point before you continue.
                </p>
                <div className="bg-white/4 border border-white/8 rounded-2xl p-4 space-y-2 text-xs font-mono">
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mb-1">Windows</p>
                  <p className="text-white/70">Search “Create a restore point” → System Protection → Create</p>
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mt-3 mb-1">Linux</p>
                  <p className="text-white/70">Use Timeshift or your distro's snapshot tool before continuing.</p>
                </div>
                <p className="text-amber-400/80 text-xs">
                  A restore point helps if you need to roll back boot changes.
                </p>
              </div>
              <div className="flex gap-3 mt-6">
                <button
                  onClick={() => setShowRecoveryPrompt(false)}
                  className="flex-1 py-3 rounded-xl bg-white/5 border border-white/10 text-white/50 text-sm font-medium hover:bg-white/8 transition-all cursor-pointer"
                >
                  Go Back
                </button>
                <button
                  onClick={() => { setShowRecoveryPrompt(false); setStep('prereq'); }}
                  className="flex-1 py-3 rounded-xl bg-white text-black text-sm font-bold hover:bg-white/90 transition-all cursor-pointer"
                >
                  Begin Installation
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── MBR PARTITION TABLE WARNING ─────────────────────────── */}
      <AnimatePresence>
        {showDiskWarning && diskInfo && (
          <motion.div
            key="mbr-warning"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/80 backdrop-blur-md"
          >
            <motion.div
              initial={{ scale: 0.95, y: 20 }}
              animate={{ scale: 1, y: 0 }}
              exit={{ scale: 0.95, y: 20 }}
              className="bg-[#0d0d0f] border border-rose-500/20 rounded-3xl max-w-lg w-full p-8 shadow-2xl"
            >
              <div className="flex items-center gap-3 mb-5">
                <div className="w-10 h-10 rounded-xl bg-rose-500/10 border border-rose-500/20 flex items-center justify-center">
                  <ShieldCheck className="w-5 h-5 text-rose-400" />
                </div>
                <div>
                  <h2 className="text-xl font-bold text-white">MBR Partition Table Detected</h2>
                  <p className="text-xs text-rose-400 font-mono mt-0.5">{diskInfo.device}</p>
                </div>
              </div>
              <div className="space-y-3 text-sm text-white/60 leading-relaxed mb-5">
                <p>
                  This drive uses an <span className="text-rose-300 font-semibold">MBR (Master Boot Record)</span> partition table.
                  OpenCore requires <span className="text-white font-semibold">GPT (GUID Partition Table)</span>.
                  Flashing to an MBR disk will fail or corrupt the partition table.
                </p>
                <p>Convert the disk to GPT first, then retry. <span className="text-amber-400">This will erase all data on the drive.</span></p>
              </div>
              <div className="bg-white/4 border border-white/8 rounded-2xl p-4 space-y-3 text-xs font-mono mb-6">
                <div>
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mb-1">macOS</p>
                  <p className="text-white/70">diskutil eraseDisk FAT32 OPENCORE GPTFormat {diskInfo.device}</p>
                </div>
                <div>
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mb-1">Windows (Admin PowerShell)</p>
                  <p className="text-white/70">Get-Disk X | Clear-Disk -RemoveData -Confirm:$false</p>
                  <p className="text-white/70">Initialize-Disk -Number X -PartitionStyle GPT</p>
                </div>
                <div>
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mb-1">Linux</p>
                  <p className="text-white/70">sudo parted {diskInfo.device} --script mklabel gpt</p>
                </div>
              </div>
              <div className="flex gap-3">
                <button
                  onClick={() => { setShowDiskWarning(false); setDiskInfo(null); }}
                  className="flex-1 py-3 rounded-xl bg-white/5 border border-white/10 text-white/50 text-sm font-medium hover:bg-white/8 transition-all cursor-pointer"
                >
                  Cancel — Select a Different Drive
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── UNKNOWN PARTITION TABLE WARNING ─────────────────────────── */}
      <AnimatePresence>
        {showUnknownPartitionWarning && diskInfo && (
          <motion.div
            key="unknown-pt-warning"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/80 backdrop-blur-md"
          >
            <motion.div
              initial={{ scale: 0.95, y: 20 }}
              animate={{ scale: 1, y: 0 }}
              exit={{ scale: 0.95, y: 20 }}
              className="bg-[#0d0d0f] border border-rose-500/20 rounded-3xl max-w-lg w-full p-8 shadow-2xl"
            >
              <div className="flex items-center gap-3 mb-5">
                <div className="w-10 h-10 rounded-xl bg-rose-500/10 border border-rose-500/20 flex items-center justify-center">
                  <ShieldCheck className="w-5 h-5 text-rose-400" />
                </div>
                <div>
                  <h2 className="text-xl font-bold text-white">Unrecognised Drive Structure</h2>
                  <p className="text-xs text-rose-400 font-mono mt-0.5">{diskInfo.device}</p>
                </div>
              </div>
              <div className="space-y-3 text-sm text-white/60 leading-relaxed mb-5">
                <p>
                  The partition table on this drive could not be read. Writing to an unrecognised device is highly unsafe — it could silently corrupt data if it's a proprietary format, raid volume, or encrypted disk.
                </p>
                <p>
                  To use this drive safely, eject and reconnect it. If the issue persists, format it to <span className="text-white font-semibold">GPT (GUID Partition Table)</span> using your system's disk utility, then retry.
                </p>
              </div>
              <div className="flex gap-3">
                <button
                  onClick={() => { setShowUnknownPartitionWarning(false); setDiskInfo(null); }}
                  className="flex-1 py-3 rounded-xl bg-white/5 border border-white/10 text-white/50 text-sm font-medium hover:bg-white/8 transition-all cursor-pointer"
                >
                  Cancel — Select a Different Drive
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── LOCAL PARTITION CONFIRMATION ─────────────────────────── */}
      <AnimatePresence>
        {showPartitionConfirm && selectedUsb && (
          <motion.div
            key="part-confirm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/80 backdrop-blur-md"
          >
            <motion.div
              initial={{ scale: 0.95, y: 20 }}
              animate={{ scale: 1, y: 0 }}
              exit={{ scale: 0.95, y: 20 }}
              className="bg-[#0d0d0f] border border-amber-500/20 rounded-3xl max-w-lg w-full p-8 shadow-2xl"
            >
              <div className="flex items-center gap-3 mb-5">
                <div className="w-10 h-10 rounded-xl bg-amber-500/10 border border-amber-500/20 flex items-center justify-center">
                  <ShieldAlert className="w-5 h-5 text-amber-400" />
                </div>
                <div>
                  <h2 className="text-xl font-bold text-white">Shrink System Partition?</h2>
                  <p className="text-xs text-amber-400 font-mono mt-0.5">{selectedUsb}</p>
                </div>
              </div>
              <div className="space-y-3 text-sm text-white/60 leading-relaxed mb-6">
                <p>
                  You are about to shrink your primary system partition by <span className="text-white font-bold">16 GB</span> to create a macOS bootstrap area.
                </p>
                <p>
                  While this is generally safe, it involves modifying your live partition table. <span className="text-amber-300 font-semibold">Ensure you have a full backup of your critical data before proceeding.</span>
                </p>
                <ul className="space-y-2 mt-4">
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-500 flex-shrink-0 mt-0.5" />
                    <span>The app will attempt to shrink the partition safely.</span>
                  </li>
                  <li className="flex items-start gap-2">
                    <CheckCircle className="w-4 h-4 text-emerald-500 flex-shrink-0 mt-0.5" />
                    <span>A new FAT32 partition will be created for OpenCore.</span>
                  </li>
                </ul>
              </div>
              <div className="flex gap-3">
                <button
                  onClick={() => setShowPartitionConfirm(false)}
                  className="flex-1 py-3 rounded-xl bg-white/5 border border-white/10 text-white/50 text-sm font-medium hover:bg-white/8 transition-all cursor-pointer"
                >
                  Cancel
                </button>
                <button
                  onClick={() => preparePartition(true)}
                  className="flex-[2] py-3 rounded-xl bg-amber-600 text-white text-sm font-bold hover:bg-amber-500 transition-all cursor-pointer shadow-lg shadow-amber-600/20"
                >
                  Yes, Shrink and Prepare →
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── FLASH CONFIRMATION MODAL ────────────────────────────── */}

      <AnimatePresence>
        {showFlashConfirm && selectedUsb && (() => {
          const dev = usbDevices.find(d => d.device === selectedUsb);
          const ptRaw = dev?.partitionTable ?? diskInfo?.partitionTable;
          const pt = ptRaw === 'gpt' ? 'GPT' : ptRaw === 'mbr' ? 'MBR' : ptRaw === 'unknown' ? 'Unknown' : '—';
          const shortId = (() => {
            const d = selectedUsb ?? '';
            if (d.startsWith('/dev/')) return d.replace('/dev/', '');
            if (d.startsWith('\\\\.\\')) return d.replace('\\\\.\\', '');
            return d;
          })();
          const FLASH_CHECK_IDS = ['correct-drive', 'bios-reviewed'];
          const allChecked = FLASH_CHECK_IDS.every(id => flashChecks.has(id));
          const validationBlocked = validationResult?.overall === 'blocked';
          const confirmationExpired = flashConfirmationExpiresAt !== null && Date.now() > flashConfirmationExpiresAt;
          const confirmValid = allChecked && flashConfirmText === shortId && !validationBlocked && !!flashConfirmationToken && !confirmationExpired;
          const toggleCheck = (id: string) => setFlashChecks(prev => {
            const n = new Set(prev);
            n.has(id) ? n.delete(id) : n.add(id);
            return n;
          });
          return (
            <motion.div
              key="flash-confirm"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/85 backdrop-blur-md overflow-y-auto"
            >
              <motion.div
                initial={{ scale: 0.95, y: 20 }}
                animate={{ scale: 1, y: 0 }}
                exit={{ scale: 0.95, y: 20 }}
                className="bg-[#0d0d0f] border border-red-500/25 rounded-3xl max-w-lg w-full p-8 shadow-2xl my-4"
              >
                {/* Header */}
                <div className="flex items-center gap-3 mb-6">
                  <div className="w-10 h-10 rounded-xl bg-red-500/12 border border-red-500/25 flex items-center justify-center flex-shrink-0">
                    <ShieldAlert className="w-5 h-5 text-red-400" />
                  </div>
                  <div>
                    <h2 className="text-xl font-bold text-white">Confirm the drive</h2>
                    <p className="text-[10px] text-red-400/70 font-mono uppercase tracking-widest mt-0.5">Drive will be erased</p>
                  </div>
                </div>

                {/* Drive details highlight box */}
                <div className="bg-red-500/6 border border-red-500/20 rounded-2xl overflow-hidden mb-5">
                  {[
                    { label: 'Drive name', value: dev?.name ?? selectedUsb ?? '—' },
                    { label: 'Size', value: dev?.size ?? '—' },
                    { label: 'Disk identifier', value: selectedUsb ?? '—' },
                    { label: 'Partition table', value: pt },
                    { label: 'Removable', value: dev?.removable === true ? 'Yes' : dev?.removable === false ? 'No' : 'Unknown' },
                  ].map(({ label, value }) => (
                    <div key={label} className="flex items-center justify-between px-5 py-2.5 border-b border-red-500/10 last:border-0">
                      <span className="text-xs text-white/35 font-medium w-36">{label}</span>
                      <span className="text-sm font-mono text-white/80 text-right truncate max-w-[200px]">{value}</span>
                    </div>
                  ))}
                </div>

                {/* Irreversible warning */}
                <div className="mb-5 flex items-start gap-3 px-4 py-3 rounded-2xl bg-red-500/8 border border-red-500/20">
                  <ShieldAlert className="w-4 h-4 text-red-400 flex-shrink-0 mt-0.5" />
                  <p className="text-xs text-red-300/80 leading-relaxed">
                    <span className="font-bold text-red-300">This drive will be erased.</span>
                  </p>
                </div>

                {/* Identifier instability reminder (Gap 3) */}
                <div className="mb-5 flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/6 border border-amber-500/15">
                  <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
                  <p className="text-xs text-amber-300/70 leading-relaxed">
                    <span className="font-bold text-amber-300">Reminder:</span> Replugging the drive can change its identifier.
                  </p>
                </div>

                {/* EFI validation summary */}
                <div className="mb-5">
                  <ValidationSummary result={validationResult} isRunning={validationRunning} />
                </div>

                {/* 4-checkbox final checklist */}
                <div className="mb-5 space-y-2">
                  <div className="text-[10px] font-bold uppercase tracking-widest text-white/25 mb-2">Before you flash</div>
                  {([
                    { id: 'correct-drive', label: 'This is the right drive' },
                    { id: 'bios-reviewed', label: 'My BIOS settings are ready' },
                  ] as { id: string; label: string }[]).map(({ id, label }) => (
                    <label key={id} className="flex items-start gap-3 cursor-pointer group">
                      <div
                        onClick={() => toggleCheck(id)}
                        className={`w-5 h-5 rounded-md border flex-shrink-0 mt-0.5 flex items-center justify-center transition-all cursor-pointer ${
                          flashChecks.has(id)
                            ? 'bg-red-500 border-red-500'
                            : 'bg-white/5 border-white/15 group-hover:border-white/30'
                        }`}
                      >
                        {flashChecks.has(id) && <Check className="w-3 h-3 text-white stroke-[3px]" />}
                      </div>
                      <span
                        onClick={() => toggleCheck(id)}
                        className={`text-xs leading-relaxed transition-colors ${flashChecks.has(id) ? 'text-white/70' : 'text-white/40'}`}
                      >
                        {label}
                      </span>
                    </label>
                  ))}
                </div>

                {/* Typed confirmation — user must type the disk identifier */}
                <div className="mb-6">
                  <label className="block text-xs text-white/40 mb-2 leading-relaxed">
                    Type disk id:{' '}
                    <span className="font-mono font-bold text-white/70">{shortId}</span>
                  </label>
                  <input
                    type="text"
                    value={flashConfirmText}
                    onChange={e => setFlashConfirmText(e.target.value)}
                    placeholder={shortId}
                    onKeyDown={e => {
                      if (e.key === 'Enter' && confirmValid) executeFlash();
                    }}
                    className="w-full px-4 py-3 bg-white/4 border border-white/10 rounded-xl text-white font-mono text-sm placeholder-white/15 focus:outline-none focus:border-white/25 transition-colors"
                  />
                  {confirmationExpired && (
                    <p className="mt-2 text-[11px] text-red-300/70 leading-relaxed">
                      Confirmation expired. Reopen this dialog to continue.
                    </p>
                  )}
                </div>

                {/* Buttons — Cancel is autoFocus (safe default); Confirm is right */}
                <div className="flex gap-3">
                  <button
                    onClick={() => { clearFlashConfirmationState(); }}
                    autoFocus
                    className="flex-1 py-3 rounded-xl bg-white/8 border border-white/15 text-white/70 text-sm font-bold hover:bg-white/12 transition-all cursor-pointer"
                  >
                    Cancel
                  </button>
                  <button
                    disabled={!confirmValid}
                    onClick={executeFlash}
                    className="flex-1 py-3 rounded-xl text-sm font-bold transition-all cursor-pointer bg-red-600 text-white hover:bg-red-500 shadow-lg shadow-red-600/20 disabled:opacity-25 disabled:cursor-not-allowed disabled:shadow-none"
                  >
                    Flash drive
                  </button>
                </div>
              </motion.div>
            </motion.div>
          );
        })()}
      </AnimatePresence>

      <AnimatePresence>
        {globalNotice && (
          <motion.div
            key="global-notice"
            initial={{ opacity: 0, y: 20, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 20, scale: 0.98 }}
            className="fixed bottom-6 left-1/2 z-40 w-[92vw] max-w-xl -translate-x-1/2 rounded-3xl border border-emerald-500/18 bg-[#0b0b0d]/94 px-5 py-4 shadow-2xl backdrop-blur-xl"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="space-y-1">
                <div className="text-[11px] font-black uppercase tracking-[0.22em] text-emerald-300/70">Notice</div>
                <p className="text-sm leading-relaxed text-white/80">{globalNotice}</p>
              </div>
              <button onClick={() => setGlobalNotice(null)} className="rounded-xl p-2 text-white/25 transition-colors hover:bg-white/6 hover:text-white/70">
                <X className="h-4 w-4" />
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <AnimatePresence>
        {globalError && recoveryView && (
          <motion.div key="global-error-recovery" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
            <FailureRecoveryPanel
              title={recoveryView.title}
              whatFailed={recoveryView.whatFailed}
              likelyCause={recoveryView.likelyCause}
              nextActions={recoveryView.nextActions}
              technicalDetails={recoveryView.technicalDetails}
              onDismiss={() => setGlobalError(null)}
              actions={[
                {
                  label: 'Retry',
                  onClick: handleRecoveryRetry,
                  tone: 'primary',
                },
                {
                  label: 'Back to Safety',
                  onClick: handleBackToSafety,
                  tone: 'subtle',
                },
              ]}
              extra={(
                <div className="grid gap-3 md:grid-cols-3">
                  <CopyDiagnosticsButton
                    extraContext={recoveryPayload?.contextNote ?? (typeof globalError === 'string' ? globalError : JSON.stringify(globalError))}
                    className="w-full justify-center rounded-2xl border border-white/10 bg-white/5 py-3 text-sm"
                  />
                  <button
                    onClick={() => void handleSaveSupportLog()}
                    className="rounded-2xl border border-emerald-500/20 bg-emerald-500/10 px-4 py-3 text-sm font-bold text-emerald-100 transition-all hover:bg-emerald-500/20"
                  >
                    Save Log
                  </button>
                  <button
                    onClick={() => void handleOpenIssueReport()}
                    className="rounded-2xl border border-rose-500/20 bg-rose-500/10 px-4 py-3 text-sm font-bold text-rose-200 transition-all hover:bg-rose-500/20"
                  >
                    Open Issue
                  </button>
                </div>
              )}
            />
          </motion.div>
        )}
      </AnimatePresence>
      {/* ── DEBUG OVERLAY ─────────────────────────────────────────── */}
      <AnimatePresence>
        {debugOpen && (
          <DebugOverlay
            appVersion="2.3.3"
            platform={platform}
            sessionId={debugSessionId}
            currentStep={step}
            selectedVersion={profile?.targetOS ?? null}
            installMethod={method}
            profile={profile}
            beginnerSafetyMode={BEGINNER_SAFETY_MODE}
            selectedDisk={selectedUsb}
            diskTier={null}
            flashConfirmed={flashChecks.size >= 4}
            tasks={tasks}
            firmwareHostContext={firmwareInfo?.hostContext ?? null}
            hwConfidence={hwInterpretation?.overallConfidence ?? profile?.scanConfidence ?? null}
            secureBootStatus={biosStatus?.secureBootDisabled === true ? false : biosStatus?.secureBootDisabled === false ? true : biosStatus?.secureBootDisabled ?? null}
            lastError={globalError}
            lastWarning={null}
            recentEvents={recentEvents}
            watchdogTriggers={watchdogCount}
            recoveryAttempts={recoveryTryCount}
            lastSuggestion={lastSuggestion}
            onCopyDiagnostics={async () => {
              try {
                const d = await window.electron.getDiagnostics();
                const text = JSON.stringify(d, null, 2);
                await navigator.clipboard.writeText(text);
              } catch {}
            }}
            onCopyLog={async () => {
              try {
                const logs = await window.electron.getLogTail(50);
                const text = logs.map((l: any) => JSON.stringify(l)).join('\n');
                await navigator.clipboard.writeText(text);
              } catch {}
            }}
            onClearLog={() => {
              window.electron.logClear();
              setRecentEvents([]);
              setWatchdogCount(0);
              setRecoveryTryCount(0);
            }}
            onOpenLogFolder={async () => {
              try {
                const logPath = await window.electron.getLogPath();
                await window.electron.openFolder(logPath);
              } catch {}
            }}
            onClose={() => setDebugOpen(false)}
          />
        )}
      </AnimatePresence>

      {/* Debug trigger dot — nearly invisible */}
      {!debugOpen && (
        <button
          onClick={() => setDebugOpen(true)}
          className="fixed bottom-2 right-2 w-1 h-1 rounded-full bg-white/[0.08] hover:bg-white/30 transition-opacity z-50 cursor-pointer"
          aria-label="Open debug panel"
        />
      )}

    </div>
  );
}
