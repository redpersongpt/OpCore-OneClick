import { contextBridge, ipcRenderer } from 'electron';
import type { BiosSettingSelection } from './bios/types.js';
import type { PublicDiagnosticsSnapshot } from './releaseDiagnostics.js';

// ── Renderer crash capture ────────────────────────────────────────────────────
// These run in the renderer context and send errors back to the main process
// logger. They fire before React mounts, so they catch module-load crashes too.
function sendRendererError(type: string, message: string, stack?: string, source?: string, line?: number) {
  try {
    ipcRenderer.send('renderer-error', { type, message, stack, source, line });
  } catch {}
}

window.onerror = (message, source, lineno, _colno, error) => {
  sendRendererError('onerror', String(message), error?.stack, String(source ?? ''), lineno ?? 0);
  return false; // don't suppress default handling
};

window.onunhandledrejection = (event) => {
  const err = event.reason;
  sendRendererError(
    'unhandledrejection',
    err instanceof Error ? err.message : String(err),
    err instanceof Error ? err.stack : undefined,
  );
};

// Signal to main that preload executed successfully (fires before renderer JS runs)
try {
  ipcRenderer.send('renderer-error', {
    type: 'preload-ping',
    message: 'Preload script executed — contextBridge about to be wired',
  });
} catch {}

// ── contextBridge ─────────────────────────────────────────────────────────────
try {
  contextBridge.exposeInMainWorld('electron', {
    // Hardware scan
    scanHardware: () => ipcRenderer.invoke('scan-hardware'),
    getLatestHardwareProfile: () => ipcRenderer.invoke('hardware-profile:get-latest'),
    saveHardwareProfile: (payload: {
      profile: import('./configGenerator').HardwareProfile;
      interpretation?: import('./hardwareProfileArtifact').HardwareProfileInterpretationMetadata | null;
      source?: import('./hardwareProfileArtifact').HardwareProfileArtifact['source'];
    }) => ipcRenderer.invoke('hardware-profile:save', payload),
    exportHardwareProfile: (
      artifact?: import('./hardwareProfileArtifact').HardwareProfileArtifact | null,
    ) => ipcRenderer.invoke('hardware-profile:export', artifact ?? null),
    importHardwareProfile: () => ipcRenderer.invoke('hardware-profile:import'),
    inspectEfiBackupPolicy: (device: string) => ipcRenderer.invoke('efi-backup:inspect-policy', device),

    // EFI build
    buildEFI: (profile: object) => ipcRenderer.invoke('build-efi', profile),

    // Kext fetcher
    fetchLatestKexts: (efiPath: string, kextNames: string[]) =>
      ipcRenderer.invoke('fetch-latest-kexts', efiPath, kextNames),

    // Recovery download
    downloadRecovery: (targetPath: string, macOSVersion: string, startOffset?: number) =>
      ipcRenderer.invoke('download-recovery', targetPath, macOSVersion, startOffset ?? 0),

    // USB
    listUsbDevices: () => ipcRenderer.invoke('list-usb-devices'),
    prepareFlashConfirmation: (device: string, efiPath: string, expectedIdentity?: { devicePath?: string; sizeBytes?: number; model?: string; vendor?: string; serialNumber?: string; transport?: string; removable?: boolean; partitionTable?: string }) =>
      ipcRenderer.invoke('flash:prepare-confirmation', device, efiPath, expectedIdentity),
    flashUsb: (device: string, efiPath: string, confirmed: boolean, confirmationToken?: string | null) =>
      ipcRenderer.invoke('flash-usb', device, efiPath, confirmed, confirmationToken ?? null),

    // EFI validation
    validateEfi: (efiPath: string, profile?: object | null) => ipcRenderer.invoke('validate-efi', efiPath, profile ?? null),

    // Production lock
    enableProductionLock: (efiPath: string, targetOS?: string) => ipcRenderer.invoke('enable-production-lock', efiPath, targetOS),

    // BIOS orchestration
    getBiosState: (profile: object) => ipcRenderer.invoke('bios:get-state', profile),
    applySupportedBiosChanges: (profile: object, selectedChanges: Record<string, BiosSettingSelection>) =>
      ipcRenderer.invoke('bios:apply-supported', profile, selectedChanges),
    verifyManualBiosChanges: (profile: object, selectedChanges: Record<string, BiosSettingSelection>) =>
      ipcRenderer.invoke('bios:verify-manual', profile, selectedChanges),
    restartToFirmwareWithSession: (profile: object, selectedChanges: Record<string, BiosSettingSelection>) =>
      ipcRenderer.invoke('bios:restart-to-firmware', profile, selectedChanges),
    clearBiosSession: () => ipcRenderer.invoke('bios:clear-session'),
    getBiosResumeState: () => ipcRenderer.invoke('bios:resume-state'),
    getBiosRestartCapability: () => ipcRenderer.invoke('bios:restart-capability'),

    // Flow guards
    guardBuild: (profile: object) => ipcRenderer.invoke('flow:guard-build', profile),
    guardDeploy: (profile: object, efiPath: string) => ipcRenderer.invoke('flow:guard-deploy', profile, efiPath),

    // File system / diagnostics
    openFolder: (folderPath: string) => ipcRenderer.invoke('open-folder', folderPath),
    getLogPath: (): Promise<string> => ipcRenderer.invoke('get-log-path'),

    // State Persistence
    getPersistedState: () => ipcRenderer.invoke('get-persisted-state'),
    saveState: (state: object) => ipcRenderer.invoke('save-state', state),
    clearState: () => ipcRenderer.invoke('clear-state'),

    // BIOS & System
    probeBios: () => ipcRenderer.invoke('probe-bios'),
    probeFirmware: () => ipcRenderer.invoke('probe-firmware'),
    restartComputer: () => ipcRenderer.invoke('restart-computer'),
    restartToBios: () => ipcRenderer.invoke('restart-to-bios'),
    disableAutostart: () => ipcRenderer.invoke('disable-autostart'),
    getHardDrives: () => ipcRenderer.invoke('get-hard-drives'),
    shrinkPartition: (disk: string, sizeGB: number, confirmed: boolean) => ipcRenderer.invoke('shrink-partition', disk, sizeGB, confirmed),
    createBootPartition: (disk: string, efiPath: string, confirmed: boolean, profile?: object | null) => ipcRenderer.invoke('create-boot-partition', disk, efiPath, confirmed, profile ?? null),
    getDownloadResumeState: () => ipcRenderer.invoke('get-download-resume-state'),

    // Preflight & safety
    runPreflight: () => ipcRenderer.invoke('run-preflight'),
    getDiskInfo: (device: string) => ipcRenderer.invoke('get-disk-info', device),

    // Task manager — unified task updates
    onTaskUpdate: (callback: (payload: import('./taskManager').TaskUpdatePayload) => void) => {
      ipcRenderer.removeAllListeners('task:update');
      ipcRenderer.on('task:update', (_event, data) => callback(data));
    },
    offTaskUpdate: () => ipcRenderer.removeAllListeners('task:update'),
    taskList: () => ipcRenderer.invoke('task:list'),
    taskCancel: (taskId: string) => ipcRenderer.invoke('task:cancel', taskId),

    // Enhanced logging
    getLogTail: (n: number) => ipcRenderer.invoke('log:get-tail', n),
    getOpsTail: (n: number) => ipcRenderer.invoke('log:get-ops-tail', n),
    logClear: () => ipcRenderer.invoke('log:clear'),
    getSessionId: () => ipcRenderer.invoke('log:get-session-id'),

    // Issue reporter
    reportIssue: (extraContext?: string | null): Promise<{ success: boolean; body: string; baseUrl: string }> =>
      ipcRenderer.invoke('report-issue', extraContext ?? null),

    // Recovery Cache & Import
    importRecovery: (targetPath: string, macOSVersion: string) => ipcRenderer.invoke('recovery:import', targetPath, macOSVersion),
    getCachedRecoveryInfo: (version: string) => ipcRenderer.invoke('recovery:get-cached-info', version),
    clearRecoveryCache: (version: string) => ipcRenderer.invoke('recovery:clear-cache', version),

    // Extended prechecks (PrecheckStep)
    runPrechecks: (): Promise<{
      platform: string;
      adminPrivileges: boolean;
      freeSpaceMB: number;
      networkOk: boolean;
      usbDetected: boolean;
      firmwareDetectionAvailable: boolean;
      missingBinaries: string[];
    }> => ipcRenderer.invoke('run-prechecks'),

    // Prevention Layer — preflight checks
    runPreflightChecks: (kextNames: string[]) => ipcRenderer.invoke('preflight:run', kextNames),
    recordFailure: (code: string, message: string) => ipcRenderer.invoke('preflight:record-failure', code, message),
    shouldSkipRetry: (code: string): Promise<boolean> => ipcRenderer.invoke('preflight:should-skip-retry', code),
    getFailureMemory: () => ipcRenderer.invoke('preflight:failure-memory'),
    clearFailureMemory: () => ipcRenderer.invoke('preflight:clear-memory'),

    // Deterministic Layer — dry-run simulation + hard contracts
    simulateBuild: (kextNames: string[], ssdtNames: string[], smbios: string) =>
      ipcRenderer.invoke('deterministic:simulate-build', kextNames, ssdtNames, smbios),
    dryRunRecovery: (targetOS: string, smbios: string) =>
      ipcRenderer.invoke('deterministic:dry-run-recovery', targetOS, smbios),
    verifyBuildState: (efiPath: string, requiredKexts: string[]) =>
      ipcRenderer.invoke('deterministic:verify-build-state', efiPath, requiredKexts),
    verifyEfiBuildSuccess: (efiPath: string, requiredKexts: string[]) =>
      ipcRenderer.invoke('deterministic:verify-efi-success', efiPath, requiredKexts),
    verifyRecoverySuccess: (recoveryDir: string) =>
      ipcRenderer.invoke('deterministic:verify-recovery-success', recoveryDir),
    runSafeSimulation: (profile: import('./configGenerator').HardwareProfile) =>
      ipcRenderer.invoke('safe-simulation:run', profile),
    getResourcePlan: (profile: import('./configGenerator').HardwareProfile, efiPath?: string | null) =>
      ipcRenderer.invoke('resource-plan:get', profile, efiPath ?? null),

    // Diagnostics snapshot (CopyDiagnosticsButton / ReportStep)
    getDiagnostics: (): Promise<PublicDiagnosticsSnapshot> => ipcRenderer.invoke('get-diagnostics'),
    saveSupportLog: (extraContext?: string | null): Promise<{ fileName: string; savedTo: 'Desktop' }> =>
      ipcRenderer.invoke('log:save-support-log', extraContext ?? null),
    logUiEvent: (eventName: string, detail?: Record<string, unknown> | null): Promise<boolean> =>
      ipcRenderer.invoke('log:ui-event', eventName, detail ?? null),
    notifyRendererReady: (): Promise<boolean> => ipcRenderer.invoke('renderer:ready'),
  });

  // Confirm bridge wired
  ipcRenderer.send('renderer-error', {
    type: 'preload-ping',
    message: 'contextBridge.exposeInMainWorld completed — window.electron is live',
  });
} catch (error) {
  // Bridge failed — signal to main so the crash is logged even if renderer is silent
  sendRendererError('preload-bridge-failed', String((error as Error)?.message ?? error), (error as Error)?.stack);
}
