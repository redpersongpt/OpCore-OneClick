import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import { inferLaptopFormFactor } from '../electron/formFactor.js';

describe('inferLaptopFormFactor', () => {
  test('does not classify a desktop as a laptop from cpu suffix alone', () => {
    const result = inferLaptopFormFactor({
      cpuName: '13th Gen Intel(R) Core(TM) i7-13620H',
      chassisTypes: [3],
      modelName: 'MS-7D25',
      batteryPresent: false,
    });

    assert.equal(result, false);
  });

  test('classifies a real laptop when mobile cpu and battery are both present', () => {
    const result = inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i7-10750H CPU @ 2.60GHz',
      chassisTypes: [3],
      modelName: 'ROG Strix G15',
      batteryPresent: true,
    });

    assert.equal(result, true);
  });

  test('classifies a laptop from portable chassis even without battery data', () => {
    const result = inferLaptopFormFactor({
      cpuName: 'Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz',
      chassisTypes: [10],
      modelName: 'ThinkPad T480',
      batteryPresent: false,
    });

    assert.equal(result, true);
  });
});
