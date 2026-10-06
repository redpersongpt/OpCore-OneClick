import { useEffect } from 'react';
import { api } from '../bridge/api';
import { EVENTS, subscribe } from '../bridge/events';
import { routeFlashProgress, routeRecoveryProgress, routeTaskUpdate } from '../stores/routing';

/** App-wide event subscriptions plus a one-time sync of tasks already running in the backend. */
export function useBackendEvents(): void {
  useEffect(() => {
    const disposers = [
      subscribe(EVENTS.taskUpdate, routeTaskUpdate),
      subscribe(EVENTS.flashProgress, routeFlashProgress),
      subscribe(EVENTS.recoveryProgress, routeRecoveryProgress),
    ];
    let cancelled = false;
    api.taskList().then(
      (tasks) => {
        if (!cancelled) tasks.forEach(routeTaskUpdate);
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
      disposers.forEach((dispose) => dispose());
    };
  }, []);
}
