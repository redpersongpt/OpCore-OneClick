import { CheckCircle2, ExternalLink, LifeBuoy, RotateCcw } from 'lucide-react';
import { guideFor, POST_INSTALL } from '../content/postInstall';
import { NotesList } from '../components/feedback/NotesList';
import { ExportEfiButton } from '../components/review/ExportEfiButton';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Section } from '../components/ui/Section';
import { useI18n } from '../i18n';
import { findOption } from '../lib/compat';
import { openExternal } from '../lib/external';
import { macosLabel } from '../lib/macos';
import { hasIntelWifi } from '../lib/profile';
import { useApp } from '../stores/app';
import { useBuild } from '../stores/build';
import { useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import { startOver } from '../stores/flow';
import { useHardware } from '../stores/hardware';

export default function Complete() {
  const { t, lang } = useI18n();
  const result = useBuild((s) => s.result);
  const report = useCompat((s) => s.report);
  const profile = useHardware((s) => s.profile);
  const isDemo = useHardware((s) => s.isDemo);
  const flashed = useDeploy((s) => s.flashed);
  const openTroubleshoot = useApp((s) => s.openTroubleshoot);

  const target = result?.target ?? null;
  const option = findOption(report, target);
  const steps = guideFor(POST_INSTALL[lang], {
    needsRootPatch: option?.needsRootPatch ?? false,
    tahoe: target === '26',
    intelWifi: hasIntelWifi(profile),
    analogAudio: profile?.audio != null,
  });

  return (
    <div className="space-y-5">
      <div className="flex flex-col items-center pt-4 text-center">
        <CheckCircle2 size={40} className="text-ok" aria-hidden />
        <h1 className="mt-3 text-xl font-semibold text-fg">{flashed ? t('complete.titleUsb') : t('complete.titleEfi')}</h1>
        <p className="mt-1 max-w-md text-base text-fg-3">
          {target ? t('complete.subtitle', { version: macosLabel(target) }) : t('complete.subtitleNoBuild')}
        </p>
      </div>

      {isDemo && <Banner tone="warning">{t('complete.demo')}</Banner>}
      {!flashed && !isDemo && result && <Banner tone="info">{t('complete.notFlashed')}</Banner>}

      {result && result.plan.postInstall.length > 0 && (
        <Section title={t('complete.forThisBuild')}>
          <NotesList notes={result.plan.postInstall} />
        </Section>
      )}

      <Section title={t('complete.guide')} flush>
        <ol className="divide-y divide-line">
          {steps.map((step, i) => (
            <li key={step.id} className="px-4 py-3">
              <div className="flex items-start gap-3">
                <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-panel-3 text-xs text-fg-2">
                  {i + 1}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="text-base font-medium text-fg">{step.title}</p>
                  <p className="mt-0.5 text-sm text-fg-2">{step.body}</p>
                  <ul className="mt-1.5 list-disc space-y-0.5 pl-4 text-sm text-fg-3">
                    {step.steps.map((s) => (
                      <li key={s}>{s}</li>
                    ))}
                  </ul>
                  {step.link && (
                    <button
                      type="button"
                      onClick={() => void openExternal(step.link ?? '')}
                      className="mt-1.5 inline-flex items-center gap-1 text-sm text-accent-fg hover:underline"
                    >
                      {t('complete.readMore')} <ExternalLink size={11} aria-hidden />
                    </button>
                  )}
                </div>
              </div>
            </li>
          ))}
        </ol>
      </Section>

      <div className="flex flex-wrap items-start justify-between gap-3 border-t border-line pt-4">
        {result ? <ExportEfiButton efiPath={result.efiPath} /> : <span />}
        <div className="flex gap-2">
          <Button icon={<LifeBuoy />} onClick={() => openTroubleshoot(true)}>
            {t('nav.troubleshoot')}
          </Button>
          <Button icon={<RotateCcw />} onClick={() => void startOver()}>
            {t('complete.startOver')}
          </Button>
        </div>
      </div>
    </div>
  );
}
