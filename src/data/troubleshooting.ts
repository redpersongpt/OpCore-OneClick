/**
 * Troubleshooting database compiled from:
 * - https://dortania.github.io/OpenCore-Install-Guide/troubleshooting/extended/opencore-issues.html
 * - https://dortania.github.io/OpenCore-Install-Guide/troubleshooting/extended/kernel-issues.html
 * - https://dortania.github.io/OpenCore-Install-Guide/troubleshooting/extended/userspace-issues.html
 * - https://dortania.github.io/OpenCore-Install-Guide/troubleshooting/extended/post-issues.html
 * - https://dortania.github.io/OpenCore-Install-Guide/troubleshooting/extended/misc-issues.html
 */
export type Severity = 'error' | 'warning' | 'info';

export interface TroubleshootingEntry {
  error: string;
  category: string;
  fix: string;
  details?: string;
  severity: Severity;
}

export const troubleshootingData: TroubleshootingEntry[] = [
  // ─── OpenCore Boot Issues ────────────────────────────────────────────────
  {
    error: 'Stuck on black screen before OpenCore picker appears',
    category: 'OpenCore Boot Issues',
    fix: 'Check that HideSelf is disabled in NVRAM → Add → 7C436110... Ensure OpenCanopy.efi or correct Resolution values are set. Try adjusting Resolution to Max in config.plist → UEFI → Output.',
    details: 'Often caused by wrong resolution or driver being hidden by GOP (Graphics Output Protocol) issues.',
    severity: 'error',
  },
  {
    error: 'Stuck on "no vault provided!"',
    category: 'OpenCore Boot Issues',
    fix: 'Set Misc → Security → Vault to Optional in config.plist, or properly sign the vault using the correct tools.',
    severity: 'error',
  },
  {
    error: 'Stuck on "OC: Invalid Vault mode"',
    category: 'OpenCore Boot Issues',
    fix: 'Vault.plist is present but Vault is not enabled, or vice versa. Set Vault to Optional to disable, or provide the signed vault files.',
    severity: 'error',
  },
  {
    error: "Can't see macOS partitions in OpenCore picker",
    category: 'OpenCore Boot Issues',
    fix: 'Enable ScanPolicy = 0 in Misc → Security for testing. Ensure the target drive is using GPT, not MBR. Check Misc → Boot → HideAuxiliary.',
    severity: 'error',
  },
  {
    error: 'OCB: OcScanForBootEntries failure - Not Found',
    category: 'OpenCore Boot Issues',
    fix: 'Set ScanPolicy = 0 temporarily so OpenCore can find any boot entries. Check if the drive is properly formatted and visible in BIOS.',
    severity: 'error',
  },
  {
    error: 'OCB: failed to match a default boot option',
    category: 'OpenCore Boot Issues',
    fix: 'Enable Misc → Boot → ShowPicker to see all options. Set DefaultBootEntry to your macOS entry. Make sure there is at least one valid boot entry.',
    severity: 'error',
  },
  {
    error: 'OCS: No schema for DSDT, KernelAndKextPatch, RtVariable, SMBIOS, SystemParameters',
    category: 'OpenCore Boot Issues',
    fix: 'You are using an old Clover config.plist with OpenCore. Use a fresh config.plist from the Dortania guide for your CPU generation.',
    severity: 'error',
  },
  {
    error: "OC: Driver XXX.efi at 0 cannot be found",
    category: 'OpenCore Boot Issues',
    fix: 'The .efi driver listed in config.plist → UEFI → Drivers is not present in EFI/OC/Drivers/. Add the file or remove the entry from config.plist.',
    severity: 'error',
  },
  {
    error: 'Failed to parse real field of type 1',
    category: 'OpenCore Boot Issues',
    fix: 'config.plist is malformed. A number field contains an invalid value. Use ProperTree to validate and fix. Run OC Validate.',
    severity: 'error',
  },
  {
    error: "Can't select anything in the OpenCore picker",
    category: 'OpenCore Boot Issues',
    fix: 'Set Misc → Boot → PollAppleHotKeys = True and enable KeySupport under UEFI → Input. Try PointerSupportMode if using a mouse.',
    severity: 'warning',
  },
  {
    error: 'OpenCore reboots to BIOS instead of loading',
    category: 'OpenCore Boot Issues',
    fix: 'The EFI partition is not being read. Ensure OpenCore is on a FAT32 formatted EFI partition. Re-run the EFI creation step. Check BIOS boot order.',
    details: 'Also verify that BootOrder and BootNext NVRAM entries are set correctly.',
    severity: 'error',
  },
  {
    error: 'OCABC: Incompatible OpenRuntime r4, require r10',
    category: 'OpenCore Boot Issues',
    fix: 'OpenCore.efi and OpenRuntime.efi versions do not match. Always use matching files from the same OpenCore release. Do not mix versions.',
    severity: 'error',
  },
  {
    error: 'Failed to open OpenCore image - Access Denied',
    category: 'OpenCore Boot Issues',
    fix: 'Secure Boot is blocking OpenCore. Disable Secure Boot in BIOS. Alternatively, enroll the OpenCore MOK certificate.',
    severity: 'error',
  },
  {
    error: 'Legacy boot stuck on BOOT FAIL!',
    category: 'OpenCore Boot Issues',
    fix: 'Legacy boot requires BOOTx64.efi to be properly set up. Check BootInstall.command was run correctly. Ensure BIOS is in Legacy+UEFI or Legacy mode.',
    severity: 'error',
  },
  // ─── Kernel Issues ───────────────────────────────────────────────────────
  {
    error: 'Stuck on [EB|#LOG:EXITBS:START]',
    category: 'Kernel Issues',
    fix: 'This is a booter/memory map issue. Enable DevirtualiseMmio, ProtectUefiServices, and SyncRuntimePermissions. On Intel add ProvideCustomSlide. On AMD, disable EnableWriteUnprotector.',
    details: 'The most common cause is avalanche of BooterQuirks being wrong for your platform.',
    severity: 'error',
  },
  {
    error: 'Stuck on EndRandomSeed',
    category: 'Kernel Issues',
    fix: 'ProvideConsoleGop is not enabled. Set Booter → Quirks → ProvideConsoleGop = True. Also check your GPU framebuffer patch.',
    severity: 'error',
  },
  {
    error: 'Stuck after selecting macOS in OpenCore picker',
    category: 'Kernel Issues',
    fix: 'Add -v (verbose boot) to boot-args to see the actual error. Common causes: missing kext, bad ACPI patch, or framebuffer issue.',
    details: 'Enable debug logging: add debug=0x100 and keepsyms=1 to boot-args.',
    severity: 'error',
  },
  {
    error: 'Kernel Panic on Invalid frame pointer',
    category: 'Kernel Issues',
    fix: 'Usually a Lilu plugin incompatibility or an incompatible boot-arg. Boot with -liludbgall to identify the offending kext. Ensure all kexts are the correct version for your macOS.',
    severity: 'error',
  },
  {
    error: 'OCB: LoadImage failed - Security Violation',
    category: 'Kernel Issues',
    fix: 'Firmware Secure Boot is still active or your OpenCore security settings do not match the target macOS path. Disable firmware Secure Boot and use the documented SecureBootModel for your selected macOS version.',
    severity: 'error',
  },
  {
    error: "Stuck on 'This version of Mac OS X is not supported: Reason Mac...'",
    category: 'Kernel Issues',
    fix: 'Your SMBIOS is not compatible with the target macOS version. Change your SMBIOS to one that supports the macOS version you are installing. Example: iMac19,1 for macOS Catalina.',
    severity: 'error',
  },
  {
    error: "Couldn't allocate runtime area / slide=X errors",
    category: 'Kernel Issues',
    fix: 'Enable AvoidRuntimeDefrag in Booter → Quirks. On problematic firmwares also enable DevirtualiseMmio. Add slide=X to test specific slide values.',
    details: 'Run the KASLR slide tool from Dortania: https://dortania.github.io/OpenCore-Install-Guide/extras/kaslr-fix.html',
    severity: 'error',
  },
  {
    error: 'Stuck on RTC, PCI Configuration Begins, Previous Shutdown, HPET, HID: Legacy',
    category: 'Kernel Issues',
    fix: 'These stalls indicate ACPI or PCIe initialisation issues. Add FixHPET, FixIPIC, FixRTC SSDTs or use SSDTTime. On AMD, try adding npci=0x2000 boot-arg.',
    severity: 'error',
  },
  {
    error: 'Stuck at ACPI table loading on B550',
    category: 'Kernel Issues',
    fix: 'AMD B550 requires SSDT-CPUR.aml. Download it from the Dortania Getting-Started-with-ACPI repository and place it in EFI/OC/ACPI/.',
    severity: 'error',
  },
  {
    error: '"Waiting for Root Device" or Prohibited Sign (⊘)',
    category: 'Kernel Issues',
    fix: 'USB or SATA device not recognised. Try every USB 2.0 port with the installer USB. Ensure SATA mode is AHCI in BIOS (not RAID or IDE). Add built-in property spoof to fix USB recognition.',
    severity: 'error',
  },
  {
    error: 'Kernel panic with IOPCIFamily on X99',
    category: 'Kernel Issues',
    fix: 'Add npci=0x2000 to boot-args. On some boards also set Above4GDecoding and enable DevirtualiseMmio.',
    severity: 'error',
  },
  {
    error: 'IOConsoleUsers: gIOScreenLock / gIOLockState (3) - Black screen',
    category: 'Kernel Issues',
    fix: 'GPU framebuffer not properly initialised. Ensure WhateverGreen.kext is loaded. For Navi cards (RX 5000/6000), add agdpmod=pikera to boot-args.',
    details: 'Also check that your IGPU device-id is correct if you have an Intel iGPU.',
    severity: 'error',
  },
  {
    error: 'Kernel Panic: AppleIntelMCEReporter',
    category: 'Kernel Issues',
    fix: 'Add AppleMCEReporterDisabler.kext. Required on AMD systems with macOS 12.3+ and on dual-socket Intel with macOS 10.15+.',
    severity: 'error',
  },
  {
    error: 'Kernel Panic: AppleIntelCPUPowerManagement',
    category: 'Kernel Issues',
    fix: 'CFG Lock is enabled in your BIOS. Either disable it in BIOS, or enable AppleCpuPmCfgLock and AppleXcpmCfgLock in Kernel → Quirks.',
    details: 'Disabling in BIOS is strongly preferred over using quirks.',
    severity: 'error',
  },
  {
    error: 'Samsung PM981, PM991 or Micron 2200S kernel panics on NVMe',
    category: 'Kernel Issues',
    fix: 'Add NVMeFix.kext to EFI/OC/Kexts/. These SSDs are incompatible out of the box. Note: they may still cause intermittent boot issues.',
    severity: 'error',
  },
  {
    error: 'macOS frozen right before the login screen',
    category: 'Kernel Issues',
    fix: 'This is usually a GPU/display issue. Add -v to boot-args to see the hang point. Try adding igfxonln=1 for iGPU. For AMD discrete GPUs add agdpmod=pikera.',
    severity: 'error',
  },
  {
    error: 'Keyboard works but trackpad does not on laptops',
    category: 'Kernel Issues',
    fix: 'Missing VoodooI2C or VoodooPS2 kext. Ensure the correct trackpad kext is loaded (I2C vs PS2 vs SMBus). SSDT-GPI0 may be required.',
    severity: 'warning',
  },
  {
    error: 'Kernel panic on "Wrong CD Clock Frequency" — Icelake laptop',
    category: 'Kernel Issues',
    fix: 'Add igfxrpsc=1 to boot-args. This is a known Ice Lake dGPU boot issue resolved by WhateverGreen.',
    severity: 'error',
  },
  {
    error: 'Stuck on "Forcing CS_RUNTIME for entitlement" in Big Sur',
    category: 'Kernel Issues',
    fix: 'ApECID is set but the machine has not been personalised. Set ApECID = 0 in Misc → Security, or properly run the SIP personalisation.',
    severity: 'error',
  },
  // ─── Userspace Issues ────────────────────────────────────────────────────
  {
    error: 'macOS installer crashes or restarts during installation',
    category: 'Userspace Issues',
    fix: 'Check that your target drive is formatted APFS (not HFS+). Ensure there is enough space (30+ GB). Try different USB ports — some USB 3.0 ports cause issues.',
    severity: 'error',
  },
  {
    error: 'iMessage, FaceTime, or App Store not working',
    category: 'Userspace Issues',
    fix: 'Your SMBIOS serial number may be in use or invalid. Generate new SerialNumber, MLB, and ROM values with GenSMBIOS. Enable en0 built-in property for your Ethernet.',
    details: 'The Ethernet interface MUST be set as built-in = true in DeviceProperties for iServices to work.',
    severity: 'warning',
  },
  {
    error: 'Audio not working after install',
    category: 'Userspace Issues',
    fix: 'AppleALC.kext is loaded but wrong layout-id is used. Try different alcid values: alcid=1, 2, 3, 7, 11 etc. Use IORegistryExplorer to identify your codec.',
    details: 'Check Dortania\'s AppleALC supported codec list: https://github.com/acidanthera/AppleALC/wiki/Supported-codecs',
    severity: 'warning',
  },
  {
    error: 'USB ports not working or randomly disconnecting',
    category: 'Userspace Issues',
    fix: 'You need to map USB ports post-install using USBMap or USBToolbox. macOS limits to 15 USB ports. Use the -uia_exclude_bb boot-arg during mapping.',
    details: 'Proper USB mapping is required for stable operation.',
    severity: 'warning',
  },
  {
    error: 'Sleep / Wake issues',
    category: 'Userspace Issues',
    fix: 'This has multiple causes: USB not mapped (add SSDT-GPRW or SSDT-PTLP for wake), GPU power management (add SSDT-DGPU-OFF for disabled dGPU), or use pmset to disable hibernation.',
    severity: 'warning',
  },
  {
    error: 'DRM content not playing (Netflix, Prime Video, Apple TV)',
    category: 'Userspace Issues',
    fix: "DRM requires an iGPU-compatible SMBIOS or a supported AMD GPU. On iGPU-only systems use iMac19,1 SMBIOS. Ensure shikigva boot-arg is set correctly via WhateverGreen.",
    severity: 'warning',
  },
  // ─── Post-Install Issues ─────────────────────────────────────────────────
  {
    error: 'iGPU or GPU acceleration not working (no Metal)',
    category: 'Post-Install Issues',
    fix: 'WhateverGreen.kext is required. For Intel iGPU, ensure the correct platform-id is set in DeviceProperties. For AMD, verify the correct GPU ID is injected.',
    severity: 'error',
  },
  {
    error: 'Bluetooth not working',
    category: 'Post-Install Issues',
    fix: 'Ensure BrcmPatchRAM3.kext, BrcmFirmwareData.kext, and BlueToolFixup.kext are loaded (for Big Sur+). For Intel BT, use IntelBluetoothInjector.kext + IntelBluetoothFirmware.kext.',
    severity: 'warning',
  },
  {
    error: 'CPU not running at full speed / performance',
    category: 'Post-Install Issues',
    fix: 'CPU power management is not set up. For Intel 3rd gen or older, run ssdtPRGen.sh to create CPU-PM.aml. For newer, SSDT-PLUG.aml enables XNU power management. Check CPUFriend.kext.',
    severity: 'warning',
  },
  {
    error: 'Ethernet not detected',
    category: 'Post-Install Issues',
    fix: 'Missing Ethernet kext. Identify your NIC chipset and add the correct kext: IntelMausi (Intel I217/I218/I219), RealtekRTL8111 (Realtek RTL), AtherosE2200Ethernet (Atheros/Killer).',
    severity: 'error',
  },
  {
    error: 'Second monitor not detected or no display output',
    category: 'Post-Install Issues',
    fix: 'Check WhateverGreen connector-type patches for your GPU. For Intel iGPU with HDMI/DP, set correct connector-type in framebuffer patch. For AMD RX 5000/6000, add agdpmod=pikera.',
    severity: 'warning',
  },
  // ─── Miscellaneous Issues ────────────────────────────────────────────────
  {
    error: 'Board-specific firmware issue on a 500-series AMD motherboard',
    category: 'Miscellaneous Issues',
    fix: 'Do not rely on old blanket motherboard folklore. Check the current Dortania motherboard and AMD Zen notes for your exact board and firmware revision, then verify whether a specific ACPI or BIOS workaround still applies.',
    severity: 'warning',
  },
  {
    error: 'Intel WiFi — unofficial support only',
    category: 'Miscellaneous Issues',
    fix: 'Intel WiFi uses a third-party driver (AirportItlwm / Heliport). It works on many cards but is not as stable as Broadcom. For best compatibility, replace with a supported Broadcom card.',
    details: 'Check the WiFi Buyer\'s Guide: https://dortania.github.io/Wireless-Buyers-Guide/',
    severity: 'info',
  },
  {
    error: 'No Ethernet — using Android phone for internet',
    category: 'Miscellaneous Issues',
    fix: "Use Android USB Tethering with HoRNDIS: connect your phone to WiFi, enable USB tethering, and macOS will see it as a wired connection via HoRNDIS kext.",
    details: 'HoRNDIS: https://joshuawise.com/horndis',
    severity: 'info',
  },
  {
    error: 'macOS Ventura+ fails to boot — AVX2 missing',
    category: 'Miscellaneous Issues',
    fix: 'macOS 13 (Ventura) and newer require AVX2 CPU support. Processors older than Intel Haswell (4th gen) or AMD Ryzen do not support AVX2. You must use macOS Monterey or older.',
    severity: 'error',
  },
  {
    error: 'macOS Mojave+ won\'t boot — SSE4.2 missing',
    category: 'Miscellaneous Issues',
    fix: 'macOS 10.14 (Mojave) and newer require SSE4.2. For SSE4.1-only CPUs use the telemetrap.kext workaround linked in the Dortania guide. Very old CPUs cannot run Mojave.',
    severity: 'error',
  },
  {
    error: 'Intel 12th gen (Alder Lake) performance issues',
    category: 'Miscellaneous Issues',
    fix: 'Add CPUTopologyRebuild.kext. Alder Lake\'s P-core/E-core heterogeneous architecture needs special handling. Also ensure RestrictEvents.kext is present.',
    severity: 'warning',
  },
  {
    error: 'NVRAM not saving between reboots',
    category: 'Miscellaneous Issues',
    fix: 'Enable LegacySchema in config.plist → NVRAM → LegacyOverwrite for Z390 boards (requires SSDT-PMC.aml). For other boards, ensure OpenRuntime.efi is loaded and working.',
    details: 'Z390 boards do not have native NVRAM support and require SSDT-PMC.',
    severity: 'warning',
  },
  {
    error: 'OpenCore picker extremely slow or laggy',
    category: 'Miscellaneous Issues',
    fix: 'Disable ConnectDrivers or reduce scan policy. If using OpenCanopy, ensure the resource folder is present. Reduce Timeout value. Check for slow drives being scanned.',
    severity: 'info',
  },
  {
    error: 'Intel 600p NVMe causing kernel panics',
    category: 'Miscellaneous Issues',
    fix: 'The Intel 600p NVMe drive is known to cause severe IO-related kernel panics in macOS. The only reliable fix is to replace the drive, or disable it via an SSDT if installing macOS to a different drive.',
    details: 'macOS NVMe drivers conflict with Intel 600p controller behavior.',
    severity: 'error',
  },
  {
    error: 'AMD laptop discrete GPU not working',
    category: 'Miscellaneous Issues',
    fix: 'Laptop dGPUs (both NVIDIA Optimus and AMD Switchable Graphics/PRIME) are completely unsupported in macOS since they rely on software switching not present in macOS. You must disable the dGPU in BIOS or via an SSDT (e.g. SSDT-dGPU-Off) and use only the integrated GPU.',
    severity: 'error',
  },
  {
    error: 'Intel I225-V 2.5GbE ethernet not working',
    category: 'Miscellaneous Issues',
    fix: 'The Intel I225-V network controller requires special DeviceProperties injection (device-id: F2150000) and sometimes boot-args (e1000=0) depending on macOS version. In macOS Monterey+, AppleIGC.kext is usually the best solution.',
    severity: 'warning',
  }
];
