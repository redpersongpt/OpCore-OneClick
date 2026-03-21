import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import {
  WINDOWS_HARDWARE_QUERIES,
  normalizeWindowsGpuList,
  resolveGpuVendor,
  type GpuDevice,
} from '../electron/hardwareDetect.js';

describe('hardwareDetect Windows queries', () => {
  test('uses Win32 classes for CPU and GPU detection', () => {
    assert.match(WINDOWS_HARDWARE_QUERIES.cpuName, /Win32_Processor/);
    assert.match(WINDOWS_HARDWARE_QUERIES.cpuVendor, /Win32_Processor/);
    assert.match(WINDOWS_HARDWARE_QUERIES.gpuJson, /Win32_VideoController/);
    assert.doesNotMatch(WINDOWS_HARDWARE_QUERIES.cpuName, /CIM_Processor/);
    assert.doesNotMatch(WINDOWS_HARDWARE_QUERIES.gpuJson, /CIM_VideoController/);
  });

  test('classifies microsoft remote display adapters without poisoning gpu vendor detection', () => {
    assert.equal(resolveGpuVendor(null, 'Microsoft Remote Display Adapter'), 'Microsoft');
  });

  test('drops software display adapters when real pci gpus are present', () => {
    const gpus: GpuDevice[] = [
      {
        name: 'Microsoft Remote Display Adapter',
        vendorId: null,
        deviceId: null,
        vendorName: 'Microsoft',
        confidence: 'partially-detected',
      },
      {
        name: 'Intel(R) UHD Graphics',
        vendorId: '8086',
        deviceId: '46a6',
        vendorName: 'Intel',
        confidence: 'detected',
      },
      {
        name: 'NVIDIA GeForce RTX 4060 Laptop GPU',
        vendorId: '10de',
        deviceId: '28e0',
        vendorName: 'NVIDIA',
        confidence: 'detected',
      },
    ];

    assert.deepEqual(
      normalizeWindowsGpuList(gpus).map((gpu) => gpu.name),
      ['Intel(R) UHD Graphics', 'NVIDIA GeForce RTX 4060 Laptop GPU'],
    );
  });
});
