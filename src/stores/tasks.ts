import { create } from 'zustand';
import type { TaskUpdate } from '../bridge/types';

export const TASK_KINDS = {
  scan: 'hardware-scan',
  build: 'efi-build',
  recovery: 'recovery-download',
  flash: 'usb-flash',
} as const;

/** Task kinds the user may cancel from the task bar. */
export const CANCELLABLE_KINDS: readonly string[] = [TASK_KINDS.build, TASK_KINDS.recovery];

interface TasksState {
  tasks: Record<string, TaskUpdate>;
  /** Task ids, most recently updated last. */
  order: string[];
  dismissed: string[];

  apply: (update: TaskUpdate) => void;
  dismiss: (taskId: string) => void;
  /** Most recently updated task of `kind`. */
  latest: (kind: string) => TaskUpdate | null;
  /** The task the task bar should show: newest running one, else the newest undismissed finished one. */
  visible: () => TaskUpdate | null;
  clear: () => void;
}

export const useTasks = create<TasksState>((set, get) => ({
  tasks: {},
  order: [],
  dismissed: [],

  apply: (update) =>
    set((s) => ({
      tasks: { ...s.tasks, [update.taskId]: update },
      order: [...s.order.filter((id) => id !== update.taskId), update.taskId],
    })),

  dismiss: (taskId) =>
    set((s) => (s.dismissed.includes(taskId) ? s : { dismissed: [...s.dismissed, taskId] })),

  latest: (kind) => {
    const { tasks, order } = get();
    for (let i = order.length - 1; i >= 0; i -= 1) {
      const task = tasks[order[i]];
      if (task && task.kind === kind) return task;
    }
    return null;
  },

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

  clear: () => set({ tasks: {}, order: [], dismissed: [] }),
}));
