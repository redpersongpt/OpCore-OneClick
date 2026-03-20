import type { CompatibilityReport } from '../../electron/compatibility.js';
import type { BIOSConfig, HardwareProfile } from '../../electron/configGenerator.js';
import type { FlowGuardResult } from './stateMachine.js';

export type StepId =
  | 'landing'
  | 'welcome'
  | 'prereq'
  | 'precheck'
  | 'scanning'
  | 'version-select'
  | 'report'
  | 'method-select'
  | 'bios'
  | 'building'
  | 'kext-fetch'
  | 'recovery-download'
  | 'usb-select'
  | 'part-prep'
  | 'flashing'
  | 'complete'
  | 'troubleshooting';

export interface StepGuardState {
  profile: HardwareProfile | null;
  compat: CompatibilityReport | null;
  hasLiveHardwareContext: boolean;
  biosReady: boolean;
  biosAccepted: boolean;
  buildReady: boolean;
  efiPath: string | null;
  biosConf: BIOSConfig | null;
  selectedUsb: string | null;
  compatibilityBlocked: boolean;
  validationBlocked: boolean;
  postBuildReady: boolean;
  localBuildGuard: FlowGuardResult;
  localDeployGuard: FlowGuardResult;
}

export interface StepGuardDecision {
  ok: boolean;
  reason?: string;
  redirect?: StepId;
}

function evaluatePlanningOnlyBlock(target: StepId, state: StepGuardState): StepGuardDecision | null {
  if (
    !state.hasLiveHardwareContext &&
    (target === 'bios' ||
      target === 'building' ||
      target === 'recovery-download' ||
      target === 'method-select' ||
      target === 'usb-select' ||
      target === 'part-prep' ||
      target === 'flashing')
  ) {
    return {
      ok: false,
      reason: 'Imported or restored hardware profiles are planning inputs only. Run a live hardware scan in this session before BIOS, build, or deployment actions.',
      redirect: 'report',
    };
  }

  return null;
}

function evaluateLegacyPrereq(target: StepId, state: StepGuardState): StepGuardDecision {
  const planningOnlyBlock = evaluatePlanningOnlyBlock(target, state);
  if (planningOnlyBlock) return planningOnlyBlock;

  switch (target) {
    case 'version-select':
      return state.profile
        ? { ok: true }
        : { ok: false, reason: 'Hardware scan required before selecting a version.', redirect: 'precheck' };
    case 'report':
      return state.profile && state.compat
        ? { ok: true }
        : { ok: false, reason: 'Hardware scan and version selection required.', redirect: 'precheck' };
    case 'bios':
      return state.profile && state.biosConf && state.compat && state.compat.errors.length === 0
        ? { ok: true }
        : { ok: false, reason: 'Compatibility must be clear before BIOS preparation.', redirect: 'report' };
    case 'building':
      return state.profile && state.compat && state.compat.errors.length === 0 && state.biosReady
        ? { ok: true }
        : {
            ok: false,
            reason: 'Compatibility and BIOS preparation must be complete before building an EFI.',
            redirect: state.compat && state.compat.errors.length === 0 ? 'bios' : 'report',
          };
    case 'recovery-download':
      return state.efiPath && state.compat && state.compat.errors.length === 0 && state.biosReady
        ? { ok: true }
        : {
            ok: false,
            reason: 'A compatible validated EFI and completed BIOS preparation are required before downloading recovery.',
            redirect: state.compat && state.compat.errors.length === 0 ? 'bios' : 'report',
          };
    case 'method-select':
      return state.buildReady && state.efiPath && state.compat && state.compat.errors.length === 0 && state.biosReady
        ? { ok: true }
        : {
            ok: false,
            reason: 'A compatible validated EFI and completed BIOS preparation are required before selecting an installation method.',
            redirect: state.compat && state.compat.errors.length === 0 ? 'bios' : 'report',
          };
    case 'usb-select':
    case 'part-prep':
      return state.buildReady && state.efiPath && state.compat && state.compat.errors.length === 0 && state.biosReady
        ? { ok: true }
        : {
            ok: false,
            reason: 'A compatible validated EFI and completed BIOS preparation are required.',
            redirect: state.compat && state.compat.errors.length === 0 ? 'bios' : 'report',
          };
    case 'flashing':
      return state.buildReady && state.efiPath && state.selectedUsb && state.compat && state.compat.errors.length === 0 && state.biosReady
        ? { ok: true }
        : {
            ok: false,
            reason: 'USB drive must be selected, the hardware path must remain compatible, and BIOS preparation must be complete.',
            redirect: state.compat && state.compat.errors.length === 0 ? 'bios' : 'report',
          };
    default:
      return { ok: true };
  }
}

export function evaluateStepTransition(target: StepId, state: StepGuardState): StepGuardDecision {
  const planningOnlyBlock = evaluatePlanningOnlyBlock(target, state);
  if (planningOnlyBlock) {
    return planningOnlyBlock;
  }

  if (target === 'bios' && state.compatibilityBlocked) {
    return { ok: false, reason: 'Compatibility must be clear before BIOS preparation.', redirect: 'report' };
  }

  if (target === 'building') {
    return state.localBuildGuard.allowed
      ? { ok: true }
      : {
          ok: false,
          reason: state.localBuildGuard.reason ?? 'BIOS preparation must be complete before building.',
          redirect: state.compatibilityBlocked ? 'report' : 'bios',
        };
  }

  if (target === 'recovery-download' || target === 'method-select' || target === 'usb-select') {
    return state.postBuildReady
      ? { ok: true }
      : {
          ok: false,
          reason: state.validationBlocked
            ? 'EFI validation must pass before continuing.'
            : state.compatibilityBlocked
            ? 'Compatibility must remain unblocked before continuing.'
            : (state.biosReady || state.biosAccepted)
            ? 'A validated EFI is required before continuing.'
            : 'BIOS preparation must be complete before continuing.',
          redirect: state.compatibilityBlocked || state.validationBlocked || !state.buildReady || !state.efiPath
            ? 'report'
            : 'bios',
        };
  }

  if (target === 'part-prep') {
    return state.buildReady && state.efiPath && state.compat && state.compat.errors.length === 0 && state.biosReady
      ? { ok: true }
      : {
          ok: false,
          reason: state.validationBlocked
            ? 'EFI validation must pass before continuing.'
            : state.compatibilityBlocked
            ? 'Compatibility must remain unblocked before continuing.'
            : state.biosReady
            ? 'A validated EFI is required before continuing.'
            : 'BIOS preparation must be complete before continuing.',
          redirect: state.compatibilityBlocked || state.validationBlocked || !state.buildReady || !state.efiPath ? 'report' : 'bios',
        };
  }

  if (target === 'flashing') {
    return state.localDeployGuard.allowed && !!state.selectedUsb
      ? { ok: true }
      : {
          ok: false,
          reason: !state.selectedUsb
            ? 'Select a target drive before flashing.'
            : !state.localDeployGuard.allowed
            ? (state.localDeployGuard.reason ?? 'Deployment is blocked until the BIOS and EFI remain valid.')
            : state.validationBlocked
            ? 'EFI validation must pass before flashing.'
            : state.compatibilityBlocked
            ? 'Compatibility must remain unblocked before flashing.'
            : state.biosReady
            ? 'A validated EFI is required before flashing.'
            : 'BIOS preparation must be complete before flashing.',
          redirect: state.compatibilityBlocked || state.validationBlocked || !state.buildReady || !state.efiPath ? 'report' : 'bios',
        };
  }

  return evaluateLegacyPrereq(target, state);
}
