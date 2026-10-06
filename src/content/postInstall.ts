import type { Localized } from '../i18n/lang';

export interface GuideStep {
  id: 'boot' | 'copy-efi' | 'usb-map' | 'tahoe-audio' | 'root-patch' | 'intel-wifi' | 'iservices' | 'backup';
  title: string;
  body: string;
  steps: string[];
  link?: string;
}

export interface GuideContext {
  needsRootPatch: boolean;
  tahoe: boolean;
  intelWifi: boolean;
  /** The profile has an onboard (analog) audio codec. */
  analogAudio: boolean;
}

const EN: GuideStep[] = [
  {
    id: 'boot',
    title: 'Install macOS from the USB drive',
    body: 'Boot the target PC from the USB drive and install macOS. The first boot after each install phase takes a while.',
    steps: [
      'Open the firmware boot menu (often F8, F11 or F12) and pick the USB drive (UEFI entry).',
      'In the OpenCore picker choose the recovery entry ("macOS Base System" or similar).',
      'In Disk Utility choose View → Show All Devices and erase only the disk you want macOS on (APFS, GUID partition map). Everything on that disk is deleted, so use a separate disk if you keep Windows or Linux.',
      'Run "Reinstall macOS". The installer downloads macOS from Apple, so a working (preferably wired) network connection is needed.',
      'Keep choosing the macOS installer / the disk name in the picker until the first macOS setup screen appears.',
      'Installing on the same disk as Windows needs an EFI partition of at least 200 MB as the first partition; Windows often creates only 100 MB. A separate disk avoids this.',
    ],
    link: 'https://dortania.github.io/OpenCore-Install-Guide/installation/installation-process.html',
  },
  {
    id: 'copy-efi',
    title: 'Copy the EFI to the internal disk',
    body: 'Until you do this the machine can only start macOS with the USB drive plugged in.',
    steps: [
      'In macOS, run "diskutil list" in Terminal and find the EFI partition of the macOS disk (for example disk0s1).',
      'Mount it with "sudo diskutil mount disk0s1". The USB drive written by this app shows up in Finder on its own.',
      'Copy the whole EFI folder from the USB drive (or the copy you exported from this app) to the internal EFI partition.',
      'Remove the USB drive, reboot and make sure the internal disk starts OpenCore. Set it as the first boot option in the firmware.',
      'If Windows keeps putting its boot manager first, set Misc → Boot → LauncherOption to Full in config.plist.',
    ],
    link: 'https://dortania.github.io/OpenCore-Post-Install/universal/oc2hdd.html',
  },
  {
    id: 'usb-map',
    title: 'Map your USB ports',
    body: 'macOS supports at most 15 ports per USB controller. A port map makes every port, including the internal Bluetooth one, work reliably.',
    steps: [
      'On Windows (easiest, can be done before installing): run USBToolBox, discover the ports by plugging a USB 2 and a USB 3 device into each one, then build the map.',
      'Copy UTBMap.kext into EFI/OC/Kexts next to USBToolBox.kext, remove UTBDefault.kext if it is there, and update config.plist (ProperTree OC Snapshot).',
      'Remove USBInjectAll.kext if present and turn the XhciPortLimit quirk off.',
      'macOS 26 Tahoe: UTBMap.kext works as it is with USBToolBox.kext 1.2.0 or newer. A native USBMap.kext (without USBToolBox) needs both the old (port / UsbConnector) and the new (usb-port-number / usb-port-type) keys.',
    ],
    link: 'https://github.com/USBToolBox/tool',
  },
  {
    id: 'tahoe-audio',
    title: 'Onboard audio on macOS 26 Tahoe',
    body: 'macOS 26 no longer includes AppleHDA, so the speakers and the headphone jack do not work with AppleALC alone.',
    steps: [
      'HDMI/DisplayPort audio from an AMD graphics card and USB audio devices work without changes.',
      'VoodooHDA can drive the onboard codec (lower quality than AppleALC); it needs SIP partially disabled.',
      'AppleHDA can be restored with a root patch, but it has to be applied again after every macOS update.',
      'If onboard audio matters to you, macOS 15 Sequoia keeps working with AppleALC.',
    ],
    link: 'https://dortania.github.io/OpenCore-Install-Guide/extras/tahoe.html',
  },
  {
    id: 'root-patch',
    title: 'Apply root patches for the missing drivers',
    body: 'The macOS version you chose no longer ships some drivers this machine needs (graphics, Wi-Fi or audio). They have to be patched back in after installing.',
    steps: [
      'Finish the installation and the first boot without the affected device (expect no acceleration, Wi-Fi or audio until patched).',
      'Use OpenCore Legacy Patcher\'s post-install root patching for the affected component.',
      'Root patching changes SIP (csr-active-config) and SecureBootModel; it has to be applied again after every macOS update.',
    ],
    link: 'https://dortania.github.io/OpenCore-Legacy-Patcher/',
  },
  {
    id: 'intel-wifi',
    title: 'Intel Wi-Fi',
    body: 'Intel Wi-Fi uses the OpenIntelWireless kexts. With itlwm you join networks through the HeliPort app instead of the menu bar.',
    steps: [
      'Install the HeliPort app and add it to the login items.',
      'itlwm does not work in macOS Recovery; use Ethernet for the installation.',
      'On macOS 15 and 26 the native Wi-Fi menu (AirportItlwm) is not available without extra root patching.',
    ],
    link: 'https://openintelwireless.github.io/',
  },
  {
    id: 'iservices',
    title: 'iMessage, FaceTime and the App Store',
    body: 'The generated serial numbers are unique to this EFI. Keep the same EFI (or the "keep serial numbers" option) when you rebuild.',
    steps: [
      'Check the serial on Apple\'s coverage page: it should be reported as not valid.',
      'Make sure the built-in Ethernet port is en0 (System Settings → Network on macOS 13 and newer).',
      'Sign in to iCloud first, then to Messages and FaceTime.',
    ],
    link: 'https://dortania.github.io/OpenCore-Post-Install/universal/iservices.html',
  },
  {
    id: 'backup',
    title: 'Keep a working copy',
    body: 'Keep the USB drive (or the exported EFI) as a known-good fallback before you experiment with kexts or updates.',
    steps: [
      'Before changing kexts or updating OpenCore, copy the working EFI folder somewhere safe.',
      'Update OpenCore and kexts together; the config.plist must match the OpenCore version.',
    ],
  },
];

const TR: GuideStep[] = [
  {
    id: 'boot',
    title: "macOS'u USB bellekten kurun",
    body: "Hedef bilgisayarı USB bellekten başlatıp macOS'u kurun. Her kurulum aşamasından sonraki ilk açılış biraz uzun sürer.",
    steps: [
      'Firmware açılış menüsünü açın (genellikle F8, F11 veya F12) ve USB belleği (UEFI girişi) seçin.',
      'OpenCore menüsünde kurtarma girişini seçin ("macOS Base System" veya benzeri).',
      'Disk İzlencesi\'nde Görüntü → Tüm Aygıtları Göster\'i seçin ve yalnızca macOS\'u kuracağınız diski silin (APFS, GUID bölüm şeması). O diskteki her şey silinir; Windows veya Linux\'u koruyacaksanız ayrı bir disk kullanın.',
      '"macOS\'u Yeniden Yükle"yi çalıştırın. Yükleyici macOS\'u Apple\'dan indirir, bu yüzden çalışan (tercihen kablolu) bir ağ bağlantısı gerekir.',
      'İlk macOS kurulum ekranı açılana kadar menüde macOS yükleyicisini / disk adını seçmeye devam edin.',
      "Windows ile aynı diske kurulum için ilk bölüm olarak en az 200 MB'lık bir EFI bölümü gerekir; Windows çoğu zaman yalnızca 100 MB oluşturur. Ayrı bir disk bu sorunu ortadan kaldırır.",
    ],
    link: 'https://dortania.github.io/OpenCore-Install-Guide/installation/installation-process.html',
  },
  {
    id: 'copy-efi',
    title: "EFI'yi dahili diske kopyalayın",
    body: "Bunu yapana kadar bilgisayar macOS'u yalnızca USB bellek takılıyken başlatabilir.",
    steps: [
      'macOS\'ta Terminal\'de "diskutil list" komutunu çalıştırın ve macOS diskinin EFI bölümünü bulun (örneğin disk0s1).',
      '"sudo diskutil mount disk0s1" ile bağlayın. Bu uygulamanın yazdığı USB bellek Finder\'da kendiliğinden görünür.',
      'EFI klasörünün tamamını USB bellekten (veya bu uygulamadan dışa aktardığınız kopyadan) dahili EFI bölümüne kopyalayın.',
      "USB belleği çıkarın, yeniden başlatın ve dahili diskin OpenCore'u açtığından emin olun. Firmware'de ilk açılış seçeneği yapın.",
      "Windows kendi önyükleme yöneticisini sürekli başa alıyorsa config.plist'te Misc → Boot → LauncherOption değerini Full yapın.",
    ],
    link: 'https://dortania.github.io/OpenCore-Post-Install/universal/oc2hdd.html',
  },
  {
    id: 'usb-map',
    title: 'USB portlarınızı eşleyin',
    body: "macOS, USB denetleyicisi başına en fazla 15 portu destekler. Port haritası, dahili Bluetooth portu dahil her portun düzgün çalışmasını sağlar.",
    steps: [
      "Windows'ta (en kolayı, kurulumdan önce de yapılabilir): USBToolBox'ı çalıştırın, her porta bir USB 2 ve bir USB 3 cihaz takarak portları tanıtın ve haritayı oluşturun.",
      "UTBMap.kext'i EFI/OC/Kexts içine USBToolBox.kext'in yanına kopyalayın, varsa UTBDefault.kext'i kaldırın ve config.plist'i güncelleyin (ProperTree OC Snapshot).",
      'Varsa USBInjectAll.kext\'i kaldırın ve XhciPortLimit ayarını kapatın.',
      "macOS 26 Tahoe: UTBMap.kext, USBToolBox.kext 1.2.0 veya daha yenisiyle olduğu gibi çalışır. Yerel bir USBMap.kext (USBToolBox olmadan) hem eski (port / UsbConnector) hem yeni (usb-port-number / usb-port-type) anahtarları içermelidir.",
    ],
    link: 'https://github.com/USBToolBox/tool',
  },
  {
    id: 'tahoe-audio',
    title: "macOS 26 Tahoe'da dahili ses",
    body: "macOS 26 artık AppleHDA içermiyor; bu yüzden hoparlörler ve kulaklık girişi yalnızca AppleALC ile çalışmaz.",
    steps: [
      'AMD ekran kartının HDMI/DisplayPort sesi ve USB ses aygıtları değişiklik gerektirmeden çalışır.',
      "VoodooHDA dahili codec'i sürebilir (AppleALC'den daha düşük kalite); SIP'in kısmen kapatılmasını gerektirir.",
      'AppleHDA bir root yamasıyla geri getirilebilir, ancak her macOS güncellemesinden sonra yeniden uygulanmalıdır.',
      "Dahili ses sizin için önemliyse macOS 15 Sequoia, AppleALC ile çalışmaya devam eder.",
    ],
    link: 'https://dortania.github.io/OpenCore-Install-Guide/extras/tahoe.html',
  },
  {
    id: 'root-patch',
    title: 'Eksik sürücüler için root yaması uygulayın',
    body: 'Seçtiğiniz macOS sürümü bu makinenin ihtiyaç duyduğu bazı sürücüleri (grafik, Wi-Fi veya ses) artık içermiyor. Bunların kurulumdan sonra yamayla geri eklenmesi gerekir.',
    steps: [
      'Kurulumu ve ilk açılışı ilgili cihaz olmadan tamamlayın (yamaya kadar hızlandırma, Wi-Fi veya ses olmayabilir).',
      "İlgili bileşen için OpenCore Legacy Patcher'ın kurulum sonrası root yamasını kullanın.",
      'Root yaması SIP (csr-active-config) ve SecureBootModel ayarlarını değiştirir; her macOS güncellemesinden sonra yeniden uygulanmalıdır.',
    ],
    link: 'https://dortania.github.io/OpenCore-Legacy-Patcher/',
  },
  {
    id: 'intel-wifi',
    title: 'Intel Wi-Fi',
    body: 'Intel Wi-Fi, OpenIntelWireless kext\'lerini kullanır. itlwm ile ağlara menü çubuğu yerine HeliPort uygulamasından bağlanırsınız.',
    steps: [
      'HeliPort uygulamasını kurun ve giriş öğelerine ekleyin.',
      "itlwm, macOS Kurtarma'da çalışmaz; kurulum için Ethernet kullanın.",
      'macOS 15 ve 26\'da yerel Wi-Fi menüsü (AirportItlwm) ek root yaması olmadan kullanılamaz.',
    ],
    link: 'https://openintelwireless.github.io/',
  },
  {
    id: 'iservices',
    title: "iMessage, FaceTime ve App Store",
    body: 'Oluşturulan seri numaraları bu EFI\'ye özeldir. Yeniden derlerken aynı EFI\'yi (veya "seri numaralarını koru" seçeneğini) kullanın.',
    steps: [
      "Seri numarasını Apple'ın kapsam sorgulama sayfasında kontrol edin: geçersiz olarak görünmelidir.",
      'Dahili Ethernet portunun en0 olduğundan emin olun (macOS 13 ve sonrasında Sistem Ayarları → Ağ).',
      "Önce iCloud'a, ardından Mesajlar ve FaceTime'a giriş yapın.",
    ],
    link: 'https://dortania.github.io/OpenCore-Post-Install/universal/iservices.html',
  },
  {
    id: 'backup',
    title: 'Çalışan bir kopya saklayın',
    body: "Kext'ler veya güncellemelerle denemeler yapmadan önce USB belleği (veya dışa aktarılan EFI'yi) çalıştığı bilinen bir yedek olarak saklayın.",
    steps: [
      "Kext'leri değiştirmeden veya OpenCore'u güncellemeden önce çalışan EFI klasörünü güvenli bir yere kopyalayın.",
      "OpenCore'u ve kext'leri birlikte güncelleyin; config.plist, OpenCore sürümüyle uyumlu olmalıdır.",
    ],
  },
];

export const POST_INSTALL: Localized<GuideStep[]> = { en: EN, tr: TR };

/** Guide steps that apply to this build. */
export function guideFor(steps: readonly GuideStep[], ctx: GuideContext): GuideStep[] {
  return steps.filter((s) => {
    if (s.id === 'root-patch') return ctx.needsRootPatch;
    if (s.id === 'tahoe-audio') return ctx.tahoe && ctx.analogAudio;
    if (s.id === 'intel-wifi') return ctx.intelWifi;
    return true;
  });
}
