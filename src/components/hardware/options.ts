import type {
  CatalogOption,
  CpuVendor,
  DeviceBus,
  GpuVendor,
  InputBus,
  TouchpadVendor,
  VmKind,
} from '../../bridge/types';
import type { MessageKey, Translate } from '../../i18n';
import type { SelectOption } from '../ui/Field';

export function catalogOptions(options: readonly CatalogOption[] | undefined): SelectOption<string>[] {
  return (options ?? []).map((o) => ({ value: o.id, label: o.detail ? `${o.label} — ${o.detail}` : o.label }));
}

function translated<V extends string>(t: Translate, values: readonly V[], prefix: string): SelectOption<V>[] {
  return values.map((value) => ({ value, label: t(`${prefix}.${value}` as MessageKey) }));
}

export const CPU_VENDORS: readonly CpuVendor[] = ['intel', 'amd', 'unknown'];
export const GPU_VENDORS: readonly GpuVendor[] = ['intel', 'amd', 'nvidia', 'virtual', 'unknown'];
export const DEVICE_BUSES: readonly DeviceBus[] = ['pci', 'usb', 'sdio', 'unknown'];
export const INPUT_BUSES: readonly InputBus[] = ['ps2', 'i2c', 'smbus', 'usb', 'unknown'];
export const TOUCHPAD_VENDORS: readonly TouchpadVendor[] = ['synaptics', 'elan', 'alps', 'other', 'unknown'];
export const VM_KINDS: readonly VmKind[] = ['kvm', 'vmware', 'hyper_v', 'virtual_box', 'parallels', 'other'];

export const cpuVendorOptions = (t: Translate) => translated(t, CPU_VENDORS, 'enum.cpuVendor');
export const gpuVendorOptions = (t: Translate) => translated(t, GPU_VENDORS, 'enum.gpuVendor');
export const deviceBusOptions = (t: Translate) => translated(t, DEVICE_BUSES, 'enum.deviceBus');
export const inputBusOptions = (t: Translate) => translated(t, INPUT_BUSES, 'enum.inputBus');
export const touchpadVendorOptions = (t: Translate) => translated(t, TOUCHPAD_VENDORS, 'enum.touchpadVendor');

/** VM kind plus "none" (encoded as the empty string). */
export function vmOptions(t: Translate): SelectOption<VmKind | ''>[] {
  return [{ value: '', label: t('enum.vm.none') }, ...translated(t, VM_KINDS, 'enum.vm')];
}

/** Tri-state for Option<bool> fields ("" = unknown). */
export type TriState = '' | 'yes' | 'no';
export const toTri = (v: boolean | null): TriState => (v === null ? '' : v ? 'yes' : 'no');
export const fromTri = (v: TriState): boolean | null => (v === '' ? null : v === 'yes');
export function triOptions(t: Translate): SelectOption<TriState>[] {
  return [
    { value: '', label: t('common.unknown') },
    { value: 'yes', label: t('common.yes') },
    { value: 'no', label: t('common.no') },
  ];
}
