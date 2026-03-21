import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

export interface FolderIntegrityIssues {
  modified: string[];
  missing: string[];
  untracked: string[];
}

export interface FolderIntegrityResult {
  valid: boolean;
  issues: FolderIntegrityIssues;
}

export function sha256File(filePath: string): string | null {
  if (!fs.existsSync(filePath) || fs.statSync(filePath).isDirectory()) return null;
  const hash = crypto.createHash('sha256');
  const fd = fs.openSync(filePath, 'r');
  try {
    const buffer = Buffer.alloc(64 * 1024);
    let bytesRead = 0;
    do {
      bytesRead = fs.readSync(fd, buffer, 0, buffer.length, null);
      if (bytesRead > 0) hash.update(buffer.subarray(0, bytesRead));
    } while (bytesRead > 0);
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest('hex');
}

function collectRelativeFiles(folderPath: string, manifestPath: string): string[] {
  const files: string[] = [];
  const walk = (currentPath: string) => {
    for (const entry of fs.readdirSync(currentPath, { withFileTypes: true })) {
      const fullPath = path.join(currentPath, entry.name);
      if (entry.isDirectory()) {
        walk(fullPath);
        continue;
      }
      if (!entry.isFile()) continue;
      if (path.resolve(fullPath) === path.resolve(manifestPath)) continue;
      files.push(path.relative(folderPath, fullPath).replaceAll(path.sep, '/'));
    }
  };
  walk(folderPath);
  return files.sort();
}

export function generateFolderManifest(folderPath: string, manifestPath = path.join(folderPath, 'manifest.json')): Record<string, string> | null {
  if (!fs.existsSync(folderPath) || !fs.statSync(folderPath).isDirectory()) return null;
  const manifest: Record<string, string> = {};
  for (const relativePath of collectRelativeFiles(folderPath, manifestPath)) {
    const digest = sha256File(path.join(folderPath, relativePath));
    if (!digest) continue;
    manifest[relativePath] = digest;
  }
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
  return manifest;
}

export function verifyFolderIntegrity(folderPath: string, manifestPath = path.join(folderPath, 'manifest.json')): FolderIntegrityResult {
  const emptyIssues: FolderIntegrityIssues = { modified: [], missing: [], untracked: [] };
  if (!fs.existsSync(folderPath) || !fs.statSync(folderPath).isDirectory()) {
    return { valid: false, issues: { ...emptyIssues, missing: ['<folder>'] } };
  }
  if (!fs.existsSync(manifestPath)) {
    return { valid: false, issues: { ...emptyIssues, missing: ['manifest.json'] } };
  }

  let manifest: Record<string, string>;
  try {
    manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf-8')) as Record<string, string>;
  } catch {
    return { valid: false, issues: { ...emptyIssues, modified: ['manifest.json'] } };
  }

  const actualFiles = new Set(collectRelativeFiles(folderPath, manifestPath));
  const manifestFiles = new Set(Object.keys(manifest));

  for (const relativePath of manifestFiles) {
    const fullPath = path.join(folderPath, relativePath);
    if (!actualFiles.has(relativePath) || !fs.existsSync(fullPath)) {
      emptyIssues.missing.push(relativePath);
      continue;
    }
    if (sha256File(fullPath) !== manifest[relativePath]) {
      emptyIssues.modified.push(relativePath);
    }
  }

  for (const relativePath of actualFiles) {
    if (!manifestFiles.has(relativePath)) {
      emptyIssues.untracked.push(relativePath);
    }
  }

  return {
    valid: emptyIssues.modified.length === 0 && emptyIssues.missing.length === 0 && emptyIssues.untracked.length === 0,
    issues: emptyIssues,
  };
}
