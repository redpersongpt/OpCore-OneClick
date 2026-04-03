import type { HardwareProfile } from './configGenerator.js';
import type { CommunityEvidenceSummary } from './communityEvidence.js';
import { getCommunityEvidenceSummary } from './communityEvidence.js';
import {
  classifyGpu,
  getBestSupportedGpuPath,
  getEligibleMacOSVersions,
  getGpuCeiling,
  getProfileGpuDevices,
  hasSupportedDisplayPath,
  hasUnsupportedDiscreteGpu,
  parseMacOSVersion,
} from './hackintoshRules.js';
import { getBroadcomWifiPolicy } from './wifiPolicy.js';

export type CompatibilityLevel =
  | 'supported'
  | 'experimental'
  | 'risky'
  | 'blocked';

export type ConfigStrategy =
  | 'canonical'
  | 'conservative'
  | 'blocked';

export type CompatibilityGuidanceSource = 'rule' | 'community' | 'fallback';
export type CompatibilityGuidanceConfidence = 'high' | 'medium' | 'low';
export type CompatibilityFailureLikelihood = 'very likely' | 'likely' | 'possible';

export interface CompatibilityNextAction {
  title: string;
  detail: string;
  source: CompatibilityGuidanceSource;
  confidence: CompatibilityGuidanceConfidence;
}

export interface CompatibilityAdvisoryConfidence {
  score: number;
  label: 'High confidence' | 'Medium confidence' | 'Low confidence';
  explanation: string;
}

export interface CompatibilityFailurePoint {
  title: string;
  detail: string;
  likelihood: CompatibilityFailureLikelihood;
  source: CompatibilityGuidanceSource;
}

export interface CompatibilityReport {
  level: CompatibilityLevel;
  strategy: ConfigStrategy;
  confidence: 'high' | 'medium' | 'low';
  explanation: string;
  manualVerificationRequired: boolean;
  isCompatible: boolean;
  maxOSVersion: string;
  eligibleVersions: { id: string; name: string; icon: string }[];
  recommendedVersion: string;
  warnings: string[];
  errors: string[];
  minReqMet: boolean;
  communityEvidence: CommunityEvidenceSummary | null;
  nextActions: CompatibilityNextAction[];
  advisoryConfidence: CompatibilityAdvisoryConfidence;
  mostLikelyFailurePoints: CompatibilityFailurePoint[];
}

const COMPATIBILITY_RANK: Record<CompatibilityLevel, number> = {
  supported: 0,
  experimental: 1,
  risky: 2,
  blocked: 3,
};

function capFromCpu(profile: HardwareProfile): number | null {
  const cpu = profile.cpu.toLowerCase();

  if (profile.architecture === 'Intel') {
    if (['Penryn', 'Wolfdale', 'Yorkfield'].includes(profile.generation)) return 10.13;
    if (['Nehalem', 'Arrandale', 'Clarkdale', 'Westmere'].includes(profile.generation)) return 11;
    if (['Sandy Bridge', 'Ivy Bridge', 'Ivy Bridge-E', 'Unknown'].includes(profile.generation)) return 12;
    if (['Haswell', 'Broadwell', 'Haswell-E', 'Broadwell-E'].includes(profile.generation)) return 12;
    if (cpu.includes('pentium') || cpu.includes('celeron') || cpu.includes('atom')) return 12;
  }

  return null;
}

function pushUnique(items: string[], value: string): void {
  if (!items.includes(value)) items.push(value);
}

function worsenCompatibilityLevel(current: CompatibilityLevel, next: CompatibilityLevel): CompatibilityLevel {
  return COMPATIBILITY_RANK[next] > COMPATIBILITY_RANK[current] ? next : current;
}

function applyAdvisoryLevel(
  report: CompatibilityReport,
  level: Exclude<CompatibilityLevel, 'supported' | 'blocked'>,
  explanation: string,
  warning?: string,
): void {
  const currentLevel = report.level;
  const nextLevel = worsenCompatibilityLevel(currentLevel, level);
  report.level = nextLevel;

  if (nextLevel !== 'supported') {
    report.strategy = 'conservative';
    report.manualVerificationRequired = true;
  }

  if (warning) pushUnique(report.warnings, warning);

  if (COMPATIBILITY_RANK[level] >= COMPATIBILITY_RANK[currentLevel]) {
    report.explanation = explanation;
  }
}

function setBlocked(report: CompatibilityReport, explanation: string): CompatibilityReport {
  report.level = 'blocked';
  report.strategy = 'blocked';
  report.isCompatible = false;
  report.explanation = explanation;
  report.eligibleVersions = [];
  report.recommendedVersion = '';
  report.maxOSVersion = 'Blocked';
  report.nextActions = [];
  report.advisoryConfidence = {
    score: 15,
    label: 'Low confidence',
    explanation: 'Low confidence because the current hardware or selected target is blocked by the compatibility engine.',
  };
  report.mostLikelyFailurePoints = [];
  return report;
}

function isOlderIntelLaptop(profile: HardwareProfile): boolean {
  return profile.architecture === 'Intel'
    && profile.isLaptop
    && ['Penryn', 'Nehalem', 'Arrandale', 'Clarkdale', 'Westmere', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell', 'Unknown'].includes(profile.generation);
}

function buildCommunityNote(evidence: CommunityEvidenceSummary | null): string {
  if (!evidence?.highestReportedVersion) return '';
  return ` Community evidence for similar hardware is strongest around ${evidence.highestReportedVersion}, with manual caveats still reported.`;
}

function buildCompatibilityConfidenceLabel(score: number): CompatibilityAdvisoryConfidence['label'] {
  if (score >= 70) return 'High confidence';
  if (score >= 45) return 'Medium confidence';
  return 'Low confidence';
}

function buildCompatibilityAdvisoryConfidence(
  profile: HardwareProfile,
  report: CompatibilityReport,
  targetVersion: number,
): CompatibilityAdvisoryConfidence {
  let score = 55;

  if (report.level === 'supported') score += 20;
  else if (report.level === 'experimental') score -= 5;
  else if (report.level === 'risky') score -= 18;
  else score -= 35;

  if (profile.scanConfidence === 'high') score += 8;
  else if (profile.scanConfidence === 'medium') score += 2;
  else score -= 10;

  const evidence = report.communityEvidence;
  if (evidence) {
    if (evidence.signal === 'strong') score += 14;
    else if (evidence.signal === 'moderate') score += 8;
    else if (evidence.signal === 'weak') score += 3;

    if (evidence.bestMatchScore >= 6) score += 8;
    else if (evidence.bestMatchScore >= 5) score += 4;

    if (evidence.highestReportedVersionNumeric != null) {
      if (targetVersion <= evidence.highestReportedVersionNumeric) score += 6;
      else score -= 10;
    }
  } else if (report.level !== 'supported') {
    score -= 8;
  }

  if (report.warnings.length > 0) score -= Math.min(report.warnings.length * 2, 8);
  if (report.errors.length > 0) score -= 25;

  const boundedScore = Math.max(0, Math.min(100, score));
  const label = buildCompatibilityConfidenceLabel(boundedScore);
  const explanation = evidence
    ? `${label} based on ${evidence.signal} community evidence, a ${evidence.matchLevel} community match, and the selected macOS target relative to documented success ceilings.`
    : `${label} based on the current hardware match, target macOS version, and rule-based compatibility analysis without strong community corroboration.`;

  return {
    score: boundedScore,
    label,
    explanation,
  };
}

function buildCompatibilityNextActions(
  profile: HardwareProfile,
  report: CompatibilityReport,
  targetVersion: number,
  bestDisplayPathName: string | null,
  bestSelectedDisplayPathName: string | null,
): CompatibilityNextAction[] {
  const actions: CompatibilityNextAction[] = [];
  const addAction = (action: CompatibilityNextAction): void => {
    if (!actions.some((existing) => existing.title === action.title && existing.detail === action.detail)) {
      actions.push(action);
    }
  };

  if (report.level === 'blocked') return actions;

  if (report.level === 'experimental' || report.level === 'risky') {
    addAction({
      title: `Start with ${report.recommendedVersion || profile.targetOS}`,
      detail: report.recommendedVersion && report.recommendedVersion !== profile.targetOS
        ? `Your current target is ${profile.targetOS}, but the safest known starting point is ${report.recommendedVersion}. Use that first, then only try newer versions after validation and simulation look clean.`
        : `Use ${report.recommendedVersion || profile.targetOS} as the first pass. This is the most plausible target before you start testing newer or less-proven versions.`,
      source: report.communityEvidence ? 'community' : 'rule',
      confidence: report.communityEvidence ? 'high' : 'medium',
    });

    addAction({
      title: 'Run Simulation before testing real hardware',
      detail: 'Use Safe Simulation to preview the generated EFI, validation findings, and resource plan before you spend time troubleshooting a manual install path.',
      source: 'rule',
      confidence: 'high',
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /sleep/i.test(item))) {
    addAction({
      title: 'Treat sleep as optional first',
      detail: 'Community reports for similar hardware often break on sleep or wake. Start by disabling sleep, then try ACPI sleep fixes and USB mapping only after the system is stable enough to boot and idle reliably.',
      source: 'community',
      confidence: 'medium',
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /audio|alc|layout/i.test(item))) {
    addAction({
      title: 'Tune AppleALC layout-id if audio is missing',
      detail: 'If audio fails, test alternative AppleALC layout-id values for the codec on this board class. Community-proven laptop paths often need layout-id swaps before speakers or headphone output behave correctly.',
      source: 'community',
      confidence: 'medium',
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /trackpad|keyboard|gesture|ps2/i.test(item))) {
    addAction({
      title: 'Expect PS2 input tuning on older laptops',
      detail: 'If the trackpad or keyboard behaves badly, compare VoodooPS2Controller variants and vendor-specific configs. Older ThinkPad-class machines often need a different PS2 stack than the default one.',
      source: 'community',
      confidence: 'medium',
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /thunderbolt/i.test(item))) {
    addAction({
      title: 'Treat Thunderbolt as optional until core boot is stable',
      detail: 'Thunderbolt often remains unresolved even on otherwise working laptop builds. Verify boot, graphics, audio, and storage first, then return to Thunderbolt tuning as a separate task.',
      source: 'community',
      confidence: 'medium',
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /wifi|bluetooth|wireless/i.test(item))) {
    addAction({
      title: 'Plan for wireless card or kext tuning',
      detail: 'Community reports for similar hardware often need Wi-Fi/Bluetooth tweaks. Be ready to swap between AirportItlwm and Itlwm or replace the card if you need full Apple wireless features.',
      source: 'community',
      confidence: 'medium',
    });
  }

  if (report.warnings.some((warning) => /broadcom wi-fi/i.test(warning))) {
    addAction({
      title: 'Validate the Broadcom wireless path separately',
      detail: report.warnings.some((warning) => /sonoma|sequoia|tahoe|root patches|oclp/i.test(warning))
        ? 'This Broadcom Wi-Fi path is no longer a clean native route on newer macOS targets. Expect OCLP/root patches or a replacement card if wireless matters.'
        : 'Broadcom Wi-Fi support on this card family is conditional. Confirm the exact card model, verify whether AirportBrcmFixup is enough, and treat Bluetooth as a separate check.',
      source: 'rule',
      confidence: 'medium',
    });
  }

  if (report.warnings.some((warning) => /discrete gpu/i.test(warning))) {
    addAction({
      title: 'Force the laptop onto the iGPU path',
      detail: 'Unsupported laptop dGPUs should stay disabled. Use the integrated display route only, verify that internal and external outputs stay on the iGPU, and avoid treating the dGPU as a boot requirement.',
      source: 'rule',
      confidence: 'high',
    });
  }

  if (!bestSelectedDisplayPathName) {
    addAction({
      title: 'Verify the real display path before trusting this EFI',
      detail: bestDisplayPathName
        ? `The broader hardware scan found ${bestDisplayPathName}, but not for ${profile.targetOS}. Check which ports and panel are actually connected to the supported GPU path and confirm framebuffer assumptions before boot testing.`
        : 'The current scan could not prove a supported display route. Confirm the active iGPU path, framebuffer assumptions, and panel routing before boot testing.',
      source: report.communityEvidence ? 'community' : 'fallback',
      confidence: report.communityEvidence ? 'medium' : 'low',
    });
  }

  if (profile.architecture === 'AMD' && profile.isLaptop) {
    addAction({
      title: 'Expect manual AMD laptop patching',
      detail: 'AMD laptops remain a narrow path. Keep expectations limited, verify the exact iGPU/dGPU route, and plan around NootedRed or AMD dGPU-specific workarounds rather than assuming a clean install.',
      source: 'fallback',
      confidence: 'low',
    });
  }

  if (report.communityEvidence?.highestReportedVersionNumeric != null && targetVersion > report.communityEvidence.highestReportedVersionNumeric) {
    addAction({
      title: `Prefer ${report.communityEvidence.highestReportedVersion} before newer macOS targets`,
      detail: `Documented success for similar hardware is strongest at ${report.communityEvidence.highestReportedVersion}. Newer versions may still be possible, but they are less proven and should be treated as an upgrade experiment after you reach a stable base install.`,
      source: 'community',
      confidence: 'high',
    });
  }

  if (report.level === 'experimental' || report.level === 'risky') {
    addAction({
      title: 'Change one variable at a time once the conservative base boots',
      detail: 'Stabilize the conservative target first, then test one change at a time: newer macOS, alternate SMBIOS, alternate layout-id, or a different input stack.',
      source: 'fallback',
      confidence: report.level === 'risky' ? 'low' : 'medium',
    });

    if (profile.isLaptop) {
      addAction({
        title: 'Compare alternative laptop tuning only after baseline validation passes',
        detail: 'If the base EFI validates and boots, compare alternate AppleALC layout-id values, VoodooPS2 variants, and device-id or framebuffer tweaks one at a time. Keep rollback notes for every experiment.',
        source: report.communityEvidence ? 'community' : 'fallback',
        confidence: report.communityEvidence ? 'medium' : 'low',
      });
    }
  }

  return actions.slice(0, 6);
}

interface FailurePointCandidate extends CompatibilityFailurePoint {
  weight: number;
}

function buildFailureLikelihood(weight: number): CompatibilityFailureLikelihood {
  if (weight >= 8) return 'very likely';
  if (weight >= 5) return 'likely';
  return 'possible';
}

function buildMostLikelyFailurePoints(
  profile: HardwareProfile,
  report: CompatibilityReport,
  targetVersion: number,
  bestDisplayPathName: string | null,
  bestSelectedDisplayPathName: string | null,
): CompatibilityFailurePoint[] {
  const candidates = new Map<string, FailurePointCandidate>();
  const addCandidate = (
    key: string,
    candidate: Omit<FailurePointCandidate, 'likelihood'> & { likelihood?: CompatibilityFailureLikelihood },
  ): void => {
    const next: FailurePointCandidate = {
      ...candidate,
      likelihood: candidate.likelihood ?? buildFailureLikelihood(candidate.weight),
    };
    const existing = candidates.get(key);
    if (!existing || next.weight > existing.weight) {
      candidates.set(key, next);
    }
  };

  if (report.level !== 'experimental' && report.level !== 'risky') {
    return [];
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /sleep/i.test(item)) || (profile.isLaptop && report.level === 'risky')) {
    addCandidate('sleep', {
      title: 'Sleep instability',
      detail: 'Sleep and wake are a common failure point on this class of machine. Expect to disable sleep first, then test ACPI sleep fixes and USB tuning after the base install is stable.',
      source: report.communityEvidence?.whatDidNotWork.some((item) => /sleep/i.test(item)) ? 'community' : 'rule',
      weight: report.communityEvidence?.whatDidNotWork.some((item) => /sleep/i.test(item)) ? 9 : 6,
    });
  }

  if (
    report.communityEvidence?.whatDidNotWork.some((item) => /audio|alc|layout/i.test(item))
    || profile.bootArgs.includes('alcid=')
    || profile.kexts.includes('AppleALC')
  ) {
    addCandidate('audio', {
      title: 'Audio may require manual layout-id tuning',
      detail: 'AppleALC usually gets you close, but speakers, combo jacks, or internal microphones often need alternate layout-id values or codec-specific tuning on laptop-class boards.',
      source: report.communityEvidence?.whatDidNotWork.some((item) => /audio|alc|layout/i.test(item)) ? 'community' : 'rule',
      weight: report.communityEvidence?.whatDidNotWork.some((item) => /audio|alc|layout/i.test(item)) ? 8 : 5,
    });
  }

  if (
    report.communityEvidence?.whatDidNotWork.some((item) => /trackpad|keyboard|gesture|ps2/i.test(item))
    || (profile.isLaptop && profile.kexts.includes('VoodooPS2Controller'))
  ) {
    addCandidate('input', {
      title: 'Trackpad or keyboard reliability',
      detail: 'Older laptops often need PS2 stack swaps or vendor-specific input tuning. Expect to compare VoodooPS2 variants and trackpad settings if input feels unstable.',
      source: report.communityEvidence?.whatDidNotWork.some((item) => /trackpad|keyboard|gesture|ps2/i.test(item)) ? 'community' : 'rule',
      weight: report.communityEvidence?.whatDidNotWork.some((item) => /trackpad|keyboard|gesture|ps2/i.test(item)) ? 8 : 5,
    });
  }

  if (report.warnings.some((warning) => /discrete gpu/i.test(warning)) || !bestSelectedDisplayPathName) {
    addCandidate('display-path', {
      title: 'Display routing or unsupported dGPU path',
      detail: bestDisplayPathName
        ? `The broader scan found ${bestDisplayPathName}, but not a clean display path for ${profile.targetOS}. Internal panel or external outputs may still be routed through the wrong GPU.`
        : 'The scan did not confirm a clean supported display path for the selected target. Boot may fail until the active panel and output routing are verified.',
      source: !bestSelectedDisplayPathName && report.communityEvidence ? 'community' : 'rule',
      weight: !bestSelectedDisplayPathName ? 9 : 7,
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /wifi|bluetooth|wireless/i.test(item))) {
    addCandidate('wireless', {
      title: 'Wireless card or Bluetooth tuning',
      detail: 'Community builds for similar hardware frequently need AirportItlwm vs Itlwm swaps, BIOS toggles, or a different wireless card before Apple-style Wi-Fi and Bluetooth behave correctly.',
      source: 'community',
      weight: 6,
    });
  }

  if (report.warnings.some((warning) => /broadcom wi-fi/i.test(warning))) {
    addCandidate('broadcom-wireless', {
      title: 'Broadcom Wi-Fi support path may still need manual validation',
      detail: report.warnings.some((warning) => /sonoma|sequoia|tahoe|root patches|oclp/i.test(warning))
        ? 'Newer macOS targets dropped this Broadcom path as a clean native route. Wireless may need OCLP/root patches or a replacement card even if the rest of the machine is bootable.'
        : 'This Broadcom Wi-Fi family is only conditionally supported. Expect card-model-specific validation before you trust wireless or Bluetooth behavior.',
      source: 'rule',
      weight: report.warnings.some((warning) => /sonoma|sequoia|tahoe|root patches|oclp/i.test(warning)) ? 8 : 5,
    });
  }

  if (report.communityEvidence?.whatDidNotWork.some((item) => /thunderbolt/i.test(item))) {
    addCandidate('thunderbolt', {
      title: 'Thunderbolt or advanced I/O features',
      detail: 'Thunderbolt, docks, and other advanced I/O paths often remain flaky even when the main system boots cleanly. Treat them as post-boot tuning work, not day-one expectations.',
      source: 'community',
      weight: 5,
    });
  }

  if (profile.motherboard.toLowerCase().includes('pm981') || profile.motherboard.toLowerCase().includes('pm991') || profile.motherboard.toLowerCase().includes('2200s') || profile.motherboard.toLowerCase().includes('600p')) {
    addCandidate('storage', {
      title: 'NVMe boot instability',
      detail: 'This storage controller family is known for macOS boot or resume instability. Even a working EFI can still hit random installer or boot failures until the storage path is swapped or tuned.',
      source: 'rule',
      weight: 8,
    });
  }

  if (report.communityEvidence?.highestReportedVersionNumeric != null && targetVersion > report.communityEvidence.highestReportedVersionNumeric) {
    addCandidate('os-ceiling', {
      title: 'Selected macOS version is above the community comfort zone',
      detail: `Similar hardware is documented most strongly around ${report.communityEvidence.highestReportedVersion}. Newer versions may still boot, but they are the first place to expect breakage.`,
      source: 'community',
      weight: 8,
    });
  } else if (report.level === 'risky' && isOlderIntelLaptop(profile)) {
    addCandidate('older-platform', {
      title: 'Older platform quirks will stack up quickly',
      detail: 'This path is viable mainly because the community keeps older Intel laptops alive with manual tweaks. Expect several small fixes rather than a single clean pass.',
      source: report.communityEvidence ? 'community' : 'fallback',
      weight: 6,
    });
  }

  return [...candidates.values()]
    .sort((left, right) => right.weight - left.weight)
    .slice(0, 3)
    .map(({ weight: _weight, ...point }) => point);
}

function hasLegacyIntelPlanningValue(profile: HardwareProfile): boolean {
  return profile.architecture === 'Intel'
    && ['Unknown', 'Penryn', 'Nehalem', 'Arrandale', 'Clarkdale', 'Westmere', 'Wolfdale', 'Yorkfield', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell', 'Skylake', 'Kaby Lake'].includes(profile.generation);
}

export function checkCompatibility(
  profile: HardwareProfile,
): CompatibilityReport {
  const report: CompatibilityReport = {
    level: 'supported',
    strategy: 'canonical',
    confidence: profile.scanConfidence || 'low',
    explanation: 'System appears to be a valid OpenCore target.',
    manualVerificationRequired: false,
    isCompatible: true,
    maxOSVersion: 'macOS Tahoe 26',
    eligibleVersions: getEligibleMacOSVersions(26),
    recommendedVersion: 'macOS Sequoia 15',
    warnings: [],
    errors: [],
    minReqMet: true,
    communityEvidence: null,
    nextActions: [],
    advisoryConfidence: {
      score: 50,
      label: 'Medium confidence',
      explanation: 'Medium confidence based on the current hardware profile and target macOS version.',
    },
    mostLikelyFailurePoints: [],
  };

  const targetVersion = parseMacOSVersion(profile.targetOS);
  const gpuDevices = getProfileGpuDevices(profile);
  const gpuAssessments = gpuDevices.map(classifyGpu);
  const bestAnyDisplayPath = getBestSupportedGpuPath(gpuDevices);
  const bestSelectedDisplayPath = getBestSupportedGpuPath(gpuDevices, targetVersion);
  const bestDisplayCeiling = getGpuCeiling(gpuDevices);
  const cpuCeiling = capFromCpu(profile);
  const motherboard = profile.motherboard.toLowerCase();
  const cpu = profile.cpu.toLowerCase();
  const communityEvidence = getCommunityEvidenceSummary(profile);
  const hasCommunityEvidence = communityEvidence.signal !== 'none';
  let advisoryFallbackCeiling: number | null = null;

  report.communityEvidence = hasCommunityEvidence ? communityEvidence : null;

  const broadcomWifiPolicy = getBroadcomWifiPolicy(profile.wifiChipset, profile.targetOS);
  if (broadcomWifiPolicy?.supportClass === 'sonoma_root_patch') {
    applyAdvisoryLevel(
      report,
      'risky',
      'Broadcom Wi-Fi path detected. Sonoma and newer removed this as a clean native wireless route, so EFI planning can continue but wireless now depends on OCLP/root patches or a replacement card.',
      `Broadcom Wi-Fi detected (${broadcomWifiPolicy.chipset}). Sonoma and newer are not a clean native path for this card family.`,
    );
  } else if (broadcomWifiPolicy?.supportClass === 'legacy_unsupported_on_target') {
    applyAdvisoryLevel(
      report,
      'risky',
      'Legacy Broadcom Wi-Fi path detected. The rest of the machine may still boot, but this wireless card family tops out around Catalina and should not be treated as a normal modern macOS path.',
      `Broadcom Wi-Fi detected (${broadcomWifiPolicy.chipset}). This older card family tops out around Catalina and is not a normal ${profile.targetOS} wireless path.`,
    );
  } else if (broadcomWifiPolicy?.supportClass === 'unknown_broadcom') {
    applyAdvisoryLevel(
      report,
      'experimental',
      'Broadcom Wi-Fi path detected, but the exact chipset support is not modeled cleanly in the generator yet. Planning can continue, but wireless should stay an explicit manual-validation item.',
      `Broadcom Wi-Fi detected (${broadcomWifiPolicy.chipset}), but the exact support path is not modeled yet.`,
    );
  } else if (broadcomWifiPolicy) {
    pushUnique(report.warnings, broadcomWifiPolicy.summary);
  }

  if (report.confidence === 'low') {
    applyAdvisoryLevel(
      report,
      'risky',
      'Hardware detection was incomplete. Planning can continue, but treat the result as a risky manual-verification path until the exact board, GPU, and output route are confirmed.',
    );
  } else if (report.confidence === 'medium') {
    applyAdvisoryLevel(
      report,
      'experimental',
      'Some hardware values were inferred. The build stays on conservative defaults and should be treated as experimental until the inferred devices are verified.',
    );
  }

  const ramGB = parseInt(profile.ram, 10) || 0;
  if (ramGB > 0 && ramGB < 4) {
    pushUnique(report.warnings, 'RAM is below 4 GB. The system may boot, but installer reliability and performance will be poor.');
    report.minReqMet = false;
    applyAdvisoryLevel(
      report,
      'experimental',
      'Low-memory system detected. Planning remains available, but the install path should be treated as experimental because memory pressure can destabilize recovery and the installer.',
    );
  }

  if (profile.architecture === 'Apple Silicon') {
    report.errors.push('Apple Silicon systems already run macOS natively and are not Hackintosh targets. You silly goose :) ');
    return setBlocked(report, 'Apple Silicon hardware is not a valid OpenCore/Hackintosh target.');
  }

  if (profile.isVM) {
    applyAdvisoryLevel(
      report,
      'risky',
      'Virtual machine target detected. Planning remains available as a lab path, but GPU passthrough and Metal support stay lower-confidence than bare-metal installs.',
      'Virtual machines need PCIe GPU passthrough for usable acceleration. Without passthrough, macOS will run in VESA mode only.',
    );
  }

  if (cpu.includes('pentium') || cpu.includes('celeron') || cpu.includes('atom')) {
    if (profile.isLaptop) {
      report.errors.push('Mobile Pentium, Celeron, and Atom systems are not valid Hackintosh targets.');
      return setBlocked(report, 'Unsupported mobile Intel CPU family.');
    }
    if (!hasSupportedDisplayPath(gpuDevices)) {
      report.errors.push('Desktop Pentium/Celeron requires a separate supported display GPU. No supported output path was detected.');
      return setBlocked(report, 'No supported display path remains on this low-end Intel system.');
    }

    applyAdvisoryLevel(
      report,
      'risky',
      'Low-end Intel desktop path detected. Planning can continue, but this remains a risky setup with limited macOS headroom and a higher chance of manual fixes.',
      'Low-end Intel desktop CPUs still need careful SMBIOS and display-path validation.',
    );
  }

  if (profile.architecture === 'AMD' && profile.isLaptop) {
    const hasLimitedAmdLaptopPath = gpuAssessments.some((assessment) =>
      assessment.requiresNootedRed ||
      assessment.name.toLowerCase().includes('5300m') ||
      assessment.name.toLowerCase().includes('5500m') ||
      assessment.name.toLowerCase().includes('5600m') ||
      assessment.name.toLowerCase().includes('5700m'),
    );

    if (!hasLimitedAmdLaptopPath) {
      report.errors.push('AMD laptops are not generally supported by canonical OpenCore automation. No documented supported display path was detected.');
      return setBlocked(report, 'Unsupported AMD laptop path.');
    }

    applyAdvisoryLevel(
      report,
      'risky',
      'AMD laptop path detected. A limited subset of AMD laptop GPUs can work, but this remains a risky manual-tuning route rather than a normal supported build.',
      'AMD laptop support is limited and lower-confidence even on the few documented working GPU paths.',
    );
  }

  const unsupportedGpuNames = gpuAssessments
    .filter((assessment) => assessment.tier === 'unsupported')
    .map((assessment) => assessment.name);
  const hasExplicitlyUnsupportedIntegratedGpu = gpuAssessments.some(
    (assessment) => !assessment.isLikelyDiscrete && assessment.tier === 'unsupported',
  );
  const hasOnlyUnconfirmedOrDiscreteProblemGpus = gpuAssessments.length > 0 && gpuAssessments.every((assessment) =>
    assessment.tier === 'unknown' || (assessment.isLikelyDiscrete && assessment.tier === 'unsupported'),
  );
  const canUseCommunityFallback =
    !profile.isVM &&
    hasCommunityEvidence &&
    profile.architecture === 'Intel' &&
    communityEvidence.highestReportedVersionNumeric != null &&
    targetVersion <= communityEvidence.highestReportedVersionNumeric &&
    !hasExplicitlyUnsupportedIntegratedGpu &&
    hasOnlyUnconfirmedOrDiscreteProblemGpus;
  const canUseLegacyIntelFallback =
    !profile.isVM &&
    !canUseCommunityFallback &&
    hasLegacyIntelPlanningValue(profile) &&
    !hasExplicitlyUnsupportedIntegratedGpu &&
    hasOnlyUnconfirmedOrDiscreteProblemGpus;

  if (!profile.isVM && !hasSupportedDisplayPath(gpuDevices)) {
    if (canUseCommunityFallback) {
      advisoryFallbackCeiling = communityEvidence.highestReportedVersionNumeric;
      applyAdvisoryLevel(
        report,
        'risky',
        `The scan did not confirm a supported display path, but similar ${profile.isLaptop ? `${profile.generation} laptops` : `${profile.generation} desktops`} do have documented community builds.${buildCommunityNote(communityEvidence)} Planning can stay open as a risky path while you verify the real display route yourself.`,
        'The scan could not confirm the active display path. Verify the real GPU and output route before you trust this build.',
      );
    } else if (canUseLegacyIntelFallback) {
      advisoryFallbackCeiling = cpuCeiling ?? 12;
      applyAdvisoryLevel(
        report,
        'risky',
        `The scan did not confirm a supported display path, but this older Intel system is still worth planning for manual community-style builds. The app keeps the path open as risky and caps it to older macOS targets while you verify the real display route yourself.${buildCommunityNote(report.communityEvidence)}`,
        'The scan could not confirm the active display path. This older Intel system stays open for risky planning only, with a conservative macOS ceiling.',
      );
    } else {
      if (unsupportedGpuNames.length > 0) {
        report.errors.push(`No supported display path remains. Unsupported GPU(s): ${unsupportedGpuNames.join(', ')}.`);
      } else {
        report.errors.push('No supported display path remains. The detected GPU path is unknown or unsupported for the selected target.');
      }
      return setBlocked(report, 'OpenCore build blocked because no supported display path remains.');
    }
  }

  if (profile.isLaptop && hasUnsupportedDiscreteGpu(gpuDevices)) {
    applyAdvisoryLevel(
      report,
      report.level === 'risky' ? 'risky' : 'experimental',
      'Laptop with an unsupported discrete GPU detected. A supported iGPU path may still boot macOS, but only as an experimental route with the dGPU disabled or ignored.',
      'Laptop discrete GPU must be disabled unless you have confirmed the internal panel and required outputs stay on a supported path.',
    );
  } else if (!profile.isLaptop && hasUnsupportedDiscreteGpu(gpuDevices)) {
    applyAdvisoryLevel(
      report,
      'experimental',
      'Unsupported discrete GPU detected. macOS can still work if the display is routed through a supported iGPU or AMD GPU, but that keeps this path experimental.',
      'Unsupported discrete GPU detected. Make sure the monitor is connected to a supported iGPU or supported AMD GPU before booting macOS.',
    );
  }

  for (const assessment of gpuAssessments) {
    for (const note of assessment.notes) {
      if (assessment.tier !== 'unsupported') {
        pushUnique(report.warnings, note);
      }
    }
  }

  if (bestAnyDisplayPath?.tier === 'supported_with_limit') {
    const olderPathLevel = ['Penryn', 'Sandy Bridge', 'Ivy Bridge'].includes(profile.generation) ? 'risky' : 'experimental';
    applyAdvisoryLevel(
      report,
      olderPathLevel,
      `Older GPU-limited macOS path detected through ${bestAnyDisplayPath.name}. This remains viable, but usually on older macOS targets and with more manual tuning than a modern canonical build.${buildCommunityNote(report.communityEvidence)}`,
    );
  }

  if (bestAnyDisplayPath?.tier === 'partial_support') {
    applyAdvisoryLevel(
      report,
      'risky',
      `Lower-confidence GPU path detected through ${bestAnyDisplayPath.name}. EFI generation and simulation remain available, but expect manual fixes and likely boot issues.`,
    );
  }

  if (bestAnyDisplayPath?.requiresNootRX) {
    applyAdvisoryLevel(
      report,
      'risky',
      'Detected AMD Navi 22 GPU requires NootRX. This is a community-proven but non-native graphics path, so it remains risky.',
      'Detected AMD Navi 22 GPU requires NootRX and is not a native WhateverGreen path.',
    );
  }

  if (bestAnyDisplayPath?.requiresNootedRed) {
    applyAdvisoryLevel(
      report,
      'risky',
      'Detected AMD Vega APU path requires NootedRed. This can be made to work, but it stays a risky manual-tuning path rather than a normal supported route.',
      'Detected AMD Vega APU path requires NootedRed and remains lower-confidence than native Intel or AMD dGPU paths.',
    );
  }

  if (isOlderIntelLaptop(profile) && (bestAnyDisplayPath || advisoryFallbackCeiling != null)) {
    const level = ['Penryn', 'Sandy Bridge', 'Ivy Bridge'].includes(profile.generation) ? 'risky' : 'experimental';
    applyAdvisoryLevel(
      report,
      level,
      `Older Intel laptop path detected. These machines are still used successfully in the community, but usually with older macOS versions, conservative SMBIOS choices, and extra manual patching.${buildCommunityNote(report.communityEvidence)}`,
      'Older laptop path: expect older macOS ceilings, manual patches, and lower confidence than a modern canonical build.',
    );
  }

  if (motherboard.includes('pm981') || motherboard.includes('pm991') || motherboard.includes('2200s')) {
    applyAdvisoryLevel(
      report,
      report.level === 'risky' ? 'risky' : 'experimental',
      'Known-problem NVMe path detected. Planning can continue, but storage stability stays below canonical expectations.',
      'Known-problem NVMe drive detected. NVMeFix helps, but this storage path is still lower-confidence.',
    );
  }

  if (motherboard.includes('600p')) {
    applyAdvisoryLevel(
      report,
      report.level === 'risky' ? 'risky' : 'experimental',
      'Intel 600p NVMe detected. macOS can still work, but this storage path is known for boot instability and should be treated as experimental.',
      'Intel 600p NVMe is known to cause boot instability in macOS.',
    );
  }

  let maxVersion = 26;
  if (cpuCeiling != null) maxVersion = Math.min(maxVersion, cpuCeiling);
  if (bestDisplayCeiling != null) maxVersion = Math.min(maxVersion, bestDisplayCeiling);
  if (advisoryFallbackCeiling != null) maxVersion = Math.min(maxVersion, advisoryFallbackCeiling);

  const eligibleVersions = getEligibleMacOSVersions(maxVersion);
  report.eligibleVersions = eligibleVersions;
  report.maxOSVersion = eligibleVersions[0]?.name ?? 'Blocked';

  const recommendedCeiling = report.communityEvidence
    && report.level !== 'supported'
    && report.communityEvidence.signal !== 'weak'
    && report.communityEvidence.highestReportedVersionNumeric != null
    ? Math.min(maxVersion, report.communityEvidence.highestReportedVersionNumeric)
    : maxVersion;

  const recommendedVersions = getEligibleMacOSVersions(recommendedCeiling);
  report.recommendedVersion = recommendedVersions[0]?.name ?? eligibleVersions[0]?.name ?? '';

  if (
    report.communityEvidence
    && report.level !== 'supported'
    && report.communityEvidence.highestReportedVersionNumeric != null
    && report.communityEvidence.highestReportedVersionNumeric < maxVersion
  ) {
    pushUnique(
      report.warnings,
      `Community-proven ceiling for similar hardware is strongest around ${report.communityEvidence.highestReportedVersion}. Newer eligible versions may still require extra manual fixes.`,
    );
  }

  const selectedVersionCoveredByCommunityFallback =
    advisoryFallbackCeiling != null && targetVersion <= advisoryFallbackCeiling;

  if (!profile.isVM && !bestSelectedDisplayPath && !selectedVersionCoveredByCommunityFallback) {
    const highestSupported = eligibleVersions[0]?.name ?? 'an older supported version';
    const communityNote = report.communityEvidence?.highestReportedVersion
      && report.communityEvidence.highestReportedVersion !== highestSupported
      ? ` Similar community reports are strongest around ${report.communityEvidence.highestReportedVersion}.`
      : '';
    report.level = 'blocked';
    report.strategy = 'blocked';
    report.isCompatible = false;
    report.errors.push(`Selected target ${profile.targetOS} exceeds the supported GPU or display-path ceiling. Choose ${highestSupported} or older.${communityNote}`);
    report.explanation = `The current target macOS version is above the supported GPU/display-path ceiling. Select ${highestSupported} or older.${communityNote}`;
  }

  if (report.level === 'supported') {
    if (
      profile.architecture === 'Intel'
      && ['Coffee Lake', 'Comet Lake', 'Rocket Lake'].includes(profile.generation)
      && bestAnyDisplayPath
    ) {
      report.explanation = `Intel ${profile.generation} system with a supported display path. This is a solid OpenCore starting point.`;
    } else if (profile.architecture === 'AMD' && !profile.isLaptop && bestAnyDisplayPath) {
      report.explanation = 'AMD desktop with a supported display path. This is a solid OpenCore starting point.';
    } else if (bestAnyDisplayPath) {
      report.explanation = `Supported display path detected through ${bestAnyDisplayPath.name}. This stays within the normal OpenCore path.`;
    }
  }

  if (report.errors.length > 0) {
    report.isCompatible = false;
    report.level = 'blocked';
    report.strategy = 'blocked';
  }

  report.nextActions = buildCompatibilityNextActions(
    profile,
    report,
    targetVersion,
    bestAnyDisplayPath?.name ?? null,
    bestSelectedDisplayPath?.name ?? null,
  );
  report.advisoryConfidence = buildCompatibilityAdvisoryConfidence(profile, report, targetVersion);
  report.mostLikelyFailurePoints = buildMostLikelyFailurePoints(
    profile,
    report,
    targetVersion,
    bestAnyDisplayPath?.name ?? null,
    bestSelectedDisplayPath?.name ?? null,
  );

  return report;
}
