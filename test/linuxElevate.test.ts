import { describe, it, expect, vi, beforeEach } from 'vitest';

// We test the exported logic by importing the module.
// Since we're on macOS in CI, elevateCommand returns the command unchanged (non-Linux).
// We test the contract: on non-Linux, commands pass through unchanged.
// Linux-specific behavior (pkexec wrapping) is validated structurally.

describe('linuxElevate — non-Linux passthrough', () => {
  it('elevateCommand returns command unchanged on non-Linux', async () => {
    const { elevateCommand } = await import('../electron/linuxElevate.js');
    // On macOS/Windows (CI runs on macOS), commands pass through
    expect(elevateCommand('parted /dev/sda --script mklabel gpt')).toBe('parted /dev/sda --script mklabel gpt');
  });

  it('elevateCommand preserves complex commands', async () => {
    const { elevateCommand } = await import('../electron/linuxElevate.js');
    const cmd = 'umount /dev/sda* 2>/dev/null || true';
    expect(elevateCommand(cmd)).toBe(cmd);
  });
});

describe('linuxElevate — elevation method detection', () => {
  it('detectElevationMethod returns a valid method', async () => {
    const { detectElevationMethod } = await import('../electron/linuxElevate.js');
    const method = await detectElevationMethod();
    // On macOS CI: will be 'root' if running as root, or 'pkexec'/'sudo'/'none'
    expect(['root', 'pkexec', 'sudo', 'none']).toContain(method);
  });

  it('elevationDescription returns a non-empty string', async () => {
    const { detectElevationMethod, elevationDescription } = await import('../electron/linuxElevate.js');
    await detectElevationMethod();
    const desc = elevationDescription();
    expect(desc.length).toBeGreaterThan(0);
  });
});

describe('linuxElevate — privilege guidance', () => {
  it('never tells users to run the full Electron GUI as root', async () => {
    const { elevationDescription } = await import('../electron/linuxElevate.js');
    // After detection, the description should not contain "run as root" or a suggestion to sudo-launch the GUI.
    const desc = elevationDescription().toLowerCase();
    expect(desc).not.toContain('run as root');
    expect(desc).not.toContain('sudo ./macos-one-click');
    expect(desc).not.toContain('sudo ./opcore-oneclick');
    expect(desc).not.toContain('run the entire app as root');
  });
});
