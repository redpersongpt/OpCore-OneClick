import { describe, it, expect, beforeAll } from 'vitest';
import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';

const CLI_PATH = path.resolve('dist-electron/electron/cli.js');

// Compile the CLI if not already built
beforeAll(() => {
  if (!fs.existsSync(CLI_PATH)) {
    execSync('npx tsc -p electron/tsconfig.json', { timeout: 30_000 });
  }
}, 60_000);

function runCLI(args: string): { stdout: string; stderr: string; exitCode: number } {
  try {
    const stdout = execSync(`node ${CLI_PATH} ${args}`, {
      encoding: 'utf-8',
      timeout: 30_000,
      env: { ...process.env, NODE_OPTIONS: '' },
    });
    return { stdout, stderr: '', exitCode: 0 };
  } catch (err: any) {
    return {
      stdout: err.stdout ?? '',
      stderr: err.stderr ?? '',
      exitCode: err.status ?? 1,
    };
  }
}

describe('CLI — version command', () => {
  it('prints version and exits 0', () => {
    const { stdout, exitCode } = runCLI('version');
    expect(exitCode).toBe(0);
    expect(stdout.trim()).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe('CLI — help', () => {
  it('prints usage on --help', () => {
    const { stdout, exitCode } = runCLI('--help');
    expect(exitCode).toBe(0);
    expect(stdout).toContain('Commands:');
    expect(stdout).toContain('scan');
    expect(stdout).toContain('compatible');
    expect(stdout).toContain('report');
    expect(stdout).toContain('matrix');
  });

  it('prints usage on unknown command with exit code 2', () => {
    const { exitCode } = runCLI('nonexistent');
    expect(exitCode).toBe(2);
  });
});

describe('CLI — scan', () => {
  it('scan --json produces valid JSON with profile', () => {
    const { stdout, exitCode } = runCLI('scan --json');
    expect(exitCode).toBe(0);
    const data = JSON.parse(stdout);
    expect(data).toHaveProperty('profile');
    expect(data.profile).toHaveProperty('cpu');
    expect(data.profile).toHaveProperty('architecture');
    expect(data.profile).toHaveProperty('generation');
    expect(data.profile).toHaveProperty('gpu');
    expect(data.profile).toHaveProperty('smbios');
  });

  it('scan human-readable shows CPU line', () => {
    const { stdout, exitCode } = runCLI('scan');
    expect(exitCode).toBe(0);
    expect(stdout).toContain('CPU:');
    expect(stdout).toContain('Architecture:');
    expect(stdout).toContain('SMBIOS:');
  });
});

describe('CLI — compatible', () => {
  it('compatible --json produces valid JSON', () => {
    const { stdout } = runCLI('compatible --json');
    const data = JSON.parse(stdout);
    expect(data).toHaveProperty('compatible');
    expect(data).toHaveProperty('level');
    expect(data).toHaveProperty('smbios');
    expect(typeof data.compatible).toBe('boolean');
  });
});

describe('CLI — report', () => {
  it('report --json includes compatibility and resources', () => {
    const { stdout } = runCLI('report --json');
    const data = JSON.parse(stdout);
    expect(data).toHaveProperty('profile');
    expect(data).toHaveProperty('compatibility');
    expect(data).toHaveProperty('resources');
    expect(data.resources).toHaveProperty('kexts');
    expect(data.resources).toHaveProperty('ssdts');
  });
});

describe('CLI — matrix', () => {
  it('matrix --json lists macOS versions', () => {
    const { stdout } = runCLI('matrix --json');
    const data = JSON.parse(stdout);
    expect(data).toHaveProperty('recommendedVersion');
    expect(data).toHaveProperty('rows');
    expect(Array.isArray(data.rows)).toBe(true);
    expect(data.rows.length).toBeGreaterThan(0);
    expect(data.rows[0]).toHaveProperty('version');
    expect(data.rows[0]).toHaveProperty('status');
  });
});

describe('CLI — exit codes', () => {
  it('unsupported destructive commands are not available', () => {
    const { exitCode, stdout } = runCLI('flash');
    expect(exitCode).toBe(2);
    expect(stdout).toContain('Commands:');
  });

  it('build command is not available', () => {
    const { exitCode } = runCLI('build');
    expect(exitCode).toBe(2);
  });
});
