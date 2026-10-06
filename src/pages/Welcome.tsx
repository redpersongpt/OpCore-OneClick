import { History, Monitor, ShieldCheck, Usb } from 'lucide-react';
import { motion } from 'motion/react';
import Logo from '../components/Logo';
import { Button } from '../components/ui/Button';
import { Section } from '../components/ui/Section';
import { useI18n } from '../i18n';
import { formatDateTime } from '../lib/format';
import { macosLabel } from '../lib/macos';
import { toNum } from '../lib/num';
import { useApp } from '../stores/app';
import { resume, startOver } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { firstIncompleteIndex, STEPS, useWizard } from '../stores/wizard';

export default function Welcome() {
  const { t, locale } = useI18n();
  const persisted = useApp((s) => s.persisted);
  const dismissPersisted = useApp((s) => s.dismissPersisted);
  const info = useApp((s) => s.info);
  const hasSession = useHardware((s) => s.profile !== null);
  const locked = useWizard((s) => s.locks.length > 0);
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);
  const completed = useWizard((s) => s.completed);

  // Back to where the user left off (the first step that is not done yet).
  const continueSession = () => {
    if (!completed.includes('welcome')) complete('welcome');
    else goTo(STEPS[firstIncompleteIndex(completed)]);
  };

  const savedAt = formatDateTime(toNum(persisted?.timestamp), locale);

  return (
    <div className="flex flex-col items-center pt-8 pb-6 text-center">
      <motion.div initial={{ opacity: 0, scale: 0.9 }} animate={{ opacity: 1, scale: 1 }} transition={{ duration: 0.4 }}>
        <Logo size={72} className="text-fg" />
      </motion.div>
      <h1 className="mt-6 text-2xl font-semibold tracking-tight text-fg">OpCore-OneClick</h1>
      <p className="mt-2 max-w-md text-base leading-relaxed text-fg-3">{t('welcome.tagline')}</p>

      {persisted?.profile && (
        <Section className="mt-8 w-full max-w-md text-left">
          <div className="flex gap-3">
            <History size={16} className="mt-0.5 shrink-0 text-accent-fg" aria-hidden />
            <div className="min-w-0 flex-1">
              <p className="text-base font-medium text-fg">{t('welcome.resumeTitle')}</p>
              <p className="mt-0.5 truncate text-sm text-fg-2">
                {persisted.profile.cpu.name || t('hardware.unnamedCpu')}
                {persisted.target ? ` · ${macosLabel(persisted.target)}` : ''}
              </p>
              {savedAt && <p className="text-xs text-fg-3">{t('welcome.savedAt', { time: savedAt })}</p>}
              <div className="mt-3 flex gap-2">
                <Button size="sm" variant="primary" onClick={() => resume(persisted)} disabled={locked}>
                  {t('welcome.resume')}
                </Button>
                <Button size="sm" variant="ghost" onClick={dismissPersisted}>
                  {t('welcome.discard')}
                </Button>
              </div>
            </div>
          </div>
        </Section>
      )}

      <div className="mt-8 flex gap-2">
        {hasSession ? (
          <>
            <Button variant="primary" onClick={continueSession}>
              {t('welcome.continueSession')}
            </Button>
            <Button onClick={() => void startOver()} disabled={locked}>
              {t('welcome.startOver')}
            </Button>
          </>
        ) : (
          <Button variant="primary" onClick={() => void startOver()} disabled={locked}>
            {t('welcome.start')}
          </Button>
        )}
      </div>

      <ul className="mt-10 grid w-full max-w-lg grid-cols-3 gap-3 text-left">
        {[
          { icon: <Monitor size={15} />, title: t('welcome.feature.scan'), body: t('welcome.feature.scanBody') },
          { icon: <ShieldCheck size={15} />, title: t('welcome.feature.build'), body: t('welcome.feature.buildBody') },
          { icon: <Usb size={15} />, title: t('welcome.feature.usb'), body: t('welcome.feature.usbBody') },
        ].map((f) => (
          <li key={f.title} className="rounded-md border border-line bg-panel px-3 py-2.5">
            <span className="text-fg-3" aria-hidden>
              {f.icon}
            </span>
            <p className="mt-1.5 text-sm font-medium text-fg">{f.title}</p>
            <p className="mt-0.5 text-xs leading-snug text-fg-3">{f.body}</p>
          </li>
        ))}
      </ul>

      <p className="mt-8 text-xs text-fg-3">
        {t('welcome.platforms')}
        {info ? ` · v${info.version}` : ''}
      </p>
    </div>
  );
}
