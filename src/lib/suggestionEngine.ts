// ── Auto-Fix Suggestion Engine v4 ───────────────────────────────────────────
// Decision-closure system with recommendation signals and escalation logic.
// Every suggestion is explainable, decisive, and confidence-guiding.
// Advisory-only: NEVER bypasses validation, NEVER triggers actions
// automatically, NEVER alters files silently.

import type { HardwareProfile } from '../../electron/configGenerator';
import type { ValidationIssue, ValidationTrace } from '../../electron/configValidator';
import {
  getBestSupportedGpuPath,
  getProfileGpuDevices,
  hasSupportedDisplayPath,
  hasUnsupportedModernNvidia,
  parseMacOSVersion,
} from '../../electron/hackintoshRules.js';
import { normalizeErrorMessage } from './errorMessage.js';
import { structureError } from './structuredErrors';

// ─── Types ──────────────────────────────────────────────────────────────────

export type SuggestionCategory =
  | 'environment_error'
  | 'hardware_error'
  | 'network_error'
  | 'permission_error'
  | 'validation_error'
  | 'timeout_error'
  | 'device_error';

export type ActionConfidence = 'high' | 'medium' | 'low';
export type ActionGroup = 'fix_now' | 'try_alternative' | 'learn_more';

export interface SuggestedAction {
  text: string;
  confidence: ActionConfidence;
  confidenceReason: string;
  group: ActionGroup;
  reason: string;
  expectedOutcome: string;
  risk?: string;
  recommended: boolean;
}

export interface Suggestion {
  code: string;
  category: SuggestionCategory;
  title: string;
  explanation: string;
  decisionSummary: string;
  primaryAction: SuggestedAction;
  alternatives: SuggestedAction[];
  contextNote?: string;
  severity: 'critical' | 'actionable' | 'informational';
}

export interface SuggestionContext {
  profile?: HardwareProfile | null;
  platform?: string;
  errorMessage: string;
  step?: string;
  retryCount?: number;
  diskInfo?: { partitionTable?: string; isSystemDisk?: boolean } | null;
  validationIssues?: ValidationIssue[];
  validationTrace?: ValidationTrace | null;
  kextSources?: Record<string, 'github' | 'embedded' | 'direct' | 'failed'>;
}

// ─── Helper ─────────────────────────────────────────────────────────────────

function act(
  text: string,
  confidence: ActionConfidence,
  confidenceReason: string,
  group: ActionGroup,
  reason: string,
  expectedOutcome: string,
  risk?: string,
): SuggestedAction {
  // recommended defaults to false — set by the recommendation logic after build
  return { text, confidence, confidenceReason, group, reason, expectedOutcome, recommended: false, ...(risk ? { risk } : {}) };
}

// ─── Escalation thresholds ──────────────────────────────────────────────────
// retryCount 0 → first failure → recommend primary (usually retry)
// retryCount 1 → second failure → recommend primary if it changed, else alt
// retryCount 2+ → third+ failure → recommend fallback/manual path

function getEscalationTier(retryCount: number): 'first' | 'second' | 'escalated' {
  if (retryCount <= 0) return 'first';
  if (retryCount === 1) return 'second';
  return 'escalated';
}

// ─── Templates ──────────────────────────────────────────────────────────────

interface SuggestionTemplate {
  test: (msg: string) => boolean;
  code: string;
  category: SuggestionCategory;
  build: (ctx: SuggestionContext) => Omit<Suggestion, 'code' | 'category'>;
}

const TEMPLATES: SuggestionTemplate[] = [
  // ── Recovery auth rejection (401/403) ─────────────────────────
  {
    test: m => (m.includes('401') || m.includes('403')) && (m.includes('apple') || m.includes('recovery')),
    code: 'recovery_auth_rejected',
    category: 'network_error',
    build: (ctx) => {
      const tier = getEscalationTier(ctx.retryCount ?? 0);
      if (tier === 'escalated') {
        return {
          title: 'Apple keeps rejecting recovery requests',
          explanation: 'Multiple serial rotations have failed. Apple\'s servers are consistently rejecting this macOS version for generated serials.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Switch to EFI-only mode — bypasses Apple\'s recovery servers entirely',
            'high',
            'This completely avoids Apple\'s serial validation system',
            'try_alternative',
            'Three retry attempts have failed — continued retries have diminishing returns',
            'You get a working bootloader without needing Apple\'s recovery servers',
            'You will need to create the macOS installer USB separately on a real Mac',
          ),
          alternatives: [
            act(
              'Select a different macOS version — Monterey and Big Sur have higher success rates',
              'high',
              'Older macOS versions use less strict server-side serial validation',
              'try_alternative',
              'Newer versions are more aggressively filtered by Apple\'s servers',
              'Monterey and Big Sur have the highest recovery download success rate',
            ),
            act(
              'Use Dortania\'s macrecovery Python script for manual download',
              'medium',
              'The manual tool uses different request patterns that may succeed',
              'learn_more',
              'The manual tool gives you more control over serial and board ID',
              'You download the recovery image outside this app',
            ),
          ],
        };
      }
      if (tier === 'second') {
        return {
          title: 'Apple rejected the recovery request again',
          explanation: 'A second serial rotation was also rejected. This macOS version may have stricter validation.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Select a different macOS version — older versions have higher success rates',
            'high',
            'Older macOS versions (Monterey, Big Sur) use less strict server-side serial validation',
            'try_alternative',
            'Two retry attempts with different serials failed — the version itself is likely the issue',
            'Higher chance of successful recovery download with an older target version',
          ),
          alternatives: [
            act(
              'Skip recovery download — use EFI-only method',
              'high',
              'This bypasses Apple\'s recovery servers entirely',
              'try_alternative',
              'You do not need the recovery image if you create a USB installer on a real Mac',
              'You get a working EFI; you create the macOS installer separately',
              'Requires access to a Mac or existing macOS install to create the USB installer',
            ),
            act(
              'Try one more serial rotation — there is still a chance it passes',
              'low',
              'Two failures make a third retry unlikely to succeed for the same version',
              'fix_now',
              'The app generates a fresh serial each attempt, but the version may be blocked',
              'Small chance the next serial passes Apple\'s validation',
            ),
            act(
              'Check Dortania\'s macrecovery guide for manual download instructions',
              'medium',
              'Dortania\'s Python script uses different request patterns that may succeed',
              'learn_more',
              'The manual tool gives you more control over the serial and board ID used',
              'You can download the recovery image outside this app and place it in the EFI folder',
            ),
          ],
        };
      }
      // first attempt
      return {
        title: 'Apple rejected the recovery request',
        explanation: 'Apple\'s servers rejected the serial/board ID used for this macOS version. This is a server-side policy — not a network issue.',
        severity: 'actionable',
        decisionSummary: '',
        primaryAction: act(
          'Retry with a new serial rotation',
          'medium',
          'The app generates a fresh serial/board ID pair each attempt, but Apple may reject multiple serials for the same version',
          'fix_now',
          'Apple rejects specific serial numbers, not your machine — a different serial may pass',
          'The download may succeed on the next attempt with a different generated serial',
        ),
        alternatives: [
          act(
            'Select a different macOS version — older versions have higher success rates',
            'high',
            'Older macOS versions use less strict server-side serial validation',
            'try_alternative',
            'Newer versions are more aggressively filtered by Apple\'s servers',
            'Monterey and Big Sur have the highest recovery download success rate',
          ),
          act(
            'Skip recovery download — use EFI-only method',
            'high',
            'This bypasses Apple\'s recovery servers entirely',
            'try_alternative',
            'You do not need the recovery image if you create a USB installer on a real Mac',
            'You get a working EFI; you create the macOS installer separately',
            'Requires access to a Mac or existing macOS install to create the USB installer',
          ),
          act(
            'Check Dortania\'s macrecovery guide for manual download instructions',
            'medium',
            'Dortania\'s Python script uses different request patterns that may succeed',
            'learn_more',
            'The manual tool gives you more control over the serial and board ID used',
            'You can download the recovery image outside this app and place it in the EFI folder',
          ),
        ],
      };
    },
  },

  // ── Recovery auth rejection (classified message) ─────────────────────────
  {
    test: m => m.includes('apple recovery server rejected the request') || m.includes('apple rejected the recovery request'),
    code: 'recovery_auth_rejected',
    category: 'network_error',
    build: () => ({
      title: 'Apple rejected the recovery request',
      explanation: 'Apple’s recovery service refused the selected download request. This is a server-side policy limit, not a disk or renderer problem.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Try an older recommended macOS version or use manual recovery import',
        'high',
        'Older versions and manual import avoid the exact server-side rejection path that just failed',
        'fix_now',
        'The download request itself was rejected by Apple, so repeating the same request blindly is unlikely to help',
        'You either move to a version with a higher success rate or bypass Apple’s automated rejection path with a manual import',
      ),
      alternatives: [
        act(
          'Switch to EFI-only mode if you already have separate installer media',
          'high',
          'EFI generation can still continue without relying on Apple’s recovery service',
          'try_alternative',
          'The blocked piece is recovery acquisition, not EFI generation',
          'You keep a valid bootloader build and provide installer media separately',
        ),
        act(
          'Retry once later after changing macOS target selection',
          'low',
          'A plain retry against the same target is lower confidence than changing the recovery path',
          'learn_more',
          'The rejection came from Apple’s service, not a local transient write failure',
          'You may get a different outcome if you change the requested macOS path',
        ),
      ],
    }),
  },

  // ── Recovery download interrupted ─────────────────────────────
  {
    test: m => m.includes('recovery') && (m.includes('failed') || m.includes('error') || m.includes('interrupted')),
    code: 'recovery_download_failed',
    category: 'network_error',
    build: (ctx) => {
      const tier = getEscalationTier(ctx.retryCount ?? 0);
      if (tier === 'escalated') {
        return {
          title: 'Recovery download keeps failing',
          explanation: 'Multiple download attempts have failed. Your network environment is consistently interrupting the transfer.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Switch to EFI-only mode — skip the recovery download entirely',
            'high',
            'This eliminates the unreliable download step completely',
            'try_alternative',
            'Three download attempts failed — the network issue is persistent, not transient',
            'You get a working bootloader; create the macOS installer separately on a Mac',
            'Requires access to a Mac or existing macOS install for the USB installer',
          ),
          alternatives: [
            act(
              'Switch to a wired Ethernet connection and disable VPN/proxy',
              'medium',
              'Removes the two most common network interference sources simultaneously',
              'fix_now',
              'Wi-Fi drops and network middleboxes are the top causes of repeated download failures',
              'Stable connection for the ~500 MB download',
            ),
          ],
        };
      }
      if (tier === 'second') {
        return {
          title: 'Recovery download failed again',
          explanation: 'The download stream was broken a second time. This is likely a network environment issue rather than a transient blip.',
          severity: 'actionable',
          decisionSummary: '',
          primaryAction: act(
            'Switch to a wired Ethernet connection',
            'high',
            'Wired connections do not drop packets or renegotiate like Wi-Fi during large transfers',
            'fix_now',
            'Wi-Fi interruptions are the most common cause of repeated download failures',
            'Stable connection for the remainder of the ~500 MB download',
          ),
          alternatives: [
            act(
              'Disable VPN, proxy, or firewall that may be interrupting large downloads',
              'medium',
              'Some security software terminates long-lived HTTP connections after a timeout',
              'fix_now',
              'Enterprise firewalls and VPNs commonly interfere with multi-hundred-MB downloads',
              'Removing the network middlebox lets the download complete without artificial interruption',
            ),
            act(
              'Skip recovery and use EFI-only mode',
              'high',
              'This removes the need to download from Apple entirely',
              'try_alternative',
              'The EFI is already built and valid — recovery is optional if you have another way to create a macOS USB',
              'You get a working bootloader without the recovery image',
              'You will need to create the macOS installer USB on a real Mac or from an existing macOS install',
            ),
          ],
        };
      }
      // first attempt
      return {
        title: 'Recovery download interrupted',
        explanation: 'The download stream to Apple\'s CDN was broken. The partial file is preserved on disk.',
        severity: 'actionable',
        decisionSummary: '',
        primaryAction: act(
          'Resume download — the app continues from where it stopped',
          'high',
          'The partial BaseSystem.dmg file is on disk and the server supports HTTP Range requests',
          'fix_now',
          'Network interruptions during large downloads are common and usually transient',
          'Download resumes from the last byte received — no data is re-downloaded',
        ),
        alternatives: [
          act(
            'Skip recovery and use EFI-only mode',
            'high',
            'This removes the need to download from Apple entirely',
            'try_alternative',
            'The EFI is already built and valid — recovery is optional if you have another way to create a macOS USB',
            'You get a working bootloader without the recovery image',
            'You will need to create the macOS installer USB on a real Mac or from an existing macOS install',
          ),
        ],
      };
    },
  },

  // ── Pre-build blockers from deterministic/preflight checks ───────────────
  {
    test: m => m.includes('build_blocked_by_guard') || m.includes('efi build is blocked'),
    code: 'build_blocked_by_guard',
    category: 'validation_error',
    build: () => ({
      title: 'EFI build is blocked',
      explanation: 'The app stopped the EFI build before generation because a prerequisite is still unsatisfied.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Resolve the blocking prerequisite shown in the BIOS or report step, then retry the EFI build',
        'high',
        'The guard already identified the exact class of blocker, so retrying blindly is low value',
        'fix_now',
        'Build guard failures are deterministic gate checks, not transient renderer noise',
        'The next build starts only after the prerequisite is satisfied',
      ),
      alternatives: [],
    }),
  },
  {
    test: m => m.includes('build_ipc_failed') || m.includes('efi build failed'),
    code: 'build_ipc_failed',
    category: 'validation_error',
    build: () => ({
      title: 'EFI build failed',
      explanation: 'The backend build flow returned a concrete runtime failure while generating the EFI.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Review the concrete build error, fix it, then retry the EFI build once',
        'high',
        'This is a real backend build failure, not a safe retry candidate without first understanding the blocker',
        'fix_now',
        'Repeating the same failed build without addressing the reported cause is low signal',
        'The next build attempt starts from a corrected state',
      ),
      alternatives: [],
    }),
  },
  {
    test: m => m.includes('pre-build check failed') || m.includes('build will fail'),
    code: 'build_precheck_failed',
    category: 'validation_error',
    build: () => ({
      title: 'Build is blocked before generation',
      explanation: 'A concrete dependency, environment, or download blocker was found before the EFI build could succeed.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Fix the blocker named in the report before rebuilding',
        'high',
        'The app already found a specific failure point, so repeating the same build without fixing it is low-value',
        'fix_now',
        'Preflight and deterministic checks are meant to stop a known-bad build before time is wasted',
        'The next build starts from a materially better state instead of repeating the same failure',
      ),
      alternatives: [
        act(
          'Copy the report and inspect the blocker details before changing anything else',
          'medium',
          'The diagnostics already contain the concrete blocker that stopped the build',
          'learn_more',
          'This avoids random fixes when the failing dependency is already known',
          'You can address the exact missing tool, blocked download, or environment issue',
        ),
      ],
    }),
  },

  // ── EFI on-disk verification failure ──────────────────────────────────────
  {
    test: m => m.includes('efi build contract failed'),
    code: 'efi_build_contract_failed',
    category: 'validation_error',
    build: () => ({
      title: 'Generated EFI failed on-disk verification',
      explanation: 'The EFI was generated, but at least one required component did not land on disk correctly.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Inspect the failed component named in the report, then rebuild once the dependency issue is fixed',
        'high',
        'The contract failure means the app already knows a concrete component did not verify',
        'fix_now',
        'This is an integrity failure after build generation, not a vague UI error',
        'The rebuilt EFI passes the on-disk contract and can move to validation cleanly',
      ),
      alternatives: [
        act(
          'Copy the report and open the EFI folder to confirm the missing file or empty kext directory',
          'medium',
          'Manual inspection is useful when the same post-build verification failure repeats',
          'learn_more',
          'The contract output points to the exact failed check',
          'You can verify whether the problem is a missing file, empty directory, or broken download',
        ),
      ],
    }),
  },

  // ── Kext fetch failure ────────────────────────────────────────
  {
    test: m => m.includes('failed to fetch') || (m.includes('kext') && m.includes('fail')),
    code: 'kext_fetch_failed',
    category: 'network_error',
    build: (ctx) => ({
      title: 'Kext download failed',
      explanation: 'One or more required kernel extensions could not be downloaded from GitHub. This usually means a transient network issue or a GitHub rate limit.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Retry the build — transient network errors usually resolve on the second attempt',
        'high',
        'Most kext fetch failures are caused by temporary connection drops',
        'fix_now',
        'The build process will re-attempt all missing downloads',
        'A complete EFI structure with all required drivers',
      ),
      alternatives: [
        act(
          'Wait 5 minutes for GitHub API quotas to reset',
          'medium',
          'Programmatic downloads are subject to rate limiting',
          'try_alternative',
          'If you have made multiple rapid build attempts, GitHub may be throttling your IP',
          'Success on the next attempt after the cool-down period',
        ),
        act(
          'Identify missing components in diagnostics and download them manually',
          'medium',
          'Manual downloads via browser often bypass programmatic API limits',
          'learn_more',
          'If a specific kext keeps failing, you can place it in the EFI folder manually',
          'A fully functional EFI with manually-placed drivers',
        ),
      ],
    }),
  },

  // ── EFI validation failure ────────────────────────────────────
  {
    test: m => m.includes('efi') && (m.includes('validation') || m.includes('blocked') || m.includes('incomplete')),
    code: 'efi_validation_fail',
    category: 'validation_error',
    build: (ctx) => {
      const tier = getEscalationTier(ctx.retryCount ?? 0);
      const isConservative = ctx.profile?.strategy === 'conservative';

      if (tier === 'escalated') {
        return {
          title: 'EFI build keeps failing validation',
          explanation: 'Multiple rebuild attempts have failed. The issue is likely a persistent kext download failure or a config logic problem.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Open the EFI folder and manually identify which kext directories are empty or missing',
            'high',
            'After multiple failed rebuilds, manual inspection directly identifies the broken component',
            'fix_now',
            'Automated rebuilds have failed repeatedly — the specific failing component must be identified',
            'You find the exact missing file and can download it manually from its GitHub Releases page',
          ),
          alternatives: [
            act(
              'Wait 15 minutes for GitHub rate limits to fully reset, then rebuild',
              'medium',
              'If the failure was rate-limit-related, a longer wait ensures a full quota reset',
              'try_alternative',
              'Multiple rapid rebuilds can exhaust the rate limit faster than it resets',
              'All kext downloads succeed after a complete rate limit reset',
            ),
            ...(isConservative ? [act(
              'Switch to canonical strategy — uses fewer kexts and reduces download failure points',
              'medium',
              'Conservative mode adds optional kexts that may not have GitHub release assets',
              'try_alternative',
              'Fewer downloads means fewer chances for a kext fetch to fail',
              'A leaner EFI that passes validation with fewer external dependencies',
              'Canonical mode may not include safety quirks needed for edge-case hardware',
            )] : []),
          ],
        };
      }
      if (tier === 'second') {
        return {
          title: 'EFI build failed validation again',
          explanation: 'A second rebuild also failed. This is likely a GitHub rate limit or a consistently unavailable kext release.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Wait 5 minutes for GitHub rate limits to reset, then rebuild',
            'high',
            'GitHub rate limits reset on a per-hour rolling window — 5 minutes is usually sufficient',
            'fix_now',
            'Two consecutive failures strongly suggest GitHub rate limiting rather than a transient issue',
            'After the reset, all kext downloads succeed',
          ),
          alternatives: [
            act(
              'Open the EFI folder and check which kext directories are empty',
              'medium',
              'Empty kext directories directly indicate which GitHub download failed',
              'learn_more',
              'This lets you identify the specific failed component instead of rebuilding blindly',
              'You can manually download the missing kext from its GitHub Releases page',
            ),
          ],
        };
      }
      // first attempt
      return {
        title: 'EFI build failed integrity check',
        explanation: 'Required OpenCore files are missing or corrupted. This typically means a kext download was interrupted or GitHub rate-limited the request.',
        severity: 'critical',
        decisionSummary: '',
        primaryAction: act(
          'Go back and rebuild the EFI — all components will be re-downloaded',
          'high',
          'Most EFI validation failures are caused by a single interrupted kext download, which succeeds on retry',
          'fix_now',
          'The build process re-fetches every kext from GitHub — a transient failure on one kext does not persist',
          'A fresh build with a stable connection produces a valid, complete EFI',
        ),
        alternatives: [
          act(
            'Open the EFI folder and check which kext directories are empty',
            'medium',
            'Empty kext directories directly indicate which GitHub download failed',
            'learn_more',
            'This lets you identify the specific failed component instead of rebuilding blindly',
            'You can manually download the missing kext from its GitHub Releases page',
          ),
        ],
      };
    },
  },

  // ── Disk identity collision — must precede generic flash matcher ───
  {
    test: m => m.includes('disk identity collision') || m.includes('identity_collision'),
    code: 'disk_identity_collision',
    category: 'device_error',
    build: (ctx) => {
      return {
        title: 'Disk identity collision — multiple drives look identical',
        explanation: 'The app detected two or more connected drives with similar identity fields (serial number, model, size). To prevent writing to the wrong drive, flashing is blocked.',
        severity: 'critical',
        decisionSummary: 'Disconnect all USB drives except the one you want to flash, then retry.',
        primaryAction: act(
          'Disconnect duplicate USB drives and retry',
          'high',
          'Removing the ambiguity lets the app confirm which physical drive you selected',
          'fix_now',
          'Identical USB drives from the same batch often share serial numbers',
          'With only one matching drive connected, the identity check passes cleanly',
        ),
        alternatives: [
          act(
            'Unplug all USB drives, reconnect only the target, and re-select it',
            'high',
            'A fresh device enumeration gives the app a clean identity baseline',
            'fix_now',
            'Windows may cache stale device entries if drives are swapped quickly',
            'The target drive gets a unique identity context',
          ),
        ],
      };
    },
  },

  // ── Flash prepare blocked by BIOS readiness ───────────────────
  {
    test: m => m.includes('bios readiness is no longer satisfied'),
    code: 'bios_readiness_blocked',
    category: 'validation_error',
    build: () => ({
      title: 'Flashing is blocked by BIOS readiness',
      explanation: 'The app rechecked the firmware prerequisites right before the destructive flash step, and they are no longer satisfied.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Return to the BIOS step and run Recheck BIOS',
        'high',
        'The flash never started — the firmware gate blocked it first',
        'fix_now',
        'Retrying the USB step will not help until the firmware checklist is valid again',
        'The BIOS state is refreshed and flashing can be prepared safely again',
      ),
      alternatives: [
        act(
          'Go back to the report step if the target compatibility also changed',
          'medium',
          'A BIOS block can coexist with a compatibility block after the target or hardware path changes',
          'try_alternative',
          'If the machine is no longer bootable for the selected macOS target, the BIOS step alone is not the only blocker',
          'You return to the step that shows the current machine-level blocker clearly',
        ),
      ],
    }),
  },

  // ── Flash prepare blocked by compatibility / display path ────
  {
    test: m => m.includes('compatibility is blocked') || m.includes('no supported display path'),
    code: 'compatibility_blocked',
    category: 'validation_error',
    build: () => ({
      title: 'Selected macOS target is no longer deployable',
      explanation: 'Flashing was stopped before any USB write because the current machine no longer has a supported deployment path for the selected macOS target.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Return to the report step and fix the compatibility blocker',
        'high',
        'Retrying the USB step will not make an unsupported target bootable',
        'fix_now',
        'The blocker is in compatibility, not USB write access',
        'You end up back on a supported target or hardware path before flashing again',
      ),
      alternatives: [],
    }),
  },

  // ── Flash prepare blocked by missing selected disk ────────────
  {
    test: m => m.includes('no target disk is selected for flashing') || m.includes('target disk') && m.includes('no longer available'),
    code: 'selected_disk_missing',
    category: 'device_error',
    build: () => ({
      title: 'Target drive is no longer selected',
      explanation: 'The flash confirmation step was reached without a currently selected, readable target drive.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Return to USB selection, refresh the drive list, and select the drive again',
        'high',
        'The destructive flash step cannot continue without a live selected device',
        'fix_now',
        'This is a missing-device precondition failure, not a write failure',
        'The app regains a current disk handle before you reopen the flash confirmation dialog',
      ),
      alternatives: [],
    }),
  },

  // ── Flash prepare blocked by missing disk identity ────────────
  {
    test: m => m.includes('disk identity could not be confirmed') || m.includes('no disk identity fingerprint was captured'),
    code: 'disk_identity_missing',
    category: 'validation_error',
    build: () => ({
      title: 'Target drive identity could not be confirmed',
      explanation: 'The app stopped at the destructive flash boundary because it could not prove which physical drive is selected.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Reconnect the drive, re-select it, and wait for its details to load',
        'high',
        'Drive identity must be confirmed again immediately before a destructive write',
        'fix_now',
        'This safety check prevents the app from guessing which physical drive you meant',
        'The drive gets a fresh identity snapshot before the flash confirmation dialog opens again',
      ),
      alternatives: [],
    }),
  },

  // ── Drive letter assignment failure — issue #30 ───────────────
  {
    test: m => m.includes('did not assign a drive letter') || m.includes('add-partitionaccesspath'),
    code: 'drive_letter_assignment_failed',
    category: 'device_error',
    build: (ctx) => {
      return {
        title: 'Windows refused to assign a drive letter to the OPENCORE partition',
        explanation: 'The FAT32 OPENCORE partition was created successfully, but Windows would not assign a drive letter despite multiple attempts (diskpart, Set-Partition, Add-PartitionAccessPath). Another process may be holding a lock on the volume.',
        severity: 'critical',
        decisionSummary: 'Unplug the drive, wait 10 seconds, reconnect it, and try again.',
        primaryAction: act(
          'Unplug, wait 10 seconds, reconnect, and retry',
          'high',
          'A fresh USB reconnect clears stale volume locks and mount entries',
          'fix_now',
          'Windows volume manager sometimes holds a lock on newly created partitions',
          'The drive gets a clean enumeration and auto-letter assignment on reconnect',
        ),
        alternatives: [
          act(
            'Open Disk Management (diskmgmt.msc) and manually assign a drive letter',
            'medium',
            'Disk Management bypasses the programmatic letter-assignment path',
            'try_alternative',
            'Some antivirus or backup tools block programmatic letter assignment',
            'Once a letter is assigned, the flash can proceed',
          ),
        ],
      };
    },
  },

  // ── Flash / write error ───────────────────────────────────────
  {
    test: m => (m.includes('flash') || m.includes('write') || m.includes('dd:')) && (m.includes('fail') || m.includes('error')),
    code: 'flash_write_error',
    category: 'device_error',
    build: (ctx) => {
      const tier = getEscalationTier(ctx.retryCount ?? 0);
      const msg = normalizeErrorMessage(ctx.errorMessage).toLowerCase();
      const looksLikePermissionError = msg.includes('permission denied') || msg.includes('eacces') || msg.includes('eperm') || msg.includes('administrator') || msg.includes('sudo');
      const looksLikeTimeoutError = msg.includes('timeout') || msg.includes('timed out');
      const looksLikeVerificationError = msg.includes('verification failed') || msg.includes('not found on usb after copy');
      const looksLikeMediaError = msg.includes('write-protect') || msg.includes('write protect') || msg.includes('i/o error') || msg.includes('input/output error') || msg.includes('media is write protected') || msg.includes('device rejected write');

      if (looksLikePermissionError) {
        return {
          title: 'USB write blocked by permissions',
          explanation: 'The app could not get the system access needed to write to the drive.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: ctx.platform === 'win32'
            ? act(
                'Close the app and run it as Administrator',
                'high',
                'Windows blocks raw disk writes without elevation',
                'fix_now',
                'Permission problems can look like generic USB write failures',
                'The flash can proceed with the correct privileges',
              )
            : act(
                'Run the app with elevated privileges',
                'high',
                'Raw disk writes need elevated access',
                'fix_now',
                'Permission problems can look like generic USB write failures',
                'The flash can proceed with the correct privileges',
              ),
          alternatives: [],
        };
      }

      if (looksLikeTimeoutError) {
        return {
          title: 'USB write preparation timed out',
          explanation: 'A long-running USB step did not finish in time. This does not prove the drive is bad.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Reconnect the drive and retry once',
            'high',
            'Transient USB controller stalls are common and often recover on a clean reconnect',
            'fix_now',
            'A timeout alone is not enough evidence to call the drive faulty',
            'The write path gets a clean device session',
          ),
          alternatives: [],
        };
      }

      if (looksLikeVerificationError) {
        return {
          title: 'USB write verification failed',
          explanation: 'The write step finished, but the app could not verify the expected files on the drive afterward.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            'Reconnect the drive, then retry the flash',
            'high',
            'Verification failures are often caused by remount timing or transient device state',
            'fix_now',
            'This is a post-write verification problem, not direct proof of media failure',
            'The app can verify the written files on a clean retry',
          ),
          alternatives: [],
        };
      }

      if (tier === 'escalated') {
        return {
          title: 'USB write keeps failing',
          explanation: looksLikeMediaError
            ? 'Multiple low-level write attempts have failed on this drive. The drive may be failing or write-protected.'
            : 'Multiple USB write attempts have failed, but the current errors do not prove the drive itself is faulty.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: act(
            looksLikeMediaError
              ? 'Try a different USB drive'
              : 'Try a different USB port or reconnect the drive before replacing it',
            'high',
            looksLikeMediaError
              ? 'Repeated low-level write errors are strong evidence of a media or controller problem'
              : 'These retries failed, but the messages still point to a transport or environment problem first',
            'try_alternative',
            looksLikeMediaError
              ? 'Repeated low-level write errors usually come from the drive or its controller'
              : 'A generic write failure can still be caused by USB transport, permissions, or remount timing',
            looksLikeMediaError
              ? 'A known-good drive removes the failing media from the equation'
              : 'A clean port or reconnect may resolve the issue without replacing the drive',
          ),
          alternatives: [
            ...(ctx.platform === 'win32' ? [act(
              'Also ensure you are running as Administrator',
              'high',
              'Windows blocks raw disk writes without UAC elevation',
              'fix_now',
              'Permission issues can look identical to hardware failures on Windows',
              'Correct privilege level allows the flash to proceed',
            )] : []),
          ],
        };
      }
      return {
        title: 'USB write operation failed',
        explanation: 'The write stream to the USB drive was interrupted. This is usually a device, connection, or permission issue — not a config problem.',
        severity: 'critical',
        decisionSummary: '',
        primaryAction: ctx.platform === 'win32'
          ? act(
              'Close the app and re-run as Administrator',
              'high',
              'Windows blocks raw disk writes without UAC elevation — this is the most common cause on Windows',
              'fix_now',
              'Windows requires Administrator privileges for direct disk access (bypassing the filesystem)',
              'The flash operation succeeds with the correct privilege level',
            )
          : act(
              'Use a different USB port — preferably USB 2.0 or a rear motherboard port',
              'high',
              'Front-panel USB ports and hubs have higher failure rates for sustained writes due to power delivery limits',
              'fix_now',
              'USB 3.0 hubs and front ports sometimes drop connections during large sequential writes',
              'A direct motherboard connection provides stable power and signal integrity',
            ),
        alternatives: [
          act(
            'Replace the USB drive',
            'high',
            'Counterfeit or worn flash drives fail silently at the block level — this is undetectable before writing',
            'try_alternative',
            'Faulty drives pass size checks but fail during sustained writes',
            'A known-good drive from a reputable brand eliminates the hardware variable',
          ),
          act(
            'Check for a physical write-protect switch on the drive',
            'medium',
            'Some SD-to-USB adapters and older drives have a mechanical lock switch',
            'fix_now',
            'A locked switch causes every write to fail with a generic I/O error',
            'Unlocking the switch allows writes immediately — no reformat needed',
          ),
          ...(ctx.platform === 'linux' ? [act(
            'Ensure the drive is not mounted: run `umount /dev/sdX*` before retrying',
            'high',
            'Linux auto-mounts removable drives — a mounted partition blocks raw device writes',
            'fix_now',
            'The kernel rejects writes to a device with mounted partitions',
            'Unmounting releases the device for exclusive raw access',
          )] : []),
        ],
      };
    },
  },

  // ── Watchdog / stall ──────────────────────────────────────────
  {
    test: m => m.includes('stall') || m.includes('watchdog') || m.includes('no progress'),
    code: 'watchdog_trigger',
    category: 'timeout_error',
    build: () => ({
      title: 'Operation stalled',
      explanation: 'A background task stopped making progress for over 60 seconds. The process may be deadlocked.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Cancel the stalled operation',
        'high',
        'Cancellation is safe — your EFI build and downloaded files are preserved on disk',
        'fix_now',
        'The watchdog detected 60 seconds of zero progress, which indicates the task is stuck, not slow',
        'The operation stops cleanly and you can retry from the same point',
      ),
      alternatives: [
        act(
          'If stalled during kext download: GitHub may be rate-limiting — wait 5 minutes',
          'medium',
          'GitHub silently drops connections instead of returning 429 when close to the rate limit boundary',
          'try_alternative',
          'Stalls during HTTP downloads are commonly caused by the server throttling without an explicit error',
          'After the rate limit window resets, downloads proceed normally',
        ),
        act(
          'If stalled during USB flash: the drive may be failing',
          'medium',
          'Flash memory write stalls are a common symptom of worn or counterfeit drives',
          'try_alternative',
          'Healthy USB drives do not stall for 60+ seconds during sequential writes',
          'Replacing the drive resolves the stall if the drive is the cause',
        ),
      ],
    }),
  },

  // ── Hardware disconnect ───────────────────────────────────────
  {
    test: m => m.includes('disconnect') || (m.includes('not found') && !m.includes('binary')) || m.includes('device_not_found') || m.includes('DEVICE_NOT_FOUND'),
    code: 'hardware_disconnect',
    category: 'device_error',
    build: () => ({
      title: 'Target drive is no longer available',
      explanation: 'The drive was physically disconnected, ejected by the OS, or lost USB bus power.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Reconnect the drive to a different USB port, then refresh the drive list',
        'high',
        'Reconnecting re-enumerates the device on the USB bus — the app will detect it on refresh',
        'fix_now',
        'The OS lost the device node, which means the physical connection was interrupted',
        'The drive reappears in the list and can be re-selected',
      ),
      alternatives: [
        act(
          'Use a powered USB hub if the drive keeps disconnecting',
          'medium',
          'Some drives draw 500+ mA, exceeding what a single unpowered port provides',
          'try_alternative',
          'Repeated disconnects suggest insufficient bus power rather than a faulty drive',
          'Stable power delivery prevents mid-operation disconnects',
        ),
        act(
          'If disconnected mid-write, the partial write is corrupted — reflash from scratch',
          'high',
          'A partial GPT/FAT32 write leaves the partition table in an inconsistent state',
          'learn_more',
          'The flash process writes the partition table first, then files — an interruption corrupts the layout',
          'A full reflash rewrites the partition table and all files cleanly',
        ),
      ],
    }),
  },

  // ── Permission denied ─────────────────────────────────────────
  {
    test: m => m.includes('permission') || m.includes('eacces') || m.includes('eperm') || m.includes('administrator') || m.includes('sudo'),
    code: 'permission_denied',
    category: 'permission_error',
    build: (ctx) => ({
      title: 'Elevated privileges required',
      explanation: 'Disk operations require OS-level permission to access raw block devices.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: ctx.platform === 'win32'
        ? act(
            'Close the app → right-click the .exe → "Run as administrator" → retry',
            'high',
            'Windows requires UAC elevation for all direct disk I/O — this is deterministic, not intermittent',
            'fix_now',
            'The app attempted raw disk access, which Windows blocks without Administrator privileges',
            'The flash operation succeeds immediately with the correct privilege level',
          )
        : ctx.platform === 'darwin'
          ? act(
              'Approve the system password dialog when macOS prompts for disk access',
              'high',
              'macOS grants per-operation disk access via system prompts — no persistent root session needed',
              'fix_now',
              'macOS uses a just-in-time privilege model for disk operations',
              'The operation proceeds after you authenticate in the system dialog',
            )
          : act(
              'Install polkit: sudo apt install policykit-1 — the app will prompt for your password when disk access is needed',
              'high',
              'Linux uses per-operation elevation via pkexec — do not run the entire app as root',
              'fix_now',
              'pkexec provides a graphical password prompt for individual disk commands',
              'Disk operations proceed after you authenticate in the pkexec dialog',
            ),
      alternatives: [],
    }),
  },

  // ── Operation timeout ─────────────────────────────────────────
  {
    test: m => m.includes('timed out') || m.includes('timeout'),
    code: 'operation_timeout',
    category: 'timeout_error',
    build: (ctx) => {
      const isDuringDownload = ctx.step === 'recovery-download' || ctx.step === 'kext-fetch';
      return {
        title: isDuringDownload ? 'Download timed out' : 'Operation timed out',
        explanation: isDuringDownload
          ? 'The remote server did not respond within the expected time. This is usually a network issue.'
          : 'An internal operation exceeded its time limit and was stopped to prevent the app from freezing.',
        severity: 'actionable',
        decisionSummary: '',
        primaryAction: isDuringDownload
          ? act(
              'Verify your internet connection is active, then retry',
              'high',
              'Downloads resume from the last checkpoint — no data is lost on timeout',
              'fix_now',
              'The server stopped responding, which indicates a network path issue rather than a server-side problem',
              'The download resumes from the last received byte',
            )
          : act(
              'Restart the app and retry the operation',
              'medium',
              'Local operation timeouts are rare and may indicate a one-time system resource issue',
              'fix_now',
              'The operation exceeded the safety timeout, which prevents the app from hanging indefinitely',
              'A fresh app process resets all internal state and retry counters',
            ),
        alternatives: isDuringDownload
          ? [act(
              'If behind a corporate firewall, ensure HTTPS to github.com and osrecovery.apple.com is allowed',
              'medium',
              'Corporate firewalls commonly block or throttle unfamiliar HTTPS destinations',
              'learn_more',
              'The download targets Apple and GitHub CDNs, which may be restricted on managed networks',
              'Whitelisting these domains allows downloads to complete at full speed',
            )]
          : [act(
              'Copy diagnostics and report the issue — local operation timeouts may indicate a bug',
              'medium',
              'The diagnostics include the exact operation, timeout duration, and system state at failure time',
              'learn_more',
              'Local timeouts should not happen under normal conditions',
              'A bug report with diagnostics helps identify and fix the root cause',
            )],
      };
    },
  },

  // ── System disk blocked ───────────────────────────────────────
  {
    test: m => m.includes('system disk') || m.includes('system/boot') || m.includes('safety block') || m.includes('system_disk'),
    code: 'system_disk_blocked',
    category: 'device_error',
    build: () => ({
      title: 'System disk cannot be used',
      explanation: 'This drive contains your operating system. The app blocks it to prevent data loss.',
      severity: 'critical',
      decisionSummary: '',
      primaryAction: act(
        'Select a removable USB drive from the list',
        'high',
        'The system disk is detected by OS-level queries and will always be blocked — this is not a false positive',
        'fix_now',
        'The app identified this drive as the boot disk where your OS is installed',
        'Selecting a removable drive allows the flash to proceed safely',
      ),
      alternatives: [
        act(
          'If no USB drives appear, insert a 16 GB+ drive and click Refresh',
          'high',
          'The app only lists drives confirmed as removable — an absent drive simply needs to be plugged in',
          'fix_now',
          'The drive list is empty because no removable drives are connected',
          'The inserted drive appears in the list after Refresh',
        ),
      ],
    }),
  },

  // ── MBR partition table ───────────────────────────────────────
  {
    test: m => m.includes('mbr') || m.includes('mbr_partition_table'),
    code: 'mbr_partition',
    category: 'device_error',
    build: (ctx) => ({
      title: 'Incompatible partition table (MBR)',
      explanation: 'OpenCore requires GPT. This drive uses the older MBR format.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Use a different USB drive that already has GPT',
        'high',
        'Most drives manufactured after 2015 ship with GPT by default — switching drives avoids reformatting',
        'try_alternative',
        'This drive uses MBR, which OpenCore cannot boot from',
        'A GPT-formatted drive is immediately usable without any conversion step',
      ),
      alternatives: [
        ctx.platform === 'win32'
          ? act(
              'Convert to GPT: Command Prompt (admin) → diskpart → select disk N → clean → convert gpt',
              'medium',
              'diskpart reliably converts MBR to GPT, but the `clean` command erases all data on the drive',
              'fix_now',
              'The drive\'s current MBR layout must be replaced with GPT for OpenCore compatibility',
              'The drive becomes GPT-formatted and ready for flashing',
              'All data on the drive will be erased by the `clean` command',
            )
          : ctx.platform === 'darwin'
            ? act(
                'Reformat in Disk Utility: select the drive → Erase → choose "GUID Partition Map" scheme',
                'medium',
                'Disk Utility\'s Erase function reliably creates a GPT layout',
                'fix_now',
                'Selecting "GUID Partition Map" as the scheme creates a GPT partition table',
                'The drive becomes GPT-formatted and ready for flashing',
                'All data on the drive will be erased',
              )
            : act(
                'Reformat with: sudo parted /dev/sdX mklabel gpt',
                'medium',
                'parted directly writes a new GPT partition table to the block device',
                'fix_now',
                'The current MBR layout must be replaced with GPT',
                'The drive becomes GPT-formatted and ready for flashing',
                'All data on the drive will be erased',
              ),
      ],
    }),
  },

  // ── Insufficient disk space ───────────────────────────────────
  {
    test: m => m.includes('enospc') || m.includes('disk space') || m.includes('not enough space'),
    code: 'insufficient_space',
    category: 'environment_error',
    build: () => ({
      title: 'Insufficient free space',
      explanation: 'The recovery image and EFI build need at least 8 GB of temporary space on your main drive.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Free 8 GB+ on your main drive — delete temp files, empty trash, or move large files',
        'high',
        'The space requirement is fixed: ~500 MB for EFI + ~700 MB for recovery + working overhead',
        'fix_now',
        'The OS reported ENOSPC (no space left on device) during a file write',
        'The operation succeeds once sufficient free space is available',
      ),
      alternatives: [
        act(
          'Skip recovery download and use EFI-only mode — needs less than 500 MB',
          'high',
          'EFI-only mode skips the ~700 MB recovery download, dramatically reducing space requirements',
          'try_alternative',
          'Most of the space requirement comes from the macOS recovery image',
          'The EFI builds successfully; you create the macOS installer separately',
          'You will need access to a Mac or existing macOS to create the full installer USB',
        ),
      ],
    }),
  },

  // ── Hardware scan failure ─────────────────────────────────────
  {
    test: m => m.includes('scan') && (m.includes('fail') || m.includes('error')),
    code: 'hardware_scan_failed',
    category: 'hardware_error',
    build: (ctx) => {
      const tier = getEscalationTier(ctx.retryCount ?? 0);
      if (tier === 'escalated') {
        return {
          title: 'Hardware detection keeps failing',
          explanation: 'Multiple scan attempts have failed. The system query mechanism may need manual intervention.',
          severity: 'critical',
          decisionSummary: '',
          primaryAction: ctx.platform === 'win32'
            ? act(
                'Open Services (services.msc) and restart "Windows Management Instrumentation"',
                'high',
                'Restarting the WMI service clears stale locks and corrupted caches that persist across retries',
                'fix_now',
                'Multiple scan failures indicate the WMI service itself is degraded, not just busy',
                'WMI queries succeed immediately after the service restarts',
              )
            : ctx.platform === 'linux'
              ? act(
                  'Install required tools: sudo apt install lshw pciutils dmidecode',
                  'high',
                  'The scan directly invokes these binaries — if they are missing, all queries fail every time',
                  'fix_now',
                  'Repeated failures on Linux usually mean the tools are missing, not that the queries are flaky',
                  'The scan completes with full PCI-ID-based hardware identification',
                )
              : act(
                  'Copy diagnostics and report the issue — this may indicate a macOS system query bug',
                  'medium',
                  'macOS system_profiler failures after multiple retries are unusual and may indicate a system issue',
                  'learn_more',
                  'macOS hardware queries rarely fail — persistent failure warrants investigation',
                  'A bug report with diagnostics helps identify and fix the root cause',
                ),
          alternatives: [],
        };
      }
      return {
        title: 'Hardware detection failed',
        explanation: ctx.platform === 'win32'
          ? 'A WMI query returned an error. This sometimes happens when another program is using the WMI service.'
          : 'A system information query failed. This may be a permissions issue or a missing system tool.',
        severity: 'actionable',
        decisionSummary: '',
        primaryAction: act(
          'Retry the scan',
          'medium',
          'WMI/system query failures are often transient — caused by a temporary lock from another process',
          'fix_now',
          ctx.platform === 'win32'
            ? 'WMI queries can fail when another process holds an exclusive lock on the WMI repository'
            : 'System information queries can fail when tools are missing or permissions are insufficient',
          'The scan completes and hardware is detected for config generation',
        ),
        alternatives: [
          ...(ctx.platform === 'win32' ? [act(
            'Open Services (services.msc) and restart "Windows Management Instrumentation"',
            'medium',
            'Restarting the WMI service clears stale locks and corrupted caches',
            'fix_now',
            'The WMI service can become unresponsive after long uptime or conflicting queries',
            'WMI queries succeed immediately after the service restarts',
          )] : []),
          ...(ctx.platform === 'linux' ? [act(
            'Install required tools: sudo apt install lshw pciutils dmidecode',
            'high',
            'The scan directly invokes these binaries — if they are missing, all queries fail',
            'fix_now',
            'Minimal Linux installs may not include hardware detection utilities',
            'The scan completes with full PCI-ID-based hardware identification',
          )] : []),
        ],
      };
    },
  },

  // ── GitHub rate limit ─────────────────────────────────────────
  {
    test: m => m.includes('rate limit') || m.includes('429'),
    code: 'github_rate_limit',
    category: 'network_error',
    build: () => ({
      title: 'GitHub rate limit reached',
      explanation: 'GitHub limits unauthenticated API requests to 60/hour. The app has exhausted this quota.',
      severity: 'actionable',
      decisionSummary: '',
      primaryAction: act(
        'Wait for the rate limit reset — typically 5–15 minutes',
        'high',
        'GitHub\'s rate limit is a hard server-side counter that resets on a rolling hourly window',
        'fix_now',
        'The 60-request/hour quota has been exhausted — retrying immediately will also fail',
        'After the reset window, all kext downloads succeed at full speed',
      ),
      alternatives: [
        act(
          'Download kexts manually from each repo\'s Releases page and place in EFI/OC/Kexts/',
          'medium',
          'Browser downloads use different rate limit buckets than API requests',
          'try_alternative',
          'The API rate limit only applies to programmatic requests, not browser downloads',
          'You get the exact same kext binaries — placed in the correct directory',
          'Requires manually visiting each kext\'s GitHub page and extracting the correct release asset',
        ),
      ],
    }),
  },
];

// ─── Context-aware enhancement ──────────────────────────────────────────────

function enhanceWithContext(suggestion: Suggestion, ctx: SuggestionContext): Suggestion {
  const enhanced = { ...suggestion, alternatives: [...suggestion.alternatives] };
  const profile = ctx.profile;
  if (!profile) return enhanced;

  const targetVersion = parseMacOSVersion(profile.targetOS);
  const gpuDevices = getProfileGpuDevices(profile);
  const hasModernUnsupportedNvidia = hasUnsupportedModernNvidia(gpuDevices);
  const supportedDisplayPath = getBestSupportedGpuPath(gpuDevices, targetVersion);
  const hasAnySupportedDisplayPath = hasSupportedDisplayPath(gpuDevices, targetVersion);

  if (hasModernUnsupportedNvidia) {
    if (supportedDisplayPath?.vendor === 'Intel') {
      enhanced.contextNote = 'An unsupported NVIDIA dGPU was detected, but a supported Intel iGPU display path remains. The NVIDIA path must stay disabled for macOS.';
    } else if (supportedDisplayPath?.vendor === 'AMD') {
      enhanced.contextNote = 'An unsupported NVIDIA dGPU was detected, but a supported AMD display path remains. Keep the monitor on the supported AMD output and leave the NVIDIA path unused.';
    } else {
      enhanced.contextNote = 'An unsupported NVIDIA GPU was detected and no supported display path remains. This hardware needs a supported Intel iGPU or supported AMD GPU before macOS can boot reliably.';
      enhanced.severity = 'critical';
    }
  }

  if (profile.isVM) {
    enhanced.alternatives.push(act(
      'Configure PCIe GPU passthrough in your hypervisor',
      'high',
      'macOS requires Metal-capable GPU — virtual GPUs do not support Metal',
      'learn_more',
      'Without GPU passthrough, macOS runs on a virtual display adapter without hardware acceleration',
      'Full Metal GPU acceleration in the VM, making macOS usable for real work',
      'Requires compatible hardware (IOMMU/VT-d), a second GPU, and hypervisor configuration',
    ));
  }

  if (profile.scanConfidence === 'low') {
    enhanced.alternatives.push(act(
      'Verify your CPU and GPU models match what the config expects',
      'medium',
      'The hardware scan could not confirm component identities via PCI vendor IDs',
      'learn_more',
      'Low-confidence detection means the config may be targeting the wrong hardware generation',
      'You confirm (or correct) the hardware assumptions before flashing, avoiding a non-booting config',
    ));
  }

  if (profile.architecture === 'AMD' && profile.isLaptop) {
    enhanced.severity = 'critical';
    enhanced.contextNote = (enhanced.contextNote ? enhanced.contextNote + ' ' : '') +
      'AMD laptops cannot run macOS — the kernel does not support AMD mobile platforms.';
  }

  const allowCompatibilityContextNote = ![
    'bios_readiness_blocked',
    'selected_disk_missing',
    'disk_identity_missing',
    'flash_write_error',
    'watchdog_trigger',
  ].includes(suggestion.code);

  if (!hasAnySupportedDisplayPath && allowCompatibilityContextNote) {
    enhanced.severity = 'critical';
    enhanced.contextNote = (enhanced.contextNote ? enhanced.contextNote + ' ' : '') +
      'No supported display path remains for the selected macOS target. Retrying other steps will not make this machine bootable.';
  }

  return enhanced;
}

// ─── Constants ──────────────────────────────────────────────────────────────

function describeRetryTarget(step?: string): string {
  switch (step) {
    case 'scanning':
      return 'Retry the hardware scan';
    case 'bios':
      return 'Recheck the BIOS step';
    case 'building':
    case 'kext-fetch':
      return 'Retry the EFI build';
    case 'recovery-download':
      return 'Retry Apple recovery download';
    case 'usb-select':
      return 'Refresh the removable-drive list';
    case 'part-prep':
      return 'Refresh the disk list';
    default:
      return 'Retry the current step';
  }
}

function buildUnknownFallback(ctx: SuggestionContext): Suggestion {
  const retryTarget = describeRetryTarget(ctx.step);
  return {
    code: 'unknown_error',
    category: 'environment_error',
    severity: 'actionable',
    title: 'Something went wrong',
    explanation: 'An unexpected error interrupted this step before the app could classify it cleanly.',
    decisionSummary: `Recommended: ${retryTarget}. If it fails again, copy the report instead of repeating the same action blindly.`,
    primaryAction: {
      text: retryTarget,
      confidence: 'medium',
      confidenceReason: 'The safest first move is to retry the exact step that failed once with the current state.',
      group: 'fix_now',
      reason: 'Unexpected failures are often transient, but the retry should stay anchored to the step that actually failed.',
      expectedOutcome: 'The failed step either succeeds or produces a clearer, more specific error.',
      recommended: true,
    },
    alternatives: [
      act(
        'Copy the report and open an issue if the same error repeats',
        'high',
        'Repeated unknown failures need diagnostics instead of repeated blind retries.',
        'learn_more',
        'A sanitized report preserves the useful details without exposing secrets or local paths.',
        'You can report the failure with enough context to debug it.',
      ),
    ],
  };
}

// ─── Recommendation logic ───────────────────────────────────────────────────

/** Apply recommendation signal: exactly one action gets recommended: true.
 *  Context overrides can shift the recommendation away from primary. */
function applyRecommendation(suggestion: Suggestion, ctx: SuggestionContext): Suggestion {
  const result: Suggestion = {
    ...suggestion,
    primaryAction: { ...suggestion.primaryAction, recommended: false },
    alternatives: suggestion.alternatives.map(a => ({ ...a, recommended: false })),
  };

  // Context-based override: NVIDIA unsupported + no iGPU → don't recommend retry on scan failures
  const profile = ctx.profile;
  let recommendedSet = false;

  if (profile) {
    const targetVersion = parseMacOSVersion(profile.targetOS);
    const gpuDevices = getProfileGpuDevices(profile);
    const isModernNvidia = hasUnsupportedModernNvidia(gpuDevices);
    const noSupportedDisplayPath = !hasSupportedDisplayPath(gpuDevices, targetVersion);

    // If GPU is unsupported and no fallback, and this is a hardware scan...
    // don't recommend retry — recommend the alternative that addresses the real issue
    if (isModernNvidia && noSupportedDisplayPath && suggestion.code === 'hardware_scan_failed') {
      const altIdx = result.alternatives.findIndex(a =>
        a.text.toLowerCase().includes('gpu') || a.text.toLowerCase().includes('passthrough')
      );
      if (altIdx >= 0) {
        result.alternatives[altIdx].recommended = true;
        result.decisionSummary = `Recommended: ${result.alternatives[altIdx].text} — your NVIDIA GPU has no macOS support and there is no iGPU fallback, so fixing the scan alone won't resolve the GPU issue.`;
        recommendedSet = true;
      }
    }

    // AMD laptop — nothing we recommend will make macOS work
    if (!recommendedSet && profile.architecture === 'AMD' && profile.isLaptop) {
      result.decisionSummary = 'No recommended action — AMD laptops are not supported by macOS. This is a hardware limitation that cannot be worked around.';
      recommendedSet = true;
    }
  }

  // Default: recommend primary action if nothing else set
  if (!recommendedSet) {
    result.primaryAction.recommended = true;
    result.decisionSummary = buildDecisionSummary(result.primaryAction, suggestion);
  }

  // FINAL SAFETY: Guarantee exactly one recommended action (unless specifically "none" above)
  const allActions = [result.primaryAction, ...result.alternatives];
  const recommendedCount = allActions.filter(a => a.recommended).length;

  if (recommendedCount === 0 && !result.decisionSummary.includes('No recommended action')) {
    result.primaryAction.recommended = true;
  } else if (recommendedCount > 1) {
    // Keep only the first recommended action found, set others to false
    let found = false;
    if (result.primaryAction.recommended) {
      found = true;
    }
    result.alternatives = result.alternatives.map(alt => {
      if (alt.recommended) {
        if (found) return { ...alt, recommended: false };
        found = true;
      }
      return alt;
    });
  }

  return result;
}

function buildDecisionSummary(recommended: SuggestedAction, suggestion: Suggestion): string {
  // Extract the core action text (first clause before " — ")
  const shortText = recommended.text.includes(' — ')
    ? recommended.text.split(' — ')[0]
    : recommended.text;

  // Build a summary that explains WHY this path is preferred NOW
  if (recommended.confidence === 'high') {
    return `Recommended: ${shortText} — this has the highest success rate for this type of failure.`;
  }
  if (recommended.confidence === 'medium') {
    return `Recommended: ${shortText} — this is the most likely fix, though it may take more than one attempt.`;
  }
  return `Suggested: ${shortText} — confidence is low, but this is still the best available option.`;
}

function hasGitHubOnlyKextFailure(ctx: SuggestionContext): boolean {
  const values = Object.values(ctx.kextSources ?? {});
  return values.includes('failed') && !values.includes('embedded');
}

function buildValidationSuggestion(ctx: SuggestionContext): Suggestion | null {
  const trace = ctx.validationTrace;
  if (!trace) return null;

  const exactLocation = `${trace.component} at ${trace.expectedPath}`;
  const isKextIssue = trace.code === 'KEXT_MISSING' || trace.code === 'KEXT_EXPECTED_MISSING';

  if (isKextIssue && trace.source === 'unknown' && hasGitHubOnlyKextFailure(ctx)) {
    const suggestion: Suggestion = {
      code: 'efi_validation_kext_fetch_failure',
      category: 'validation_error',
      title: 'EFI validation failed on a missing kext',
      explanation: `${exactLocation} failed validation. The required kext never landed on disk, and no embedded fallback was used.`,
      decisionSummary: '',
      severity: 'critical',
      primaryAction: act(
        'Retry the EFI build after waiting briefly for GitHub to recover',
        'high',
        'The validator identified a missing kext and the recorded kext sources show no embedded fallback was used',
        'fix_now',
        'This is a concrete missing-kext failure, not a guessed generic build issue',
        `The missing kext is downloaded and ${trace.expectedPath} is populated`,
      ),
      alternatives: [
        act(
          `Inspect ${trace.expectedPath} directly before rebuilding`,
          'medium',
          'The validator already identified the exact missing component and path',
          'learn_more',
          'This confirms the kext is absent instead of guessing from generic build output',
          'You can verify the exact missing component before retrying',
        ),
      ],
    };
    return applyRecommendation(enhanceWithContext(suggestion, ctx), ctx);
  }

  if (trace.code === 'DRIVER_MISSING' || trace.code === 'MISSING_FILE') {
    const suggestion: Suggestion = {
      code: `efi_validation_${trace.code.toLowerCase()}`,
      category: 'validation_error',
      title: 'EFI is missing a required OpenCore component',
      explanation: `${exactLocation} is missing. This is an EFI integrity failure, not a guessed network issue.`,
      decisionSummary: '',
      severity: 'critical',
      primaryAction: act(
        'Rebuild the EFI and confirm the missing OpenCore file exists before flashing',
        'high',
        'The validator identified the exact missing OpenCore component and path',
        'fix_now',
        'Boot-critical files like OpenRuntime.efi, BOOTx64.efi, OpenCore.efi, and config.plist must exist before the EFI can boot',
        `${trace.expectedPath} is restored and validation passes`,
      ),
      alternatives: [
        act(
          `Inspect ${trace.expectedPath} directly in the EFI folder`,
          'medium',
          'The failing path is already known and localized',
          'learn_more',
          'Manual inspection helps when the same file keeps disappearing across rebuilds',
          'You confirm whether the file is missing, undersized, or overwritten',
        ),
      ],
    };
    return applyRecommendation(enhanceWithContext(suggestion, ctx), ctx);
  }

  if (trace.code === 'OPENRUNTIME_VERSION_MISMATCH' || trace.code === 'OPENCORE_VERSION_MISMATCH') {
    const suggestion: Suggestion = {
      code: `efi_validation_${trace.code.toLowerCase()}`,
      category: 'validation_error',
      title: 'OpenCore binaries do not come from one release set',
      explanation: `${exactLocation} has a version mismatch. Mixed OpenCore binaries are a real boot blocker.`,
      decisionSummary: '',
      severity: 'critical',
      primaryAction: act(
        'Rebuild the EFI from one consistent OpenCore release',
        'high',
        'The validator found a release-marker mismatch between boot-critical binaries',
        'fix_now',
        'Mixed OpenCore.efi, BOOTx64.efi, and OpenRuntime.efi sets are not safe to boot',
        'All OpenCore sidecar markers match and the EFI becomes self-consistent',
      ),
      alternatives: [
        act(
          'Delete the generated EFI folder and rebuild from scratch',
          'medium',
          'A clean rebuild avoids carrying stale binaries forward',
          'try_alternative',
          'Repeated version mismatches often come from stale files surviving a prior build',
          'The EFI contains only one OpenCore release set',
        ),
      ],
    };
    return applyRecommendation(enhanceWithContext(suggestion, ctx), ctx);
  }

  if (trace.code === 'AIRPORTITLWM_SECUREBOOT_REQUIRED') {
    const suggestion: Suggestion = {
      code: 'efi_validation_airportitlwm_secureboot_required',
      category: 'validation_error',
      title: 'AirportItlwm was configured on an invalid SecureBootModel path',
      explanation: `${exactLocation} failed validation because AirportItlwm needs SecureBootModel-enabled behavior for native and Recovery networking.`,
      decisionSummary: '',
      severity: 'critical',
      primaryAction: act(
        'Enable SecureBootModel for the AirportItlwm path or switch to Itlwm',
        'high',
        'The validator matched a concrete Intel Wi-Fi configuration error',
        'fix_now',
        'AirportItlwm is only valid when SecureBootModel behavior is enabled correctly',
        'Intel Wi-Fi uses a documented path instead of a broken mixed configuration',
      ),
      alternatives: [
        act(
          'Use wired Ethernet for the installer instead of relying on Intel Wi-Fi',
          'high',
          'Ethernet avoids Intel Wi-Fi Recovery-path limitations entirely',
          'try_alternative',
          'Wired Ethernet is the most reliable installer networking path',
          'Recovery networking works without Intel Wi-Fi special cases',
        ),
      ],
    };
    return applyRecommendation(enhanceWithContext(suggestion, ctx), ctx);
  }

  if (trace.code === 'KEXT_LILU_DEPENDENCY') {
    const suggestion: Suggestion = {
      code: 'efi_validation_kext_lilu_dependency',
      category: 'validation_error',
      title: 'A Lilu plugin was selected without Lilu',
      explanation: `${exactLocation} failed validation because a plugin kext depends on Lilu.kext.`,
      decisionSummary: '',
      severity: 'critical',
      primaryAction: act(
        'Add Lilu.kext and rebuild the EFI',
        'high',
        'This is a deterministic dependency failure, not a guessed download problem',
        'fix_now',
        'WhateverGreen, AppleALC, RestrictEvents, and similar plugins cannot load without Lilu',
        'Plugin kexts have their required dependency and validation passes',
      ),
      alternatives: [
        act(
          'Remove the dependent plugin if you do not actually need it',
          'medium',
          'Removing the plugin also resolves the missing dependency chain',
          'try_alternative',
          'A smaller kext set reduces failure points when the plugin is not needed',
          'The EFI no longer references an invalid dependency chain',
        ),
      ],
    };
    return applyRecommendation(enhanceWithContext(suggestion, ctx), ctx);
  }

  const suggestion: Suggestion = {
    code: `efi_validation_${trace.code.toLowerCase()}`,
    category: 'validation_error',
    title: 'EFI validation failed on a specific component',
    explanation: `${exactLocation} failed validation. ${trace.detail}`,
    decisionSummary: '',
    severity: 'critical',
    primaryAction: act(
      'Rebuild the EFI now that the exact failing path is known',
      'high',
      'The validator identified the exact component and path instead of a guessed cause',
      'fix_now',
      'This is a concrete EFI structure failure',
      `${trace.expectedPath} is rebuilt correctly and validation passes`,
    ),
    alternatives: [
      act(
        `Open the EFI folder and inspect ${trace.expectedPath}`,
        'medium',
        'The failure is localized to one known path',
        'learn_more',
        'Manual inspection is useful when the same component fails repeatedly across rebuilds',
        `You confirm why ${trace.component} is missing, undersized, or invalid`,
      ),
    ],
  };
  return applyRecommendation(enhanceWithContext(suggestion, ctx), ctx);
}

// ─── Public API ─────────────────────────────────────────────────────────────

export function getSuggestion(ctx: SuggestionContext): Suggestion {
  const normalizedCtx: SuggestionContext = {
    ...ctx,
    errorMessage: normalizeErrorMessage(ctx.errorMessage),
  };
  const msg = normalizedCtx.errorMessage.toLowerCase();
  const validationSuggestion = buildValidationSuggestion(normalizedCtx);
  if (validationSuggestion) return validationSuggestion;

  for (const tmpl of TEMPLATES) {
    if (tmpl.test(msg)) {
      const built = tmpl.build(normalizedCtx);
      const suggestion: Suggestion = {
        code: tmpl.code,
        category: tmpl.category,
        ...built,
      };
      const enhanced = enhanceWithContext(suggestion, normalizedCtx);
      return applyRecommendation(enhanced, normalizedCtx);
    }
  }

  return buildUnknownFallback(normalizedCtx);
}

export interface ActionPayload {
  text: string;
  confidence: string;
  confidenceReason: string;
  group: string;
  reason: string;
  expectedOutcome: string;
  risk?: string;
  recommended: boolean;
}

export function getSuggestionPayload(ctx: SuggestionContext): {
  message: string;
  explanation?: string;
  decisionSummary?: string;
  suggestion?: string;
  suggestionRecommended?: boolean;
  suggestionReason?: string;
  suggestionOutcome?: string;
  suggestionConfidenceReason?: string;
  suggestionRisk?: string;
  alternatives?: ActionPayload[];
  category?: string;
  contextNote?: string;
  code?: string;
  severity?: string;
  validationCode?: string;
  validationComponent?: string;
  validationPath?: string;
  validationSource?: string;
  validationDetail?: string;
} {
  const structured = structureError(ctx.errorMessage);
  const suggestion = getSuggestion(ctx);

  if (!suggestion) {
    return {
      message: structured.title,
      explanation: structured.what,
      suggestion: structured.nextStep,
    };
  }

  const p = suggestion.primaryAction;
  return {
    message: suggestion.title,
    explanation: suggestion.explanation,
    decisionSummary: suggestion.decisionSummary,
    suggestion: p.text,
    suggestionRecommended: p.recommended,
    suggestionReason: p.reason,
    suggestionOutcome: p.expectedOutcome,
    suggestionConfidenceReason: p.confidenceReason,
    suggestionRisk: p.risk,
    alternatives: suggestion.alternatives.map(a => ({
      text: a.text,
      confidence: a.confidence,
      confidenceReason: a.confidenceReason,
      group: a.group,
      reason: a.reason,
      expectedOutcome: a.expectedOutcome,
      recommended: a.recommended,
      ...(a.risk ? { risk: a.risk } : {}),
    })),
    category: suggestion.category,
    contextNote: suggestion.contextNote,
    code: suggestion.code,
    severity: suggestion.severity,
    validationCode: ctx.validationTrace?.code,
    validationComponent: ctx.validationTrace?.component,
    validationPath: ctx.validationTrace?.expectedPath,
    validationSource: ctx.validationTrace?.source,
    validationDetail: ctx.validationTrace?.detail,
  };
}
