import { describe, test } from 'vitest';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  generateFolderManifest,
  verifyFolderIntegrity,
} from '../electron/resourceIntegrity.js';

function makeTempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `resource-integrity-${prefix}-`));
}

describe('resourceIntegrity', () => {
  test('accepts an unchanged manifest-backed folder', () => {
    const dir = makeTempDir('valid');
    try {
      fs.mkdirSync(path.join(dir, 'nested'), { recursive: true });
      fs.writeFileSync(path.join(dir, 'nested', 'file.txt'), 'hello');
      fs.writeFileSync(path.join(dir, '.version'), '1.0.0');
      generateFolderManifest(dir);

      const result = verifyFolderIntegrity(dir);
      assert.equal(result.valid, true);
      assert.deepEqual(result.issues, { modified: [], missing: [], untracked: [] });
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('detects modified and untracked files', () => {
    const dir = makeTempDir('changed');
    try {
      fs.writeFileSync(path.join(dir, '.version'), '1.0.0');
      fs.writeFileSync(path.join(dir, 'payload.bin'), 'abc');
      generateFolderManifest(dir);

      fs.writeFileSync(path.join(dir, 'payload.bin'), 'changed');
      fs.writeFileSync(path.join(dir, 'extra.bin'), 'extra');

      const result = verifyFolderIntegrity(dir);
      assert.equal(result.valid, false);
      assert.deepEqual(result.issues.modified, ['payload.bin']);
      assert.deepEqual(result.issues.untracked, ['extra.bin']);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
