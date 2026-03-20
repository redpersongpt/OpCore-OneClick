/**
 * failureReplay.ts — Simulate failure scenarios and capture structured log output.
 * Run with: npx tsx test/failureReplay.ts
 */
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createLogger } from '../electron/logger.js';
import { createTaskRegistry } from '../electron/taskManager.js';
import type { TaskUpdatePayload } from '../electron/taskManager.js';

function makeTempDir(label: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `failure-replay-${label}-`));
}

function makeLogger(dir: string) {
  return createLogger({
    logFile:          path.join(dir, 'app.log'),
    opsFile:          path.join(dir, 'operations.log'),
    minLevel:         'DEBUG',
    maxFileSizeBytes: 10 * 1024 * 1024,
    rotationCount:    3,
    flushIntervalMs:  100,
    crashSafeSync:    true,
    isPackaged:       true,
  });
}

function makeRegistry(dir: string) {
  const logger = makeLogger(dir);
  const emitted: TaskUpdatePayload[] = [];
  const registry = createTaskRegistry((p) => emitted.push(p), logger);
  return { registry, logger, emitted };
}

function readOpsLog(dir: string): any[] {
  try {
    const content = fs.readFileSync(path.join(dir, 'operations.log'), 'utf-8');
    return content.trim().split('\n').filter(Boolean).map(l => JSON.parse(l));
  } catch { return []; }
}

function readAppLog(dir: string): any[] {
  try {
    const content = fs.readFileSync(path.join(dir, 'app.log'), 'utf-8');
    return content.trim().split('\n').filter(Boolean).map(l => JSON.parse(l));
  } catch { return []; }
}

function printHeader(title: string) {
  console.log('\n' + '═'.repeat(70));
  console.log(`  SCENARIO: ${title}`);
  console.log('═'.repeat(70));
}

function printSection(label: string) {
  console.log(`\n  ── ${label} ──`);
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1: Cancelled recovery download
// ─────────────────────────────────────────────────────────────────────────────
async function scenario1() {
  printHeader('Cancelled Recovery Download');
  const dir = makeTempDir('s1');
  const { registry, logger, emitted } = makeRegistry(dir);

  const token = registry.create('recovery-download');
  logger.withTask(token.taskId).info('recovery', 'Starting macOS Sequoia 15 download', { targetOS: 'macOS Sequoia 15' });

  // Simulate progress at 10%, 40%, 60%
  for (const pct of [10, 40, 60]) {
    await new Promise(r => setTimeout(r, 30)); // simulate time passing between chunks
    registry.updateProgress(token.taskId, {
      kind: 'recovery-download',
      percent: pct,
      status: `Downloading… ${pct}%`,
      bytesDownloaded: Math.round(pct * 1024 * 1024 * 8),
      dmgDest: '/tmp/recovery.dmg',
      clDest: '/tmp/com.apple.recovery.boot',
    });
  }

  // User cancels at 60%
  registry.cancel(token.taskId);
  logger.info('recovery', 'User cancelled download', { taskId: token.taskId });

  // Flush
  logger.flush();

  const finalState = registry.get(token.taskId);
  printSection('Final task state');
  console.log(JSON.stringify(finalState, null, 2));

  printSection('Timeline entries (operations.log)');
  for (const entry of readOpsLog(dir)) {
    console.log(JSON.stringify(entry));
  }

  printSection('pushFn call count');
  console.log(`  Total pushFn calls: ${emitted.length} (create=1, up-to-2 throttled progress updates, cancel=1)`);
  console.log(`  Status sequence: ${emitted.map(e => e.task.status).join(' → ')}`);

  fs.rmSync(dir, { recursive: true, force: true });
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2: Blocked system disk flash
// ─────────────────────────────────────────────────────────────────────────────
async function scenario2() {
  printHeader('Blocked System Disk Flash');
  const dir = makeTempDir('s2');
  const { registry, logger, emitted } = makeRegistry(dir);

  const token = registry.create('usb-flash');
  logger.withTask(token.taskId).info('usb-flash', 'Safety checks initiated', { device: '/dev/disk0' });

  // Safety check returns SYSTEM_DISK fatal violation
  const safetyError = 'SAFETY BLOCK: /dev/disk0 is your system boot disk — cannot flash the OS drive';
  logger.withTask(token.taskId).warn('usb-flash', 'Safety check blocked flash', {
    code: 'SYSTEM_DISK',
    device: '/dev/disk0',
    error: safetyError,
  });

  registry.fail(token.taskId, safetyError);
  logger.flush();

  const finalState = registry.get(token.taskId);
  printSection('Final task state');
  console.log(JSON.stringify(finalState, null, 2));

  printSection('App log entries');
  for (const entry of readAppLog(dir)) {
    if (entry.level !== 'DEBUG') {
      console.log(JSON.stringify(entry));
    }
  }

  printSection('Timeline entries (operations.log)');
  for (const entry of readOpsLog(dir)) {
    if (entry.kind !== 'app_start') {
      console.log(JSON.stringify(entry));
    }
  }

  printSection('pushFn call count');
  console.log(`  Total pushFn calls: ${emitted.length} (create=1, fail=1)`);
  console.log(`  Status sequence: ${emitted.map(e => e.task.status).join(' → ')}`);

  fs.rmSync(dir, { recursive: true, force: true });
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3: Failed kext download (partial success)
// ─────────────────────────────────────────────────────────────────────────────
async function scenario3() {
  printHeader('Failed Kext Download (Rate Limit)');
  const dir = makeTempDir('s3');
  const { registry, logger, emitted } = makeRegistry(dir);

  const kexts = ['Lilu', 'WhateverGreen', 'AppleALC'];
  const token = registry.create('kext-fetch');
  logger.withTask(token.taskId).info('kext-fetch', 'Starting kext downloads', { count: kexts.length });

  // Kext 0: Lilu succeeds
  registry.updateProgress(token.taskId, {
    kind: 'kext-fetch', kextName: 'Lilu', version: '1.6.8', index: 0, total: 3
  });
  logger.withTask(token.taskId).info('kext-fetch', 'Downloaded kext', { kextName: 'Lilu', version: '1.6.8' });

  // Kext 1: WhateverGreen hits rate limit
  registry.updateProgress(token.taskId, {
    kind: 'kext-fetch', kextName: 'WhateverGreen', version: 'pending', index: 1, total: 3
  });
  const rateError = 'HTTP 429: GitHub API rate limit exceeded — retry after 60s';
  logger.withTask(token.taskId).error('kext-fetch', 'Kext download failed', {
    kextName: 'WhateverGreen',
    error: rateError,
  });

  registry.fail(token.taskId, rateError);

  // Kext 2: AppleALC would have been cancelled — but task is already terminal
  // (demonstrates stale event scenario)
  logger.withTask(token.taskId).warn('kext-fetch', 'AppleALC download skipped — task already terminal', {
    kextName: 'AppleALC'
  });

  logger.flush();

  const finalState = registry.get(token.taskId);
  printSection('Final task state');
  console.log(JSON.stringify(finalState, null, 2));

  printSection('App log entries');
  for (const entry of readAppLog(dir)) {
    if (entry.level !== 'DEBUG') {
      console.log(JSON.stringify(entry));
    }
  }

  printSection('Timeline entries (operations.log)');
  for (const entry of readOpsLog(dir)) {
    if (entry.kind !== 'app_start') {
      console.log(JSON.stringify(entry));
    }
  }

  printSection('pushFn call count');
  console.log(`  Total pushFn calls: ${emitted.length} (create=1, 2x progress, fail=1)`);
  console.log(`  Status sequence: ${emitted.map(e => e.task.status).join(' → ')}`);

  fs.rmSync(dir, { recursive: true, force: true });
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────
(async () => {
  console.log('\n  macOS One-Click — Failure Replay\n');
  console.log(`  Platform: ${process.platform} | Node: ${process.version} | Date: ${new Date().toISOString()}`);

  await scenario1();
  await scenario2();
  await scenario3();

  console.log('\n' + '═'.repeat(70));
  console.log('  All scenarios complete.');
  console.log('═'.repeat(70) + '\n');
})();
