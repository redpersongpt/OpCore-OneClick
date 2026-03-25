const LAPTOP_CHASSIS = new Set([8, 9, 10, 11, 12, 14, 18, 21, 31, 32]);

// CPU suffixes that are ALWAYS laptop/mobile — never used in desktop CPUs.
// U = ultra-low-power mobile, Y = extreme low-power mobile.
// These alone are strong enough evidence for laptop classification.
// The \d before the suffix ensures we match the model number suffix (e.g. "4510U"),
// not random U/Y letters elsewhere in the CPU name string.
// Y-series uses "7Y30" format where Y appears mid-model — handled by the second pattern.
const DEFINITIVE_MOBILE_CPU = /\d(U)\s*(?:CPU|@|\b)|\d(Y)\d/i;

// CPU suffixes that are mobile-class but occasionally used in small form factor
// desktops (NUCs, mini PCs). Need battery or model hint to confirm laptop.
const MOBILE_CPU_SUFFIX = /\d(HQ|MQ|G[1-7]|H|HS|HX|P)\s*(?:CPU|@|\b)/i;

const PORTABLE_MODEL_HINT = /\b(book|laptop|notebook|ultrabook|surface|thinkpad|ideapad|yoga|elitebook|probook|latitude|precision|xps|zenbook|vivobook|travelmate|lifebook|spectre|envy|pavilion.*laptop|inspiron.*laptop|vostro)\b/i;

// OEM manufacturer + model patterns that definitively indicate a laptop family
// even when chassis type or battery data is unavailable.
const OEM_LAPTOP_MODEL = /\b(probook|elitebook|zbook|latitude|thinkpad|ideapad|yoga|zenbook|vivobook|travelmate|lifebook|spectre|swift|aspire.*notebook)\b/i;

// Mobile GPU naming patterns — discrete GPUs with mobile suffixes
const MOBILE_GPU_HINT = /\b(M\d{3,4}|MX\d{2,3}|GTX?\s*\d{3,4}M)\b/i;

export interface FormFactorEvidence {
  cpuName: string;
  chassisTypes?: number[];
  modelName?: string;
  batteryPresent?: boolean;
  /** System manufacturer (e.g. "Hewlett-Packard", "Dell Inc.", "LENOVO") */
  manufacturer?: string;
  /** GPU name(s) for mobile GPU suffix detection */
  gpuName?: string;
}

/**
 * Determine whether the machine is a laptop using weighted evidence fusion.
 *
 * Uses three tiers of evidence:
 * - DEFINITIVE: Any single definitive signal is sufficient (chassis type, model name hint,
 *   U/Y CPU suffix). These are never ambiguous.
 * - STRONG: Two or more strong signals together are sufficient (mobile CPU suffix +
 *   battery, mobile CPU suffix + mobile GPU, battery + manufacturer laptop family).
 * - FALLBACK: Conservative — insufficient evidence stays desktop.
 *
 * This prevents old office laptops from being misclassified as desktops when one
 * fragile signal (chassis query, battery query) times out or fails.
 */
export function inferLaptopFormFactor(evidence: FormFactorEvidence): boolean {
  const chassisTypes = evidence.chassisTypes ?? [];
  const modelName = evidence.modelName?.trim() ?? '';
  const manufacturer = evidence.manufacturer?.trim() ?? '';
  const gpuName = evidence.gpuName?.trim() ?? '';

  // ── Tier 1: Definitive signals (any one is sufficient) ──

  // Chassis type from SMBIOS — authoritative when present
  if (chassisTypes.some((value) => LAPTOP_CHASSIS.has(value))) {
    return true;
  }

  // Model name matches known laptop family (e.g. "HP ProBook 450 G2")
  if (modelName && PORTABLE_MODEL_HINT.test(modelName)) {
    return true;
  }

  // U/Y CPU suffix — these are NEVER used in desktop CPUs
  if (DEFINITIVE_MOBILE_CPU.test(evidence.cpuName)) {
    return true;
  }

  // ── Tier 2: Strong combined signals (two or more needed) ──

  const hasMobileCpu = MOBILE_CPU_SUFFIX.test(evidence.cpuName);
  const hasBattery = evidence.batteryPresent === true;
  const hasLaptopModel = OEM_LAPTOP_MODEL.test(modelName) || OEM_LAPTOP_MODEL.test(manufacturer);
  const hasMobileGpu = MOBILE_GPU_HINT.test(gpuName);

  // Mobile CPU (H/HQ/MQ/etc.) + battery → laptop
  if (hasMobileCpu && hasBattery) return true;

  // Mobile CPU + known laptop OEM model → laptop
  if (hasMobileCpu && hasLaptopModel) return true;

  // Battery + known laptop OEM model → laptop
  if (hasBattery && hasLaptopModel) return true;

  // Mobile GPU (e.g. "R5 M255", "MX150") + any other laptop hint → laptop
  if (hasMobileGpu && (hasMobileCpu || hasBattery || hasLaptopModel)) return true;

  // Battery + mobile GPU → laptop
  if (hasBattery && hasMobileGpu) return true;

  return false;
}
