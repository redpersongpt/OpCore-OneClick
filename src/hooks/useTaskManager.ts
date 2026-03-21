import { startTransition, useState, useEffect, useCallback } from 'react';
import type { TaskState, TaskKind, TaskUpdatePayload } from '../../electron/taskManager';

export interface UseTaskManagerReturn {
  tasks: Map<string, TaskState>;
  activeTask: (kind: TaskKind) => TaskState | undefined;
  cancelTask: (taskId: string) => Promise<boolean>;
}

export function useTaskManager(): UseTaskManagerReturn {
  const [tasks, setTasks] = useState<Map<string, TaskState>>(new Map());

  useEffect(() => {
    // Hydrate in-flight tasks on mount (handles app reload mid-operation)
    window.electron.taskList().then((list: TaskState[]) => {
      startTransition(() => {
        setTasks(new Map(list.map(t => [t.taskId, t])));
      });
    }).catch(() => {});

    const unsubscribe = window.electron.onTaskUpdate((payload: TaskUpdatePayload) => {
      startTransition(() => {
        setTasks(prev => {
          const current = prev.get(payload.task.taskId);
          if (current
            && current.status === payload.task.status
            && current.error === payload.task.error
            && current.lastUpdateAt === payload.task.lastUpdateAt
            && current.endedAt === payload.task.endedAt
            && current.progress === payload.task.progress) {
            return prev;
          }
          const next = new Map(prev);
          next.set(payload.task.taskId, payload.task);
          return next;
        });
      });
    });

    return unsubscribe;
  }, []);

  const activeTask = useCallback((kind: TaskKind): TaskState | undefined => {
    return Array.from(tasks.values() as IterableIterator<TaskState>).find(
      (t: TaskState) => t.kind === kind && (t.status === 'running' || t.status === 'pending')
    );
  }, [tasks]);

  const cancelTask = useCallback(async (taskId: string): Promise<boolean> => {
    try { return await window.electron.taskCancel(taskId); } catch { return false; }
  }, []);

  return { tasks, activeTask, cancelTask };
}
