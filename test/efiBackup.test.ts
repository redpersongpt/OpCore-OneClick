import React from 'react';
import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
import {
  buildEfiBackupManifestHash,
  createEfiBackupManager,
  createEfiBackupManifest,
  evaluateEfiBackupPolicy,
} from '../electron/efiBackup.js';
import EfiBackupPanel from '../src/components/EfiBackupPanel.js';

function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `efi-backup-${prefix}-`));
}

const diskIdentity = {
  devicePath: '/dev/disk4',
  serialNumber: 'USB-12345',
  partitionTable: 'gpt' as const,
  sizeBytes: 32_000_000_000,
  removable: true,
};

describe('efi backup policy and manifest', () => {
  test('maps readable, unreadable, and absent EFI states to the required policy states', () => {
    assert.equal(evaluateEfiBackupPolicy({ status: 'readable' }).status, 'required');
    assert.equal(
      evaluateEfiBackupPolicy({ status: 'unreadable', reason: 'mount failed' }).status,
      'blocked',
    );
    assert.equal(evaluateEfiBackupPolicy({ status: 'absent' }).status, 'not_required');
  });

  test('produces stable manifest hashes and preserves hardware-profile binding', () => {
    const manifestA = createEfiBackupManifest({
      targetDevice: '/dev/disk4',
      diskIdentity,
      hardwareProfileDigest: 'profile-digest-123',
      configHash: 'config-hash-abc',
      backupPath: '/tmp/backup-a',
      createdAt: 1_700_000_000_000,
    });
    const manifestB = createEfiBackupManifest({
      targetDevice: '/dev/disk4',
      diskIdentity,
      hardwareProfileDigest: 'profile-digest-123',
      configHash: 'config-hash-abc',
      backupPath: '/tmp/backup-a',
      createdAt: 1_700_000_000_000,
    });

    assert.equal(manifestA.manifestHash, manifestB.manifestHash);
    assert.equal(
      manifestA.manifestHash,
      buildEfiBackupManifestHash({
        version: manifestA.version,
        backupId: manifestA.backupId,
        createdAt: manifestA.createdAt,
        targetDevice: manifestA.targetDevice,
        diskIdentity: manifestA.diskIdentity,
        hardwareProfileDigest: manifestA.hardwareProfileDigest,
        configHash: manifestA.configHash,
        backupPath: manifestA.backupPath,
      }),
    );
    assert.equal(manifestA.hardwareProfileDigest, 'profile-digest-123');
  });

  test('captures config hash and writes a manifest when backup is required', async () => {
    const dir = makeTempDir('capture');
    const manager = createEfiBackupManager(dir);

    const policy = await manager.captureIfRequired({
      targetDevice: '/dev/disk4',
      diskIdentity,
      hardwareProfileDigest: 'profile-digest-123',
      inspectExistingEfi: async () => ({ status: 'readable' }),
      copyExistingEfi: async (_device, destinationPath) => {
        fs.mkdirSync(path.join(destinationPath, 'EFI', 'OC'), { recursive: true });
        fs.writeFileSync(path.join(destinationPath, 'EFI', 'OC', 'config.plist'), '<plist />');
        return {
          status: 'readable',
          configHash: 'config-hash-abc',
        };
      },
      now: () => 1_700_000_000_000,
    });

    assert.equal(policy.status, 'required');
    assert.ok(policy.latestBackup);
    assert.equal(policy.latestBackup?.configHash, 'config-hash-abc');

    const manifestPath = path.join(policy.latestBackup!.backupPath, 'manifest.json');
    assert.equal(fs.existsSync(manifestPath), true);
    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
    assert.equal(manifest.hardwareProfileDigest, 'profile-digest-123');
    assert.equal(manifest.configHash, 'config-hash-abc');
  });

  test('hard-blocks when an existing EFI becomes unreadable during capture', async () => {
    const dir = makeTempDir('blocked');
    const manager = createEfiBackupManager(dir);

    const policy = await manager.captureIfRequired({
      targetDevice: '/dev/disk4',
      diskIdentity,
      hardwareProfileDigest: 'profile-digest-123',
      inspectExistingEfi: async () => ({ status: 'readable' }),
      copyExistingEfi: async () => ({
        status: 'unreadable',
        reason: 'EFI partition could not be mounted read-only.',
        configHash: null,
      }),
      now: () => 1_700_000_000_000,
    });

    assert.equal(policy.status, 'blocked');
    assert.match(policy.reason, /could not be mounted read-only/i);
  });

  test('does not capture a backup when no existing EFI is present', async () => {
    const dir = makeTempDir('absent');
    const manager = createEfiBackupManager(dir);
    let copyCalls = 0;

    const policy = await manager.captureIfRequired({
      targetDevice: '/dev/disk4',
      diskIdentity,
      hardwareProfileDigest: 'profile-digest-123',
      inspectExistingEfi: async () => ({ status: 'absent' }),
      copyExistingEfi: async () => {
        copyCalls += 1;
        return { status: 'readable', configHash: 'unexpected' };
      },
    });

    assert.equal(policy.status, 'not_required');
    assert.equal(copyCalls, 0);
  });

  test('renders backup required, blocked, and not-needed states clearly', () => {
    const requiredHtml = renderToStaticMarkup(React.createElement(EfiBackupPanel, {
      policy: {
        status: 'required',
        reason: 'Backup is mandatory before flashing.',
        existingEfiState: 'readable',
        latestBackup: null,
      },
    }));
    const blockedHtml = renderToStaticMarkup(React.createElement(EfiBackupPanel, {
      policy: {
        status: 'blocked',
        reason: 'Existing EFI could not be inspected safely.',
        existingEfiState: 'unreadable',
        latestBackup: null,
      },
    }));
    const notNeededHtml = renderToStaticMarkup(React.createElement(EfiBackupPanel, {
      policy: {
        status: 'not_required',
        reason: 'No existing EFI was found.',
        existingEfiState: 'absent',
        latestBackup: null,
      },
    }));

    assert.match(requiredHtml, /Backup Required/);
    assert.match(requiredHtml, /Backup is mandatory before flashing\./);
    assert.match(blockedHtml, /Backup Blocked/);
    assert.match(blockedHtml, /Existing EFI could not be inspected safely\./);
    assert.match(notNeededHtml, /Backup Not Needed/);
    assert.match(notNeededHtml, /No existing EFI was found\./);
  });
});
