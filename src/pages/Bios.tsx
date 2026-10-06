import { useEffect } from 'react';
import { RefreshCw } from 'lucide-react';
import type { BiosSetting, FirmwareCheck } from '../bridge/types';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { EmptyState, LoadingState } from '../components/feedback/States';
import { Badge } from '../components/ui/Badge';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { PageHeader, Section, StepActions } from '../components/ui/Section';
import { Spinner } from '../components/ui/Spinner';
import { useT, type MessageKey } from '../i18n';
import { checkState, firmwareChecks, probeFor } from '../lib/firmware';
import { macosLabel } from '../lib/macos';
import { CHECK_TONE } from '../lib/tones';
import { compatKey, useCompat } from '../stores/compat';
import { useFirmware } from '../stores/firmware';
import { useHardware } from '../stores/hardware';
import { useWizard } from '../stores/wizard';

const PROBE_CONFIDENCE: Record<string, MessageKey> = {
  high: 'bios.confidence.high',
  medium: 'bios.confidence.medium',
  low: 'bios.confidence.low',
  not_applicable: 'bios.confidence.na',
};

const SETTING_VALUE: Record<string, MessageKey> = {
  enable: 'bios.value.enable',
  enabled: 'bios.value.enable',
  disable: 'bios.value.disable',
  disabled: 'bios.value.disable',
};

export default function Bios() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const target = useCompat((s) => s.target);
  const fw = useFirmware();
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);

  const key = profile && target ? compatKey(profile, target) : null;
  const { loadSettings, runProbe, settingsRequestKey, settingsLoading, probeAttempted } = fw;

  useEffect(() => {
    if (!profile || !target || !key || settingsLoading || settingsRequestKey === key) return;
    void loadSettings(profile, target);
  }, [profile, target, key, settingsLoading, settingsRequestKey, loadSettings]);

  useEffect(() => {
    if (!probeAttempted) void runProbe();
  }, [probeAttempted, runProbe]);

  if (!profile || !target) {
    return (
      <EmptyState
        title={t('bios.noTarget')}
        action={<Button onClick={() => goTo('compatibility')}>{t('bios.goCompat')}</Button>}
      />
    );
  }

  const settings = fw.settingsKey === key ? fw.settings : null;
  // The probe reads the computer the app runs on; it says nothing about an
  // imported or manually entered machine.
  const probeApplies = profile.source === 'scan';
  const confidenceKey = PROBE_CONFIDENCE[fw.probe?.confidence ?? ''];
  const required = settings?.filter((s) => s.required) ?? [];
  const doneRequired = required.filter((s) => fw.checked[s.name]).length;

  return (
    <>
      <PageHeader title={t('bios.title')} subtitle={t('bios.subtitle', { version: macosLabel(target) })} />
      <div className="space-y-4">
        <Banner tone="warning" title={t('bios.bitlockerTitle')}>
          <p>{t('bios.bitlockerBody')}</p>
          <p className="mt-1.5">{t('bios.secureBootDualBoot')}</p>
        </Banner>

        <Section
          title={t('bios.checklist')}
          description={settings ? t('bios.progress', { done: doneRequired, total: required.length }) : undefined}
          flush
        >
          {fw.settingsError && fw.settingsRequestKey === key ? (
            <div className="p-4">
              <ErrorPanel
                error={fw.settingsError}
                title={t('bios.settingsFailed')}
                actions={
                  <Button size="sm" onClick={() => void loadSettings(profile, target)}>
                    {t('common.retry')}
                  </Button>
                }
              />
            </div>
          ) : !settings ? (
            <LoadingState message={t('bios.loading')} />
          ) : settings.length === 0 ? (
            <p className="px-4 py-3 text-sm text-fg-3">{t('bios.empty')}</p>
          ) : (
            <ul className="divide-y divide-line">
              {settings.map((setting) => (
                <SettingRow
                  key={setting.name}
                  setting={setting}
                  checked={!!fw.checked[setting.name]}
                  onToggle={() => fw.toggle(setting.name)}
                  probe={probeApplies ? probeFor(setting, fw.probe) : null}
                />
              ))}
            </ul>
          )}
        </Section>

        <Section
          title={t('bios.probe')}
          description={probeApplies ? t('bios.probeHint') : t('bios.probeOtherMachine')}
          actions={
            <Button size="sm" variant="ghost" icon={<RefreshCw />} onClick={() => void runProbe()} loading={fw.probeLoading}>
              {t('bios.probeAgain')}
            </Button>
          }
          flush
        >
          {fw.probeError ? (
            <div className="p-4">
              <ErrorPanel error={fw.probeError} title={t('bios.probeFailed')} compact />
            </div>
          ) : !fw.probe ? (
            <div className="flex items-center gap-2 px-4 py-3 text-sm text-fg-3">
              <Spinner size={14} /> {t('bios.probing')}
            </div>
          ) : (
            <>
              <ul className="divide-y divide-line">
                {firmwareChecks(fw.probe).map((check) => (
                  <ProbeRow key={check.name} check={check} />
                ))}
              </ul>
              <p className="border-t border-line px-4 py-2.5 text-xs text-fg-3">
                {[fw.probe.biosVendor, fw.probe.biosVersion].filter(Boolean).join(' ')}
                {fw.probe.confidence
                  ? ` · ${t('bios.confidence', { value: confidenceKey ? t(confidenceKey) : fw.probe.confidence })}`
                  : ''}
              </p>
            </>
          )}
        </Section>
      </div>

      <StepActions left={<Button onClick={() => goTo('compatibility')}>{t('common.back')}</Button>}>
        {settings && doneRequired < required.length && (
          <span className="text-sm text-fg-3">{t('bios.remaining', { count: required.length - doneRequired })}</span>
        )}
        <Button variant="primary" onClick={() => complete('bios')} disabled={!settings && !fw.settingsError}>
          {t('common.continue')}
        </Button>
      </StepActions>
    </>
  );
}

function SettingRow({
  setting,
  checked,
  onToggle,
  probe,
}: {
  setting: BiosSetting;
  checked: boolean;
  onToggle: () => void;
  probe: FirmwareCheck | null;
}) {
  const t = useT();
  const state = probe ? checkState(probe) : null;
  return (
    <li className="flex gap-3 px-4 py-3">
      <input
        type="checkbox"
        checked={checked}
        onChange={onToggle}
        aria-label={t('bios.markDone', { name: setting.name })}
        className="mt-1 size-3.5 shrink-0 cursor-pointer accent-accent"
      />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className={`text-base font-medium ${checked ? 'text-fg-3 line-through' : 'text-fg'}`}>{setting.name}</span>
          <Badge tone="info">{SETTING_VALUE[setting.value.toLowerCase()] ? t(SETTING_VALUE[setting.value.toLowerCase()]) : setting.value}</Badge>
          {setting.required ? <Badge tone="warning">{t('bios.required')}</Badge> : <Badge>{t('bios.optional')}</Badge>}
          {state && state !== 'na' && (
            <Badge tone={CHECK_TONE[state]} dot title={probe?.evidence}>
              {t(`check.${state}`)}
            </Badge>
          )}
        </div>
        {setting.reason && <p className="mt-0.5 text-sm text-fg-2">{setting.reason}</p>}
        {setting.locationHint && <p className="mt-0.5 font-mono text-xs text-fg-3">{setting.locationHint}</p>}
      </div>
    </li>
  );
}

function ProbeRow({ check }: { check: FirmwareCheck }) {
  const t = useT();
  const state = checkState(check);
  return (
    <li className="flex items-center gap-3 px-4 py-2.5">
      <span className="min-w-0 flex-1">
        <span className="block text-base text-fg">{check.name}</span>
        <span className="block truncate text-xs text-fg-3">{check.evidence}</span>
      </span>
      {check.required && <span className="text-2xs text-fg-3 uppercase">{t('bios.required')}</span>}
      <Badge tone={CHECK_TONE[state]} dot>
        {t(`check.${state}`)}
      </Badge>
    </li>
  );
}
