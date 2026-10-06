import { useEffect, type ReactNode } from 'react';
import { Cpu, FileInput, FlaskConical, PencilLine, RefreshCw, ScanSearch } from 'lucide-react';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { LoadingState } from '../components/feedback/States';
import { Badge } from '../components/ui/Badge';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Progress } from '../components/ui/Progress';
import { PageHeader, Section, StepActions } from '../components/ui/Section';
import { HostNotices } from '../components/hardware/HostNotices';
import { useProfileDialogs } from '../hooks/useProfileDialogs';
import { useT } from '../i18n';
import { formatPercent } from '../lib/format';
import { blocksBuild, holdsScanStep, isAppleSiliconHost, profileNotices, scanNotices } from '../lib/host';
import { profileHeadline } from '../lib/profile';
import { profileSource } from '../lib/verdict';
import { useApp } from '../stores/app';
import { runScan, startDemo, startManual } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { isCancellable, TASK_KINDS, useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

export default function Scan() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const detected = useHardware((s) => s.detected);
  const scanning = useHardware((s) => s.scanning);
  const scanError = useHardware((s) => s.scanError);
  const scanAttempted = useHardware((s) => s.scanAttempted);
  const ioError = useHardware((s) => s.ioError);
  const scanCancelled = useHardware((s) => s.scanCancelled);
  const cancelScan = useHardware((s) => s.cancelScan);
  const isDemo = useHardware((s) => s.isDemo);
  const info = useApp((s) => s.info);
  const complete = useWizard((s) => s.complete);
  const running = useTasks((s) => s.running(TASK_KINDS.scan));
  const cancelState = useTasks((s) => (running ? s.cancels[running.taskId] : undefined));
  const { busy, doImport } = useProfileDialogs();
  // An Apple silicon Mac cannot use an OpenCore EFI: scanning it is only useful on request.
  const appleHost = isAppleSiliconHost(info);

  const scan = async () => {
    if (!(await runScan())) return;
    const after = useHardware.getState();
    // Stay here when the result needs the user's attention before moving on.
    if (!holdsScanStep(scanNotices(after.detected, after.profile))) complete('scan');
  };

  useEffect(() => {
    // Runs once per session: scanAttempted flips synchronously when the scan starts.
    if (!profile && !scanning && !scanAttempted && !appleHost) void scan();
  }, [profile, scanning, scanAttempted, appleHost]);

  if (scanning) {
    const canCancel = isCancellable(running) && cancelState === undefined;
    return (
      <>
        <PageHeader title={t('scan.title')} subtitle={t('scan.subtitle')} />
        <LoadingState message={t('scan.scanning')}>
          <div className="w-64">
            <Progress value={running?.progress ?? null} label={t('scan.scanning')} />
            {formatPercent(running?.progress) && (
              <p className="mt-1 text-xs tabular-nums text-fg-3">{formatPercent(running?.progress)}</p>
            )}
          </div>
          <Button size="sm" variant="ghost" onClick={() => void cancelScan()} disabled={!canCancel}>
            {cancelState === 'requested' ? t('task.cancelling') : t('common.cancel')}
          </Button>
        </LoadingState>
      </>
    );
  }

  const importOther = () => void doImport();

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
          {appleHost && !scanError && (
            <HostNotices notices={['apple_silicon']} onImport={importOther} onManual={startManual} importing={busy === 'import'} />
          )}
          {scanCancelled && !scanError && <Banner tone="info">{t('scan.cancelled')}</Banner>}
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
                <p className="flex-1 text-base text-fg-2">{appleHost ? t('scan.idleAppleHost') : t('scan.idle')}</p>
                <Button variant={appleHost ? 'secondary' : 'primary'} onClick={() => void scan()}>
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
  const notices = profileNotices(detected, profile);
  const unusable = blocksBuild(notices);

  return (
    <>
      <PageHeader title={t('scan.title')} subtitle={unusable ? t('scan.unusableSubtitle') : t('scan.doneSubtitle')} />
      <div className="space-y-4">
        {isDemo && <Banner tone="warning" title={t('scan.demoActive')}>{t('scan.demoActiveBody')}</Banner>}
        {/* A failed re-scan keeps the previous profile; say so instead of failing silently. */}
        {scanError && <ErrorPanel error={scanError} title={t('scan.rescanFailed')} compact />}
        {scanCancelled && <Banner tone="info">{t('scan.rescanCancelled')}</Banner>}
        <HostNotices notices={notices} onImport={importOther} onManual={startManual} importing={busy === 'import'} />
        {ioError && <ErrorPanel error={ioError} title={t('scan.importFailed')} compact />}
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
            {unusable ? (
              <Badge tone="danger" dot>
                {t('scan.unusable')}
              </Badge>
            ) : (
              <Badge tone="success" dot>
                {t('scan.ready')}
              </Badge>
            )}
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
        <Button variant="primary" onClick={() => complete('scan')} disabled={unusable}>
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
