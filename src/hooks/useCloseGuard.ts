import { useEffect } from 'react';
import { appWindow, windowAction } from '../lib/window';
import { useApp } from '../stores/app';
import { TASK_KINDS, useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

/** Why closing the window now would interrupt something: a USB write, another operation, or nothing. */
export function closeRisk(): 'flash' | 'busy' | null {
  const locks = useWizard.getState().locks;
  const tasks = useTasks.getState();
  if (locks.includes('flash') || tasks.running(TASK_KINDS.flash)) return 'flash';
  if (locks.length > 0 || tasks.anyRunning()) return 'busy';
  return null;
}

/** Close the window, asking first while an operation runs. */
export function requestClose(): void {
  if (closeRisk()) useApp.getState().openCloseConfirm(true);
  else windowAction('close');
}

/** Close without asking again (the user confirmed). */
export function closeNow(): void {
  useApp.getState().openCloseConfirm(false);
  windowAction('destroy');
}

/**
 * Ask before the window closes while an operation runs, also for Alt+F4, the
 * taskbar or the system menu. While this listener exists the window only
 * closes through `destroy()` (capability `core:window:allow-destroy`).
 */
export function useCloseGuard(): void {
  useEffect(() => {
    const win = appWindow();
    if (!win) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    try {
      win
        .onCloseRequested((event) => {
          if (!closeRisk()) return;
          event.preventDefault();
          useApp.getState().openCloseConfirm(true);
        })
        .then((fn) => {
          if (disposed) fn();
          else unlisten = fn;
        })
        .catch(() => undefined);
    } catch {
      // Not inside the desktop runtime.
    }
    return () => {
      disposed = true;
      unlisten?.();
      unlisten = null;
    };
  }, []);
}
