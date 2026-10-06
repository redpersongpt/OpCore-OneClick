import { useState } from 'react';
import { Copy, RefreshCw } from 'lucide-react';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { NotesList } from '../components/feedback/NotesList';
import { EmptyState } from '../components/feedback/States';
import { ExportEfiButton } from '../components/review/ExportEfiButton';
import { IdentityCard } from '../components/review/IdentityCard';
import { Badge, type Tone } from '../components/ui/Badge';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Checkbox } from '../components/ui/Field';
import { KeyValue, PageHeader, Section, StepActions } from '../components/ui/Section';
import { useT } from '../i18n';
import { copyText } from '../lib/external';
import { macosLabel } from '../lib/macos';
import { ARTIFACT_TONE, NOTE_BADGE_TONE } from '../lib/tones';
import { validationVerdict, type Verdict } from '../lib/verdict';
import { useBuild } from '../stores/build';
import { useWizard } from '../stores/wizard';

const VERDICT_TONE: Record<Verdict, Tone> = { passed: 'success', warnings: 'warning', failed: 'danger' };

export default function Review() {
  const t = useT();
  const result = useBuild((s) => s.result);
  const validation = useBuild((s) => s.validation);
  const validating = useBuild((s) => s.validating);
  const validateError = useBuild((s) => s.validateError);
  const revalidate = useBuild((s) => s.revalidate);
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);
  const [override, setOverride] = useState(false);
  const [copied, setCopied] = useState(false);

  if (!result) {
    return (
      <EmptyState
        title={t('review.empty')}
        action={<Button onClick={() => goTo('build')}>{t('review.goBuild')}</Button>}
      />
    );
  }

  const v = validation ?? result.validation;
  const verdict = validationVerdict(v);
  const canContinue = verdict !== 'failed' || override;
  const notes = result.plan.notes;
  const bootArgs = result.plan.bootArgs.join(' ');

  return (
    <>
      <PageHeader
        title={t('review.title')}
        subtitle={t('review.subtitle', { version: macosLabel(result.target), oc: result.opencoreVersion })}
      />
      <div className="space-y-4">
        <Section
          title={t('review.validation')}
          actions={
            <>
              <Badge tone={VERDICT_TONE[verdict]} dot>
                {t(`review.verdict.${verdict}`)}
              </Badge>
              <Button size="sm" variant="ghost" icon={<RefreshCw />} onClick={() => void revalidate()} loading={validating}>
                {t('review.revalidate')}
              </Button>
            </>
          }
        >
          <div className="space-y-3">
            {!v.ocvalidateRan && <Banner tone="info">{t('review.noOcvalidate')}</Banner>}
            {validateError && <ErrorPanel error={validateError} title={t('review.validateFailed')} compact />}
            {v.issues.length === 0 ? (
              <p className="text-sm text-fg-3">{t('review.noIssues')}</p>
            ) : (
              <ul className="space-y-2">
                {v.issues.map((issue, i) => (
                  <li key={`${issue.source}-${i}`} className="flex items-start gap-2.5">
                    <Badge tone={NOTE_BADGE_TONE[issue.level]}>{t(`note.${issue.level}`)}</Badge>
                    <div className="min-w-0 flex-1">
                      <p className="text-base text-fg">{issue.message}</p>
                      <p className="font-mono text-xs text-fg-3">
                        {issue.source}
                        {issue.path ? ` · ${issue.path}` : ''}
                      </p>
                    </div>
                  </li>
                ))}
              </ul>
            )}
            {v.ocvalidateOutput && (
              <details>
                <summary className="cursor-pointer text-sm text-fg-2 select-none">{t('review.ocvalidateOutput')}</summary>
                <pre className="mt-2 max-h-64 overflow-auto rounded-md border border-line bg-bg p-3 font-mono text-xs leading-5 whitespace-pre-wrap text-fg-2">
                  {v.ocvalidateOutput}
                </pre>
              </details>
            )}
          </div>
        </Section>

        <IdentityCard identity={result.identity} secureBootModel={result.plan.smbios.secureBootModel} />

        <Section title={t('review.boot')}>
          <KeyValue label={t('plan.bootArgs')} mono>
            <span className="inline-flex items-start gap-2">
              <span className="break-all">{bootArgs || '—'}</span>
              {bootArgs && (
                <button
                  type="button"
                  onClick={async () => {
                    if (await copyText(bootArgs)) {
                      setCopied(true);
                      window.setTimeout(() => setCopied(false), 1500);
                    }
                  }}
                  className="rounded p-0.5 text-fg-3 hover:text-fg"
                  aria-label={t('common.copy')}
                >
                  <Copy size={12} aria-hidden />
                </button>
              )}
              {copied && <span className="font-sans text-xs text-ok">{t('common.copied')}</span>}
            </span>
          </KeyValue>
          <KeyValue label={t('review.efiPath')} mono>
            {result.efiPath}
          </KeyValue>
          <KeyValue label={t('review.buildId')} mono>
            {result.buildId}
          </KeyValue>
        </Section>

        <Section title={t('review.kexts', { count: result.kexts.length })} flush>
          <ul className="divide-y divide-line">
            {result.kexts.map((k) => (
              <li key={`${k.catalogId}-${k.name}`} className="flex items-start gap-3 px-4 py-2">
                <div className="w-52 shrink-0">
                  <p className={`truncate font-mono text-sm ${k.enabled ? 'text-fg' : 'text-fg-3 line-through'}`}>{k.name}</p>
                  {k.version && <p className="text-xs text-fg-3">{k.version}</p>}
                </div>
                <div className="min-w-0 flex-1 text-sm text-fg-2">
                  {k.reason}
                  {k.error && <p className="text-err-fg">{k.error}</p>}
                </div>
                <Badge tone={ARTIFACT_TONE[k.status]}>{t(`artifact.${k.status}`)}</Badge>
              </li>
            ))}
          </ul>
        </Section>

        <Section title={t('review.ssdts', { count: result.ssdts.length })} flush>
          {result.ssdts.length === 0 ? (
            <p className="px-4 py-3 text-sm text-fg-3">—</p>
          ) : (
            <ul className="divide-y divide-line">
              {result.ssdts.map((s) => (
                <li key={s.fileName} className="flex items-start gap-3 px-4 py-2">
                  <span className="w-52 shrink-0 truncate font-mono text-sm text-fg">{s.fileName}</span>
                  <span className="min-w-0 flex-1 text-sm text-fg-2">{s.reason}</span>
                  <Badge tone={ARTIFACT_TONE[s.status]}>{t(`artifact.${s.status}`)}</Badge>
                </li>
              ))}
            </ul>
          )}
        </Section>

        {(notes.length > 0 || result.warnings.length > 0) && (
          <Section title={t('review.notes')}>
            <div className="space-y-3">
              <NotesList notes={notes} />
              {result.warnings.length > 0 && (
                <ul className="list-disc space-y-0.5 pl-4 text-sm text-warn-fg">
                  {result.warnings.map((w) => (
                    <li key={w}>{w}</li>
                  ))}
                </ul>
              )}
            </div>
          </Section>
        )}

        <Section title={t('review.export')} description={t('review.exportHint')}>
          <ExportEfiButton efiPath={result.efiPath} />
        </Section>

        {verdict === 'failed' && (
          <Banner tone="danger" title={t('review.failedTitle')}>
            <p>{t('review.failedBody')}</p>
            <Checkbox checked={override} onChange={setOverride} label={t('review.override')} />
          </Banner>
        )}
      </div>

      <StepActions left={<Button onClick={() => goTo('build')}>{t('common.back')}</Button>}>
        <Button variant="primary" disabled={!canContinue} onClick={() => complete('review')}>
          {t('review.toDeploy')}
        </Button>
      </StepActions>
    </>
  );
}
