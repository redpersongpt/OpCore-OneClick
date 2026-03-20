// ── Failure Explainer Engine ─────────────────────────────────────────────────
// Translates raw validation/compatibility errors into human-readable explanations
// with context: what went wrong, why it matters, and what the user can do.

import type { ValidationIssue } from '../../electron/configValidator';

export interface ExplainedFailure {
  headline: string;
  explanation: string;
  impact: string;
  action: string;
  severity: 'blocked' | 'warning';
  category: 'missing_file' | 'config_mismatch' | 'dependency' | 'security' | 'structure' | 'compatibility' | 'unknown';
}

// ── Code-to-explanation map ──────────────────────────────────────────────────

const CODE_EXPLANATIONS: Record<string, Omit<ExplainedFailure, 'severity'>> = {
  MISSING_BOOTX64: {
    headline: 'Missing BOOTx64.efi',
    explanation: 'The EFI is missing BOOTx64.efi, the UEFI boot stub that firmware loads first. Without it, the system cannot find OpenCore.',
    impact: 'The USB drive will not appear as a bootable device in your BIOS boot menu.',
    action: 'Rebuild the EFI. This file is always generated — its absence indicates a corrupted or incomplete build.',
    category: 'missing_file',
  },
  MISSING_OC_EFI: {
    headline: 'Missing OpenCore.efi',
    explanation: 'OpenCore.efi is the bootloader itself. Without it, the boot stub has nothing to hand off to.',
    impact: 'The boot process will fail immediately after firmware POST.',
    action: 'Rebuild the EFI. This is a core component that should always be present.',
    category: 'missing_file',
  },
  MISSING_CONFIG_PLIST: {
    headline: 'Missing config.plist',
    explanation: 'The config.plist is OpenCore\'s configuration file — it defines every aspect of the boot process, from kext loading to ACPI patches.',
    impact: 'OpenCore will fail to initialize and display an error or hang at boot.',
    action: 'Rebuild the EFI. This file is generated from your hardware profile.',
    category: 'missing_file',
  },
  MISSING_OPENRUNTIME: {
    headline: 'Missing OpenRuntime.efi',
    explanation: 'OpenRuntime.efi provides critical memory services during the boot process. It handles memory mapping, slide calculation, and UEFI runtime fixes.',
    impact: 'OpenCore will fail before macOS begins loading — typically with a "OC: Failed to load booter" error.',
    action: 'Rebuild the EFI. OpenRuntime is a required OpenCore driver.',
    category: 'missing_file',
  },
  MISSING_HFSPLUS: {
    headline: 'Missing HfsPlus.efi',
    explanation: 'HfsPlus.efi allows OpenCore to read HFS+ volumes. The macOS Recovery partition uses HFS+ format.',
    impact: 'OpenCore cannot read the macOS Recovery image on the USB drive. The installer will not appear in the boot picker.',
    action: 'Rebuild the EFI if you need Recovery/Installer support. If using an EFI-only setup, this may be acceptable.',
    category: 'missing_file',
  },
  DRIVER_CONFIG_MISMATCH: {
    headline: 'Driver listed in config but missing from disk',
    explanation: 'The config.plist references a UEFI driver that does not exist at the expected path. OpenCore loads drivers from the list in config.plist — if a file is missing, it fails.',
    impact: 'OpenCore will panic or hang during driver loading phase.',
    action: 'Rebuild the EFI to regenerate the driver set, or manually remove the entry from config.plist.',
    category: 'config_mismatch',
  },
  KEXT_CONFIG_MISMATCH: {
    headline: 'Kext listed in config but missing from disk',
    explanation: 'A kernel extension referenced in config.plist is not present in the EFI/OC/Kexts folder.',
    impact: 'OpenCore will fail to inject the kext, potentially causing a kernel panic if the kext is critical (like Lilu or VirtualSMC).',
    action: 'Rebuild the EFI. If a kext download failed, check your internet connection and try again.',
    category: 'config_mismatch',
  },
  KEXT_MISSING_EXECUTABLE: {
    headline: 'Kext bundle is empty or corrupt',
    explanation: 'The kext bundle exists but its executable (declared in Info.plist as CFBundleExecutable) is missing. This usually means the download was incomplete.',
    impact: 'macOS will fail to load this kext. If it\'s a critical kext like VirtualSMC, the system will kernel panic.',
    action: 'Rebuild the EFI to re-download the kext. If the issue persists, the kext release may be broken upstream.',
    category: 'missing_file',
  },
  MISSING_LILU_WITH_PLUGINS: {
    headline: 'Lilu is missing but plugins depend on it',
    explanation: 'Lilu is the core patching framework. Kexts like WhateverGreen, AppleALC, and NootRX are Lilu plugins — they literally cannot function without it.',
    impact: 'All Lilu-dependent kexts will fail to load. This typically means no GPU acceleration, no audio, and possibly a kernel panic.',
    action: 'Rebuild the EFI. Lilu should always be included when any plugin kext is selected.',
    category: 'dependency',
  },
  AIRPORTITLWM_SECUREBOOT: {
    headline: 'AirportItlwm requires SecureBootModel',
    explanation: 'AirportItlwm uses Apple\'s native IO80211 framework, which requires SecureBootModel to be configured. Without it, the kext is blocked by macOS security checks.',
    impact: 'Wi-Fi will not work in macOS or during Recovery/Installation.',
    action: 'Enable SecureBootModel in the OpenCore configuration, or switch to Itlwm (which does not require it but lacks Recovery Wi-Fi).',
    category: 'security',
  },
  ACPI_DSL_PRESENT: {
    headline: 'Source .dsl file found in ACPI folder',
    explanation: 'The EFI ACPI folder contains a .dsl source file. Only compiled .aml files should be in the EFI. Source files indicate the ACPI table was not compiled or was accidentally copied.',
    impact: 'OpenCore may attempt to load the .dsl file and fail, or it will be ignored while the compiled version is missing.',
    action: 'Remove .dsl files from EFI/OC/ACPI. Only .aml (compiled) files belong there.',
    category: 'structure',
  },
  DSDT_IN_ACPI: {
    headline: 'Full DSDT dump found in ACPI folder',
    explanation: 'A complete DSDT (Differentiated System Description Table) dump was found in the ACPI folder. Dortania strongly advises against using a full DSDT replacement — it can cause instability across macOS updates.',
    impact: 'May work initially but will likely break on macOS updates. Can cause random crashes and hardware issues.',
    action: 'Remove the DSDT and use targeted SSDT patches instead. This is the Dortania-recommended approach.',
    category: 'structure',
  },
  MIXED_OC_VERSIONS: {
    headline: 'Mixed OpenCore component versions',
    explanation: 'Different OpenCore components (OpenCore.efi, OpenRuntime.efi, etc.) appear to be from different OpenCore releases. This can cause subtle incompatibilities.',
    impact: 'May boot fine but could cause random crashes, boot failures on updates, or incorrect behavior.',
    action: 'Rebuild the EFI to ensure all OpenCore components are from the same release.',
    category: 'structure',
  },
};

// ── Pattern-based explanation for unknown codes ──────────────────────────────

function explainByPattern(issue: ValidationIssue): ExplainedFailure {
  const msg = issue.message.toLowerCase();
  const comp = issue.component.toLowerCase();

  if (msg.includes('missing') || msg.includes('not found')) {
    return {
      headline: `Missing: ${issue.component}`,
      explanation: `The file ${issue.expectedPath} is required but was not found on disk. ${issue.actualCondition}`,
      impact: 'This component is needed for the EFI to function correctly.',
      action: 'Rebuild the EFI to regenerate missing files.',
      severity: issue.severity,
      category: 'missing_file',
    };
  }

  if (msg.includes('mismatch') || msg.includes('does not match')) {
    return {
      headline: `Configuration mismatch: ${issue.component}`,
      explanation: `The config.plist and the actual EFI contents are out of sync. ${issue.detail ?? issue.actualCondition}`,
      impact: 'OpenCore may fail to load components or load unexpected ones.',
      action: 'Rebuild the EFI to synchronize the configuration with the actual file contents.',
      severity: issue.severity,
      category: 'config_mismatch',
    };
  }

  return {
    headline: issue.message,
    explanation: `${issue.component} at ${issue.expectedPath}: ${issue.actualCondition}`,
    impact: issue.severity === 'blocked' ? 'This issue will prevent the EFI from working correctly.' : 'This issue may cause problems but is not necessarily fatal.',
    action: issue.detail ?? 'Review the issue and rebuild if necessary.',
    severity: issue.severity,
    category: 'unknown',
  };
}

// ── Main explainer ──────────────────────────────────────────────────────────

export function explainValidationFailures(issues: ValidationIssue[]): ExplainedFailure[] {
  const seen = new Set<string>();
  const results: ExplainedFailure[] = [];

  for (const issue of issues) {
    // Deduplicate by code
    const dedupeKey = issue.code + ':' + issue.expectedPath;
    if (seen.has(dedupeKey)) continue;
    seen.add(dedupeKey);

    const known = CODE_EXPLANATIONS[issue.code];
    if (known) {
      results.push({ ...known, severity: issue.severity });
    } else {
      results.push(explainByPattern(issue));
    }
  }

  // Sort: blocked first, then warnings
  results.sort((a, b) => {
    if (a.severity === 'blocked' && b.severity !== 'blocked') return -1;
    if (a.severity !== 'blocked' && b.severity === 'blocked') return 1;
    return 0;
  });

  return results;
}

// ── Compatibility error explainer ───────────────────────────────────────────

export function explainCompatibilityErrors(errors: string[]): ExplainedFailure[] {
  return errors.map(error => {
    const lower = error.toLowerCase();

    if (lower.includes('gpu') || lower.includes('display') || lower.includes('graphics')) {
      return {
        headline: 'No Supported Display Path',
        explanation: error,
        impact: 'macOS cannot boot without a working display driver. Without GPU acceleration, the system would be unusable.',
        action: 'Check if your GPU is on the Dortania supported hardware list. If using a laptop with unsupported dGPU, ensure the iGPU is available.',
        severity: 'blocked' as const,
        category: 'compatibility' as const,
      };
    }

    if (lower.includes('amd') && lower.includes('laptop')) {
      return {
        headline: 'AMD Laptop — Not Supported',
        explanation: error,
        impact: 'AMD laptop iGPUs (Vega/RDNA integrated) have very limited macOS support. Most AMD laptops cannot run macOS at all.',
        action: 'Consider using a desktop system or an Intel-based laptop instead.',
        severity: 'blocked' as const,
        category: 'compatibility' as const,
      };
    }

    return {
      headline: 'Compatibility Issue',
      explanation: error,
      impact: 'This issue may prevent macOS from booting or functioning correctly.',
      action: 'Review the Dortania guide for your specific hardware configuration.',
      severity: 'blocked' as const,
      category: 'compatibility' as const,
    };
  });
}
