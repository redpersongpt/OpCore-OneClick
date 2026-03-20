import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { DiskIdentityFingerprint } from './flashSafety.js';
import type { ExistingEfiCopyResult, ExistingEfiInspection } from './diskOps.js';

export const EFI_BACKUP_MANIFEST_VERSION = 1;

export type EfiBackupPolicyStatus = 'required' | 'blocked' | 'not_required';

export interface EfiBackupManifest {
  version: typeof EFI_BACKUP_MANIFEST_VERSION;
  backupId: string;
  createdAt: number;
  targetDevice: string;
  diskIdentity: DiskIdentityFingerprint;
  hardwareProfileDigest: string;
  configHash: string | null;
  backupPath: string;
  manifestHash: string;
}

export interface EfiBackupPolicy {
  status: EfiBackupPolicyStatus;
  reason: string;
  existingEfiState: ExistingEfiInspection['status'];
  latestBackup: {
    backupId: string;
    createdAt: number;
    manifestHash: string;
    configHash: string | null;
    backupPath: string;
  } | null;
}

type StableSerializable =
  | null
  | boolean
  | number
  | string
  | StableSerializable[]
  | { [key: string]: StableSerializable };

function stableSerialize(value: StableSerializable): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map((item) => stableSerialize(item)).join(',')}]`;
  return `{${Object.entries(value)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, entryValue]) => `${JSON.stringify(key)}:${stableSerialize(entryValue)}`)
    .join(',')}}`;
}

export function buildEfiBackupManifestHash(input: Omit<EfiBackupManifest, 'manifestHash'>): string {
  return crypto.createHash('sha256').update(stableSerialize(input as unknown as StableSerializable)).digest('hex');
}

export function createEfiBackupManifest(input: {
  targetDevice: string;
  diskIdentity: DiskIdentityFingerprint;
  hardwareProfileDigest: string;
  configHash: string | null;
  backupPath: string;
  createdAt?: number;
}): EfiBackupManifest {
  const createdAt = input.createdAt ?? Date.now();
  const backupId = crypto.createHash('sha256')
    .update(stableSerialize({
      targetDevice: input.targetDevice,
      diskIdentity: input.diskIdentity as unknown as StableSerializable,
      hardwareProfileDigest: input.hardwareProfileDigest,
      createdAt,
    }))
    .digest('hex')
    .slice(0, 24);

  const manifestBase = {
    version: EFI_BACKUP_MANIFEST_VERSION,
    backupId,
    createdAt,
    targetDevice: input.targetDevice,
    diskIdentity: input.diskIdentity,
    hardwareProfileDigest: input.hardwareProfileDigest,
    configHash: input.configHash,
    backupPath: input.backupPath,
  } satisfies Omit<EfiBackupManifest, 'manifestHash'>;

  return {
    ...manifestBase,
    manifestHash: buildEfiBackupManifestHash(manifestBase),
  };
}

export function evaluateEfiBackupPolicy(inspection: ExistingEfiInspection): EfiBackupPolicy {
  if (inspection.status === 'readable') {
    return {
      status: 'required',
      reason: 'A readable existing EFI was found on the target. Backup is mandatory before flash authorization can proceed.',
      existingEfiState: inspection.status,
      latestBackup: null,
    };
  }

  if (inspection.status === 'unreadable') {
    return {
      status: 'blocked',
      reason: inspection.reason ?? 'An existing EFI was detected but could not be read. Flashing is blocked until the target can be inspected safely.',
      existingEfiState: inspection.status,
      latestBackup: null,
    };
  }

  return {
    status: 'not_required',
    reason: 'No existing EFI was found on the selected target, so pre-flash backup is not required.',
    existingEfiState: inspection.status,
    latestBackup: null,
  };
}

export function createEfiBackupManager(basePath: string) {
  const rootPath = path.resolve(basePath, 'efi-backups');
  fs.mkdirSync(rootPath, { recursive: true });

  return {
    rootPath,
    async inspectPolicy(
      device: string,
      inspectExistingEfi: (device: string) => Promise<ExistingEfiInspection>,
    ): Promise<EfiBackupPolicy> {
      return evaluateEfiBackupPolicy(await inspectExistingEfi(device));
    },
    async captureIfRequired(input: {
      targetDevice: string;
      diskIdentity: DiskIdentityFingerprint;
      hardwareProfileDigest: string;
      inspectExistingEfi: (device: string) => Promise<ExistingEfiInspection>;
      copyExistingEfi: (device: string, destinationPath: string) => Promise<ExistingEfiCopyResult>;
      now?: () => number;
    }): Promise<EfiBackupPolicy> {
      const policy = await this.inspectPolicy(input.targetDevice, input.inspectExistingEfi);
      if (policy.status !== 'required') {
        return policy;
      }

      const createdAt = input.now ? input.now() : Date.now();
      const identitySlug = crypto.createHash('sha256')
        .update(stableSerialize(input.diskIdentity as unknown as StableSerializable))
        .digest('hex')
        .slice(0, 12);
      const backupPath = path.resolve(rootPath, `${createdAt}-${identitySlug}`);
      const filesPath = path.resolve(backupPath, 'files');
      fs.mkdirSync(filesPath, { recursive: true });

      const copyResult = await input.copyExistingEfi(input.targetDevice, filesPath);
      if (copyResult.status === 'unreadable') {
        fs.rmSync(backupPath, { recursive: true, force: true });
        return {
          status: 'blocked',
          reason: copyResult.reason ?? 'Existing EFI became unreadable during backup capture. Flashing is blocked.',
          existingEfiState: 'unreadable',
          latestBackup: null,
        };
      }
      if (copyResult.status === 'absent') {
        fs.rmSync(backupPath, { recursive: true, force: true });
        return {
          status: 'not_required',
          reason: 'No existing EFI was found on the target when backup capture was attempted.',
          existingEfiState: 'absent',
          latestBackup: null,
        };
      }

      const manifest = createEfiBackupManifest({
        targetDevice: input.targetDevice,
        diskIdentity: input.diskIdentity,
        hardwareProfileDigest: input.hardwareProfileDigest,
        configHash: copyResult.configHash,
        backupPath,
        createdAt,
      });
      fs.writeFileSync(path.resolve(backupPath, 'manifest.json'), JSON.stringify(manifest, null, 2));

      return {
        status: 'required',
        reason: 'A readable existing EFI was found and backed up before flash authorization.',
        existingEfiState: 'readable',
        latestBackup: {
          backupId: manifest.backupId,
          createdAt: manifest.createdAt,
          manifestHash: manifest.manifestHash,
          configHash: manifest.configHash,
          backupPath: manifest.backupPath,
        },
      };
    },
  };
}
