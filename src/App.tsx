import React, { useState, useEffect, useMemo, useRef } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import {
  CheckCircle, Usb, Search, Box, Download, Settings, ChevronRight,
  HardDrive, ShieldCheck, ShieldAlert, Check, Info, AlertTriangle,
  X, HelpCircle, Package, RefreshCcw, ChevronDown, ChevronLeft
} from 'lucide-react';
import BrandIcon from './components/BrandIcon';
import { getBIOSSettings, getRequiredResources, getSMBIOSForProfile, type HardwareProfile, type BIOSConfig } from '../electron/configGenerator';
import { checkCompatibility, type CompatibilityReport } from '../electron/compatibility';
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
import CopyDiagnosticsButton from './components/CopyDiagnosticsButton';
import DebugOverlay from './components/DebugOverlay';
import ValidationSummary from './components/ValidationSummary';
import EfiReportPanel from './components/EfiReport';
import CommunityPanel from './components/CommunityPanel';
import type { ValidationResult } from '../electron/configValidator';
import { generateEfiReport, type EfiReport } from './lib/efiReport';
import { getRelevantIssues, type CommunityIssue } from './data/communityKnowledge';
import { useTaskManager } from './hooks/useTaskManager';
import { BEGINNER_SAFETY_MODE } from './config';
import { getSuggestionPayload, type Suggestion } from './lib/suggestionEngine';
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
import { evaluateStepTransition, type StepId } from './lib/installStepGuards.js';
import type { PreflightReport, ConfidenceLevel } from '../electron/preventionLayer';
import type { BuildPlan, RecoveryDryRun, Certainty } from '../electron/deterministicLayer';
import type {
  HardwareProfileArtifact,
  HardwareProfileInterpretationMetadata,
} from '../electron/hardwareProfileArtifact';
import type { EfiBackupPolicy } from '../electron/efiBackup';
import type { ResourcePlan } from '../electron/resourcePlanner';
import type { SafeSimulationResult } from '../electron/safeSimulation';
import type { PublicDiagnosticsSnapshot } from '../electron/releaseDiagnostics';
type KextFetchResult = { name: string; version: string; source?: 'github' | 'embedded' | 'failed' };

declare global {
  interface Window {
    electron: {
      scanHardware: () => Promise<{ profile: HardwareProfile; interpretation: import('../electron/hardwareInterpret').HardwareInterpretation | null; artifact: HardwareProfileArtifact }>;
      getLatestHardwareProfile: () => Promise<HardwareProfileArtifact | null>;
      saveHardwareProfile: (payload: { profile: HardwareProfile; interpretation?: HardwareProfileInterpretationMetadata | null; source?: HardwareProfileArtifact['source'] }) => Promise<HardwareProfileArtifact>;
      exportHardwareProfile: (artifact?: HardwareProfileArtifact | null) => Promise<{ filePath: string; artifact: HardwareProfileArtifact } | null>;
      importHardwareProfile: () => Promise<HardwareProfileArtifact | null>;
      inspectEfiBackupPolicy: (device: string) => Promise<EfiBackupPolicy>;
      buildEFI: (p: HardwareProfile) => Promise<string>;
      fetchLatestKexts: (efi: string, ks: string[]) => Promise<KextFetchResult[]>;
      downloadRecovery: (dir: string, osv: string, startOffset?: number) => Promise<{ dmgPath: string; recoveryDir: string }>;
      listUsbDevices: () => Promise<{ name: string; device: string; size: string }[]>;
      prepareFlashConfirmation: (dev: string, efi: string, expectedIdentity?: { devicePath?: string; sizeBytes?: number; model?: string; vendor?: string; serialNumber?: string; transport?: string; removable?: boolean; partitionTable?: string }) => Promise<{ token: string; expiresAt: number; diskInfo: { device: string; devicePath?: string; isSystemDisk: boolean; partitionTable: string; sizeBytes?: number; model?: string; vendor?: string; serialNumber?: string; transport?: string; removable?: boolean; identityConfidence?: string; identityFieldsUsed?: string[] }; backupPolicy: EfiBackupPolicy }>;
      flashUsb: (dev: string, efi: string, ok: boolean, confirmationToken?: string | null) => Promise<boolean>;
      validateEfi: (efiPath: string, profile?: import('../electron/configGenerator').HardwareProfile | null) => Promise<import('../electron/configValidator').ValidationResult>;
      enableProductionLock: (efi: string, targetOS?: string) => Promise<boolean>;
      getBiosState: (profile: import('../electron/configGenerator').HardwareProfile) => Promise<BiosOrchestratorState>;
      applySupportedBiosChanges: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<{ state: BiosOrchestratorState; appliedCount: number; message: string }>;
      verifyManualBiosChanges: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<BiosOrchestratorState>;
      restartToFirmwareWithSession: (profile: import('../electron/configGenerator').HardwareProfile, selectedChanges: Record<string, BiosSettingSelection>) => Promise<{ supported: boolean; error?: string; state: BiosOrchestratorState }>;
      clearBiosSession: () => Promise<boolean>;
      getBiosResumeState: () => Promise<import('../electron/bios/types').BiosResumeStateResponse>;
      getBiosRestartCapability: () => Promise<import('../electron/bios/types').FirmwareRestartCapability>;
      guardBuild: (profile: import('../electron/configGenerator').HardwareProfile) => Promise<import('./lib/stateMachine').FlowGuardResult>;
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
      getDiskInfo: (device: string) => Promise<{ device: string; devicePath?: string; isSystemDisk: boolean; partitionTable: string; sizeBytes?: number; model?: string; vendor?: string; serialNumber?: string; transport?: string; removable?: boolean; identityConfidence?: string; identityFieldsUsed?: string[] }>;
      runPreflight: () => Promise<{ ok: boolean; issues: Array<{ severity: string; message: string }>; adminPrivileges: boolean; binaries: Record<string, boolean>; freeSpaceMB: number }>;
      // Task manager
      onTaskUpdate: (cb: (payload: { task: import('../electron/taskManager').TaskState }) => void) => void;
      offTaskUpdate: () => void;
      taskList: () => Promise<import('../electron/taskManager').TaskState[]>;
      taskCancel: (taskId: string) => Promise<boolean>;
      // Enhanced logging
      getLogTail: (n: number) => Promise<Record<string, unknown>[]>;
      getOpsTail: (n: number) => Promise<Record<string, unknown>[]>;
      logClear: () => Promise<boolean>;
      getSessionId: () => Promise<string>;
      // Issue reporter
      reportIssue: () => Promise<{ success: boolean; body: string; baseUrl: string }>;
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
  const [usbDevices, setUsbDevices] = useState<import('./components/steps/UsbStep').DriveInfo[]>([]);
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
  const lastRecovSaveRef = useRef(0);
  const isDeployingRef = useRef(false);
  const isScanningRef = useRef(false);
  const [diskInfo, setDiskInfo] = useState<{ device: string; devicePath?: string; isSystemDisk: boolean; partitionTable: string; sizeBytes?: number; model?: string; vendor?: string; serialNumber?: string; transport?: string; removable?: boolean; identityConfidence?: string; identityFieldsUsed?: string[] } | null>(null);
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

  const buildKextSourceMap = (results: KextFetchResult[] = kextResults): Record<string, 'github' | 'embedded' | 'failed'> =>
    Object.fromEntries(
      results
        .filter((result): result is KextFetchResult & { source: 'github' | 'embedded' | 'failed' } => !!result.source)
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

  // ── Debug Overlay ──────────────────────────────────────────────
  const [debugOpen, setDebugOpen] = useState(false);
  const [debugSessionId, setDebugSessionId] = useState('');
  const [recentEvents, setRecentEvents] = useState<any[]>([]);
  const [watchdogCount, setWatchdogCount] = useState(0);
  const [recoveryTryCount, setRecoveryTryCount] = useState(0);
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
  const localBuildGuard = useMemo(
    () => evaluateBuildGuard({
      compatibilityBlocked,
      biosFlowState,
      releaseFlowState,
    }),
    [biosFlowState, compatibilityBlocked, releaseFlowState],
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
  const postBuildReady = !compatibilityBlocked && biosReady && buildReady && !!efiPath && !validationBlocked;

  // ── Task Manager ────────────────────────────────────────────────
  const { tasks, activeTask, cancelTask } = useTaskManager();
  const recovTask = activeTask('recovery-download');
  const kextTask  = activeTask('kext-fetch');
  const efiTask   = activeTask('efi-build');
  const flashTask = activeTask('usb-flash');

  const isImportingRef = useRef(false);
  const isRetryingRecovRef = useRef(false);
  const isFlashingRef = useRef(false);
  const lastRuntimeErrorRef = useRef<string | null>(null);
  const handleImportRecovery = async () => {
    if (!efiPath || !profile?.targetOS || isImportingRef.current) return;
    isImportingRef.current = true;
    setRecovError(null);
    try {
      const res = await window.electron.importRecovery(efiPath, profile.targetOS);
      if (res) {
        setRecovPct(100);
        setRecovStatus('Recovery imported manually.');
        setStep('method-select');
      } else {
        setRecovError('Import returned no result — the file may be invalid or missing.');
      }
    } catch (e: any) {
      const msg = e.message || 'Recovery import failed';
      setRecovError(msg);
      setErrorWithSuggestion(msg, 'recovery-download');
    } finally { isImportingRef.current = false; }
  };

  /** Set a global error with context-aware suggestion.
   *  Tracks retry counts per error code so suggestions evolve on repeated failures. */
  const setErrorWithSuggestion = (
    errorMessage: string,
    overrideStep?: string,
    options?: {
      validationResult?: ValidationResult | null;
      kextSources?: Record<string, 'github' | 'embedded' | 'failed'>;
    },
  ) => {
    const trace = options?.validationResult?.firstFailureTrace ?? validationResult?.firstFailureTrace ?? null;
    // Pre-compute a rough code for retry counting (before full payload build)
    const msgLower = errorMessage.toLowerCase();
    const roughCode = trace?.code ?? (msgLower.includes('401') || msgLower.includes('403') ? 'recovery_auth'
      : msgLower.includes('recovery') ? 'recovery_dl'
      : msgLower.includes('efi') && msgLower.includes('valid') ? 'efi_val'
      : msgLower.includes('flash') || msgLower.includes('write') ? 'flash'
      : 'other');
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
    setGlobalError(JSON.stringify(payload));
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

  const invalidateGeneratedBuild = () => {
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
    setSelectedUsb(null);
    setDiskInfo(null);
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

    const guard = await window.electron.guardBuild(activeProfile);
    if (!guard.allowed && options?.surfaceError !== false) {
      const redirect = getBuildGuardRedirect(activeCompat);
      setErrorWithSuggestion(guard.reason ?? 'Build is blocked by the current firmware or compatibility state.', redirect);
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
  const setStep = (target: StepId) => {
    const result = evaluateStepTransition(target, {
      profile,
      compat,
      hasLiveHardwareContext,
      biosReady,
      buildReady,
      efiPath,
      biosConf,
      selectedUsb,
      compatibilityBlocked,
      validationBlocked,
      postBuildReady,
      localBuildGuard,
      localDeployGuard,
    });
    if (!result.ok) {
      debugWarn(`[guard] Blocked transition to "${target}": ${result.reason}`);
      if (result.redirect) _setStepRaw(result.redirect);
      return;
    }
    _setStepRaw(target);
  };

  const refreshBiosState = async (activeProfile: HardwareProfile, options?: { redirectIfBlocked?: boolean }) => {
    if (!hasLiveHardwareContext) {
      setBiosState(null);
      return null;
    }
    try {
      const nextState = await window.electron.getBiosState(activeProfile);
      setBiosState(nextState);
      if (options?.redirectIfBlocked && (!(nextState.readyToBuild && nextState.stage === 'complete')) && STEP_ORDER.indexOf(step) > STEP_ORDER.indexOf('bios')) {
        _setStepRaw('bios');
      }
      return nextState;
    } catch (e: any) {
      setErrorWithSuggestion(e.message || 'Failed to evaluate BIOS preparation state.', 'bios');
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
      return;
    }

    let cancelled = false;
    window.electron.getResourcePlan(profile, efiPath)
      .then((plan) => {
        if (!cancelled) setResourcePlan(plan);
      })
      .catch(() => {
        if (!cancelled) setResourcePlan(null);
      });

    return () => {
      cancelled = true;
    };
  }, [profile, efiPath]);

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
        try {
          window.electron.saveState({
            currentStep: 'recovery-download',
            profile,
            timestamp: now,
            efiPath,
            recoveryDownloadOffset: p.bytesDownloaded ?? recovOffset,
            recoveryDmgDest: persistedDmgDest ?? undefined,
            recoveryClDest: persistedClDest ?? undefined,
            recoveryTargetOS: profile?.targetOS || 'macOS Sequoia 15',
          });
        } catch {}
      }
    }
  }, [recovTask?.progress, recovClDest, recovDmgDest, recovOffset]); // eslint-disable-line react-hooks/exhaustive-deps

  // Drive kextResults / progress from the live kext task state
  useEffect(() => {
    const p = kextTask?.progress as { kind: string; kextName?: string; version?: string; index?: number; total?: number } | null | undefined;
    if (!p || p.kind !== 'kext-fetch') return;
    if (p.kextName) {
      setKextResults(prev => prev.find(k => k.name === p.kextName) ? prev : [...prev, { name: p.kextName!, version: p.version ?? '' }]);
    }
    if (p.index !== undefined && p.total) {
      setProgress(Math.round((p.index / p.total) * 100));
    }
  }, [kextTask?.progress]); // eslint-disable-line react-hooks/exhaustive-deps

  // Drive building progress from the live EFI build task state
  useEffect(() => {
    const p = efiTask?.progress as { kind: string; phase?: string; detail?: string } | null | undefined;
    if (!p) return;
    if (p.phase) setStatus(p.phase);
  }, [efiTask?.progress]);

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

  const filteredIssues = useMemo(() =>
    troubleshootingData.filter(it => {
      const matchCat = cat === 'All' || it.category === cat;
      const q = search.toLowerCase();
      const matchQ = !q || it.error.toLowerCase().includes(q) || it.fix.toLowerCase().includes(q) || it.category.toLowerCase().includes(q);
      return matchCat && matchQ;
    }), [search, cat]);

  const getStatus = (id: string): 'active' | 'complete' | 'pending' => {
    if (id === step || (id === 'scanning' && step === 'version-select')) return 'active';
    const ci = STEP_ORDER.indexOf(step), si = STEP_ORDER.indexOf(id as StepId);
    return si < ci ? 'complete' : 'pending';
  };

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

    const s = await window.electron.getPersistedState();
    let restoredCompatibilityBlocked = false;
    let restoredBiosReady = true;
    let restoredBuildReady = false;
    if (s && s.profile && s.currentStep && Date.now() - s.timestamp < 4 * 3600 * 1000) {
      const restore = restoreFlowDecision(s.profile, s.currentStep);
      restoredCompatibilityBlocked = isCompatibilityBlocked(restore.compatibility);
      const latestArtifact = s.profileArtifactDigest
        ? await window.electron.getLatestHardwareProfile().catch(() => null)
        : null;

      setPlanningProfileContext('saved_artifact');
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
      invalidateGeneratedBuild();
      restoredBiosReady = false;
      restoredBuildReady = false;
    }

    // Auto-resume an interrupted recovery download
    try {
      const resumeState = await window.electron.getDownloadResumeState();
      const resumeDecision = recoveryResumeDecision({
        compatibilityBlocked: restoredCompatibilityBlocked,
        biosReady: restoredBiosReady,
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

  const startScan = async () => {
    if (isScanningRef.current) return;
    isScanningRef.current = true;
    try {
      invalidateGeneratedBuild();
      setStep('scanning'); setProgress(20);
      const scanResult = await window.electron.scanHardware();
      const hw = scanResult.profile;
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
      
      setCompat(report);
      setProfile(hw); // Update with strategy
      setBiosConf(getBIOSSettings(hw));
      await refreshBiosState(hw);
      setProgress(100);
      setTimeout(() => setStep('version-select'), 700);
    } catch (e: any) {
      setErrorWithSuggestion(e.message || 'Hardware scan failed', 'scanning');
      setStep('landing');
    } finally { isScanningRef.current = false; }
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
    const result = await window.electron.applySupportedBiosChanges(profile, selectedChanges);
    setBiosState(result.state);
    return { message: result.message };
  };

  const verifyBiosAndContinue = async (selectedChanges: Record<string, BiosSettingSelection>) => {
    if (!profile) throw new Error('Hardware profile missing for BIOS verification.');
    const state = await window.electron.verifyManualBiosChanges(profile, selectedChanges);
    setBiosState(state);
    if (!state.readyToBuild || state.stage !== 'complete') {
      setErrorWithSuggestion(state.blockingIssues[0] ?? 'BIOS preparation is incomplete. Verify the required firmware settings before continuing.', 'bios');
      return false;
    }
    const guard = await ensureBuildGuard(profile, { surfaceError: true });
    if (!guard.allowed) {
      return false;
    }
    setStep('building');
    return true;
  };

  const restartToFirmwareWithSession = async (selectedChanges: Record<string, BiosSettingSelection>) => {
    if (!profile) return { supported: false, error: 'Hardware profile missing for BIOS reboot.' };
    const result = await window.electron.restartToFirmwareWithSession(profile, selectedChanges);
    setBiosState(result.state);
    window.electron.getBiosResumeState().then(setBiosResumeState).catch(() => {});
    if (!result.supported && result.error) {
      setErrorWithSuggestion(result.error, 'bios');
    }
    return { supported: result.supported, error: result.error };
  };

  const startDeploy = async () => {
    if (!profile || isDeployingRef.current) return;
    const liveBiosState = biosReady ? biosState : await refreshBiosState(profile, { redirectIfBlocked: true });
    if (!liveBiosState) {
      return;
    }
    const guard = await ensureBuildGuard(profile, { surfaceError: true });
    if (!guard.allowed) {
      return;
    }
    isDeployingRef.current = true;
    invalidateGeneratedBuild();
    setBuildPlan(null);
    setRecoveryDryRun(null);
    try {
      setStep('building'); setProgress(0);

      // Stage 0a: Prevention Layer — preflight environment check
      setStatus('Running pre-build environment checks…');
      setProgress(1);
      try {
        const { kexts: requiredKexts } = getRequiredResources(profile);
        setPreflightRunning(true);
        const report = await (window.electron as any).runPreflightChecks(requiredKexts);
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
      setProgress(3);

      // Stage 0b: Deterministic Layer — build dry-run simulation
      // Verifies every kext URL, OpenCore URL, and disk BEFORE real build
      setStatus('Simulating build — verifying all dependencies…');
      try {
        const { kexts: requiredKexts, ssdts: requiredSSDTs } = getRequiredResources(profile);
        const plan = await (window.electron as any).simulateBuild(requiredKexts, requiredSSDTs, profile.smbios);
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
      setProgress(5);

      // Stage 1: build EFI
      setStatus('Generating OpenCore configuration…');
      setProgress(10);
      await new Promise(r => setTimeout(r, 800));
      const built = await window.electron.buildEFI(profile);
      setEfiPath(built); setProgress(45);
      setStatus('Injecting ACPI SSDTs…');
      await new Promise(r => setTimeout(r, 600));
      setProgress(100);
      await new Promise(r => setTimeout(r, 800));

      // Stage 2: kexts — progress is driven by kextTask useEffect above
      setStep('kext-fetch'); setProgress(0);
      setKextResults([]);
      const { kexts } = getRequiredResources(profile);
      let fetchedKextResults: KextFetchResult[] = [];
      try {
        fetchedKextResults = await window.electron.fetchLatestKexts(built, kexts);
        setKextResults(fetchedKextResults);
        for (const k of fetchedKextResults.filter(k => k.version === 'offline')) {
          (window.electron as any).recordFailure(`kext_${k.name}`, 'Download failed').catch(() => {});
        }
      } catch (e) {
        fetchedKextResults = kexts.map(k => ({ name: k, version: 'offline', source: 'failed' }));
        setKextResults(fetchedKextResults);
        (window.electron as any).recordFailure('kext_batch', String((e as Error)?.message || 'Batch kext fetch failed')).catch(() => {});
      }
      setProgress(100);
      await new Promise(r => setTimeout(r, 800));

      // Stage 3: Build Integrity Check — existing configValidator + deterministic hard contract
      setStatus('Verifying EFI structure…');
      const validation = await window.electron.validateEfi(built, profile);
      setValidationResult(validation);
      if (validation.overall === 'blocked') {
        setBuildReady(false);
        setErrorWithSuggestion(describeValidationFailure(validation), 'building', {
          validationResult: validation,
          kextSources: buildKextSourceMap(fetchedKextResults),
        });
        setStep('report');
        return;
      }

      // Phase 4: Hard success contract — verify from disk, not flags
      try {
        const contract = await (window.electron as any).verifyEfiBuildSuccess(built, kexts);
        if (!contract.passed) {
          const failed = contract.checks.filter((c: any) => !c.passed);
          throw new Error(`EFI build contract failed: ${failed.map((c: any) => `${c.name}: ${c.detail}`).join('; ')}`);
        }
      } catch (contractErr: any) {
        if (contractErr.message?.startsWith('EFI build contract failed:')) throw contractErr;
        debugWarn('[deterministic] EFI contract check failed to execute:', contractErr);
      }

      setBuildReady(true); // BUILD IS NOW VERIFIED READY (by disk, not trust)

      // Generate EFI Intelligence Report + Community Knowledge
      try {
        const report = generateEfiReport(profile, compat, fetchedKextResults);
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
      setStep('recovery-download'); setRecovPct(0); setRecovError(null); setRecovOffset(0); setRecovDmgDest(null); setRecovClDest(null);
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
        await window.electron.downloadRecovery(built, profile.targetOS || 'macOS Sequoia 15');
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
        const recovContract = await (window.electron as any).verifyRecoverySuccess(built);
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

      // Stage 5: Method select
      setStep('method-select');
    } catch (e: any) {
      setBuildReady(false);
      setErrorWithSuggestion(e.message || 'Build failed. Please check the hardware compatibility.', 'building');
      setStep('report');
    } finally { isDeployingRef.current = false; }
  };

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

  const refreshUsbTargets = async () => {
    try {
      const devices = await window.electron.listUsbDevices();
      setUsbDevices(await enrichDrives(devices));
      setSelectedUsb(null);
      setEfiBackupPolicy(null);
      clearFlashConfirmationState();
    } catch (error: any) {
      setErrorWithSuggestion(error?.message || 'Failed to refresh removable drives. Reconnect the target USB and try again.', 'usb-select');
    }
  };

  const refreshPartitionTargets = async () => {
    try {
      const drives = await window.electron.getHardDrives();
      setUsbDevices(await enrichDrives(drives));
      setSelectedUsb(null);
      setEfiBackupPolicy(null);
      clearFlashConfirmationState();
    } catch (error: any) {
      setErrorWithSuggestion(error?.message || 'Failed to refresh disks. Retry or rescan before modifying a drive.', 'part-prep');
    }
  };

  const selectMethod = async (m: 'usb' | 'partition') => {
    if (!profile) return;
    if (!buildReady || !efiPath) {
      setErrorWithSuggestion('EFI validation failed — build integrity check failed or incomplete. Please go back and build the EFI first.');
      setStep('report');
      return;
    }

    const { guard, validation } = await ensureDeployGuard(profile, efiPath, {
      surfaceError: true,
      reasonSuffix: ' Select a build method only after the BIOS and EFI are still valid.',
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
      setSelectedUsb(null);
      clearFlashConfirmationState();
      if (m === 'usb') {
        const devs = await window.electron.listUsbDevices();
        const enriched = await enrichDrives(devs);
        setUsbDevices(enriched);
        setStep('usb-select');
      } else {
        const drives = await window.electron.getHardDrives();
        const enriched = await enrichDrives(drives);
        setUsbDevices(enriched);
        setStep('part-prep');
      }
    } catch (e: any) {
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
    clearFlashConfirmationState();
    const { guard, validation } = await ensureDeployGuard(profile, efiPath, {
      surfaceError: true,
      reasonSuffix: ' Flashing is blocked until the BIOS and EFI are still valid.',
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
      const info = await window.electron.getDiskInfo(selectedUsb);
      setDiskInfo(info);
      if (info.isSystemDisk) {
        setErrorWithSuggestion(`SYSTEM_DISK: ${selectedUsb} is your system/boot disk. Select a different USB drive.`);
        return;
      }
      if (info.partitionTable === 'mbr') {
        setShowDiskWarning(true);
        return;
      }
      if (info.partitionTable === 'unknown') {
        setShowUnknownPartitionWarning(true);
        return;
      }
    } catch { /* proceed — disk info is best-effort */ }
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
        return;
      }
    } catch (e) {
      debugWarn('[flash] Pre-flash EFI validation failed:', e);
      setValidationResult(null);
      setBuildReady(false);
      setErrorWithSuggestion('EFI validation failed — failed to verify build integrity before flashing.', 'report');
      setStep('report');
      return;
    } finally {
      setValidationRunning(false);
    }
    try {
      const prepared = await window.electron.prepareFlashConfirmation(
        selectedUsb,
        efiPath,
        diskInfo ? {
          devicePath: diskInfo.devicePath,
          sizeBytes: diskInfo.sizeBytes,
          model: diskInfo.model,
          vendor: diskInfo.vendor,
          serialNumber: diskInfo.serialNumber,
          transport: diskInfo.transport,
          removable: diskInfo.removable,
          partitionTable: diskInfo.partitionTable,
        } : undefined,
      );
      setDiskInfo(prepared.diskInfo);
      setEfiBackupPolicy(prepared.backupPolicy);
      setFlashConfirmationToken(prepared.token);
      setFlashConfirmationExpiresAt(prepared.expiresAt);
    } catch (e: any) {
      setErrorWithSuggestion(e.message || 'Flash confirmation could not be prepared. Re-select the drive and try again.', 'usb-select');
      return;
    }
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
        const info = await window.electron.getDiskInfo(selectedUsb);
        if (!info) throw new Error('Device lost');
        setDiskInfo(info);
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

  // ── Sidebar helper ──────────────────────────────────────────

  const SidebarItem = ({ id, label, icon: Icon }: { id: string; label: string; icon: any }) => {
    const s = getStatus(id);
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
    { label: 'Generate config.plist', sublabel: profile ? `${profile.generation} · ${profile.smbios} · ${profile.kexts.length} kexts` : '550+ options for your hardware', done: progress >= 45, active: progress >= 5 && progress < 45 },
    { label: 'Inject ACPI tables', sublabel: profile ? `${profile.ssdts.length} SSDTs — power management, EC, USB` : 'Platform-specific ACPI patches', done: progress >= 100, active: progress >= 45 && progress < 100 },
  ];
  const kextStages = kextResults.map((k: any) => {
    const src = k.source === 'embedded' ? 'embedded fallback' : k.source === 'failed' ? 'FAILED' : 'GitHub';
    return {
      label: k.name,
      sublabel: k.version === 'offline' ? `FAILED — no source available` : `v${k.version} — ${src}`,
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

  // ── Render ──────────────────────────────────────────────────

  return (
    <div className="min-h-screen bg-[#050505] text-[#EDEDED] font-sans flex items-center justify-center p-4 overflow-hidden relative">
      <div className="bg-grain" />
      {/* Background glows */}
      <div className="absolute inset-0 z-0 pointer-events-none overflow-hidden">
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
            className="z-10 flex flex-col items-center text-center max-w-2xl px-6"
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
              className="w-40 h-40 mb-10 flex items-center justify-center rounded-[2.5rem] bg-white/5 border border-white/10 backdrop-blur-xl shadow-2xl relative"
            >
              <div className="absolute inset-0 rounded-[2.5rem] bg-blue-500 blur-3xl opacity-20 pointer-events-none" />
              <BrandIcon className="w-24 h-24 text-white relative z-10" />
            </motion.div>
            
            <motion.div
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.2 }}
            >
              <h1 className="text-8xl font-black tracking-tighter mb-6 bg-clip-text text-transparent bg-gradient-to-b from-white to-white/40 leading-[0.9]">
                Install <br/> macOS
              </h1>
            </motion.div>

            <motion.p 
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.3 }}
              className="text-xl text-white/50 mb-12 leading-relaxed font-medium max-w-lg"
            >
              Scan your hardware. Build an OpenCore EFI. Write a bootable installer. Done.
            </motion.p>

            <motion.div 
              initial={{ opacity: 0, y: 20 }} 
              animate={{ opacity: 1, y: 0 }} 
              transition={{ delay: 0.4 }} 
              className="flex flex-col items-center gap-4"
            >
              <div className="flex gap-4">
                <button
                  onClick={() => {
                    if (adminPrivileges === false && platform !== 'darwin') {
                      setErrorWithSuggestion(
                        platform === 'win32'
                          ? 'Please run the app as Administrator to continue. Right-click the .exe and select "Run as administrator".'
                          : 'Please run the app with sudo to continue: sudo ./macOS-One-Click'
                      );
                      return;
                    }
                    setStep('welcome');
                  }}
                  className="group px-10 py-5 bg-white text-black rounded-2xl font-bold text-lg hover:scale-105 active:scale-95 transition-all shadow-[0_20px_50px_rgba(255,255,255,0.12)] flex items-center gap-3 cursor-pointer relative overflow-hidden">
                  <div className="absolute inset-0 bg-gradient-to-r from-transparent via-white/20 to-transparent translate-x-[-100%] group-hover:translate-x-[100%] transition-transform duration-700 ease-in-out" />
                  Begin Installation
                </button>
                <button onClick={() => setStep('troubleshooting')} className="px-10 py-5 bg-white/5 border border-white/10 text-white rounded-2xl font-bold text-lg hover:bg-white/10 hover:scale-105 active:scale-95 transition-all flex items-center gap-3 cursor-pointer backdrop-blur-md">
                  <HelpCircle className="w-5 h-5 text-white/40" /> Troubleshoot
                </button>
              </div>
            </motion.div>

            <motion.p 
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ delay: 1 }}
              className="mt-12 text-[10px] text-white/20 font-mono uppercase tracking-[0.4em]"
            >
              Frontier Edition
            </motion.p>
          </motion.div>
        )}

        {/* ── WIZARD SHELL ── */}
        {step !== 'landing' && (
          <motion.div key="shell" initial={{ opacity: 0, y: 20 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
            className="z-10 w-full max-w-5xl h-[660px] flex glass-card overflow-hidden">

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
                <span className="font-bold text-sm tracking-wide text-white">Frontier Edition</span>
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
                </div>
              )}
            </div>

            {/* Content */}
            <div className="flex-1 overflow-y-auto custom-scrollbar p-10 relative">
              <div className="absolute top-6 right-8 opacity-10 pointer-events-none flex items-center gap-2">
                <BrandIcon className="w-4 h-4 text-white" />
                <span className="text-[10px] font-bold uppercase tracking-widest">macOS Frontier</span>
              </div>
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
                    <PrecheckStep onContinue={startScan} />
                  </motion.div>
                )}

                {/* SCANNING */}
                {step === 'scanning' && (
                  <motion.div key="scan" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ScanStep progress={progress} profile={profile} />
                  </motion.div>
                )}


                {/* VERSION SELECT */}
                {step === 'version-select' && compat && (
                  <motion.div key="ver" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION}>
                    <VersionStep report={compat} matrix={compatibilityMatrix ?? buildCompatibilityMatrix(profile!)} selectedVersion={profile?.targetOS ?? compat.recommendedVersion} onSelect={v => {
                      if (!profile) return;
                      const selection = targetSelectionDecision(profile, v);
                      setProfile(selection.profile);
                      setCompat(selection.compatibility);
                      setBiosConf(selection.biosConfig);
                      invalidateGeneratedBuild();
                      refreshBiosState(selection.profile).catch(() => {});
                      setStep(selection.nextStep);
                    }} />
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
                      resourcePlan={resourcePlan}
                      planningOnly={!hasLiveHardwareContext}
                      planningProfileContext={planningProfileContext}
                      simulationResult={safeSimulationResult}
                      simulationRunning={simulationRunning}
                      onSaveProfile={saveCurrentPlanningProfile}
                      onExportProfile={exportCurrentPlanningProfile}
                      onImportProfile={importPlanningProfile}
                      onRunSimulation={runSafeSimulationPreview}
                      onRunLiveScan={startScan}
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
                      onVerifyAndContinue={verifyBiosAndContinue}
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
                      stages={buildStages}
                      onBegin={startDeploy}
                      briefing={{
                        heading: 'Getting ready to prepare your installer',
                        bullets: [
                          'An OpenCore EFI folder will be generated and configured for your specific hardware.',
                          'Kext drivers (hardware compatibility files) will be downloaded from GitHub.',
                          'A macOS recovery image will be downloaded from Apple\'s servers (~500 MB).',
                          'You will then select a USB drive and write the installer to it.',
                        ],
                        estimatedMinutes: 15,
                        interruptionWarning: 'Once begun, the download steps can be paused and resumed later. The USB write step cannot be interrupted once started — do not remove the drive during that phase.',
                      }}
                    />
                  </motion.div>
                )}

                {/* KEXT FETCH */}
                {step === 'kext-fetch' && (
                  <motion.div key="kext" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <ProgressStep title="Downloading Kexts" subtitle="Fetching the latest stable versions from GitHub." icon={Package} progress={progress} statusText={kextResults.length ? `${kextResults.length} downloaded` : 'Querying GitHub releases…'} stages={kextStages.length ? kextStages : [{ label: 'Connecting to GitHub…', sublabel: '', done: false, active: true }]} />
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
                          <button disabled={isRetryingRecovRef.current} onClick={async () => { if (isRetryingRecovRef.current) return; isRetryingRecovRef.current = true; setRecovError(null); setRecovPct(0); try { await window.electron.downloadRecovery(efiPath!, profile?.targetOS || 'macOS Sequoia 15'); try { const c = await (window.electron as any).verifyRecoverySuccess(efiPath!); if (!c.passed) { const failed = c.checks.filter((x: any) => !x.passed); setRecovError(`Recovery verification failed: ${failed.map((x: any) => `${x.name}: ${x.detail}`).join('; ')}`); setErrorWithSuggestion('Recovery download completed but verification failed. File may be incomplete.', 'recovery-download'); return; } } catch {} setStep('method-select'); } catch (e: any) { const msg = e.message || 'Retry failed'; const code = msg.includes('401') || msg.includes('403') || msg.includes('rejected') ? 'recovery_auth' : 'recovery_dl'; (window.electron as any).recordFailure(code, msg).catch(() => {}); setRecovError(msg); setErrorWithSuggestion(msg, 'recovery-download'); } finally { isRetryingRecovRef.current = false; } }}
                            className="w-full text-left p-4 bg-white/4 border border-white/8 rounded-xl hover:bg-white/8 transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-4">
                            <RefreshCcw className="w-5 h-5 text-white/60 shrink-0" />
                            <div>
                              <p className="text-sm font-bold text-white">Retry once more</p>
                              <p className="text-xs text-[#666]">Try Apple's servers again with a fresh request</p>
                            </div>
                          </button>

                          {/* Option 2: Use cached recovery */}
                          {cachedRecovInfo && !cachedRecovInfo.isPartial && (
                            <button onClick={async () => { setRecovError(null); setRecovPct(0); try { await window.electron.downloadRecovery(efiPath!, profile?.targetOS || 'macOS Sequoia 15'); try { const c = await (window.electron as any).verifyRecoverySuccess(efiPath!); if (!c.passed) { const failed = c.checks.filter((x: any) => !x.passed); setRecovError(`Cached recovery verification failed: ${failed.map((x: any) => `${x.name}: ${x.detail}`).join('; ')}`); return; } } catch {} setStep('method-select'); } catch (e: any) { setRecovError(e.message || 'Cache retrieval failed'); } }}
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
                          <button onClick={() => setStep('method-select')}
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
                      <ProgressStep native title="Preparing Installation" subtitle={`Setting up ${profile?.targetOS || 'the OS'} for your hardware.`} icon={Package} progress={recovPct} statusText={recovStatus || 'Connecting…'} stages={recovStages} />
                    )}
                  </motion.div>
                )}

                {/* METHOD SELECT */}
                {step === 'method-select' && (
                  <motion.div key="meth" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <MethodStep onSelect={selectMethod} onBack={() => setStep('recovery-download')} />
                  </motion.div>
                )}

                {/* PARTITION PREP */}
                {step === 'part-prep' && (
                  <motion.div key="part" initial={stepEnter} animate={stepActive} exit={stepExit} transition={STEP_TRANSITION} className="h-full">
                    <UsbStep devices={usbDevices} selected={selectedUsb} onSelect={v => { setSelectedUsb(v); setDiskInfo(null); setEfiBackupPolicy(null); clearFlashConfirmationState(); }} onRefresh={refreshPartitionTargets} requireFullSize={true} />
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
                      <span className="text-[11px] text-white/30">USB flashing is in beta — validate with the real-device checklist before broad deployment</span>
                    </div>
                    <UsbStep
                      devices={usbDevices}
                      selected={selectedUsb}
                      backupPolicy={efiBackupPolicy}
                      onSelect={v => { setSelectedUsb(v); setDiskInfo(null); setEfiBackupPolicy(null); clearFlashConfirmationState(); }}
                      onDeselect={() => { setSelectedUsb(null); setDiskInfo(null); setEfiBackupPolicy(null); clearFlashConfirmationState(); }}
                      onRefresh={refreshUsbTargets}
                      onConfirmDrive={initiateFlash}
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
                        {profile.generation} {profile.cpu.split(/\s+/).find(w => /i[3579]|ryzen|xeon|threadripper/i.test(w)) ?? ''} · {efiReport.hardware.items.find(i => i.label === 'Graphics')?.value.split(/\s+/).slice(0, 3).join(' ') ?? profile.gpu.split(/\s+/).slice(0, 3).join(' ')} · {profile.targetOS.replace('macOS ', '')} target · {efiReport.confidenceScore >= 85 ? 'Excellent' : efiReport.confidenceScore >= 70 ? 'High' : efiReport.confidenceScore >= 50 ? 'Moderate' : efiReport.confidenceScore >= 25 ? 'Low' : 'Critical'} confidence build
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

              {/* Back navigation — available on all non-auto, non-destructive steps */}
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
                  <div className="absolute bottom-10 left-10">
                    <button onClick={() => setStep(target)} className="flex items-center gap-1.5 text-sm text-[#555] hover:text-white transition-colors cursor-pointer">
                      <ChevronLeft className="w-4 h-4" /> Back
                    </button>
                  </div>
                );
              })()}
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
                  <span className="text-white font-semibold">macOS One-Click</span> is a community project and isn't officially supported by Apple.
                </p>
                <p>
                  We don't provide any warranty—you're doing this at your own risk. Make sure you know what you're getting into, grab a coffee, and let's build a Hackintosh.
                </p>
              </div>
              <div className="flex gap-3 mt-6">
                <button
                  onClick={() => { setShowDisclaimer(false); }}
                  className="flex-1 py-3 rounded-xl bg-white/5 border border-white/10 text-white/50 text-sm font-medium hover:bg-white/8 transition-all cursor-pointer"
                >
                  Nevermind
                </button>
                <button
                  onClick={() => { setDisclaimerAccepted(true); setShowDisclaimer(false); setShowRecoveryPrompt(true); }}
                  className="flex-1 py-3 rounded-xl bg-white text-black text-sm font-bold hover:bg-white/90 transition-all cursor-pointer"
                >
                  Sounds Good — Let's Go
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
                  Before proceeding, we <span className="text-white font-semibold">strongly recommend</span> creating a system recovery point on your current OS. This will allow you to undo any changes if something goes wrong.
                </p>
                <div className="bg-white/4 border border-white/8 rounded-2xl p-4 space-y-2 text-xs font-mono">
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mb-1">Windows</p>
                  <p className="text-white/70">Search → "Create a restore point" → System Protection → Create</p>
                  <p className="text-white/40 text-[10px] uppercase tracking-widest mt-3 mb-1">Linux</p>
                  <p className="text-white/70">Use Timeshift or your distro's snapshot tool before continuing.</p>
                </div>
                <p className="text-amber-400/80 text-xs">
                  ⚠️ macOS One-Click writes to your EFI partition and may modify boot settings. A restore point is your safety net.
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
                  Done — Begin Installation →
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
          const FLASH_CHECK_IDS = ['no-data', 'correct-drive', 'bios-reviewed', 'irreversible'];
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
                    <h2 className="text-xl font-bold text-white">Last chance — are you absolutely sure?</h2>
                    <p className="text-[10px] text-red-400/70 font-mono uppercase tracking-widest mt-0.5">This action cannot be undone</p>
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
                    <span className="font-bold text-red-300">ALL DATA ON THIS DRIVE WILL BE PERMANENTLY AND IRREVERSIBLY ERASED.</span>{' '}
                    This cannot be undone.
                  </p>
                </div>

                {/* Identifier instability reminder (Gap 3) */}
                <div className="mb-5 flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/6 border border-amber-500/15">
                  <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
                  <p className="text-xs text-amber-300/70 leading-relaxed">
                    <span className="font-bold text-amber-300">Reminder:</span>{' '}
                    If you disconnected and reconnected this drive since selecting it, its identifier may have changed.
                    Verify the identifier shown above matches the physical drive you intend to erase.
                  </p>
                </div>

                {/* EFI validation summary */}
                <div className="mb-5">
                  <ValidationSummary result={validationResult} isRunning={validationRunning} />
                </div>

                {/* 4-checkbox final checklist */}
                <div className="mb-5 space-y-2">
                  <div className="text-[10px] font-bold uppercase tracking-widest text-white/25 mb-2">Confirm before continuing</div>
                  {([
                    { id: 'no-data',      label: 'This USB drive contains no important data' },
                    { id: 'correct-drive', label: 'I have confirmed this is the correct drive' },
                    { id: 'bios-reviewed', label: 'I have reviewed the BIOS settings for my target PC' },
                    { id: 'irreversible',  label: 'I understand this action cannot be undone' },
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
                    Type the drive identifier to confirm:{' '}
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
                      Confirmation expired. Close this dialog and reopen it to refresh the destructive-write authorization.
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
                    Erase and continue
                  </button>
                </div>
              </motion.div>
            </motion.div>
          );
        })()}
      </AnimatePresence>

      {/* ── GLOBAL ERROR TOAST ───────────────────────────────────── */}
      <AnimatePresence>
        {globalError && (() => {
          let err: any = null;
          try {
            err = typeof globalError === 'string' && globalError.startsWith('{') ? JSON.parse(globalError) : { message: globalError };
          } catch {
            err = { message: globalError };
          }

          const isHardware = err.category === 'hardware_error';
          const isEnv = err.category === 'environment_error';
          const isCritical = err.severity === 'critical';
          const alts: Array<{ text: string; confidence: string; group: string; recommended?: boolean; reason?: string; expectedOutcome?: string; risk?: string }> = err.alternatives ?? [];
          const recommendedAlt = alts.find((a: any) => a.recommended);
          const fixNow = alts.filter((a: any) => a.group === 'fix_now' && !a.recommended);
          const tryAlt = alts.filter((a: any) => a.group === 'try_alternative' && !a.recommended);
          const learnMore = alts.filter((a: any) => a.group === 'learn_more' && !a.recommended);

          return (
            <motion.div
              key="global-error"
              initial={{ opacity: 0, y: 40, scale: 0.95 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: 40, scale: 0.95 }}
              className={`fixed bottom-6 left-1/2 -translate-x-1/2 z-50 flex flex-col gap-3 p-1 bg-[#0d0d0f]/90 border ${isCritical ? 'border-rose-500/40' : 'border-rose-500/30'} rounded-3xl shadow-2xl backdrop-blur-xl max-w-lg w-[90vw]`}
            >
              <div className="flex items-start gap-4 px-5 py-4">
                <div className={`w-10 h-10 rounded-2xl flex items-center justify-center flex-shrink-0 ${
                  isHardware ? 'bg-amber-500/10 text-amber-400' : isEnv ? 'bg-blue-500/10 text-blue-400' : 'bg-rose-500/10 text-rose-400'
                }`}>
                  <AlertTriangle className="w-5 h-5" />
                </div>
                <div className="flex-1 space-y-1.5">
                  <h3 className="text-sm font-bold text-white leading-tight">{err?.message ?? 'An error occurred'}</h3>
                  {err?.explanation && <p className="text-[11px] text-white/50 leading-relaxed">{err.explanation}</p>}
                  
                  {/* Recovery Hardening Fallback UI */}
                  {err.message && (typeof err.message === 'string') && (err.message.includes('Apple server rejected') || err.message.includes('RECOVERY_AUTH') || err.message.includes('401') || err.message.includes('403')) && (
                    <div className="mt-4 flex flex-col gap-2 pt-3 border-t border-white/5">
                      <p className="text-[10px] text-white/30 italic leading-snug">Apple servers rejected the request multiple times. This is not caused by your system.</p>
                      <div className="flex flex-wrap gap-2 pt-1">
                        <button
                          onClick={() => { setStep('method-select'); setGlobalError(null); }}
                          className="px-3 py-1.5 bg-blue-600/20 border border-blue-500/30 rounded-lg text-[10px] font-bold text-blue-300 hover:bg-blue-600/30 transition-all cursor-pointer"
                        >
                          Continue with EFI only (skip recovery)
                        </button>
                        <button
                          onClick={() => { setStep('version-select'); setGlobalError(null); }}
                          className="px-3 py-1.5 bg-white/5 border border-white/10 rounded-lg text-[10px] font-bold text-white/60 hover:bg-white/10 transition-all cursor-pointer"
                        >
                          Try a different macOS version
                        </button>
                      </div>
                    </div>
                  )}

                  {err?.decisionSummary && (
                    <p className="mt-1.5 text-[11px] font-semibold text-emerald-300/80 leading-relaxed">{err.decisionSummary}</p>
                  )}
                  {(err?.validationComponent || err?.validationPath) && (
                    <div className="mt-2 p-2.5 rounded-xl bg-white/4 border border-white/8 space-y-1">
                      {err?.validationCode && (
                        <p className="text-[9px] font-mono text-white/40">Code: {err.validationCode}</p>
                      )}
                      {err?.validationComponent && (
                        <p className="text-[10px] text-white/65">Component: {err.validationComponent}</p>
                      )}
                      {err?.validationPath && (
                        <p className="text-[10px] text-white/65">Path: {err.validationPath}</p>
                      )}
                      {err?.validationSource && (
                        <p className="text-[10px] text-white/45">Source: {err.validationSource}</p>
                      )}
                      {err?.validationDetail && (
                        <p className="text-[10px] text-white/45">{err.validationDetail}</p>
                      )}
                    </div>
                  )}
                  {err?.suggestion && err?.suggestionRecommended && (
                    <div className="mt-2 p-2.5 rounded-xl bg-emerald-500/8 border border-emerald-500/15 space-y-1">
                      <div className="flex items-start gap-1.5 text-[10px] font-bold text-emerald-400">
                        <CheckCircle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                        <span className="leading-relaxed">{err.suggestion}</span>
                      </div>
                      {err?.suggestionReason && (
                        <p className="text-[9px] text-white/30 leading-relaxed ml-[20px]">{err.suggestionReason}</p>
                      )}
                      {err?.suggestionOutcome && (
                        <p className="text-[9px] text-emerald-400/40 leading-relaxed ml-[20px]">Expected: {err.suggestionOutcome}</p>
                      )}
                      {err?.suggestionRisk && (
                        <p className="text-[9px] text-amber-400/50 leading-relaxed ml-[20px]">⚠ {err.suggestionRisk}</p>
                      )}
                    </div>
                  )}
                  {err?.suggestion && !err?.suggestionRecommended && (
                    <div className="mt-2 space-y-1">
                      <div className="flex items-start gap-1.5 text-[10px] font-medium text-white/50">
                        <span className="text-white/20 mt-0.5 shrink-0">•</span>
                        <span className="leading-relaxed">{err.suggestion}</span>
                      </div>
                      {err?.suggestionReason && (
                        <p className="text-[9px] text-white/25 leading-relaxed ml-[14px]">{err.suggestionReason}</p>
                      )}
                    </div>
                  )}
                  {recommendedAlt && (
                    <div className="mt-2 p-2.5 rounded-xl bg-emerald-500/8 border border-emerald-500/15 space-y-1">
                      <div className="flex items-start gap-1.5 text-[10px] font-bold text-emerald-400">
                        <CheckCircle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                        <span className="leading-relaxed">{recommendedAlt.text}</span>
                      </div>
                      {recommendedAlt.reason && (
                        <p className="text-[9px] text-white/30 leading-relaxed ml-[20px]">{recommendedAlt.reason}</p>
                      )}
                      {recommendedAlt.expectedOutcome && (
                        <p className="text-[9px] text-emerald-400/40 leading-relaxed ml-[20px]">Expected: {recommendedAlt.expectedOutcome}</p>
                      )}
                      {recommendedAlt.risk && (
                        <p className="text-[9px] text-amber-400/50 leading-relaxed ml-[20px]">⚠ {recommendedAlt.risk}</p>
                      )}
                    </div>
                  )}
                  {fixNow.length > 0 && (
                    <div className="mt-1.5 space-y-1.5">
                      {fixNow.map((a: any, i: number) => (
                        <div key={i} className="space-y-0.5">
                          <div className="flex items-start gap-1.5 text-[10px] text-white/40">
                            <span className="text-white/20 mt-0.5 shrink-0">•</span>
                            <span className="leading-relaxed">{a?.text}</span>
                          </div>
                          {a?.reason && <p className="text-[9px] text-white/20 leading-relaxed ml-[14px]">{a.reason}</p>}
                        </div>
                      ))}
                    </div>
                  )}
                  {tryAlt.length > 0 && (
                    <div className="mt-1.5 space-y-1.5">
                      <span className="text-[9px] font-bold uppercase tracking-widest text-white/20">Alternatives</span>
                      {tryAlt.map((a: any, i: number) => (
                        <div key={i} className="space-y-0.5">
                          <div className="flex items-start gap-1.5 text-[10px] text-blue-400/70">
                            <span className="text-blue-400/30 mt-0.5 shrink-0">→</span>
                            <span className="leading-relaxed">{a?.text}</span>
                          </div>
                          {a?.reason && <p className="text-[9px] text-blue-400/30 leading-relaxed ml-[14px]">{a.reason}</p>}
                          {a?.expectedOutcome && <p className="text-[9px] text-white/20 leading-relaxed ml-[14px]">→ {a.expectedOutcome}</p>}
                          {a?.risk && <p className="text-[9px] text-amber-400/40 leading-relaxed ml-[14px]">⚠ {a.risk}</p>}
                        </div>
                      ))}
                    </div>
                  )}
                  {learnMore.length > 0 && (
                    <div className="mt-1 space-y-0.5">
                      {learnMore.map((a: any, i: number) => (
                        <p key={i} className="text-[9px] text-white/25 leading-relaxed">{a?.text}</p>
                      ))}
                    </div>
                  )}
                  {err?.contextNote && (
                    <p className="mt-1.5 text-[10px] text-amber-400/60 leading-relaxed">{err.contextNote}</p>
                  )}
                </div>
                <button onClick={() => setGlobalError(null)} className="text-white/20 hover:text-white/50 transition-colors cursor-pointer p-1">
                  <X className="w-4 h-4" />
                </button>
              </div>

              <div className="flex items-center justify-between gap-3 px-5 pb-4">
                <div className="flex-1">
                  <CopyDiagnosticsButton 
                    extraContext={typeof globalError === 'string' ? globalError : JSON.stringify(globalError)} 
                    className="w-full justify-center py-2.5 bg-white/5 border-white/10 text-[10px]" 
                  />
                </div>
                <button
                  onClick={async () => {
                    try {
                      const res = await window.electron.reportIssue();
                      // Always copy body to clipboard — the URL only contains the title
                      try { await navigator.clipboard.writeText(res.body); } catch {}
                      if (res.success) {
                        setGlobalError('Issue report opened in your browser. The diagnostic details have been copied to your clipboard — paste them into the issue body.');
                      } else {
                        setGlobalError(`Could not open the browser automatically. The issue text has been copied — paste it manually at: ${res.baseUrl}`);
                      }
                    } catch {
                      setGlobalError('Could not generate the issue report. Please file manually at: https://github.com/redpersongpt/macOS-One-Click/issues/new');
                    }
                  }}
                  className="flex-1 py-2.5 rounded-xl bg-rose-500/10 border border-rose-500/20 text-rose-300 text-[10px] font-bold hover:bg-rose-500/20 transition-all cursor-pointer whitespace-nowrap text-center"
                >
                  Send Report
                </button>
              </div>
            </motion.div>
          );
        })()}
      </AnimatePresence>
      {/* ── DEBUG OVERLAY ─────────────────────────────────────────── */}
      <AnimatePresence>
        {debugOpen && (
          <DebugOverlay
            appVersion="2.2.2"
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
