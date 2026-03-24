import { ArrowUpRight, CheckCircle2, Download, Loader2, RefreshCcw, RotateCw } from 'lucide-react';
import type { AppUpdateState } from '../../electron/appUpdater';

interface UpdaterPanelProps {
  state: AppUpdateState | null;
  onRefresh: () => void;
  onPrimaryAction: () => void;
  onOpenRelease: () => void;
}

function formatRefreshTimestamp(timestamp: number | null | undefined): string | null {
  if (!timestamp) return null;
  const deltaMs = Date.now() - timestamp;
  if (deltaMs < 15_000) return 'Just now';
  if (deltaMs < 60_000) return 'Less than a minute ago';
  return new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function buildPrimaryAction(state: AppUpdateState | null): {
  label: string;
  disabled: boolean;
  icon: 'restart' | 'download' | 'none';
} {
  if (state?.installing) {
    return { label: 'Closing to install…', icon: 'restart', disabled: true };
  }
  if (state?.restartRequired) {
    return { label: 'Restart now', icon: 'restart', disabled: false };
  }
  if (state?.readyToInstall) {
    return {
      label: state.installing ? 'Installing…' : 'Install update',
      icon: 'download',
      disabled: !!state.installing || !!state.checking,
    };
  }
  if (state?.available) {
    return {
      label: state.downloading ? 'Downloading…' : 'Download update',
      icon: 'download',
      disabled: !!state.downloading || !!state.installing || !!state.checking || !state.supported,
    };
  }
  return { label: 'Up to date', icon: 'none', disabled: true };
}

export default function UpdaterPanel({
  state,
  onRefresh,
  onPrimaryAction,
  onOpenRelease,
}: UpdaterPanelProps) {
  const progressPercent = state?.totalBytes && state.totalBytes > 0
    ? Math.min(100, Math.round((state.downloadedBytes / state.totalBytes) * 100))
    : state?.readyToInstall || state?.restartRequired || state?.installing ? 100 : 0;

  const showProgress = !!(state?.downloading || state?.readyToInstall || state?.restartRequired || state?.installing);
  const refreshBusy = !!(state?.checking || state?.downloading || state?.installing);

  const headline = state?.installing
    ? 'Installing update…'
    : state?.restartRequired
    ? 'Update ready — restart to finish'
    : state?.checking
    ? 'Checking for updates…'
    : state?.downloading
    ? `Downloading ${state.latestVersion ?? 'update'}…`
    : state?.readyToInstall
    ? `${state.latestVersion ?? 'Update'} ready to install`
    : state?.available
    ? `${state.latestVersion ?? 'Update'} available`
    : 'Up to date';

  const detail = state?.installing
    ? 'The app will close and reopen automatically.'
    : state?.restartRequired
    ? `${state.latestVersion ?? 'Update'} is downloaded and ready. Restart to finish installing.`
    : state?.error
    ? state.error
    : state?.available
    ? state.assetName ?? 'New version available'
    : `v${state?.currentVersion ?? '?'}`;

  const primaryAction = buildPrimaryAction(state);
  const showPrimary = !!(state?.available || state?.readyToInstall || state?.restartRequired || state?.installing);
  const checkedAt = formatRefreshTimestamp(state?.lastCheckedAt);

  return (
    <div className="w-full min-w-0 rounded-2xl border border-white/10 bg-white/[0.04] px-4 py-3.5 text-left backdrop-blur-md">
      {/* Header row: headline + refresh */}
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {state?.checking ? (
              <Loader2 className="h-3.5 w-3.5 flex-shrink-0 animate-spin text-blue-300" />
            ) : state?.installing || state?.restartRequired ? (
              <RotateCw className="h-3.5 w-3.5 flex-shrink-0 text-emerald-400" />
            ) : showProgress ? (
              <Download className="h-3.5 w-3.5 flex-shrink-0 text-blue-300" />
            ) : state?.available ? (
              <Download className="h-3.5 w-3.5 flex-shrink-0 text-white/60" />
            ) : (
              <CheckCircle2 className="h-3.5 w-3.5 flex-shrink-0 text-white/30" />
            )}
            <span className="min-w-0 truncate text-sm font-semibold text-white">
              {headline}
            </span>
          </div>
          <p className="mt-0.5 min-w-0 truncate text-xs text-white/40 pl-[22px]">
            {detail}
          </p>
        </div>
        <button
          onClick={onRefresh}
          disabled={refreshBusy}
          title="Check for updates"
          className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg border border-white/8 bg-white/[0.04] text-white/50 transition-colors hover:bg-white/[0.08] hover:text-white disabled:cursor-not-allowed disabled:opacity-40"
        >
          <RefreshCcw className={`h-3.5 w-3.5 ${state?.checking ? 'animate-spin' : ''}`} />
        </button>
      </div>

      {/* Progress bar */}
      {showProgress && (
        <div className="mt-2.5 pl-[22px]">
          <div className="flex items-center gap-3">
            <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/8">
              <div
                className="h-full rounded-full bg-white/80 transition-[width] duration-300"
                style={{ width: `${progressPercent}%` }}
              />
            </div>
            <span className="flex-shrink-0 text-[11px] tabular-nums text-white/35">
              {progressPercent}%
            </span>
          </div>
        </div>
      )}

      {/* Actions */}
      {(showPrimary || state?.error) && (
        <div className="mt-2.5 flex items-center gap-2 pl-[22px]">
          {showPrimary && (
            <button
              onClick={onPrimaryAction}
              disabled={primaryAction.disabled}
              className="inline-flex min-w-0 items-center gap-1.5 rounded-lg bg-white px-3.5 py-1.5 text-xs font-bold text-black transition-transform hover:scale-[1.02] active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50"
            >
              {primaryAction.icon === 'restart' && <RotateCw className="h-3 w-3 flex-shrink-0" />}
              {primaryAction.icon === 'download' && <Download className="h-3 w-3 flex-shrink-0" />}
              {primaryAction.label}
            </button>
          )}
          <button
            onClick={onOpenRelease}
            className="inline-flex min-w-0 items-center gap-1 rounded-lg border border-white/8 bg-white/[0.04] px-3 py-1.5 text-xs font-medium text-white/50 transition-colors hover:bg-white/[0.08] hover:text-white"
          >
            Release
            <ArrowUpRight className="h-3 w-3 flex-shrink-0 text-white/30" />
          </button>
        </div>
      )}

      {/* Footer: last checked */}
      {checkedAt && !state?.checking && (
        <div className="mt-2 pl-[22px] text-[10px] text-white/25">
          Checked {checkedAt}
        </div>
      )}
    </div>
  );
}
