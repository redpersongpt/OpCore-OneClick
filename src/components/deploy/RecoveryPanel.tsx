import { Download } from 'lucide-react';
import type { MacOsVersion } from '../../bridge/types';
import { useT } from '../../i18n';
import { formatBytes, formatPercent } from '../../lib/format';
import { recoveryPhaseLabel } from '../../lib/labels';
import { macosLabel } from '../../lib/macos';
import { toNum } from '../../lib/num';
import { useDeploy } from '../../stores/deploy';
import { TASK_KINDS, useTasks } from '../../stores/tasks';
import { ErrorPanel } from '../feedback/ErrorPanel';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Toggle } from '../ui/Field';
import { Progress } from '../ui/Progress';
import { Section } from '../ui/Section';

/** Recovery image for the same macOS the EFI was built for. */
export function RecoveryPanel({ target, disabled }: { target: MacOsVersion; disabled: boolean }) {
  const t = useT();
  const d = useDeploy();
  const info = d.recoveryInfoFor === target ? d.recoveryInfo : null;
  const ready = info?.available === true && info.verified && info.version === target;
  const progress = d.recoveryProgress;
  const downloading = d.recoveryDownloading && d.recoveryFor === target;
  const task = useTasks((s) => s.running(TASK_KINDS.recovery));
  const taskId = d.recoveryTaskId ?? task?.taskId ?? null;
  const cancelState = useTasks((s) => (taskId ? s.cancels[taskId] : undefined));
  const canCancel = downloading && taskId !== null && cancelState === undefined;

  return (
    <Section title={t('recovery.title', { version: macosLabel(target) })} description={t('recovery.hint')}>
      <div className="space-y-3">
        <Toggle
          checked={d.includeRecovery}
          onChange={d.setIncludeRecovery}
          label={t('recovery.include')}
          description={d.includeRecovery ? t('recovery.includeHint') : t('recovery.efiOnlyHint')}
          disabled={disabled || downloading}
        />

        {d.includeRecovery && (
          <>
            {downloading ? (
              <div className="space-y-1.5">
                <div className="flex items-center justify-between text-sm">
                  <span className="text-fg-2">{t(recoveryPhaseLabel(progress?.phase))}</span>
                  <span className="tabular-nums text-fg-3">
                    {progress
                      ? `${formatBytes(toNum(progress.downloaded))} / ${progress.total !== null ? formatBytes(toNum(progress.total)) : '?'}`
                      : ''}
                    {progress && formatPercent(progress.progress) ? ` · ${formatPercent(progress.progress)}` : ''}
                  </span>
                </div>
                <Progress value={progress?.progress ?? null} label={t('recovery.downloading')} />
                <div className="flex justify-end">
                  <Button size="sm" variant="ghost" onClick={() => void d.cancelRecovery()} disabled={!canCancel}>
                    {cancelState === 'requested' ? t('task.cancelling') : t('common.cancel')}
                  </Button>
                </div>
              </div>
            ) : ready ? (
              <div className="flex items-center gap-2">
                <Badge tone="success" dot>
                  {t('recovery.ready')}
                </Badge>
                <span className="text-sm text-fg-3">{formatBytes(toNum(info?.sizeBytes))}</span>
              </div>
            ) : (
              <div className="flex items-center gap-3">
                <Button variant="primary" icon={<Download />} onClick={() => void d.downloadRecovery(target)} disabled={disabled}>
                  {info?.available && !info.verified ? t('recovery.redownload') : t('recovery.download')}
                </Button>
                <span className="text-sm text-fg-3">{t('recovery.size')}</span>
              </div>
            )}
            {d.recoveryError && d.recoveryFor === target && !downloading && (
              <ErrorPanel error={d.recoveryError} title={t('recovery.failed')} compact />
            )}
            {d.recoveryInfoError && <ErrorPanel error={d.recoveryInfoError} title={t('recovery.infoFailed')} compact />}
          </>
        )}
      </div>
    </Section>
  );
}
