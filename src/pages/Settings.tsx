import { useCallback, useEffect, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { Bug, Download, ExternalLink, FileDown, LifeBuoy, RefreshCw, Trash2 } from 'lucide-react';
import { api } from '../bridge/api';
import { toAppError, type AppError } from '../bridge/errors';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { Badge } from '../components/ui/Badge';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Field, Select, Toggle } from '../components/ui/Field';
import { Modal } from '../components/ui/Modal';
import { KeyValue, Section } from '../components/ui/Section';
import { LANGUAGES, useI18n, type Lang } from '../i18n';
import { collectDiagnostics } from '../lib/diagnostics';
import { openExternal, RELEASES_URL, REPO_URL } from '../lib/external';
import { buildIssueUrl } from '../lib/issue';
import { useApp } from '../stores/app';
import { useBuild } from '../stores/build';
import { useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import { afterCacheCleared, setOptions } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

/** Errors that only mean "another operation is using the files right now". */
const BUSY_CODES = ['BUSY', 'BUILD_IN_PROGRESS', 'FLASH_IN_PROGRESS', 'RECOVERY_IN_PROGRESS'];

type Busy = 'export' | 'cache' | 'recovery' | 'state' | 'report' | null;

export default function Settings() {
  const { t, lang, setLang } = useI18n();
  const open = useApp((s) => s.settingsOpen);
  const setOpen = useApp((s) => s.openSettings);
  const openTroubleshoot = useApp((s) => s.openTroubleshoot);
  const info = useApp((s) => s.info);
  const update = useApp((s) => s.update);
  const updateChecking = useApp((s) => s.updateChecking);
  const updateError = useApp((s) => s.updateError);
  const checkUpdates = useApp((s) => s.checkUpdates);
  const wizardLocked = useWizard((s) => s.locks.length > 0);
  const taskRunning = useTasks((s) => Object.values(s.tasks).some((task) => task.status === 'running'));
  const locked = wizardLocked || taskRunning;
  const useLatest = useBuild((s) => s.draft.useLatestReleases);

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [log, setLog] = useState('');
  const [logLoading, setLogLoading] = useState(false);
  const [busy, setBusy] = useState<Busy>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busyNotice, setBusyNotice] = useState(false);

  const loadLog = useCallback(async () => {
    setLogLoading(true);
    try {
      const [sid, tail] = await Promise.all([api.logGetSessionId(), api.logGetTail(200)]);
      setSessionId(sid);
      setLog(tail);
    } catch (err) {
      setLog('');
      setError(toAppError(err));
    } finally {
      setLogLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    setStatus(null);
    setError(null);
    setBusyNotice(false);
    void loadLog();
  }, [open, loadLog]);

  const run = async (kind: Exclude<Busy, null>, action: () => Promise<string | null>) => {
    setBusy(kind);
    setStatus(null);
    setError(null);
    setBusyNotice(false);
    try {
      const message = await action();
      if (message) setStatus(message);
    } catch (err) {
      const appError = toAppError(err);
      if (BUSY_CODES.includes(appError.code)) setBusyNotice(true);
      else setError(appError);
    } finally {
      setBusy(null);
    }
  };

  const exportLog = () =>
    run('export', async () => {
      const path = await save({
        defaultPath: `opcore-oneclick-support-${new Date().toISOString().slice(0, 10)}.log`,
        filters: [{ name: t('settings.logFile'), extensions: ['log', 'txt'] }],
      });
      if (!path) return null;
      await api.saveSupportLog(path);
      return t('settings.logSaved', { path });
    });

  const clearCache = () =>
    run('cache', async () => {
      await api.clearAppCache();
      afterCacheCleared();
      return t('settings.cacheCleared');
    });

  const clearRecovery = () =>
    run('recovery', async () => {
      await api.clearRecoveryCache();
      useDeploy.getState().resetRecovery();
      return t('settings.recoveryCleared');
    });

  const forgetSession = () =>
    run('state', async () => {
      await api.clearState();
      useApp.getState().dismissPersisted();
      return t('settings.sessionCleared');
    });

  const report = () =>
    run('report', async () => {
      const diagnostics = await collectDiagnostics({
        info,
        profile: useHardware.getState().profile,
        report: useCompat.getState().report,
        result: useBuild.getState().result,
      });
      await openExternal(
        buildIssueUrl({ title: t('settings.issueTitle'), description: t('settings.issueDescription'), diagnostics }),
      );
      return null;
    });

  return (
    <Modal
      open={open}
      onClose={() => setOpen(false)}
      title={t('settings.title')}
      width="max-w-2xl"
      footer={<Button onClick={() => setOpen(false)}>{t('common.close')}</Button>}
    >
      <div className="space-y-4">
        <Section title={t('settings.general')}>
          <Field label={t('settings.language')} className="max-w-xs">
            {(id) => (
              <Select<Lang>
                id={id}
                value={lang}
                options={LANGUAGES.map((l) => ({ value: l.id, label: l.label }))}
                onChange={setLang}
              />
            )}
          </Field>
        </Section>

        <Section title={t('settings.downloads')}>
          <Toggle
            checked={useLatest}
            onChange={(value) => setOptions({ useLatestReleases: value })}
            label={t('settings.latest')}
            description={t('settings.latestHint')}
            disabled={locked}
          />
          <ul className="mt-2 list-disc space-y-0.5 pl-9 text-xs text-fg-3">
            <li>{t('settings.latestOff')}</li>
            <li>{t('settings.latestOn')}</li>
            <li>{t('settings.latestFails')}</li>
          </ul>
        </Section>

        <Section title={t('settings.about')}>
          <KeyValue label={t('settings.version')}>{info ? `v${info.version}` : '—'}</KeyValue>
          <KeyValue label="OpenCore">{info?.opencoreVersion ?? '—'}</KeyValue>
          <KeyValue label={t('settings.host')}>{info ? `${info.hostOs} / ${info.arch}` : '—'}</KeyValue>
          <KeyValue label={t('settings.session')} mono>
            {sessionId ?? '—'}
          </KeyValue>
          <button
            type="button"
            onClick={() => void openExternal(REPO_URL)}
            className="mt-2 inline-flex items-center gap-1 text-sm text-accent-fg hover:underline"
          >
            {t('settings.repo')} <ExternalLink size={11} aria-hidden />
          </button>
        </Section>

        <Section
          title={t('settings.updates')}
          actions={
            <Button size="sm" variant="ghost" icon={<RefreshCw />} onClick={() => void checkUpdates()} loading={updateChecking}>
              {t('settings.checkNow')}
            </Button>
          }
        >
          {updateError ? (
            <ErrorPanel error={updateError} title={t('settings.updateFailed')} compact />
          ) : update ? (
            update.updateAvailable ? (
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <Badge tone="info" dot>
                    {t('settings.updateAvailable', { version: update.latest ?? '?' })}
                  </Badge>
                  <span className="text-sm text-fg-3">{t('settings.current', { version: update.current })}</span>
                </div>
                {update.notes && <p className="line-clamp-4 text-sm whitespace-pre-line text-fg-2">{update.notes}</p>}
                <Button size="sm" variant="primary" icon={<Download />} onClick={() => void openExternal(update.url ?? RELEASES_URL)}>
                  {t('settings.openRelease')}
                </Button>
              </div>
            ) : (
              <p className="text-sm text-fg-2">{t('settings.upToDate', { version: update.current })}</p>
            )
          ) : (
            <p className="text-sm text-fg-3">{updateChecking ? t('settings.checking') : t('settings.notChecked')}</p>
          )}
        </Section>

        <Section title={t('settings.maintenance')}>
          <div className="grid grid-cols-2 gap-2">
            <Button icon={<FileDown />} onClick={() => void exportLog()} loading={busy === 'export'}>
              {t('settings.exportLog')}
            </Button>
            <Button icon={<Bug />} onClick={() => void report()} loading={busy === 'report'}>
              {t('settings.report')}
            </Button>
            <Button icon={<Trash2 />} onClick={() => void clearCache()} loading={busy === 'cache'} disabled={locked}>
              {t('settings.clearCache')}
            </Button>
            <Button icon={<Trash2 />} onClick={() => void clearRecovery()} loading={busy === 'recovery'} disabled={locked}>
              {t('settings.clearRecovery')}
            </Button>
            <Button icon={<Trash2 />} onClick={() => void forgetSession()} loading={busy === 'state'}>
              {t('settings.forgetSession')}
            </Button>
            <Button
              icon={<LifeBuoy />}
              onClick={() => {
                setOpen(false);
                openTroubleshoot(true);
              }}
            >
              {t('nav.troubleshoot')}
            </Button>
          </div>
          {locked && <p className="mt-2 text-xs text-fg-3">{t('settings.lockedHint')}</p>}
          {status && <Banner tone="success" className="mt-3">{status}</Banner>}
          {busyNotice && (
            <Banner tone="warning" className="mt-3" title={t('settings.busyTitle')}>
              {t('settings.busyBody')}
            </Banner>
          )}
          {error && (
            <div className="mt-3">
              <ErrorPanel error={error} compact />
            </div>
          )}
        </Section>

        <Section
          title={t('settings.log')}
          description={t('settings.logHint')}
          actions={
            <Button size="sm" variant="ghost" icon={<RefreshCw />} onClick={() => void loadLog()} loading={logLoading}>
              {t('common.refresh')}
            </Button>
          }
        >
          <pre className="max-h-64 overflow-auto rounded-md border border-line bg-bg p-3 font-mono text-2xs leading-4 whitespace-pre-wrap break-words text-fg-2">
            {logLoading ? t('common.loading') : log || t('settings.logEmpty')}
          </pre>
        </Section>
      </div>
    </Modal>
  );
}
