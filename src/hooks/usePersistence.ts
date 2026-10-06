import { useEffect } from 'react';
import { api } from '../bridge/api';
import { stableStringify } from '../lib/stable';
import { useBuild } from '../stores/build';
import { useCompat } from '../stores/compat';
import { persistedSnapshot } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { useWizard } from '../stores/wizard';

const SAVE_DELAY_MS = 800;

/** Save profile, target, identity and EFI path with `save_state` whenever they change. */
export function usePersistence(): void {
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    let lastSaved = '';

    const schedule = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        const snapshot = persistedSnapshot();
        if (!snapshot) return;
        const key = stableStringify(snapshot);
        if (key === lastSaved) return;
        lastSaved = key;
        api.saveState(snapshot).catch(() => {
          lastSaved = '';
        });
      }, SAVE_DELAY_MS);
    };

    const unsubscribers = [
      useHardware.subscribe((s, prev) => {
        if (s.profile !== prev.profile) schedule();
      }),
      useCompat.subscribe((s, prev) => {
        if (s.target !== prev.target) schedule();
      }),
      useBuild.subscribe((s, prev) => {
        if (s.result !== prev.result || s.identity !== prev.identity) schedule();
      }),
      useWizard.subscribe((s, prev) => {
        if (s.step !== prev.step) schedule();
      }),
    ];

    return () => {
      if (timer) clearTimeout(timer);
      unsubscribers.forEach((u) => u());
    };
  }, []);
}
