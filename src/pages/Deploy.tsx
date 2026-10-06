import { useEffect } from 'react';
import { Usb } from 'lucide-react';
import { DiskList } from '../components/deploy/DiskList';
import { FlashConfirmDialog } from '../components/deploy/FlashConfirmDialog';
import { FlashProgressView } from '../components/deploy/FlashProgressView';
import { RecoveryPanel } from '../components/deploy/RecoveryPanel';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { EmptyState } from '../components/feedback/States';
import { ExportEfiButton } from '../components/review/ExportEfiButton';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { PageHeader, StepActions } from '../components/ui/Section';
import { useT } from '../i18n';
import { diskBlock, minimumDiskBytes } from '../lib/disk';
import { macosLabel } from '../lib/macos';
import { toNum } from '../lib/num';
import { useBuild } from '../stores/build';
import { useDeploy } from '../stores/deploy';
import { useHardware } from '../stores/hardware';
import { useWizard } from '../stores/wizard';

const DISK_POLL_MS = 5000;

export default function Deploy() {
  const t = useT();
  const result = useBuild((s) => s.result);
  const isDemo = useHardware((s) => s.isDemo);
  const d = useDeploy();
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);

  const target = result?.target ?? null;
  const { loadPrivileges, refreshDisks, loadRecoveryInfo } = d;
  const needPrivileges = d.privileges === null && d.privilegesError === null;
  const needDisks = !d.disksLoaded && !d.disksLoading;
  const needRecoveryInfo = target !== null && d.recoveryInfoFor !== target && !d.recoveryDownloading;

  useEffect(() => {
    if (needPrivileges) void loadPrivileges();
  }, [needPrivileges, loadPrivileges]);

  useEffect(() => {
    if (needDisks && !isDemo) void refreshDisks();
  }, [needDisks, isDemo, refreshDisks]);

  useEffect(() => {
    if (needRecoveryInfo && target) void loadRecoveryInfo(target);
  }, [needRecoveryInfo, target, loadRecoveryInfo]);

  // Nothing plugged in yet: look again every few seconds until a drive shows up.
  const waitingForDrive = d.disksLoaded && d.disksError === null && d.disks.length === 0;
  const busy = d.flashStatus === 'running' || d.recoveryDownloading || d.preparing;
  useEffect(() => {
    if (!waitingForDrive || isDemo || busy) return;
    const timer = window.setInterval(() => void refreshDisks(), DISK_POLL_MS);
    return () => window.clearInterval(timer);
  }, [waitingForDrive, isDemo, busy, refreshDisks]);

  if (!result || !target) {
    return (
      <EmptyState title={t('review.empty')} action={<Button onClick={() => goTo('build')}>{t('review.goBuild')}</Button>} />
    );
  }

  const efiPath = result.efiPath;
  const recovery = d.includeRecovery ? target : null;
  const info = d.recoveryInfoFor === target ? d.recoveryInfo : null;
  const recoveryReady = !d.includeRecovery || (info?.available === true && info.verified && info.version === target);
  const minBytes = minimumDiskBytes(d.includeRecovery, toNum(info?.sizeBytes));
  const disk = d.disks.find((x) => x.devicePath === d.selected) ?? null;
  const diskOk = disk !== null && diskBlock(disk, minBytes) === null;
  const canWrite = !isDemo && diskOk && recoveryReady && !busy;

  const prepare = () => void d.prepare(efiPath, recovery);
  // A failed write may have repartitioned the drive: list it again before the next attempt.
  const afterFailure = () => {
    d.resetFlash();
    void refreshDisks();
  };

  if (d.flashStatus !== 'idle') {
    return (
      <>
        <PageHeader title={t('deploy.title')} subtitle={t('deploy.subtitle', { version: macosLabel(target) })} />
        <FlashProgressView
          status={d.flashStatus}
          progress={d.flashProgress}
          phases={d.flashPhases}
          error={d.flashError}
          withRecovery={recovery !== null}
        />
        <StepActions
          left={
            d.flashStatus === 'failed' ? <Button onClick={afterFailure}>{t('common.back')}</Button> : undefined
          }
        >
          {d.flashStatus === 'failed' && (
            <Button
              variant="primary"
              onClick={() => {
                afterFailure();
                prepare();
              }}
            >
              {t('flash.tryAgain')}
            </Button>
          )}
          {d.flashStatus === 'done' && (
            <Button variant="primary" onClick={() => complete('deploy')}>
              {t('common.continue')}
            </Button>
          )}
        </StepActions>
      </>
    );
  }

  return (
    <>
      <PageHeader title={t('deploy.title')} subtitle={t('deploy.subtitle', { version: macosLabel(target) })} />
      <div className="space-y-4">
        {isDemo && (
          <Banner tone="warning" title={t('deploy.demoTitle')}>
            {t('deploy.demoBody')}
          </Banner>
        )}
        {d.privileges && !d.privileges.elevated && (
          <Banner tone={d.privileges.canElevate ? 'info' : 'warning'} title={d.privileges.canElevate ? t('deploy.elevate') : t('deploy.needAdmin')}>
            {d.privileges.detail}
          </Banner>
        )}
        {d.privilegesError && <ErrorPanel error={d.privilegesError} title={t('deploy.privilegesFailed')} compact />}

        <RecoveryPanel target={target} disabled={isDemo || busy} />
        {!isDemo && <DiskList minBytes={minBytes} disabled={busy} />}
        {d.prepareError && <ErrorPanel error={d.prepareError} title={t('deploy.prepareFailed')} compact />}

        <div className="rounded-lg border border-line bg-panel px-4 py-3">
          <p className="text-sm text-fg-2">{t('deploy.exportInstead')}</p>
          <div className="mt-2">
            <ExportEfiButton efiPath={efiPath} />
          </div>
        </div>
      </div>

      <StepActions
        left={
          <>
            <Button onClick={() => goTo('review')} disabled={busy}>
              {t('common.back')}
            </Button>
            <Button variant="ghost" onClick={() => complete('deploy')} disabled={busy}>
              {t('deploy.skip')}
            </Button>
          </>
        }
      >
        {!recoveryReady && !isDemo && <span className="text-sm text-fg-3">{t('deploy.needRecovery')}</span>}
        <Button variant="danger" icon={<Usb />} onClick={prepare} disabled={!canWrite} loading={d.preparing}>
          {t('deploy.write')}
        </Button>
      </StepActions>

      <FlashConfirmDialog
        confirmation={d.confirmation}
        disk={disk}
        disks={d.disks}
        onCancel={d.dismissConfirmation}
        onRenew={prepare}
        renewing={d.preparing}
        onConfirm={() => void d.flash(efiPath)}
      />
    </>
  );
}
