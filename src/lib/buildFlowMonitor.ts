import type { TaskKind, TaskState, TaskStatus } from '../../electron/taskManager.js';
import type { StepGuardDecision, StepGuardState, StepId } from './installStepGuards.js';
import { evaluateStepTransition } from './installStepGuards.js';

export type BuildFlowPhase =
  | 'idle'
  | 'preflight'
  | 'simulation'
  | 'efi-build'
  | 'kext-fetch'
  | 'validation'
  | 'recovery-dry-run'
  | 'recovery-download'
  | 'finalizing'
  | 'complete'
  | 'failed'
  | 'taking_longer'
  | 'stalled';

export interface BuildFlowSnapshot {
  active: boolean;
  runId: number;
  phase: BuildFlowPhase;
  uiStep: StepId;
  startedAt: number;
  lastProgressAt: number | null;
  activeTaskKind: TaskKind | null;
  activeTaskStatus: TaskStatus | null;
  lastTaskPhase: string | null;
  taskCompleteEventFired: boolean;
  validationStarted: boolean;
  validationFinished: boolean;
  pendingRendererExpectation: string | null;
  transitionGuardBlocked: string | null;
  stalledReason: string | null;
}

export interface BuildFlowThresholds {
  softMs: number;
  hardMs: number;
  terminalWaitMs: number;
}

export interface BuildFlowStallDecision {
  level: 'healthy' | 'taking_longer' | 'stalled';
  reason: string | null;
  pendingCondition: string | null;
}

export const DEFAULT_BUILD_FLOW_THRESHOLDS: BuildFlowThresholds = {
  softMs: 25_000,
  hardMs: 70_000,
  terminalWaitMs: 6_000,
};

export function evaluateStepTransitionWithOverrides(
  target: StepId,
  state: StepGuardState,
  overrides?: Partial<StepGuardState>,
): StepGuardDecision {
  return evaluateStepTransition(target, {
    ...state,
    ...(overrides ?? {}),
  });
}

export function latestTaskByKind(
  tasks: Iterable<TaskState>,
  kind: TaskKind,
): TaskState | undefined {
  let latest: TaskState | undefined;
  for (const task of tasks) {
    if (task.kind !== kind) continue;
    if (!latest) {
      latest = task;
      continue;
    }
    if (task.startedAt > latest.startedAt || task.lastUpdateAt > latest.lastUpdateAt) {
      latest = task;
    }
  }
  return latest;
}

export function evaluateBuildFlowStall(
  snapshot: BuildFlowSnapshot | null,
  now: number,
  thresholds: BuildFlowThresholds = DEFAULT_BUILD_FLOW_THRESHOLDS,
): BuildFlowStallDecision {
  if (!snapshot?.active) {
    return { level: 'healthy', reason: null, pendingCondition: null };
  }

  if (snapshot.transitionGuardBlocked) {
    return {
      level: 'stalled',
      reason: `The renderer could not leave ${snapshot.uiStep} because ${snapshot.transitionGuardBlocked}`,
      pendingCondition: snapshot.pendingRendererExpectation,
    };
  }

  if (snapshot.activeTaskStatus === 'failed' || snapshot.activeTaskStatus === 'cancelled') {
    return {
      level: 'stalled',
      reason: `The ${snapshot.activeTaskKind ?? 'current'} task ended ${snapshot.activeTaskStatus} before the renderer reached a terminal result.`,
      pendingCondition: snapshot.pendingRendererExpectation,
    };
  }

  const lastSignalAt = snapshot.lastProgressAt ?? snapshot.startedAt;
  const idleMs = now - lastSignalAt;

  if (snapshot.taskCompleteEventFired && snapshot.pendingRendererExpectation) {
    if (idleMs >= thresholds.terminalWaitMs) {
      return {
        level: 'stalled',
        reason: `The ${snapshot.activeTaskKind ?? 'current'} task reported complete, but the renderer is still waiting for ${snapshot.pendingRendererExpectation}.`,
        pendingCondition: snapshot.pendingRendererExpectation,
      };
    }
    return {
      level: 'taking_longer',
      reason: `The task completed, but the renderer is still waiting for ${snapshot.pendingRendererExpectation}.`,
      pendingCondition: snapshot.pendingRendererExpectation,
    };
  }

  if (idleMs >= thresholds.hardMs) {
    return {
      level: 'stalled',
      reason: `No progress signal arrived for ${(idleMs / 1000).toFixed(0)} seconds during ${snapshot.phase}.`,
      pendingCondition: snapshot.pendingRendererExpectation,
    };
  }

  if (idleMs >= thresholds.softMs) {
    return {
      level: 'taking_longer',
      reason: `This phase has not reported progress for ${(idleMs / 1000).toFixed(0)} seconds.`,
      pendingCondition: snapshot.pendingRendererExpectation,
    };
  }

  return {
    level: 'healthy',
    reason: null,
    pendingCondition: snapshot.pendingRendererExpectation,
  };
}

export function buildBuildFlowContext(snapshot: BuildFlowSnapshot | null): string {
  if (!snapshot) return 'No build flow context was captured.';

  const lines = [
    `Current phase: ${snapshot.phase}`,
    `Visible step: ${snapshot.uiStep}`,
    `Started at: ${new Date(snapshot.startedAt).toISOString()}`,
    `Last progress signal: ${snapshot.lastProgressAt ? new Date(snapshot.lastProgressAt).toISOString() : 'none'}`,
    `Active task: ${snapshot.activeTaskKind ?? 'none'} (${snapshot.activeTaskStatus ?? 'n/a'})`,
    `Last main-process phase: ${snapshot.lastTaskPhase ?? 'none'}`,
    `Validation started: ${snapshot.validationStarted ? 'yes' : 'no'}`,
    `Validation finished: ${snapshot.validationFinished ? 'yes' : 'no'}`,
    `Task complete event fired: ${snapshot.taskCompleteEventFired ? 'yes' : 'no'}`,
    `Pending renderer expectation: ${snapshot.pendingRendererExpectation ?? 'none'}`,
    `Transition guard blocked next step: ${snapshot.transitionGuardBlocked ?? 'no'}`,
  ];

  if (snapshot.stalledReason) {
    lines.push(`Observed stall reason: ${snapshot.stalledReason}`);
  }

  return lines.join('\n');
}
