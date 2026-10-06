import { Download, FileInput, RefreshCw } from 'lucide-react';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { EmptyState } from '../components/feedback/States';
import { ProfileEditor } from '../components/hardware/ProfileEditor';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { PageHeader, StepActions } from '../components/ui/Section';
import { useProfileDialogs } from '../hooks/useProfileDialogs';
import { useT } from '../i18n';
import { formatPercent } from '../lib/format';
import { useApp } from '../stores/app';
import { editProfile, refreshProfile } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { useWizard } from '../stores/wizard';

const LOW_CONFIDENCE = 0.6;

export default function Hardware() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const detected = useHardware((s) => s.detected);
  const dirty = useHardware((s) => s.dirty);
  const isDemo = useHardware((s) => s.isDemo);
  const refreshing = useHardware((s) => s.refreshing);
  const refreshError = useHardware((s) => s.refreshError);
  const ioError = useHardware((s) => s.ioError);
  const catalogError = useApp((s) => s.catalogError);
  const catalogLoading = useApp((s) => s.catalogLoading);
  const loadCatalog = useApp((s) => s.loadCatalog);
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);
  const { busy, exportedTo, doImport, doExport } = useProfileDialogs();

  if (!profile) {
    return (
      <EmptyState
        title={t('hardware.emptyTitle')}
        description={t('hardware.emptyBody')}
        action={<Button onClick={() => goTo('scan')}>{t('hardware.goScan')}</Button>}
      />
    );
  }

  const platformMissing = profile.cpu.platform === 'unknown';

  const onContinue = async () => {
    if (dirty && !(await refreshProfile())) return;
    complete('hardware');
  };

  return (
    <>
      <PageHeader
        title={t('hardware.title')}
        subtitle={t('hardware.subtitle')}
        actions={
          <>
            <Button size="sm" icon={<FileInput />} onClick={() => void doImport()} loading={busy === 'import'}>
              {t('hardware.import')}
            </Button>
            <Button size="sm" icon={<Download />} onClick={() => void doExport()} loading={busy === 'export'}>
              {t('hardware.export')}
            </Button>
          </>
        }
      />

      <div className="mb-4 space-y-3">
        {isDemo && <Banner tone="warning" title={t('scan.demoActive')}>{t('scan.demoActiveBody')}</Banner>}
        {profile.source === 'manual' && <Banner tone="info">{t('hardware.manualHint')}</Banner>}
        {profile.source === 'scan' && profile.scanConfidence < LOW_CONFIDENCE && (
          <Banner tone="warning" title={t('hardware.lowConfidence', { value: formatPercent(profile.scanConfidence) ?? '?' })}>
            {t('hardware.lowConfidenceBody')}
          </Banner>
        )}
        {detected && detected.warnings.length > 0 && (
          <Banner tone="warning" title={t('scan.warnings')}>
            <ul className="list-disc space-y-0.5 pl-4">
              {detected.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          </Banner>
        )}
        {catalogError && (
          <ErrorPanel
            error={catalogError}
            title={t('hardware.catalogFailed')}
            compact
            actions={
              <Button size="sm" onClick={() => void loadCatalog()} loading={catalogLoading}>
                {t('common.retry')}
              </Button>
            }
          />
        )}
        {ioError && <ErrorPanel error={ioError} title={t('hardware.ioFailed')} compact />}
        {exportedTo && <Banner tone="success">{t('hardware.exported', { path: exportedTo })}</Banner>}
        {refreshError && <ErrorPanel error={refreshError} title={t('hardware.refreshFailed')} compact />}
        {dirty && (
          <Banner
            tone="info"
            title={t('hardware.dirtyTitle')}
            actions={
              <Button size="sm" icon={<RefreshCw />} onClick={() => void refreshProfile()} loading={refreshing}>
                {t('hardware.apply')}
              </Button>
            }
          >
            {t('hardware.dirtyBody')}
          </Banner>
        )}
      </div>

      <ProfileEditor profile={profile} edit={editProfile} />

      <StepActions left={<Button onClick={() => goTo('scan')}>{t('common.back')}</Button>}>
        {platformMissing && <span className="text-sm text-warn-fg">{t('hardware.platformRequired')}</span>}
        <Button variant="primary" onClick={() => void onContinue()} loading={refreshing} disabled={platformMissing}>
          {t('common.continue')}
        </Button>
      </StepActions>
    </>
  );
}
