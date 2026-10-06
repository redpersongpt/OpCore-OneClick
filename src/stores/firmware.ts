import { create } from 'zustand';
import { api } from '../bridge/api';
import { toAppError, type AppError } from '../bridge/errors';
import type { BiosSetting, FirmwareReport, HardwareProfile, MacOsVersion } from '../bridge/types';
import { compatKey } from './compat';

interface FirmwareState {
  settings: BiosSetting[] | null;
  settingsKey: string | null;
  settingsRequestKey: string | null;
  settingsLoading: boolean;
  settingsError: AppError | null;

  probe: FirmwareReport | null;
  probeAttempted: boolean;
  probeLoading: boolean;
  probeError: AppError | null;

  /** Checklist items the user ticked, by setting name. */
  checked: Record<string, boolean>;

  loadSettings: (profile: HardwareProfile, target: MacOsVersion) => Promise<void>;
  runProbe: () => Promise<void>;
  toggle: (name: string) => void;
  clearSettings: () => void;
  clear: () => void;
}

export const useFirmware = create<FirmwareState>((set, get) => ({
  settings: null,
  settingsKey: null,
  settingsRequestKey: null,
  settingsLoading: false,
  settingsError: null,
  probe: null,
  probeAttempted: false,
  probeLoading: false,
  probeError: null,
  checked: {},

  loadSettings: async (profile, target) => {
    const key = compatKey(profile, target);
    set({ settingsLoading: true, settingsError: null, settingsRequestKey: key });
    try {
      const settings = await api.getBiosSettings(profile, target);
      if (get().settingsRequestKey !== key) return;
      set({ settings, settingsKey: key, settingsLoading: false });
    } catch (err) {
      if (get().settingsRequestKey !== key) return;
      set({ settingsError: toAppError(err), settingsLoading: false });
    }
  },

  runProbe: async () => {
    if (get().probeLoading) return;
    set({ probeLoading: true, probeError: null, probeAttempted: true });
    try {
      const probe = await api.probeFirmware();
      set({ probe, probeLoading: false });
    } catch (err) {
      set({ probeError: toAppError(err), probeLoading: false });
    }
  },

  toggle: (name) => set((s) => ({ checked: { ...s.checked, [name]: !s.checked[name] } })),

  clearSettings: () =>
    set({ settings: null, settingsKey: null, settingsRequestKey: null, settingsLoading: false, settingsError: null }),

  clear: () =>
    set({
      settings: null,
      settingsKey: null,
      settingsRequestKey: null,
      settingsLoading: false,
      settingsError: null,
      probe: null,
      probeAttempted: false,
      probeLoading: false,
      probeError: null,
      checked: {},
    }),
}));
