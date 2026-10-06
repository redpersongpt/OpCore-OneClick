import { Ban, CheckCircle2, Circle, Loader2, MinusCircle, XCircle } from 'lucide-react';
import { useT } from '../../i18n';
import {
  BUILD_PHASES,
  buildPhaseLabel,
  itemDone,
  phaseStates,
  type BuildDetail,
  type BuildItems,
  type BuildPhase,
  type PhaseState,
} from '../../lib/buildProgress';
import { formatBytes, formatPercent } from '../../lib/format';
import { Progress } from '../ui/Progress';

export type BuildOutcome = 'running' | 'done' | 'failed' | 'cancelled';

/** Phases whose items are listed one by one (per-kext progress). */
const ITEMIZED: readonly BuildPhase[] = ['kexts'];

/**
 * Phase list of an EFI build with the current item, its download size and
 * the kext packages fetched so far. Fed from `TaskUpdate.detail`.
 */
export function BuildProgressView({
  detail,
  seen,
  items,
  progress,
  message,
  outcome,
}: {
  detail: BuildDetail | null;
  seen: readonly BuildPhase[];
  items: BuildItems;
  progress: number | null;
  message: string | null;
  outcome: BuildOutcome;
}) {
  const t = useT();
  const states = phaseStates(detail?.phase ?? null, seen, outcome);
  const value = outcome === 'done' ? 1 : progress;
  const percent = formatPercent(value);
  const downloading = outcome === 'running' && detail?.item && detail.downloaded !== null ? detail : null;

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <div className="flex items-center justify-between gap-3">
          <p className="min-w-0 flex-1 truncate text-base text-fg">
            {detail ? t('build.phaseStep', { step: detail.step, total: detail.total, phase: t(buildPhaseLabel(detail.phase)) }) : t('build.starting')}
          </p>
          {percent && <span className="text-sm tabular-nums text-fg-3">{percent}</span>}
        </div>
        <Progress
          value={value}
          tone={outcome === 'failed' ? 'danger' : outcome === 'done' ? 'success' : 'accent'}
          label={t('build.running')}
        />
        {/* The backend's own wording, like a log line; the download box below already says the same. */}
        {message && !downloading && <p className="truncate font-mono text-xs text-fg-3">{message}</p>}
      </div>

      {downloading && (
        <div className="rounded-md border border-line bg-panel-2 px-3 py-2">
          <div className="flex items-center justify-between gap-3 text-sm">
            <span className="min-w-0 truncate text-fg-2">{t('build.downloadingItem', { item: downloading.item ?? '' })}</span>
            <span className="shrink-0 tabular-nums text-fg-3">
              {downloading.size !== null
                ? t('build.bytesOf', { done: formatBytes(downloading.downloaded), total: formatBytes(downloading.size) })
                : formatBytes(downloading.downloaded)}
            </span>
          </div>
          <Progress
            value={downloading.size ? (downloading.downloaded ?? 0) / downloading.size : null}
            className="mt-1.5"
            label={t('build.downloadingItem', { item: downloading.item ?? '' })}
          />
        </div>
      )}

      <ol className="space-y-1" aria-label={t('build.phases')}>
        {BUILD_PHASES.map((phase, i) => {
          const state = states[i];
          const active = detail?.phase === phase ? detail : null;
          const listed = ITEMIZED.includes(phase) ? (items[phase] ?? []) : [];
          // Single downloads (OpenCore, OcBinaryData, AMD patches): their size next to the phase.
          const download = ITEMIZED.includes(phase) ? null : (items[phase] ?? []).find((i) => i.size !== null || i.downloaded !== null);
          const downloadSize =
            download && (state === 'done' || state === 'active') ? itemSize(download.size, download.downloaded, state === 'done') : '';
          return (
            <li key={phase}>
              <div className="flex items-center gap-2 text-sm">
                <PhaseIcon state={state} />
                <span className={state === 'pending' || state === 'skipped' ? 'text-fg-3' : 'text-fg'}>
                  {t(buildPhaseLabel(phase))}
                </span>
                {state === 'skipped' && <span className="text-xs text-fg-4">{t('build.phaseSkipped')}</span>}
                {state === 'active' && active && active.index !== null && active.count !== null && active.count > 1 && (
                  <span className="text-xs tabular-nums text-fg-3">
                    {t('build.itemOf', { index: active.index, count: active.count })}
                  </span>
                )}
                {downloadSize && <span className="ml-auto text-xs tabular-nums text-fg-3">{downloadSize}</span>}
              </div>
              {listed.length > 0 && (
                <ul className="mt-1 mb-1.5 ml-6 max-h-40 space-y-0.5 overflow-y-auto pr-1">
                  {listed.map((item) => {
                    const done = outcome === 'done' || itemDone(item, detail, phase);
                    const current = !done && detail?.item === item.name && detail.phase === phase;
                    let itemState: PhaseState = done ? 'done' : current ? 'active' : 'pending';
                    if (current && (outcome === 'failed' || outcome === 'cancelled')) itemState = outcome;
                    return (
                      <li key={item.name} className="flex items-center gap-2 text-xs">
                        <PhaseIcon state={itemState} small />
                        <span className={`min-w-0 flex-1 truncate font-mono ${done || current ? 'text-fg-2' : 'text-fg-3'}`}>
                          {item.name}
                        </span>
                        <span className="shrink-0 tabular-nums text-fg-3">{itemSize(item.size, item.downloaded, done)}</span>
                      </li>
                    );
                  })}
                </ul>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

function itemSize(size: number | null, downloaded: number | null, done: boolean): string {
  if (size !== null) return done || downloaded === null ? formatBytes(size) : `${formatBytes(downloaded)} / ${formatBytes(size)}`;
  return downloaded !== null && downloaded > 0 ? formatBytes(downloaded) : '';
}

function PhaseIcon({ state, small = false }: { state: PhaseState; small?: boolean }) {
  const size = small ? 11 : 14;
  switch (state) {
    case 'done':
      return <CheckCircle2 size={size} className="shrink-0 text-ok" aria-hidden />;
    case 'active':
      return <Loader2 size={size} className="shrink-0 animate-spin text-accent" aria-hidden />;
    case 'failed':
      return <XCircle size={size} className="shrink-0 text-err" aria-hidden />;
    case 'cancelled':
      return <Ban size={size} className="shrink-0 text-fg-3" aria-hidden />;
    case 'skipped':
      return <MinusCircle size={size} className="shrink-0 text-fg-4" aria-hidden />;
    default:
      return <Circle size={size} className="shrink-0 text-fg-4" aria-hidden />;
  }
}
