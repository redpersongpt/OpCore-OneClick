import type { BiosBackend, BiosOrchestratorContext, BiosSettingId, BiosSupportLevel } from '../types.js';

const AUTO_CAPABLE = new Set<BiosSettingId>([
  'secure-boot',
  'csm',
  'fast-boot',
  'above4g',
  'xhci-handoff',
  'sata-ahci',
  'vt-d',
]);

export const hpBiosBackend: BiosBackend = {
  id: 'hp',
  vendor: 'HP',
  label: 'HP vendor backend',
  vendorMatchers: [/hewlett-packard/i, /\bhp\b/i, /omen/i],
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
