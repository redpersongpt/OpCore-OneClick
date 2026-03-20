// ── State Machine — typed, guarded, deterministic ─────────────────────────────
//
// Two machines:
//   1. BiosFlowMachine  — firmware preparation lifecycle
//   2. ReleaseFlowMachine — overall app pipeline lifecycle
//
// Both enforce explicit transitions and provide guard hooks for
// invalidation rules (e.g. changing target macOS invalidates build).

// ── Generic machine ──────────────────────────────────────────────────────────

export interface Transition<S extends string> {
  target: S;
  guard?: () => boolean;
}

export type TransitionMap<S extends string, E extends string> = {
  [state in S]: Partial<Record<E, S | Transition<S>>>;
};

export interface MachineSnapshot<S extends string> {
  state: S;
  history: S[];
}

export function createMachine<S extends string, E extends string>(
  initial: S,
  transitions: TransitionMap<S, E>,
): {
  current: () => S;
  can: (event: E) => boolean;
  send: (event: E) => S;
  matches: (...states: S[]) => boolean;
  snapshot: () => MachineSnapshot<S>;
  reset: () => void;
} {
  let state: S = initial;
  const history: S[] = [initial];

  function resolve(entry: S | Transition<S> | undefined): S | null {
    if (!entry) return null;
    if (typeof entry === 'string') return entry;
    if (entry.guard && !entry.guard()) return null;
    return entry.target;
  }

  return {
    current: () => state,
    can(event: E): boolean {
      const entry = transitions[state]?.[event];
      return resolve(entry) !== null;
    },
    send(event: E): S {
      const entry = transitions[state]?.[event];
      const next = resolve(entry);
      if (!next) {
        throw new Error(`Invalid transition: ${state} + ${event}`);
      }
      state = next;
      history.push(state);
      return state;
    },
    matches(...states: S[]): boolean {
      return states.includes(state);
    },
    snapshot(): MachineSnapshot<S> {
      return { state, history: [...history] };
    },
    reset() {
      state = initial;
      history.length = 0;
      history.push(initial);
    },
  };
}

// ── BIOS Flow State Machine ──────────────────────────────────────────────────

export type BiosFlowState =
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

export type BiosFlowEvent =
  | 'PLAN'
  | 'START_AUTO_APPLY'
  | 'AUTO_APPLY_DONE'
  | 'REQUEST_REBOOT'
  | 'REBOOT_ACCEPTED'
  | 'REBOOT_REJECTED'
  | 'REBOOT_UNSUPPORTED'
  | 'USER_RETURNED'
  | 'START_VERIFY'
  | 'VERIFY_COMPLETE'
  | 'VERIFY_PARTIAL'
  | 'VERIFY_BLOCKED'
  | 'MARK_MANUAL_COMPLETE'
  | 'RESET';

export const BIOS_FLOW_TRANSITIONS: TransitionMap<BiosFlowState, BiosFlowEvent> = {
  idle: {
    PLAN: 'planned',
  },
  planned: {
    START_AUTO_APPLY: 'auto_applying',
    REQUEST_REBOOT: 'ready_for_reboot',
    MARK_MANUAL_COMPLETE: 'verifying',
    START_VERIFY: 'verifying',
    RESET: 'idle',
  },
  auto_applying: {
    AUTO_APPLY_DONE: 'planned',
    RESET: 'idle',
  },
  ready_for_reboot: {
    REBOOT_ACCEPTED: 'rebooting_to_firmware',
    REBOOT_REJECTED: 'planned',
    REBOOT_UNSUPPORTED: 'unsupported_host',
    RESET: 'idle',
  },
  rebooting_to_firmware: {
    USER_RETURNED: 'awaiting_return',
  },
  awaiting_return: {
    START_VERIFY: 'resumed_from_firmware',
    RESET: 'idle',
  },
  resumed_from_firmware: {
    START_VERIFY: 'verifying',
    VERIFY_COMPLETE: 'complete',
    VERIFY_PARTIAL: 'partially_verified',
    VERIFY_BLOCKED: 'blocked',
    RESET: 'idle',
  },
  verifying: {
    VERIFY_COMPLETE: 'complete',
    VERIFY_PARTIAL: 'partially_verified',
    VERIFY_BLOCKED: 'blocked',
    RESET: 'idle',
  },
  partially_verified: {
    START_VERIFY: 'verifying',
    MARK_MANUAL_COMPLETE: 'verifying',
    REQUEST_REBOOT: 'ready_for_reboot',
    RESET: 'idle',
  },
  complete: {
    RESET: 'idle',
  },
  blocked: {
    START_VERIFY: 'verifying',
    MARK_MANUAL_COMPLETE: 'verifying',
    RESET: 'idle',
  },
  unsupported_host: {
    MARK_MANUAL_COMPLETE: 'verifying',
    PLAN: 'planned',
    RESET: 'idle',
  },
};

export function createBiosFlowMachine(initial: BiosFlowState = 'idle') {
  return createMachine<BiosFlowState, BiosFlowEvent>(initial, BIOS_FLOW_TRANSITIONS);
}

// ── Release Flow State Machine ───────────────────────────────────────────────

export type ReleaseFlowState =
  | 'scan'
  | 'compatibility'
  | 'bios'
  | 'build'
  | 'validate'
  | 'method'
  | 'deploy'
  | 'complete'
  | 'blocked';

export type ReleaseFlowEvent =
  | 'SCAN_COMPLETE'
  | 'COMPATIBILITY_PASS'
  | 'COMPATIBILITY_FAIL'
  | 'BIOS_COMPLETE'
  | 'BIOS_BLOCKED'
  | 'BUILD_COMPLETE'
  | 'BUILD_FAIL'
  | 'VALIDATION_PASS'
  | 'VALIDATION_FAIL'
  | 'METHOD_SELECTED'
  | 'DEPLOY_COMPLETE'
  | 'DEPLOY_FAIL'
  | 'INVALIDATE_BUILD'
  | 'INVALIDATE_BIOS'
  | 'RESET';

export const RELEASE_FLOW_TRANSITIONS: TransitionMap<ReleaseFlowState, ReleaseFlowEvent> = {
  scan: {
    SCAN_COMPLETE: 'compatibility',
    RESET: 'scan',
  },
  compatibility: {
    COMPATIBILITY_PASS: 'bios',
    COMPATIBILITY_FAIL: 'blocked',
    RESET: 'scan',
  },
  bios: {
    BIOS_COMPLETE: 'build',
    BIOS_BLOCKED: 'blocked',
    INVALIDATE_BIOS: 'bios',
    RESET: 'scan',
  },
  build: {
    BUILD_COMPLETE: 'validate',
    BUILD_FAIL: 'blocked',
    INVALIDATE_BUILD: 'bios',
    INVALIDATE_BIOS: 'bios',
    RESET: 'scan',
  },
  validate: {
    VALIDATION_PASS: 'method',
    VALIDATION_FAIL: 'blocked',
    INVALIDATE_BUILD: 'bios',
    RESET: 'scan',
  },
  method: {
    METHOD_SELECTED: 'deploy',
    INVALIDATE_BUILD: 'bios',
    RESET: 'scan',
  },
  deploy: {
    DEPLOY_COMPLETE: 'complete',
    DEPLOY_FAIL: 'blocked',
    RESET: 'scan',
  },
  complete: {
    RESET: 'scan',
    INVALIDATE_BUILD: 'bios',
  },
  blocked: {
    RESET: 'scan',
    COMPATIBILITY_PASS: 'bios',
    BIOS_COMPLETE: 'build',
    VALIDATION_PASS: 'method',
  },
};

export function createReleaseFlowMachine(initial: ReleaseFlowState = 'scan') {
  return createMachine<ReleaseFlowState, ReleaseFlowEvent>(initial, RELEASE_FLOW_TRANSITIONS);
}

// ── Invalidation rules ───────────────────────────────────────────────────────
// These encode the product rules from the handoff document.

export interface InvalidationContext {
  releaseFlow: ReturnType<typeof createReleaseFlowMachine>;
  biosFlow: ReturnType<typeof createBiosFlowMachine>;
}

/** Changing target macOS invalidates build + validation. */
export function invalidateOnTargetChange(ctx: InvalidationContext): void {
  if (ctx.releaseFlow.can('INVALIDATE_BUILD')) {
    ctx.releaseFlow.send('INVALIDATE_BUILD');
  }
}

/** Failed BIOS verification blocks build. */
export function invalidateOnBiosFailure(ctx: InvalidationContext): void {
  if (ctx.releaseFlow.can('INVALIDATE_BIOS')) {
    ctx.releaseFlow.send('INVALIDATE_BIOS');
  }
}

/** Check whether a destructive deploy step is eligible. */
export function canDeploy(ctx: InvalidationContext): { eligible: boolean; reason: string | null } {
  if (!ctx.biosFlow.matches('complete')) {
    return { eligible: false, reason: 'BIOS preparation must be complete before deployment.' };
  }
  if (!ctx.releaseFlow.matches('deploy')) {
    return { eligible: false, reason: `Cannot deploy from state: ${ctx.releaseFlow.current()}` };
  }
  return { eligible: true, reason: null };
}

/** Check whether build step is eligible. */
export function canBuild(ctx: InvalidationContext): { eligible: boolean; reason: string | null } {
  if (!ctx.biosFlow.matches('complete')) {
    return { eligible: false, reason: 'BIOS preparation must be complete before building.' };
  }
  if (!ctx.releaseFlow.matches('build', 'bios')) {
    return { eligible: false, reason: `Cannot build from state: ${ctx.releaseFlow.current()}` };
  }
  return { eligible: true, reason: null };
}

// ── Flow guard result (shared IPC type) ──────────────────────────────────────

export interface FlowGuardResult {
  allowed: boolean;
  reason: string | null;
  currentState: string;
  biosState: string;
}

export interface BiosStateDerivationInput {
  stage?: string | null;
  readyToBuild?: boolean;
}

export function deriveBiosFlowState(input: BiosStateDerivationInput | null | undefined): BiosFlowState {
  if (!input?.stage) return 'idle';
  if (input.readyToBuild && input.stage === 'complete') return 'complete';

  switch (input.stage) {
    case 'planned':
    case 'auto_applying':
    case 'ready_for_reboot':
    case 'rebooting_to_firmware':
    case 'awaiting_return':
    case 'resumed_from_firmware':
    case 'verifying':
    case 'partially_verified':
    case 'complete':
    case 'blocked':
    case 'unsupported_host':
      return input.stage;
    default:
      return 'idle';
  }
}

export interface ReleaseStateDerivationInput {
  step?: string | null;
  hasProfile: boolean;
  compatibilityBlocked: boolean;
  biosFlowState: BiosFlowState;
  buildReady: boolean;
  hasEfi: boolean;
  validationBlocked?: boolean;
}

export function deriveReleaseFlowState(input: ReleaseStateDerivationInput): ReleaseFlowState {
  if (!input.hasProfile) return 'scan';
  if (input.compatibilityBlocked) return 'blocked';
  if (input.validationBlocked) return 'blocked';

  switch (input.step) {
    case 'welcome':
    case 'prereq':
    case 'precheck':
    case 'scanning':
    case 'landing':
    case undefined:
    case null:
      return 'scan';
    case 'version-select':
    case 'report':
      return 'compatibility';
    case 'bios':
      return 'bios';
    case 'building':
    case 'kext-fetch':
      return 'build';
    case 'recovery-download':
      return input.buildReady && input.hasEfi ? 'validate' : 'build';
    case 'method-select':
      return 'method';
    case 'usb-select':
    case 'part-prep':
    case 'flashing':
      return 'deploy';
    case 'complete':
      return 'complete';
    default:
      break;
  }

  if (!input.hasEfi || !input.buildReady) {
    return input.biosFlowState === 'complete' ? 'build' : 'bios';
  }

  return 'validate';
}

export interface SharedFlowGuardContext {
  compatibilityBlocked: boolean;
  biosFlowState: BiosFlowState;
  releaseFlowState: ReleaseFlowState;
}

export interface DeployFlowGuardContext extends SharedFlowGuardContext {
  validationBlocked: boolean;
  hasEfi: boolean;
}

export function evaluateBuildGuard(ctx: SharedFlowGuardContext): FlowGuardResult {
  if (ctx.compatibilityBlocked) {
    return {
      allowed: false,
      reason: 'Compatibility is blocked. Fix the compatibility report before building.',
      currentState: ctx.releaseFlowState,
      biosState: ctx.biosFlowState,
    };
  }

  const result = canBuild({
    releaseFlow: createReleaseFlowMachine(ctx.releaseFlowState),
    biosFlow: createBiosFlowMachine(ctx.biosFlowState),
  });

  return {
    allowed: result.eligible,
    reason: result.reason,
    currentState: ctx.releaseFlowState,
    biosState: ctx.biosFlowState,
  };
}

export function evaluateDeployGuard(ctx: DeployFlowGuardContext): FlowGuardResult {
  if (ctx.compatibilityBlocked) {
    return {
      allowed: false,
      reason: 'Compatibility is blocked. Fix the compatibility report before deployment.',
      currentState: ctx.releaseFlowState,
      biosState: ctx.biosFlowState,
    };
  }

  if (!ctx.hasEfi) {
    return {
      allowed: false,
      reason: 'A validated EFI is required before deployment.',
      currentState: ctx.releaseFlowState,
      biosState: ctx.biosFlowState,
    };
  }

  if (ctx.validationBlocked) {
    return {
      allowed: false,
      reason: 'EFI validation is blocked. Rebuild before deployment.',
      currentState: ctx.releaseFlowState,
      biosState: ctx.biosFlowState,
    };
  }

  const result = canDeploy({
    releaseFlow: createReleaseFlowMachine(ctx.releaseFlowState),
    biosFlow: createBiosFlowMachine(ctx.biosFlowState),
  });

  return {
    allowed: result.eligible,
    reason: result.reason,
    currentState: ctx.releaseFlowState,
    biosState: ctx.biosFlowState,
  };
}
