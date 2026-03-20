import { exec } from 'child_process';
import util from 'util';
import fs from 'fs';

const execPromise = util.promisify(exec);

// ── Types ─────────────────────────────────────────────────────────────────────

/**
 * How a firmware requirement value was determined.
 *
 * confirmed     — Authoritative OS API returned the value directly.
 *                 (Confirm-SecureBootUEFI, /sys/firmware/efi, Win32_Processor CIM)
 *
 * inferred      — Indirect signal suggests the value; not authoritative.
 *                 User should still verify in BIOS.
 *                 (ACPI DMAR table, dmesg keywords, registry fallback)
 *
 * unverified    — Could not probe this value on the current platform.
 *                 User must check manually.
 *
 * failing       — An authoritative source confirmed the requirement is NOT met.
 *                 Only set when evidence level is 'authoritative'; never for heuristics.
 *
 * not_applicable — This requirement cannot be checked in the current host context.
 *                  Set for all Hackintosh requirements when running on macOS,
 *                  where the host is not the target machine.
 */
export type RequirementStatus =
  | 'confirmed'
  | 'inferred'
  | 'unverified'
  | 'failing'
  | 'not_applicable';

export interface FirmwareRequirement {
  id: 'uefi-mode' | 'secure-boot' | 'vt-x' | 'vt-d' | 'above4g';
  name: string;
  /** Why this matters for Hackintosh / OpenCore. */
  description: string;
  /** One-line reason this setting is required. */
  why: string;
  /** One-line consequence of leaving it misconfigured. */
  consequence: string;
  requiredValue: string;
  /** Human-readable current state, or null when status is unverified / not_applicable. */
  detectedValue: string | null;
  status: RequirementStatus;
  /** How this value was determined — the OS command or signal used. */
  source: string;
  critical: boolean;
}

/**
 * Relationship between the machine running the app and the Hackintosh target.
 *
 * is_target      — Windows or Linux: this machine IS the Hackintosh target.
 *                  Full firmware probing applies.
 *
 * running_on_mac — macOS host: the target is a different PC (or this Mac,
 *                  but we cannot distinguish — firmware cannot be read remotely).
 *                  All Hackintosh requirements are not_applicable.
 */
export type HostContext = 'is_target' | 'running_on_mac';

export interface FirmwareInfo {
  hostContext: HostContext;

  // Host BIOS/EFI identity — always populated from the host machine
  vendor: string;
  version: string;
  releaseDate: string;

  // Raw detected booleans — null means detection failed or not attempted
  isUefi: boolean | null;
  secureBoot: boolean | null;
  vtEnabled: boolean | null;
  vtdEnabled: boolean | null;
  above4GDecoding: boolean | null;
  firmwareMode: 'UEFI' | 'Legacy' | 'Unknown';

  /**
   * Overall probe reliability shown in the confidence pill.
   * 'high'           — all critical values confirmed authoritatively
   * 'medium'         — mix of confirmed and inferred/unverified
   * 'low'            — mostly undetected (e.g. PowerShell blocked)
   * 'not_applicable' — macOS host; probing not relevant
   */
  confidence: 'high' | 'medium' | 'low' | 'not_applicable';

  requirements: FirmwareRequirement[];
}

// ── Evidence classifier ───────────────────────────────────────────────────────

type EvidenceLevel = 'authoritative' | 'heuristic' | 'none';

interface RequirementInput {
  detected: boolean | null;
  /** True means the requirement is met when detected===true. */
  expectedTruthy: boolean;
  evidenceLevel: EvidenceLevel;
  source: string;
}

function classify(
  input: RequirementInput,
): Pick<FirmwareRequirement, 'status' | 'source'> {
  const { detected, expectedTruthy, evidenceLevel, source } = input;

  if (evidenceLevel === 'none' || detected === null) {
    return { status: 'unverified', source };
  }

  const met = detected === expectedTruthy;

  if (evidenceLevel === 'authoritative') {
    return { status: met ? 'confirmed' : 'failing', source };
  }

  // Heuristic: regardless of whether the value looks "met", we cannot
  // authoritatively confirm OR deny — always inferred.
  return { status: 'inferred', source };
}

const NOT_APPLICABLE: Pick<FirmwareRequirement, 'status' | 'source'> = {
  status: 'not_applicable',
  source: 'cannot detect remote machine firmware',
};

// ── Requirement definitions ───────────────────────────────────────────────────

function buildRequirements(
  inputs: {
    uefiMode: RequirementInput | null;
    secureBoot: RequirementInput | null;
    vtx: RequirementInput | null;
    vtd: RequirementInput | null;
    above4g: RequirementInput | null;
  },
  notApplicable: boolean,
): FirmwareRequirement[] {
  const pick = (
    input: RequirementInput | null,
  ): Pick<FirmwareRequirement, 'status' | 'source'> => {
    if (notApplicable) return NOT_APPLICABLE;
    if (!input) return { status: 'unverified', source: 'not detectable on this platform' };
    return classify(input);
  };

  const uefiResult = pick(inputs.uefiMode);
  const sbResult = pick(inputs.secureBoot);
  const vtxResult = pick(inputs.vtx);
  const vtdResult = pick(inputs.vtd);
  const above4gResult = pick(inputs.above4g);

  // Derive human-readable detectedValue labels
  const uefiDetected = notApplicable ? null
    : inputs.uefiMode?.detected === true ? 'UEFI'
    : inputs.uefiMode?.detected === false ? 'Legacy BIOS'
    : null;

  const sbDetected = notApplicable ? null
    : inputs.secureBoot?.detected === true ? 'Enabled — needs disabling'
    : inputs.secureBoot?.detected === false ? 'Disabled'
    : null;

  const vtxDetected = notApplicable ? null
    : inputs.vtx?.detected === true ? 'Supported by CPU'
    : inputs.vtx?.detected === false ? 'Not reported by CPU'
    : null;

  const vtdDetected = notApplicable ? null
    : inputs.vtd?.detected === true ? 'Likely enabled'
    : inputs.vtd?.detected === false ? 'Likely disabled'
    : null;

  const above4gDetected = notApplicable ? null
    : inputs.above4g?.detected === true ? 'Likely enabled'
    : inputs.above4g?.detected === false ? 'Likely disabled'
    : null;

  return [
    {
      id: 'uefi-mode',
      name: 'UEFI Boot Mode',
      description:
        'OpenCore requires UEFI firmware. Legacy (BIOS/CSM) mode is not supported. ' +
        'Set Boot Mode to UEFI in your firmware Boot settings. ' +
        'Disable CSM / Legacy Support if present.',
      why: 'OpenCore\'s bootloader is a UEFI application and cannot execute on Legacy/BIOS firmware.',
      consequence: 'Boot fails at the firmware stage — you will never reach the OpenCore picker.',
      requiredValue: 'UEFI',
      detectedValue: uefiDetected,
      critical: true,
      ...uefiResult,
    },
    {
      id: 'secure-boot',
      name: 'Secure Boot',
      description:
        'Secure Boot rejects unsigned bootloaders including OpenCore. ' +
        'Disable it under Security or Boot settings. ' +
        'You can restore it with custom keys after setup if needed.',
      why: 'Secure Boot rejects unsigned EFI binaries; OpenCore\'s BOOTx64.efi is not signed by Microsoft.',
      consequence: 'Firmware refuses to load OpenCore — the system reboots or shows a security error.',
      requiredValue: 'Disabled',
      detectedValue: sbDetected,
      critical: true,
      ...sbResult,
    },
    {
      id: 'vt-x',
      name: 'CPU Virtualisation (VT-x / AMD-V)',
      description:
        'Required for certain macOS kexts and optimal virtualisation performance. ' +
        'Enable in CPU or Advanced settings. ' +
        'Note: CPU flags confirm hardware support — BIOS-level enablement must still be verified in firmware.',
      why: 'Required by AppleHV and other kexts; also needed if you run VMs alongside macOS.',
      consequence: 'Certain kexts panic at boot; virtualisation-dependent features will not work.',
      requiredValue: 'Enabled',
      detectedValue: vtxDetected,
      critical: false,
      ...vtxResult,
    },
    {
      id: 'vt-d',
      name: 'VT-d / AMD-Vi (IOMMU)',
      description:
        'IOMMU remapping can cause kernel panics on some boards with OpenCore. ' +
        'Disable it, or add DisableIoMapper=Yes to your config.plist if you need it on. ' +
        'Detection uses an indirect OS signal — verify in firmware.',
      why: 'IOMMU DMA remapping conflicts with OpenCore\'s memory map on many consumer boards.',
      consequence: 'Intermittent kernel panics or black screen at boot, especially with a discrete GPU.',
      requiredValue: 'Disabled',
      detectedValue: vtdDetected,
      critical: false,
      ...vtdResult,
    },
    {
      id: 'above4g',
      name: 'Above 4G Decoding',
      description:
        'Required for modern discrete GPUs (RX 5000/6000, RTX 20/30/40 series) to initialise correctly. ' +
        'Enable in PCI or PCIe settings. ' +
        'If Resizable BAR / Smart Access Memory is also present, disable it for Hackintosh.',
      why: 'Modern dGPUs need MMIO space above 4 GB; without it the GPU cannot initialise.',
      consequence: 'GPU fails to init — black screen or WhateverGreen errors at boot on RX 5000+ / RTX 20+.',
      requiredValue: 'Enabled',
      detectedValue: above4gDetected,
      critical: false,
      ...above4gResult,
    },
  ];
}

// ── Windows ───────────────────────────────────────────────────────────────────

export async function probeFirmwareWindows(): Promise<FirmwareInfo> {
  const ps = (cmd: string) =>
    execPromise(`powershell -NoProfile -Command "${cmd}"`).catch(() => ({ stdout: '' }));

  const [biosRes, sbCmdletRes, vtRes, vtdRes, sbRegRes] = await Promise.all([
    ps('Get-CimInstance Win32_BIOS | Select-Object Manufacturer, SMBIOSBIOSVersion, ReleaseDate | ConvertTo-Json -Compress'),
    // Confirm-SecureBootUEFI: returns true/false on UEFI, throws on Legacy
    ps('try { $v = Confirm-SecureBootUEFI; $v.ToString().ToLower() } catch { "cmdlet-error" }'),
    // VirtualizationFirmwareEnabled: authoritative CIM query
    ps('try { (Get-CimInstance Win32_Processor).VirtualizationFirmwareEnabled.ToString().ToLower() } catch { "" }'),
    // DMAR ACPI table: heuristic for VT-d — presence implies enabled, absence is not definitive
    ps('try { $null = [System.IO.File]::ReadAllBytes("\\\\?\\Global??\\GLOBALROOT\\Device\\Mup\\acpi\\DMAR"); "present" } catch { "absent" }'),
    // Registry fallback for Secure Boot state (less authoritative than PowerShell cmdlet)
    ps('try { (Get-ItemPropertyValue "HKLM:\\SYSTEM\\CurrentControlSet\\Control\\SecureBoot\\State" -Name "UEFISecureBootEnabled").ToString() } catch { "unknown" }'),
  ]);

  // ── UEFI mode + Secure Boot ──────────────────────────────────────────────
  const sbCmdlet = sbCmdletRes.stdout.trim().toLowerCase();
  let uefiModeInput: RequirementInput;
  let secureBootInput: RequirementInput;

  if (sbCmdlet === 'true' || sbCmdlet === 'false') {
    // Cmdlet succeeded → confirmed UEFI; Secure Boot state is authoritative
    uefiModeInput = {
      detected: true,
      expectedTruthy: true,
      evidenceLevel: 'authoritative',
      source: 'Confirm-SecureBootUEFI PowerShell',
    };
    secureBootInput = {
      detected: sbCmdlet === 'true',   // true = Secure Boot ON = bad
      expectedTruthy: false,           // requirement: must be off
      evidenceLevel: 'authoritative',
      source: 'Confirm-SecureBootUEFI PowerShell',
    };
  } else if (sbCmdlet === 'cmdlet-error') {
    // Cmdlet threw → very likely Legacy BIOS (confirmed Legacy = failing)
    uefiModeInput = {
      detected: false,
      expectedTruthy: true,
      evidenceLevel: 'authoritative',
      source: 'Confirm-SecureBootUEFI PowerShell',
    };
    // On Legacy, Secure Boot is N/A — fall through to registry for best-effort
    const regVal = sbRegRes.stdout.trim();
    secureBootInput = regVal === '0' || regVal === '1'
      ? {
          detected: regVal === '1',
          expectedTruthy: false,
          evidenceLevel: 'heuristic',    // registry is not as authoritative as cmdlet
          source: 'HKLM UEFISecureBootEnabled registry',
        }
      : {
          detected: null,
          expectedTruthy: false,
          evidenceLevel: 'none',
          source: 'not detectable — Confirm-SecureBootUEFI unavailable',
        };
  } else {
    // PowerShell blocked or returned unexpected output
    const regVal = sbRegRes.stdout.trim();
    if (regVal === '0' || regVal === '1') {
      // Registry present → infer UEFI and Secure Boot state (heuristic)
      uefiModeInput = {
        detected: true,
        expectedTruthy: true,
        evidenceLevel: 'heuristic',
        source: 'HKLM UEFISecureBootEnabled registry',
      };
      secureBootInput = {
        detected: regVal === '1',
        expectedTruthy: false,
        evidenceLevel: 'heuristic',
        source: 'HKLM UEFISecureBootEnabled registry',
      };
    } else {
      uefiModeInput  = { detected: null, expectedTruthy: true,  evidenceLevel: 'none', source: 'PowerShell probing unavailable' };
      secureBootInput = { detected: null, expectedTruthy: false, evidenceLevel: 'none', source: 'PowerShell probing unavailable' };
    }
  }

  // ── VT-x ────────────────────────────────────────────────────────────────
  const vtOut = vtRes.stdout.trim().toLowerCase();
  const vtxInput: RequirementInput = vtOut === 'true' || vtOut === 'false'
    ? { detected: vtOut === 'true', expectedTruthy: true, evidenceLevel: 'authoritative', source: 'Win32_Processor.VirtualizationFirmwareEnabled CIM' }
    : { detected: null, expectedTruthy: true, evidenceLevel: 'none', source: 'Win32_Processor CIM query failed' };

  // ── VT-d (ACPI DMAR heuristic) ──────────────────────────────────────────
  const vtdOut = vtdRes.stdout.trim().toLowerCase();
  const vtdInput: RequirementInput = vtdOut === 'present' || vtdOut === 'absent'
    ? {
        detected: vtdOut === 'present',
        expectedTruthy: false,   // requirement: VT-d should be OFF
        evidenceLevel: 'heuristic',   // DMAR table ≠ definitive BIOS setting
        source: 'ACPI DMAR table heuristic',
      }
    : { detected: null, expectedTruthy: false, evidenceLevel: 'none', source: 'ACPI DMAR table not accessible' };

  // ── Above 4G — not readable from Windows userspace ──────────────────────
  const above4gInput: RequirementInput = {
    detected: null,
    expectedTruthy: true,
    evidenceLevel: 'none',
    source: 'not detectable from Windows userspace',
  };

  // ── BIOS identity ────────────────────────────────────────────────────────
  let vendor = 'Unknown', version = 'Unknown', releaseDate = 'Unknown';
  try {
    const b = JSON.parse(biosRes.stdout.trim());
    vendor = b.Manufacturer ?? 'Unknown';
    version = b.SMBIOSBIOSVersion ?? 'Unknown';
    const d: string = b.ReleaseDate ?? '';
    releaseDate = d.length >= 8 ? `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}` : d || 'Unknown';
  } catch {}

  const isUefi =
    uefiModeInput.evidenceLevel !== 'none' && uefiModeInput.detected !== null
      ? uefiModeInput.detected
      : null;

  const secureBoot =
    secureBootInput.evidenceLevel !== 'none' && secureBootInput.detected !== null
      ? secureBootInput.detected
      : null;

  const vtEnabled =
    vtxInput.evidenceLevel !== 'none' && vtxInput.detected !== null
      ? vtxInput.detected
      : null;

  const requirements = buildRequirements(
    { uefiMode: uefiModeInput, secureBoot: secureBootInput, vtx: vtxInput, vtd: vtdInput, above4g: above4gInput },
    false,
  );

  // Confidence: high if UEFI + SecureBoot both authoritative
  const authCount = requirements.filter(r => r.status === 'confirmed' || r.status === 'failing').length;
  const confidence: FirmwareInfo['confidence'] =
    authCount >= 3 ? 'high' : authCount >= 1 ? 'medium' : 'low';

  return {
    hostContext: 'is_target',
    vendor,
    version,
    releaseDate,
    isUefi,
    secureBoot,
    vtEnabled,
    vtdEnabled: null,   // heuristic only — don't surface raw bool as fact
    above4GDecoding: null,
    firmwareMode: isUefi === true ? 'UEFI' : isUefi === false ? 'Legacy' : 'Unknown',
    confidence,
    requirements,
  };
}

// ── Linux ─────────────────────────────────────────────────────────────────────

export async function probeFirmwareLinux(): Promise<FirmwareInfo> {
  const run = (cmd: string) => execPromise(cmd).catch(() => ({ stdout: '' }));

  const [dmidecodeRes, bootctlRes, lscpuRes, procCpuInfoRes, dmesgDmarRes, dmesgAbove4gRes] =
    await Promise.all([
      run('dmidecode -t bios 2>/dev/null'),
      run('bootctl status 2>/dev/null'),
      run('lscpu 2>/dev/null'),
      run('grep -m1 "flags" /proc/cpuinfo 2>/dev/null'),
      run('dmesg 2>/dev/null | grep -iE "DMAR|IOMMU enabled|AMD-Vi" | head -5'),
      run('dmesg 2>/dev/null | grep -iE "above 4G|above4g" | head -3'),
    ]);

  // ── UEFI mode — /sys/firmware/efi is kernel-created only on EFI boot ────
  const efiDirExists = fs.existsSync('/sys/firmware/efi');
  const uefiModeInput: RequirementInput = {
    detected: efiDirExists,
    expectedTruthy: true,
    evidenceLevel: 'authoritative',  // kernel only creates this dir on UEFI
    source: '/sys/firmware/efi presence',
  };

  // ── Secure Boot — bootctl ────────────────────────────────────────────────
  const bootctlOut = bootctlRes.stdout.toLowerCase();
  let sbDetected: boolean | null = null;
  let sbSource = 'bootctl not available';
  if (bootctlOut.includes('secure boot: enabled')) {
    sbDetected = true;
    sbSource = 'bootctl status';
  } else if (bootctlOut.includes('secure boot: disabled') || bootctlOut.includes('secure boot: not enabled')) {
    sbDetected = false;
    sbSource = 'bootctl status';
  } else if (bootctlOut.includes('secure boot:')) {
    sbDetected = false;   // present but not "enabled" → treat as off
    sbSource = 'bootctl status (not-enabled state)';
  }
  const secureBootInput: RequirementInput = {
    detected: sbDetected,
    expectedTruthy: false,
    evidenceLevel: sbDetected !== null ? 'authoritative' : 'none',
    source: sbSource,
  };

  // ── VT-x / AMD-V — CPU flags ─────────────────────────────────────────────
  // CPU flags confirm hardware capability, not that the BIOS has it enabled.
  const lscpuOut = lscpuRes.stdout;
  const cpuFlagsOut = procCpuInfoRes.stdout.toLowerCase();
  const vtxSupported =
    lscpuOut.includes('VT-x') || lscpuOut.includes('AMD-V') ||
    lscpuOut.toLowerCase().includes('vmx') || lscpuOut.toLowerCase().includes('svm') ||
    cpuFlagsOut.includes(' vmx ') || cpuFlagsOut.includes(' svm ');

  // If lscpu/cpuinfo returned content at all, we have a result
  const vtxInput: RequirementInput = lscpuOut.trim() || cpuFlagsOut.trim()
    ? {
        detected: vtxSupported,
        expectedTruthy: true,
        evidenceLevel: 'authoritative',   // CPU flags are authoritative for capability
        source: 'lscpu / /proc/cpuinfo CPU flags (capability, not BIOS state)',
      }
    : { detected: null, expectedTruthy: true, evidenceLevel: 'none', source: 'lscpu not available' };

  // ── VT-d / IOMMU — dmesg keyword heuristic ──────────────────────────────
  const dmarOut = dmesgDmarRes.stdout.toLowerCase();
  const vtdInput: RequirementInput = dmarOut.trim()
    ? {
        detected: true,   // keywords present = IOMMU likely active
        expectedTruthy: false,
        evidenceLevel: 'heuristic',
        source: 'dmesg DMAR/IOMMU heuristic',
      }
    : { detected: null, expectedTruthy: false, evidenceLevel: 'none', source: 'no DMAR/IOMMU entries in dmesg' };

  // ── Above 4G — dmesg heuristic ───────────────────────────────────────────
  const above4gOut = dmesgAbove4gRes.stdout.toLowerCase();
  const above4gInput: RequirementInput = above4gOut.trim()
    ? {
        detected: true,
        expectedTruthy: true,
        evidenceLevel: 'heuristic',
        source: 'dmesg above-4G-decoding heuristic',
      }
    : { detected: null, expectedTruthy: true, evidenceLevel: 'none', source: 'not detectable from Linux userspace' };

  // ── BIOS identity ────────────────────────────────────────────────────────
  let vendor = 'Unknown', version = 'Unknown', releaseDate = 'Unknown';
  const dmi = dmidecodeRes.stdout;
  const vendorMatch = dmi.match(/Vendor:\s*(.+)/);
  const verMatch = dmi.match(/Version:\s*(.+)/);
  const dateMatch = dmi.match(/Release Date:\s*(.+)/);
  if (vendorMatch) vendor = vendorMatch[1].trim();
  if (verMatch) version = verMatch[1].trim();
  if (dateMatch) releaseDate = dateMatch[1].trim();

  const isUefi = efiDirExists;
  const requirements = buildRequirements(
    { uefiMode: uefiModeInput, secureBoot: secureBootInput, vtx: vtxInput, vtd: vtdInput, above4g: above4gInput },
    false,
  );

  const authCount = requirements.filter(r => r.status === 'confirmed' || r.status === 'failing').length;
  const confidence: FirmwareInfo['confidence'] =
    authCount >= 3 ? 'high' : authCount >= 1 ? 'medium' : 'low';

  return {
    hostContext: 'is_target',
    vendor,
    version,
    releaseDate,
    isUefi,
    secureBoot: sbDetected,
    vtEnabled: vtxSupported,
    vtdEnabled: null,
    above4GDecoding: null,
    firmwareMode: isUefi ? 'UEFI' : 'Legacy',
    confidence,
    requirements,
  };
}

// ── macOS ─────────────────────────────────────────────────────────────────────
// The host is a Mac. The Hackintosh target is a different PC (or this same Mac,
// but we cannot detect that automatically). We cannot read the target's firmware.
// All Hackintosh requirements are not_applicable.

export async function probeFirmwareMac(): Promise<FirmwareInfo> {
  const run = (cmd: string) => execPromise(cmd).catch(() => ({ stdout: '' }));
  const bootRomRes = await run("system_profiler SPHardwareDataType 2>/dev/null | grep 'Boot ROM'");
  const romLine = bootRomRes.stdout.split('\n').find(l => l.includes('Boot ROM'));
  const version = romLine ? (romLine.split(':')[1]?.trim() ?? 'Unknown') : 'Unknown';

  const requirements = buildRequirements(
    { uefiMode: null, secureBoot: null, vtx: null, vtd: null, above4g: null },
    true,   // notApplicable = true
  );

  return {
    hostContext: 'running_on_mac',
    vendor: 'Apple',
    version,
    releaseDate: 'Unknown',
    isUefi: true,           // Macs use EFI — informational only
    secureBoot: null,
    vtEnabled: null,
    vtdEnabled: null,
    above4GDecoding: null,
    firmwareMode: 'UEFI',   // informational only
    confidence: 'not_applicable',
    requirements,
  };
}

// ── Entry point ───────────────────────────────────────────────────────────────

export async function probeFirmware(): Promise<FirmwareInfo> {
  if (process.platform === 'win32') return probeFirmwareWindows();
  if (process.platform === 'linux') return probeFirmwareLinux();
  return probeFirmwareMac();
}
