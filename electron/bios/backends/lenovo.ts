import type { BiosBackend, BiosOrchestratorContext, BiosSettingId, BiosSupportLevel } from '../types.js';

const AUTO_CAPABLE = new Set<BiosSettingId>([
  'secure-boot',
  'csm',
  'fast-boot',
  'above4g',
  'xhci-handoff',
  'sata-ahci',
  'vt-d',
  'svm',
]);

export const lenovoBiosBackend: BiosBackend = {
  id: 'lenovo',
  vendor: 'Lenovo',
  label: 'Lenovo vendor backend',
  vendorMatchers: [/lenovo/i, /thinkpad/i, /legion/i, /ideapad/i],
  getSupportLevel(settingId: BiosSettingId, ctx: BiosOrchestratorContext): BiosSupportLevel {
    if (settingId === 'cfg-lock') return 'manual';
    if (!AUTO_CAPABLE.has(settingId)) return 'manual';
    if (ctx.platform !== 'win32') return 'assisted';
    return ctx.safeMode ? 'assisted' : 'managed';
  },
  rebootSupported(platform: NodeJS.Platform): boolean {
    return platform === 'win32';
  },
};
