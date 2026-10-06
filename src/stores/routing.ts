import { normalizeFlashProgress, normalizeRecoveryProgress, normalizeTaskUpdate } from '../bridge/events';
import { useBuild } from './build';
import { useDeploy } from './deploy';
import { useTasks } from './tasks';

/** `task:update` → task list, build progress, deploy task ids. Invalid payloads are ignored. */
export function routeTaskUpdate(raw: unknown): void {
  const update = normalizeTaskUpdate(raw);
  if (!update) return;
  useTasks.getState().apply(update);
  useBuild.getState().onTask(update);
  useDeploy.getState().onTask(update);
}

export function routeFlashProgress(raw: unknown): void {
  const progress = normalizeFlashProgress(raw);
  if (progress) useDeploy.getState().onFlashProgress(progress);
}

export function routeRecoveryProgress(raw: unknown): void {
  const progress = normalizeRecoveryProgress(raw);
  if (progress) useDeploy.getState().onRecoveryProgress(progress);
}
