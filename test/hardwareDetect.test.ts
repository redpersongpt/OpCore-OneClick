import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import {
  WINDOWS_HARDWARE_QUERIES,
  normalizeWindowsGpuList,
  resolveGpuVendor,
  type GpuDevice,
} from '../electron/hardwareDetect.js';

describe('hardwareDetect Windows queries', () => {
  test('uses the stable known-good v2.3.6 windows detector queries', () => {
    assert.match(WINDOWS_HARDWARE_QUERIES.cpuName, /CIM_Processor/);
    assert.match(WINDOWS_HARDWARE_QUERIES.cpuVendor, /CIM_Processor/);
    assert.match(WINDOWS_HARDWARE_QUERIES.gpuJson, /CIM_VideoController/);
    assert.match(WINDOWS_HARDWARE_QUERIES.chassisTypes, /CIM_SystemEnclosure/);
    assert.match(WINDOWS_HARDWARE_QUERIES.manufacturer, /CIM_ComputerSystem/);
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
