import { getCurrentWindow, type Window } from '@tauri-apps/api/window';

/** The app window, or null outside the desktop runtime (browser preview, tests). */
export function appWindow(): Window | null {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

export function windowAction(action: 'minimize' | 'toggleMaximize' | 'close' | 'destroy'): void {
  const win = appWindow();
  if (!win) return;
  try {
    void win[action]().catch(() => undefined);
  } catch {
    // The IPC bridge is missing.
  }
}
