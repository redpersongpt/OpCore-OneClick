import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { afterEach, describe, test } from 'vitest';
import type { HardwareProfile } from '../electron/configGenerator.js';
import { runEfiBuildFlow, type EfiBuildRegistry } from '../electron/efiBuildFlow.js';

function makeProfile(overrides: Partial<HardwareProfile> = {}): HardwareProfile {
  return {
    cpu: 'Intel Core i5-8250U',
    architecture: 'Intel',
    generation: 'Kaby Lake',
    coreCount: 4,
    gpu: 'Intel UHD Graphics 620',
    gpuDevices: [{ name: 'Intel UHD Graphics 620', vendorName: 'Intel' }],
    ram: '16 GB',
    motherboard: 'Lenovo 20L5',
    targetOS: 'macOS Ventura 13',
    smbios: 'MacBookPro14,1',
    kexts: [],
    ssdts: [],
    bootArgs: '-v',
    isLaptop: true,
    isVM: false,
    audioLayoutId: 3,
    strategy: 'canonical',
    scanConfidence: 'high',
    ...overrides,
  };
}

function makeRegistry(events: string[]): EfiBuildRegistry {
  return {
    create: () => ({
      taskId: 'task-1',
      aborted: false,
      abort: () => {},
      check: () => {},
      registerProcess: () => {},
    }),
    updateProgress: (_taskId, payload) => {
      events.push(`progress:${payload.phase}`);
    },
    complete: (taskId) => {
      events.push(`complete:${taskId}`);
    },
    fail: (taskId, message) => {
      events.push(`fail:${taskId}:${message}`);
    },
    cancel: (taskId) => {
      events.push(`cancel:${taskId}`);
    },
  };
}

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    const target = tempDirs.pop()!;
    fs.rmSync(target, { recursive: true, force: true });
  }
});

describe('runEfiBuildFlow', () => {
  test('accepted BIOS session resolves the EFI build path instead of hanging', async () => {
    const events: string[] = [];
    const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'efi-build-flow-'));
    tempDirs.push(tmpRoot);
    const ensureCalls: Array<{ allowAcceptedSession?: boolean }> = [];
    const structureCalls: string[] = [];

    const efiPath = await runEfiBuildFlow(
      {
        profile: makeProfile(),
        allowAcceptedSession: true,
      },
      {
        registry: makeRegistry(events),
        getUserDataPath: () => tmpRoot,
        log: () => {},
        rememberFailureContext: () => {},
        checkCompatibility: () => ({
          isCompatible: true,
          errors: [],
          explanation: 'ok',
        }),
        ensureBiosReady: async (_profile, options) => {
          ensureCalls.push(options ?? {});
        },
        createEfiStructure: async (targetPath) => {
          structureCalls.push(targetPath);
          fs.mkdirSync(path.join(targetPath, 'EFI', 'OC'), { recursive: true });
          fs.writeFileSync(path.join(targetPath, 'EFI', 'OC', 'OpenCore.efi'), '');
        },
        withTimeout: async (promise) => await promise,
        classifyError: () => ({
          message: 'classified failure',
          explanation: 'classified failure',
          category: 'build_error',
        }),
        createClassifiedIpcError: (_classified, error) => error as Error,
        removeDir: (targetPath) => {
          fs.rmSync(targetPath, { recursive: true, force: true });
        },
      },
    );

    assert.equal(ensureCalls.length, 1);
    assert.deepEqual(ensureCalls[0], { allowAcceptedSession: true });
    assert.equal(structureCalls.length, 1);
    assert.equal(structureCalls[0], efiPath);
    assert.ok(fs.existsSync(path.join(efiPath, 'EFI', 'OC', 'OpenCore.efi')));
    assert.ok(events.includes('progress:initialising'));
    assert.ok(events.includes('complete:task-1'));
  });

  test('emits build progress before waiting on BIOS readiness', async () => {
    const events: string[] = [];
    const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'efi-build-flow-'));
    tempDirs.push(tmpRoot);

    await runEfiBuildFlow(
      {
        profile: makeProfile(),
        allowAcceptedSession: false,
      },
      {
        registry: makeRegistry(events),
        getUserDataPath: () => tmpRoot,
        log: () => {},
        rememberFailureContext: () => {},
        checkCompatibility: () => ({
          isCompatible: true,
          errors: [],
          explanation: 'ok',
        }),
        ensureBiosReady: async () => {
          assert.equal(events[0], 'progress:checking BIOS state');
        },
        createEfiStructure: async (targetPath) => {
          fs.mkdirSync(path.join(targetPath, 'EFI', 'OC'), { recursive: true });
          fs.writeFileSync(path.join(targetPath, 'EFI', 'OC', 'OpenCore.efi'), '');
        },
        withTimeout: async (promise) => await promise,
        classifyError: () => ({
          message: 'classified failure',
          explanation: 'classified failure',
          category: 'build_error',
        }),
        createClassifiedIpcError: (_classified, error) => error as Error,
        removeDir: (targetPath) => {
          fs.rmSync(targetPath, { recursive: true, force: true });
        },
      },
    );

    assert.ok(events.includes('progress:checking BIOS state'));
    assert.ok(events.includes('complete:task-1'));
  });
});
