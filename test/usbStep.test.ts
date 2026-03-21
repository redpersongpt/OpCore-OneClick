import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import { classifyDrive, type DriveInfo } from '../src/components/steps/UsbStep';

function makeDrive(overrides: Partial<DriveInfo> = {}): DriveInfo {
  return {
    name: 'SanDisk Ultra',
    device: '\\\\.\\PhysicalDrive3',
    size: '31.9 GB',
    ...overrides,
  };
}

describe('UsbStep drive classification', () => {
  test('keeps unverified USB drives selectable while details are still loading', () => {
    const result = classifyDrive(makeDrive(), true, { allowUnverifiedSelection: true });

    assert.equal(result.tier, 'safe');
    assert.equal(result.pendingVerification, true);
    assert.equal(result.reasons[0]?.code, 'UNVERIFIED');
  });

  test('still treats unverified drives as suspicious when eager selection is disabled', () => {
    const result = classifyDrive(makeDrive(), true);

    assert.equal(result.tier, 'suspicious');
    assert.equal(result.reasons[0]?.code, 'UNVERIFIED');
  });

  test('blocks drives whose size cannot be determined', () => {
    const result = classifyDrive(makeDrive({ size: 'Unknown' }), true, { allowUnverifiedSelection: true });

    assert.equal(result.tier, 'blocked');
    assert.equal(result.reasons[0]?.code, 'DEVICE_SIZE_UNKNOWN');
  });

  test('still blocks internal or system disks after verification', () => {
    const result = classifyDrive(makeDrive({
      isSystemDisk: true,
      partitionTable: 'gpt',
      removable: false,
    }), true, { allowUnverifiedSelection: true });

    assert.equal(result.tier, 'blocked');
  });
});
