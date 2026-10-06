import { create } from 'zustand';
import { api } from '../bridge/api';
import { isCancellation, toAppError, type AppError } from '../bridge/errors';
import type {
  DiskInfo,
  FlashConfirmation,
  FlashProgress,
  MacOsVersion,
  PrivilegeStatus,
  RecoveryCacheInfo,
  RecoveryProgress,
  TaskUpdate,
} from '../bridge/types';
import { TASK_KINDS, useTasks } from './tasks';
import { useWizard } from './wizard';

export type FlashStatus = 'idle' | 'running' | 'done' | 'failed';

interface DeployState {
  privileges: PrivilegeStatus | null;
  privilegesError: AppError | null;

  disks: DiskInfo[];
  disksLoaded: boolean;
  disksLoading: boolean;
  disksError: AppError | null;
  selected: string | null;

  includeRecovery: boolean;
  recoveryInfo: RecoveryCacheInfo | null;
  recoveryInfoFor: MacOsVersion | null;
  recoveryInfoError: AppError | null;
  recoveryDownloading: boolean;
  recoveryFor: MacOsVersion | null;
  recoveryTaskId: string | null;
  recoveryProgress: RecoveryProgress | null;
  recoveryError: AppError | null;

  confirmation: FlashConfirmation | null;
  preparing: boolean;
  prepareError: AppError | null;

  flashStatus: FlashStatus;
  flashTaskId: string | null;
  flashProgress: FlashProgress | null;
  /** Phases seen during the current flash, in order. */
  flashPhases: string[];
  flashError: AppError | null;
  /** The USB drive holds the current EFI (written successfully and not invalidated since). */
  flashed: boolean;

  loadPrivileges: () => Promise<void>;
  refreshDisks: () => Promise<void>;
  select: (device: string | null) => void;
  setIncludeRecovery: (value: boolean) => void;
  loadRecoveryInfo: (version: MacOsVersion) => Promise<void>;
  downloadRecovery: (version: MacOsVersion) => Promise<void>;
  cancelRecovery: () => Promise<void>;
  onRecoveryProgress: (progress: RecoveryProgress) => void;
  prepare: (efiPath: string, recovery: MacOsVersion | null) => Promise<FlashConfirmation | null>;
  dismissConfirmation: () => void;
  /** Spend the confirmation token; the recovery choice is the one the token was issued for. */
  flash: (efiPath: string) => Promise<boolean>;
  onFlashProgress: (progress: FlashProgress) => void;
  onTask: (update: TaskUpdate) => void;
  resetFlash: () => void;
  resetRecovery: () => void;
  clear: () => void;
}

const flashInitial = {
  confirmation: null,
  preparing: false,
  prepareError: null,
  flashStatus: 'idle' as FlashStatus,
  flashTaskId: null,
  flashProgress: null,
  flashPhases: [],
  flashError: null,
  flashed: false,
};

const recoveryInitial = {
  recoveryInfo: null,
  recoveryInfoFor: null,
  recoveryInfoError: null,
  recoveryDownloading: false,
  recoveryFor: null,
  recoveryTaskId: null,
  recoveryProgress: null,
  recoveryError: null,
};

export const useDeploy = create<DeployState>((set, get) => ({
  privileges: null,
  privilegesError: null,
  disks: [],
  disksLoaded: false,
  disksLoading: false,
  disksError: null,
  selected: null,
  includeRecovery: true,
  ...recoveryInitial,
  ...flashInitial,

  loadPrivileges: async () => {
    try {
      set({ privileges: await api.checkPrivileges(), privilegesError: null });
    } catch (err) {
      set({ privilegesError: toAppError(err) });
    }
  },

  refreshDisks: async () => {
    if (get().disksLoading) return;
    set({ disksLoading: true, disksError: null });
    try {
      const disks = await api.listUsbDevices();
      const selected = get().selected;
      set({
        disks,
        disksLoaded: true,
        disksLoading: false,
        selected: selected && disks.some((d) => d.devicePath === selected) ? selected : null,
      });
    } catch (err) {
      set({ disksError: toAppError(err), disksLoading: false, disksLoaded: true });
    }
  },

  select: (device) => set({ selected: device, confirmation: null, prepareError: null }),

  setIncludeRecovery: (value) => set({ includeRecovery: value, confirmation: null }),

  loadRecoveryInfo: async (version) => {
    set({ recoveryInfoFor: version, recoveryInfoError: null });
    try {
      const info = await api.getCachedRecoveryInfo(version);
      if (get().recoveryInfoFor === version) set({ recoveryInfo: info });
    } catch (err) {
      if (get().recoveryInfoFor === version) set({ recoveryInfoError: toAppError(err) });
    }
  },

  downloadRecovery: async (version) => {
    if (get().recoveryDownloading) return;
    set({
      recoveryDownloading: true,
      recoveryFor: version,
      recoveryTaskId: null,
      recoveryProgress: null,
      recoveryError: null,
    });
    useWizard.getState().lock('recovery');
    try {
      const info = await api.downloadRecovery(version);
      set({ recoveryInfo: info, recoveryInfoFor: version, recoveryDownloading: false });
    } catch (err) {
      const error = toAppError(err);
      set({ recoveryDownloading: false, recoveryError: isCancellation(error) ? null : error });
    } finally {
      useWizard.getState().unlock('recovery');
    }
  },

  cancelRecovery: async () => {
    if (!get().recoveryDownloading) return;
    const taskId = get().recoveryTaskId ?? useTasks.getState().running(TASK_KINDS.recovery)?.taskId ?? null;
    if (!taskId) return;
    await useTasks.getState().cancel(taskId);
  },

  onRecoveryProgress: (progress) => {
    const state = get();
    if (!state.recoveryDownloading || progress.version !== state.recoveryFor) return;
    set({ recoveryProgress: progress, recoveryTaskId: progress.taskId });
  },

  prepare: async (efiPath, recovery) => {
    const device = get().selected;
    if (!device || get().preparing) return null;
    set({ preparing: true, prepareError: null, confirmation: null });
    try {
      const confirmation = await api.flashPrepareConfirmation(device, efiPath, recovery);
      set({ confirmation, preparing: false });
      return confirmation;
    } catch (err) {
      set({ prepareError: toAppError(err), preparing: false });
      return null;
    }
  },

  dismissConfirmation: () => set({ confirmation: null }),

  flash: async (efiPath) => {
    const { confirmation, flashStatus } = get();
    if (!confirmation || flashStatus === 'running') return false;
    set({
      flashStatus: 'running',
      flashTaskId: null,
      flashProgress: null,
      flashPhases: [],
      flashError: null,
      confirmation: null,
      // Whatever the drive held before is being erased.
      flashed: false,
    });
    useWizard.getState().lock('flash');
    try {
      await api.flashUsb(confirmation.device, efiPath, confirmation.token, confirmation.recovery);
      set({ flashStatus: 'done', flashed: true });
      return true;
    } catch (err) {
      set({ flashStatus: 'failed', flashError: toAppError(err) });
      return false;
    } finally {
      useWizard.getState().unlock('flash');
    }
  },

  onFlashProgress: (progress) => {
    const state = get();
    if (state.flashStatus !== 'running') return;
    if (state.flashTaskId && state.flashTaskId !== progress.taskId) return;
    const phases = state.flashPhases.includes(progress.phase)
      ? state.flashPhases
      : [...state.flashPhases, progress.phase];
    set({ flashProgress: progress, flashTaskId: progress.taskId, flashPhases: phases });
  },

  onTask: (update) => {
    const state = get();
    if (update.kind === TASK_KINDS.recovery && state.recoveryDownloading && !state.recoveryTaskId) {
      set({ recoveryTaskId: update.taskId });
    }
    if (update.kind === TASK_KINDS.flash && state.flashStatus === 'running' && !state.flashTaskId) {
      set({ flashTaskId: update.taskId });
    }
  },

  resetFlash: () => {
    if (get().flashStatus === 'running') return;
    set({ ...flashInitial });
  },

  resetRecovery: () => {
    if (get().recoveryDownloading) return;
    set({ ...recoveryInitial });
  },

  clear: () => {
    if (get().flashStatus === 'running' || get().recoveryDownloading) return;
    set({
      privileges: null,
      privilegesError: null,
      disks: [],
      disksLoaded: false,
      disksLoading: false,
      disksError: null,
      selected: null,
      includeRecovery: true,
      ...recoveryInitial,
      ...flashInitial,
    });
  },
}));
