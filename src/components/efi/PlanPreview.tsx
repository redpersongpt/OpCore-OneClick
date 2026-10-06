import type { BuildPlan } from '../../bridge/types';
import { useT } from '../../i18n';
import { macosLabel } from '../../lib/macos';
import { NotesList } from '../feedback/NotesList';
import { Badge } from '../ui/Badge';
import { KeyValue } from '../ui/Section';

export function PlanPreview({ plan }: { plan: BuildPlan }) {
  const t = useT();
  const enabledKexts = plan.kexts.filter((k) => k.enabled);
  const enabledDrivers = plan.drivers.filter((d) => d.enabled);

  return (
    <div className="space-y-4">
      <div>
        <KeyValue label={t('plan.target')}>{macosLabel(plan.target)}</KeyValue>
        <KeyValue label={t('plan.smbios')}>
          <span className="font-medium">{plan.smbios.model}</span>
          {plan.smbios.boardIdSkip && (
            <Badge tone="warning" className="ml-2">
              {t('plan.boardIdSkip')}
            </Badge>
          )}
          {plan.smbios.reason && <span className="block text-sm text-fg-3">{plan.smbios.reason}</span>}
        </KeyValue>
        <KeyValue label={t('plan.secureBoot')} mono>
          {plan.smbios.secureBootModel}
        </KeyValue>
        <KeyValue label={t('plan.bootArgs')} mono>
          {plan.bootArgs.length > 0 ? plan.bootArgs.join(' ') : '—'}
        </KeyValue>
        {plan.amdCoreCount !== null && <KeyValue label={t('plan.amdCores')}>{plan.amdCoreCount}</KeyValue>}
        <KeyValue label={t('plan.drivers')} mono>
          {enabledDrivers.map((d) => d.path).join(', ') || '—'}
        </KeyValue>
      </div>

      <details className="group rounded-md border border-line" open>
        <summary className="cursor-pointer px-3 py-2 text-sm font-medium text-fg-2 select-none">
          {t('plan.kexts', { count: enabledKexts.length })}
        </summary>
        <ul className="divide-y divide-line border-t border-line">
          {plan.kexts.map((k) => (
            <li key={`${k.catalogId}-${k.bundle}`} className="flex items-start gap-3 px-3 py-2">
              <span className={`w-48 shrink-0 truncate font-mono text-sm ${k.enabled ? 'text-fg' : 'text-fg-3 line-through'}`}>
                {k.bundle}
              </span>
              <span className="min-w-0 flex-1 text-sm text-fg-2">{k.reason}</span>
              {k.required ? <Badge tone="info">{t('plan.required')}</Badge> : <Badge>{t('plan.optional')}</Badge>}
            </li>
          ))}
        </ul>
      </details>

      <details className="group rounded-md border border-line">
        <summary className="cursor-pointer px-3 py-2 text-sm font-medium text-fg-2 select-none">
          {t('plan.ssdts', { count: plan.ssdts.length })}
        </summary>
        <ul className="divide-y divide-line border-t border-line">
          {plan.ssdts.map((s) => (
            <li key={s.fileName} className="flex items-start gap-3 px-3 py-2">
              <span className="w-48 shrink-0 truncate font-mono text-sm text-fg">{s.fileName}</span>
              <span className="min-w-0 flex-1 text-sm text-fg-2">{s.reason}</span>
              <Badge>{t(`ssdtSource.${s.source.kind}`)}</Badge>
            </li>
          ))}
          {plan.ssdts.length === 0 && <li className="px-3 py-2 text-sm text-fg-3">—</li>}
        </ul>
      </details>

      {plan.notes.length > 0 && <NotesList notes={plan.notes} />}
    </div>
  );
}
