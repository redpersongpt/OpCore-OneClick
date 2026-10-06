import { Check, LifeBuoy, Lock, Settings2 } from 'lucide-react';
import { useT } from '../../i18n';
import { useApp } from '../../stores/app';
import { STEPS, firstIncompleteIndex, stepIndex, useWizard } from '../../stores/wizard';
import Logo from '../Logo';

export default function Sidebar() {
  const t = useT();
  const step = useWizard((s) => s.step);
  const completed = useWizard((s) => s.completed);
  const locks = useWizard((s) => s.locks);
  const goTo = useWizard((s) => s.goTo);
  const info = useApp((s) => s.info);
  const updateAvailable = useApp((s) => s.update?.updateAvailable ?? false);
  const openSettings = useApp((s) => s.openSettings);
  const openTroubleshoot = useApp((s) => s.openTroubleshoot);

  const limit = firstIncompleteIndex(completed);
  const locked = locks.length > 0;

  return (
    <aside className="flex w-[184px] shrink-0 flex-col border-r border-line bg-sidebar">
      <div className="flex h-11 items-center gap-2 px-4" data-tauri-drag-region>
        <Logo size={18} className="text-fg-2" />
        <span className="text-xs font-semibold tracking-[0.08em] text-fg-3 uppercase" data-tauri-drag-region>
          OpCore
        </span>
      </div>

      <nav aria-label={t('nav.steps')} className="flex-1 overflow-y-auto px-2 py-2">
        <ol>
          {STEPS.map((s, idx) => {
            const current = s === step;
            const done = completed.includes(s);
            const reachable = stepIndex(s) <= limit;
            const enabled = current || (reachable && !locked);
            return (
              <li key={s}>
                <button
                  type="button"
                  onClick={() => goTo(s)}
                  disabled={!enabled}
                  aria-current={current ? 'step' : undefined}
                  className={`mb-px flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors ${
                    current
                      ? 'bg-panel-2 font-medium text-fg'
                      : enabled
                        ? 'text-fg-3 hover:bg-panel hover:text-fg-2'
                        : 'cursor-not-allowed text-fg-4'
                  }`}
                >
                  <span
                    className={`flex size-4 shrink-0 items-center justify-center rounded-full text-2xs tabular-nums ${
                      done && !current
                        ? 'bg-ok-soft text-ok'
                        : current
                          ? 'bg-accent-soft text-accent-fg'
                          : 'text-fg-4'
                    }`}
                    aria-hidden
                  >
                    {done && !current ? <Check size={10} strokeWidth={3} /> : idx + 1}
                  </span>
                  <span className="truncate">{t(`step.${s}`)}</span>
                  {done && !current && <span className="sr-only">{t('nav.completed')}</span>}
                </button>
              </li>
            );
          })}
        </ol>
        {locked && (
          <p className="mt-3 flex items-start gap-1.5 px-2 text-2xs leading-snug text-fg-3">
            <Lock size={11} className="mt-px shrink-0" aria-hidden />
            {t('nav.locked')}
          </p>
        )}
      </nav>

      <div className="space-y-1 border-t border-line px-2 py-2">
        <button
          type="button"
          onClick={() => openTroubleshoot(true)}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm text-fg-3 hover:bg-panel hover:text-fg-2"
        >
          <LifeBuoy size={14} aria-hidden />
          {t('nav.troubleshoot')}
        </button>
        <button
          type="button"
          onClick={() => openSettings(true)}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm text-fg-3 hover:bg-panel hover:text-fg-2"
        >
          <Settings2 size={14} aria-hidden />
          {t('nav.settings')}
          {updateAvailable && (
            <span className="ml-auto rounded bg-accent-soft px-1 text-2xs text-accent-fg">{t('nav.update')}</span>
          )}
        </button>
        <p className="px-2 pt-1 text-2xs text-fg-3">{info ? `v${info.version} · OpenCore ${info.opencoreVersion}` : ' '}</p>
      </div>
    </aside>
  );
}
