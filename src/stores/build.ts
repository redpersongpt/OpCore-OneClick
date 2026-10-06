import { create } from 'zustand';
import { api } from '../bridge/api';
import { isCancellation, toAppError, type AppError } from '../bridge/errors';
import type {
  BuildOptions,
  BuildPlan,
  BuildResult,
  HardwareProfile,
  MacOsVersion,
  PlatformIdentity,
  TaskUpdate,
  ValidationResult,
} from '../bridge/types';
import { stableStringify } from '../lib/stable';
import { TASK_KINDS, useTasks } from './tasks';
import { useWizard } from './wizard';

/** User-editable build options; the target comes from the compatibility step. */
export interface OptionsDraft {
  smbiosOverride: string | null;
  extraBootArgs: string | null;
  verbose: boolean;
  debugOpencore: boolean;
  picker: BuildOptions['picker'];
  intelWifi: BuildOptions['intelWifi'];
  useLatestReleases: boolean;
  disableUnsupportedGpus: boolean;
  pickerTimeout: number | null;
  /** Reuse serial/MLB/UUID/ROM from the previous build (keeps iServices stable). */
  keepIdentity: boolean;
}

export const DEFAULT_DRAFT: OptionsDraft = {
  smbiosOverride: null,
  extraBootArgs: null,
  verbose: true,
  debugOpencore: false,
  picker: 'graphical',
  intelWifi: 'auto',
  useLatestReleases: false,
  disableUnsupportedGpus: true,
  pickerTimeout: null,
  keepIdentity: true,
};

/**
 * True when the previous serial/MLB/UUID/ROM can be reused: they are only
 * valid for the SMBIOS model they were generated for. `plannedModel` is the
 * planner's choice when no model is forced (null while the plan is unknown).
 */
export function canReuseIdentity(
  draft: OptionsDraft,
  identity: PlatformIdentity | null,
  plannedModel: string | null,
): identity is PlatformIdentity {
  if (!draft.keepIdentity || identity === null) return false;
  const model = draft.smbiosOverride ?? plannedModel;
  return model === null || model === identity.model;
}

export function toBuildOptions(
  draft: OptionsDraft,
  target: MacOsVersion,
  identity: PlatformIdentity | null,
  plannedModel: string | null = null,
): BuildOptions {
  const keep = canReuseIdentity(draft, identity, plannedModel);
  const extra = draft.extraBootArgs?.trim() ?? '';
  return {
    target,
    smbiosOverride: draft.smbiosOverride,
    extraBootArgs: extra ? extra : null,
    verbose: draft.verbose,
    debugOpencore: draft.debugOpencore,
    picker: draft.picker,
    intelWifi: draft.intelWifi,
    useLatestReleases: draft.useLatestReleases,
    identity: keep ? identity : null,
    disableUnsupportedGpus: draft.disableUnsupportedGpus,
    pickerTimeout: draft.pickerTimeout,
  };
}

/**
 * Key of the inputs that determine a plan or a build. The identity is left
 * out on purpose: reusing (or not) the previous serials never makes an
 * existing EFI stale.
 */
export function buildKey(profile: HardwareProfile | null, options: BuildOptions): string {
  return stableStringify({ profile, options: { ...options, identity: null } });
}

const MAX_LOG = 8;

interface BuildState {
  draft: OptionsDraft;
  /** Identity of the last build (or of a resumed session). */
  identity: PlatformIdentity | null;
  /** EFI path of a previous session, offered after resume. */
  previousEfiPath: string | null;

  plan: BuildPlan | null;
  planKey: string | null;
  planRequestKey: string | null;
  planLoading: boolean;
  planError: AppError | null;

  result: BuildResult | null;
  resultKey: string | null;
  building: boolean;
  taskId: string | null;
  progress: number | null;
  message: string | null;
  log: string[];
  error: AppError | null;
  cancelled: boolean;

  validation: ValidationResult | null;
  validating: boolean;
  validateError: AppError | null;

  setDraft: (patch: Partial<OptionsDraft>) => void;
  setIdentity: (identity: PlatformIdentity | null) => void;
  setPreviousEfiPath: (path: string | null) => void;
  loadPlan: (profile: HardwareProfile, options: BuildOptions) => Promise<void>;
  build: (profile: HardwareProfile, options: BuildOptions) => Promise<BuildResult | null>;
  cancel: () => Promise<void>;
  onTask: (update: TaskUpdate) => void;
  revalidate: () => Promise<void>;
  clearPlan: () => void;
  clearResult: () => void;
  clear: () => void;
}

let planSequence = 0;

const resultInitial = {
  result: null,
  resultKey: null,
  building: false,
  taskId: null,
  progress: null,
  message: null,
  log: [],
  error: null,
  cancelled: false,
  validation: null,
  validating: false,
  validateError: null,
};

const planInitial = {
  plan: null,
  planKey: null,
  planRequestKey: null,
  planLoading: false,
  planError: null,
};

export const useBuild = create<BuildState>((set, get) => ({
  draft: { ...DEFAULT_DRAFT },
  identity: null,
  previousEfiPath: null,
  ...planInitial,
  ...resultInitial,

  setDraft: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),
  setIdentity: (identity) => set({ identity }),
  setPreviousEfiPath: (path) => set({ previousEfiPath: path }),

  loadPlan: async (profile, options) => {
    const key = buildKey(profile, options);
    const ticket = ++planSequence;
    set({ planLoading: true, planError: null, planRequestKey: key });
    try {
      const plan = await api.planBuild(profile, options);
      if (ticket !== planSequence) return;
      set({ plan, planKey: key, planLoading: false });
    } catch (err) {
      if (ticket !== planSequence) return;
      set({ planError: toAppError(err), planLoading: false });
    }
  },

  build: async (profile, options) => {
    if (get().building) return null;
    const key = buildKey(profile, options);
    set({ ...resultInitial, building: true });
    useWizard.getState().lock('build');
    try {
      const result = await api.buildEfi(profile, options);
      set({
        result,
        resultKey: key,
        building: false,
        progress: 1,
        identity: result.identity,
        validation: result.validation,
      });
      return result;
    } catch (err) {
      const error = toAppError(err);
      set({ building: false, error, cancelled: get().cancelled || isCancellation(error) });
      return null;
    } finally {
      useWizard.getState().unlock('build');
    }
  },

  cancel: async () => {
    const { building, cancelled } = get();
    if (!building || cancelled) return;
    // The first task:update may not have arrived yet; fall back to the task list.
    const running = useTasks.getState().latest(TASK_KINDS.build);
    const taskId = get().taskId ?? (running?.status === 'running' ? running.taskId : null);
    if (!taskId) return;
    set({ cancelled: true, taskId });
    try {
      await api.taskCancel(taskId);
    } catch {
      // The build may have finished in the meantime.
    }
  },

  onTask: (update) => {
    if (update.kind !== TASK_KINDS.build) return;
    const state = get();
    if (!state.building) return;
    if (state.taskId && state.taskId !== update.taskId) return;
    const message = update.message?.trim() || null;
    const log =
      message && state.log[state.log.length - 1] !== message ? [...state.log, message].slice(-MAX_LOG) : state.log;
    set({
      taskId: update.taskId,
      progress: update.progress ?? state.progress,
      message: message ?? state.message,
      log,
      cancelled: state.cancelled || update.status === 'cancelled',
    });
  },

  revalidate: async () => {
    const result = get().result;
    if (!result || get().validating) return;
    set({ validating: true, validateError: null });
    try {
      const validation = await api.validateEfi(result.efiPath);
      set({ validation, validating: false });
    } catch (err) {
      set({ validateError: toAppError(err), validating: false });
    }
  },

  clearPlan: () => {
    planSequence += 1;
    set({ ...planInitial });
  },

  clearResult: () => {
    if (get().building) return;
    set({ ...resultInitial });
  },

  clear: () => {
    planSequence += 1;
    set({ draft: { ...DEFAULT_DRAFT }, identity: null, previousEfiPath: null, ...planInitial, ...resultInitial });
  },
}));
