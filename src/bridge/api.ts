import { invoke } from '@tauri-apps/api/core';
import { toAppError } from './errors';
import type {
  AppVersionInfo,
  BiosSetting,
  BuildOptions,
  BuildPlan,
  BuildResult,
  Catalog,
  CompatibilityReport,
  DiskInfo,
  FirmwareReport,
  FlashConfirmation,
  HardwareProfile,
  MacOsVersion,
  PersistedState,
  PrivilegeStatus,
  RecoveryCacheInfo,
  ScanResult,
  TaskUpdate,
  UpdateInfo,
  ValidationResult,
} from './types';

/** Invoke a backend command; rejections are always normalised `AppError` objects. */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toAppError(err);
  }
}

/**
 * Typed wrappers for every `#[tauri::command]` registered in `src-tauri/src/lib.rs`.
 * Argument names are camelCase; Tauri maps them to the snake_case Rust parameters.
 */
export const api = {
  // App
  getAppInfo: () => call<AppVersionInfo>('get_app_info'),
  checkForUpdates: () => call<UpdateInfo>('check_for_updates'),

  // Hardware
  scanHardware: () => call<ScanResult>('scan_hardware'),
  refreshProfile: (profile: HardwareProfile) => call<HardwareProfile>('refresh_profile', { profile }),
  getCatalog: () => call<Catalog>('get_catalog'),
  exportProfile: (profile: HardwareProfile, path: string) => call<void>('export_profile', { profile, path }),
  importProfile: (path: string) => call<HardwareProfile>('import_profile', { path }),

  // Compatibility, planning, EFI
  checkCompatibility: (profile: HardwareProfile, target: MacOsVersion | null) =>
    call<CompatibilityReport>('check_compatibility', { profile, target }),
  planBuild: (profile: HardwareProfile, options: BuildOptions) =>
    call<BuildPlan>('plan_build', { profile, options }),
  getBiosSettings: (profile: HardwareProfile, target: MacOsVersion) =>
    call<BiosSetting[]>('get_bios_settings', { profile, target }),
  buildEfi: (profile: HardwareProfile, options: BuildOptions) =>
    call<BuildResult>('build_efi', { profile, options }),
  validateEfi: (path: string) => call<ValidationResult>('validate_efi', { path }),
  exportEfi: (efiPath: string, destination: string) => call<string>('export_efi', { efiPath, destination }),

  // Disks
  listUsbDevices: () => call<DiskInfo[]>('list_usb_devices'),
  getDiskInfo: (device: string) => call<DiskInfo>('get_disk_info', { device }),
  checkPrivileges: () => call<PrivilegeStatus>('check_privileges'),
  flashPrepareConfirmation: (device: string, efiPath: string, recovery: MacOsVersion | null) =>
    call<FlashConfirmation>('flash_prepare_confirmation', { device, efiPath, recovery }),
  flashUsb: (device: string, efiPath: string, token: string, recovery: MacOsVersion | null) =>
    call<void>('flash_usb', { device, efiPath, token, recovery }),

  // Firmware
  probeFirmware: () => call<FirmwareReport>('probe_firmware'),

  // Recovery
  downloadRecovery: (version: MacOsVersion) => call<RecoveryCacheInfo>('download_recovery', { version }),
  getCachedRecoveryInfo: (version: MacOsVersion) =>
    call<RecoveryCacheInfo>('get_cached_recovery_info', { version }),
  clearRecoveryCache: () => call<void>('clear_recovery_cache'),

  // Diagnostics
  logGetSessionId: () => call<string>('log_get_session_id'),
  logGetTail: (lines?: number) => call<string>('log_get_tail', { lines: lines ?? null }),
  saveSupportLog: (path: string) => call<void>('save_support_log', { path }),
  clearAppCache: () => call<void>('clear_app_cache'),

  // Persisted state
  getPersistedState: () => call<PersistedState>('get_persisted_state'),
  saveState: (state: PersistedState) => call<void>('save_state', { state }),
  clearState: () => call<void>('clear_state'),

  // Tasks
  taskList: () => call<TaskUpdate[]>('task_list'),
  taskCancel: (taskId: string) => call<boolean>('task_cancel', { taskId }),
};

export type Api = typeof api;
