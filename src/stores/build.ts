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
import {
  parseBuildDetail,
  recordItem,
  type BuildDetail,
  type BuildItems,
  type BuildPhase,
} from '../lib/buildProgress';
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
  /** macOS 26: prepare for restoring analog audio after install (lowers SIP). */
  prepareAudioPatch: boolean;
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
  prepareAudioPatch: false,
};

const LATEST_KEY = 'oneclick.useLatestReleases';

/** The "use latest releases" choice is a preference: it survives restarts and "start over". */
export function storedUseLatest(): boolean {
  try {
    return window.localStorage.getItem(LATEST_KEY) === 'true';
  } catch {
    return false;
  }
}

function storeUseLatest(value: boolean): void {
  try {
    if (value) window.localStorage.setItem(LATEST_KEY, 'true');
    else window.localStorage.removeItem(LATEST_KEY);
  } catch {
    // Not persisted; the choice still applies to this session.
  }
}

function initialDraft(): OptionsDraft {
  return { ...DEFAULT_DRAFT, useLatestReleases: storedUseLatest() };
}

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
    prepareAudioPatch: target === '26' && draft.prepareAudioPatch,
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
  /** Latest structured progress of the running (or last) build. */
  detail: BuildDetail | null;
  /** Phases reported so far, in order. */
  phasesSeen: BuildPhase[];
  items: BuildItems;
  error: AppError | null;
  cancelled: boolean;
  /** The backend refused the cancel (the build was already being saved). */
  cancelRefused: boolean;

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
  detail: null,
  phasesSeen: [],
  items: {},
  error: null,
  cancelled: false,
  cancelRefused: false,
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
  draft: initialDraft(),
  identity: null,
  previousEfiPath: null,
  ...planInitial,
  ...resultInitial,

  setDraft: (patch) => {
    if (patch.useLatestReleases !== undefined) storeUseLatest(patch.useLatestReleases);
    set((s) => ({ draft: { ...s.draft, ...patch } }));
  },
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
    const { building, cancelled, cancelRefused } = get();
    if (!building || cancelled || cancelRefused) return;
    // The first task:update may not have arrived yet; fall back to the task list.
    const taskId = get().taskId ?? useTasks.getState().running(TASK_KINDS.build)?.taskId ?? null;
    if (!taskId) return;
    // The task bar may have sent the cancel already; follow its outcome instead of asking twice.
    const earlier = useTasks.getState().cancels[taskId];
    if (earlier) {
      set({ taskId, cancelled: earlier === 'requested', cancelRefused: earlier === 'refused' });
      return;
    }
    set({ cancelled: true, taskId });
    const accepted = await useTasks.getState().cancel(taskId);
    // Refused: the build is being saved and finishes on its own.
    if (!accepted && get().building) set({ cancelled: false, cancelRefused: true });
  },

  onTask: (update) => {
    if (update.kind !== TASK_KINDS.build) return;
    const state = get();
    if (!state.building) return;
    if (state.taskId && state.taskId !== update.taskId) return;
    const message = update.message?.trim() || null;
    const detail = parseBuildDetail(update.detail);
    set({
      taskId: update.taskId,
      progress: update.progress ?? state.progress,
      message: message ?? state.message,
      detail: detail ?? state.detail,
      phasesSeen: detail && !state.phasesSeen.includes(detail.phase) ? [...state.phasesSeen, detail.phase] : state.phasesSeen,
      items: detail ? recordItem(state.items, detail) : state.items,
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
    set({ draft: initialDraft(), identity: null, previousEfiPath: null, ...planInitial, ...resultInitial });
  },
}));
