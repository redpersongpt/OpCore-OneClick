// Source: https://github.com/AMD-OSX/AMD_Vanilla (Ryzen 17h/19h)
// Updated for macOS Sonoma / Sequoia compatibility

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

export function getAMDPatches(coreCount: number): KernelPatch[] {
  // Hex value for core count (e.g., 6 -> 06, 8 -> 08, 12 -> 0C, 16 -> 10)
  const coreCountHex = coreCount.toString(16).toUpperCase().padStart(2, '0');
  
  return [
    // TODO: These three patches require the correct binary Find bytes from
    // https://github.com/AMD-OSX/AMD_Vanilla — the values below are placeholders
    // and will never match. Enable and set correct Find/Replace after verifying
    // the actual byte sequences against the AMD_Vanilla config.plist for your OS.
    {
      Arch: "x86_64",
      Base: "_cpuid_set_info",
      Comment: "algrey - Force cpuid_cores_per_package 10.13,10.14 [DISABLED: verify Find bytes]",
      Count: 1,
      Enabled: false,
      Find: "uAAAAAPog==",
      Identifier: "kernel",
      Limit: 0,
      Mask: "//D/AAAAAAAA",
      MaxKernel: "18.99.99",
      MinKernel: "17.0.0",
      Replace: "uAAAAAPog==",
      ReplaceMask: "//AAAAAAAAA",
      Skip: 0
    },
    {
      Arch: "x86_64",
      Base: "_cpuid_set_info",
      Comment: "algrey - Force cpuid_cores_per_package 10.15,11.0 [DISABLED: verify Find bytes]",
      Count: 1,
      Enabled: false,
      Find: "uAAAAAPog==",
      Identifier: "kernel",
      Limit: 0,
      Mask: "//D/AAAAAAAA",
      MaxKernel: "20.99.99",
      MinKernel: "19.0.0",
      Replace: "uAAAAAPog==",
      ReplaceMask: "//AAAAAAAAA",
      Skip: 0
    },
    {
      Arch: "x86_64",
      Base: "_cpuid_set_info",
      Comment: "algrey - Force cpuid_cores_per_package 12.0/13.0/14.0/15.0 [DISABLED: verify Find bytes]",
      Count: 1,
      Enabled: false,
      Find: "uAAAAAPog==",
      Identifier: "kernel",
      Limit: 0,
      Mask: "//D/AAAAAAAA",
      MaxKernel: "",
      MinKernel: "21.0.0",
      Replace: "uAAAAAPog==",
      ReplaceMask: "//AAAAAAAAA",
      Skip: 0
    },
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
        MaxKernel: "24.99.99",
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
        Comment: "Goldfish64 - Bypass GenuineIntel check panic - 12.0/13.0/14.0/15.0",
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
