#!/usr/bin/env node
/**
 * OpCore-OneClick CLI — headless hardware scan, compatibility check, and report.
 *
 * Usage:
 *   node dist-electron/electron/cli.js scan [--json]
 *   node dist-electron/electron/cli.js compatible [--json] [--target "macOS Sequoia 15.x"]
 *   node dist-electron/electron/cli.js report [--json]
 *   node dist-electron/electron/cli.js matrix [--json]
 *   node dist-electron/electron/cli.js version
 *
 * Exit codes:
 *   0 — success (or compatible)
 *   1 — error / incompatible / blocked
 *   2 — usage error
 */

import { detectHardware } from './hardwareDetect.js';
import { interpretHardware } from './hardwareInterpret.js';
import { detectCpuGeneration, detectArchitecture, mapDetectedToProfile } from './hardwareMapper.js';
import { checkCompatibility } from './compatibility.js';
import { buildCompatibilityMatrix } from './compatibilityMatrix.js';
import { getSMBIOSForProfile, getRequiredResources } from './configGenerator.js';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);

// ── Argument parsing ─────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const command = args[0] ?? '';
const flags = new Set(args.filter(a => a.startsWith('--')));
const jsonMode = flags.has('--json');
const targetFlag = args.find((_, i) => args[i - 1] === '--target') ?? null;

// ── Helpers ──────────────────────────────────────────────────────────────────

function printJson(data: unknown): void {
  process.stdout.write(JSON.stringify(data, null, 2) + '\n');
}

function printHuman(lines: string[]): void {
  for (const line of lines) process.stdout.write(line + '\n');
}

function getVersion(): string {
  try {
    const pkgPath = path.resolve(path.dirname(__filename), '..', '..', 'package.json');
    const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf-8'));
    return pkg.version ?? 'unknown';
  } catch {
    return 'unknown';
  }
}

function usage(): void {
  const v = getVersion();
  printHuman([
    `OpCore-OneClick CLI v${v}`,
    '',
    'Commands:',
    '  scan         Detect hardware and print the machine profile',
    '  compatible   Check if this hardware can run macOS (use --target to specify version)',
    '  report       Full compatibility report with warnings, errors, and guidance',
    '  matrix       Show all macOS versions and their compatibility status',
    '  version      Print version and exit',
    '',
    'Flags:',
    '  --json       Output as JSON instead of human-readable text',
    '  --target     Target macOS version (e.g. "macOS Sequoia 15.x", "macOS Tahoe 26")',
    '',
    'Examples:',
    '  opcore-oneclick scan --json',
    '  opcore-oneclick compatible --target "macOS Tahoe 26"',
    '  opcore-oneclick report',
    '  opcore-oneclick matrix --json',
    '',
    'Legacy alias:',
    '  macos-oneclick … (still supported for continuity)',
    '',
    'Note: This CLI performs read-only operations only.',
    '      Destructive operations (flash, build) require the GUI app.',
  ]);
}

// ── Commands ─────────────────────────────────────────────────────────────────

async function cmdScan(): Promise<void> {
  const hw = await detectHardware();
  const interpretation = interpretHardware(hw);
  const profile = mapDetectedToProfile(hw);

  if (jsonMode) {
    printJson({
      profile,
      interpretation: {
        overallConfidence: interpretation.overallConfidence,
        summary: interpretation.summary,
        manualVerificationNeeded: interpretation.manualVerificationNeeded,
      },
      detected: {
        cpu: hw.cpu.name,
        gpus: hw.gpus.map(g => g.name),
        motherboard: hw.motherboardModel || hw.motherboardVendor,
        ram: hw.ramBytes,
        coreCount: hw.coreCount,
        isLaptop: hw.isLaptop,
        isVM: hw.isVM,
      },
    });
  } else {
    printHuman([
      `CPU:          ${profile.cpu}`,
      `Architecture: ${profile.architecture}`,
      `Generation:   ${profile.generation}`,
      `GPU:          ${profile.gpu}`,
      `Motherboard:  ${profile.motherboard}`,
      `RAM:          ${profile.ram}`,
      `Cores:        ${profile.coreCount ?? 'unknown'}`,
      `Form factor:  ${profile.isLaptop ? 'Laptop' : 'Desktop'}`,
      `VM:           ${profile.isVM ? 'Yes' : 'No'}`,
      `SMBIOS:       ${profile.smbios}`,
      `Confidence:   ${interpretation.overallConfidence}`,
      '',
      `Summary: ${interpretation.summary}`,
      ...(interpretation.manualVerificationNeeded.length > 0
        ? ['', 'Manual verification needed:', ...interpretation.manualVerificationNeeded.map(v => `  - ${v}`)]
        : []),
    ]);
  }
}

async function cmdCompatible(): Promise<void> {
  const hw = await detectHardware();
  const profile = mapDetectedToProfile(hw);
  if (targetFlag) {
    profile.targetOS = targetFlag;
    profile.smbios = getSMBIOSForProfile(profile);
  }

  const report = checkCompatibility(profile);

  if (jsonMode) {
    printJson({
      compatible: report.isCompatible,
      level: report.level,
      strategy: report.strategy,
      confidence: report.confidence,
      target: profile.targetOS,
      smbios: profile.smbios,
    });
  } else {
    const symbol = report.isCompatible ? 'YES' : 'NO';
    printHuman([
      `Compatible:   ${symbol}`,
      `Level:        ${report.level}`,
      `Strategy:     ${report.strategy}`,
      `Confidence:   ${report.confidence}`,
      `Target:       ${profile.targetOS}`,
      `SMBIOS:       ${profile.smbios}`,
    ]);
  }

  process.exitCode = report.isCompatible ? 0 : 1;
}

async function cmdReport(): Promise<void> {
  const hw = await detectHardware();
  const interpretation = interpretHardware(hw);
  const profile = mapDetectedToProfile(hw);
  if (targetFlag) {
    profile.targetOS = targetFlag;
    profile.smbios = getSMBIOSForProfile(profile);
  }

  const report = checkCompatibility(profile);
  const resources = getRequiredResources(profile);

  if (jsonMode) {
    printJson({
      profile,
      compatibility: {
        level: report.level,
        strategy: report.strategy,
        isCompatible: report.isCompatible,
        confidence: report.confidence,
        explanation: report.explanation,
        warnings: report.warnings,
        errors: report.errors,
        recommendedVersion: report.recommendedVersion,
        eligibleVersions: report.eligibleVersions,
        nextActions: report.nextActions,
        failurePoints: report.mostLikelyFailurePoints,
      },
      resources: {
        kexts: resources.kexts,
        ssdts: resources.ssdts,
      },
      interpretation: {
        overallConfidence: interpretation.overallConfidence,
        summary: interpretation.summary,
        manualVerificationNeeded: interpretation.manualVerificationNeeded,
      },
    });
  } else {
    printHuman([
      '── Hardware ──',
      `CPU:          ${profile.cpu} (${profile.architecture} ${profile.generation})`,
      `GPU:          ${profile.gpu}`,
      `Motherboard:  ${profile.motherboard}`,
      `RAM:          ${profile.ram}`,
      `SMBIOS:       ${profile.smbios}`,
      '',
      '── Compatibility ──',
      `Level:        ${report.level}`,
      `Compatible:   ${report.isCompatible ? 'Yes' : 'No'}`,
      `Strategy:     ${report.strategy}`,
      `Target:       ${profile.targetOS}`,
      `Recommended:  ${report.recommendedVersion}`,
      '',
      report.explanation,
      ...(report.warnings.length > 0 ? ['', 'Warnings:', ...report.warnings.map(w => `  ⚠ ${w}`)] : []),
      ...(report.errors.length > 0 ? ['', 'Errors:', ...report.errors.map(e => `  ✗ ${e}`)] : []),
      ...(report.nextActions.length > 0 ? ['', 'Next steps:', ...report.nextActions.map(a => `  → ${a.title}: ${a.detail}`)] : []),
      '',
      '── Required Resources ──',
      `Kexts:   ${resources.kexts.join(', ')}`,
      `SSDTs:   ${resources.ssdts.join(', ')}`,
    ]);
  }

  process.exitCode = report.isCompatible ? 0 : 1;
}

async function cmdMatrix(): Promise<void> {
  const hw = await detectHardware();
  const profile = mapDetectedToProfile(hw);
  const matrix = buildCompatibilityMatrix(profile);

  if (jsonMode) {
    printJson({
      recommendedVersion: matrix.recommendedVersion,
      rows: matrix.rows.map(r => ({
        version: r.versionName,
        numeric: r.numeric,
        status: r.status,
        recommended: r.recommended,
        reason: r.reason,
      })),
    });
  } else {
    printHuman([
      `Recommended: ${matrix.recommendedVersion}`,
      '',
      ...matrix.rows.map(r => {
        const tag = r.recommended ? ' ★' : '';
        const pad = r.versionName.padEnd(22);
        return `  ${pad} ${r.status.padEnd(14)} ${r.reason}${tag}`;
      }),
    ]);
  }
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  switch (command) {
    case 'scan':
      await cmdScan();
      break;
    case 'compatible':
      await cmdCompatible();
      break;
    case 'report':
      await cmdReport();
      break;
    case 'matrix':
      await cmdMatrix();
      break;
    case 'version':
      printHuman([getVersion()]);
      break;
    case 'help':
    case '--help':
    case '-h':
      usage();
      break;
    default:
      if (command) {
        process.stderr.write(`Unknown command: ${command}\n\n`);
      }
      usage();
      process.exitCode = 2;
  }
}

main().catch(err => {
  if (jsonMode) {
    printJson({ error: err.message ?? String(err) });
  } else {
    process.stderr.write(`Error: ${err.message ?? err}\n`);
  }
  process.exitCode = 1;
});
