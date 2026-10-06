import { create } from 'zustand';
import { api } from '../bridge/api';
import { toAppError, type AppError } from '../bridge/errors';
import type { AppVersionInfo, Catalog, PersistedState, UpdateInfo } from '../bridge/types';

const UPDATE_CACHE_KEY = 'oneclick.updateCheck';
const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

interface UpdateCache {
  at: number;
  info: UpdateInfo;
}

/** Last update check result, if it is recent enough to reuse (GitHub rate-limits anonymous API calls). */
export function readUpdateCache(now = Date.now()): UpdateInfo | null {
  try {
    const raw = window.localStorage.getItem(UPDATE_CACHE_KEY);
    if (!raw) return null;
    const cache = JSON.parse(raw) as Partial<UpdateCache>;
    if (typeof cache.at !== 'number' || !cache.info || typeof cache.info.current !== 'string') return null;
    return now - cache.at < UPDATE_CHECK_INTERVAL_MS ? cache.info : null;
  } catch {
    return null;
  }
}

function writeUpdateCache(info: UpdateInfo): void {
  try {
    const cache: UpdateCache = { at: Date.now(), info };
    window.localStorage.setItem(UPDATE_CACHE_KEY, JSON.stringify(cache));
  } catch {
    // Storage unavailable; the next start simply checks again.
  }
}

interface AppState {
  info: AppVersionInfo | null;
  catalog: Catalog | null;
  catalogError: AppError | null;
  catalogLoading: boolean;

  update: UpdateInfo | null;
  updateChecking: boolean;
  updateError: AppError | null;

  /** State saved by a previous session, offered for resume on the welcome screen. */
  persisted: PersistedState | null;

  settingsOpen: boolean;
  troubleshootOpen: boolean;
  /** "Close while an operation runs?" dialog. */
  closeConfirmOpen: boolean;

  initialized: boolean;
  init: () => Promise<void>;
  loadCatalog: () => Promise<void>;
  checkUpdates: () => Promise<void>;
  dismissPersisted: () => void;
  openSettings: (open: boolean) => void;
  openTroubleshoot: (open: boolean) => void;
  openCloseConfirm: (open: boolean) => void;
}

export const useApp = create<AppState>((set, get) => ({
  info: null,
  catalog: null,
  catalogError: null,
  catalogLoading: false,
  update: null,
  updateChecking: false,
  updateError: null,
  persisted: null,
  settingsOpen: false,
  troubleshootOpen: false,
  closeConfirmOpen: false,
  initialized: false,

  init: async () => {
    if (get().initialized) return;
    set({ initialized: true });

    const updates = async () => {
      const info = await api.getAppInfo().catch(() => null);
      if (info) set({ info });
      const cached = readUpdateCache();
      // A cached answer is only valid for the version that is running now.
      if (cached && info && cached.current === info.version) set({ update: cached });
      else await get().checkUpdates();
    };

    await Promise.all([
      updates(),
      get().loadCatalog(),
      api
        .getPersistedState()
        .then((persisted) => set({ persisted: persisted.profile ? persisted : null }), () => undefined),
    ]);
  },

  loadCatalog: async () => {
    if (get().catalogLoading) return;
    set({ catalogLoading: true, catalogError: null });
    try {
      set({ catalog: await api.getCatalog(), catalogLoading: false });
    } catch (err) {
      set({ catalogError: toAppError(err), catalogLoading: false });
    }
  },

  checkUpdates: async () => {
    if (get().updateChecking) return;
    set({ updateChecking: true, updateError: null });
    try {
      const update = await api.checkForUpdates();
      writeUpdateCache(update);
      set({ update, updateChecking: false });
    } catch (err) {
      set({ updateError: toAppError(err), updateChecking: false });
    }
  },

  dismissPersisted: () => set({ persisted: null }),
  openSettings: (open) => set({ settingsOpen: open }),
  openTroubleshoot: (open) => set({ troubleshootOpen: open }),
  openCloseConfirm: (open) => set({ closeConfirmOpen: open }),
}));
