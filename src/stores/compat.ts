import { create } from 'zustand';
import { api } from '../bridge/api';
import { toAppError, type AppError } from '../bridge/errors';
import type { CompatibilityReport, HardwareProfile, MacOsVersion } from '../bridge/types';
import { defaultTarget, findOption, reachOf, type Reach } from '../lib/compat';
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
  /** For releases the report does not support: expert option or out of reach. */
  reach: Partial<Record<MacOsVersion, Reach>>;
  /** profileKey `reach` was computed for. */
  reachKey: string | null;
  loading: boolean;
  error: AppError | null;

  check: (profile: HardwareProfile, target: MacOsVersion | null) => Promise<void>;
  /** Evaluate every unsupported release once to tell expert options from dead ends. */
  classify: (profile: HardwareProfile) => Promise<void>;
  setTarget: (target: MacOsVersion | null) => void;
  setExpert: (accepted: boolean) => void;
  clearReport: () => void;
  clear: () => void;
}

let sequence = 0;
/** Bumped whenever the profile (and so every classification) changes. */
let reachGeneration = 0;
const inflight = new Set<string>();

export const useCompat = create<CompatState>((set, get) => ({
  report: null,
  reportKey: null,
  requestKey: null,
  target: null,
  expertFor: null,
  reach: {},
  reachKey: null,
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
      // A report for an unsupported release also says whether it is an expert option.
      const evaluated = report.target;
      const option = findOption(report, evaluated);
      const pKey = profileKey(profile);
      const reach =
        evaluated && option && !option.supported
          ? { ...(get().reachKey === pKey ? get().reach : {}), [evaluated]: reachOf(report) }
          : get().reachKey === pKey
            ? get().reach
            : {};
      set({
        report,
        reportKey: effectiveKey,
        requestKey: effectiveKey,
        target: chosen,
        reach,
        reachKey: pKey,
        loading: false,
      });
    } catch (err) {
      if (ticket !== sequence) return;
      set({ error: toAppError(err), loading: false });
    }
  },

  classify: async (profile) => {
    const report = get().report;
    if (!report) return;
    const pKey = profileKey(profile);
    const generation = reachGeneration;
    const known = get().reachKey === pKey ? get().reach : {};
    const pending = report.versions
      .map((v) => v.version)
      .filter((v) => !findOption(report, v)?.supported && known[v] === undefined && !inflight.has(`${generation}|${v}`));
    if (pending.length === 0) return;
    pending.forEach((v) => inflight.add(`${generation}|${v}`));
    try {
      const results = await Promise.all(
        pending.map((version) =>
          api.checkCompatibility(profile, version).then(
            (r) => [version, reachOf(r)] as const,
            () => null,
          ),
        ),
      );
      // A new profile clears the report and starts a new generation: drop stale answers.
      if (generation !== reachGeneration) return;
      const reach = { ...(get().reachKey === pKey ? get().reach : {}) };
      for (const entry of results) if (entry) reach[entry[0]] = entry[1];
      set({ reach, reachKey: pKey });
    } finally {
      pending.forEach((v) => inflight.delete(`${generation}|${v}`));
    }
  },

  setTarget: (target) => set({ target, expertFor: null }),

  setExpert: (accepted) => set({ expertFor: accepted ? get().target : null }),

  clearReport: () => {
    sequence += 1;
    reachGeneration += 1;
    set({ report: null, reportKey: null, requestKey: null, loading: false, error: null, expertFor: null, reach: {}, reachKey: null });
  },

  clear: () => {
    sequence += 1;
    reachGeneration += 1;
    set({
      report: null,
      reportKey: null,
      requestKey: null,
      target: null,
      expertFor: null,
      reach: {},
      reachKey: null,
      loading: false,
      error: null,
    });
  },
}));
