import assert from 'node:assert/strict';
import { describe, test } from 'vitest';
import { resolveScanSuccessStep } from '../src/lib/scanFlow';

describe('scanFlow', () => {
  test('defaults to version-select when a click event or unknown value is passed', () => {
    assert.equal(resolveScanSuccessStep(undefined), 'version-select');
    assert.equal(resolveScanSuccessStep(null), 'version-select');
    assert.equal(resolveScanSuccessStep({ type: 'click', currentTarget: { dataset: {} } }), 'version-select');
  });

  test('preserves explicit scan destinations used by report rescans', () => {
    assert.equal(resolveScanSuccessStep('version-select'), 'version-select');
    assert.equal(resolveScanSuccessStep('report'), 'report');
  });
});
