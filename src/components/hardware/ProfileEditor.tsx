import type { ReactNode } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import type {
  CpuPlatform,
  FormFactor,
  GpuFamily,
  HardwareProfile,
  InputBus,
  ProfileGpu,
  ProfileNic,
  TouchpadVendor,
  VmKind,
} from '../../bridge/types';
import { useT } from '../../i18n';
import { formatBytes, formatCodecId, formatVram, normalizePciId, parseCodecId, parseCount } from '../../lib/format';
import { toNum } from '../../lib/num';
import { blankAudio, blankGpu, blankNic } from '../../lib/profile';
import { useApp } from '../../stores/app';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Field, ParsedInput, Select, TextInput, Toggle } from '../ui/Field';
import { Section } from '../ui/Section';
import {
  catalogOptions,
  cpuVendorOptions,
  deviceBusOptions,
  fromTri,
  gpuVendorOptions,
  inputBusOptions,
  toTri,
  touchpadVendorOptions,
  triOptions,
  vmOptions,
  type TriState,
} from './options';

type Edit = (update: (profile: HardwareProfile) => HardwareProfile) => void;

const countInput = { format: (n: number) => String(n), parse: parseCount };
const pciInput = { format: (s: string) => s, parse: normalizePciId };
const codecInput = { format: (n: number) => formatCodecId(n), parse: parseCodecId };

function Grid({ children, cols = 2 }: { children: ReactNode; cols?: 2 | 3 | 4 }) {
  const cls = cols === 4 ? 'grid-cols-4' : cols === 3 ? 'grid-cols-3' : 'grid-cols-2';
  return <div className={`grid gap-3 ${cls}`}>{children}</div>;
}

export function ProfileEditor({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  return (
    <div className="space-y-4">
      <CpuSection profile={profile} edit={edit} />
      <MachineSection profile={profile} edit={edit} />
      <GpuSection profile={profile} edit={edit} />
      <AudioSection profile={profile} edit={edit} />
      <NetworkSection profile={profile} edit={edit} />
      <InputSection profile={profile} edit={edit} />
      <StorageSection profile={profile} />
    </div>
  );
}

function CpuSection({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  const t = useT();
  const catalog = useApp((s) => s.catalog);
  const cpu = profile.cpu;
  const setCpu = (patch: Partial<HardwareProfile['cpu']>) => edit((p) => ({ ...p, cpu: { ...p.cpu, ...patch } }));

  return (
    <Section title={t('hardware.cpu')} description={cpu.codename || undefined}>
      <div className="space-y-3">
        <Field label={t('hardware.cpuName')}>
          {(id) => <TextInput id={id} value={cpu.name} onChange={(name) => setCpu({ name })} placeholder="Intel Core i7-9700K" />}
        </Field>
        <Grid>
          <Field label={t('hardware.vendor')}>
            {(id) => <Select id={id} value={cpu.vendor} options={cpuVendorOptions(t)} onChange={(vendor) => setCpu({ vendor })} />}
          </Field>
          <Field label={t('hardware.platform')} hint={catalog ? undefined : t('hardware.catalogMissing')}>
            {(id) => (
              <Select<string>
                id={id}
                value={cpu.platform}
                options={catalogOptions(catalog?.cpuPlatforms)}
                onChange={(platform) => setCpu({ platform: platform as CpuPlatform })}
              />
            )}
          </Field>
        </Grid>
        <Grid cols={4}>
          <Field label={t('hardware.cores')}>
            {(id) => <ParsedInput id={id} value={cpu.cores || null} {...countInput} onChange={(v) => setCpu({ cores: v ?? 0 })} />}
          </Field>
          <Field label={t('hardware.threads')}>
            {(id) => <ParsedInput id={id} value={cpu.threads || null} {...countInput} onChange={(v) => setCpu({ threads: v ?? 0 })} />}
          </Field>
          <Field label={t('hardware.avx2')}>
            {(id) => (
              <Select<TriState> id={id} value={toTri(cpu.hasAvx2)} options={triOptions(t)} onChange={(v) => setCpu({ hasAvx2: fromTri(v) })} />
            )}
          </Field>
          <Field label={t('hardware.sse42')}>
            {(id) => (
              <Select<TriState> id={id} value={toTri(cpu.hasSse42)} options={triOptions(t)} onChange={(v) => setCpu({ hasSse42: fromTri(v) })} />
            )}
          </Field>
        </Grid>
        <Grid>
          <Toggle checked={cpu.isMobile} onChange={(isMobile) => setCpu({ isMobile })} label={t('hardware.mobileCpu')} />
          <Toggle
            checked={cpu.isHybrid}
            onChange={(isHybrid) => setCpu({ isHybrid })}
            label={t('hardware.hybrid')}
            description={t('hardware.hybridHint')}
          />
        </Grid>
      </div>
    </Section>
  );
}

function MachineSection({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  const t = useT();
  const catalog = useApp((s) => s.catalog);
  const set = (patch: Partial<HardwareProfile>) => edit((p) => ({ ...p, ...patch }));

  return (
    <Section title={t('hardware.machine')}>
      <div className="space-y-3">
        <Grid>
          <Field label={t('hardware.formFactor')}>
            {(id) => (
              <Select<string>
                id={id}
                value={profile.formFactor}
                options={catalogOptions(catalog?.formFactors)}
                onChange={(v) => set({ formFactor: v as FormFactor })}
              />
            )}
          </Field>
          <Field label={t('hardware.vm')}>
            {(id) => (
              <Select<VmKind | ''>
                id={id}
                value={profile.vm ?? ''}
                options={vmOptions(t)}
                onChange={(v) => set({ vm: v === '' ? null : v })}
              />
            )}
          </Field>
        </Grid>
        <Grid cols={3}>
          <Field label={t('hardware.boardVendor')}>
            {(id) => <TextInput id={id} value={profile.motherboardVendor} onChange={(v) => set({ motherboardVendor: v })} />}
          </Field>
          <Field label={t('hardware.boardModel')}>
            {(id) => <TextInput id={id} value={profile.motherboardModel} onChange={(v) => set({ motherboardModel: v })} />}
          </Field>
          <Field label={t('hardware.chipset')} hint={t('hardware.chipsetHint')}>
            {(id) => (
              <TextInput
                id={id}
                value={profile.chipset ?? ''}
                placeholder="Z390"
                onChange={(v) => set({ chipset: v.trim() ? v : null })}
              />
            )}
          </Field>
        </Grid>
        <Grid cols={3}>
          <Field label={t('hardware.ram')}>
            {(id) => <ParsedInput id={id} value={profile.ramGb || null} {...countInput} onChange={(v) => set({ ramGb: v ?? 0 })} />}
          </Field>
          <Field label={t('hardware.uefi')}>
            {(id) => (
              <Select<TriState>
                id={id}
                value={toTri(profile.firmwareUefi)}
                options={triOptions(t)}
                onChange={(v) => set({ firmwareUefi: fromTri(v) })}
              />
            )}
          </Field>
          <div className="pt-5">
            <Toggle checked={profile.hasBattery} onChange={(hasBattery) => set({ hasBattery })} label={t('hardware.battery')} />
          </div>
        </Grid>
      </div>
    </Section>
  );
}

function GpuSection({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  const t = useT();
  const catalog = useApp((s) => s.catalog);
  const setGpu = (index: number, patch: Partial<ProfileGpu>) =>
    edit((p) => ({ ...p, gpus: p.gpus.map((g, i) => (i === index ? { ...g, ...patch } : g)) }));

  return (
    <Section
      title={t('hardware.gpus')}
      description={t('hardware.gpusHint')}
      actions={
        <Button size="sm" variant="ghost" icon={<Plus />} onClick={() => edit((p) => ({ ...p, gpus: [...p.gpus, blankGpu()] }))}>
          {t('common.add')}
        </Button>
      }
      flush
    >
      {profile.gpus.length === 0 ? (
        <p className="px-4 py-3 text-sm text-fg-3">{t('hardware.noGpus')}</p>
      ) : (
        <ul className="divide-y divide-line">
          {profile.gpus.map((gpu, index) => (
            <li key={index} className="space-y-3 px-4 py-3">
              <div className="flex items-center gap-2">
                <Badge tone={gpu.isIgpu ? 'info' : 'neutral'}>{gpu.isIgpu ? t('hardware.igpu') : t('hardware.dgpu')}</Badge>
                {gpu.disabled && <Badge tone="warning">{t('hardware.gpuDisabled')}</Badge>}
                <span className="font-mono text-xs text-fg-3">
                  {gpu.vendorId && gpu.deviceId ? `${gpu.vendorId}:${gpu.deviceId}` : ''}
                  {formatVram(toNum(gpu.vramMb)) ? ` · ${formatVram(toNum(gpu.vramMb))}` : ''}
                </span>
                <span className="flex-1" />
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<Trash2 />}
                  aria-label={t('common.remove')}
                  onClick={() => edit((p) => ({ ...p, gpus: p.gpus.filter((_, i) => i !== index) }))}
                />
              </div>
              <Grid>
                <Field label={t('hardware.gpuName')}>
                  {(id) => <TextInput id={id} value={gpu.name} onChange={(name) => setGpu(index, { name })} />}
                </Field>
                <Field label={t('hardware.gpuFamily')}>
                  {(id) => (
                    <Select<string>
                      id={id}
                      value={gpu.family}
                      options={catalogOptions(catalog?.gpuFamilies)}
                      onChange={(family) => setGpu(index, { family: family as GpuFamily })}
                    />
                  )}
                </Field>
              </Grid>
              <Grid cols={4}>
                <Field label={t('hardware.vendor')}>
                  {(id) => <Select id={id} value={gpu.vendor} options={gpuVendorOptions(t)} onChange={(vendor) => setGpu(index, { vendor })} />}
                </Field>
                <Field label={t('hardware.vendorId')}>
                  {(id) => (
                    <ParsedInput id={id} mono value={gpu.vendorId} {...pciInput} placeholder="1002" onChange={(vendorId) => setGpu(index, { vendorId })} />
                  )}
                </Field>
                <Field label={t('hardware.deviceId')}>
                  {(id) => (
                    <ParsedInput id={id} mono value={gpu.deviceId} {...pciInput} placeholder="67df" onChange={(deviceId) => setGpu(index, { deviceId })} />
                  )}
                </Field>
                <div className="pt-5">
                  <Toggle checked={gpu.isIgpu} onChange={(isIgpu) => setGpu(index, { isIgpu })} label={t('hardware.integrated')} />
                </div>
              </Grid>
              <Toggle
                checked={gpu.disabled}
                onChange={(disabled) => setGpu(index, { disabled })}
                label={t('hardware.disableGpu')}
                description={t('hardware.disableGpuHint')}
              />
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function AudioSection({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  const t = useT();
  const audio = profile.audio;
  const setAudio = (patch: Partial<NonNullable<HardwareProfile['audio']>>) =>
    edit((p) => ({ ...p, audio: { ...(p.audio ?? blankAudio()), ...patch } }));

  return (
    <Section
      title={t('hardware.audio')}
      actions={
        audio ? (
          <Button size="sm" variant="ghost" icon={<Trash2 />} onClick={() => edit((p) => ({ ...p, audio: null }))}>
            {t('common.remove')}
          </Button>
        ) : (
          <Button size="sm" variant="ghost" icon={<Plus />} onClick={() => edit((p) => ({ ...p, audio: blankAudio() }))}>
            {t('common.add')}
          </Button>
        )
      }
    >
      {audio ? (
        <Grid cols={3}>
          <Field label={t('hardware.codec')}>
            {(id) => <TextInput id={id} value={audio.codecName} placeholder="Realtek ALC897" onChange={(codecName) => setAudio({ codecName })} />}
          </Field>
          <Field label={t('hardware.codecId')} hint={t('hardware.codecIdHint')}>
            {(id) => (
              <ParsedInput id={id} mono value={audio.codecId} {...codecInput} placeholder="0x10EC0897" onChange={(codecId) => setAudio({ codecId })} />
            )}
          </Field>
          <Field label={t('hardware.layoutId')} hint={t('hardware.layoutIdHint')}>
            {(id) => (
              <ParsedInput
                id={id}
                value={audio.layoutId}
                {...countInput}
                placeholder={t('common.auto')}
                onChange={(layoutId) => setAudio({ layoutId })}
              />
            )}
          </Field>
        </Grid>
      ) : (
        <p className="text-sm text-fg-3">{t('hardware.noAudio')}</p>
      )}
    </Section>
  );
}

function NicFields({ nic, onChange }: { nic: ProfileNic; onChange: (patch: Partial<ProfileNic>) => void }) {
  const t = useT();
  return (
    <Grid cols={4}>
      <Field label={t('hardware.deviceName')} className="col-span-1">
        {(id) => <TextInput id={id} value={nic.name} onChange={(name) => onChange({ name })} />}
      </Field>
      <Field label={t('hardware.bus')}>
        {(id) => <Select id={id} value={nic.bus} options={deviceBusOptions(t)} onChange={(bus) => onChange({ bus })} />}
      </Field>
      <Field label={t('hardware.vendorId')}>
        {(id) => <ParsedInput id={id} mono value={nic.vendorId} {...pciInput} placeholder="8086" onChange={(vendorId) => onChange({ vendorId })} />}
      </Field>
      <Field label={t('hardware.deviceId')}>
        {(id) => <ParsedInput id={id} mono value={nic.deviceId} {...pciInput} placeholder="15bc" onChange={(deviceId) => onChange({ deviceId })} />}
      </Field>
    </Grid>
  );
}

function OptionalNic({
  title,
  nic,
  onSet,
}: {
  title: string;
  nic: ProfileNic | null;
  onSet: (nic: ProfileNic | null) => void;
}) {
  const t = useT();
  return (
    <div className="space-y-2 px-4 py-3">
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium text-fg-2">{title}</p>
        {nic ? (
          <Button size="sm" variant="ghost" icon={<Trash2 />} onClick={() => onSet(null)}>
            {t('common.remove')}
          </Button>
        ) : (
          <Button size="sm" variant="ghost" icon={<Plus />} onClick={() => onSet(blankNic())}>
            {t('common.add')}
          </Button>
        )}
      </div>
      {nic ? <NicFields nic={nic} onChange={(patch) => onSet({ ...nic, ...patch })} /> : <p className="text-sm text-fg-3">{t('hardware.none')}</p>}
    </div>
  );
}

function NetworkSection({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  const t = useT();
  return (
    <Section title={t('hardware.network')} flush>
      <div className="divide-y divide-line">
        <div className="space-y-2 px-4 py-3">
          <div className="flex items-center justify-between">
            <p className="text-sm font-medium text-fg-2">{t('hardware.ethernet')}</p>
            <Button
              size="sm"
              variant="ghost"
              icon={<Plus />}
              onClick={() => edit((p) => ({ ...p, ethernet: [...p.ethernet, blankNic()] }))}
            >
              {t('common.add')}
            </Button>
          </div>
          {profile.ethernet.length === 0 && <p className="text-sm text-fg-3">{t('hardware.none')}</p>}
          {profile.ethernet.map((nic, index) => (
            <div key={index} className="flex items-end gap-2">
              <div className="flex-1">
                <NicFields
                  nic={nic}
                  onChange={(patch) =>
                    edit((p) => ({ ...p, ethernet: p.ethernet.map((n, i) => (i === index ? { ...n, ...patch } : n)) }))
                  }
                />
              </div>
              <Button
                size="sm"
                variant="ghost"
                icon={<Trash2 />}
                aria-label={t('common.remove')}
                onClick={() => edit((p) => ({ ...p, ethernet: p.ethernet.filter((_, i) => i !== index) }))}
              />
            </div>
          ))}
        </div>
        <OptionalNic title={t('hardware.wifi')} nic={profile.wifi} onSet={(wifi) => edit((p) => ({ ...p, wifi }))} />
        <OptionalNic
          title={t('hardware.bluetooth')}
          nic={profile.bluetooth}
          onSet={(bluetooth) => edit((p) => ({ ...p, bluetooth }))}
        />
      </div>
    </Section>
  );
}

function InputSection({ profile, edit }: { profile: HardwareProfile; edit: Edit }) {
  const t = useT();
  const input = profile.input;
  const setInput = (patch: Partial<HardwareProfile['input']>) => edit((p) => ({ ...p, input: { ...p.input, ...patch } }));

  return (
    <Section title={t('hardware.input')} description={t('hardware.inputHint')}>
      <div className="space-y-3">
        <Grid cols={4}>
          <Field label={t('hardware.keyboardBus')}>
            {(id) => (
              <Select<InputBus> id={id} value={input.keyboardBus} options={inputBusOptions(t)} onChange={(keyboardBus) => setInput({ keyboardBus })} />
            )}
          </Field>
          <Field label={t('hardware.touchpadBus')}>
            {(id) => (
              <Select<InputBus | ''>
                id={id}
                value={input.touchpadBus ?? ''}
                options={[{ value: '', label: t('hardware.noTouchpad') }, ...inputBusOptions(t)]}
                onChange={(v) => setInput({ touchpadBus: v === '' ? null : v })}
              />
            )}
          </Field>
          <Field label={t('hardware.touchpadVendor')}>
            {(id) => (
              <Select<TouchpadVendor | ''>
                id={id}
                value={input.touchpadVendor ?? ''}
                disabled={input.touchpadBus === null}
                options={[{ value: '', label: t('common.unknown') }, ...touchpadVendorOptions(t)]}
                onChange={(v) => setInput({ touchpadVendor: v === '' ? null : v })}
              />
            )}
          </Field>
          <Field label={t('hardware.touchpadHid')}>
            {(id) => (
              <TextInput
                id={id}
                mono
                value={input.touchpadHid ?? ''}
                placeholder="SYNA2B33"
                disabled={input.touchpadBus === null}
                onChange={(v) => setInput({ touchpadHid: v.trim() ? v.trim().toUpperCase() : null })}
              />
            )}
          </Field>
        </Grid>
        <Toggle checked={input.hasTouchscreen} onChange={(hasTouchscreen) => setInput({ hasTouchscreen })} label={t('hardware.touchscreen')} />
      </div>
    </Section>
  );
}

function StorageSection({ profile }: { profile: HardwareProfile }) {
  const t = useT();
  return (
    <Section title={t('hardware.storage')} flush>
      {profile.storage.length === 0 ? (
        <p className="px-4 py-3 text-sm text-fg-3">{t('hardware.noStorage')}</p>
      ) : (
        <ul className="divide-y divide-line">
          {profile.storage.map((disk, index) => (
            <li key={index} className="flex items-center gap-3 px-4 py-2.5">
              <Badge tone={disk.kind === 'raid' ? 'danger' : 'neutral'}>{t(`enum.storageKind.${disk.kind}`)}</Badge>
              <span className="min-w-0 flex-1 truncate text-base text-fg">{disk.name}</span>
              <span className="text-sm text-fg-3">{formatBytes(toNum(disk.sizeBytes))}</span>
            </li>
          ))}
        </ul>
      )}
      {profile.storage.some((d) => d.kind === 'raid') && (
        <p className="border-t border-line px-4 py-2.5 text-sm text-warn-fg">{t('hardware.raidWarning')}</p>
      )}
    </Section>
  );
}
