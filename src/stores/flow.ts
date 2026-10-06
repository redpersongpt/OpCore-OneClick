/**
 * Cross-store actions. Anything that changes an input of a later step
 * (hardware source, profile, target, build options) goes through here so the
 * results that depend on it are dropped and the wizard re-locks the steps
 * that must be redone.
 */
import { api } from '../bridge/api';
import type { DetectedHardware, HardwareProfile, MacOsVersion, PersistedState } from '../bridge/types';
import { demoScanResult } from '../lib/demo';
import { blankProfile, profileKey } from '../lib/profile';
import { useApp } from './app';
import { useBuild, type OptionsDraft } from './build';
import { useCompat } from './compat';
import { useDeploy } from './deploy';
import { useFirmware } from './firmware';
import { useHardware } from './hardware';
import { useTasks } from './tasks';
import { useWizard, type Step } from './wizard';

export type InvalidationScope = 'source' | 'profile' | 'target' | 'options';

function dropBuild(): void {
  const build = useBuild.getState();
  build.clearPlan();
  build.clearResult();
  useDeploy.getState().resetFlash();
}

export function invalidate(scope: InvalidationScope, from?: Step): void {
  const wizard = useWizard.getState();
  switch (scope) {
    case 'source':
      wizard.invalidateFrom(from ?? 'hardware');
      useCompat.getState().clear();
      useFirmware.getState().clearSettings();
      // Ticks on the BIOS checklist were made for the previous machine.
      useFirmware.setState({ checked: {} });
      dropBuild();
      break;
    case 'profile':
      wizard.invalidateFrom(from ?? 'compatibility');
      useCompat.getState().clearReport();
      useFirmware.getState().clearSettings();
      dropBuild();
      break;
    case 'target':
      wizard.invalidateFrom(from ?? 'compatibility');
      useFirmware.getState().clearSettings();
      dropBuild();
      break;
    case 'options':
      wizard.invalidateFrom(from ?? 'build');
      dropBuild();
      break;
  }
}

/** Use a profile from a scan, an import, manual entry or the demo, then open the editor. */
export function applyProfile(profile: HardwareProfile, detected: DetectedHardware | null, isDemo = false): void {
  useHardware.getState().setProfile(profile, detected, isDemo);
  invalidate('source');
  const wizard = useWizard.getState();
  if (wizard.step === 'scan') wizard.complete('scan');
}

export async function runScan(): Promise<boolean> {
  const before = useHardware.getState().profile;
  const ok = await useHardware.getState().scan();
  if (ok) {
    const { profile, detected } = useHardware.getState();
    if (profile && profileKey(profile) !== profileKey(before)) {
      useHardware.getState().setProfile(profile, detected, false);
      invalidate('source');
    }
  }
  return ok;
}

export function startManual(): void {
  applyProfile(blankProfile(), null);
}

export function startDemo(): void {
  const demo = demoScanResult();
  applyProfile(demo.profile, demo.detected, true);
}

export async function importProfile(path: string): Promise<boolean> {
  const profile = await useHardware.getState().importFrom(path);
  if (!profile) return false;
  // "scan" means "this computer"; a profile from a file may describe another one
  // (the firmware probe on the BIOS step must not be applied to it).
  applyProfile(profile.source === 'scan' ? { ...profile, source: 'imported' } : profile, null);
  return true;
}

/**
 * Local edit of the profile. The hardware step itself must be confirmed again,
 * so later steps never see an edit that `refresh_profile` has not interpreted.
 */
export function editProfile(update: (profile: HardwareProfile) => HardwareProfile): void {
  useHardware.getState().editProfile(update);
  invalidate('profile', 'hardware');
}

/** Re-interpret the edited profile; returns false when the backend rejected it. */
export async function refreshProfile(): Promise<boolean> {
  const before = profileKey(useHardware.getState().profile);
  const next = await useHardware.getState().refresh();
  if (!next) return false;
  if (profileKey(next) !== before) invalidate('profile');
  return true;
}

/**
 * Change the target macOS. From the compatibility step the step itself must be
 * confirmed again; from the build step only the build is dropped (the build
 * page re-checks compatibility for the new target before allowing a build).
 */
export function selectTarget(target: MacOsVersion, origin: 'compatibility' | 'build' = 'compatibility'): void {
  const compat = useCompat.getState();
  if (compat.target === target && compat.report) return;
  compat.setTarget(target);
  invalidate('target', origin === 'build' ? 'build' : 'compatibility');
  const profile = useHardware.getState().profile;
  if (profile) void useCompat.getState().check(profile, target);
}

export function setOptions(patch: Partial<OptionsDraft>): void {
  const draft = useBuild.getState().draft;
  const changed = (Object.keys(patch) as (keyof OptionsDraft)[]).filter((k) => patch[k] !== draft[k]);
  if (changed.length === 0) return;
  useBuild.getState().setDraft(patch);
  // Reusing the previous serials or not never makes the current EFI stale.
  if (changed.some((k) => k !== 'keepIdentity')) invalidate('options');
}

/** Builds and USB images were deleted from disk (Settings → clear cache). */
export function afterCacheCleared(): void {
  useBuild.getState().clearResult();
  useBuild.getState().setPreviousEfiPath(null);
  useDeploy.getState().resetFlash();
  useDeploy.getState().resetRecovery();
  useWizard.getState().invalidateFrom('build');
}

export function resetAll(): void {
  useHardware.getState().clear();
  useCompat.getState().clear();
  useFirmware.getState().clear();
  useBuild.getState().clear();
  useDeploy.getState().clear();
  useTasks.getState().clear();
  useWizard.getState().reset();
}

/** Discard everything (including the saved session) and start at the scan step. */
export async function startOver(): Promise<void> {
  if (useWizard.getState().isLocked()) return;
  resetAll();
  useApp.getState().dismissPersisted();
  useWizard.getState().complete('welcome');
  try {
    await api.clearState();
  } catch {
    // Nothing saved, or the backend is unavailable.
  }
}

/** Continue a saved session: profile, target and identity are restored; the build is redone. */
export function resume(state: PersistedState): void {
  if (!state.profile || useWizard.getState().isLocked()) return;
  resetAll();
  useHardware.getState().setProfile(state.profile, null, state.profile.source === 'demo');
  if (state.target) useCompat.getState().setTarget(state.target);
  useBuild.getState().setIdentity(state.identity);
  useBuild.getState().setPreviousEfiPath(state.efiPath);
  useWizard.getState().restore('compatibility', ['welcome', 'scan', 'hardware']);
  useApp.getState().dismissPersisted();
}

/** Snapshot saved with `save_state`, or null when there is nothing worth saving. */
export function persistedSnapshot(): PersistedState | null {
  const { profile, isDemo } = useHardware.getState();
  if (!profile || isDemo) return null;
  const build = useBuild.getState();
  return {
    currentStep: useWizard.getState().step,
    profile,
    target: useCompat.getState().target,
    identity: build.result?.identity ?? build.identity,
    efiPath: build.result?.efiPath ?? build.previousEfiPath,
    timestamp: null,
  };
}
