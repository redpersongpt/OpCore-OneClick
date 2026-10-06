#!/usr/bin/env node
// Prints the CHANGELOG.md section of one version, for use as GitHub release
// notes. Accepts a version or a "v"-prefixed tag. Falls back to a pointer to
// the changelog when the version has no section.
//
//   node scripts/changelog-notes.mjs v6.0.0 > notes.md

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const arg = process.argv[2];
if (!arg) {
  console.error('usage: node scripts/changelog-notes.mjs <version|tag>');
  process.exit(2);
}
const version = arg.replace(/^refs\/tags\//, '').replace(/^v/, '');
const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
// "## 6.0.0", "## [6.0.0]", "## 6.0.0 - 2026-10-05", "## 6.0.0 — unreleased"
const heading = new RegExp(`^## \\[?v?${escaped}\\]?(?:\\s|$)`);

const lines = readFileSync(join(root, 'CHANGELOG.md'), 'utf8').split(/\r?\n/);
const start = lines.findIndex((line) => heading.test(line));

const repo = process.env.GITHUB_REPOSITORY ?? 'redpersongpt/OpCore-OneClick';
const footer = [
  '',
  '---',
  '',
  'Verify a download with `SHA256SUMS.txt` from this release:',
  '`sha256sum -c SHA256SUMS.txt --ignore-missing` (Linux), `shasum -a 256 -c SHA256SUMS.txt --ignore-missing` (macOS)',
  'or `Get-FileHash <file> -Algorithm SHA256` (Windows).',
];

if (start === -1) {
  console.log(
    [`See [CHANGELOG.md](https://github.com/${repo}/blob/main/CHANGELOG.md) for the changes in ${version}.`, ...footer].join('\n'),
  );
  process.exit(0);
}

let end = lines.findIndex((line, index) => index > start && /^## /.test(line));
if (end === -1) end = lines.length;

const body = lines.slice(start + 1, end).join('\n').trim();
console.log([body, ...footer].join('\n'));
