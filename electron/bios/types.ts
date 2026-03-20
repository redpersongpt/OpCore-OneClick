import type { HardwareProfile, BIOSConfig } from '../configGenerator.js';
import type { FirmwareInfo } from '../firmwarePreflight.js';

export type BiosVendor = 'Generic' | 'HP' | 'Dell' | 'Lenovo';
export type BiosBackendId = 'generic' | 'hp' | 'dell' | 'lenovo';
export type BiosSupportLevel = 'manual' | 'assisted' | 'managed';
export type BiosApplyMode = 'manual' | 'assisted' | 'skipped';

// Extended session stage — matches BiosFlowState in stateMachine.ts
export type BiosSessionStage =
  | 'idle'
  | 'planned'
  | 'auto_applying'
  | 'ready_for_reboot'
  | 'rebooting_to_firmware'
  | 'awaiting_return'
  | 'resumed_from_firmware'
  | 'verifying'
  | 'partially_verified'
  | 'complete'
  | 'blocked'
  | 'unsupported_host';

export type BiosVerificationStatus = 'verified' | 'unverified' | 'unknown';
export type BiosRiskLevel = 'low' | 'medium' | 'high';
export type BiosDetectionConfidence = 'high' | 'medium' | 'low';

export type BiosSettingId =
  | 'uefi-mode'
  | 'secure-boot'
  | 'csm'
  | 'sata-ahci'
  | 'vt-d'
  | 'svm'
  | 'above4g'
  | 'xhci-handoff'
  | 'cfg-lock'
  | 'fast-boot'
  | 'intel-sgx'
  | 'platform-trust';

export interface BiosSettingSelection {
  approved: boolean;
  applyMode: BiosApplyMode;
}

export interface BiosSessionState {
  sessionId: string;
  hardwareFingerprint: string;
  selectedChanges: Record<BiosSettingId, BiosSettingSelection>;
  stage: BiosSessionStage;
  vendor: BiosVendor;
  rebootRequested: boolean;
  timestamp: number;
}

export interface BiosSettingPlan {
  id: BiosSettingId;
  name: string;
  description: string;
  plainTitle?: string;
  biosLocation?: string;
  jargonDef?: string;
  currentStatus: string;
  currentValue: string | null;
  recommendedValue: string;
  confidence: BiosDetectionConfidence;
  detectionMethod: string;
  riskLevel: BiosRiskLevel;
  supportLevel: BiosSupportLevel;
  allowedApplyModes: BiosApplyMode[];
  applyMode: BiosApplyMode;
  verificationStatus: BiosVerificationStatus;
  verificationDetail: string;
  required: boolean;
}

export interface BiosOrchestratorContext {
  profile: HardwareProfile;
  biosConfig: BIOSConfig;
  firmwareInfo: FirmwareInfo | null;
  platform: NodeJS.Platform;
  safeMode: boolean;
}

export interface BiosBackend {
  id: BiosBackendId;
  vendor: BiosVendor;
  label: string;
  vendorMatchers: RegExp[];
  getSupportLevel(settingId: BiosSettingId, ctx: BiosOrchestratorContext): BiosSupportLevel;
  rebootSupported(platform: NodeJS.Platform): boolean;
}

export interface BiosVerificationRow {
  id: BiosSettingId;
  status: BiosVerificationStatus;
  detail: string;
  detectionMethod: string;
  confidence: BiosDetectionConfidence;
}

export interface BiosVerificationResult {
  rows: Record<BiosSettingId, BiosVerificationRow>;
  readyToBuild: boolean;
  blockingIssues: string[];
}

export interface BiosOrchestratorState {
  vendor: BiosVendor;
  backendId: BiosBackendId;
  backendLabel: string;
  supportLevel: BiosSupportLevel;
  safeMode: boolean;
  rebootSupported: boolean;
  stage: BiosSessionStage;
  hardwareFingerprint: string;
  settings: BiosSettingPlan[];
  requiredCompletionCount: number;
  completedRequiredCount: number;
  readyToBuild: boolean;
  blockingIssues: string[];
  session: BiosSessionState | null;
  summary: string;
}

// ── IPC Contract Types ───────────────────────────────────────────────────────
// These define the exact request/response shapes for BIOS-related IPC.

export interface BiosGetStateResponse extends BiosOrchestratorState {}

export interface BiosApplyResponse {
  state: BiosOrchestratorState;
  appliedCount: number;
  message: string;
}

export interface BiosVerifyResponse extends BiosOrchestratorState {}

export interface BiosRestartResponse {
  supported: boolean;
  error?: string;
  state: BiosOrchestratorState;
}

export interface BiosResumeStateResponse {
  hasSession: boolean;
  stage: BiosSessionStage | null;
  fingerprint: string | null;
  stale: boolean;
  message: string;
}

export interface FirmwareRestartCapability {
  supported: boolean;
  method: 'uefi_firmware_command' | 'vendor_tool' | 'none';
  requiresAdmin: boolean;
  platform: string;
}

// ── Structured app error (shared) ────────────────────────────────────────────

export interface StructuredAppError {
  code: string;
  message: string;
  explanation: string;
  category: 'bios_error' | 'validation_error' | 'build_error' | 'deploy_error' | 'environment_error' | 'hardware_error' | 'unknown';
  severity: 'info' | 'warning' | 'critical';
  recoverable: boolean;
}
