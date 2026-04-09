import { useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { useWizard } from '../stores/wizard';
import { ChevronDown, ChevronRight, Usb, Cpu, Wifi, Shield, ExternalLink } from 'lucide-react';
import Logo from '../components/Logo';

interface PostInstallTip {
  id: string;
  icon: React.ReactNode;
  title: string;
  priority: 'critical' | 'recommended' | 'optional';
  description: string;
  steps: string[];
  link?: { label: string; url: string };
}

const POST_INSTALL_TIPS: PostInstallTip[] = [
  {
    id: 'usb-map',
    icon: <Usb size={14} />,
    title: 'USB Port Mapping',
    priority: 'critical',
    description:
      'macOS has a 15-port USB limit. Your initial boot uses USBInjectAll which works around this, but for a stable setup you need a proper USB port map.',
    steps: [
      'Boot into macOS and install Python 3',
      'Download USBMap from github.com/corpnewt/USBMap',
      'Run USBMap.command and follow the discovery process',
      'Plug a USB device into each physical port one by one',
      'After mapping all ports, select which 15 to keep',
      'Generate USBMap.kext and add it to your EFI/OC/Kexts',
      'Remove USBInjectAll.kext and set XhciPortLimit to false',
      'Reboot and verify all selected ports work',
    ],
    link: {
      label: 'USBMap Guide',
      url: 'https://dortania.github.io/OpenCore-Post-Install/usb/',
    },
  },
  {
    id: 'cpu-pm',
    icon: <Cpu size={14} />,
    title: 'CPU Power Management',
    priority: 'recommended',
    description:
      'For proper CPU frequency scaling (especially on laptops), generate a CPUFriendDataProvider that matches your SMBIOS model.',
    steps: [
      'Install CPUFriend.kext (already included if detected)',
      'Download CPUFriendFriend from github.com/corpnewt/CPUFriendFriend',
      'Run it and select your desired performance profile',
      'It generates CPUFriendDataProvider.kext',
      'Add both kexts to EFI/OC/Kexts and config.plist',
      'Reboot and verify with Intel Power Gadget or iStatMenus',
    ],
    link: {
      label: 'CPUFriend Guide',
      url: 'https://dortania.github.io/OpenCore-Post-Install/universal/pm.html',
    },
  },
  {
    id: 'iservices',
    icon: <Shield size={14} />,
    title: 'iServices Setup',
    priority: 'recommended',
    description:
      'iMessage, FaceTime, and other Apple services require valid SMBIOS serials. This app generates unique ones, but additional steps may be needed.',
    steps: [
      'Check your serial at checkcoverage.apple.com — it should NOT be valid',
      'In System Preferences > Network, delete all interfaces',
      'Re-add Ethernet first, then Wi-Fi (Ethernet must be en0)',
      'Sign out of all Apple services',
      'Reset NVRAM from OpenCore boot picker',
      'Sign back into iCloud, then iMessage',
    ],
    link: {
      label: 'iServices Guide',
      url: 'https://dortania.github.io/OpenCore-Post-Install/universal/iservices.html',
    },
  },
  {
    id: 'wifi-post',
    icon: <Wifi size={14} />,
    title: 'Intel Wi-Fi Setup',
    priority: 'optional',
    description:
      'If you have Intel Wi-Fi, itlwm provides basic connectivity but the HeliPort app gives you a native-feeling menu bar Wi-Fi manager.',
    steps: [
      'Download HeliPort from github.com/OpenIntelWireless/HeliPort',
      'Install and launch — it adds a Wi-Fi icon to the menu bar',
      'Connect to your network through HeliPort',
      'Optionally set HeliPort to launch at login',
    ],
    link: {
      label: 'OpenIntelWireless',
      url: 'https://openintelwireless.github.io/',
    },
  },
];

const PRIORITY_STYLES: Record<string, { label: string; color: string; bg: string }> = {
  critical: { label: 'Critical', color: '#ef4444', bg: '#ef444415' },
  recommended: { label: 'Recommended', color: '#f59e0b', bg: '#f59e0b15' },
  optional: { label: 'Optional', color: '#3b82f6', bg: '#3b82f615' },
};

export default function Complete() {
  const { reset } = useWizard();
  const [expandedTip, setExpandedTip] = useState<string | null>(null);

  return (
    <motion.div
      className="flex flex-col items-center pt-10 pb-8"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.4 }}
    >
      {/* Success glow */}
      <motion.div
        className="relative mb-6"
        initial={{ opacity: 0, scale: 0.7 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.6, type: 'spring', stiffness: 200 }}
      >
        <div className="absolute -inset-10 rounded-full bg-[#22c55e]/[0.06] blur-3xl" />
        <div className="relative flex items-center justify-center w-14 h-14 rounded-full bg-[#22c55e]/10 border border-[#22c55e]/20">
          <motion.svg
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            stroke="#22c55e"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            initial={{ pathLength: 0 }}
            animate={{ pathLength: 1 }}
            transition={{ delay: 0.3, duration: 0.5, ease: 'easeOut' }}
          >
            <motion.polyline
              points="20 6 9 17 4 12"
              initial={{ pathLength: 0 }}
              animate={{ pathLength: 1 }}
              transition={{ delay: 0.4, duration: 0.4, ease: 'easeOut' }}
            />
          </motion.svg>
        </div>
      </motion.div>

      <motion.h2
        className="text-[18px] font-semibold text-[#f0f0f2] mb-1.5"
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.4 }}
      >
        EFI Ready
      </motion.h2>

      <motion.p
        className="text-[12px] text-[#5a5a62] mb-6 text-center max-w-[320px] leading-relaxed"
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.55 }}
      >
        Your OpenCore EFI has been generated and deployed.
        Boot from the USB drive to start macOS installation.
      </motion.p>

      {/* Post-Install Guide */}
      <motion.div
        className="w-full max-w-[480px] mb-6"
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.65 }}
      >
        <p className="text-[10px] uppercase tracking-wide text-[#3e3e44] font-medium mb-2 px-1">
          Post-Install Guide
        </p>
        <div className="space-y-1.5">
          {POST_INSTALL_TIPS.map((tip) => {
            const isExpanded = expandedTip === tip.id;
            const style = PRIORITY_STYLES[tip.priority];

            return (
              <div
                key={tip.id}
                className="rounded-lg border border-[#1a1a1d] bg-[#0d0d0f] overflow-hidden"
              >
                <button
                  onClick={() => setExpandedTip(isExpanded ? null : tip.id)}
                  className="w-full flex items-center gap-2.5 px-3 py-2.5 text-left hover:bg-[#111113] transition-colors"
                >
                  <span className="text-[#5a5a62]">{tip.icon}</span>
                  <span className="flex-1 text-[12px] font-medium text-[#dadadf]">{tip.title}</span>
                  <span
                    className="text-[8px] uppercase tracking-wider font-semibold px-1.5 py-0.5 rounded"
                    style={{ color: style.color, backgroundColor: style.bg }}
                  >
                    {style.label}
                  </span>
                  {isExpanded ? (
                    <ChevronDown size={12} className="text-[#3e3e44]" />
                  ) : (
                    <ChevronRight size={12} className="text-[#3e3e44]" />
                  )}
                </button>

                <AnimatePresence>
                  {isExpanded && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.2 }}
                      className="overflow-hidden"
                    >
                      <div className="px-3 pb-3 pt-1 border-t border-[#1a1a1d]">
                        <p className="text-[11px] text-[#6e6e76] leading-relaxed mb-2.5">
                          {tip.description}
                        </p>
                        <ol className="space-y-1.5 mb-2">
                          {tip.steps.map((step, i) => (
                            <li key={i} className="flex items-start gap-2 text-[11px] text-[#a0a0a8]">
                              <span className="text-[#3e3e44] font-mono text-[9px] mt-0.5 w-3 shrink-0 text-right">
                                {i + 1}.
                              </span>
                              {step}
                            </li>
                          ))}
                        </ol>
                        {tip.link && (
                          <a
                            href={tip.link.url}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex items-center gap-1.5 text-[10px] text-[#3b82f6] hover:text-[#60a5fa]"
                          >
                            {tip.link.label}
                            <ExternalLink size={10} />
                          </a>
                        )}
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            );
          })}
        </div>
      </motion.div>

      <motion.button
        onClick={reset}
        className="h-8 px-4 rounded-[6px] bg-[#141416] border border-[#1e1e22] text-[12px] text-[#6e6e76] hover:text-[#a0a0a8] hover:border-[#2e2e32] transition-all duration-150"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.8 }}
        whileHover={{ scale: 1.02 }}
        whileTap={{ scale: 0.98 }}
      >
        Start Over
      </motion.button>
    </motion.div>
  );
}
