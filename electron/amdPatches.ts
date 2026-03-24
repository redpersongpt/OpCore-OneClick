// Source: https://github.com/AMD-OSX/AMD_Vanilla (Ryzen 17h/19h)
// Updated for macOS Sonoma / Sequoia / Tahoe compatibility

export interface KernelPatch {
  Arch: string;
  Base: string;
  Comment: string;
  Count: number;
  Enabled: boolean;
  Find: string;
  Identifier: string;
  Limit: number;
  Mask: string;
  MaxKernel: string;
  MinKernel: string;
  Replace: string;
  ReplaceMask: string;
  Skip: number;
}

/**
 * AMD core count patch status.
 * The cpuid_cores_per_package patches require exact binary Find bytes
 * sourced from AMD_Vanilla for each kernel version range. These bytes
 * change between macOS versions and cannot be safely generated.
 *
 * Current status:
 * - The 8 universal AMD patches (wrmsr, rdmsr, cache, microcode, etc.) are
 *   real and verified from AMD_Vanilla.
 * - The cpuid_cores_per_package patches are NOT included because the Find
 *   bytes are version-specific and must be verified against the actual kernel.
 *   Users targeting AMD should source these from AMD_Vanilla for their
 *   specific macOS version and core count.
 *
 * The generator will emit a validation warning when AMD patches are used.
 */
export const AMD_PATCH_COMPLETENESS = {
  hasCoreCountPatches: false,
  missingPatches: ['cpuid_cores_per_package (all kernel ranges)'],
  recommendation: 'Source cpuid_cores_per_package patches from https://github.com/AMD-OSX/AMD_Vanilla for your specific macOS version and core count.',
} as const;

export function getAMDPatches(_coreCount: number): KernelPatch[] {
  // Note: coreCount is accepted for future use when verified core-count
  // patches are available. Currently unused — see AMD_PATCH_COMPLETENESS.

  return [
    // ── Verified patches from AMD_Vanilla ────────────────────────────────
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _i386_init_slave - Remove wrmsr 0x1c8",
        Count: 0,
        Enabled: true,
        Find: "uAEAAADD",
        Identifier: "kernel",
        Limit: 0,
        Mask: "",
        // Extended to cover Tahoe (kernel 25.x) — the byte pattern is
        // stable across kernel versions 20-25 per AMD_Vanilla tracking.
        MaxKernel: "",
        MinKernel: "20.0.0",
        Replace: "Zg8fhAAAAAA=",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _commpage_populate - Remove rdmsr",
        Count: 1,
        Enabled: true,
        Find: "uaABAAAPMg==",
        Identifier: "kernel",
        Limit: 0,
        Mask: "",
        MaxKernel: "",
        MinKernel: "19.0.0",
        Replace: "Dx+AAAAAAA==",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _cpuid_set_cache_info - Set cpuid to 0x8000001D instead 0",
        Count: 1,
        Enabled: true,
        Find: "McAx2zHJMdIPokGJxkGJ0YM9weNA",
        Identifier: "kernel",
        Limit: 0,
        Mask: "/////////////////////wAA////",
        MaxKernel: "",
        MinKernel: "17.0.0",
        Replace: "McAx2zHJMdIPokGJxkGJ0uhHgAAD",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _cpuid_set_generic_info - Remove wrmsr",
        Count: 1,
        Enabled: true,
        Find: "uYsAAAAxwDHSDzA=",
        Identifier: "kernel",
        Limit: 0,
        Mask: "",
        MaxKernel: "18.99.99",
        MinKernel: "17.0.0",
        Replace: "Zg8fhAAAAAAAZpA=",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _cpuid_set_generic_info - Set microcode=186",
        Count: 1,
        Enabled: true,
        Find: "uYsAAAAPMg==",
        Identifier: "kernel",
        Limit: 0,
        Mask: "",
        MaxKernel: "18.99.99",
        MinKernel: "17.0.0",
        Replace: "uroAAABmkA==",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _cpuid_set_generic_info - Set flag=1",
        Count: 1,
        Enabled: true,
        Find: "uRcAAAAPMsHqEoDiBw==",
        Identifier: "kernel",
        Limit: 0,
        Mask: "",
        MaxKernel: "18.99.99",
        MinKernel: "17.0.0",
        Replace: "sgFmDx+EAAAAAABmkA==",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "algrey - _cpuid_set_generic_info - Disable check cpuid_0x80000005",
        Count: 1,
        Enabled: true,
        Find: "Pc94Hw==",
        Identifier: "kernel",
        Limit: 0,
        Mask: "",
        MaxKernel: "18.99.99",
        MinKernel: "17.0.0",
        Replace: "mZbpAA==",
        ReplaceMask: "",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "Shaneee - _cpuid_set_info - Force rb_ucores to 0 to fix 13.3+ restart",
        Count: 1,
        Enabled: true,
        Find: "ugAAAAAA/w+GLw==",
        Identifier: "kernel",
        Limit: 0,
        Mask: "////AAAA/////w==",
        MaxKernel: "22.4.0",
        MinKernel: "22.4.0",
        Replace: "ugAAAAAA/w+GLw==",
        ReplaceMask: "////AAAA/////w==",
        Skip: 0
    },
    {
        Arch: "x86_64",
        Base: "",
        Comment: "Goldfish64 - Bypass GenuineIntel check panic - 12.0+",
        Count: 1,
        Enabled: true,
        Find: "uW4AAAAPvsA5wQAAAAAAAA==",
        Identifier: "kernel",
        Limit: 0,
        Mask: "/////////////wAAAAAAAA==",
        MaxKernel: "",
        MinKernel: "21.0.0",
        Replace: "Zg8fhAAAAAAAZg8fhAAAAA==",
        ReplaceMask: "",
        Skip: 0
    }
  ];
}
