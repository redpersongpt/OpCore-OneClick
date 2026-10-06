import { create } from 'zustand';
import { api } from '../bridge/api';
import { toAppError, type AppError } from '../bridge/errors';
import type { DetectedHardware, HardwareProfile } from '../bridge/types';
import { useWizard } from './wizard';

interface HardwareState {
  detected: DetectedHardware | null;
  profile: HardwareProfile | null;
  /** Profile was edited locally and not yet re-interpreted by `refresh_profile`. */
  dirty: boolean;
  isDemo: boolean;

  scanning: boolean;
  scanError: AppError | null;
  /** A scan was started at least once in this session (prevents auto-scan loops). */
  scanAttempted: boolean;

  refreshing: boolean;
  refreshError: AppError | null;
  ioError: AppError | null;

  /** Run `scan_hardware`. Resolves to true on success. Concurrent calls are ignored. */
  scan: () => Promise<boolean>;
  setProfile: (profile: HardwareProfile, detected: DetectedHardware | null, isDemo?: boolean) => void;
  editProfile: (update: (profile: HardwareProfile) => HardwareProfile) => void;
  /** Re-interpret the edited profile. Resolves to the new profile or null on failure. */
  refresh: () => Promise<HardwareProfile | null>;
  importFrom: (path: string) => Promise<HardwareProfile | null>;
  exportTo: (path: string) => Promise<boolean>;
  clear: () => void;
}

const initial = {
  detected: null,
  profile: null,
  dirty: false,
  isDemo: false,
  scanning: false,
  scanError: null,
  scanAttempted: false,
  refreshing: false,
  refreshError: null,
  ioError: null,
};

export const useHardware = create<HardwareState>((set, get) => ({
  ...initial,

  scan: async () => {
    if (get().scanning) return false;
    set({ scanning: true, scanError: null, scanAttempted: true });
    useWizard.getState().lock('scan');
    try {
      const result = await api.scanHardware();
      set({ detected: result.detected, profile: result.profile, dirty: false, isDemo: false, scanning: false });
      return true;
    } catch (err) {
      set({ scanError: toAppError(err), scanning: false });
      return false;
    } finally {
      useWizard.getState().unlock('scan');
    }
  },

  setProfile: (profile, detected, isDemo = false) =>
    set({ profile, detected, isDemo, dirty: false, scanError: null, refreshError: null, ioError: null }),

  editProfile: (update) => {
    const current = get().profile;
    if (!current) return;
    set({ profile: update(current), dirty: true });
  },

  refresh: async () => {
    const profile = get().profile;
    if (!profile || get().refreshing) return null;
    set({ refreshing: true, refreshError: null });
    try {
      const next = await api.refreshProfile(profile);
      if (get().profile !== profile) {
        // Edited again while the request ran: keep the newer edits (still unapplied).
        set({ refreshing: false });
        return null;
      }
      set({ profile: next, dirty: false, refreshing: false });
      return next;
    } catch (err) {
      set({ refreshError: toAppError(err), refreshing: false });
      return null;
    }
  },

  importFrom: async (path) => {
    set({ ioError: null });
    try {
      return await api.importProfile(path);
    } catch (err) {
      set({ ioError: toAppError(err) });
      return null;
    }
  },

  exportTo: async (path) => {
    const profile = get().profile;
    if (!profile) return false;
    set({ ioError: null });
    try {
      await api.exportProfile(profile, path);
      return true;
    } catch (err) {
      set({ ioError: toAppError(err) });
      return false;
    }
  },

  clear: () => set({ ...initial }),
}));
