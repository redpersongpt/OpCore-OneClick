import type { BIOSConfig, HardwareProfile } from '../configGenerator.js';
import type { FirmwareInfo } from '../firmwarePreflight.js';
import { genericBiosBackend } from './backends/generic.js';
import { hpBiosBackend } from './backends/hp.js';
import { dellBiosBackend } from './backends/dell.js';
import { lenovoBiosBackend } from './backends/lenovo.js';
import { buildHardwareFingerprint } from './sessionState.js';
import { verifyBiosSelections } from './verification.js';
import type {
  BiosApplyMode,
  BiosBackend,
  BiosDetectionConfidence,
  BiosOrchestratorContext,
  BiosOrchestratorState,
  BiosRiskLevel,
  BiosSessionState,
  BiosSettingId,
  BiosSettingPlan,
  BiosSettingSelection,
  BiosSupportLevel,
  BiosVendor,
} from './types.js';

const BIOS_BACKENDS: BiosBackend[] = [
  hpBiosBackend,
  dellBiosBackend,
  lenovoBiosBackend,
  genericBiosBackend,
];

interface SettingMeta {
  id: BiosSettingId;
  name: string;
  required: (profile: HardwareProfile) => boolean;
  riskLevel: BiosRiskLevel;
  matchTokens: string[];
  fallbackDescription: string;
}

const SETTING_META: SettingMeta[] = [
  {
    id: 'uefi-mode',
    name: 'UEFI Boot Mode',
    required: () => true,
    riskLevel: 'low',
    matchTokens: ['uefi', 'os type'],
    fallbackDescription: 'OpenCore requires UEFI boot mode.',
  },
  {
    id: 'secure-boot',
    name: 'Secure Boot',
    required: () => true,
    riskLevel: 'low',
    matchTokens: ['secure boot'],
    fallbackDescription: 'OpenCore boot media must not be blocked by firmware Secure Boot.',
  },
  {
    id: 'csm',
    name: 'CSM / Legacy Boot',
    required: () => true,
    riskLevel: 'low',
    matchTokens: ['csm', 'legacy / csm', 'legacy'],
    fallbackDescription: 'CSM must be disabled for a canonical OpenCore UEFI path.',
  },
  {
    id: 'sata-ahci',
    name: 'SATA Mode (AHCI)',
    required: () => true,
    riskLevel: 'medium',
    matchTokens: ['sata mode', 'ahci'],
    fallbackDescription: 'macOS expects AHCI-compatible SATA mode.',
  },
  {
    id: 'vt-d',
    name: 'VT-d / IOMMU',
    required: () => false,
    riskLevel: 'medium',
    matchTokens: ['vt-d', 'iommu'],
    fallbackDescription: 'Disable VT-d / IOMMU unless you have a known-good OpenCore remapping path.',
  },
  {
    id: 'svm',
    name: 'SVM / CPU Virtualisation',
    required: () => false,
    riskLevel: 'low',
    matchTokens: ['svm', 'vt-x', 'virtualization'],
    fallbackDescription: 'Useful for virtualization-related paths and some firmware expectations.',
  },
  {
    id: 'above4g',
    name: 'Above 4G Decoding',
    required: (profile) => !profile.isLaptop || /amd|nvidia/i.test(profile.gpu),
    riskLevel: 'medium',
    matchTokens: ['above 4g'],
    fallbackDescription: 'Required for many modern GPU paths.',
  },
  {
    id: 'xhci-handoff',
    name: 'EHCI/XHCI Hand-off',
    required: () => true,
    riskLevel: 'low',
    matchTokens: ['xhci hand-off', 'ehci/xhci hand-off', 'usb controller hand-off'],
    fallbackDescription: 'Lets macOS take control of USB controllers.',
  },
  {
    id: 'cfg-lock',
    name: 'CFG Lock',
    required: () => false,
    riskLevel: 'high',
    matchTokens: ['cfg lock'],
    fallbackDescription: 'Prefer disabling CFG Lock, but OpenCore quirk fallback exists.',
  },
  {
    id: 'fast-boot',
    name: 'Fast Boot',
    required: () => false,
    riskLevel: 'low',
    matchTokens: ['fast boot'],
    fallbackDescription: 'Disable Fast Boot to avoid skipped device initialization.',
  },
  {
    id: 'intel-sgx',
    name: 'Intel SGX',
    required: () => false,
    riskLevel: 'low',
    matchTokens: ['intel sgx', 'sgx'],
    fallbackDescription: 'Disable Intel SGX on Intel systems.',
  },
  {
    id: 'platform-trust',
    name: 'Intel Platform Trust',
    required: () => false,
    riskLevel: 'low',
    matchTokens: ['platform trust', 'ptt'],
    fallbackDescription: 'Disable Intel Platform Trust when the board exposes it.',
  },
];

function findConfigEntry(biosConfig: BIOSConfig, meta: SettingMeta) {
  const allEntries = [...biosConfig.enable, ...biosConfig.disable];
  const lowerTokens = meta.matchTokens.map(token => token.toLowerCase());
  return allEntries.find(entry => {
    const haystack = `${entry.name} ${entry.plainTitle ?? ''}`.toLowerCase();
    return lowerTokens.some(token => haystack.includes(token));
  }) ?? null;
}

function summarizeRequirement(
  settingId: BiosSettingId,
  firmwareInfo: FirmwareInfo | null,
): {
  currentStatus: string;
  currentValue: string | null;
  confidence: BiosDetectionConfidence;
  detectionMethod: string;
} {
  const requirementId =
    settingId === 'uefi-mode' ? 'uefi-mode'
    : settingId === 'secure-boot' ? 'secure-boot'
    : settingId === 'vt-d' ? 'vt-d'
    : settingId === 'above4g' ? 'above4g'
    : settingId === 'svm' ? 'vt-x'
    : null;

  if (!firmwareInfo || !requirementId) {
    return {
      currentStatus: 'Unknown',
      currentValue: null,
      confidence: 'low',
      detectionMethod: 'Manual firmware inspection required',
    };
  }

  const requirement = firmwareInfo.requirements.find(item => item.id === requirementId);
  if (!requirement) {
    return {
      currentStatus: 'Unknown',
      currentValue: null,
      confidence: 'low',
      detectionMethod: 'Manual firmware inspection required',
    };
  }

  const confidence =
    requirement.status === 'confirmed' || requirement.status === 'failing' ? 'high'
    : requirement.status === 'inferred' ? 'medium'
    : 'low';

  return {
    currentStatus:
      requirement.status === 'confirmed' ? 'Verified'
      : requirement.status === 'inferred' ? 'Likely configured'
      : requirement.status === 'failing' ? 'Needs change'
      : 'Unknown',
    currentValue: requirement.detectedValue,
    confidence,
    detectionMethod: requirement.source,
  };
}

function detectVendorBackend(firmwareInfo: FirmwareInfo | null, profile: HardwareProfile): BiosBackend {
  const fingerprint = `${firmwareInfo?.vendor ?? ''} ${profile.motherboard}`.trim();
  for (const backend of BIOS_BACKENDS) {
    if (backend.id === 'generic') continue;
    if (backend.vendorMatchers.some(matcher => matcher.test(fingerprint))) {
      return backend;
    }
  }
  return genericBiosBackend;
}

function getAllowedApplyModes(supportLevel: BiosSupportLevel, safeMode: boolean): BiosApplyMode[] {
  const base: BiosApplyMode[] = ['manual', 'skipped'];
  if (supportLevel === 'manual') return base;
  if (supportLevel === 'assisted' || (supportLevel === 'managed' && safeMode)) {
    return ['manual', 'assisted', 'skipped'];
  }
  return ['manual', 'assisted', 'skipped'];
}

function getDefaultApplyMode(supportLevel: BiosSupportLevel, safeMode: boolean): BiosApplyMode {
  if (supportLevel === 'manual') return 'manual';
  if (safeMode) return 'manual';
  return supportLevel === 'managed' ? 'assisted' : 'assisted';
}

function selectOverallSupportLevel(levels: BiosSupportLevel[]): BiosSupportLevel {
  if (levels.includes('managed')) return 'managed';
  if (levels.includes('assisted')) return 'assisted';
  return 'manual';
}

export function createDefaultSelections(settings: Pick<BiosSettingPlan, 'id' | 'supportLevel'>[], safeMode: boolean): Record<BiosSettingId, BiosSettingSelection> {
  const selections = {} as Record<BiosSettingId, BiosSettingSelection>;
  for (const setting of settings) {
    selections[setting.id] = {
      approved: false,
      applyMode: getDefaultApplyMode(setting.supportLevel, safeMode),
    };
  }
  return selections;
}

function normalizeSelections(
  settings: Pick<BiosSettingPlan, 'id' | 'supportLevel'>[],
  safeMode: boolean,
  existing?: Record<string, BiosSettingSelection> | null,
): Record<BiosSettingId, BiosSettingSelection> {
  const defaults = createDefaultSelections(settings, safeMode);
  if (!existing) return defaults;

  for (const setting of settings) {
    const prior = existing[setting.id];
    if (prior) {
      defaults[setting.id] = {
        approved: Boolean(prior.approved),
        applyMode: getAllowedApplyModes(setting.supportLevel, safeMode).includes(prior.applyMode)
          ? prior.applyMode
          : defaults[setting.id].applyMode,
      };
    }
  }
  return defaults;
}

export function buildBiosOrchestratorState(input: {
  profile: HardwareProfile;
  biosConfig: BIOSConfig;
  firmwareInfo: FirmwareInfo | null;
  platform: NodeJS.Platform;
  safeMode?: boolean;
  session?: BiosSessionState | null;
}): BiosOrchestratorState {
  const safeMode = input.safeMode ?? true;
  const backend = detectVendorBackend(input.firmwareInfo, input.profile);
  const ctx: BiosOrchestratorContext = {
    profile: input.profile,
    biosConfig: input.biosConfig,
    firmwareInfo: input.firmwareInfo,
    platform: input.platform,
    safeMode,
  };
  const hardwareFingerprint = buildHardwareFingerprint(input.profile);
  const session = input.session && input.session.hardwareFingerprint === hardwareFingerprint ? input.session : null;

  const settings = SETTING_META
    .filter(meta => {
      if (meta.id === 'vt-d' && input.profile.architecture === 'AMD') return false;
      if (meta.id === 'svm' && input.profile.architecture !== 'AMD') return false;
      if ((meta.id === 'intel-sgx' || meta.id === 'platform-trust') && input.profile.architecture !== 'Intel') return false;
      return true;
    })
    .map<BiosSettingPlan>(meta => {
      const configEntry = findConfigEntry(input.biosConfig, meta);
      const supportLevel = backend.getSupportLevel(meta.id, ctx);
      const requirementSummary = summarizeRequirement(meta.id, input.firmwareInfo);
      return {
        id: meta.id,
        name: meta.name,
        description: configEntry?.description ?? meta.fallbackDescription,
        plainTitle: configEntry?.plainTitle,
        biosLocation: configEntry?.biosLocation,
        jargonDef: configEntry?.jargonDef,
        currentStatus: requirementSummary.currentStatus,
        currentValue: requirementSummary.currentValue,
        recommendedValue: configEntry?.value ?? (meta.id === 'csm' ? 'Disable' : 'Enable'),
        confidence: requirementSummary.confidence,
        detectionMethod: requirementSummary.detectionMethod,
        riskLevel: meta.riskLevel,
        supportLevel,
        allowedApplyModes: getAllowedApplyModes(supportLevel, safeMode),
        applyMode: getDefaultApplyMode(supportLevel, safeMode),
        verificationStatus: 'unknown',
        verificationDetail: 'Not yet verified.',
        required: meta.required(input.profile),
      };
    });

  const selections = normalizeSelections(settings, safeMode, session?.selectedChanges ?? null);
  const verification = verifyBiosSelections({
    settings,
    firmwareInfo: input.firmwareInfo,
    selectedChanges: selections,
  });

  const hydratedSettings = settings.map(setting => {
    const selection = selections[setting.id];
    const verificationRow = verification.rows[setting.id];
    return {
      ...setting,
      applyMode: selection.applyMode,
      verificationStatus: verificationRow.status,
      verificationDetail: verificationRow.detail,
      detectionMethod: verificationRow.detectionMethod,
      confidence: verificationRow.confidence,
    };
  });

  const requiredSettings = hydratedSettings.filter(setting => setting.required);
  const completedRequiredCount = requiredSettings.filter(setting => verification.rows[setting.id].status === 'verified').length;
  const overallSupportLevel = selectOverallSupportLevel(hydratedSettings.map(setting => setting.supportLevel));
  const stage = session?.stage ?? 'idle';
  const summary =
    verification.readyToBuild
      ? 'Required BIOS settings are verified for this hardware fingerprint.'
      : `Build remains blocked until ${verification.blockingIssues.length} required BIOS setting${verification.blockingIssues.length === 1 ? ' is' : 's are'} verified.`;

  return {
    vendor: backend.vendor,
    backendId: backend.id,
    backendLabel: backend.label,
    supportLevel: overallSupportLevel,
    safeMode,
    rebootSupported: backend.rebootSupported(input.platform),
    stage,
    hardwareFingerprint,
    settings: hydratedSettings,
    requiredCompletionCount: requiredSettings.length,
    completedRequiredCount,
    readyToBuild: verification.readyToBuild,
    blockingIssues: verification.blockingIssues,
    session,
    summary,
  };
}
