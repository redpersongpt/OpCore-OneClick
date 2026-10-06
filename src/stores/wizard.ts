import { create } from 'zustand';

export const STEPS = [
  'welcome',
  'scan',
  'hardware',
  'compatibility',
  'bios',
  'build',
  'review',
  'deploy',
  'complete',
] as const;

export type Step = (typeof STEPS)[number];

/** Long-running operations that lock navigation while they run. */
export type LockReason = 'scan' | 'build' | 'flash' | 'recovery';

export function stepIndex(step: Step): number {
  return STEPS.indexOf(step);
}

/** Index of the first step that is not completed (steps unlock in order). */
export function firstIncompleteIndex(completed: readonly Step[]): number {
  const idx = STEPS.findIndex((s) => !completed.includes(s));
  return idx === -1 ? STEPS.length - 1 : idx;
}

interface WizardState {
  step: Step;
  /** Always a prefix of STEPS (invalidation removes everything after a point). */
  completed: Step[];
  locks: LockReason[];

  canVisit: (step: Step) => boolean;
  isLocked: () => boolean;
  goTo: (step: Step) => boolean;
  /** Mark `step` done and (by default) move to the next one. */
  complete: (step: Step, advance?: boolean) => void;
  /** Forget completion of `step` and everything after it. */
  invalidateFrom: (step: Step) => void;
  lock: (reason: LockReason) => void;
  unlock: (reason: LockReason) => void;
  restore: (step: Step, completed: Step[]) => void;
  reset: () => void;
}

export const useWizard = create<WizardState>((set, get) => ({
  step: 'welcome',
  completed: [],
  locks: [],

  canVisit: (step) => stepIndex(step) <= firstIncompleteIndex(get().completed),

  isLocked: () => get().locks.length > 0,

  goTo: (step) => {
    const state = get();
    if (step === state.step) return true;
    if (state.locks.length > 0 || !state.canVisit(step)) return false;
    set({ step });
    return true;
  },

  complete: (step, advance = true) => {
    const state = get();
    if (stepIndex(step) > firstIncompleteIndex(state.completed)) return;
    const completed = STEPS.filter((s) => state.completed.includes(s) || s === step);
    const next = STEPS[Math.min(stepIndex(step) + 1, STEPS.length - 1)];
    set({ completed, step: advance ? next : state.step });
  },

  invalidateFrom: (step) => {
    const state = get();
    const cut = stepIndex(step);
    const completed = state.completed.filter((s) => stepIndex(s) < cut);
    const limit = firstIncompleteIndex(completed);
    const current = stepIndex(state.step) > limit ? STEPS[limit] : state.step;
    set({ completed, step: current });
  },

  lock: (reason) => {
    const { locks } = get();
    if (!locks.includes(reason)) set({ locks: [...locks, reason] });
  },

  unlock: (reason) => set({ locks: get().locks.filter((l) => l !== reason) }),

  restore: (step, completed) => {
    const ordered = STEPS.filter((s) => completed.includes(s));
    const prefix: Step[] = [];
    for (const s of ordered) {
      if (stepIndex(s) !== prefix.length) break;
      prefix.push(s);
    }
    const limit = firstIncompleteIndex(prefix);
    set({ completed: prefix, step: stepIndex(step) > limit ? STEPS[limit] : step });
  },

  reset: () => set({ step: 'welcome', completed: [], locks: [] }),
}));
