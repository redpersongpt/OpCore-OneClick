import { ArrowUpRight, CheckCircle2, Download, Loader2, RefreshCcw } from 'lucide-react';
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
  if (deltaMs < 15_000) return 'Checked just now';
  if (deltaMs < 60_000) return 'Checked less than a minute ago';
  return `Checked at ${new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
}

function buildPrimaryAction(state: AppUpdateState | null): { label: string; disabled: boolean } {
  if (state?.restartRequired) {
    return { label: 'Restart app', disabled: false };
  }
  if (state?.readyToInstall) {
    return { label: state.installing ? 'Preparing…' : 'Install update', disabled: !!state.installing || !!state.checking };
  }
  if (state?.available) {
    return {
      label: state.downloading ? 'Downloading…' : 'Download update',
      disabled: !!state.downloading || !!state.installing || !!state.checking || !state.supported,
    };
  }
  return {
    label: state?.checking ? 'Refreshing…' : 'No update available',
    disabled: true,
  };
}

export default function UpdaterPanel({
  state,
  onRefresh,
  onPrimaryAction,
  onOpenRelease,
}: UpdaterPanelProps) {
  const progressPercent = state?.totalBytes && state.totalBytes > 0
    ? Math.min(100, Math.round((state.downloadedBytes / state.totalBytes) * 100))
    : state?.readyToInstall || state?.restartRequired
    ? 100
    : 0;
  const refreshLabel = state?.checking
    ? 'Refreshing now'
    : formatRefreshTimestamp(state?.lastCheckedAt)
      ?? 'Check for the latest release and installer.';
  const headline = state?.restartRequired
    ? 'Set up complete - please restart the app'
    : state?.checking
    ? 'Refreshing update status…'
    : state?.downloading
    ? 'Downloading update…'
    : state?.readyToInstall
    ? 'Update ready to finish'
    : state?.available
    ? `${state.latestVersion} is ready`
    : "You're up to date";
  const detail = state?.restartRequired
    ? `The updated files are staged. Restart the app to finish loading ${state.latestVersion ?? 'the latest version'}.`
    : state?.checking
    ? 'Checking the latest published release and installer availability for this platform.'
    : state?.readyToInstall
    ? 'Install the downloaded update, then restart the app once setup finishes.'
    : state?.available
    ? `Download ${state.assetName ?? 'the latest build'} directly in the app.`
    : state?.error
    ? state.error
    : `Current version: ${state?.currentVersion ?? 'unknown'}`;
  const primaryAction = buildPrimaryAction(state);

  return (
    <div className="w-full min-w-0 rounded-[1.6rem] border border-white/10 bg-white/[0.04] p-5 text-left backdrop-blur-md">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 flex-1 space-y-3">
          <div className="text-[11px] font-bold uppercase tracking-[0.28em] text-white/35">Updater</div>
          <div className="min-w-0 text-lg font-bold text-white break-words">
            {headline}
          </div>
          <p className="min-w-0 text-sm leading-relaxed text-white/50 break-words">
            {detail}
          </p>
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className={`inline-flex min-w-0 items-center gap-2 rounded-full border px-3 py-1 font-semibold ${
              state?.checking
                ? 'border-blue-400/20 bg-blue-500/10 text-blue-100'
                : 'border-white/10 bg-white/5 text-white/55'
            }`}>
              {state?.checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CheckCircle2 className="h-3.5 w-3.5" />}
              <span className="min-w-0 break-words">{refreshLabel}</span>
            </span>
            {state?.assetName && (
              <span className="min-w-0 break-words rounded-full border border-white/10 bg-white/5 px-3 py-1 font-medium text-white/45">
                Asset: {state.assetName}
              </span>
            )}
          </div>
        </div>

        <button
          onClick={onRefresh}
          disabled={state?.checking || state?.downloading || state?.installing}
          className="inline-flex w-full min-w-0 items-center justify-center gap-2 rounded-2xl border border-blue-400/25 bg-blue-500/10 px-4 py-3 text-sm font-semibold text-blue-100 transition-all hover:bg-blue-500/16 hover:text-white disabled:cursor-not-allowed disabled:opacity-60 lg:w-auto"
        >
          <RefreshCcw className={`h-4 w-4 ${state?.checking ? 'animate-spin' : ''}`} />
          <span className="min-w-0 break-words text-center">
            {state?.checking ? 'Refreshing…' : 'Refresh'}
          </span>
        </button>
      </div>

      {(state?.downloading || state?.readyToInstall || state?.restartRequired) && (
        <div className="mt-4">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-white/45">
            <span className="min-w-0 break-words">
              {state.restartRequired
                ? 'Set up complete'
                : state.downloading
                ? 'Downloading update…'
                : 'Download complete'}
            </span>
            <span>{progressPercent > 0 ? `${progressPercent}%` : '—'}</span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-white/8">
            <div
              className="h-full rounded-full bg-white transition-all"
              style={{ width: `${progressPercent}%` }}
            />
          </div>
        </div>
      )}

      <div className="mt-4 flex flex-col gap-3 sm:flex-row">
        <button
          onClick={onPrimaryAction}
          disabled={primaryAction.disabled}
          className="inline-flex w-full min-w-0 items-center justify-center gap-2 rounded-2xl bg-white px-5 py-3 text-sm font-bold text-black transition-transform hover:scale-[1.02] active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-60 sm:flex-1"
        >
          <Download className="h-4 w-4 flex-shrink-0" />
          <span className="min-w-0 break-words text-center leading-tight">
            {primaryAction.label}
          </span>
        </button>
        <button
          onClick={onOpenRelease}
          className="inline-flex w-full min-w-0 items-center justify-center gap-2 rounded-2xl border border-white/10 bg-white/5 px-4 py-3 text-sm font-semibold text-white/70 transition-colors hover:bg-white/10 hover:text-white sm:flex-1"
        >
          <span className="min-w-0 break-words text-center leading-tight">View release</span>
          <ArrowUpRight className="h-4 w-4 flex-shrink-0 text-white/35" />
        </button>
      </div>
    </div>
  );
}
