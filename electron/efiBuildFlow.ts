import fs from 'node:fs';
import path from 'node:path';
import type { HardwareProfile } from './configGenerator.js';
import type { OpToken } from './taskManager.js';

export interface EfiBuildRegistry {
  create(kind: 'efi-build'): OpToken;
  updateProgress(taskId: string, payload: { kind: 'efi-build'; phase: string; detail: string }): void;
  complete(taskId: string): void;
  fail(taskId: string, message: string): void;
  cancel(taskId: string): void;
}

export interface ClassifiedBuildError {
  message: string;
  explanation?: string | null;
  category?: string | null;
}

export interface RunEfiBuildFlowDependencies {
  registry: EfiBuildRegistry;
  getUserDataPath(): string;
  log(level: string, area: string, message: string, detail?: Record<string, unknown>): void;
  rememberFailureContext(input: {
    trigger: string;
    message: string;
    detail?: string | null;
    code?: string | null;
  }): void;
  checkCompatibility(profile: HardwareProfile): {
    isCompatible: boolean;
    errors: string[];
    explanation: string;
  };
  ensureBiosReady(
    profile: HardwareProfile,
    options?: { allowAcceptedSession?: boolean },
  ): Promise<void>;
  createEfiStructure(
    efiPath: string,
    profile: HardwareProfile,
    token: OpToken,
    onPhase?: (phase: string, detail: string) => void,
  ): Promise<void>;
  withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T>;
  classifyError(error: unknown): ClassifiedBuildError;
  createClassifiedIpcError(classified: ClassifiedBuildError, error: unknown): Error;
  removeDir(targetPath: string): void;
}

export interface RunEfiBuildFlowInput {
  profile: HardwareProfile;
  allowAcceptedSession?: boolean;
}

export async function runEfiBuildFlow(
  input: RunEfiBuildFlowInput,
  deps: RunEfiBuildFlowDependencies,
): Promise<string> {
  const { profile, allowAcceptedSession } = input;
  const token = deps.registry.create('efi-build');
  const efiPath = path.resolve(deps.getUserDataPath(), 'EFI_Build_' + Date.now());
  deps.log('INFO', 'efi', 'Building EFI', {
    efiPath,
    cpu: profile.cpu,
    smbios: profile.smbios,
    taskId: token.taskId,
  });

  const compatibility = deps.checkCompatibility(profile);
  if (!compatibility.isCompatible || compatibility.errors.length > 0) {
    deps.rememberFailureContext({
      trigger: 'efi_build_failure',
      message: compatibility.errors[0] ?? compatibility.explanation,
      detail: compatibility.explanation,
    });
    deps.registry.fail(token.taskId, compatibility.errors[0] ?? compatibility.explanation);
    throw new Error(compatibility.errors[0] ?? 'Hardware compatibility is blocked for this EFI build.');
  }

  deps.registry.updateProgress(token.taskId, {
    kind: 'efi-build',
    phase: 'checking BIOS state',
    detail: allowAcceptedSession === true
      ? 'Using the accepted BIOS session for this non-destructive EFI build.'
      : 'Verifying that BIOS preparation is complete before the EFI build starts.',
  });

  try {
    await deps.ensureBiosReady(profile, { allowAcceptedSession: allowAcceptedSession === true });
  } catch (error) {
    const classified = deps.classifyError(error);
    deps.rememberFailureContext({
      trigger: 'efi_build_failure',
      message: classified.message,
      detail: classified.explanation,
      code: classified.category,
    });
    deps.registry.fail(token.taskId, classified.message);
    throw deps.createClassifiedIpcError(classified, error);
  }

  if (!fs.existsSync(efiPath)) fs.mkdirSync(efiPath, { recursive: true });

  try {
    deps.registry.updateProgress(token.taskId, {
      kind: 'efi-build',
      phase: 'initialising',
      detail: 'Preparing build environment',
    });

    await deps.withTimeout(
      deps.createEfiStructure(efiPath, profile, token, (phase, detail) => {
        deps.registry.updateProgress(token.taskId, { kind: 'efi-build', phase, detail });
      }),
      120_000,
      'createEfiStructure',
    );

    deps.registry.updateProgress(token.taskId, {
      kind: 'efi-build',
      phase: 'EFI structure complete',
      detail: 'Base OpenCore files and placeholders are ready for validation.',
    });
    deps.registry.complete(token.taskId);
    deps.log('INFO', 'efi', 'EFI build complete', { efiPath });
    return efiPath;
  } catch (error) {
    const classified = deps.classifyError(error);
    deps.rememberFailureContext({
      trigger: 'efi_build_failure',
      message: classified.message,
      detail: classified.explanation,
      code: classified.category,
    });
    deps.log('ERROR', 'efi', 'EFI build failed', { error: classified.explanation });
    if (!token.aborted) deps.registry.fail(token.taskId, classified.message);
    else deps.registry.cancel(token.taskId);
    deps.removeDir(efiPath);
    throw deps.createClassifiedIpcError(classified, error);
  }
}
