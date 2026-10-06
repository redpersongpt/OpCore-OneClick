import type {
  HardwareProfile,
  ProfileAudio,
  ProfileGpu,
  ProfileNic,
} from '../bridge/types';
import { stableStringify } from './stable';

/** Starting point for "Enter hardware manually". */
export function blankProfile(): HardwareProfile {
  return {
    cpu: {
      name: '',
      vendor: 'unknown',
      platform: 'unknown',
      codename: '',
      family: null,
      model: null,
      stepping: null,
      cores: 0,
      threads: 0,
      isMobile: false,
      hasAvx2: null,
      hasSse42: null,
      isHybrid: false,
    },
    formFactor: 'desktop',
    vm: null,
    gpus: [],
    audio: null,
    ethernet: [],
    wifi: null,
    bluetooth: null,
    input: {
      keyboardBus: 'usb',
      touchpadBus: null,
      touchpadVendor: null,
      touchpadHid: null,
      hasTouchscreen: false,
    },
    storage: [],
    motherboardVendor: '',
    motherboardModel: '',
    chipset: null,
    ramGb: 0,
    hasBattery: false,
    firmwareUefi: true,
    acpi: null,
    acpiTablesDir: null,
    source: 'manual',
    scanConfidence: 0,
  };
}

export function blankGpu(): ProfileGpu {
  return {
    name: '',
    vendor: 'unknown',
    family: 'unknown',
    vendorId: null,
    deviceId: null,
    subsystemId: null,
    isIgpu: false,
    pciPath: null,
    acpiPath: null,
    vramMb: null,
    disabled: false,
  };
}

export function blankNic(): ProfileNic {
  return { name: '', bus: 'pci', vendorId: null, deviceId: null, subsystemId: null, pciPath: null, macAddress: null };
}

export function blankAudio(): ProfileAudio {
  return {
    codecName: '',
    codecId: null,
    controllerVendorId: null,
    controllerDeviceId: null,
    controllerPciPath: null,
    layoutId: null,
  };
}

export function profileKey(profile: HardwareProfile | null): string {
  return profile ? stableStringify(profile) : '';
}

export function hasIntelWifi(profile: HardwareProfile | null): boolean {
  return profile?.wifi?.vendorId?.toLowerCase() === '8086';
}

/** Short one-line description ("Intel Core i7-9700K · Desktop"). */
export function profileHeadline(profile: HardwareProfile): string {
  return [profile.cpu.name, [profile.motherboardVendor, profile.motherboardModel].filter(Boolean).join(' ')]
    .filter((s) => s && s.trim())
    .join(' · ');
}
