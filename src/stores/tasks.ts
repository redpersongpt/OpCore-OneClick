import { create } from 'zustand';
import { api } from '../bridge/api';
import type { TaskUpdate } from '../bridge/types';
import { parseBuildDetail, UNCANCELLABLE_PHASES } from '../lib/buildProgress';

export const TASK_KINDS = {
  scan: 'hardware-scan',
  build: 'efi-build',
  recovery: 'recovery-download',
  flash: 'usb-flash',
} as const;

/**
 * Task kinds the user may cancel. A USB write is never offered: once the
 * drive is being erased, stopping half way only leaves it unusable.
 */
export const CANCELLABLE_KINDS: readonly string[] = [TASK_KINDS.scan, TASK_KINDS.build, TASK_KINDS.recovery];

/** "requested": task_cancel was sent; "refused": the backend said the task can no longer be stopped. */
export type CancelState = 'requested' | 'refused';

/** True when `task` is running and the backend would accept a cancel for its current step. */
export function isCancellable(task: TaskUpdate | null | undefined): boolean {
  if (!task || task.status !== 'running' || !CANCELLABLE_KINDS.includes(task.kind)) return false;
  if (task.kind === TASK_KINDS.build) {
    const detail = parseBuildDetail(task.detail);
    if (detail && UNCANCELLABLE_PHASES.includes(detail.phase)) return false;
  }
  return true;
}

interface TasksState {
  tasks: Record<string, TaskUpdate>;
  /** Task ids, most recently updated last. */
  order: string[];
  dismissed: string[];
  cancels: Record<string, CancelState>;

  apply: (update: TaskUpdate) => void;
  dismiss: (taskId: string) => void;
  /** Ask the backend to cancel `taskId`. Resolves to true when it accepted. */
  cancel: (taskId: string) => Promise<boolean>;
  /** Most recently updated task of `kind`. */
  latest: (kind: string) => TaskUpdate | null;
  /** Most recently updated running task of `kind`. */
  running: (kind: string) => TaskUpdate | null;
  /** True while any backend task runs. */
  anyRunning: () => boolean;
  /** The task the task bar should show: newest running one, else the newest undismissed finished one. */
  visible: () => TaskUpdate | null;
  clear: () => void;
}

export const useTasks = create<TasksState>((set, get) => ({
  tasks: {},
  order: [],
  dismissed: [],
  cancels: {},

  apply: (update) =>
    set((s) => ({
      tasks: { ...s.tasks, [update.taskId]: update },
      order: [...s.order.filter((id) => id !== update.taskId), update.taskId],
    })),

  dismiss: (taskId) =>
    set((s) => (s.dismissed.includes(taskId) ? s : { dismissed: [...s.dismissed, taskId] })),

  cancel: async (taskId) => {
    if (get().cancels[taskId] === 'requested') return false;
    set((s) => ({ cancels: { ...s.cancels, [taskId]: 'requested' } }));
    let accepted = false;
    try {
      accepted = (await api.taskCancel(taskId)) === true;
    } catch {
      // The task finished in the meantime.
    }
    if (!accepted) set((s) => ({ cancels: { ...s.cancels, [taskId]: 'refused' } }));
    return accepted;
  },

  latest: (kind) => {
    const { tasks, order } = get();
    for (let i = order.length - 1; i >= 0; i -= 1) {
      const task = tasks[order[i]];
      if (task && task.kind === kind) return task;
    }
    return null;
  },

  running: (kind) => {
    const task = get().latest(kind);
    return task?.status === 'running' ? task : null;
  },

  anyRunning: () => Object.values(get().tasks).some((t) => t.status === 'running'),

  visible: () => {
    const { tasks, order, dismissed } = get();
    for (let i = order.length - 1; i >= 0; i -= 1) {
      const task = tasks[order[i]];
      if (task?.status === 'running') return task;
    }
    // Only the most recent finished task is shown; older results never resurface.
    const last = order.length > 0 ? tasks[order[order.length - 1]] : undefined;
    return last && !dismissed.includes(last.taskId) ? last : null;
  },

  clear: () => set({ tasks: {}, order: [], dismissed: [], cancels: {} }),
}));
