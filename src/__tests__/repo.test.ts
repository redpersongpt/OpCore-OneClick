import { execFileSync } from 'node:child_process';
import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const ROOT = join(__dirname, '..', '..');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.(tsx?|css|json|svg)$/.test(name) ? [relative(ROOT, path)] : [];
  });
}

function ignoredBy(paths: string[]): string[] | null {
  try {
    const out = execFileSync('git', ['check-ignore', '--no-index', '--stdin'], {
      cwd: ROOT,
      input: paths.join('\n'),
      encoding: 'utf8',
    });
    return out.split('\n').filter(Boolean);
  } catch (err) {
    // Exit status 1 means "nothing ignored"; anything else means git is unavailable.
    const status = (err as { status?: number }).status;
    return status === 1 ? [] : null;
  }
}

describe('repository', () => {
  // `.gitignore` has broad patterns such as `build/`; a source folder with such a
  // name would silently be left out of every commit.
  it('no frontend source file is matched by .gitignore', () => {
    const ignored = ignoredBy(files(join(ROOT, 'src')));
    if (ignored === null) return;
    expect(ignored).toEqual([]);
  });
});
