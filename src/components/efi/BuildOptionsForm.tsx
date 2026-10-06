import type { CompatibilityReport, IntelWifiStrategy, MacOsVersion, PickerStyle, PlatformIdentity, SmbiosPlan } from '../../bridge/types';
import { useT } from '../../i18n';
import { sortedVersions } from '../../lib/compat';
import { parseCount } from '../../lib/format';
import { macosLabel } from '../../lib/macos';
import { useApp } from '../../stores/app';
import { useCompat } from '../../stores/compat';
import type { OptionsDraft } from '../../stores/build';
import { Field, ParsedInput, Select, TextInput, Toggle, type SelectOption } from '../ui/Field';

const WIFI: readonly IntelWifiStrategy[] = ['auto', 'itlwm', 'airport_itlwm', 'none'];
const PICKERS: readonly PickerStyle[] = ['graphical', 'text'];

export function BuildOptionsForm({
  draft,
  onChange,
  target,
  onTarget,
  report,
  smbios,
  identity,
  identityReusable,
  plannedModel,
  showIntelWifi,
  disabled,
}: {
  draft: OptionsDraft;
  onChange: (patch: Partial<OptionsDraft>) => void;
  target: MacOsVersion;
  onTarget: (target: MacOsVersion) => void;
  report: CompatibilityReport | null;
  smbios: SmbiosPlan | null;
  identity: PlatformIdentity | null;
  /** False when the previous serials belong to another SMBIOS model than this build's. */
  identityReusable: boolean;
  plannedModel: string | null;
  showIntelWifi: boolean;
  disabled: boolean;
}) {
  const t = useT();
  const catalog = useApp((s) => s.catalog);
  const reach = useCompat((s) => s.reach);

  const targetOptions: SelectOption<MacOsVersion>[] = report
    ? sortedVersions(report).map((v) => ({
        value: v.version,
        label: v.supported
          ? macosLabel(v.version)
          : `${macosLabel(v.version)} — ${reach[v.version] === 'expert' ? t('compat.expertOption') : t('compat.notSupported')}`,
      }))
    : [{ value: target, label: macosLabel(target) }];

  const seen = new Set<string>();
  const smbiosOptions: SelectOption<string>[] = [
    { value: '', label: smbios ? t('build.smbiosAutoWith', { model: smbios.model }) : t('build.smbiosAuto') },
  ];
  for (const model of smbios?.alternatives ?? []) {
    if (seen.has(model)) continue;
    seen.add(model);
    smbiosOptions.push({ value: model, label: t('build.smbiosAlternative', { model }) });
  }
  for (const o of catalog?.smbiosModels ?? []) {
    if (seen.has(o.id)) continue;
    seen.add(o.id);
    smbiosOptions.push({ value: o.id, label: o.detail ? `${o.label} — ${o.detail}` : o.label });
  }

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3">
        <Field label={t('build.target')}>
          {(id) => <Select id={id} value={target} options={targetOptions} onChange={onTarget} disabled={disabled} />}
        </Field>
        <Field label={t('build.smbios')} hint={t('build.smbiosHint')}>
          {(id) => (
            <Select<string>
              id={id}
              value={draft.smbiosOverride ?? ''}
              options={smbiosOptions}
              disabled={disabled}
              onChange={(v) => onChange({ smbiosOverride: v === '' ? null : v })}
            />
          )}
        </Field>
        <Field label={t('build.picker')}>
          {(id) => (
            <Select<PickerStyle>
              id={id}
              value={draft.picker}
              options={PICKERS.map((p) => ({ value: p, label: t(`enum.picker.${p}`) }))}
              disabled={disabled}
              onChange={(picker) => onChange({ picker })}
            />
          )}
        </Field>
        <Field label={t('build.timeout')} hint={t('build.timeoutHint')}>
          {(id) => (
            <ParsedInput
              id={id}
              value={draft.pickerTimeout}
              format={(n) => String(n)}
              parse={parseCount}
              placeholder={t('common.default')}
              disabled={disabled}
              onChange={(pickerTimeout) => onChange({ pickerTimeout })}
            />
          )}
        </Field>
        {showIntelWifi && (
          <Field label={t('build.intelWifi')} hint={t(`build.intelWifiHint.${draft.intelWifi}`)} className="col-span-2">
            {(id) => (
              <Select<IntelWifiStrategy>
                id={id}
                value={draft.intelWifi}
                options={WIFI.map((w) => ({ value: w, label: t(`enum.intelWifi.${w}`) }))}
                disabled={disabled}
                onChange={(intelWifi) => onChange({ intelWifi })}
              />
            )}
          </Field>
        )}
        <Field label={t('build.bootArgs')} hint={t('build.bootArgsHint')} className="col-span-2">
          {(id) => (
            <TextInput
              id={id}
              mono
              value={draft.extraBootArgs ?? ''}
              placeholder="alcid=11 -wegnoegpu"
              disabled={disabled}
              onChange={(v) => onChange({ extraBootArgs: v === '' ? null : v })}
            />
          )}
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-x-6">
        <Toggle
          checked={draft.verbose}
          onChange={(verbose) => onChange({ verbose })}
          label={t('build.verbose')}
          description={t('build.verboseHint')}
          disabled={disabled}
        />
        <Toggle
          checked={draft.debugOpencore}
          onChange={(debugOpencore) => onChange({ debugOpencore })}
          label={t('build.debug')}
          description={t('build.debugHint')}
          disabled={disabled}
        />
        <Toggle
          checked={draft.disableUnsupportedGpus}
          onChange={(disableUnsupportedGpus) => onChange({ disableUnsupportedGpus })}
          label={t('build.disableGpus')}
          description={t('build.disableGpusHint')}
          disabled={disabled}
        />
        {target === '26' && (
          <Toggle
            checked={draft.prepareAudioPatch}
            onChange={(prepareAudioPatch) => onChange({ prepareAudioPatch })}
            label={t('build.audioPatch')}
            description={t('build.audioPatchHint')}
            disabled={disabled}
          />
        )}
        <Toggle
          checked={draft.useLatestReleases}
          onChange={(useLatestReleases) => onChange({ useLatestReleases })}
          label={t('build.latest')}
          description={t('build.latestHint')}
          disabled={disabled}
        />
        {identity && (
          <Toggle
            checked={draft.keepIdentity}
            onChange={(keepIdentity) => onChange({ keepIdentity })}
            label={t('build.keepIdentity', { model: identity.model })}
            description={
              identityReusable
                ? t('build.keepIdentityHint')
                : t('build.keepIdentityMismatch', { old: identity.model, model: draft.smbiosOverride ?? plannedModel ?? '?' })
            }
            disabled={disabled}
          />
        )}
      </div>
    </div>
  );
}
