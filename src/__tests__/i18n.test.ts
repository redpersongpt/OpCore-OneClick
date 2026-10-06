import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CPU_VENDORS,
  DEVICE_BUSES,
  GPU_VENDORS,
  INPUT_BUSES,
  TOUCHPAD_VENDORS,
  VM_KINDS,
} from '../components/hardware/options';
import { guideFor, POST_INSTALL } from '../content/postInstall';
import { TROUBLE_CATEGORIES, TROUBLESHOOTING } from '../content/troubleshoot';
import { DICTIONARIES, interpolate, translate } from '../i18n';
import { en } from '../i18n/en';
import { detectLanguage, LANGUAGES } from '../i18n/lang';
import { tr } from '../i18n/tr';
import { COMPONENT_IDS } from '../lib/labels';
import { STEPS } from '../stores/wizard';

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'generated' || name === '__tests__' ? [] : sourceFiles(path);
    return /\.tsx?$/.test(name) ? [path] : [];
  });
}

describe('i18n dictionaries', () => {
  it('every key exists in both languages', () => {
    expect(Object.keys(tr).sort()).toEqual(Object.keys(en).sort());
    for (const lang of LANGUAGES) expect(DICTIONARIES[lang.id]).toBeDefined();
  });

  it('no translation is empty and placeholders match', () => {
    for (const key of Object.keys(en) as (keyof typeof en)[]) {
      expect(en[key].trim(), key).not.toBe('');
      expect(tr[key].trim(), key).not.toBe('');
      expect(placeholders(tr[key]), key).toEqual(placeholders(en[key]));
    }
  });

  it('every statically referenced key exists', () => {
    const missing: string[] = [];
    for (const file of sourceFiles(join(__dirname, '..'))) {
      const text = readFileSync(file, 'utf8');
      for (const m of text.matchAll(/\bt\('([\w.-]+)'/g)) {
        if (!(m[1] in en)) missing.push(`${file}: ${m[1]}`);
      }
    }
    expect(missing).toEqual([]);
  });

  it('dynamic key families are complete', () => {
    const families: [string, readonly string[]][] = [
      ['step', STEPS],
      ['enum.cpuVendor', CPU_VENDORS],
      ['enum.gpuVendor', GPU_VENDORS],
      ['enum.deviceBus', DEVICE_BUSES],
      ['enum.inputBus', INPUT_BUSES],
      ['enum.touchpadVendor', TOUCHPAD_VENDORS],
      ['enum.vm', [...VM_KINDS, 'none']],
      ['enum.storageKind', ['nvme', 'sata', 'raid', 'emmc', 'usb', 'other']],
      ['enum.picker', ['graphical', 'text']],
      ['enum.intelWifi', ['auto', 'itlwm', 'airport_itlwm', 'none']],
      ['build.intelWifiHint', ['auto', 'itlwm', 'airport_itlwm', 'none']],
      ['support', ['supported', 'partial', 'unsupported', 'unknown']],
      ['note', ['info', 'warning', 'blocking']],
      ['check', ['ok', 'action', 'inferred', 'unknown', 'na']],
      ['artifact', ['downloaded', 'cached', 'bundled', 'generated', 'skipped', 'failed']],
      ['ssdtSource', ['generated', 'oc_sample', 'dortania']],
      ['source', ['scan', 'manual', 'imported', 'demo']],
      ['task.status', ['running', 'completed', 'failed', 'cancelled']],
      ['task.kind', ['scan', 'build', 'recovery', 'flash']],
      ['recovery.phase', ['resolving', 'downloading', 'verifying', 'complete', 'failed']],
      ['review.verdict', ['passed', 'warnings', 'failed']],
      ['flash.phase', ['prepare', 'partition', 'format', 'copyEfi', 'copyRecovery', 'verify']],
      ['trouble.category', TROUBLE_CATEGORIES],
      ['component', COMPONENT_IDS],
    ];
    const missing = families.flatMap(([prefix, values]) =>
      values.map((v) => `${prefix}.${v}`).filter((key) => !(key in en)),
    );
    expect(missing).toEqual([]);
  });

  it('troubleshooting entries are written in both languages', () => {
    const ids = new Set<string>();
    for (const entry of TROUBLESHOOTING) {
      expect(ids.has(entry.id), entry.id).toBe(false);
      ids.add(entry.id);
      const { en: e, tr: t } = entry.text;
      for (const field of ['symptoms', 'causes', 'fixes'] as const) {
        expect(e[field].length, `${entry.id}.${field}`).toBeGreaterThan(0);
        expect(t[field].length, `${entry.id}.${field}`).toBe(e[field].length);
      }
      expect(t.advanced?.length ?? 0, `${entry.id}.advanced`).toBe(e.advanced?.length ?? 0);
      expect(t.title.trim()).not.toBe('');
    }
  });

  it('covers the install-time failures that used to be missing', () => {
    const ids = TROUBLESHOOTING.map((e) => e.id);
    for (const id of ['security-violation', 'memory-allocation', 'recovery-server', 'amd-cpur', 'alder-lake', 'tahoe', 'exitbs', 'root-device', 'cfg-lock']) {
      expect(ids).toContain(id);
    }
    const all = JSON.stringify(TROUBLESHOOTING.map((e) => e.text.en));
    expect(all).not.toContain('Intel Power Gadget or');
    expect(all).toContain('AppleHDA');
    expect(all).toContain('IntelBTPatcher');
  });

  it('post-install guide is written in both languages', () => {
    expect(POST_INSTALL.tr.map((s) => s.id)).toEqual(POST_INSTALL.en.map((s) => s.id));
    POST_INSTALL.en.forEach((step, i) => {
      expect(POST_INSTALL.tr[i].steps.length, step.id).toBe(step.steps.length);
    });
    expect(POST_INSTALL.en.map((s) => s.id)).toEqual(expect.arrayContaining(['copy-efi', 'usb-map', 'root-patch']));
  });

  it('post-install guide shows only the steps that apply', () => {
    const base = { needsRootPatch: false, tahoe: false, intelWifi: false, analogAudio: true };
    const ids = (ctx: typeof base) => guideFor(POST_INSTALL.en, ctx).map((s) => s.id);
    expect(ids(base)).toEqual(['boot', 'copy-efi', 'usb-map', 'iservices', 'backup']);
    expect(ids({ ...base, tahoe: true })).toContain('tahoe-audio');
    expect(ids({ ...base, tahoe: true, analogAudio: false })).not.toContain('tahoe-audio');
    expect(ids({ ...base, needsRootPatch: true, intelWifi: true })).toEqual(expect.arrayContaining(['root-patch', 'intel-wifi']));
  });
});

describe('translate', () => {
  it('interpolates parameters and leaves unknown ones visible', () => {
    expect(interpolate('Saved {time} by {who}', { time: 'now' })).toBe('Saved now by {who}');
    expect(translate('en', 'flash.expiresIn', { seconds: 42 })).toBe('This confirmation expires in 42 s.');
    expect(translate('tr', 'common.retry')).toBe('Tekrar dene');
  });

  it('detects the language from the browser preferences', () => {
    expect(detectLanguage('tr-TR')).toBe('tr');
    expect(detectLanguage(['de-DE', 'tr'])).toBe('tr');
    expect(detectLanguage(['en-GB', 'tr-TR'])).toBe('en');
    expect(detectLanguage('fr-FR')).toBe('en');
    expect(detectLanguage(undefined)).toBe('en');
  });
});
