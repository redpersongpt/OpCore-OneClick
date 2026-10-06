import { useEffect } from 'react';
import { Ban, CheckCircle2, Loader2, X, XCircle } from 'lucide-react';
import type { RecoveryProgress, TaskUpdate } from '../../bridge/types';
import { useT, type MessageKey, type Translate } from '../../i18n';
import { buildPhaseLabel, parseBuildDetail } from '../../lib/buildProgress';
import { formatBytes, formatPercent } from '../../lib/format';
import { flashPhaseLabel } from '../../lib/flash';
import { recoveryPhaseLabel } from '../../lib/labels';
import { useDeploy } from '../../stores/deploy';
import { CANCELLABLE_KINDS, isCancellable, TASK_KINDS, useTasks } from '../../stores/tasks';
import { Button } from '../ui/Button';
import { Progress } from '../ui/Progress';

const KIND_LABEL: Record<string, MessageKey> = {
  [TASK_KINDS.scan]: 'task.kind.scan',
  [TASK_KINDS.build]: 'task.kind.build',
  [TASK_KINDS.recovery]: 'task.kind.recovery',
  [TASK_KINDS.flash]: 'task.kind.flash',
};

const AUTO_HIDE_MS = 4000;

export default function TaskBar() {
  const t = useT();
  const task = useTasks((s) => s.visible());
  const dismiss = useTasks((s) => s.dismiss);
  const cancel = useTasks((s) => s.cancel);
  const cancelState = useTasks((s) => (task ? s.cancels[task.taskId] : undefined));
  const flashProgress = useDeploy((s) => s.flashProgress);
  const recoveryProgress = useDeploy((s) => s.recoveryProgress);

  useEffect(() => {
    if (!task || task.status !== 'completed') return;
    const timer = window.setTimeout(() => dismiss(task.taskId), AUTO_HIDE_MS);
    return () => window.clearTimeout(timer);
  }, [task, dismiss]);

  if (!task) return null;

  const kindLabel = KIND_LABEL[task.kind] ? t(KIND_LABEL[task.kind]) : task.kind;
  const running = task.status === 'running';
  const percent = formatPercent(task.progress);
  let detail: string | null;
  if (running) {
    detail = runningDetail(t, task, {
      flashPhase: flashProgress?.taskId === task.taskId ? flashProgress.phase : null,
      recovery: recoveryProgress?.taskId === task.taskId ? recoveryProgress : null,
    });
  } else {
    const status = t(`task.status.${task.status}`);
    // A failure carries the backend's reason.
    detail = task.status === 'failed' && task.message?.trim() ? `${status}: ${task.message.trim()}` : status;
  }
  const offerCancel = running && CANCELLABLE_KINDS.includes(task.kind);
  const cancellable = isCancellable(task) && cancelState === undefined;

  return (
    <div className="shrink-0 border-t border-line bg-bg px-4 py-2" aria-live="polite">
      <div className="flex items-center gap-3">
        <StatusIcon task={task} />
        <span className="min-w-0 flex-1 truncate text-sm text-fg-2">
          <span className="font-medium text-fg">{kindLabel}</span>
          {detail ? ` — ${detail}` : ''}
        </span>
        {running && percent && <span className="text-xs tabular-nums text-fg-3">{percent}</span>}
        {offerCancel && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void cancel(task.taskId)}
            disabled={!cancellable}
            title={cancellable || cancelState === 'requested' ? undefined : t('task.cannotCancel')}
          >
            {cancelState === 'requested' ? t('task.cancelling') : t('common.cancel')}
          </Button>
        )}
        {!running && (
          <button
            type="button"
            onClick={() => dismiss(task.taskId)}
            aria-label={t('common.dismiss')}
            className="rounded p-1 text-fg-3 hover:bg-panel-2 hover:text-fg"
          >
            <X size={13} aria-hidden />
          </button>
        )}
      </div>
      {running && <Progress value={task.progress} className="mt-1.5" label={kindLabel} />}
    </div>
  );
}

/**
 * Localized description of a running task. Build, USB write and recovery
 * download carry structured progress; the scan has none worth showing.
 */
function runningDetail(
  t: Translate,
  task: TaskUpdate,
  extra: { flashPhase: string | null; recovery: RecoveryProgress | null },
): string | null {
  switch (task.kind) {
    case TASK_KINDS.build: {
      const detail = parseBuildDetail(task.detail);
      if (!detail) return null;
      const phase = t(buildPhaseLabel(detail.phase));
      if (!detail.item) return phase;
      const position =
        detail.index !== null && detail.count !== null
          ? ` (${t('build.itemOf', { index: detail.index, count: detail.count })})`
          : '';
      return `${phase}: ${detail.item}${position}`;
    }
    case TASK_KINDS.flash: {
      const key = extra.flashPhase ? flashPhaseLabel(extra.flashPhase) : null;
      return key ? t(key) : null;
    }
    case TASK_KINDS.recovery: {
      const progress = extra.recovery;
      if (!progress) return null;
      if (progress.phase === 'downloading' && progress.downloaded > 0) {
        return progress.total !== null
          ? t('build.bytesOf', { done: formatBytes(progress.downloaded), total: formatBytes(progress.total) })
          : formatBytes(progress.downloaded);
      }
      return t(recoveryPhaseLabel(progress.phase));
    }
    case TASK_KINDS.scan:
      return null;
    default:
      return task.message?.trim() || null;
  }
}

function StatusIcon({ task }: { task: TaskUpdate }) {
  switch (task.status) {
    case 'completed':
      return <CheckCircle2 size={14} className="shrink-0 text-ok" aria-hidden />;
    case 'failed':
      return <XCircle size={14} className="shrink-0 text-err" aria-hidden />;
    case 'cancelled':
      return <Ban size={14} className="shrink-0 text-fg-3" aria-hidden />;
    default:
      return <Loader2 size={14} className="shrink-0 animate-spin text-accent" aria-hidden />;
  }
}
