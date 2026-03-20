import type { FirmwareInfo, RequirementStatus } from '../firmwarePreflight.js';
import type {
  BiosDetectionConfidence,
  BiosOrchestratorState,
  BiosSettingId,
  BiosSettingSelection,
  BiosVerificationResult,
  BiosVerificationRow,
} from './types.js';

function mapRequirementStatus(status: RequirementStatus): {
  verification: BiosVerificationRow['status'];
  confidence: BiosDetectionConfidence;
} {
  switch (status) {
    case 'confirmed':
      return { verification: 'verified', confidence: 'high' };
    case 'inferred':
      return { verification: 'verified', confidence: 'medium' };
    case 'failing':
      return { verification: 'unverified', confidence: 'high' };
    case 'unverified':
    case 'not_applicable':
    default:
      return { verification: 'unknown', confidence: 'low' };
  }
}

function getRequirement(
  firmwareInfo: FirmwareInfo | null,
  id: 'uefi-mode' | 'secure-boot' | 'vt-d' | 'above4g' | 'vt-x',
) {
  return firmwareInfo?.requirements.find(requirement => requirement.id === id) ?? null;
}

function buildRowFromFirmware(
  settingId: BiosSettingId,
  firmwareInfo: FirmwareInfo | null,
): BiosVerificationRow | null {
  const requirementId =
    settingId === 'uefi-mode' ? 'uefi-mode'
    : settingId === 'secure-boot' ? 'secure-boot'
    : settingId === 'vt-d' ? 'vt-d'
    : settingId === 'above4g' ? 'above4g'
    : settingId === 'svm' ? 'vt-x'
    : null;

  if (!requirementId) return null;
  const requirement = getRequirement(firmwareInfo, requirementId);
  if (!requirement) return null;

  const mapped = mapRequirementStatus(requirement.status);
  return {
    id: settingId,
    status: mapped.verification,
    detail: requirement.detectedValue
      ? `${requirement.detectedValue} (${requirement.source})`
      : requirement.source,
    detectionMethod: requirement.source,
    confidence: mapped.confidence,
  };
}

export function verifyBiosSelections(input: {
  settings: BiosOrchestratorState['settings'];
  firmwareInfo: FirmwareInfo | null;
  selectedChanges: Record<string, BiosSettingSelection>;
}): BiosVerificationResult {
  const rows = {} as Record<BiosSettingId, BiosVerificationRow>;
  const blockingIssues: string[] = [];

  for (const setting of input.settings) {
    const selection = input.selectedChanges[setting.id];
    const firmwareRow = buildRowFromFirmware(setting.id, input.firmwareInfo);
    let status: BiosVerificationRow['status'] = firmwareRow?.status ?? 'unknown';
    let detail = firmwareRow?.detail ?? 'No firmware readback available for this setting.';
    let detectionMethod = firmwareRow?.detectionMethod ?? setting.detectionMethod;
    let confidence = firmwareRow?.confidence ?? setting.confidence;

    if (selection?.approved && selection.applyMode !== 'skipped' && status === 'unknown') {
      status = 'verified';
      detail = 'User confirmed this setting was applied in firmware.';
      detectionMethod = 'manual confirmation';
      confidence = 'medium';
    }

    if (selection?.applyMode === 'skipped' && setting.required) {
      status = 'unverified';
      detail = 'Required BIOS setting was explicitly skipped.';
      detectionMethod = 'user selection';
      confidence = 'high';
    }

    rows[setting.id] = {
      id: setting.id,
      status,
      detail,
      detectionMethod,
      confidence,
    };

    if (setting.required && status !== 'verified') {
      blockingIssues.push(`${setting.name} is not verified`);
    }
  }

  return {
    rows,
    readyToBuild: blockingIssues.length === 0,
    blockingIssues,
  };
}
