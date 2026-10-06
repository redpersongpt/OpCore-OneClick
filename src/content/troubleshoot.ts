import type { CpuPlatform, CpuVendor, MacOsVersion } from '../bridge/types';
import type { Localized } from '../i18n/lang';

export type TroubleCategory = 'boot' | 'install' | 'hardware' | 'usb' | 'post-install';

export const TROUBLE_CATEGORIES: readonly TroubleCategory[] = ['boot', 'install', 'hardware', 'usb', 'post-install'];

export interface TroubleText {
  title: string;
  symptoms: string[];
  causes: string[];
  fixes: string[];
  advanced?: string[];
}

/** What is known about the machine, used to flag the entries that apply to it. */
export interface TroubleContext {
  vendor: CpuVendor | null;
  platform: CpuPlatform | null;
  hybrid: boolean;
  chipset: string | null;
  target: MacOsVersion | null;
}

export interface TroubleEntry {
  id: string;
  category: TroubleCategory;
  kexts?: string[];
  relevant?: (ctx: TroubleContext) => boolean;
  text: Localized<TroubleText>;
}

export const TROUBLESHOOTING: TroubleEntry[] = [
  {
    id: 'black-screen',
    category: 'boot',
    kexts: ['WhateverGreen.kext', 'Lilu.kext'],
    text: {
      en: {
        title: 'Black screen or no display after booting',
        symptoms: [
          'The screen goes black after the verbose text or the Apple logo',
          'The monitor reports no signal once macOS starts loading',
          'Verbose output stops at “IOConsoleUsers: gIOScreenLockState 3”',
        ],
        causes: [
          'The monitor is connected to an output macOS does not drive (motherboard ports with a headless iGPU, or the other way round)',
          'A GPU macOS cannot drive is still active (NVIDIA GTX 10/RTX, AMD RX 6400/6500 and RX 7000 or newer, Intel Arc)',
          'Wrong or missing iGPU framebuffer properties (AAPL,ig-platform-id, connector patches)',
          'An AMD RX 5000 / RX 6000 (Navi) card without the agdpmod=pikera boot-arg',
        ],
        fixes: [
          'Boot with -v (verbose) to see the last line before the screen goes dark',
          'Connect the monitor to the graphics card when the iGPU is headless, and to the motherboard when the iGPU drives the display',
          'Disable unsupported GPUs in the BIOS or keep the “Disable unsupported GPUs” build option on',
          'AMD RX 5000 / RX 6000 (Navi): add agdpmod=pikera (not for Polaris or Vega cards); on macOS 26 with WhateverGreen agdpmod=ignore is advised',
          'Laptops: the internal panel must be driven by the iGPU, and the backlight needs SSDT-PNLF',
        ],
        advanced: [
          'Try another connector type (DisplayPort vs HDMI); some framebuffers enable only certain connectors',
          'WhateverGreen debug boot-args such as -igfxdump help when you report the problem',
        ],
      },
      tr: {
        title: 'Açılıştan sonra siyah ekran veya görüntü yok',
        symptoms: [
          'Ayrıntılı (verbose) yazılar veya Apple logosundan sonra ekran kararıyor',
          'macOS yüklenmeye başlayınca monitör sinyal yok diyor',
          'Ayrıntılı çıktı “IOConsoleUsers: gIOScreenLockState 3” satırında duruyor',
        ],
        causes: [
          "Monitör macOS'un sürmediği bir çıkışa bağlı (iGPU başsız çalışırken anakart portları ya da tam tersi)",
          "macOS'un süremediği bir GPU hâlâ etkin (NVIDIA GTX 10/RTX, AMD RX 6400/6500 ve RX 7000 ve sonrası, Intel Arc)",
          'iGPU framebuffer özellikleri yanlış veya eksik (AAPL,ig-platform-id, konnektör yamaları)',
          'AMD RX 5000 / RX 6000 (Navi) kartta agdpmod=pikera boot-arg eksik',
        ],
        fixes: [
          'Ekran kararmadan önceki son satırı görmek için -v (verbose) ile açın',
          'iGPU başsız çalışıyorsa monitörü ekran kartına, görüntüyü iGPU veriyorsa anakarta bağlayın',
          "Desteklenmeyen GPU'ları BIOS'tan kapatın veya “Desteklenmeyen GPU'ları devre dışı bırak” derleme seçeneğini açık tutun",
          "AMD RX 5000 / RX 6000 (Navi): agdpmod=pikera ekleyin (Polaris ve Vega kartlar için değil); macOS 26'da WhateverGreen ile agdpmod=ignore önerilir",
          'Dizüstüler: dahili ekranı iGPU sürmelidir; arka ışık için SSDT-PNLF gerekir',
        ],
        advanced: [
          'Başka bir bağlantı türü deneyin (DisplayPort / HDMI); bazı framebuffer\'lar yalnızca belirli konnektörleri açar',
          '-igfxdump gibi WhateverGreen hata ayıklama argümanları sorunu bildirirken işe yarar',
        ],
      },
    },
  },
  {
    id: 'exitbs',
    category: 'boot',
    text: {
      en: {
        title: 'Stuck at [EB|#LOG:EXITBS:START] or “End RandomSeed”',
        symptoms: [
          'Verbose output stops at [EB|#LOG:EXITBS:START]',
          'The last line is “End RandomSeed” before the kernel starts',
          'The machine reboots right after choosing macOS in the picker',
        ],
        causes: [
          'Booter quirks do not suit this firmware (DevirtualiseMmio, SetupVirtualMap, RebuildAppleMemoryMap, SyncRuntimePermissions, EnableWriteUnprotector)',
          'Missing CPU patches: AMD needs the AMD_Vanilla kernel patches with the right core count',
          'The CPU platform in the profile is wrong, so the EFI was built for another generation',
          'CSM is enabled or the firmware is not in pure UEFI mode',
        ],
        fixes: [
          'Check the CPU platform in the Hardware step and rebuild; Booter quirks are chosen per platform',
          'AMD: make sure Kernel → Patch contains the AMD_Vanilla patches and the core count equals your physical cores',
          'Disable CSM, enable Above 4G decoding when available, and load the BIOS defaults before re-applying the checklist',
          'Update the BIOS; old firmware is a common cause on some boards',
        ],
        advanced: [
          'On some boards DevirtualiseMmio needs MmioWhitelist entries; see the Dortania guide for your platform',
          'The DEBUG OpenCore build (build option) writes a log to the EFI partition that shows the last step reached',
        ],
      },
      tr: {
        title: '[EB|#LOG:EXITBS:START] veya “End RandomSeed” satırında takılma',
        symptoms: [
          'Ayrıntılı çıktı [EB|#LOG:EXITBS:START] satırında duruyor',
          'Çekirdek başlamadan önceki son satır “End RandomSeed”',
          'Menüden macOS seçildikten hemen sonra bilgisayar yeniden başlıyor',
        ],
        causes: [
          "Booter ayarları bu firmware'e uymuyor (DevirtualiseMmio, SetupVirtualMap, RebuildAppleMemoryMap, SyncRuntimePermissions, EnableWriteUnprotector)",
          'İşlemci yamaları eksik: AMD için doğru çekirdek sayılı AMD_Vanilla yamaları gerekir',
          'Profildeki işlemci platformu yanlış, bu yüzden EFI başka bir nesil için derlenmiş',
          "CSM açık veya firmware saf UEFI modunda değil",
        ],
        fixes: [
          'Donanım adımında işlemci platformunu kontrol edip yeniden derleyin; Booter ayarları platforma göre seçilir',
          'AMD: Kernel → Patch bölümünde AMD_Vanilla yamalarının olduğundan ve çekirdek sayısının fiziksel çekirdek sayınıza eşit olduğundan emin olun',
          "CSM'yi kapatın, varsa Above 4G decoding'i açın ve kontrol listesini yeniden uygulamadan önce BIOS varsayılanlarını yükleyin",
          "BIOS'u güncelleyin; bazı kartlarda eski firmware sık görülen bir nedendir",
        ],
        advanced: [
          'Bazı kartlarda DevirtualiseMmio için MmioWhitelist girdileri gerekir; platformunuzun Dortania rehberine bakın',
          'DEBUG OpenCore derlemesi (derleme seçeneği) EFI bölümüne ulaşılan son adımı gösteren bir günlük yazar',
        ],
      },
    },
  },
  {
    id: 'pci-config',
    category: 'boot',
    text: {
      en: {
        title: 'Stuck at “PCI Configuration Begin”',
        symptoms: ['Verbose output stops at “[ PCI configuration begin ]”', 'The boot hangs for minutes without further output'],
        causes: [
          'PCI resource allocation fails (Above 4G decoding setting)',
          'IRQ conflicts on older laptops and desktops',
          'An unsupported storage controller or NVMe drive',
          'NVRAM or RTC problems (AWAC clock on 300-series and newer Intel chipsets)',
        ],
        fixes: [
          'Enable Above 4G decoding; if the BIOS has no such option add npci=0x2000 (or npci=0x3000) to the boot-args',
          'Remove or disable extra PCIe cards and drives during installation',
          'Reset NVRAM from the OpenCore picker',
          '300-series and newer Intel boards need SSDT-AWAC or SSDT-RTC0; check the SSDT list on the Review step',
        ],
      },
      tr: {
        title: '“PCI Configuration Begin” satırında takılma',
        symptoms: ['Ayrıntılı çıktı “[ PCI configuration begin ]” satırında duruyor', 'Açılış dakikalarca yeni bir çıktı vermeden bekliyor'],
        causes: [
          'PCI kaynak ataması başarısız oluyor (Above 4G decoding ayarı)',
          'Eski dizüstü ve masaüstülerde IRQ çakışmaları',
          'Desteklenmeyen bir depolama denetleyicisi veya NVMe disk',
          'NVRAM veya RTC sorunları (300 serisi ve sonrası Intel yonga setlerinde AWAC saati)',
        ],
        fixes: [
          "Above 4G decoding'i açın; BIOS'ta böyle bir seçenek yoksa boot-args'a npci=0x2000 (veya npci=0x3000) ekleyin",
          'Kurulum sırasında ek PCIe kartlarını ve diskleri çıkarın veya devre dışı bırakın',
          "OpenCore menüsünden NVRAM'i sıfırlayın",
          '300 serisi ve sonrası Intel kartlar SSDT-AWAC veya SSDT-RTC0 ister; İnceleme adımındaki SSDT listesini kontrol edin',
        ],
      },
    },
  },
  {
    id: 'root-device',
    category: 'install',
    kexts: ['XHCI-unsupported.kext', 'USBToolBox.kext'],
    text: {
      en: {
        title: '“Waiting for Root Device” or a prohibited sign',
        symptoms: ['Verbose output ends with “Still waiting for root device”', 'A crossed-out circle appears instead of the installer'],
        causes: [
          'macOS cannot see the USB controller the installer drive is plugged into',
          'More than 15 ports on a controller without a port map, or XHCI hand-off disabled',
          'Storage in RAID / Intel RST / VMD mode instead of AHCI',
          'A chipset without native XHCI support (for example H310, B360, H370, X79, X99, X299) without XHCI-unsupported.kext',
        ],
        fixes: [
          'Plug the installer into a USB 2.0 port or another USB 3 port on the back panel',
          'Enable XHCI hand-off and set the SATA mode to AHCI (disable Intel VMD) in the BIOS',
          'Map your USB ports with USBToolBox on Windows before installing, or keep XhciPortLimit on only until the map is done',
          'macOS 26: a UTBMap.kext needs USBToolBox.kext 1.2.0 or newer; a native USBMap.kext needs the new per-port keys (usb-port-number / usb-port-type)',
        ],
      },
      tr: {
        title: '“Waiting for Root Device” veya yasak işareti',
        symptoms: ['Ayrıntılı çıktı “Still waiting for root device” ile bitiyor', 'Yükleyici yerine üzeri çizili bir daire görünüyor'],
        causes: [
          "macOS, yükleyici belleğin takılı olduğu USB denetleyicisini göremiyor",
          'Port haritası olmadan bir denetleyicide 15\'ten fazla port var veya XHCI hand-off kapalı',
          'Depolama AHCI yerine RAID / Intel RST / VMD modunda',
          'Yerel XHCI desteği olmayan bir yonga seti (ör. H310, B360, H370, X79, X99, X299) ve XHCI-unsupported.kext eksik',
        ],
        fixes: [
          'Yükleyiciyi arka paneldeki bir USB 2.0 portuna veya başka bir USB 3 portuna takın',
          "BIOS'ta XHCI hand-off'u açın ve SATA modunu AHCI yapın (Intel VMD'yi kapatın)",
          "Kurulumdan önce Windows'ta USBToolBox ile portlarınızı eşleyin veya harita hazır olana kadar yalnızca XhciPortLimit'i açık tutun",
          'macOS 26: UTBMap.kext için USBToolBox.kext 1.2.0 veya daha yenisi gerekir; yerel bir USBMap.kext yeni port anahtarlarını (usb-port-number / usb-port-type) içermelidir',
        ],
      },
    },
  },
  {
    id: 'security-violation',
    category: 'boot',
    text: {
      en: {
        title: 'OCB: LoadImage failed - Security Violation',
        symptoms: [
          '“OCB: LoadImage failed - Security Violation” after choosing macOS',
          'The picker comes back or freezes after selecting the installer',
        ],
        causes: [
          'Apple Secure Boot (SecureBootModel) is enabled but the Preboot volume lacks matching Secure Boot manifests',
          'Stale NVRAM from an earlier configuration',
          'ApECID is set while installing',
        ],
        fixes: [
          'Set Misc → Security → SecureBootModel to Disabled; for macOS 14.4 and newer the Review step should already show Disabled — rebuild if you changed the target',
          'Reset NVRAM from the OpenCore picker and try again',
          'Keep ApECID at 0 until macOS is installed and updated',
        ],
      },
      tr: {
        title: 'OCB: LoadImage failed - Security Violation',
        symptoms: [
          "macOS seçildikten sonra “OCB: LoadImage failed - Security Violation” hatası",
          'Yükleyici seçildikten sonra menü geri geliyor veya donuyor',
        ],
        causes: [
          'Apple Secure Boot (SecureBootModel) açık ama Preboot biriminde uyumlu Secure Boot manifestleri yok',
          "Önceki bir yapılandırmadan kalan eski NVRAM",
          'Kurulum sırasında ApECID ayarlı',
        ],
        fixes: [
          "Misc → Security → SecureBootModel'i Disabled yapın; macOS 14.4 ve sonrası için İnceleme adımında zaten Disabled görünmelidir — hedefi değiştirdiyseniz yeniden derleyin",
          "OpenCore menüsünden NVRAM'i sıfırlayıp yeniden deneyin",
          "macOS kurulup güncellenene kadar ApECID'yi 0 bırakın",
        ],
      },
    },
  },
  {
    id: 'memory-allocation',
    category: 'boot',
    text: {
      en: {
        title: 'Memory allocation errors (OCABC)',
        symptoms: [
          '“OCABC: Memory pool allocation failure - Not Found”',
          '“Couldn\'t allocate runtime area” or “Error allocating 0x… pages”',
          '“OCABC: Only N/256 slide values are usable!”',
        ],
        causes: [
          'The firmware leaves too little contiguous memory for the macOS kernel',
          'Booter quirks that do not match the board (ProvideCustomSlide, AvoidRuntimeDefrag, SetupVirtualMap)',
          'CSM enabled, or Above 4G / Resizable BAR settings the firmware handles badly',
        ],
        fixes: [
          'Keep ProvideCustomSlide and AvoidRuntimeDefrag enabled (Booter → Quirks)',
          'Disable CSM and load the optimised BIOS defaults, then re-apply the checklist',
          'Enable Above 4G decoding; with Resizable BAR enabled either disable it in the BIOS or set ResizeAppleGpuBars to 0',
          'Update or downgrade the BIOS; some versions have a fragmented memory map',
        ],
        advanced: ['As a last resort calculate a working slide value from the OpenCore memory map and add slide=N to the boot-args'],
      },
      tr: {
        title: 'Bellek ayırma hataları (OCABC)',
        symptoms: [
          '“OCABC: Memory pool allocation failure - Not Found”',
          '“Couldn\'t allocate runtime area” veya “Error allocating 0x… pages”',
          '“OCABC: Only N/256 slide values are usable!”',
        ],
        causes: [
          'Firmware, macOS çekirdeği için yeterli bitişik bellek bırakmıyor',
          'Karta uymayan Booter ayarları (ProvideCustomSlide, AvoidRuntimeDefrag, SetupVirtualMap)',
          "CSM açık veya firmware'in iyi işleyemediği Above 4G / Resizable BAR ayarları",
        ],
        fixes: [
          "ProvideCustomSlide ve AvoidRuntimeDefrag'i açık tutun (Booter → Quirks)",
          "CSM'yi kapatın ve optimize BIOS varsayılanlarını yükleyip kontrol listesini yeniden uygulayın",
          "Above 4G decoding'i açın; Resizable BAR açıksa BIOS'tan kapatın veya ResizeAppleGpuBars'ı 0 yapın",
          "BIOS'u güncelleyin veya eski sürüme dönün; bazı sürümlerin bellek haritası parçalıdır",
        ],
        advanced: ["Son çare olarak OpenCore bellek haritasından çalışan bir slide değeri hesaplayıp boot-args'a slide=N ekleyin"],
      },
    },
  },
  {
    id: 'cfg-lock',
    category: 'boot',
    relevant: (ctx) => ctx.vendor === 'intel',
    text: {
      en: {
        title: 'Early reboot or panic caused by CFG Lock',
        symptoms: ['Reboots or panics very early in the boot', 'The panic mentions xcpm, MSR 0xE2 or AppleIntelCPUPowerManagement'],
        causes: ['The firmware locks MSR 0xE2 (CFG Lock) and macOS tries to write it', 'The CFG Lock option is hidden in the BIOS'],
        fixes: [
          'Disable CFG Lock in the BIOS (often under CPU or power management settings)',
          'If it cannot be disabled, keep Kernel → Quirks → AppleXcpmCfgLock (Haswell and newer) or AppleCpuPmCfgLock (Ivy Bridge and older) enabled',
          'Use the ControlMsrE2 tool shipped with OpenCore to check whether CFG Lock is really off',
        ],
      },
      tr: {
        title: 'CFG Lock kaynaklı erken yeniden başlama veya panik',
        symptoms: ['Açılışın çok başında yeniden başlama veya kernel panic', 'Panik mesajında xcpm, MSR 0xE2 veya AppleIntelCPUPowerManagement geçiyor'],
        causes: ["Firmware MSR 0xE2'yi (CFG Lock) kilitliyor ve macOS ona yazmaya çalışıyor", "CFG Lock seçeneği BIOS'ta gizli"],
        fixes: [
          "BIOS'ta CFG Lock'u kapatın (genellikle CPU veya güç yönetimi ayarlarında)",
          'Kapatılamıyorsa Kernel → Quirks → AppleXcpmCfgLock (Haswell ve sonrası) veya AppleCpuPmCfgLock (Ivy Bridge ve öncesi) açık kalsın',
          "CFG Lock'un gerçekten kapalı olduğunu OpenCore ile gelen ControlMsrE2 aracıyla kontrol edin",
        ],
      },
    },
  },
  {
    id: 'kernel-panic',
    category: 'boot',
    text: {
      en: {
        title: 'Kernel panic during boot',
        symptoms: ['Text about a “panic” fills the screen and the machine reboots', 'The panic log names a kext in its backtrace'],
        causes: [
          'A kext that does not match the macOS version or the hardware',
          'Missing AMD_Vanilla patches or a wrong core count on AMD',
          'Hybrid Intel CPUs (12th gen and newer) without CPU spoofing',
          'A wrong CPU platform or SMBIOS model in the profile',
        ],
        fixes: [
          'Boot with -v keepsyms=1 debug=0x100 so the panic stays on screen, and note the kext named in the backtrace',
          'Check the CPU platform and core count in the Hardware step and rebuild',
          'Remove optional kexts one at a time to find the culprit',
          'Rebuild with the tested release set (turn “Use newest releases” off)',
        ],
        advanced: ['Hybrid CPUs: Kernel → Quirks → ProvideCurrentCpuInfo must be on and Kernel → Emulate must spoof a supported CPU'],
      },
      tr: {
        title: 'Açılışta kernel panic',
        symptoms: ['Ekranı “panic” içeren yazılar kaplıyor ve bilgisayar yeniden başlıyor', 'Panik kaydındaki geri izlemede bir kext adı geçiyor'],
        causes: [
          'macOS sürümüne veya donanıma uymayan bir kext',
          'AMD\'de eksik AMD_Vanilla yamaları veya yanlış çekirdek sayısı',
          'İşlemci taklidi (spoof) yapılmamış hibrit Intel işlemciler (12. nesil ve sonrası)',
          'Profilde yanlış işlemci platformu veya SMBIOS modeli',
        ],
        fixes: [
          'Panik ekranda kalsın diye -v keepsyms=1 debug=0x100 ile açın ve geri izlemede geçen kext\'i not edin',
          'Donanım adımında işlemci platformunu ve çekirdek sayısını kontrol edip yeniden derleyin',
          "Sorunu bulmak için isteğe bağlı kext'leri tek tek kaldırın",
          'Test edilmiş sürüm setiyle yeniden derleyin (“En yeni sürümleri kullan” kapalı)',
        ],
        advanced: ['Hibrit işlemciler: Kernel → Quirks → ProvideCurrentCpuInfo açık olmalı ve Kernel → Emulate desteklenen bir işlemciyi taklit etmelidir'],
      },
    },
  },
  {
    id: 'alder-lake',
    category: 'boot',
    kexts: ['CPUTopologyRebuild.kext'],
    relevant: (ctx) => ctx.hybrid || ctx.platform === 'alder_lake' || ctx.platform === 'raptor_lake',
    text: {
      en: {
        title: 'Alder Lake / Raptor Lake: E-cores and hybrid CPUs',
        symptoms: [
          'Panics or hangs on 12th, 13th or 14th gen Intel CPUs',
          'Only some cores show up, or the system feels very slow',
          'No graphics acceleration with the integrated UHD 7xx graphics',
        ],
        causes: ['macOS has no native support for hybrid P-core/E-core CPUs', 'The UHD 730/770 iGPU has no macOS driver'],
        fixes: [
          'The EFI must spoof the CPU (Kernel → Emulate Cpuid1Data/Cpuid1Mask) and enable ProvideCurrentCpuInfo',
          'Add CPUTopologyRebuild.kext to use the E-cores efficiently, or disable the E-cores in the BIOS if your board allows it',
          'Use a supported AMD graphics card (for example RX 580 or RX 6600); the integrated graphics cannot be used',
        ],
      },
      tr: {
        title: 'Alder Lake / Raptor Lake: E-çekirdekler ve hibrit işlemciler',
        symptoms: [
          '12., 13. veya 14. nesil Intel işlemcilerde panik veya donma',
          'Çekirdeklerin yalnızca bir kısmı görünüyor veya sistem çok yavaş',
          'Dahili UHD 7xx grafikte donanım hızlandırma yok',
        ],
        causes: ['macOS hibrit P/E çekirdekli işlemcileri yerel olarak desteklemiyor', 'UHD 730/770 iGPU için macOS sürücüsü yok'],
        fixes: [
          "EFI işlemciyi taklit etmeli (Kernel → Emulate Cpuid1Data/Cpuid1Mask) ve ProvideCurrentCpuInfo'yu açmalıdır",
          "E-çekirdekleri verimli kullanmak için CPUTopologyRebuild.kext ekleyin veya kartınız izin veriyorsa E-çekirdekleri BIOS'tan kapatın",
          'Desteklenen bir AMD ekran kartı kullanın (ör. RX 580 veya RX 6600); dahili grafik kullanılamaz',
        ],
      },
    },
  },
  {
    id: 'amd-cpur',
    category: 'boot',
    relevant: (ctx) => ctx.vendor === 'amd',
    text: {
      en: {
        title: 'AMD B550 / A520 boards: SSDT-CPUR',
        symptoms: ['Hangs or panics early in boot on B550 or A520 boards', 'Booting stopped working after a BIOS update on a B550/A520 board'],
        causes: ['These boards declare the CPU cores as ACPI0007 devices that macOS does not recognise without SSDT-CPUR'],
        fixes: [
          'Make sure SSDT-CPUR.aml is listed on the Review step and enabled in ACPI → Add',
          'If it is missing, check that the chipset in the Hardware step is correct and rebuild',
          'Keep the AMD_Vanilla patches with the matching core count',
        ],
      },
      tr: {
        title: 'AMD B550 / A520 kartlar: SSDT-CPUR',
        symptoms: ['B550 veya A520 kartlarda açılışın başında donma veya panik', 'B550/A520 kartta BIOS güncellemesinden sonra açılış durdu'],
        causes: ["Bu kartlar işlemci çekirdeklerini macOS'un SSDT-CPUR olmadan tanımadığı ACPI0007 aygıtları olarak tanımlar"],
        fixes: [
          "SSDT-CPUR.aml'in İnceleme adımında listelendiğinden ve ACPI → Add içinde açık olduğundan emin olun",
          'Eksikse Donanım adımındaki yonga setinin doğru olduğunu kontrol edip yeniden derleyin',
          'AMD_Vanilla yamalarını doğru çekirdek sayısıyla tutun',
        ],
      },
    },
  },
  {
    id: 'recovery-server',
    category: 'install',
    text: {
      en: {
        title: '“The recovery server could not be contacted”',
        symptoms: [
          'The installer stops with “The recovery server could not be contacted”',
          '“This copy of the Install macOS application is damaged”',
          'The macOS download fails partway',
        ],
        causes: [
          'The clock in recovery is wrong, so Apple\'s certificates look invalid',
          'No working network adapter in recovery (Intel Wi-Fi with itlwm does not work there; Ethernet kext missing)',
          'Apple\'s servers are temporarily refusing requests',
        ],
        fixes: [
          'Open Terminal from the Utilities menu and check the time with “date”; set it with date MMDDhhmmYYYY, for example “date 100512302026”',
          'Use a wired Ethernet connection and check that the Ethernet kext is in the EFI (Review step)',
          'Try again later or on another network',
        ],
      },
      tr: {
        title: '“The recovery server could not be contacted” hatası',
        symptoms: [
          'Yükleyici “The recovery server could not be contacted” hatasıyla duruyor',
          '“This copy of the Install macOS application is damaged” hatası',
          'macOS indirmesi yarıda kesiliyor',
        ],
        causes: [
          "Kurtarma ortamında saat yanlış, bu yüzden Apple'ın sertifikaları geçersiz görünüyor",
          'Kurtarmada çalışan ağ bağdaştırıcısı yok (itlwm ile Intel Wi-Fi orada çalışmaz; Ethernet kext\'i eksik)',
          "Apple'ın sunucuları istekleri geçici olarak reddediyor",
        ],
        fixes: [
          "İzlenceler menüsünden Terminal'i açın ve saati “date” ile kontrol edin; date AAGGssddYYYY biçimiyle ayarlayın, ör. “date 100512302026”",
          "Kablolu Ethernet kullanın ve Ethernet kext'inin EFI'de olduğunu kontrol edin (İnceleme adımı)",
          'Daha sonra veya başka bir ağda yeniden deneyin',
        ],
      },
    },
  },
  {
    id: 'tahoe',
    category: 'post-install',
    relevant: (ctx) => ctx.target === '26',
    text: {
      en: {
        title: 'macOS 26 Tahoe: known limitations',
        symptoms: [
          'No analog audio (speakers, headphone jack)',
          'No Wi-Fi menu with an Intel card',
          'Intel Bluetooth stops working',
          'A USB Wi-Fi or Bluetooth dongle no longer works',
          'A FileVault volume cannot be unlocked at boot',
        ],
        causes: [
          'Apple removed AppleHDA.kext in macOS 26, so AppleALC alone cannot enable onboard audio',
          'AirportItlwm has no official build for macOS 15 or 26; only itlwm with HeliPort works',
          'IntelBluetoothFirmware 2.4.0 predates Sequoia and Tahoe',
          'IOUSBFamily.kext was removed, which breaks many USB dongles',
          'The Tahoe APFS driver cannot unlock FileVault volumes on OpenCore systems',
        ],
        fixes: [
          'Audio: use VoodooHDA, USB audio, HDMI/DisplayPort audio from an AMD graphics card, or re-install AppleHDA with a root patch (lost on every update)',
          'Intel Wi-Fi: use itlwm with the HeliPort app',
          'Intel Bluetooth: add -ibtcompatbeta to the boot-args',
          'FileVault: load apfs_aligned.efi from macOS 15 and set UEFI → APFS → EnableJumpstart to false, or leave FileVault off',
          'If you depend on any of these, install macOS 15 Sequoia instead',
        ],
      },
      tr: {
        title: 'macOS 26 Tahoe: bilinen kısıtlamalar',
        symptoms: [
          'Analog ses yok (hoparlör, kulaklık girişi)',
          'Intel kartla Wi-Fi menüsü yok',
          'Intel Bluetooth çalışmıyor',
          'USB Wi-Fi veya Bluetooth adaptörü artık çalışmıyor',
          'FileVault birimi açılışta kilidi açılamıyor',
        ],
        causes: [
          "Apple macOS 26'da AppleHDA.kext'i kaldırdı; AppleALC tek başına dahili sesi açamıyor",
          "AirportItlwm'in macOS 15 veya 26 için resmi derlemesi yok; yalnızca itlwm + HeliPort çalışıyor",
          "IntelBluetoothFirmware 2.4.0, Sequoia ve Tahoe'dan eski",
          "IOUSBFamily.kext kaldırıldı; bu birçok USB adaptörünü bozuyor",
          "Tahoe APFS sürücüsü OpenCore sistemlerde FileVault birimlerinin kilidini açamıyor",
        ],
        fixes: [
          'Ses: VoodooHDA, USB ses, AMD ekran kartından HDMI/DisplayPort sesi kullanın veya AppleHDA\'yı root yamasıyla geri yükleyin (her güncellemede kaybolur)',
          'Intel Wi-Fi: itlwm ile HeliPort uygulamasını kullanın',
          "Intel Bluetooth: boot-args'a -ibtcompatbeta ekleyin",
          "FileVault: macOS 15'teki apfs_aligned.efi'yi yükleyin ve UEFI → APFS → EnableJumpstart'ı false yapın ya da FileVault'u kapalı bırakın",
          "Bunlardan birine ihtiyacınız varsa bunun yerine macOS 15 Sequoia kurun",
        ],
      },
    },
  },
  {
    id: 'no-wifi',
    category: 'hardware',
    kexts: ['itlwm.kext', 'AirportItlwm.kext', 'AirportBrcmFixup.kext'],
    text: {
      en: {
        title: 'Wi-Fi does not work',
        symptoms: ['No Wi-Fi in System Settings', 'The card is not detected', 'Networks are visible but connecting fails'],
        causes: [
          'Intel cards need the OpenIntelWireless kexts; macOS has no Intel Wi-Fi driver',
          'Native Broadcom cards (BCM94360, BCM43602, BCM4350) lost built-in support in macOS 14 Sonoma',
          'Current Qualcomm/Atheros, Realtek and MediaTek cards have no macOS driver',
        ],
        fixes: [
          'Intel: itlwm with the HeliPort app works up to macOS 26; AirportItlwm (native menu) only up to macOS 14',
          'Broadcom on macOS 14 and newer: OpenCore Legacy Patcher root patches, or AppleBCMWLANCompanion for BCM43602/BCM4350 on macOS 15/26 (needs VT-d, so not on AMD)',
          'Unsupported cards: use Ethernet, a supported USB adapter, or replace the card — on macOS 14 and newer even a Broadcom card needs root patches',
          'Check System Information → PCI / USB to confirm that macOS sees the card',
        ],
      },
      tr: {
        title: 'Wi-Fi çalışmıyor',
        symptoms: ['Sistem Ayarları\'nda Wi-Fi yok', 'Kart algılanmıyor', 'Ağlar görünüyor ama bağlanılamıyor'],
        causes: [
          "Intel kartlar OpenIntelWireless kext'lerine ihtiyaç duyar; macOS'ta Intel Wi-Fi sürücüsü yok",
          "Yerel Broadcom kartlar (BCM94360, BCM43602, BCM4350) macOS 14 Sonoma'da yerleşik desteği kaybetti",
          'Güncel Qualcomm/Atheros, Realtek ve MediaTek kartlar için macOS sürücüsü yok',
        ],
        fixes: [
          "Intel: itlwm + HeliPort macOS 26'ya kadar çalışır; AirportItlwm (yerel menü) yalnızca macOS 14'e kadar",
          "macOS 14 ve sonrasında Broadcom: OpenCore Legacy Patcher root yamaları veya macOS 15/26'da BCM43602/BCM4350 için AppleBCMWLANCompanion (VT-d gerekir, AMD'de olmaz)",
          'Desteklenmeyen kartlar: Ethernet, desteklenen bir USB adaptörü kullanın veya kartı değiştirin — macOS 14 ve sonrasında Broadcom kart bile root yaması ister',
          "macOS'un kartı gördüğünü Sistem Bilgisi → PCI / USB bölümünden doğrulayın",
        ],
      },
    },
  },
  {
    id: 'no-audio',
    category: 'hardware',
    kexts: ['AppleALC.kext', 'Lilu.kext'],
    text: {
      en: {
        title: 'No audio output',
        symptoms: ['No output devices in Sound settings', 'The device is listed but stays silent', 'Only HDMI/DisplayPort audio works'],
        causes: [
          'The AppleALC layout-id does not match the board\'s wiring',
          'The codec is not detected or not supported by AppleALC',
          'macOS 26 Tahoe removed AppleHDA, so onboard analog audio does not work there',
          'Intel SST / SoundWire audio on newer laptops is not supported',
        ],
        fixes: [
          'Check the codec in the Hardware step, enter another value in the Layout ID field and rebuild; for quick tests boot with alcid=N',
          'Make sure AppleALC.kext and Lilu.kext are enabled on the Review step',
          'macOS 26: see the Tahoe entry (VoodooHDA, USB audio or a root patch)',
          'Restart the audio service with “sudo killall coreaudiod”',
        ],
      },
      tr: {
        title: 'Ses çıkışı yok',
        symptoms: ['Ses ayarlarında çıkış aygıtı yok', 'Aygıt listede ama ses gelmiyor', 'Yalnızca HDMI/DisplayPort sesi çalışıyor'],
        causes: [
          "AppleALC layout-id kartın kablolamasına uymuyor",
          'Codec algılanmıyor veya AppleALC tarafından desteklenmiyor',
          "macOS 26 Tahoe AppleHDA'yı kaldırdı; orada dahili analog ses çalışmaz",
          'Yeni dizüstülerdeki Intel SST / SoundWire ses desteklenmiyor',
        ],
        fixes: [
          'Donanım adımında codec\'i kontrol edin, Layout ID alanına başka bir değer girip yeniden derleyin; hızlı deneme için alcid=N ile açın',
          "AppleALC.kext ve Lilu.kext'in İnceleme adımında açık olduğundan emin olun",
          'macOS 26: Tahoe maddesine bakın (VoodooHDA, USB ses veya root yaması)',
          'Ses servisini “sudo killall coreaudiod” ile yeniden başlatın',
        ],
      },
    },
  },
  {
    id: 'no-bluetooth',
    category: 'hardware',
    kexts: ['IntelBluetoothFirmware.kext', 'IntelBTPatcher.kext', 'BlueToolFixup.kext', 'BrcmPatchRAM3.kext'],
    text: {
      en: {
        title: 'Bluetooth does not work',
        symptoms: ['No Bluetooth in System Settings', 'Devices pair but keep disconnecting'],
        causes: [
          'Intel Bluetooth needs IntelBluetoothFirmware together with IntelBTPatcher on macOS 12 and newer',
          'Broadcom Bluetooth needs BrcmPatchRAM3 with BrcmFirmwareData',
          'BlueToolFixup is required on macOS 12 and newer',
          'The internal USB port of the Bluetooth module is not in the USB map',
        ],
        fixes: [
          'Intel: IntelBluetoothFirmware + IntelBTPatcher + BlueToolFixup; on macOS 15 and 26 add -ibtcompatbeta to the boot-args',
          'Broadcom: BrcmPatchRAM3 + BrcmFirmwareData + BlueToolFixup',
          'Include the internal Bluetooth port in your USB map',
          'USB dongles: most CSR and Realtek dongles do not work; Broadcom BCM20702-based ones work with BrcmPatchRAM',
        ],
      },
      tr: {
        title: 'Bluetooth çalışmıyor',
        symptoms: ["Sistem Ayarları'nda Bluetooth yok", 'Aygıtlar eşleşiyor ama bağlantı sürekli kopuyor'],
        causes: [
          'Intel Bluetooth, macOS 12 ve sonrasında IntelBluetoothFirmware ile birlikte IntelBTPatcher ister',
          'Broadcom Bluetooth, BrcmFirmwareData ile BrcmPatchRAM3 ister',
          'macOS 12 ve sonrasında BlueToolFixup gereklidir',
          'Bluetooth modülünün dahili USB portu USB haritasında yok',
        ],
        fixes: [
          "Intel: IntelBluetoothFirmware + IntelBTPatcher + BlueToolFixup; macOS 15 ve 26'da boot-args'a -ibtcompatbeta ekleyin",
          'Broadcom: BrcmPatchRAM3 + BrcmFirmwareData + BlueToolFixup',
          'Dahili Bluetooth portunu USB haritanıza ekleyin',
          'USB adaptörler: çoğu CSR ve Realtek adaptör çalışmaz; Broadcom BCM20702 tabanlılar BrcmPatchRAM ile çalışır',
        ],
      },
    },
  },
  {
    id: 'usb',
    category: 'usb',
    kexts: ['USBToolBox.kext', 'UTBMap.kext', 'XHCI-unsupported.kext'],
    text: {
      en: {
        title: 'USB ports missing or unreliable',
        symptoms: [
          'Some ports do not work',
          'USB 3 devices run at USB 2 speed',
          'Devices disconnect randomly',
          'Keyboard or mouse stop working during boot',
        ],
        causes: [
          'macOS allows at most 15 ports per controller and no port map was made',
          'Controllers without native support need XHCI-unsupported.kext (for example H310, B360, H370, X79, X99, X299 and some ASRock boards)',
          'XHCI hand-off is disabled in the BIOS',
        ],
        fixes: [
          'Map the ports: USBToolBox on Windows works before installing and produces UTBMap.kext',
          'Use XhciPortLimit only until the map is done (it was re-patched for macOS 26 in OpenCore 1.0.7)',
          'Enable XHCI hand-off in the BIOS',
          'USBInjectAll is deprecated for modern chipsets; remove it once you have a map',
        ],
        advanced: [
          'Keep the internal Bluetooth port and the ports you actually use; drop the rest to stay under 15',
          'macOS 26: USBToolBox maps (UTBMap.kext) work unchanged with USBToolBox.kext 1.2.0 or newer; a native USBMap.kext needs both the old and the new port keys',
        ],
      },
      tr: {
        title: 'USB portları eksik veya kararsız',
        symptoms: [
          'Bazı portlar çalışmıyor',
          'USB 3 aygıtlar USB 2 hızında çalışıyor',
          'Aygıtların bağlantısı rastgele kopuyor',
          'Açılış sırasında klavye veya fare çalışmayı bırakıyor',
        ],
        causes: [
          'macOS denetleyici başına en fazla 15 portu kabul eder ve port haritası yapılmamış',
          'Yerel desteği olmayan denetleyiciler XHCI-unsupported.kext ister (ör. H310, B360, H370, X79, X99, X299 ve bazı ASRock kartlar)',
          "BIOS'ta XHCI hand-off kapalı",
        ],
        fixes: [
          "Portları eşleyin: Windows'taki USBToolBox kurulumdan önce çalışır ve UTBMap.kext üretir",
          "XhciPortLimit'i yalnızca harita hazır olana kadar kullanın (OpenCore 1.0.7'de macOS 26 için yeniden yamalandı)",
          "BIOS'ta XHCI hand-off'u açın",
          'USBInjectAll modern yonga setlerinde artık önerilmiyor; haritanız olunca kaldırın',
        ],
        advanced: [
          '15 sınırının altında kalmak için dahili Bluetooth portunu ve gerçekten kullandığınız portları tutun, gerisini çıkarın',
          'macOS 26: USBToolBox haritaları (UTBMap.kext), USBToolBox.kext 1.2.0 veya daha yenisiyle değişmeden çalışır; yerel bir USBMap.kext hem eski hem yeni port anahtarlarını içermelidir',
        ],
      },
    },
  },
  {
    id: 'storage',
    category: 'hardware',
    kexts: ['NVMeFix.kext'],
    text: {
      en: {
        title: 'NVMe and storage problems',
        symptoms: ['The drive is missing in the installer', 'Kernel panic mentioning IONVMeFamily', 'Random freezes or very slow disk access'],
        causes: [
          'Incompatible controllers: Samsung PM981/PM991 and Micron 2200S (and drives built on them)',
          'Intel 600p drives are unreliable; Optane Memory and H10/H20 hybrid drives are unsupported',
          'The SATA controller is in RAID / Intel RST / VMD mode',
        ],
        fixes: [
          'Install macOS on another drive when yours is a PM981, PM991 or 2200S; NVMeFix does not make them safe',
          'NVMeFix.kext improves power management on supported drives',
          'Set the SATA mode to AHCI and disable Intel VMD in the BIOS',
          'Remove Optane drives or disable Optane acceleration',
        ],
      },
      tr: {
        title: 'NVMe ve depolama sorunları',
        symptoms: ['Disk yükleyicide görünmüyor', 'IONVMeFamily içeren kernel panic', 'Rastgele donmalar veya çok yavaş disk erişimi'],
        causes: [
          'Uyumsuz denetleyiciler: Samsung PM981/PM991 ve Micron 2200S (ve bunlara dayanan diskler)',
          'Intel 600p diskler güvenilmez; Optane Memory ve H10/H20 hibrit diskler desteklenmez',
          'SATA denetleyicisi RAID / Intel RST / VMD modunda',
        ],
        fixes: [
          "Diskiniz PM981, PM991 veya 2200S ise macOS'u başka bir diske kurun; NVMeFix onları güvenli yapmaz",
          'NVMeFix.kext desteklenen disklerde güç yönetimini iyileştirir',
          "BIOS'ta SATA modunu AHCI yapın ve Intel VMD'yi kapatın",
          'Optane diskleri çıkarın veya Optane hızlandırmayı kapatın',
        ],
      },
    },
  },
  {
    id: 'iservices',
    category: 'post-install',
    text: {
      en: {
        title: 'iMessage, FaceTime or App Store sign-in fails',
        symptoms: ['Messages or FaceTime say they could not sign in', 'App Store downloads fail'],
        causes: [
          'Serial, MLB or ROM missing, duplicated, or already used by a real Mac',
          'The built-in Ethernet port is not en0',
          'Apple has flagged the Apple ID or the serial',
        ],
        fixes: [
          'Use the serials generated for this EFI and keep them when rebuilding (“Keep serial numbers” option)',
          'Check the serial on Apple\'s coverage page: it should be reported as not valid',
          'Make the built-in Ethernet en0: remove all interfaces in System Settings → Network, delete /Library/Preferences/SystemConfiguration/NetworkInterfaces.plist and reboot',
          'Sign out of all Apple services, reset NVRAM, then sign in to iCloud first',
        ],
        advanced: ['A “customer code” error has to be resolved with Apple support'],
      },
      tr: {
        title: 'iMessage, FaceTime veya App Store girişi başarısız',
        symptoms: ['Mesajlar veya FaceTime giriş yapılamadı diyor', 'App Store indirmeleri başarısız oluyor'],
        causes: [
          "Seri, MLB veya ROM eksik, kopya ya da gerçek bir Mac tarafından kullanılıyor",
          'Dahili Ethernet portu en0 değil',
          'Apple, Apple Kimliğini veya seri numarasını işaretlemiş',
        ],
        fixes: [
          'Bu EFI için oluşturulan seri numaralarını kullanın ve yeniden derlerken koruyun (“Seri numaralarını koru” seçeneği)',
          "Seri numarasını Apple'ın kapsam sayfasında kontrol edin: geçersiz görünmelidir",
          "Dahili Ethernet'i en0 yapın: Sistem Ayarları → Ağ'daki tüm arayüzleri kaldırın, /Library/Preferences/SystemConfiguration/NetworkInterfaces.plist dosyasını silin ve yeniden başlatın",
          "Tüm Apple servislerinden çıkış yapın, NVRAM'i sıfırlayın ve önce iCloud'a giriş yapın",
        ],
        advanced: ['“Müşteri kodu” hatası Apple desteği ile çözülmelidir'],
      },
    },
  },
  {
    id: 'cpu-power',
    category: 'post-install',
    kexts: ['CPUFriend.kext'],
    text: {
      en: {
        title: 'CPU power management',
        symptoms: ['The CPU stays at full clock', 'Fast battery drain on laptops', 'High idle temperatures'],
        causes: [
          'The SMBIOS model does not match the CPU generation',
          'SSDT-PLUG missing on macOS 12.2 and older (Haswell to Comet Lake)',
          'Frequency vectors not tuned for this machine',
        ],
        fixes: [
          'Make sure the SMBIOS model fits your CPU (Review step)',
          'Fine-tune with CPUFriend and a CPUFriendDataProvider made with CPUFriendFriend',
          'Check the frequencies with “sudo powermetrics --samplers cpu_power” or Hackintool; Intel Power Gadget is discontinued',
          'AMD: power management works natively with the AMD_Vanilla patches; AMDRyzenCPUPowerManagement is an optional, unmaintained monitoring kext',
        ],
      },
      tr: {
        title: 'İşlemci güç yönetimi',
        symptoms: ['İşlemci sürekli tam hızda çalışıyor', 'Dizüstülerde pil hızlı bitiyor', 'Boştayken yüksek sıcaklıklar'],
        causes: [
          'SMBIOS modeli işlemci nesline uymuyor',
          "macOS 12.2 ve öncesinde SSDT-PLUG eksik (Haswell'den Comet Lake'e)",
          'Frekans vektörleri bu makineye göre ayarlanmamış',
        ],
        fixes: [
          'SMBIOS modelinin işlemcinize uyduğundan emin olun (İnceleme adımı)',
          'CPUFriend ve CPUFriendFriend ile üretilen bir CPUFriendDataProvider ile ince ayar yapın',
          "Frekansları “sudo powermetrics --samplers cpu_power” veya Hackintool ile kontrol edin; Intel Power Gadget'ın desteği sona erdi",
          'AMD: güç yönetimi AMD_Vanilla yamalarıyla yerel olarak çalışır; AMDRyzenCPUPowerManagement isteğe bağlı, bakımı yapılmayan bir izleme kext\'idir',
        ],
      },
    },
  },
];

/** Markdown copy of an entry for the clipboard. */
export function entryText(entry: TroubleEntry, text: TroubleText): string {
  return [
    `## ${text.title}`,
    '',
    ...text.symptoms.map((s) => `- ${s}`),
    '',
    ...text.causes.map((c) => `- ${c}`),
    '',
    ...text.fixes.map((f, i) => `${i + 1}. ${f}`),
    ...(text.advanced?.length ? ['', ...text.advanced.map((a) => `- ${a}`)] : []),
    ...(entry.kexts?.length ? ['', entry.kexts.join(', ')] : []),
  ].join('\n');
}
