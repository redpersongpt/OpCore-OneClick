#!/usr/bin/env node
// Verifies that every place that carries the app version agrees:
// package.json, package-lock.json, src-tauri/Cargo.toml and
// src-tauri/tauri.conf.json. With an argument (a version or a "v"-prefixed
// tag) it also checks that the version matches it.
//
//   node scripts/check-version.mjs            # consistency only
//   node scripts/check-version.mjs v6.0.0     # consistency + expected version

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (path) => readFileSync(join(root, path), 'utf8');

function cargoPackageVersion(toml) {
  let inPackage = false;
  for (const raw of toml.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith('[')) {
      inPackage = line === '[package]';
      continue;
    }
    if (!inPackage) continue;
    const match = line.match(/^version\s*=\s*"([^"]+)"/);
    if (match) return match[1];
  }
  return undefined;
}

function cargoLockVersion(lockfile, name) {
  const match = lockfile.match(new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = "([^"]+)"`));
  return match?.[1];
}

function tauriConfigVersion() {
  const version = JSON.parse(read('src-tauri/tauri.conf.json')).version;
  // Tauri also accepts a path to a package.json whose version it reuses.
  if (typeof version === 'string' && version.endsWith('package.json')) {
    return JSON.parse(readFileSync(resolve(root, 'src-tauri', version), 'utf8')).version;
  }
  return version;
}

const lock = JSON.parse(read('package-lock.json'));
const cargoToml = read('src-tauri/Cargo.toml');
const crateName = cargoToml.match(/^name\s*=\s*"([^"]+)"/m)?.[1] ?? 'opcore-oneclick';
const sources = {
  'package.json': JSON.parse(read('package.json')).version,
  'package-lock.json': lock.version,
  'package-lock.json (root package)': lock.packages?.['']?.version,
  'src-tauri/Cargo.toml': cargoPackageVersion(cargoToml),
  'src-tauri/Cargo.lock': cargoLockVersion(read('src-tauri/Cargo.lock'), crateName),
  'src-tauri/tauri.conf.json': tauriConfigVersion(),
};

const expectedArg = process.argv[2];
const expected = expectedArg ? expectedArg.replace(/^refs\/tags\//, '').replace(/^v/, '') : sources['package.json'];

let ok = true;
for (const [file, version] of Object.entries(sources)) {
  const matches = version === expected;
  ok &&= matches;
  console.log(`${matches ? 'ok      ' : 'MISMATCH'} ${file}: ${version ?? '(missing)'}`);
}

if (!ok) {
  console.error(`\nExpected every version to be ${expected}.`);
  console.error('Bump them together:');
  console.error(`  1. set "version": "${expected}" in package.json and src-tauri/tauri.conf.json`);
  console.error(`  2. set version = "${expected}" in src-tauri/Cargo.toml`);
  console.error('  3. npm install --package-lock-only   (refreshes package-lock.json)');
  console.error('  4. cargo check in src-tauri          (refreshes Cargo.lock)');
  process.exit(1);
}
console.log(`\nAll versions are ${expected}.`);
