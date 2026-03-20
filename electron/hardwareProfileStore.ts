import fs from 'node:fs';
import path from 'node:path';
import {
  parseHardwareProfileArtifact,
  type HardwareProfileArtifact,
} from './hardwareProfileArtifact.js';

function writeJsonAtomic(targetPath: string, value: unknown): void {
  const tempPath = `${targetPath}.tmp`;
  fs.writeFileSync(tempPath, JSON.stringify(value, null, 2));
  fs.renameSync(tempPath, targetPath);
}

export function createHardwareProfileStore(basePath: string) {
  const rootPath = path.resolve(basePath, 'hardware-profiles');
  const archivePath = path.resolve(rootPath, 'archive');
  const invalidPath = path.resolve(rootPath, 'invalid');
  const latestPath = path.resolve(rootPath, 'latest.json');

  fs.mkdirSync(archivePath, { recursive: true });
  fs.mkdirSync(invalidPath, { recursive: true });

  const quarantineLatest = (reason: string): void => {
    if (!fs.existsSync(latestPath)) return;
    const quarantinePath = path.resolve(
      invalidPath,
      `latest-${Date.now()}-${reason.replace(/[^a-z0-9_-]+/gi, '-')}.json`,
    );
    try {
      fs.renameSync(latestPath, quarantinePath);
    } catch {
      try { fs.unlinkSync(latestPath); } catch {}
    }
  };

  const readArtifactFile = (filePath: string): HardwareProfileArtifact => {
    const raw = JSON.parse(fs.readFileSync(filePath, 'utf8'));
    return parseHardwareProfileArtifact(raw);
  };

  return {
    rootPath,
    latestPath,
    saveLatest(artifact: HardwareProfileArtifact): HardwareProfileArtifact {
      const validated = parseHardwareProfileArtifact(artifact);
      writeJsonAtomic(latestPath, validated);
      writeJsonAtomic(
        path.resolve(archivePath, `${validated.capturedAt}-${validated.digest.slice(0, 16)}.json`),
        validated,
      );
      return validated;
    },
    loadLatest(): HardwareProfileArtifact | null {
      if (!fs.existsSync(latestPath)) return null;
      try {
        return readArtifactFile(latestPath);
      } catch (error) {
        quarantineLatest(error instanceof Error ? error.message : 'invalid');
        return null;
      }
    },
    importFromFile(filePath: string): HardwareProfileArtifact {
      return readArtifactFile(path.resolve(filePath));
    },
    exportArtifact(artifact: HardwareProfileArtifact, filePath: string): HardwareProfileArtifact {
      const validated = parseHardwareProfileArtifact(artifact);
      const resolvedPath = path.resolve(filePath);
      fs.mkdirSync(path.dirname(resolvedPath), { recursive: true });
      writeJsonAtomic(resolvedPath, validated);
      return validated;
    },
  };
}
