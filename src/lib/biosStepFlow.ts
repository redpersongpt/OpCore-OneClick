import type { BiosOrchestratorState, BiosSettingSelection } from '../../electron/bios/types.js';
import type { HardwareProfile } from '../../electron/configGenerator.js';
import type { FailureRecoveryPayload } from './failureRecovery.js';

export type BiosRecoveryCode =
  | 'bios_recheck_failed'
  | 'bios_state_unavailable'
  | 'bios_requirements_not_met'
  | 'bios_continue_blocked'
  | 'bios_restart_failed';

export interface BiosActionFeedback {
  advanced: boolean;
  message: string;
}

export interface BiosStepFlowDependencies {
  profile: HardwareProfile | null;
  currentState: BiosOrchestratorState | null;
  applyVerifiedState: (state: BiosOrchestratorState) => void;
  recheckManualChanges: (
    profile: HardwareProfile,
    selectedChanges: Record<string, BiosSettingSelection>,
  ) => Promise<BiosOrchestratorState>;
  continueWithCurrentState: (
    profile: HardwareProfile,
    selectedChanges: Record<string, BiosSettingSelection>,
  ) => Promise<BiosOrchestratorState>;
  advanceToBuildStep: () => boolean;
  openRecoverySurface: (payload: FailureRecoveryPayload) => void;
}

export function summarizeBiosBlockingIssues(
  state: Pick<BiosOrchestratorState, 'blockingIssues' | 'settings'>,
): string {
  const blockers = state.blockingIssues.filter(Boolean);
  if (blockers.length > 0) {
    if (blockers.length === 1) return `${blockers[0]}.`;
    return `${blockers[0]}. ${blockers.length - 1} more BIOS setting${blockers.length - 1 === 1 ? '' : 's'} still need attention.`;
  }

  const remainingRequired = state.settings.filter(
    (setting) => setting.required && setting.verificationStatus !== 'verified',
  );
  if (remainingRequired.length === 0) {
    return 'Required BIOS settings still need confirmation before the app can trust this step.';
  }
  if (remainingRequired.length === 1) {
    return `${remainingRequired[0].name} still needs to be confirmed in firmware.`;
  }
  return `${remainingRequired[0].name} still needs to be confirmed in firmware. ${remainingRequired.length - 1} more required BIOS setting${remainingRequired.length - 1 === 1 ? '' : 's'} still need attention.`;
}

export function buildBiosRecoveryPayload(input: {
  code: BiosRecoveryCode;
  detail?: string | null;
  state?: Pick<BiosOrchestratorState, 'blockingIssues' | 'settings'> | null;
}): FailureRecoveryPayload {
  const detail = input.detail?.trim() || '';
  const blockerSummary = input.state ? summarizeBiosBlockingIssues(input.state) : '';

  switch (input.code) {
    case 'bios_recheck_failed':
      return {
        code: 'bios_recheck_failed',
        message: 'BIOS recheck failed',
        explanation: 'The app could not refresh the firmware checklist from the current machine state.',
        decisionSummary: detail || 'The firmware inspection did not return a clean result.',
        suggestion: 'Stay on this step, verify the settings manually in BIOS, then try Recheck BIOS again.',
        alternatives: [
          {
            text: 'Continue with the current checklist only if you already verified the settings yourself',
            confidence: 'medium',
            confidenceReason: 'Manual confirmation is acceptable for the BIOS step, but the later build guard will still revalidate.',
            group: 'try_alternative',
            reason: 'This avoids treating a firmware probe failure as a hard crash.',
            expectedOutcome: 'You keep the BIOS step usable even when firmware readback is unavailable.',
            recommended: false,
          },
        ],
        category: 'bios_error',
        severity: 'warning',
        contextNote: detail || undefined,
        rawMessage: detail || undefined,
        targetStep: 'bios',
      };
    case 'bios_state_unavailable':
      return {
        code: 'bios_state_unavailable',
        message: 'BIOS state is unavailable',
        explanation: 'The current BIOS checklist is missing or stale, so the app cannot continue from this step safely.',
        decisionSummary: detail || 'The BIOS step needs a fresh known state before it can continue.',
        suggestion: 'Click Recheck BIOS to rebuild the checklist for this hardware session.',
        category: 'bios_error',
        severity: 'warning',
        contextNote: detail || undefined,
        rawMessage: detail || undefined,
        targetStep: 'bios',
      };
    case 'bios_requirements_not_met':
      return {
        code: 'bios_requirements_not_met',
        message: 'BIOS settings still need attention',
        explanation: blockerSummary || 'Required BIOS settings are still missing or unverified.',
        decisionSummary: 'The BIOS gate is doing its job and keeping the build locked until the checklist is complete.',
        suggestion: 'Review the failed or unknown BIOS items, fix them in firmware, then use Recheck BIOS to confirm them.',
        category: 'bios_error',
        severity: 'warning',
        targetStep: 'bios',
      };
    case 'bios_restart_failed':
      return {
        code: 'bios_restart_failed',
        message: 'Firmware restart is unavailable',
        explanation: 'The app could not trigger a restart directly into BIOS/UEFI settings on this system.',
        decisionSummary: detail || 'Automatic firmware restart is not available from the current host context.',
        suggestion: 'Enter BIOS manually with your vendor hotkey, then return here and use Recheck BIOS.',
        category: 'bios_error',
        severity: 'warning',
        contextNote: detail || undefined,
        rawMessage: detail || undefined,
        targetStep: 'bios',
      };
    case 'bios_continue_blocked':
    default:
      return {
        code: 'bios_continue_blocked',
        message: 'Could not continue from the BIOS step',
        explanation: 'The app could not carry the current BIOS checklist state into the next step.',
        decisionSummary: detail || 'The BIOS step did not produce a usable continuation state.',
        suggestion: 'Recheck BIOS once, then try Continue again.',
        category: 'bios_error',
        severity: 'warning',
        contextNote: detail || undefined,
        rawMessage: detail || undefined,
        targetStep: 'bios',
      };
  }
}

export async function performBiosRecheck(
  deps: BiosStepFlowDependencies,
  selectedChanges: Record<string, BiosSettingSelection>,
): Promise<BiosActionFeedback> {
  if (!deps.profile) {
    const payload = buildBiosRecoveryPayload({ code: 'bios_state_unavailable' });
    deps.openRecoverySurface(payload);
    return { advanced: false, message: payload.explanation ?? payload.message };
  }

  try {
    const state = await deps.recheckManualChanges(deps.profile, selectedChanges);
    deps.applyVerifiedState(state);
    if (state.readyToBuild && state.stage === 'complete') {
      return {
        advanced: false,
        message: 'BIOS recheck complete. Required settings are verified and you can continue when ready.',
      };
    }
    return {
      advanced: false,
      message: summarizeBiosBlockingIssues(state),
    };
  } catch (error: any) {
    const payload = buildBiosRecoveryPayload({
      code: 'bios_recheck_failed',
      detail: error?.message || 'Unknown BIOS recheck failure.',
    });
    deps.openRecoverySurface(payload);
    return { advanced: false, message: payload.explanation ?? payload.message };
  }
}

export async function performBiosContinue(
  deps: BiosStepFlowDependencies,
  selectedChanges: Record<string, BiosSettingSelection>,
): Promise<BiosActionFeedback> {
  if (!deps.profile || !deps.currentState) {
    const payload = buildBiosRecoveryPayload({ code: 'bios_state_unavailable' });
    deps.openRecoverySurface(payload);
    return { advanced: false, message: payload.explanation ?? payload.message };
  }

  try {
    const state = await deps.continueWithCurrentState(deps.profile, selectedChanges);
    deps.applyVerifiedState(state);
    if (!state.readyToBuild || state.stage !== 'complete') {
      return {
        advanced: false,
        message: summarizeBiosBlockingIssues(state),
      };
    }

    const advanced = deps.advanceToBuildStep();
    if (!advanced) {
      return {
        advanced: false,
        message: 'The app kept you on the BIOS step because another prerequisite is still blocking the EFI build.',
      };
    }

    return {
      advanced: true,
      message: 'BIOS checklist accepted. Moving to the EFI build step.',
    };
  } catch (error: any) {
    const payload = buildBiosRecoveryPayload({
      code: 'bios_continue_blocked',
      detail: error?.message || 'Unknown BIOS continuation failure.',
    });
    deps.openRecoverySurface(payload);
    return { advanced: false, message: payload.explanation ?? payload.message };
  }
}
