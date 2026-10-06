import { CheckCircle2, Circle, Loader2, XCircle } from 'lucide-react';
import { useT, type MessageKey } from '../../i18n';
import { formatPercent } from '../../lib/format';
import { flashMilestones, milestoneStates, type MilestoneState } from '../../lib/flash';
import type { FlashStatus } from '../../stores/deploy';
import type { AppError } from '../../bridge/errors';
import type { FlashProgress } from '../../bridge/types';
import { ErrorPanel } from '../feedback/ErrorPanel';
import { Progress } from '../ui/Progress';
import { Section } from '../ui/Section';

const PHASE_LABEL: Record<string, MessageKey> = {
  prepare: 'flash.phase.prepare',
  partition: 'flash.phase.partition',
  format: 'flash.phase.format',
  'copy-efi': 'flash.phase.copyEfi',
  'copy-recovery': 'flash.phase.copyRecovery',
  verify: 'flash.phase.verify',
};

export function FlashProgressView({
  status,
  progress,
  phases,
  error,
  withRecovery,
}: {
  status: FlashStatus;
  progress: FlashProgress | null;
  phases: readonly string[];
  error: AppError | null;
  withRecovery: boolean;
}) {
  const t = useT();
  const milestones = flashMilestones(withRecovery);
  const states = milestoneStates(milestones, phases, status);
  const value = status === 'done' ? 1 : progress?.progress ?? null;
  const percent = formatPercent(value);

  return (
    <Section title={status === 'done' ? t('flash.doneTitle') : status === 'failed' ? t('flash.failedTitle') : t('flash.running')}>
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <p className="min-w-0 flex-1 truncate text-base text-fg">
            {status === 'done' ? t('flash.doneBody') : progress?.message || t('flash.starting')}
          </p>
          {percent && <span className="text-sm tabular-nums text-fg-3">{percent}</span>}
        </div>
        <Progress value={value} tone={status === 'failed' ? 'danger' : status === 'done' ? 'success' : 'accent'} label={t('flash.running')} />
        <ol className="space-y-1.5">
          {milestones.map((m, i) => (
            <li key={m} className="flex items-center gap-2 text-sm">
              <MilestoneIcon state={states[i]} />
              <span className={states[i] === 'pending' ? 'text-fg-3' : 'text-fg'}>{t(PHASE_LABEL[m])}</span>
            </li>
          ))}
        </ol>
        {status === 'failed' && (error || progress?.error) && (
          <ErrorPanel
            error={
              error ?? {
                code: 'FLASH_FAILED',
                message: progress?.error ?? '',
                severity: 'error',
                recoverable: true,
                suggestion: null,
                context: null,
              }
            }
            title={t('flash.failedTitle')}
          />
        )}
      </div>
    </Section>
  );
}

function MilestoneIcon({ state }: { state: MilestoneState }) {
  switch (state) {
    case 'done':
      return <CheckCircle2 size={14} className="text-ok" aria-hidden />;
    case 'active':
      return <Loader2 size={14} className="animate-spin text-accent" aria-hidden />;
    case 'failed':
      return <XCircle size={14} className="text-err" aria-hidden />;
    default:
      return <Circle size={14} className="text-fg-4" aria-hidden />;
  }
}
