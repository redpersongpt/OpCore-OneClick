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

export const dellBiosBackend: BiosBackend = {
  id: 'dell',
  vendor: 'Dell',
  label: 'Dell vendor backend',
  vendorMatchers: [/dell/i, /alienware/i],
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
