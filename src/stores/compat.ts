import { create } from 'zustand';
import { api } from '../bridge/api';
import { toAppError, type AppError } from '../bridge/errors';
import type { CompatibilityReport, HardwareProfile, MacOsVersion } from '../bridge/types';
import { defaultTarget } from '../lib/compat';
import { profileKey } from '../lib/profile';

export function compatKey(profile: HardwareProfile | null, target: MacOsVersion | null): string {
  return `${profileKey(profile)}|${target ?? 'auto'}`;
}

interface CompatState {
  report: CompatibilityReport | null;
  /** compatKey of the inputs `report` was computed for. */
  reportKey: string | null;
  /** compatKey of the most recent request (successful or not); guards auto-fetch loops. */
  requestKey: string | null;
  target: MacOsVersion | null;
  /** Target for which the user accepted the expert override. */
  expertFor: MacOsVersion | null;
  loading: boolean;
  error: AppError | null;

  check: (profile: HardwareProfile, target: MacOsVersion | null) => Promise<void>;
  setTarget: (target: MacOsVersion | null) => void;
  setExpert: (accepted: boolean) => void;
  clearReport: () => void;
  clear: () => void;
}

let sequence = 0;

export const useCompat = create<CompatState>((set, get) => ({
  report: null,
  reportKey: null,
  requestKey: null,
  target: null,
  expertFor: null,
  loading: false,
  error: null,

  check: async (profile, target) => {
    const key = compatKey(profile, target);
    const ticket = ++sequence;
    set({ loading: true, error: null, requestKey: key });
    try {
      const report = await api.checkCompatibility(profile, target);
      if (ticket !== sequence) return;
      // Without an explicit target the backend evaluates its recommended one;
      // adopt it so the report is keyed by the target it actually describes.
      // If the preselected release is not the evaluated one, the report stays
      // keyed as "automatic" so the page checks the preselected release again.
      const chosen = target ?? defaultTarget(report);
      const describes = target ?? (report.target === chosen ? chosen : null);
      const effectiveKey = compatKey(profile, describes);
      set({
        report,
        reportKey: effectiveKey,
        requestKey: effectiveKey,
        target: chosen,
        loading: false,
      });
    } catch (err) {
      if (ticket !== sequence) return;
      set({ error: toAppError(err), loading: false });
    }
  },

  setTarget: (target) => set({ target, expertFor: null }),

  setExpert: (accepted) => set({ expertFor: accepted ? get().target : null }),

  clearReport: () => {
    sequence += 1;
    set({ report: null, reportKey: null, requestKey: null, loading: false, error: null, expertFor: null });
  },

  clear: () => {
    sequence += 1;
    set({ report: null, reportKey: null, requestKey: null, target: null, expertFor: null, loading: false, error: null });
  },
}));
