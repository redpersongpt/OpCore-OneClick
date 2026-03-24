import { getAMDPatches, AMD_PATCH_COMPLETENESS } from './amdPatches.js';
import {
    classifyGpu,
    getBestSupportedGpuPath,
    getProfileGpuDevices,
    hasMacProEraAmdGpu,
    hasUnsupportedModernNvidia,
    needsNaviPikera,
    parseMacOSVersion,
    type HardwareGpuDeviceSummary,
} from './hackintoshRules.js';

// ── SIP Policy ──────────────────────────────────────────────────────────────
// csr-active-config values, least-permissive first.
// Each level justifies exactly what it enables and why.
//
// CSR_ALLOW_UNTESTED_KEXTS      = 0x001
// CSR_ALLOW_UNRESTRICTED_FS     = 0x002
// CSR_ALLOW_UNRESTRICTED_DTRACE = 0x020
// CSR_ALLOW_UNRESTRICTED_NVRAM  = 0x040
// CSR_ALLOW_UNAUTHENTICATED_ROOT = 0x800
//
// Standard hackintosh (Dortania recommendation): 0x67
//   Allows unsigned kexts, unrestricted FS, DTRACE, NVRAM writes.
//   Sufficient for OpenCore + kexts on all supported macOS versions.
//
// Root-patching builds (OCLP-dependent GPU paths): 0xFEF
//   Full SIP disable minus Apple Internal. Required for OCLP to
//   patch the GPU driver stack into the sealed system volume.

export function getSIPPolicy(profile: HardwareProfile, gpuDevices: HardwareGpuDeviceSummary[]): { value: string; reason: string } {
    const assessments = gpuDevices.map(classifyGpu);
    const osVer = parseMacOSVersion(profile.targetOS);

    // Builds using OCLP-dependent GPU paths (legacy NVIDIA Kepler, older AMD GCN,
    // or unsupported GPUs running on older macOS with root patches)
    const hasOCLPPath = assessments.some(a =>
        a.tier === 'supported_with_limit' && a.maxMacOSVersion !== null && osVer > a.maxMacOSVersion
    );

    if (hasOCLPPath) {
        // 0xFEF = full SIP disable except CSR_ALLOW_APPLE_INTERNAL
        // Needed for OCLP root volume patching on legacy GPU paths.
        return {
            value: '7w8AAA==', // 0x00000FEF little-endian
            reason: 'OCLP root-patching path detected — near-full SIP disable required',
        };
    }

    // Standard Hackintosh: Dortania-recommended minimum
    // 0x67 = kexts + FS + DTRACE + NVRAM
    return {
        value: 'ZwAAAA==', // 0x00000067 little-endian
        reason: 'Standard SIP policy — allows unsigned kexts and NVRAM writes',
    };
}

// Kexts that have no binary (Info.plist only) — must use empty ExecutablePath in config.plist
const CODELESS_KEXTS = new Set([
    'AppleMCEReporterDisabler.kext',
]);

// --- Types ---

export interface HardwareProfile {
    cpu: string;
    architecture: 'Intel' | 'AMD' | 'Apple Silicon' | 'Unknown';
    generation: 'Penryn' | 'Nehalem' | 'Arrandale' | 'Clarkdale' | 'Westmere' | 'Wolfdale' | 'Yorkfield' | 'Bulldozer' | 'Sandy Bridge' | 'Ivy Bridge' | 'Haswell' | 'Broadwell' | 'Skylake' | 'Kaby Lake' | 'Ice Lake' | 'Coffee Lake' | 'Comet Lake' | 'Rocket Lake' | 'Alder Lake' | 'Raptor Lake' | 'Ivy Bridge-E' | 'Haswell-E' | 'Broadwell-E' | 'Cascade Lake-X' | 'Ryzen' | 'Threadripper' | 'Apple Silicon' | 'Unknown';
    coreCount?: number;
    gpu: string;
    gpuDevices?: HardwareGpuDeviceSummary[];
    ram: string;
    motherboard: string;
    targetOS: string;
    smbios: string;
    kexts: string[];
    ssdts: string[];
    bootArgs: string;
    isLaptop: boolean;
    isVM?: boolean;
    audioLayoutId?: number;
    strategy?: 'canonical' | 'conservative' | 'blocked';
    /**
     * Overall confidence level of the hardware scan result.
     * Populated by the main process from DetectionConfidence values.
     * 'high'   → all components detected with vendor IDs
     * 'medium' → some components inferred from name/fallback
     * 'low'    → one or more components unverified
     */
    scanConfidence?: 'high' | 'medium' | 'low';
}

interface Quirks {
    // Booter
    AvoidRuntimeDefrag: boolean;
    DevirtualiseMmio: boolean;
    EnableSafeModeSlide: boolean;
    EnableWriteUnprotector: boolean;
    ProtectUefiServices: boolean;
    ProvideCustomSlide: boolean;
    RebuildAppleMemoryMap: boolean;
    SetupVirtualMap: boolean;
    SyncRuntimePermissions: boolean;

    // Kernel
    AppleCpuPmCfgLock: boolean;
    AppleXcpmCfgLock: boolean;
    AppleXcpmExtraMsrs: boolean;
    DisableIoMapper: boolean;
    DisableRtcChecksum: boolean;
    FixupAppleEfiImages: boolean;
    PanicNoKextDump: boolean;
    PowerTimeoutKernelPanic: boolean;
    ProvideCurrentCpuInfo: boolean;
    XhciPortLimit: boolean;

    // UEFI
    RequestBootVarRouting: boolean;
    ReleaseUsbOwnership: boolean;
    UnblockFsConnect: boolean;
}

const BASE_QUIRKS: Quirks = {
    AvoidRuntimeDefrag: true,
    DevirtualiseMmio: false,
    EnableSafeModeSlide: true,
    EnableWriteUnprotector: false,
    ProtectUefiServices: false,
    ProvideCustomSlide: true,
    RebuildAppleMemoryMap: true,
    SetupVirtualMap: true,
    SyncRuntimePermissions: true,

    AppleCpuPmCfgLock: true,
    AppleXcpmCfgLock: true,
    AppleXcpmExtraMsrs: false,
    DisableIoMapper: true,
    DisableRtcChecksum: false,
    FixupAppleEfiImages: false,
    PanicNoKextDump: true,
    PowerTimeoutKernelPanic: true,
    ProvideCurrentCpuInfo: false,
    XhciPortLimit: false,

    RequestBootVarRouting: true,
    ReleaseUsbOwnership: true,
    UnblockFsConnect: false
};

// --- SMBIOS Lookup (Dortania-compliant, OS-version-aware) ---
// Source: ventura.html, monterey.html, tahoe.html, smbios-support.html

export function getSMBIOSForProfile(profile: HardwareProfile): string {
    if (profile.isVM) {
        return profile.architecture === 'AMD' || profile.generation.includes('Ryzen') || profile.generation.includes('Threadripper') ? 'MacPro7,1' : 'iMacPro1,1';
    }

    const osVer = parseMacOSVersion(profile.targetOS);
    const gpuDevices = getProfileGpuDevices(profile);
    const gpuAssessments = gpuDevices.map(classifyGpu);
    const bestDisplayPath = getBestSupportedGpuPath(gpuDevices, osVer);
    const hasDiscreteDisplayPath = bestDisplayPath?.isLikelyDiscrete
        ?? gpuAssessments.some(assessment => assessment.isLikelyDiscrete && assessment.tier !== 'unsupported');

    if (profile.architecture === 'AMD') {
        if (profile.generation === 'Threadripper') return 'MacPro7,1';
        if (profile.generation === 'Bulldozer') return 'iMacPro1,1';
        if (osVer >= 10.15 && hasMacProEraAmdGpu(gpuDevices)) return 'MacPro7,1';
        return 'iMacPro1,1';
    }

    // Tahoe (26+): only MacBookPro16,1+, iMac20,1+, MacPro7,1, iMacPro1,1
    // Legacy Intel (pre-Skylake) is not supported on Tahoe — fail explicitly.
    if (osVer >= 26) {
        if (TAHOE_UNSUPPORTED_GENERATIONS.has(profile.generation)) {
            throw new Error(
                `${profile.generation} is not supported on ${profile.targetOS}. ` +
                `macOS Tahoe (26+) requires Skylake or newer Intel hardware. ` +
                `The maximum supported macOS for ${profile.generation} is macOS Monterey (12).`,
            );
        }
        if (profile.isLaptop) return 'MacBookPro16,1';
        // HEDT / server
        if (profile.generation.includes('-E') || profile.generation.includes('-X')) {
            return 'MacPro7,1';
        }
        // Generations with no macOS iGPU driver — must use MacPro7,1
        if (['Rocket Lake', 'Alder Lake', 'Raptor Lake'].includes(profile.generation)) {
            return 'MacPro7,1';
        }
        // Consumer Intel desktop (Skylake–Comet Lake): iGPU works in macOS,
        // use iMac20,1 regardless of dGPU presence (dGPU runs display,
        // iGPU runs headless compute — both work under iMac20,1 SMBIOS).
        return 'iMac20,1';
    }

    // Ventura (13+): drops iMac17,x and older, MacBookPro13,x and older
    // Monterey (12): drops iMac15,x and older, MacBookPro11,3 and older

    if (profile.isLaptop) {
        switch (profile.generation) {
            case 'Arrandale':
            case 'Clarkdale':
                return osVer >= 12 ? 'MacBookPro11,4' : 'MacBookPro6,2';
            case 'Sandy Bridge':
            case 'Ivy Bridge':
                // Unsupported on Ventura+, use Monterey-safe MacBookPro11,4 fallback
                return osVer >= 13 ? 'MacBookPro14,1' : (osVer >= 12 ? 'MacBookPro11,4' : 'MacBookPro10,1');
            case 'Haswell':
                // Monterey: MacBookPro11,4/11,5 still supported. Ventura: use MacBookPro14,1
                return osVer >= 13 ? 'MacBookPro14,1' : 'MacBookPro11,4';
            case 'Broadwell': return osVer >= 13 ? 'MacBookPro14,1' : 'MacBookPro12,1';
            case 'Skylake': return osVer >= 13 ? 'MacBookPro14,1' : 'MacBookPro13,1';
            case 'Kaby Lake': return 'MacBookPro14,1';
            case 'Coffee Lake': return 'MacBookPro15,2';
            case 'Ice Lake': return 'MacBookPro16,2';
            case 'Comet Lake':
            case 'Rocket Lake':
            case 'Alder Lake':
            case 'Raptor Lake': return 'MacBookPro16,1';
            default: return 'MacBookPro16,1';
        }
    }

    // Server / HEDT
    if (profile.generation.includes('-E') || profile.generation.includes('-X')) {
        // Haswell-E / Broadwell-E / Ivy Bridge-E / Cascade Lake-X
        if (osVer >= 13) return 'MacPro7,1';
        return 'MacPro6,1';
    }

    // Desktop
    switch (profile.generation) {
        case 'Wolfdale':
        case 'Yorkfield':
        case 'Nehalem':
        case 'Westmere':
        case 'Clarkdale':
            return osVer >= 12 ? 'iMac14,4' : 'iMac10,1';
        case 'Penryn': return osVer >= 12 ? 'iMac14,4' : 'iMac10,1'; // Legacy fallback
        case 'Sandy Bridge':
            return osVer >= 13 ? (hasDiscreteDisplayPath ? 'iMac18,2' : 'iMac18,1') : (osVer >= 12 ? 'iMac16,2' : 'iMac12,2');
        case 'Ivy Bridge':
            // Monterey: MacPro6,1 for dGPU. Ventura: iMac18,x
            return osVer >= 13 ? (hasDiscreteDisplayPath ? 'iMac18,2' : 'iMac18,1') : (osVer >= 12 ? (hasDiscreteDisplayPath ? 'MacPro6,1' : 'iMac16,2') : 'iMac13,2');
        case 'Haswell':
            return osVer >= 13 ? (hasDiscreteDisplayPath ? 'iMac18,2' : 'iMac18,1') : (osVer >= 12 ? (hasDiscreteDisplayPath ? 'iMac17,1' : 'iMac16,2') : 'iMac14,4');
        case 'Broadwell': return osVer >= 13 ? (hasDiscreteDisplayPath ? 'iMac18,2' : 'iMac18,1') : 'iMac16,2';
        case 'Skylake': return osVer >= 13 ? (hasDiscreteDisplayPath ? 'iMac18,2' : 'iMac18,1') : 'iMac17,1';
        case 'Kaby Lake': return hasDiscreteDisplayPath ? 'iMac18,3' : 'iMac18,1';
        case 'Coffee Lake': return 'iMac19,1';
        case 'Comet Lake': return hasDiscreteDisplayPath ? 'iMac20,2' : 'iMac20,1';
        case 'Rocket Lake':
        case 'Alder Lake':
        case 'Raptor Lake': return 'MacPro7,1';
        default: return 'iMac19,1';
    }
}

// --- Quirks by Generation ---

export function getQuirksForGeneration(gen: HardwareProfile['generation'], motherboard: string = '', isVM: boolean = false, strategy: HardwareProfile['strategy'] = 'canonical', targetOS: string = ''): Quirks {
    const quirks = { ...BASE_QUIRKS };
    const mb = motherboard.toLowerCase();

    if (isVM) {
        quirks.ProvideCurrentCpuInfo = true;
    }

    if (strategy === 'conservative') {
        // Safer defaults for uncertain hardware
        quirks.DevirtualiseMmio = true;
        quirks.SetupVirtualMap = true;
        quirks.DisableIoMapper = true;
        quirks.AppleCpuPmCfgLock = true;
        quirks.AppleXcpmCfgLock = true;
    }

    switch (gen) {
        case 'Penryn':
        case 'Wolfdale':
        case 'Yorkfield':
        case 'Nehalem':
        case 'Westmere':
        case 'Arrandale':
        case 'Clarkdale':
        case 'Sandy Bridge':
        case 'Ivy Bridge':
            // Legacy firmware — use EnableWriteUnprotector instead of RebuildAppleMemoryMap
            quirks.EnableWriteUnprotector = true;
            quirks.RebuildAppleMemoryMap = false;
            quirks.SyncRuntimePermissions = false;
            // Pre-Sandy Bridge uses AppleIntelCPUPowerManagement, not XCPM
            if (['Penryn', 'Wolfdale', 'Yorkfield', 'Nehalem', 'Westmere', 'Arrandale', 'Clarkdale'].includes(gen)) {
                quirks.AppleXcpmCfgLock = false;
            }
            break;
        case 'Haswell':
        case 'Broadwell':
            // Moderate era — EnableWriteUnprotector still preferred for most boards
            quirks.EnableWriteUnprotector = true;
            quirks.RebuildAppleMemoryMap = false;
            quirks.SyncRuntimePermissions = false;
            // Haswell/Broadwell desktop: AppleCpuPmCfgLock not needed (XCPM only)
            quirks.AppleCpuPmCfgLock = false;
            break;
        case 'Skylake':
        case 'Kaby Lake':
        case 'Ice Lake':
            // Moderate era — EnableWriteUnprotector still preferred for most boards
            quirks.EnableWriteUnprotector = true;
            quirks.RebuildAppleMemoryMap = false;
            quirks.SyncRuntimePermissions = false;
            // Skylake+ only needs XCPM lock, not legacy AppleCpuPm
            quirks.AppleCpuPmCfgLock = false;
            break;
        case 'Ivy Bridge-E':
        case 'Haswell-E':
        case 'Broadwell-E':
            // HEDT (X99) — legacy firmware + AppleXcpmExtraMsrs for extra MSRs
            quirks.EnableWriteUnprotector = true;
            quirks.RebuildAppleMemoryMap = false;
            quirks.SyncRuntimePermissions = false;
            quirks.AppleXcpmExtraMsrs = true;
            quirks.AppleCpuPmCfgLock = false;
            break;
        case 'Cascade Lake-X':
            // HEDT (X299) — modern firmware
            quirks.EnableWriteUnprotector = false;
            quirks.DevirtualiseMmio = true;
            quirks.ProtectUefiServices = true;
            quirks.RebuildAppleMemoryMap = true;
            quirks.SyncRuntimePermissions = true;
            quirks.SetupVirtualMap = false;
            quirks.AppleXcpmExtraMsrs = true;
            quirks.AppleCpuPmCfgLock = false;
            break;
        case 'Coffee Lake':
            // 2018+ firmware — use RebuildAppleMemoryMap approach
            quirks.EnableWriteUnprotector = false;
            quirks.RebuildAppleMemoryMap = true;
            quirks.SyncRuntimePermissions = true;
            quirks.DevirtualiseMmio = true;
            quirks.AppleCpuPmCfgLock = false;
            // Z390 needs SetupVirtualMap true (older), Z370 too
            quirks.SetupVirtualMap = true;
            // Z390/Z370 boards need ProtectUefiServices — Source: config.plist/coffee-lake.html
            if (mb.includes('z390') || mb.includes('z370')) {
                quirks.ProtectUefiServices = true;
            }
            break;
        case 'Comet Lake':
            quirks.EnableWriteUnprotector = false;
            quirks.RebuildAppleMemoryMap = true;
            quirks.SyncRuntimePermissions = true;
            quirks.DevirtualiseMmio = true;
            quirks.AppleCpuPmCfgLock = false;
            // Comet Lake memory protections break SetupVirtualMap — Source: Dortania comet-lake.html
            quirks.SetupVirtualMap = false;
            // Z490 needs ProtectUefiServices — Source: Dortania comet-lake.html
            if (mb.includes('z490')) {
                quirks.ProtectUefiServices = true;
            }
            break;
        case 'Rocket Lake':
        case 'Alder Lake':
        case 'Raptor Lake':
            quirks.EnableWriteUnprotector = false;
            quirks.DevirtualiseMmio = true;
            quirks.ProtectUefiServices = true;
            quirks.SetupVirtualMap = false;
            quirks.RebuildAppleMemoryMap = true;
            quirks.SyncRuntimePermissions = true;
            quirks.ProvideCurrentCpuInfo = true;
            quirks.AppleCpuPmCfgLock = false;
            break;
        case 'Bulldozer':
        case 'Ryzen':
        case 'Threadripper':
            quirks.EnableWriteUnprotector = false;
            quirks.DevirtualiseMmio = false;
            quirks.RebuildAppleMemoryMap = true;
            quirks.SyncRuntimePermissions = true;
            quirks.SetupVirtualMap = true;
            quirks.AppleCpuPmCfgLock = false;
            quirks.AppleXcpmCfgLock = false;
            quirks.ProvideCurrentCpuInfo = true;

            // X570, B550, A520, TRx40 → SetupVirtualMap false — Source: Dortania AMD/zen.html
            if (mb.includes('x570') || mb.includes('b550') || mb.includes('a520') || mb.includes('trx40')) {
                quirks.SetupVirtualMap = false;
            }
            // TRx40 → DevirtualiseMmio true — Source: Dortania AMD/zen.html
            if (mb.includes('trx40')) {
                quirks.DevirtualiseMmio = true;
            }
            break;
    }

    // Intel HEDT explicit checks (overrides standard gen logic) — Source: config-HEDT
    if (mb.includes('x99')) {
        // Haswell-E / Broadwell-E
        quirks.EnableWriteUnprotector = true;
        quirks.DevirtualiseMmio = false;
        quirks.RebuildAppleMemoryMap = false;
        quirks.SyncRuntimePermissions = false;
        quirks.SetupVirtualMap = true;
    } else if (mb.includes('x299')) {
        // Skylake-X / Cascade Lake-X
        quirks.EnableWriteUnprotector = false;
        quirks.DevirtualiseMmio = true;
        quirks.ProtectUefiServices = true; // ProtectUefiServices needed for many X299 boards
        quirks.RebuildAppleMemoryMap = true;
        quirks.SyncRuntimePermissions = true;
        quirks.SetupVirtualMap = false; // Important for X299
    }

    // ASUS boards commonly need DisableRtcChecksum to prevent BIOS resets on reboot
    if (mb.includes('asus') || mb.includes('rog') || mb.includes('strix') || mb.includes('tuf')) {
        quirks.DisableRtcChecksum = true;
    }

    // HP systems need UnblockFsConnect
    if (mb.includes('hp') || mb.includes('hewlett')) {
        quirks.UnblockFsConnect = true;
    }

    // Tahoe (26+) requires FixupAppleEfiImages for all Skylake+ Intel and AMD — Source: Dortania tahoe.html
    const osVer = parseMacOSVersion(targetOS);
    if (osVer >= 26) {
        const needsFixup = [
            'Skylake', 'Kaby Lake', 'Ice Lake',
            'Coffee Lake', 'Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake',
            'Cascade Lake-X',
            'Ryzen', 'Threadripper', 'Bulldozer',
        ];
        if (needsFixup.includes(gen)) {
            quirks.FixupAppleEfiImages = true;
        }
    }

    return quirks;
}

// --- BIOS Settings ---

export interface BIOSSetting {
    name: string;
    value: 'Enable' | 'Disable';
    description: string;
    /** Plain-English title for beginners (optional). */
    plainTitle?: string;
    /**
     * Where to find this setting in a typical BIOS/UEFI menu.
     * Optional — not all settings have a predictable location.
     */
    biosLocation?: string;
    /**
     * Plain-English jargon definition shown in beginner mode when the
     * setting name contains technical terms.
     */
    jargonDef?: string;
}

export interface BIOSConfig {
    enable: BIOSSetting[];
    disable: BIOSSetting[];
}

export function getBIOSSettings(profile: HardwareProfile): BIOSConfig {
    const mb = profile.motherboard.toLowerCase();

    if (profile.architecture === 'AMD') {
        // Source: AMD/zen.html → AMD BIOS Settings
        return {
            disable: [
                { name: 'Fast Boot', value: 'Disable', description: 'Prevents issues with hardware initialization', plainTitle: 'Fast Boot', biosLocation: 'Typically found under Boot → Fast Boot, or Advanced → Boot Configuration', jargonDef: 'A shortcut mode that skips hardware checks to boot faster. Causes problems with macOS.' },
                { name: 'Secure Boot', value: 'Disable', description: 'macOS bootloader is not signed by Microsoft', plainTitle: 'Secure Boot', biosLocation: 'Typically found under Security → Secure Boot, or Boot → Secure Boot Control', jargonDef: 'A security feature that only allows Microsoft-signed bootloaders. OpenCore is not signed, so this must be off.' },
                { name: 'Serial/COM Port', value: 'Disable', description: 'Can cause conflicts with macOS', plainTitle: 'Serial / COM Port', biosLocation: 'Typically found under Advanced → Super IO Configuration → Serial Port' },
                { name: 'Parallel Port', value: 'Disable', description: 'Can cause conflicts with macOS', plainTitle: 'Parallel Port', biosLocation: 'Typically found under Advanced → Super IO Configuration → Parallel Port' },
                { name: 'CSM (Compatibility Support Module)', value: 'Disable', description: 'Must be off — GPU errors like gIO are common when enabled', plainTitle: 'Legacy / CSM Mode', biosLocation: 'Typically found under Boot → CSM Support, or Advanced → CSM Configuration', jargonDef: 'A compatibility layer for old operating systems. Must be off for OpenCore to work correctly with modern GPUs.' },
                { name: 'IOMMU', value: 'Disable', description: 'AMD I/O memory management — disable for macOS', plainTitle: 'IOMMU (AMD-Vi)', biosLocation: 'Typically found under Advanced → AMD CBS → NBIO Common Options → NB Configuration → IOMMU', jargonDef: 'A feature that controls how devices access memory. Can cause kernel panics in macOS when enabled.' },
            ] as BIOSSetting[],
            enable: [
                { name: 'Above 4G Decoding', value: 'Enable', description: 'Required. If unavailable, add npci=0x3000 to boot-args. On Gigabyte/AsRock this may break Ethernet — use npci instead.', plainTitle: 'Above 4G Memory Decoding', biosLocation: 'Typically found under Advanced → PCI Subsystem Settings → Above 4G Decoding, or Advanced → PCI Configuration', jargonDef: 'Allows graphics cards to use memory addresses above 4 GB. Required for modern GPUs to initialise correctly in macOS.' },
                { name: 'EHCI/XHCI Hand-off', value: 'Enable', description: 'Lets macOS control USB controllers', plainTitle: 'USB Controller Hand-off', biosLocation: 'Typically found under Advanced → USB Configuration → XHCI Hand-off', jargonDef: 'Tells the firmware to let the OS manage USB controllers. Required for macOS to recognise USB devices.' },
                { name: 'OS Type: Windows 8.1/10 UEFI Mode', value: 'Enable', description: 'Some boards may need "Other OS" instead', plainTitle: 'OS Type: Windows 8.1/10 UEFI (or Other OS)', biosLocation: 'Typically found under Boot → OS Type, or Security → Secure Boot → OS Type' },
                { name: 'SATA Mode: AHCI', value: 'Enable', description: 'Required for macOS SATA recognition', plainTitle: 'Storage Controller Mode: AHCI', biosLocation: 'Typically found under Advanced → SATA Configuration → SATA Mode Selection', jargonDef: 'AHCI is the standard mode for connecting storage drives. macOS requires this — RAID or SMART mode will not work.' },
                { name: 'SVM Mode', value: 'Enable', description: 'AMD Secure Virtual Machine — AMD virtualization', plainTitle: 'AMD CPU Virtualisation (SVM Mode)', biosLocation: 'Typically found under Advanced → CPU Configuration → SVM Mode, or Advanced → AMD CBS → CPU Common Options → SVM Mode', jargonDef: 'Enables AMD\'s virtualisation capability. Required by some macOS kexts and useful if you run VMs.' },
            ] as BIOSSetting[],
        };
    }

    // Intel BIOS settings — Source: config.plist/haswell.html, coffee-lake.html, comet-lake.html
    const config: BIOSConfig = {
        disable: [
            { name: 'Fast Boot', value: 'Disable', description: 'Prevents issues with hardware initialization', plainTitle: 'Fast Boot', biosLocation: 'Typically found under Boot → Fast Boot, or Advanced → Boot Configuration', jargonDef: 'A shortcut that skips hardware checks on startup. Causes macOS boot problems.' },
            { name: 'Secure Boot', value: 'Disable', description: 'macOS bootloader is not signed by Microsoft', plainTitle: 'Secure Boot', biosLocation: 'Typically found under Security → Secure Boot, or Boot → Secure Boot Control', jargonDef: 'Only allows Microsoft-signed boot programs. OpenCore is not signed, so this must be off.' },
            { name: 'Serial/COM Port', value: 'Disable', description: 'Can cause conflicts with macOS', plainTitle: 'Serial / COM Port', biosLocation: 'Typically found under Advanced → Super IO Configuration → Serial Port' },
            { name: 'Parallel Port', value: 'Disable', description: 'Can cause conflicts with macOS', plainTitle: 'Parallel Port', biosLocation: 'Typically found under Advanced → Super IO Configuration → Parallel Port' },
            { name: 'VT-d', value: 'Disable', description: 'Can be enabled if DisableIoMapper is YES in config.plist', plainTitle: 'VT-d (Intel IOMMU)', biosLocation: 'Typically found under Advanced → CPU Configuration → VT-d, or Chipset → System Agent → VT-d', jargonDef: 'Intel\'s version of memory remapping for devices. Can cause kernel panics in macOS with some graphics cards.' },
            { name: 'CSM (Compatibility Support Module)', value: 'Disable', description: 'Must be off — GPU errors like gIO are common when enabled', plainTitle: 'Legacy / CSM Mode', biosLocation: 'Typically found under Boot → CSM Support, or Advanced → CSM Configuration', jargonDef: 'A layer for running old operating systems. Must be off — leaving it on causes GPU errors on boot.' },
            { name: 'Thunderbolt', value: 'Disable', description: 'Disable for initial install — can cause issues if not setup correctly', plainTitle: 'Thunderbolt (disable for initial install)', biosLocation: 'Typically found under Advanced → Thunderbolt Configuration' },
            { name: 'Intel SGX', value: 'Disable', description: 'Software Guard Extensions — not supported by macOS', plainTitle: 'Intel SGX (Software Guard Extensions)', biosLocation: 'Typically found under Advanced → CPU Configuration → Software Guard Extensions (SGX)', jargonDef: 'A security feature for running trusted code. Not supported by macOS and can cause instability.' },
            { name: 'Intel Platform Trust', value: 'Disable', description: 'Platform Trust Technology — not needed for macOS', plainTitle: 'Intel Platform Trust Technology (PTT)', biosLocation: 'Typically found under Advanced → Trusted Computing → TPM Device Selection → PTT', jargonDef: 'A built-in TPM (security chip). Not required for macOS.' },
            { name: 'CFG Lock', value: 'Disable', description: 'MSR 0xE2 write protection — MUST be off. If unavailable, enable AppleXcpmCfgLock quirk. Your hack will not boot with CFG-Lock enabled.', plainTitle: 'CFG Lock (CPU Power Management Protection)', biosLocation: 'Typically found under Advanced → Power & Performance → CPU Power Management Control → CFG Lock', jargonDef: 'Prevents writing to a CPU register that macOS needs for power management. MUST be disabled. If you cannot find it, the AppleXcpmCfgLock quirk is already enabled in your config.' },
        ] as BIOSSetting[],
        enable: [
            { name: 'VT-x', value: 'Enable', description: 'Intel Virtualization Technology', plainTitle: 'Intel CPU Virtualisation (VT-x)', biosLocation: 'Typically found under Advanced → CPU Configuration → Intel Virtualization Technology', jargonDef: 'Enables Intel\'s virtualisation capability. Required by some macOS kexts and useful if you run VMs.' },
            { name: 'Above 4G Decoding', value: 'Enable', description: 'Required for 64-bit device addressing', plainTitle: 'Above 4G Memory Decoding', biosLocation: 'Typically found under Advanced → PCI Subsystem Settings → Above 4G Decoding', jargonDef: 'Allows graphics cards to use memory addresses above 4 GB. Required for modern GPUs.' },
            { name: 'Hyper-Threading', value: 'Enable', description: 'Intel multi-threading support', plainTitle: 'Hyper-Threading (Intel multi-thread)', biosLocation: 'Typically found under Advanced → CPU Configuration → Hyper-Threading', jargonDef: 'Makes each CPU core appear as two to the OS. Required for macOS performance.' },
            { name: 'Execute Disable Bit', value: 'Enable', description: 'Security feature needed by macOS', plainTitle: 'Execute Disable Bit (XD-bit)', biosLocation: 'Typically found under Advanced → CPU Configuration → Execute Disable Bit', jargonDef: 'A CPU security feature that prevents certain types of attacks. macOS requires this to be enabled.' },
            { name: 'EHCI/XHCI Hand-off', value: 'Enable', description: 'Lets macOS control USB controllers', plainTitle: 'USB Controller Hand-off', biosLocation: 'Typically found under Advanced → USB Configuration → XHCI Hand-off (or EHCI Hand-off)', jargonDef: 'Passes USB controller management to the OS. Required for macOS to recognise USB devices.' },
            { name: 'OS Type: Windows 8.1/10 UEFI Mode', value: 'Enable', description: 'Some boards may need "Other OS" instead', plainTitle: 'OS Type: Windows 8.1/10 UEFI (or Other OS)', biosLocation: 'Typically found under Boot → OS Type, or Security → Secure Boot → OS Type' },
            { name: 'DVMT Pre-Allocated (iGPU Memory): 64MB or higher', value: 'Enable', description: 'Required for Intel iGPU framebuffer — must be at least 64MB', plainTitle: 'Intel iGPU Pre-Allocated Memory (≥ 64 MB)', biosLocation: 'Typically found under Advanced → System Agent Configuration → Graphics Configuration → DVMT Pre-Allocated', jargonDef: 'The amount of system RAM reserved for the built-in Intel graphics. Must be at least 64 MB for macOS.' },
            { name: 'SATA Mode: AHCI', value: 'Enable', description: 'Required for macOS SATA recognition', plainTitle: 'Storage Controller Mode: AHCI', biosLocation: 'Typically found under Advanced → SATA Configuration → SATA Mode Selection', jargonDef: 'AHCI is the standard mode for SATA storage. macOS only works with AHCI — not RAID or IDE mode.' },
        ] as BIOSSetting[],
    };

    // Z390 note
    if (mb.includes('z390')) {
        config.enable.push({ name: 'ProtectUefiServices (in config.plist)', value: 'Enable', description: 'Z390 requires this quirk enabled in Booter → Quirks' });
    }
    // Z490 note
    if (mb.includes('z490')) {
        config.enable.push({ name: 'ProtectUefiServices (in config.plist)', value: 'Enable', description: 'Z490 requires this quirk enabled in Booter → Quirks' });
    }

    return config;
}

// --- Required Resources ---

export function getRequiredResources(profile: HardwareProfile) {
    const kexts = ['Lilu.kext', 'VirtualSMC.kext'];
    const ssdts: string[] = [];
    const osVer = parseMacOSVersion(profile.targetOS);
    const gpuDevices = getProfileGpuDevices(profile);
    const gpuAssessments = gpuDevices.map(classifyGpu);
    const motherboard = profile.motherboard.toLowerCase();

    const pushUnique = (entry: string) => {
        if (!kexts.includes(entry)) kexts.push(entry);
    };
    const pushSsdt = (entry: string) => {
        if (!ssdts.includes(entry)) ssdts.push(entry);
    };

    // GPU + Audio — always needed on x86
    if (profile.architecture === 'Intel') {
        pushUnique('WhateverGreen.kext');
        pushUnique('AppleALC.kext');
        // VirtualSMC sensor plugins — CPU temp + fan monitoring
        pushUnique('SMCProcessor.kext');
        pushUnique('SMCSuperIO.kext');
        // Intel I225/I219/I218 Ethernet — covers most Intel desktop boards
        pushUnique('IntelMausi.kext');
        // USB port enumeration for Intel platforms — Source: ktext.html
        if (!profile.isLaptop) {
            pushUnique('USBInjectAll.kext');
        }
    } else if (profile.architecture === 'AMD') {
        if (gpuAssessments.some(gpu => gpu.requiresNootRX)) {
            pushUnique('NootRX.kext');
        } else if (gpuAssessments.some(gpu => gpu.requiresNootedRed)) {
            pushUnique('NootedRed.kext');
        } else {
            pushUnique('WhateverGreen.kext');
        }
        pushUnique('AppleALC.kext');
    }

    // AMD-specific kexts — Source: AMD/zen.html
    // GPU kexts already handled in architecture block above; no duplicate push needed.
    if (profile.architecture === 'AMD') {
        if (profile.generation === 'Ryzen' || profile.generation === 'Threadripper') {
            pushUnique('AMDRyzenCPUPowerManagement.kext');
        }
        if (osVer >= 12) {
            pushUnique('AppleMCEReporterDisabler.kext');
        }
    }

    // Laptop kexts — Source: ktext.html
    if (profile.isLaptop) {
        pushUnique('SMCBatteryManager.kext');
        pushUnique('VoodooPS2Controller.kext');
        // ECEnabler: allows reading/writing EC fields > 8-bit on modern laptops — Source: ktext.html
        pushUnique('ECEnabler.kext');
        pushSsdt('SSDT-PNLF.aml');
        pushSsdt('SSDT-XOSI.aml');
    }

    // NVMe fix for known-bad SSDs — Source: troubleshooting pages
    if (motherboard.includes('pm981') || motherboard.includes('pm991') || motherboard.includes('2200s') || motherboard.includes('600p')) {
        pushUnique('NVMeFix.kext');
    }

    // OTA updates on 14.4+ — Source: tahoe.html, ventura.html
    if (osVer >= 14) {
        pushUnique('RestrictEvents.kext');
    }

    // Intel Bluetooth on Tahoe — Source: tahoe.html
    // (boot-arg -ibtcompatbeta handled in config generator boot-args section)

    // Broadcom WiFi on Sonoma+ — Source: tahoe.html
    // AppleBCMWLANCompanion brings back support without root patching (requires VT-d)
    // Only for Intel systems since AMD doesn't have VT-d
    if (osVer >= 14 && profile.architecture === 'Intel') {
        // Will be conditionally added if Broadcom card detected
    }

    const mb = profile.motherboard.toLowerCase();
    const needsAmdCpuSsdt = /\b(a520|b550|a620|b650|x670|x670e|b850|x870|x870e)\b/.test(mb);

    // SSDTs by platform — Source: per-gen config.plist pages
    if (profile.architecture === 'Intel') {
        if (['Alder Lake', 'Raptor Lake'].includes(profile.generation)) {
            pushSsdt('SSDT-PLUG-ALT.aml');
            pushSsdt('SSDT-AWAC.aml');
            pushSsdt('SSDT-EC-USBX.aml');
            // SSDT-RHUB: USB root hub reset required on Alder/Raptor Lake — Source: Dortania alder-lake.html
            pushSsdt('SSDT-RHUB.aml');
            pushUnique('CPUTopologyRebuild.kext');
        } else if (['Coffee Lake', 'Comet Lake', 'Rocket Lake'].includes(profile.generation)) {
            pushSsdt('SSDT-PLUG.aml');
            pushSsdt('SSDT-AWAC.aml');
            pushSsdt('SSDT-EC-USBX.aml');
            // SSDT-PMC required for 300-series boards (Z370/Z390/H370/B360/H310) for native NVRAM
            // Source: config.plist/coffee-lake.html — "Required for all 300-series motherboards"
            if (mb.includes('z390') || mb.includes('z370') || mb.includes('h370') || mb.includes('b360') || mb.includes('b365') || mb.includes('h310') || mb.includes('q370')) {
                pushSsdt('SSDT-PMC.aml');
            }
            // SSDT-RHUB: USB root hub reset required on Z490 Comet Lake boards — Source: Dortania comet-lake.html
            if (profile.generation === 'Comet Lake' && mb.includes('z490')) {
                pushSsdt('SSDT-RHUB.aml');
            }
        } else if (['Haswell', 'Broadwell'].includes(profile.generation)) {
            // Source: config.plist/haswell.html — USBX not needed on pre-Skylake
            pushSsdt('SSDT-PLUG.aml');
            pushSsdt('SSDT-EC.aml');
        } else if (['Skylake', 'Kaby Lake', 'Ice Lake'].includes(profile.generation)) {
            // Source: config.plist/kaby.html — USBX required for USB power management on 6th+ gen
            pushSsdt('SSDT-PLUG.aml');
            pushSsdt('SSDT-EC-USBX.aml');
            // Ice Lake needs SSDT-AWAC — Source: Dortania ice-lake.html
            if (profile.generation === 'Ice Lake') {
                pushSsdt('SSDT-AWAC.aml');
            }
        } else if (['Ivy Bridge-E', 'Haswell-E', 'Broadwell-E', 'Cascade Lake-X'].includes(profile.generation)) {
            // HEDT — SSDT-PLUG + SSDT-EC-USBX (X99/X299 have modern USB)
            // Source: config.plist/config-HEDT
            pushSsdt('SSDT-PLUG.aml');
            pushSsdt('SSDT-EC-USBX.aml');
            // Cascade Lake-X also needs SSDT-UNC — Source: Dortania HEDT guide
            if (profile.generation === 'Cascade Lake-X') {
                pushSsdt('SSDT-UNC.aml');
            }
        } else if (['Sandy Bridge', 'Ivy Bridge'].includes(profile.generation)) {
            // Sandy Bridge / Ivy Bridge — SSDT-PLUG (XCPM) + legacy EC, no USBX
            pushSsdt('SSDT-PLUG.aml');
            pushSsdt('SSDT-EC.aml');
        } else {
            // Pre-Sandy Bridge (Penryn/Nehalem/Westmere/etc) — no XCPM, EC only
            pushSsdt('SSDT-EC.aml');
        }
    } else if (profile.architecture === 'AMD') {
        pushSsdt('SSDT-EC-USBX-DESKTOP.aml');
        if (needsAmdCpuSsdt) {
            pushSsdt('SSDT-CPUR.aml');
        }
    }

    return { kexts, ssdts };
}

// --- Config.plist Generator ---

// Generations dropped by macOS Tahoe (26+). These CPUs lack the CPU features
// required by the macOS 26 kernel and are not supported by any valid SMBIOS on Tahoe.
// Source: Dortania tahoe.html compatibility table.
const TAHOE_UNSUPPORTED_GENERATIONS = new Set<HardwareProfile['generation']>([
    'Penryn', 'Sandy Bridge', 'Ivy Bridge', 'Haswell', 'Broadwell',
]);

export function generateConfigPlist(profile: HardwareProfile): string {
    const quirks = getQuirksForGeneration(profile.generation, profile.motherboard, profile.isVM, profile.strategy, profile.targetOS);
    const { kexts, ssdts } = getRequiredResources(profile);
    const osVer = parseMacOSVersion(profile.targetOS);
    const gpuDevices = getProfileGpuDevices(profile);

    if (osVer >= 26 && profile.architecture === 'Intel' && TAHOE_UNSUPPORTED_GENERATIONS.has(profile.generation)) {
        throw new Error(
            `${profile.generation} is not supported on ${profile.targetOS}. ` +
            `macOS Tahoe (26+) requires Skylake or newer Intel hardware. ` +
            `The maximum supported macOS for ${profile.generation} is macOS Monterey (12).`,
        );
    }

    const audioLayoutId = profile.audioLayoutId ?? 1;
    const sipPolicy = getSIPPolicy(profile, gpuDevices);
    let bootArgs = profile.bootArgs;

    if (profile.strategy === 'conservative') {
        if (!bootArgs.includes('-v')) bootArgs += ' -v';
        if (!bootArgs.includes('debug=0x100')) bootArgs += ' debug=0x100';
        if (!bootArgs.includes('keepsyms=1')) bootArgs += ' keepsyms=1';
    }

    // Ensure alcid uses the detected layout — AppleALC needs this on all macOS versions
    if (!bootArgs.includes('alcid=')) {
        bootArgs += ` alcid=${audioLayoutId}`;
    } else {
        bootArgs = bootArgs.replace(/alcid=\d+/, `alcid=${audioLayoutId}`);
    }

    // agdpmod=pikera — needed for Navi GPUs (always) and any AMD dGPU on iMac SMBIOS
    // (AppleGraphicsDevicePolicy blocks non-Apple board-ids) — Source: kernel-issues.html
    const smbiosNeedsPikera = profile.smbios.startsWith('iMac') &&
        gpuDevices.map(classifyGpu).some(a => a.vendor === 'AMD' && a.isLikelyDiscrete);
    if (needsNaviPikera(gpuDevices) || smbiosNeedsPikera) {
        if (!bootArgs.includes('agdpmod=pikera')) bootArgs += ' agdpmod=pikera';
    }

    // Unsupported NVIDIA — disable dGPU
    if (hasUnsupportedModernNvidia(gpuDevices)) {
        if (!bootArgs.includes('-wegnoegpu')) bootArgs += ' -wegnoegpu';
    }

    // Intel I225-V NIC DriverKit panic fix — only needed on Comet Lake+ boards (Z490/Z590/Z690/Z790)
    // where the Intel I225-V is common. Older boards use I219 which works fine with DriverKit.
    if (profile.architecture === 'Intel' && ['Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake'].includes(profile.generation)) {
        if (!bootArgs.includes('dk.e1000=0')) bootArgs += ' dk.e1000=0';
    }

    // Coffee Lake+ laptop backlight fix
    if (profile.isLaptop && ['Coffee Lake', 'Comet Lake', 'Rocket Lake', 'Alder Lake', 'Raptor Lake'].includes(profile.generation)) {
        if (!bootArgs.includes('-igfxblr')) bootArgs += ' -igfxblr';
    }

    // Tahoe (26+): Intel Bluetooth needs -ibtcompatbeta — Source: tahoe.html
    if (osVer >= 26 && !bootArgs.includes('-ibtcompatbeta')) {
        bootArgs += ' -ibtcompatbeta';
    }

    // Sonoma 14.4+ / Tahoe: OTA updates need revpatch=sbvmm — Source: tahoe.html
    if (osVer >= 14 && !bootArgs.includes('revpatch=sbvmm')) {
        bootArgs += ' revpatch=sbvmm';
    }

    // CPUID spoofing for unsupported Intel gens
    let cpuid1Data = "AAAAAAAAAAAAAA==";
    let cpuid1Mask = "AAAAAAAAAAAAAA==";
    // Rocket Lake (11th gen) needs Comet Lake CPUID spoof — unsupported CPUID in macOS
    // Alder/Raptor Lake also need spoofing — Source: Dortania per-gen guides
    if (['Rocket Lake', 'Alder Lake', 'Raptor Lake'].includes(profile.generation)) {
        cpuid1Data = "VQYKAAAAAAAAAAAAAAAAAA==";
        cpuid1Mask = "/////wAAAAAAAAAAAAAAAA==";
    }

    // Intel iGPU Device Properties
    // Alder Lake / Raptor Lake / Rocket Lake use MacPro7,1 (discrete GPU) — no iGPU driver
    // in macOS, so we skip iGPU properties entirely for these generations.
    // If any supported discrete GPU exists, the iGPU runs headless (compute only)
    // and the dGPU handles all display output.
    const gpuAssessments = gpuDevices.map(classifyGpu);
    const headlessIgpu = gpuAssessments.some(a => a.isLikelyDiscrete && a.tier !== 'unsupported');

    let gpuProperties = '';
    if (profile.architecture === 'Intel' &&
        !['Alder Lake', 'Raptor Lake', 'Rocket Lake'].includes(profile.generation)) {
        // Display ig-platform-ids — Source: Dortania per-gen config.plist guides
        const DISPLAY_IDS: Record<string, string> = {
            'Haswell':    'AwAiDQ==', // 0x0D220003
            'Broadwell':  'BwAiFg==', // 0x16220007
            'Skylake':    'AAASGQ==', // 0x19120000
            'Kaby Lake':  'AAASWQ==', // 0x59120000
            'Coffee Lake':'BwCbPg==', // 0x3E9B0007
            'Comet Lake': 'BwCbPg==', // 0x3E9B0007
        };
        // Headless ig-platform-ids — Source: Dortania per-gen config.plist guides
        const HEADLESS_IDS: Record<string, string> = {
            'Haswell':    'BAASBA==', // 0x04120004
            'Broadwell':  'BgAmFg==', // 0x16260006 (closest Broadwell headless)
            'Skylake':    'AQASGQ==', // 0x19120001
            'Kaby Lake':  'AwASWQ==', // 0x59120003
            'Coffee Lake':'AwCRPg==', // 0x3E910003
            'Comet Lake': 'AwDImw==', // 0x9BC80003
        };

        const gen = profile.generation;
        const platformId = headlessIgpu
            ? (HEADLESS_IDS[gen] ?? HEADLESS_IDS['Coffee Lake'])
            : (DISPLAY_IDS[gen] ?? DISPLAY_IDS['Coffee Lake']);

        // Framebuffer patches (stolenmem / patch-enable) are only needed when
        // the iGPU drives a display. Headless mode needs no patches.
        const fbPatches = headlessIgpu ? '' : `
                <key>framebuffer-patch-enable</key>
                <data>AQAAAA==</data>
                <key>framebuffer-stolenmem</key>
                <data>AAAwAQ==</data>`;

        gpuProperties = `
            <key>PciRoot(0x0)/Pci(0x2,0x0)</key>
            <dict>
                <key>AAPL,ig-platform-id</key>
                <data>${platformId}</data>${fbPatches}
            </dict>`;
    }

    // AMD Kernel Patches
    let kernelPatches: any[] = [];
    if (profile.architecture === 'AMD') {
        if (!profile.coreCount || profile.coreCount < 1) {
            throw new Error(`AMD build requires a detected core count — got ${profile.coreCount ?? 'none'}. Re-run the hardware scan.`);
        }
        kernelPatches = getAMDPatches(profile.coreCount);
    }

    // Z390 NVRAM fix flags
    const mb = profile.motherboard.toLowerCase();
    const needsLegacyNvram = mb.includes('z390') || mb.includes('z370') || mb.includes('h370') || mb.includes('b360') || mb.includes('b365') || mb.includes('h310') || mb.includes('q370');
    const legacyEnable = needsLegacyNvram ? 'true' : 'false';
    const legacyOverwrite = needsLegacyNvram ? 'true' : 'false';

    // Audio device path — 300-series PCH (Coffee Lake+) moved HDA to PCI 0x1F,0x3;
    // 100/200-series (Skylake, Kaby Lake) and earlier use legacy 0x1B,0x0
    const MODERN_AUDIO_GENS = new Set([
        'Coffee Lake', 'Comet Lake',
        'Rocket Lake', 'Alder Lake', 'Raptor Lake',
    ]);
    const audioDevicePath = (profile.architecture === 'Intel' && MODERN_AUDIO_GENS.has(profile.generation))
        ? 'PciRoot(0x0)/Pci(0x1f,0x3)'
        : 'PciRoot(0x0)/Pci(0x1b,0x0)';

    // Audio layout-id as base64
    const layoutIdBase64 = btoa(String.fromCharCode(audioLayoutId, 0, 0, 0));

    return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>ACPI</key>
    <dict>
        <key>Add</key>
        <array>
            ${ssdts.map(ssdt => `
            <dict>
                <key>Comment</key>
                <string>${ssdt}</string>
                <key>Enabled</key>
                <true/>
                <key>Path</key>
                <string>${ssdt}</string>
            </dict>`).join('')}
        </array>
        <key>Delete</key><array/>
        <key>Patch</key><array/>
        <key>Quirks</key>
        <dict>
            <key>FadtEnableReset</key><false/>
            <key>NormalizeHeaders</key><false/>
            <key>RebaseRegions</key><false/>
            <key>ResetHwSig</key><false/>
            <key>ResetLogoStatus</key><true/>
            <key>SyncTableIds</key><false/>
        </dict>
    </dict>
    <key>Booter</key>
    <dict>
        <key>MmioWhitelist</key><array/>
        <key>Patch</key><array/>
        <key>Quirks</key>
        <dict>
            <key>AllowRelocationBlock</key><false/>
            <key>AvoidRuntimeDefrag</key><${quirks.AvoidRuntimeDefrag}/>
            <key>DevirtualiseMmio</key><${quirks.DevirtualiseMmio}/>
            <key>DisableSingleUser</key><false/>
            <key>DisableVariableWrite</key><false/>
            <key>DiscardHibernateMap</key><false/>
            <key>EnableSafeModeSlide</key><${quirks.EnableSafeModeSlide}/>
            <key>EnableWriteUnprotector</key><${quirks.EnableWriteUnprotector}/>
            <key>ForceBooterSignature</key><false/>
            <key>ForceExitBootServices</key><false/>
            <key>ProtectMemoryRegions</key><false/>
            <key>ProtectSecureBoot</key><false/>
            <key>ProtectUefiServices</key><${quirks.ProtectUefiServices}/>
            <key>ProvideCustomSlide</key><${quirks.ProvideCustomSlide}/>
            <key>ProvideMaxSlide</key><integer>0</integer>
            <key>RebuildAppleMemoryMap</key><${quirks.RebuildAppleMemoryMap}/>
            <key>ResizeAppleGpuBars</key><integer>-1</integer>
            <key>SetupVirtualMap</key><${quirks.SetupVirtualMap}/>
            <key>SignalAppleOS</key><false/>
            <key>SyncRuntimePermissions</key><${quirks.SyncRuntimePermissions}/>
        </dict>
    </dict>
    <key>DeviceProperties</key>
    <dict>
        <key>Add</key>
        <dict>
            <key>${audioDevicePath}</key>
            <dict><key>layout-id</key><data>${layoutIdBase64}</data></dict>
            ${gpuProperties}
        </dict>
        <key>Delete</key><dict/>
    </dict>
    <key>Kernel</key>
    <dict>
        <key>Add</key>
        <array>
            ${kexts.map(kext => {
                const isCodeless = CODELESS_KEXTS.has(kext);
                return `
            <dict>
                <key>Arch</key><string>Any</string>
                <key>BundlePath</key><string>${kext}</string>
                <key>Comment</key><string>${kext}</string>
                <key>Enabled</key><true/>
                <key>ExecutablePath</key><string>${isCodeless ? '' : `Contents/MacOS/${kext.replace('.kext', '')}`}</string>
                <key>MaxKernel</key><string></string>
                <key>MinKernel</key><string></string>
                <key>PlistPath</key><string>Contents/Info.plist</string>
            </dict>`;
            }).join('')}
        </array>
        <key>Block</key><array/>
        <key>Emulate</key>
        <dict>
            <key>Cpuid1Data</key><data>${cpuid1Data}</data>
            <key>Cpuid1Mask</key><data>${cpuid1Mask}</data>
            <key>DummyPowerManagement</key><${profile.architecture === 'AMD' ? 'true' : 'false'}/>
            <key>MaxKernel</key><string></string>
            <key>MinKernel</key><string></string>
        </dict>
        <key>Force</key><array/>
        <key>Patch</key>
        <array>
            ${kernelPatches.map(patch => `
            <dict>
                <key>Arch</key><string>${patch.Arch}</string>
                <key>Base</key><string>${patch.Base}</string>
                <key>Comment</key><string>${patch.Comment}</string>
                <key>Count</key><integer>${patch.Count}</integer>
                <key>Enabled</key><${patch.Enabled}/>
                <key>Find</key><data>${patch.Find}</data>
                <key>Identifier</key><string>${patch.Identifier}</string>
                <key>Limit</key><integer>${patch.Limit}</integer>
                <key>Mask</key><data>${patch.Mask}</data>
                <key>MaxKernel</key><string>${patch.MaxKernel}</string>
                <key>MinKernel</key><string>${patch.MinKernel}</string>
                <key>Replace</key><data>${patch.Replace}</data>
                <key>ReplaceMask</key><data>${patch.ReplaceMask}</data>
                <key>Skip</key><integer>${patch.Skip}</integer>
            </dict>`).join('')}
        </array>
        <key>Quirks</key>
        <dict>
            <key>AppleCpuPmCfgLock</key><${quirks.AppleCpuPmCfgLock}/>
            <key>AppleXcpmCfgLock</key><${quirks.AppleXcpmCfgLock}/>
            <key>AppleXcpmExtraMsrs</key><${quirks.AppleXcpmExtraMsrs}/>
            <key>AppleXcpmForceBoost</key><false/>
            <key>CustomSMBIOSGuid</key><false/>
            <key>DisableIoMapper</key><${quirks.DisableIoMapper}/>
            <key>DisableLinkeditJettison</key><true/>
            <key>DisableRtcChecksum</key><${quirks.DisableRtcChecksum}/>
            <key>FixupAppleEfiImages</key><${quirks.FixupAppleEfiImages}/>
            <key>ExtendBTFeatureFlags</key><false/>
            <key>ExternalDiskIcons</key><false/>
            <key>ForceSecureBootScheme</key><false/>
            <key>IncreasePciBarSize</key><false/>
            <key>LapicKernelPanic</key><false/>
            <key>LegacyCommpage</key><false/>
            <key>PanicNoKextDump</key><${quirks.PanicNoKextDump}/>
            <key>PowerTimeoutKernelPanic</key><${quirks.PowerTimeoutKernelPanic}/>
            <key>ProvideCurrentCpuInfo</key><${quirks.ProvideCurrentCpuInfo}/>
            <key>SetApfsTrimTimeout</key><integer>-1</integer>
            <key>ThirdPartyDrives</key><false/>
            <key>XhciPortLimit</key><${quirks.XhciPortLimit}/>
        </dict>
        <key>Scheme</key>
        <dict>
            <key>CustomKernel</key><false/>
            <key>FuzzyMatch</key><true/>
            <key>KernelArch</key><string>Auto</string>
            <key>KernelCache</key><string>Auto</string>
        </dict>
    </dict>
    <key>Misc</key>
    <dict>
        <key>BlessOverride</key><array/>
        <key>Boot</key>
        <dict>
            <key>ConsoleAttributes</key><integer>0</integer>
            <key>HibernateMode</key><string>None</string>
            <key>HideAuxiliary</key><false/>
            <key>LauncherOption</key><string>Disabled</string>
            <key>LauncherPath</key><string>Default</string>
            <key>PickerAttributes</key><integer>17</integer>
            <key>PickerAudioAssist</key><false/>
            <key>PickerMode</key><string>External</string>
            <key>PickerVariant</key><string>Acidanthera\\GoldenGate</string>
            <key>PollAppleHotKeys</key><false/>
            <key>ShowPicker</key><true/>
            <key>TakeoffDelay</key><integer>0</integer>
            <key>Timeout</key><integer>5</integer>
        </dict>
        <key>Debug</key>
        <dict>
            <key>AppleDebug</key><true/>
            <key>ApplePanic</key><true/>
            <key>DisableWatchDog</key><true/>
            <key>DisplayDelay</key><integer>0</integer>
            <key>DisplayLevel</key><integer>2147483650</integer>
            <key>LogModules</key>
            <string>*</string>
            <key>SysReport</key>
            <false/>
            <key>Target</key>
            <integer>${profile.strategy === 'conservative' ? 67 : 3}</integer>
            </dict>
            <key>Entries</key><array/>

        <key>Security</key>
        <dict>
            <key>AllowNvramReset</key><true/>
            <key>AllowSetDefault</key><true/>
            <key>AllowToggleSip</key><false/>
            <key>ApECID</key><integer>0</integer>
            <key>AuthRestart</key><false/>
            <key>BlacklistAppleUpdate</key><true/>
            <key>DmgLoading</key><string>Signed</string>
            <key>EnablePassword</key><false/>
            <key>ExposeSensitiveData</key><integer>6</integer>
            <key>HaltLevel</key><integer>2147483648</integer>
            <key>PasswordHash</key><data></data>
            <key>PasswordSalt</key><data></data>
            <key>ScanPolicy</key><integer>0</integer>
            <key>SecureBootModel</key><string>Disabled</string>
            <key>Vault</key><string>Optional</string>
        </dict>
        <key>Tools</key>
        <array>
            <dict>
                <key>Arguments</key><string></string>
                <key>Auxiliary</key><true/>
                <key>Comment</key><string>OpenShell.efi</string>
                <key>Enabled</key><true/>
                <key>Flavour</key><string>OpenShell:UEFIShell:Shell</string>
                <key>Name</key><string>OpenShell.efi</string>
                <key>Path</key><string>OpenShell.efi</string>
                <key>RealPath</key><false/>
                <key>TextMode</key><false/>
            </dict>
        </array>
    </dict>
    <key>NVRAM</key>
    <dict>
        <key>Add</key>
        <dict>
            <key>4D1EDE05-38C7-4A6A-9CC6-4BCCA8B38C14</key>
            <dict><key>DefaultBackgroundColor</key><data>AAAAAA==</data></dict>
            <key>4D1FDA02-38C7-4A6A-9CC6-4BCCA8B30102</key>
            <dict><key>rtc-blacklist</key><data></data></dict>
            <key>7C436110-AB2A-4BBB-A880-FE41995C9F82</key>
            <dict>
                <key>ForceDisplayRotationInEFI</key><integer>0</integer>
                <key>SystemAudioVolume</key><data>Rg==</data>
                <key>boot-args</key><string>${bootArgs.trim()}</string>
                <key>csr-active-config</key><data>${sipPolicy.value}</data>
                <key>prev-lang:kbd</key><string>en-US:0</string>
                <key>run-efi-updater</key><string>No</string>
            </dict>
        </dict>
        <key>Delete</key>
        <dict>
            <key>4D1EDE05-38C7-4A6A-9CC6-4BCCA8B38C14</key><array><string>DefaultBackgroundColor</string></array>
            <key>4D1FDA02-38C7-4A6A-9CC6-4BCCA8B30102</key><array><string>rtc-blacklist</string></array>
            <key>7C436110-AB2A-4BBB-A880-FE41995C9F82</key><array><string>boot-args</string><string>csr-active-config</string></array>
        </dict>
        <key>LegacyEnable</key><${legacyEnable}/>
        <key>LegacyOverwrite</key><${legacyOverwrite}/>
        <key>LegacySchema</key><dict/>
        <key>WriteFlash</key><true/>
    </dict>
    <key>PlatformInfo</key>
    <dict>
        <key>Automatic</key><true/>
        <key>CustomMemory</key><false/>
        <key>Generic</key>
        <dict>
            <key>AdviseFeatures</key><false/>
            <key>MaxBIOSVersion</key><false/>
            <key>MLB</key><string>M000000000001</string>
            <key>ProcessorType</key><integer>0</integer>
            <key>ROM</key><data>ESIzAAAA</data>
            <key>SpoofVendor</key><true/>
            <key>SystemMemoryStatus</key><string>Auto</string>
            <key>SystemProductName</key><string>${profile.smbios}</string>
            <key>SystemSerialNumber</key><string>W0000000001</string>
            <key>SystemUUID</key><string>00000000-0000-0000-0000-000000000000</string>
        </dict>
        <key>UpdateDataHub</key><true/>
        <key>UpdateNVRAM</key><true/>
        <key>UpdateSMBIOS</key><true/>
        <key>UpdateSMBIOSMode</key><string>${profile.smbios === 'MacPro7,1' ? 'Custom' : 'Create'}</string>
        <key>UseRawUuidEncoding</key><false/>
    </dict>
    <key>UEFI</key>
    <dict>
        <key>APFS</key>
        <dict>
            <key>EnableJumpstart</key><true/>
            <key>GlobalConnect</key><false/>
            <key>HideVerbose</key><true/>
            <key>JumpstartHotPlug</key><false/>
            <key>MinDate</key><integer>${osVer < 10.15 ? -1 : 0}</integer>
            <key>MinVersion</key><integer>${osVer < 10.15 ? -1 : 0}</integer>
        </dict>
        <key>AppleInput</key>
        <dict>
            <key>AppleEvent</key><string>Builtin</string>
            <key>CustomDelays</key><false/>
            <key>GraphicsInputMirroring</key><true/>
            <key>KeyInitialDelay</key><integer>50</integer>
            <key>KeySubsequentDelay</key><integer>5</integer>
            <key>PointerPollMask</key><integer>-1</integer>
            <key>PointerPollMin</key><integer>10</integer>
            <key>PointerPollMax</key><integer>80</integer>
            <key>PointerSpeedDiv</key><integer>1</integer>
            <key>PointerSpeedMul</key><integer>1</integer>
        </dict>
        <key>Audio</key>
        <dict>
            <key>AudioCodec</key><integer>0</integer>
            <key>AudioDevice</key><string>${audioDevicePath}</string>
            <key>AudioOutMask</key><integer>1</integer>
            <key>AudioSupport</key><false/>
            <key>DisconnectHda</key><false/>
            <key>MaximumGain</key><integer>-15</integer>
            <key>MinimumAssistGain</key><integer>-30</integer>
            <key>MinimumAudibleGain</key><integer>-55</integer>
            <key>PlayChime</key><string>Auto</string>
            <key>ResetTrafficClass</key><false/>
            <key>SetupDelay</key><integer>0</integer>
        </dict>
        <key>ConnectDrivers</key><true/>
        <key>Drivers</key>
        <array>
            <dict>
                <key>Arguments</key><string></string>
                <key>Comment</key><string>HFS+ Driver</string>
                <key>Enabled</key><true/>
                <key>Path</key><string>OpenHfsPlus.efi</string>
            </dict>
            <dict>
                <key>Arguments</key><string></string>
                <key>Comment</key><string></string>
                <key>Enabled</key><true/>
                <key>Path</key><string>OpenRuntime.efi</string>
            </dict>
            <dict>
                <key>Arguments</key><string></string>
                <key>Comment</key><string></string>
                <key>Enabled</key><true/>
                <key>Path</key><string>OpenCanopy.efi</string>
            </dict>
        </array>
        <key>Input</key>
        <dict>
            <key>KeyFiltering</key><false/>
            <key>KeyForgetThreshold</key><integer>5</integer>
            <key>KeySupport</key><true/>
            <key>KeySupportMode</key><string>Auto</string>
            <key>KeySwap</key><false/>
            <key>PointerSupport</key><false/>
            <key>PointerSupportMode</key><string>ASUS</string>
            <key>TimerResolution</key><integer>50000</integer>
        </dict>
        <key>Output</key>
        <dict>
            <key>ClearScreenOnModeSwitch</key><false/>
            <key>ConsoleMode</key><string></string>
            <key>DirectGopRendering</key><false/>
            <key>ForceResolution</key><false/>
            <key>GopPassThrough</key><string>Disabled</string>
            <key>IgnoreTextInGraphics</key><false/>
            <key>ProvideConsoleGop</key><true/>
            <key>ReconnectGraphicsOnConnect</key><false/>
            <key>ReconnectOnResChange</key><false/>
            <key>ReplaceTabWithSpace</key><false/>
            <key>Resolution</key><string>Max</string>
            <key>SanitiseClearScreen</key><false/>
            <key>TextRenderer</key><string>BuiltinGraphics</string>
            <key>UgaPassThrough</key><false/>
            <key>UIScale</key><integer>0</integer>
        </dict>
        <key>ProtocolOverrides</key>
        <dict>
            <key>AppleAudio</key><false/>
            <key>AppleBootPolicy</key><false/>
            <key>AppleDebugLog</key><false/>
            <key>AppleEg2Info</key><false/>
            <key>AppleFramebufferInfo</key><false/>
            <key>AppleImageConversion</key><false/>
            <key>AppleImg4Verification</key><false/>
            <key>AppleKeyMap</key><false/>
            <key>AppleRtcRam</key><false/>
            <key>AppleSecureBoot</key><false/>
            <key>AppleSmcIo</key><false/>
            <key>AppleUserInterfaceTheme</key><false/>
            <key>DataHub</key><false/>
            <key>DeviceProperties</key><false/>
            <key>FirmwareVolume</key><true/>
            <key>HashServices</key><false/>
            <key>OSInfo</key><false/>
            <key>UnicodeCollation</key><false/>
        </dict>
        <key>Quirks</key>
        <dict>
            <key>ActivateHpetSupport</key><false/>
            <key>DisableSecurityPolicy</key><false/>
            <key>EnableVectorAcceleration</key><true/>
            <key>ExitBootServicesDelay</key><integer>0</integer>
            <key>ForceOcWriteFlash</key><false/>
            <key>ForgeUefiSupport</key><false/>
            <key>IgnoreInvalidFlexRatio</key><false/>
            <key>ReleaseUsbOwnership</key><${quirks.ReleaseUsbOwnership}/>
            <key>ReloadOptionRoms</key><false/>
            <key>RequestBootVarRouting</key><${quirks.RequestBootVarRouting}/>
            <key>ResizeGpuBars</key><integer>-1</integer>
            <key>TscSyncTimeout</key><integer>0</integer>
            <key>UnblockFsConnect</key><${quirks.UnblockFsConnect}/>
        </dict>
        <key>ReservedMemory</key><array/>
    </dict>
</dict>
</plist>`;
}
