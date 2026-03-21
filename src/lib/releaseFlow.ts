import {
  checkCompatibility,
  type CompatibilityReport,
} from '../../electron/compatibility.js';
import { getBIOSSettings, type BIOSConfig, type HardwareProfile } from '../../electron/configGenerator.js';
import type { ValidationResult } from '../../electron/configValidator.js';

export interface RestoreFlowDecision {
  profile: HardwareProfile;
  compatibility: CompatibilityReport;
  biosConfig: BIOSConfig;
  restoredStep: string;
  canReuseExistingEfi: boolean;
  canResumeRecovery: boolean;
}

export function isCompatibilityBlocked(report: CompatibilityReport | null | undefined): boolean {
  return !report || !report.isCompatible || report.errors.length > 0;
}

export function restoreFlowDecision(
  profile: HardwareProfile,
  requestedStep: string,
): RestoreFlowDecision {
  const compatibility = checkCompatibility(profile);
  const nextProfile: HardwareProfile = { ...profile, strategy: compatibility.strategy };
  const blocked = isCompatibilityBlocked(compatibility);

  return {
    profile: nextProfile,
    compatibility,
    biosConfig: getBIOSSettings(nextProfile),
    restoredStep: blocked ? 'report' : requestedStep,
    canReuseExistingEfi: !blocked,
    canResumeRecovery: !blocked,
  };
}

export interface TargetSelectionDecision {
  profile: HardwareProfile;
  compatibility: CompatibilityReport;
  biosConfig: BIOSConfig;
  nextStep: 'report';
  resetExistingBuild: true;
}

export function targetSelectionDecision(
  profile: HardwareProfile,
  targetOS: string,
): TargetSelectionDecision {
  const nextProfile: HardwareProfile = { ...profile, targetOS };
  const compatibility = checkCompatibility(nextProfile);
  nextProfile.strategy = compatibility.strategy;

  return {
    profile: nextProfile,
    compatibility,
    biosConfig: getBIOSSettings(nextProfile),
    nextStep: 'report',
    resetExistingBuild: true,
  };
}

export function isValidationBlockingDeployment(result: ValidationResult | null | undefined): boolean {
  return result?.overall === 'blocked';
}

export interface RecoveryResumeDecisionInput {
  compatibilityBlocked: boolean;
  efiReady: boolean;
}

export interface RecoveryResumeDecision {
  canResume: boolean;
  message: string | null;
  redirect: 'report' | 'bios' | null;
}

export function recoveryResumeDecision(input: RecoveryResumeDecisionInput): RecoveryResumeDecision {
  if (input.compatibilityBlocked) {
    return {
      canResume: false,
      message: 'Skipped recovery resume because the restored hardware profile is now blocked. Fix compatibility before resuming downloads.',
      redirect: 'report',
    };
  }

  if (!input.efiReady) {
    return {
      canResume: false,
      message: 'Skipped recovery resume because the saved EFI no longer passes validation. Rebuild the EFI before resuming downloads.',
      redirect: 'report',
    };
  }

  return {
    canResume: true,
    message: null,
    redirect: null,
  };
}
