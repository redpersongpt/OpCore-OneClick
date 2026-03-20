import type { BiosBackend, BiosOrchestratorContext, BiosSettingId, BiosSupportLevel } from '../types.js';

const MANAGED_CAPABLE_SETTINGS = new Set<BiosSettingId>([
  'secure-boot',
  'csm',
  'fast-boot',
  'above4g',
  'xhci-handoff',
  'sata-ahci',
  'vt-d',
  'svm',
]);

export const genericBiosBackend: BiosBackend = {
  id: 'generic',
  vendor: 'Generic',
  label: 'Generic firmware backend',
  vendorMatchers: [],
  getSupportLevel(settingId: BiosSettingId, ctx: BiosOrchestratorContext): BiosSupportLevel {
    if (settingId === 'cfg-lock') return 'manual';
    if (!MANAGED_CAPABLE_SETTINGS.has(settingId)) return 'manual';
    return ctx.platform === 'win32' ? 'assisted' : 'manual';
  },
  rebootSupported(platform: NodeJS.Platform): boolean {
    return platform === 'win32';
  },
};
