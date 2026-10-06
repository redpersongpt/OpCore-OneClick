import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(__dirname, '..');

/**
 * Tailwind v3 accepted `bg-[--token]` as shorthand for `bg-[var(--token)]`.
 * Tailwind v4 compiles it to invalid CSS (`background-color: --token`), so the
 * rule is silently dropped. v4 syntax is `bg-(--token)`.
 */
export const LEGACY_VAR_SHORTHAND = /[\w:/.-]-\[--[\w-]+\](?:\/\d+)?/g;

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (name === 'generated' || name === 'node_modules') continue;
      out.push(...sourceFiles(path));
    } else if (/\.(tsx?|css|html)$/.test(name) && !name.endsWith('.test.ts')) {
      out.push(path);
    }
  }
  return out;
}

describe('Tailwind v4 class syntax', () => {
  it('recognises the legacy shorthand', () => {
    expect('bg-[--color-green-1]'.match(LEGACY_VAR_SHORTHAND)).toHaveLength(1);
    expect('hover:border-[--accent]/30'.match(LEGACY_VAR_SHORTHAND)).toHaveLength(1);
    expect('bg-(--color-green-1) text-[12px] bg-[var(--x)]'.match(LEGACY_VAR_SHORTHAND)).toBeNull();
  });

  it('no source file uses the v3-only `-[--var]` shorthand', () => {
    const offenders: string[] = [];
    for (const file of [...sourceFiles(SRC), join(SRC, '..', 'index.html')]) {
      const text = readFileSync(file, 'utf8');
      for (const match of text.match(LEGACY_VAR_SHORTHAND) ?? []) {
        offenders.push(`${relative(join(SRC, '..'), file)}: ${match}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
