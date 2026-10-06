import { useEffect } from 'react';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { NotesList } from '../components/feedback/NotesList';
import { EmptyState, LoadingState } from '../components/feedback/States';
import { VersionPicker } from '../components/compat/VersionPicker';
import { Badge, Dot } from '../components/ui/Badge';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Checkbox } from '../components/ui/Field';
import { PageHeader, Section, StepActions } from '../components/ui/Section';
import { Spinner } from '../components/ui/Spinner';
import { useT } from '../i18n';
import { compatGate, findOption } from '../lib/compat';
import { formatPercent } from '../lib/format';
import { componentLabel } from '../lib/labels';
import { macosLabel } from '../lib/macos';
import { SUPPORT_TONE } from '../lib/tones';
import { compatKey, useCompat } from '../stores/compat';
import { selectTarget } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { useWizard } from '../stores/wizard';

export default function Compatibility() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const { report, reportKey, requestKey, target, expertFor, reach, loading, error, check, classify, setExpert } = useCompat();
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);

  const key = compatKey(profile, target);

  useEffect(() => {
    if (!profile || loading || requestKey === key) return;
    void check(profile, target);
  }, [profile, target, key, loading, requestKey, check]);

  // Tell expert options (workarounds) from releases that cannot run at all.
  useEffect(() => {
    if (profile && report && !loading) void classify(profile);
  }, [profile, report, loading, classify]);

  if (!profile) {
    return (
      <EmptyState
        title={t('hardware.emptyTitle')}
        description={t('hardware.emptyBody')}
        action={<Button onClick={() => goTo('scan')}>{t('hardware.goScan')}</Button>}
      />
    );
  }

  const retry = () => void check(profile, target);

  if (!report) {
    return (
      <>
        <PageHeader title={t('compat.title')} subtitle={t('compat.subtitle')} />
        {error ? (
          <ErrorPanel
            error={error}
            title={t('compat.failed')}
            actions={
              <Button size="sm" variant="primary" onClick={retry}>
                {t('common.retry')}
              </Button>
            }
          />
        ) : (
          <LoadingState message={t('compat.checking')} />
        )}
      </>
    );
  }

  const fresh = reportKey === key && !loading;
  const gate = fresh ? compatGate(report, target) : 'blocked';
  const option = findOption(report, target);
  const expertAccepted = gate === 'expert' && expertFor === target && target !== null;
  const canContinue = fresh && (gate === 'ok' || expertAccepted);

  return (
    <>
      <PageHeader title={t('compat.title')} subtitle={t('compat.subtitle')} />
      <div className="space-y-4">
        <Section>
          <div className="flex items-start gap-3">
            <Dot tone={SUPPORT_TONE[report.level]} className="mt-1.5" />
            <div className="min-w-0 flex-1">
              <p className="text-md font-semibold text-fg">
                {t(`support.${report.level}`)}
                {target && <span className="ml-2 text-base font-normal text-fg-3">{macosLabel(target)}</span>}
              </p>
              {report.summary && <p className="mt-0.5 text-sm text-fg-2">{report.summary}</p>}
            </div>
            {loading ? (
              <Spinner label={t('compat.checking')} />
            ) : (
              <span className="text-sm tabular-nums text-fg-3">
                {t('compat.confidence', { value: formatPercent(report.confidence) ?? '?' })}
              </span>
            )}
          </div>
        </Section>

        {error && (
          <ErrorPanel
            error={error}
            title={t('compat.failed')}
            compact
            actions={
              <Button size="sm" onClick={retry}>
                {t('common.retry')}
              </Button>
            }
          />
        )}

        <Section title={t('compat.versions')} description={t('compat.versionsHint')}>
          <VersionPicker report={report} reach={reach} selected={target} onSelect={(v) => selectTarget(v)} disabled={loading} />
          <p className="mt-3 text-xs text-fg-3">{t('compat.lastIntel')}</p>
        </Section>

        {option && (option.notes.length > 0 || option.needsRootPatch) && (
          <Banner
            tone={option.supported || gate === 'expert' ? 'warning' : 'danger'}
            title={t('compat.aboutVersion', { version: macosLabel(option.version) })}
          >
            <ul className="list-disc space-y-0.5 pl-4">
              {option.needsRootPatch && <li>{t('compat.rootPatchBody')}</li>}
              {option.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          </Banner>
        )}

        {report.notes.length > 0 && (
          <Section title={t('compat.notes')}>
            <NotesList notes={report.notes} />
          </Section>
        )}

        <Section title={t('compat.components')} flush>
          {report.components.length === 0 ? (
            <p className="px-4 py-3 text-sm text-fg-3">{t('compat.noComponents')}</p>
          ) : (
            <ul className="divide-y divide-line">
              {report.components.map((c, i) => (
                <li key={`${c.component}-${i}`} className="px-4 py-2.5">
                  <div className="flex items-center gap-3">
                    <span className="w-20 shrink-0 text-xs font-medium tracking-wide text-fg-3 uppercase">{componentLabel(t, c.component)}</span>
                    <span className="min-w-0 flex-1 truncate text-base text-fg">{c.name}</span>
                    <Badge tone={SUPPORT_TONE[c.level]} dot>
                      {t(`support.${c.level}`)}
                    </Badge>
                  </div>
                  {c.notes.length > 0 && (
                    <ul className="mt-1 ml-23 list-disc space-y-0.5 pl-4 text-sm text-fg-2">
                      {c.notes.map((n) => (
                        <li key={n}>{n}</li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          )}
        </Section>

        {fresh && gate === 'expert' && target && (
          <Banner tone="warning" title={t('compat.expertTitle', { version: macosLabel(target) })}>
            <p>{option?.supported ? t('compat.expertBodyNotes') : t('compat.expertBody')}</p>
            <ul className="mt-1.5 list-disc space-y-0.5 pl-4">
              <li>{t('compat.expertRisk.updates')}</li>
              <li>{t('compat.expertRisk.features')}</li>
              <li>{t('compat.expertRisk.support')}</li>
            </ul>
            <div className="mt-2">
              <Checkbox checked={expertAccepted} onChange={setExpert} label={t('compat.expertAccept')} />
            </div>
          </Banner>
        )}
        {fresh && gate === 'blocked' && (
          <Banner tone="danger" title={t('compat.blockedTitle')}>
            {t('compat.blockedBody')}
          </Banner>
        )}
      </div>

      <StepActions left={<Button onClick={() => goTo('hardware')}>{t('common.back')}</Button>}>
        <Button variant="primary" disabled={!canContinue} onClick={() => complete('compatibility')}>
          {t('common.continue')}
        </Button>
      </StepActions>
    </>
  );
}
