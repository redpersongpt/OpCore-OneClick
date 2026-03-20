import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import type { StepGuardState } from '../src/lib/installStepGuards.js';
import {
  buildBuildFlowContext,
  evaluateBuildFlowStall,
  evaluateStepTransitionWithOverrides,
  latestTaskByKind,
  type BuildFlowSnapshot,
} from '../src/lib/buildFlowMonitor.js';
import type { TaskState } from '../electron/taskManager.js';

function makeGuardState(): StepGuardState {
  return {
    profile: {} as any,
    compat: {
      errors: [],
    } as any,
    hasLiveHardwareContext: true,
    biosReady: true,
    biosAccepted: false,
    buildReady: false,
    efiPath: null,
    biosConf: {} as any,
    selectedUsb: null,
    compatibilityBlocked: false,
    validationBlocked: false,
    postBuildReady: false,
    localBuildGuard: {
      allowed: true,
      reason: null,
      currentState: 'build',
      biosState: 'complete',
    },
    localDeployGuard: {
      allowed: false,
      reason: 'A validated EFI is required before deployment.',
      currentState: 'build',
      biosState: 'complete',
    },
  };
}

function makeSnapshot(overrides?: Partial<BuildFlowSnapshot>): BuildFlowSnapshot {
  return {
    active: true,
    runId: 1,
    phase: 'recovery-download',
    uiStep: 'recovery-download',
    startedAt: 1_000,
    lastProgressAt: 2_000,
    activeTaskKind: 'recovery-download',
    activeTaskStatus: 'running',
    lastTaskPhase: 'Downloading BaseSystem.dmg',
    taskCompleteEventFired: false,
    validationStarted: true,
    validationFinished: true,
    pendingRendererExpectation: 'the installer method screen to open',
    transitionGuardBlocked: null,
    stalledReason: null,
    ...(overrides ?? {}),
  };
}

describe('buildFlowMonitor', () => {
  test('allows internal recovery transition when build state overrides are provided', () => {
    const decision = evaluateStepTransitionWithOverrides('recovery-download', makeGuardState(), {
      buildReady: true,
      efiPath: '/tmp/EFI_Build_123',
      postBuildReady: true,
      validationBlocked: false,
    });

    assert.equal(decision.ok, true);
  });

  test('surfaces a stalled decision when a task completed but the renderer still waits for the next step', () => {
    const decision = evaluateBuildFlowStall(
      makeSnapshot({
        taskCompleteEventFired: true,
        activeTaskStatus: 'complete',
        lastProgressAt: 1_000,
      }),
      8_000,
      { softMs: 2_000, hardMs: 20_000, terminalWaitMs: 5_000 },
    );

    assert.equal(decision.level, 'stalled');
    assert.match(decision.reason ?? '', /reported complete/i);
  });

  test('marks build flow as taking longer before it becomes a hard stall', () => {
    const decision = evaluateBuildFlowStall(
      makeSnapshot({ lastProgressAt: 1_000 }),
      5_500,
      { softMs: 4_000, hardMs: 20_000, terminalWaitMs: 5_000 },
    );

    assert.equal(decision.level, 'taking_longer');
    assert.match(decision.reason ?? '', /has not reported progress/i);
  });

  test('includes pending condition details in human-readable build hang context', () => {
    const context = buildBuildFlowContext(makeSnapshot({
      transitionGuardBlocked: 'a validated EFI is required before continuing',
      stalledReason: 'The renderer could not leave the validated build phase.',
    }));

    assert.match(context, /Pending renderer expectation: the installer method screen to open/);
    assert.match(context, /Transition guard blocked next step: a validated EFI is required before continuing/);
    assert.match(context, /Observed stall reason:/);
  });

  test('picks the latest task for a kind by most recent timestamps', () => {
    const tasks: TaskState[] = [
      {
        taskId: 'efi-build-1',
        kind: 'efi-build',
        status: 'running',
        progress: null,
        error: null,
        startedAt: 100,
        endedAt: null,
        lastUpdateAt: 110,
      },
      {
        taskId: 'efi-build-2',
        kind: 'efi-build',
        status: 'complete',
        progress: null,
        error: null,
        startedAt: 120,
        endedAt: 130,
        lastUpdateAt: 130,
      },
    ];

    const latest = latestTaskByKind(tasks, 'efi-build');
    assert.equal(latest?.taskId, 'efi-build-2');
  });
});
