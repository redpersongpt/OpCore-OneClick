const LAPTOP_CHASSIS = new Set([8, 9, 10, 11, 12, 14, 18, 21, 31, 32]);
const LAPTOP_CPU_SUFFIX = /(U|Y|HQ|MQ|G[1-7]|H|HS|HX|P)\s*(?:CPU|@|\b)/i;
const PORTABLE_MODEL_HINT = /\b(book|laptop|notebook|ultrabook|surface)\b/i;

export interface FormFactorEvidence {
  cpuName: string;
  chassisTypes?: number[];
  modelName?: string;
  batteryPresent?: boolean;
}

export function inferLaptopFormFactor(evidence: FormFactorEvidence): boolean {
  const chassisTypes = evidence.chassisTypes ?? [];
  if (chassisTypes.some((value) => LAPTOP_CHASSIS.has(value))) {
    return true;
  }

  const modelName = evidence.modelName?.trim() ?? '';
  if (modelName && PORTABLE_MODEL_HINT.test(modelName)) {
    return true;
  }

  if (LAPTOP_CPU_SUFFIX.test(evidence.cpuName) && evidence.batteryPresent === true) {
    return true;
  }

  return false;
}
