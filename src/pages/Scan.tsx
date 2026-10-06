import { useEffect, type ReactNode } from 'react';
import { Cpu, FileInput, FlaskConical, PencilLine, RefreshCw, ScanSearch } from 'lucide-react';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { LoadingState } from '../components/feedback/States';
import { Badge } from '../components/ui/Badge';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Progress } from '../components/ui/Progress';
import { PageHeader, Section, StepActions } from '../components/ui/Section';
import { useProfileDialogs } from '../hooks/useProfileDialogs';
import { useT } from '../i18n';
import { formatPercent } from '../lib/format';
import { profileHeadline } from '../lib/profile';
import { profileSource } from '../lib/verdict';
import { runScan, startDemo, startManual } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { TASK_KINDS, useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

export default function Scan() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const detected = useHardware((s) => s.detected);
  const scanning = useHardware((s) => s.scanning);
  const scanError = useHardware((s) => s.scanError);
  const scanAttempted = useHardware((s) => s.scanAttempted);
  const ioError = useHardware((s) => s.ioError);
  const isDemo = useHardware((s) => s.isDemo);
  const complete = useWizard((s) => s.complete);
  const scanTask = useTasks((s) => s.latest(TASK_KINDS.scan));
  const { busy, doImport } = useProfileDialogs();

  const scan = async () => {
    if (await runScan()) complete('scan');
  };

  useEffect(() => {
    // Runs once per session: scanAttempted flips synchronously when the scan starts.
    if (!profile && !scanning && !scanAttempted) void scan();
  }, [profile, scanning, scanAttempted]);

  if (scanning) {
    const running = scanTask?.status === 'running' ? scanTask : null;
    return (
      <>
        <PageHeader title={t('scan.title')} subtitle={t('scan.subtitle')} />
        <LoadingState message={running?.message || t('scan.scanning')}>
          <div className="w-64">
            <Progress value={running?.progress ?? null} label={t('scan.scanning')} />
            {formatPercent(running?.progress) && (
              <p className="mt-1 text-xs tabular-nums text-fg-3">{formatPercent(running?.progress)}</p>
            )}
          </div>
        </LoadingState>
      </>
    );
  }

  const alternatives = (
    <div className="grid grid-cols-2 gap-3">
      <OptionCard
        icon={<PencilLine size={16} />}
        title={t('scan.manualTitle')}
        body={t('scan.manualBody')}
        action={
          <Button size="sm" onClick={startManual}>
            {t('scan.manual')}
          </Button>
        }
      />
      <OptionCard
        icon={<FileInput size={16} />}
        title={t('scan.importTitle')}
        body={t('scan.importBody')}
        action={
          <Button size="sm" onClick={() => void doImport()} loading={busy === 'import'}>
            {t('scan.import')}
          </Button>
        }
      />
    </div>
  );

  if (!profile) {
    return (
      <>
        <PageHeader title={t('scan.title')} subtitle={t('scan.subtitle')} />
        <div className="space-y-4">
          {scanError ? (
            <ErrorPanel
              error={scanError}
              title={t('scan.failed')}
              actions={
                <Button size="sm" variant="primary" icon={<RefreshCw />} onClick={() => void scan()}>
                  {t('common.retry')}
                </Button>
              }
            />
          ) : (
            <Section>
              <div className="flex items-center gap-3">
                <ScanSearch size={18} className="text-fg-3" aria-hidden />
                <p className="flex-1 text-base text-fg-2">{t('scan.idle')}</p>
                <Button variant="primary" onClick={() => void scan()}>
                  {t('scan.start')}
                </Button>
              </div>
            </Section>
          )}
          {ioError && <ErrorPanel error={ioError} title={t('scan.importFailed')} compact />}
          <p className="pt-2 text-sm font-medium text-fg-2">{t('scan.otherOptions')}</p>
          {alternatives}
          <div className="flex items-center justify-between rounded-md border border-dashed border-line px-3.5 py-2.5">
            <p className="text-sm text-fg-3">{t('scan.demoBody')}</p>
            <Button size="sm" variant="ghost" icon={<FlaskConical />} onClick={startDemo}>
              {t('scan.demo')}
            </Button>
          </div>
        </div>
      </>
    );
  }

  const warnings = detected?.warnings ?? [];

  return (
    <>
      <PageHeader title={t('scan.title')} subtitle={t('scan.doneSubtitle')} />
      <div className="space-y-4">
        {isDemo && <Banner tone="warning" title={t('scan.demoActive')}>{t('scan.demoActiveBody')}</Banner>}
        {/* A failed re-scan keeps the previous profile; say so instead of failing silently. */}
        {scanError && <ErrorPanel error={scanError} title={t('scan.rescanFailed')} compact />}
        <Section>
          <div className="flex items-center gap-3">
            <Cpu size={18} className="text-fg-3" aria-hidden />
            <div className="min-w-0 flex-1">
              <p className="truncate text-base font-medium text-fg">{profileHeadline(profile) || t('hardware.unnamedCpu')}</p>
              <p className="text-sm text-fg-3">
                {t(`source.${profileSource(profile.source)}`)}
                {profile.source === 'scan' && ` · ${t('scan.confidence', { value: formatPercent(profile.scanConfidence) ?? '?' })}`}
              </p>
            </div>
            <Badge tone="success" dot>
              {t('scan.ready')}
            </Badge>
          </div>
        </Section>
        {warnings.length > 0 && (
          <Banner tone="warning" title={t('scan.warnings')}>
            <ul className="list-disc space-y-0.5 pl-4">
              {warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          </Banner>
        )}
      </div>
      <StepActions
        left={
          <Button icon={<RefreshCw />} onClick={() => void scan()}>
            {t('scan.rescan')}
          </Button>
        }
      >
        <Button variant="primary" onClick={() => complete('scan')}>
          {t('common.continue')}
        </Button>
      </StepActions>
    </>
  );
}

function OptionCard({ icon, title, body, action }: { icon: ReactNode; title: string; body: string; action: ReactNode }) {
  return (
    <div className="flex flex-col rounded-lg border border-line bg-panel px-4 py-3.5">
      <span className="text-fg-3" aria-hidden>
        {icon}
      </span>
      <p className="mt-2 text-base font-medium text-fg">{title}</p>
      <p className="mt-0.5 flex-1 text-sm text-fg-3">{body}</p>
      <div className="mt-3">{action}</div>
    </div>
  );
}
